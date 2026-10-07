use crate::{
    board::{Board, Format, Side},
    project::Project,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path, process::Command};

#[derive(Debug, Serialize)]
pub struct Assembly {
    pub bom: String,
    pub cpl: String,
    pub warnings: Vec<String>,
    pub included: usize,
}
fn row(values: impl IntoIterator<Item = String>) -> String {
    let cols: Vec<_> = values
        .into_iter()
        .map(|v| {
            if v.contains([',', '"', '\r', '\n']) {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                v
            }
        })
        .collect();
    cols.join(",") + "\r\n"
}
fn package(s: &str) -> &str {
    s.rsplit(':').next().unwrap_or(s)
}
fn decimal(n: f64) -> String {
    let text = format!("{n:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0.0".into()
    } else if text.contains('.') {
        text.into()
    } else {
        format!("{text}.0")
    }
}
pub fn assembly(board: &Board, allow_missing: bool) -> Result<Assembly> {
    let mut warnings = board.warnings.clone();
    let mut groups: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    let mut cpl = String::from("Designator,Val,Package,Mid X,Mid Y,Rotation,Layer\r\n");
    let mut included = 0;
    for p in board.parts.iter().filter(|p| !p.dnp) {
        if p.exclude_bom && p.exclude_cpl {
            continue;
        }
        ensure!(
            [p.x, p.y, p.rotation].iter().all(|v| v.is_finite()),
            "Non-finite placement {}",
            p.reference
        );
        if !p.exclude_bom {
            if p.lcsc.is_empty() {
                ensure!(allow_missing,"{} has no LCSC part. Assign it, exclude it, or explicitly allow missing parts.",p.reference);
                warnings.push(format!("{}: missing LCSC part", p.reference));
            } else {
                crate::lcsc_id(&p.lcsc).with_context(|| format!("{}", p.reference))?;
            }
            groups
                .entry((p.value.clone(), p.footprint.clone(), p.lcsc.clone()))
                .or_default()
                .push(p.reference.clone());
        }
        if !p.exclude_cpl && !p.placed {
            warnings.push(format!(
                "{}: no board placement; omitted from CPL",
                p.reference
            ));
        }
        if !p.exclude_cpl && p.placed {
            cpl += &row([
                p.reference.clone(),
                p.value.clone(),
                package(&p.footprint).into(),
                decimal(p.x),
                decimal(p.y),
                decimal(if p.rotation.rem_euclid(360.) > 180. {
                    p.rotation.rem_euclid(360.) - 360.
                } else {
                    p.rotation.rem_euclid(360.)
                }),
                if p.side == Side::Top {
                    "top".into()
                } else {
                    "bottom".into()
                },
            ]);
            included += 1;
        }
    }
    let mut bom = String::from("Comment,Designator,Footprint,LCSC,Quantity\r\n");
    for ((value, footprint, lcsc), refs) in groups {
        let footprint = package(&footprint).to_owned();
        let mut chunk = Vec::<String>::new();
        let mut length = 0;
        for r in refs {
            ensure!(r.len() < 1800, "Reference is too long for a JLCPCB BOM");
            if !chunk.is_empty() && length + r.len() + 1 > 1800 {
                bom += &row([
                    value.clone(),
                    chunk.join(","),
                    footprint.clone(),
                    lcsc.clone(),
                    chunk.len().to_string(),
                ]);
                chunk.clear();
                length = 0;
            }
            length += r.len() + usize::from(!chunk.is_empty());
            chunk.push(r);
        }
        bom += &row([
            value,
            chunk.join(","),
            footprint,
            lcsc,
            chunk.len().to_string(),
        ]);
    }
    Ok(Assembly {
        bom,
        cpl,
        warnings,
        included,
    })
}
/// Publish a complete new directory; failures leave no partial manufacturing set.
pub fn export(
    project: &Project,
    out: &Path,
    allow_missing: bool,
    gerbers: bool,
) -> Result<Assembly> {
    ensure!(
        !out.exists(),
        "Output directory already exists: {}. Choose a new directory.",
        out.display()
    );
    let board = project.snapshot()?;
    let result = assembly(&board, allow_missing)?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    std::fs::write(stage.path().join("BOM.csv"), &result.bom)?;
    if project.has_placement() {
        let missing: Vec<_> = board
            .parts
            .iter()
            .filter(|p| !p.dnp && !p.exclude_cpl && !p.placed)
            .map(|p| p.reference.as_str())
            .collect();
        ensure!(
            missing.is_empty(),
            "CPL export requires placements for: {}. Update the board before exporting.",
            missing.join(", ")
        );
        std::fs::write(stage.path().join("CPL.csv"), &result.cpl)?;
    }
    std::fs::write(
        stage.path().join("report.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    std::fs::write(
        stage.path().join("project.json"),
        serde_json::to_vec_pretty(project)?,
    )?;
    if gerbers {
        ensure!(
            project.has_placement() && board.format == Format::Kicad,
            "Eagle CAM generation requires Eagle/Fusion; this release exports Eagle BOM/CPL only"
        );
        let version = Command::new("kicad-cli")
            .arg("version")
            .output()
            .context("Install KiCad 10+ and add kicad-cli to PATH")?;
        ensure!(version.status.success(), "kicad-cli version failed");
        let v = String::from_utf8_lossy(&version.stdout);
        ensure!(
            v.trim()
                .split('.')
                .next()
                .unwrap_or("")
                .parse::<u32>()
                .unwrap_or(0)
                >= 10,
            "KiCad 10+ required, got {v}"
        );
        // Work from the exact bytes that match the assignment snapshot.
        let input = stage.path().join("source.kicad_pcb");
        std::fs::write(&input, project.source()?)?;
        let dir = stage.path().join("gerbers");
        std::fs::create_dir(&dir)?;
        for kind in ["gerbers", "drill"] {
            let output = Command::new("kicad-cli")
                .args(["pcb", "export", kind, "--output"])
                .arg(&dir)
                .arg(&input)
                .output()?;
            ensure!(
                output.status.success(),
                "KiCad {kind} failed: {} {}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
        }
        std::fs::remove_file(input)?;
    }
    project.snapshot()?; // Check all CAD sources again after slow native exports.
    std::fs::rename(stage.path(), out)?;
    Ok(result)
}

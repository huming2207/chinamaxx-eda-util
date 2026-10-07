//! Resolve CAD projects and load schematic BOMs. KiCad handles its own hierarchy.
use crate::board::{self, Board, Format, Part, Side};
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

pub struct Input {
    pub path: PathBuf,
    pub board: Option<PathBuf>,
    pub files: BTreeSet<PathBuf>,
    pub bom: Board,
}
fn ext(p: &Path) -> &str {
    p.extension().and_then(|s| s.to_str()).unwrap_or("")
}
fn unique(files: Vec<PathBuf>) -> Result<PathBuf> {
    ensure!(
        files.len() == 1,
        "Choose a specific project or schematic; found {} candidates",
        files.len()
    );
    Ok(files[0].clone())
}
pub fn resolve(path: &Path) -> Result<PathBuf> {
    if !path.is_dir() {
        return Ok(path.canonicalize()?);
    }
    let files: Vec<_> = std::fs::read_dir(path)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    for extensions in [
        &["kicad_pro", "epf"][..],
        &["kicad_sch", "sch"],
        &["kicad_pcb", "brd"],
    ] {
        let candidates = files
            .iter()
            .filter(|p| extensions.contains(&ext(p)))
            .cloned()
            .collect::<Vec<_>>();
        if !candidates.is_empty() {
            return Ok(unique(candidates)?.canonicalize()?);
        }
    }
    bail!(
        "No supported CAD project, schematic or board in {}",
        path.display()
    )
}
pub fn load(path: &Path) -> Result<Input> {
    let path = resolve(path)?;
    let mut files = BTreeSet::from([path.clone()]);
    let schematic = match ext(&path) {
        "kicad_pro" => {
            let p = path.with_extension("kicad_sch");
            ensure!(p.exists(), "Project schematic missing: {}", p.display());
            p
        }
        "epf" => {
            let p = path.with_extension("sch");
            if p.exists() {
                p
            } else {
                unique(
                    std::fs::read_dir(path.parent().unwrap())?
                        .filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| ext(p) == "sch")
                        .collect(),
                )?
            }
        }
        "kicad_sch" | "sch" => path.clone(),
        "kicad_pcb" | "brd" => {
            return Ok(Input {
                bom: board::load(&path)?,
                board: Some(path.clone()),
                path,
                files,
            })
        }
        _ => bail!("Open .kicad_pro, .epf, .kicad_sch, Eagle XML .sch, .kicad_pcb or .brd"),
    };
    files.insert(schematic.clone());
    let mut bom = if ext(&schematic) == "kicad_sch" {
        collect_sheets(&schematic, &mut files, &mut BTreeSet::new())?;
        let pro = schematic.with_extension("kicad_pro");
        if pro.exists() {
            files.insert(pro);
        }
        let tmp = tempfile::tempdir()?;
        let net = tmp.path().join("bom.xml");
        let result = Command::new("kicad-cli")
            .args([
                "sch", "export", "netlist", "--format", "kicadxml", "--output",
            ])
            .arg(&net)
            .arg(&schematic)
            .output()
            .context("Install KiCad 10+ and add kicad-cli to PATH to open KiCad schematics")?;
        ensure!(
            result.status.success(),
            "KiCad schematic export failed: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        parse_kicad_netlist(&std::fs::read_to_string(net)?)?
    } else {
        parse_eagle_schematic(&std::fs::read_to_string(&schematic)?)?
    };
    let board_path = schematic.with_extension(if bom.format == Format::Kicad {
        "kicad_pcb"
    } else {
        "brd"
    });
    let board = if board_path.exists() {
        files.insert(board_path.clone());
        let layout = board::load(&board_path)?;
        attach_layout(&mut bom, layout);
        Some(board_path)
    } else {
        bom.warnings
            .push("No matching board: BOM available; CPL requires board placement data.".into());
        None
    };
    let mut refs = BTreeSet::new();
    for p in &bom.parts {
        ensure!(
            !p.reference.is_empty() && !p.reference.contains('?'),
            "Annotate schematic before picking parts: {}",
            p.reference
        );
        ensure!(
            refs.insert(&p.reference),
            "Duplicate schematic reference {}",
            p.reference
        );
    }
    bom.parts.sort_by(|a, b| a.reference.cmp(&b.reference));
    Ok(Input {
        path,
        board,
        files,
        bom,
    })
}
fn attach_layout(bom: &mut Board, layout: Board) {
    for part in &mut bom.parts {
        if let Some(placement) = layout.parts.iter().find(|p| p.reference == part.reference) {
            part.x = placement.x;
            part.y = placement.y;
            part.rotation = placement.rotation;
            part.side = placement.side;
            part.placed = true;
            part.exclude_bom |= placement.exclude_bom;
            part.exclude_cpl |= placement.exclude_cpl;
            part.dnp |= placement.dnp;
            // Physical package and coordinates always come from the board.
            part.footprint = placement.footprint.clone();
            if part.lcsc.is_empty() {
                part.lcsc = placement.lcsc.clone();
            }
        }
    }
    bom.warnings.extend(layout.warnings);
    for extra in layout
        .parts
        .iter()
        .filter(|p| !p.exclude_bom && !bom.parts.iter().any(|s| s.reference == p.reference))
    {
        bom.warnings.push(format!(
            "{} exists only on the board; schematic BOM omits it",
            extra.reference
        ));
    }
}
fn collect_sheets(
    path: &Path,
    files: &mut BTreeSet<PathBuf>,
    active: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let path = path.canonicalize()?;
    ensure!(
        active.len() < 64 && active.insert(path.clone()),
        "Cyclic or excessively deep schematic hierarchy"
    );
    files.insert(path.clone());
    let root = crate::sexpr::parse(&std::fs::read_to_string(&path)?)?;
    ensure!(root.tag() == "kicad_sch", "Expected KiCad schematic");
    for sheet in root.all("sheet") {
        let name = sheet
            .all("property")
            .find(|p| p.at(1).eq_ignore_ascii_case("Sheetfile"))
            .context("Sheet has no Sheetfile")?
            .at(2);
        ensure!(
            !name.contains("${"),
            "Resolve schematic sheet path variable before import: {name}"
        );
        collect_sheets(&path.parent().unwrap().join(name), files, active)?;
    }
    active.remove(&path);
    Ok(())
}
fn unplaced(
    reference: &str,
    value: &str,
    footprint: &str,
    props: &BTreeMap<String, String>,
) -> Part {
    Part {
        reference: reference.into(),
        value: value.into(),
        footprint: footprint.into(),
        lcsc: board::field(props),
        x: 0.,
        y: 0.,
        rotation: 0.,
        side: Side::Top,
        placed: false,
        dnp: props
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("dnp") && (v.is_empty() || board::truth(v))),
        exclude_bom: false,
        exclude_cpl: false,
    }
}
pub fn parse_kicad_netlist(s: &str) -> Result<Board> {
    let doc = roxmltree::Document::parse(s)?;
    let components = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("components"))
        .context("Missing KiCad netlist components")?;
    let mut parts = vec![];
    for c in components.children().filter(|n| n.has_tag_name("comp")) {
        let child = |tag| {
            c.children()
                .find(|n| n.has_tag_name(tag))
                .and_then(|n| n.text())
                .unwrap_or("")
        };
        let props: BTreeMap<String, String> = c
            .descendants()
            .filter(|n| n.has_tag_name("field") || n.has_tag_name("property"))
            .map(|n| {
                (
                    n.attribute("name").unwrap_or("").into(),
                    n.attribute("value").or(n.text()).unwrap_or("").into(),
                )
            })
            .collect();
        let mut p = unplaced(
            c.attribute("ref").unwrap_or(""),
            child("value"),
            child("footprint"),
            &props,
        );
        p.exclude_bom = props.contains_key("exclude_from_bom");
        p.exclude_cpl = props.contains_key("exclude_from_board");
        parts.push(p);
    }
    Ok(Board {
        format: Format::Kicad,
        parts,
        warnings: vec![],
    })
}
pub fn parse_eagle_schematic(s: &str) -> Result<Board> {
    let doc = roxmltree::Document::parse_with_options(
        s,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )?;
    let schematic = doc
        .descendants()
        .find(|n| n.has_tag_name("schematic"))
        .context("Expected Eagle XML schematic (save binary files as Eagle 6+ XML)")?;
    let mut parts = vec![];
    for el in schematic
        .children()
        .find(|n| n.has_tag_name("parts"))
        .context("Schematic has no parts")?
        .children()
        .filter(|n| n.has_tag_name("part"))
    {
        let a = |k| el.attribute(k).unwrap_or("");
        let lib = schematic
            .descendants()
            .find(|n| n.has_tag_name("library") && n.attribute("name") == Some(a("library")))
            .context("Missing embedded Eagle library")?;
        let set = lib
            .descendants()
            .find(|n| n.has_tag_name("deviceset") && n.attribute("name") == Some(a("deviceset")))
            .context("Missing Eagle deviceset")?;
        let device = set
            .descendants()
            .find(|n| n.has_tag_name("device") && n.attribute("name") == Some(a("device")))
            .context("Missing Eagle device")?;
        let package = device.attribute("package").unwrap_or("");
        let mut props = BTreeMap::new();
        if let Some(tech) = device.descendants().find(|n| {
            n.has_tag_name("technology") && n.attribute("name").unwrap_or("") == a("technology")
        }) {
            for attr in tech.children().filter(|n| n.has_tag_name("attribute")) {
                props.insert(
                    attr.attribute("name").unwrap_or("").into(),
                    attr.attribute("value").unwrap_or("").into(),
                );
            }
        }
        for attr in el.children().filter(|n| n.has_tag_name("attribute")) {
            props.insert(
                attr.attribute("name").unwrap_or("").into(),
                attr.attribute("value").unwrap_or("").into(),
            );
        }
        let mut p = unplaced(
            a("name"),
            if a("value").is_empty() {
                a("deviceset")
            } else {
                a("value")
            },
            &format!("{}:{package}", a("library")),
            &props,
        );
        p.dnp |= a("populate") == "no";
        p.exclude_bom = package.is_empty();
        p.exclude_cpl = package.is_empty();
        parts.push(p);
    }
    Ok(Board {
        format: Format::Eagle,
        parts,
        warnings: vec!["Eagle default assembly variant is used.".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paired_board_preserves_independent_bom_exclusion() {
        let mut bom = parse_kicad_netlist(r#"<export><components><comp ref="R1"><value>10k</value><footprint>R_0603</footprint></comp></components></export>"#).unwrap();
        let mut layout = bom.clone();
        layout.parts[0].placed = true;
        layout.parts[0].exclude_bom = true;
        layout.parts[0].x = 12.;
        attach_layout(&mut bom, layout);
        let result = crate::export::assembly(&bom, false).unwrap();
        assert_eq!(result.bom.lines().count(), 1);
        assert!(result.cpl.contains("R1,10k,R_0603,12.0"));
    }
}

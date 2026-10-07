use anyhow::{bail, ensure, Context, Result};
use chinamaxxbom_core::{
    board, export,
    library::{Library, Target},
    project::Project,
    supplier::{self, Bundle},
};
use pico_args::Arguments;
use std::path::PathBuf;
const HELP: &str = r#"ChinamaxxBOM — Eagle / KiCad 10+ assembly and library tools

  chinamaxxbom cache-days [DAYS]  (7–60; default 7, shared with GUI)
  chinamaxxbom open PROJECT_OR_SCHEMATIC_OR_BOARD  (loads BOM and resumes picks)
  chinamaxxbom inspect BOARD
  chinamaxxbom init BOARD --project FILE
  chinamaxxbom assign PROJECT REFERENCE C12345
  chinamaxxbom dnp PROJECT REFERENCE true|false
  chinamaxxbom correct PROJECT REFERENCE [--dx MM] [--dy MM] [--rotation DEG]
  chinamaxxbom export PROJECT --out NEW_DIRECTORY [--allow-missing] [--gerbers]
  chinamaxxbom write-board PROJECT --out NEW_BOARD_FILE
  chinamaxxbom search QUERY [--catalog FILE] [--basic] [--in-stock] [--page N]
  chinamaxxbom fetch C12345 --out NEW_BUNDLE_JSON
  chinamaxxbom import --bundle FILE --out NEW_DIRECTORY [--target both|kicad|eagle]
  chinamaxxbom import --lcsc C12345 --out NEW_DIRECTORY [--target both|kicad|eagle]
  chinamaxxbom bundle C12345 --symbol SYMBOL_JSON --footprint FOOTPRINT_JSON --out FILE

JSON on stdout; errors on stderr (exit 1). Corrections use assembly world axes,
mm and degrees CCW. Online commands need curl. For the native desktop UI run
chinamaxxbom-gui [BOARD_OR_PROJECT]. Existing export/library paths are never overwritten.
"#;
fn check(a: Arguments) -> Result<()> {
    let rest = a.finish();
    ensure!(rest.is_empty(), "Unknown arguments: {rest:?}");
    Ok(())
}
fn run() -> Result<()> {
    let mut a = Arguments::from_env();
    if a.contains(["-h", "--help"]) {
        println!("{HELP}");
        return Ok(());
    }
    if a.contains(["-V", "--version"]) {
        println!("chinamaxxbom {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let cmd = match a.subcommand()? {
        Some(cmd) => cmd,
        None => {
            println!("{HELP}");
            return Ok(());
        }
    };
    match cmd.as_str() {
        "cache-days" => {
            let days: Option<u64> = a.opt_free_from_str()?;
            check(a)?;
            if let Some(days) = days {
                chinamaxxbom_core::cache::Cache::save_ttl_days(days)?;
            }
            println!(
                "{}",
                chinamaxxbom_core::cache::Cache::for_user()?.ttl_days()
            );
        }
        "open" => {
            let path: PathBuf = a.free_from_str()?;
            check(a)?;
            let (project, sidecar) = Project::open(&path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"project": sidecar, "bom": project.snapshot()?})
                )?
            );
        }
        "inspect" => {
            let path: PathBuf = a.free_from_str()?;
            check(a)?;
            println!("{}", serde_json::to_string_pretty(&board::load(&path)?)?);
        }
        "init" => {
            let board: PathBuf = a.free_from_str()?;
            let path: PathBuf = a.value_from_str("--project")?;
            check(a)?;
            ensure!(!path.exists(), "Project exists: {}", path.display());
            let p = Project::new(&board)?;
            chinamaxxbom_core::project::write_new(&path, &serde_json::to_vec_pretty(&p)?)?;
            println!("{}", path.display());
        }
        "assign" | "dnp" | "correct" => {
            let path: PathBuf = a.free_from_str()?;
            let reference: String = a.free_from_str()?;
            let id = if cmd == "assign" {
                Some(chinamaxxbom_core::lcsc_id(&a.free_from_str::<String>()?)?)
            } else {
                None
            };
            let dnp = if cmd == "dnp" {
                Some(a.free_from_str::<bool>()?)
            } else {
                None
            };
            let (dx, dy, rotation): (Option<f64>, Option<f64>, Option<f64>) = if cmd == "correct" {
                (
                    a.opt_value_from_str("--dx")?,
                    a.opt_value_from_str("--dy")?,
                    a.opt_value_from_str("--rotation")?,
                )
            } else {
                (None, None, None)
            };
            check(a)?;
            let mut p = Project::load(&path)?;
            let mut e = p.edits.get(&reference).cloned().unwrap_or_default();
            if let Some(id) = id {
                e.lcsc = Some(id);
            }
            if let Some(dnp) = dnp {
                e.dnp = Some(dnp);
            }
            if let Some(dx) = dx {
                e.dx = dx;
            }
            if let Some(dy) = dy {
                e.dy = dy;
            }
            if let Some(r) = rotation {
                e.rotation = r;
            }
            p.edit(&reference, e)?;
            p.save(&path)?;
            println!("{}", serde_json::to_string_pretty(&p)?);
        }
        "export" => {
            let path: PathBuf = a.free_from_str()?;
            let out: PathBuf = a.value_from_str("--out")?;
            let missing = a.contains("--allow-missing");
            let gerbers = a.contains("--gerbers");
            check(a)?;
            let r = export::export(&Project::load(&path)?, &out, missing, gerbers)?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        "write-board" => {
            let path: PathBuf = a.free_from_str()?;
            let out: PathBuf = a.value_from_str("--out")?;
            check(a)?;
            Project::load(&path)?.write_board_copy(&out)?;
            println!("{}", out.display());
        }
        "search" => {
            let query: String = a.free_from_str()?;
            let catalog: Option<PathBuf> = a.opt_value_from_str("--catalog")?;
            let basic = a.contains("--basic");
            let stock = a.contains("--in-stock");
            let page = a.opt_value_from_str("--page")?.unwrap_or(1);
            check(a)?;
            let r = if let Some(c) = catalog {
                supplier::search_catalog(&c, &query, basic, stock)?
            } else {
                supplier::search(&query, page, basic, stock)?
            };
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        "fetch" => {
            let id: String = a.free_from_str()?;
            let out: PathBuf = a.value_from_str("--out")?;
            check(a)?;
            let b = supplier::fetch(&id)?;
            chinamaxxbom_core::project::write_new(&out, &serde_json::to_vec_pretty(&b)?)?;
            println!("{}", out.display());
        }
        "bundle" => {
            let id: String = a.free_from_str()?;
            let symbol: PathBuf = a.value_from_str("--symbol")?;
            let fp: PathBuf = a.value_from_str("--footprint")?;
            let out: PathBuf = a.value_from_str("--out")?;
            check(a)?;
            let b = Bundle {
                lcsc: chinamaxxbom_core::lcsc_id(&id)?,
                symbols: vec![serde_json::from_slice(&std::fs::read(symbol)?)?],
                footprint: serde_json::from_slice(&std::fs::read(fp)?)?,
            };
            Library::parse(&b)?;
            chinamaxxbom_core::project::write_new(&out, &serde_json::to_vec_pretty(&b)?)?;
            println!("{}", out.display());
        }
        "import" => {
            let bundle: Option<PathBuf> = a.opt_value_from_str("--bundle")?;
            let id: Option<String> = a.opt_value_from_str("--lcsc")?;
            let out: PathBuf = a.value_from_str("--out")?;
            let target: String = a.opt_value_from_str("--target")?.unwrap_or("both".into());
            check(a)?;
            let target = match target.as_str() {
                "both" => Target::Both,
                "kicad" => Target::Kicad,
                "eagle" => Target::Eagle,
                _ => bail!("Target must be both, kicad, or eagle"),
            };
            ensure!(
                bundle.is_some() != id.is_some(),
                "Provide exactly one of --bundle or --lcsc"
            );
            let b = if let Some(p) = bundle {
                serde_json::from_slice(&std::fs::read(p)?)?
            } else {
                supplier::fetch(&id.context("Missing LCSC ID")?)?
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&Library::parse(&b)?.write(&out, target)?)?
            );
        }
        _ => bail!("Unknown command {cmd}. Use --help."),
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

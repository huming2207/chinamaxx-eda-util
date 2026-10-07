use chinamaxxbom_core::{
    board::{self, Side},
    export,
    library::{Library, Target},
    project::{Edit, Project},
    sexpr,
    supplier::{self, Bundle},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}
fn bundle(id: &str) -> Bundle {
    Bundle {
        lcsc: id.into(),
        symbols: vec![serde_json::from_slice(
            &std::fs::read(fixture(&format!("{id}_symbol.json"))).unwrap(),
        )
        .unwrap()],
        footprint: serde_json::from_slice(
            &std::fs::read(fixture(&format!("{id}_footprint.json"))).unwrap(),
        )
        .unwrap(),
    }
}
#[test]
fn equivalent_eagle_kicad_assembly() {
    let a = board::load(&fixture("assembly.brd")).unwrap();
    let b = board::load(&fixture("assembly.kicad_pcb")).unwrap();
    assert_eq!(a.parts.len(), 3);
    assert_eq!(b.parts.len(), 3);
    for (a, b) in a.parts.iter().zip(&b.parts) {
        assert_eq!(
            (&a.reference, a.x, a.y, a.rotation, a.side, a.dnp),
            (&b.reference, b.x, b.y, b.rotation, b.side, b.dnp)
        );
    }
    let r2 = a.parts.iter().find(|p| p.reference == "R2").unwrap();
    assert_eq!(r2.side, Side::Bottom);
    assert_eq!(r2.rotation, 150.);
    let a = export::assembly(&a, false).unwrap();
    let b = export::assembly(&b, false).unwrap();
    assert_eq!(a.bom, b.bom);
    assert_eq!(a.cpl, b.cpl);
    assert_eq!(a.included, 2);
    assert!(a.bom.contains("\"R1,R2\""));
    assert!(!a.cpl.contains("C1"));
}
#[test]
fn missing_lcsc_requires_explicit_permission() {
    let mut b = board::load(&fixture("assembly.brd")).unwrap();
    b.parts[0].dnp = false;
    assert!(export::assembly(&b, false).is_err());
    let r = export::assembly(&b, true).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("missing LCSC")));
}
#[test]
fn stale_project_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let b = d.path().join("test.brd");
    std::fs::copy(fixture("assembly.brd"), &b).unwrap();
    let p = Project::new(&b).unwrap();
    std::fs::write(&b, "changed").unwrap();
    assert!(p
        .snapshot()
        .unwrap_err()
        .to_string()
        .contains("Board changed"));
}
#[test]
fn edits_are_validated_and_rollback() {
    let mut p = Project::new(&fixture("assembly.brd")).unwrap();
    assert!(p.edit("NOPE", Edit::default()).is_err());
    assert!(p.edits.is_empty());
    assert!(p
        .edit(
            "R1",
            Edit {
                lcsc: Some("garbage".into()),
                ..Default::default()
            }
        )
        .is_err());
    assert!(p.edits.is_empty());
    p.edit(
        "R2",
        Edit {
            dx: 1.,
            dy: -2.,
            rotation: 240.,
            ..Default::default()
        },
    )
    .unwrap();
    let b = p.snapshot().unwrap();
    let p = b.parts.iter().find(|p| p.reference == "R2").unwrap();
    assert_eq!((p.x, p.y, p.rotation), (7., 6., 30.));
}
#[test]
fn nonfinite_corrections_rejected() {
    let mut p = Project::new(&fixture("assembly.brd")).unwrap();
    assert!(p
        .edit(
            "R1",
            Edit {
                dx: f64::NAN,
                ..Default::default()
            }
        )
        .is_err());
}
#[test]
fn board_copy_preserves_source_bytes() {
    for f in ["assembly.brd", "assembly.kicad_pcb"] {
        let s = std::fs::read_to_string(fixture(f)).unwrap();
        let out =
            board::assigned_copy(&s, &BTreeMap::from([("R1".into(), "C999".into())])).unwrap();
        assert_eq!(out, s.replacen("C25804", "C999", 1));
        let out =
            board::assigned_copy(&s, &BTreeMap::from([("C1".into(), "C123".into())])).unwrap();
        assert_eq!(board::parse(&out).unwrap().parts[0].lcsc, "C123");
    }
}
#[test]
fn eagle_self_closing_assignment() {
    let s="<eagle><drawing><board><elements><element name=\"U1\" package=\"P\" x=\"0\" y=\"0\"/></elements></board></drawing></eagle>";
    let r = board::assigned_copy(s, &BTreeMap::from([("U1".into(), "C1".into())])).unwrap();
    assert_eq!(board::parse(&r).unwrap().parts[0].lcsc, "C1");
}
#[test]
fn csv_escapes_quotes_and_commas() {
    let mut b = board::load(&fixture("assembly.brd")).unwrap();
    b.parts[1].value = "10k, \"special\"".into();
    let r = export::assembly(&b, false).unwrap();
    assert!(r.bom.contains("\"10k, \"\"special\"\"\""));
}
#[test]
fn source_exclusions_are_respected() {
    let s = std::fs::read_to_string(fixture("assembly.kicad_pcb"))
        .unwrap()
        .replacen(
            "(attr smd)",
            "(attr smd exclude_from_bom exclude_from_pos)",
            1,
        );
    let b = board::parse(&s).unwrap();
    let r = export::assembly(&b, false).unwrap();
    assert!(!r.bom.contains("R1"));
    assert!(!r.cpl.contains("R1"));
}
#[test]
fn duplicate_refs_rejected() {
    let s = std::fs::read_to_string(fixture("assembly.brd"))
        .unwrap()
        .replace("name=\"R2\"", "name=\"R1\"");
    assert!(board::parse(&s).is_err());
}
#[test]
fn parser_handles_escaped_unicode() {
    let n = sexpr::parse("(property \"Value\" \"µF \\\"quoted\\\" (text)\")").unwrap();
    assert_eq!(n.at(2), "µF \"quoted\" (text)");
    for s in ["(a", "(a))", "(a \"x)", "(a)(b)"] {
        assert!(sexpr::parse(s).is_err());
    }
}
#[test]
fn exports_are_non_destructive() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("out");
    let p = Project::new(&fixture("assembly.brd")).unwrap();
    export::export(&p, &out, false, false).unwrap();
    let bom = std::fs::read(out.join("BOM.csv")).unwrap();
    assert!(export::export(&p, &out, false, false).is_err());
    assert_eq!(bom, std::fs::read(out.join("BOM.csv")).unwrap());
    assert!(out.join("CPL.csv").exists());
}
#[test]
fn failure_leaves_no_output_set() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("out");
    let p = Project::new(&fixture("assembly.brd")).unwrap();
    assert!(export::export(&p, &out, false, true).is_err());
    assert!(!out.exists());
}
#[test]
fn local_search_filters_all_terms() {
    let r = supplier::search_catalog(&fixture("catalog.json"), "10k 0603", true, true).unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].lcsc, "C25804");
    assert!(
        supplier::search_catalog(&fixture("catalog.json"), "ME6217", false, true)
            .unwrap()
            .results
            .is_empty()
    );
}
#[test]
fn bad_supplier_responses_are_errors() {
    assert!(
        supplier::parse_search(serde_json::json!({"code":429,"message":"rate limited"})).is_err()
    );
    assert!(supplier::parse_search(serde_json::json!({"code":200,"data":{}})).is_err());
}
#[test]
fn real_easyeda_fixtures_export_to_both_formats() {
    for id in ["C2320", "C427602"] {
        let l = Library::parse(&bundle(id)).unwrap();
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join(id);
        let r = l.write(&out, Target::Both).unwrap();
        assert_eq!(r.pads, r.pins);
        assert_eq!(r.pads, if id == "C2320" { 3 } else { 5 });
        assert!(out.join(format!("{id}.lbr")).exists());
        let eagle = l.eagle().unwrap();
        let doc = roxmltree::Document::parse_with_options(
            &eagle,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .unwrap();
        let connects: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("connect"))
            .collect();
        assert_eq!(connects.len(), r.pins);
        let fp = sexpr::parse(&l.kicad_footprint().unwrap()).unwrap();
        assert_eq!(fp.all("pad").count(), r.pads);
    }
}
#[test]
fn exact_units_and_through_hole_radius() {
    let l = Library::parse(&bundle("C2320")).unwrap();
    let s = l.kicad_footprint().unwrap();
    let fp = sexpr::parse(&s).unwrap();
    let pad = fp.all("pad").next().unwrap();
    assert_eq!(pad.value("drill"), "0.900024");
    let size = pad.child("size").unwrap();
    assert_eq!(size.at(1), "1.524");
}
#[test]
fn unmapped_pins_fail_closed() {
    let mut b = bundle("C2320");
    let fp = &mut b.footprint;
    let shapes = fp["dataStr"]["shape"].as_array_mut().unwrap();
    let index = shapes
        .iter()
        .position(|s| s.as_str().unwrap().starts_with("PAD~"))
        .unwrap();
    shapes.remove(index);
    assert!(Library::parse(&b).is_err());
}
#[test]
fn polygon_pads_fail_closed() {
    let mut b = bundle("C2320");
    let fp = &mut b.footprint;
    let shapes = fp["dataStr"]["shape"].as_array_mut().unwrap();
    for s in shapes {
        if let Some(t) = s.as_str() {
            if t.starts_with("PAD~") {
                *s = serde_json::Value::String(t.replacen("ELLIPSE", "POLYGON", 1));
                break;
            }
        }
    }
    assert!(Library::parse(&b)
        .unwrap_err()
        .to_string()
        .contains("Unsupported pad"));
}
#[test]
fn part_ids_are_strict() {
    assert_eq!(chinamaxxbom_core::lcsc_id(" c123 ").unwrap(), "C123");
    for v in ["C", "123", "C1/../../x", "C１２３"] {
        assert!(chinamaxxbom_core::lcsc_id(v).is_err());
    }
}

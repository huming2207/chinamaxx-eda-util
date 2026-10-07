use chinamaxxbom_core::{
    export, input,
    project::{Edit, Project},
};
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}
fn eagle(dir: &Path, board: bool) -> PathBuf {
    let path = dir.join("assembly.sch");
    std::fs::copy(fixture("assembly.sch"), &path).unwrap();
    if board {
        std::fs::copy(fixture("assembly.brd"), dir.join("assembly.brd")).unwrap();
    }
    path
}
#[test]
fn schematic_picker_autosaves_and_resumes_preserving_other_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let sch = eagle(tmp.path(), true);
    let (mut p, sidecar) = Project::open(&sch).unwrap();
    p.edit(
        "R1",
        Edit {
            dx: 0.2,
            ..Default::default()
        },
    )
    .unwrap();
    p.pick(&sidecar, "R1", "C1234").unwrap();
    drop(p);
    // Leftover files from interrupted writes never replace the last committed JSON.
    std::fs::write(tmp.path().join(".tmp-interrupted-save"), "{broken").unwrap();
    let (p, resumed) = Project::open(&sch).unwrap();
    assert_eq!(sidecar, resumed);
    assert_eq!(p.edits["R1"].lcsc.as_deref(), Some("C1234"));
    assert_eq!(p.edits["R1"].dx, 0.2);
    assert!(std::fs::read_to_string(sidecar)
        .unwrap()
        .contains("\n  \"version\": 2,"));
    assert!(
        p.snapshot()
            .unwrap()
            .parts
            .iter()
            .find(|p| p.reference == "R1")
            .unwrap()
            .placed
    );
}
#[test]
fn failed_pick_does_not_change_saved_or_in_memory_assignment() {
    let tmp = tempfile::tempdir().unwrap();
    let sch = eagle(tmp.path(), false);
    let (mut p, sidecar) = Project::open(&sch).unwrap();
    p.pick(&sidecar, "R1", "C1234").unwrap();
    let before = std::fs::read(&sidecar).unwrap();
    assert!(p
        .pick(&tmp.path().join("missing/save.json"), "R1", "C999")
        .is_err());
    assert_eq!(p.edits["R1"].lcsc.as_deref(), Some("C1234"));
    assert!(p.pick(&sidecar, "NO_SUCH_PART", "C999").is_err());
    assert_eq!(std::fs::read(sidecar).unwrap(), before);
}
#[test]
fn project_resolves_schematic_and_exports_requested_csv_columns() {
    let tmp = tempfile::tempdir().unwrap();
    eagle(tmp.path(), true);
    let epf = tmp.path().join("assembly.epf");
    std::fs::write(&epf, "[Eagle]\nVersion=9\n").unwrap();
    let p = Project::new(tmp.path()).unwrap();
    assert_eq!(p.input.as_deref(), Some(epf.as_path()));
    let out = tmp.path().join("production");
    let result = export::export(&p, &out, false, false).unwrap();
    assert_eq!(
        result.bom,
        "Comment,Designator,Footprint,LCSC,Quantity\r\n10k,\"R1,R2\",R_0603,C25804,2\r\n"
    );
    assert_eq!(result.cpl, "Designator,Val,Package,Mid X,Mid Y,Rotation,Layer\r\nR1,10k,R_0603,2.0,3.0,90.0,top\r\nR2,10k,R_0603,6.0,8.0,150.0,bottom\r\n");
    assert!(out.join("CPL.csv").exists());
}
#[test]
fn schematic_only_exports_bom_without_fake_cpl() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Project::new(&eagle(tmp.path(), false)).unwrap();
    assert!(!p.has_placement());
    assert!(p.snapshot().unwrap().parts.iter().all(|p| !p.placed));
    let out = tmp.path().join("production");
    export::export(&p, &out, false, false).unwrap();
    assert!(out.join("BOM.csv").exists());
    assert!(!out.join("CPL.csv").exists());
}
#[test]
fn missing_layout_part_blocks_incomplete_cpl_and_source_changes_preserve_picks() {
    let tmp = tempfile::tempdir().unwrap();
    let sch = eagle(tmp.path(), true);
    let layout = tmp.path().join("assembly.brd");
    let text = std::fs::read_to_string(&layout)
        .unwrap()
        .replace("name=\"R2\"", "name=\"R3\"");
    std::fs::write(&layout, text).unwrap();
    let (mut p, sidecar) = Project::open(&sch).unwrap();
    p.pick(&sidecar, "R1", "C123").unwrap();
    let out = tmp.path().join("production");
    assert!(export::export(&p, &out, false, false)
        .unwrap_err()
        .to_string()
        .contains("R2"));
    assert!(!out.exists());
    let saved = std::fs::read(&sidecar).unwrap();
    std::fs::write(&sch, "changed").unwrap();
    assert!(Project::open(&sch).is_err());
    assert_eq!(std::fs::read(sidecar).unwrap(), saved);
}
#[test]
fn netlist_properties_keep_bom_exclusion_dnp_and_fields() {
    let board = input::parse_kicad_netlist(r#"<export><components>
      <comp ref="U1"><value>Dual opamp</value><footprint>Package_SO:SOIC-8</footprint><fields><field name="LCSC">C123</field></fields><units><unit name="A"/><unit name="B"/></units></comp>
      <comp ref="R1"><value>10k</value><property name="dnp"/></comp>
      <comp ref="TP1"><property name="exclude_from_bom"/><property name="exclude_from_board"/></comp>
    </components></export>"#).unwrap();
    assert_eq!(board.parts.len(), 3);
    assert_eq!(board.parts[0].lcsc, "C123");
    assert!(!board.parts[0].placed);
    assert!(board.parts[1].dnp);
    assert!(board.parts[2].exclude_bom && board.parts[2].exclude_cpl);
}

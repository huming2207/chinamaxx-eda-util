use std::{path::Path, process::Command};
#[test]
fn cli_round_trip_and_invalid_argument_protection() {
    let binary = env!("CARGO_BIN_EXE_chinamaxxbom");
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project.json");
    let board = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/assembly.brd");
    let run = |args: &[&str]| Command::new(binary).args(args).output().unwrap();
    let p = project.to_str().unwrap();
    let b = board.to_str().unwrap();
    assert!(!run(&["init", b, "--project", p, "--typo"]).status.success());
    assert!(!project.exists());
    assert!(run(&["init", b, "--project", p]).status.success());
    assert!(run(&["assign", p, "R1", "c123"]).status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&project).unwrap()).unwrap();
    assert_eq!(json["edits"]["R1"]["lcsc"], "C123");
    let previous = std::fs::read(&project).unwrap();
    assert!(!run(&["assign", p, "R1", "C99", "--typo"]).status.success());
    assert_eq!(previous, std::fs::read(&project).unwrap());
    let out = tmp.path().join("assembly");
    assert!(run(&["export", p, "--out", out.to_str().unwrap()])
        .status
        .success());
    assert!(out.join("CPL.csv").exists());
}

#[cfg(unix)]
#[test]
fn picker_calls_api_once_then_reuses_week_cache() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let curl = tmp.path().join("curl");
    let calls = tmp.path().join("calls");
    std::fs::write(&curl,r#"#!/bin/sh
/bin/cat >/dev/null
printf 'call\n' >> "$FAKE_CURL_CALLS"
printf '%s' '{"code":200,"data":{"componentPageInfo":{"total":1,"list":[{"componentCode":"C25804","componentName":"10k resistor","componentLibraryType":"base","stockCount":123}]}}}'
"#).unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let invoke = |query: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_chinamaxxbom"))
            .args(["search", query, "--basic", "--in-stock"])
            .env("PATH", tmp.path())
            .env("CHINAMAXXBOM_CACHE_DIR", tmp.path().join("cache"))
            .env("FAKE_CURL_CALLS", &calls)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let first = invoke(" 10K   0603 ");
    let second = invoke("10k 0603");
    assert_eq!(first["cache"]["hit"], false);
    assert_eq!(second["cache"]["hit"], true);
    assert_eq!(
        second["cache"]["expires_at"].as_u64().unwrap()
            - second["cache"]["fetched_at"].as_u64().unwrap(),
        604800
    );
    let setting = Command::new(env!("CARGO_BIN_EXE_chinamaxxbom"))
        .args(["cache-days", "60"])
        .env("CHINAMAXXBOM_CACHE_DIR", tmp.path().join("cache"))
        .output()
        .unwrap();
    assert!(setting.status.success());
    let third = invoke("10k 0603");
    assert_eq!(third["cache"]["hit"], true);
    assert_eq!(
        third["cache"]["expires_at"].as_u64().unwrap()
            - third["cache"]["fetched_at"].as_u64().unwrap(),
        60 * 86400
    );
    assert_eq!(second["results"][0]["lcsc"], "C25804");
    assert_eq!(std::fs::read_to_string(calls).unwrap(), "call\n");
}

#[test]
fn cache_settings_persist_and_reject_out_of_range_values() {
    let tmp = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_chinamaxxbom"))
            .args(args)
            .env("CHINAMAXXBOM_CACHE_DIR", tmp.path())
            .output()
            .unwrap()
    };
    assert_eq!(run(&["cache-days"]).stdout, b"7\n");
    for days in ["7", "30", "60"] {
        let result = run(&["cache-days", days]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(run(&["cache-days"]).stdout, format!("{days}\n").as_bytes());
    }
    for days in ["0", "6", "61", "invalid"] {
        assert!(!run(&["cache-days", days]).status.success());
        assert_eq!(run(&["cache-days"]).stdout, b"60\n");
    }
}

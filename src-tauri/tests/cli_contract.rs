// 격리된 데이터 디렉터리에서 CLI 공개 명령을 검증한다.
use serde_json::{json, Value};
use std::{
    path::Path,
    process::{Command, Output},
};
use tempfile::tempdir;
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_devdock-cli"))
        .env("DEVDOCK_DATA_DIR", dir)
        .args(args)
        .output()
        .unwrap()
}
fn ok(dir: &Path, args: &[&str]) -> Value {
    let o = run(dir, args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true);
    v["data"].clone()
}
fn bad(dir: &Path, args: &[&str]) {
    let o = run(dir, args);
    assert!(!o.status.success());
    let v: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(v["ok"], false);
}
#[test]
fn cli_project_monitor_and_progress_mutations() {
    let d = tempdir().unwrap();
    let p = d.path();
    bad(p, &["project", "add", "{bad"]);
    ok(
        p,
        &[
            "project",
            "add",
            &json!({"id":"p","name":"Project","root":"."}).to_string(),
        ],
    );
    ok(
        p,
        &[
            "project",
            "update",
            &json!({"id":"p","name":"Updated","root":"."}).to_string(),
        ],
    );
    assert_eq!(ok(p, &["project", "list"])[0]["name"], "Updated");
    ok(p,&["monitor","add",&json!({"id":"m","name":"Monitor","kind":"checklist","projectId":"p","source":"missing.md","enabled":true}).to_string()]);
    ok(p,&["monitor","update",&json!({"id":"m","name":"Changed","kind":"checklist","projectId":"p","source":"missing.md","enabled":true}).to_string()]);
    assert!(ok(p, &["monitor", "list"])
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "m" && m["name"] == "Changed"));
    ok(p,&["task","upsert",&json!({"id":"t","projectId":"p","title":"T","state":"reportedComplete","revision":"1"}).to_string()]);
    ok(p,&["evidence","add",&json!({"id":"e","taskId":"t","kind":"test","success":true,"subjectRevision":"1","reference":"test"}).to_string()]);
    ok(
        p,
        &[
            "status",
            "publish",
            &json!({"projectId":"p","currentTask":"Review"}).to_string(),
        ],
    );
    assert_eq!(ok(p, &["snapshot"])["projects"][0]["verified"], 1);
    ok(p, &["task", "archive", "t"]);
    assert_eq!(ok(p, &["snapshot"])["projects"][0]["total"], 0);
    ok(p, &["monitor", "remove", "m"]);
    ok(p, &["project", "remove", "p"]);
    assert!(ok(p, &["snapshot"])["projects"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn cli_settings_history_event_and_validation() {
    let d = tempdir().unwrap();
    let p = d.path();
    let mut settings = ok(p, &["settings"]);
    settings["retentionDays"] = json!(14);
    ok(p, &["settings", &settings.to_string()]);
    assert_eq!(ok(p, &["settings"])["retentionDays"], 14);
    let mut autostart = ok(p, &["settings"]);
    autostart["autostartEnabled"] = json!(true);
    bad(p, &["settings", &autostart.to_string()]);
    assert_eq!(ok(p, &["settings"])["autostartEnabled"], false);
    ok(p, &["validate"]);
    let mut invalid = settings;
    invalid["startTime"] = json!("25:00");
    bad(p, &["settings", &invalid.to_string()]);
    assert_eq!(ok(p, &["settings"])["startTime"], "08:10");
    bad(p, &["history", "A", "nope"]);
    assert_eq!(ok(p, &["history", "A", "24"])["monitorId"], "A");
    bad(
        p,
        &[
            "event",
            &json!({"daemonId":"bad","method":"turn/started"}).to_string(),
        ],
    );
    ok(
        p,
        &[
            "event",
            &json!({"id":"ev","daemonId":"A","method":"turn/started","at":"2026-11-17T10:00:00Z"})
                .to_string(),
        ],
    );
    assert_eq!(ok(p, &["snapshot"])["events"][0]["id"], "ev");
    std::fs::write(p.join("registry.json"), "{").unwrap();
    bad(p, &["validate"]);
}

#[test]
fn cli_markdown_evidence_revision_expires_after_edit() {
    let d = tempdir().unwrap();
    let p = d.path();
    let path = p.join("tasks.md");
    std::fs::write(&path, "- [x] T01. Done\n").unwrap();
    ok(
        p,
        &[
            "project",
            "add",
            &json!({"id":"p","name":"P","root":p,"checklistPath":"tasks.md"}).to_string(),
        ],
    );
    let s = ok(p, &["snapshot"]);
    let rev = s["projects"][0]["tasks"][0]["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(p,&["evidence","add",&json!({"id":"ev","taskId":"T01","kind":"test","success":true,"subjectRevision":rev,"reference":"test"}).to_string()]);
    assert_eq!(ok(p, &["snapshot"])["projects"][0]["verified"], 1);
    std::fs::write(path, "- [x] T01. Updated\n").unwrap();
    let p = ok(p, &["snapshot"]);
    assert_eq!(p["projects"][0]["verified"], 0);
    assert_eq!(p["projects"][0]["reportedComplete"], 1);
}

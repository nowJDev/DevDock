// CLI와 네이티브 문서 명령의 동일한 저장·검증 경계 및 읽기 전용 모드를 확인한다.
use devdock_core::service::Engine;
use serde_json::{json, Value};
fn cli(d: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_devdock-cli"))
        .env("DEVDOCK_DATA_DIR", d)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn cli_uses_same_revision_rules_and_large_json_input_file() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().join("data")).unwrap();
    let mut cfg = e.knowledge().get_document_config().unwrap();
    cfg["nasIntervalSec"] = json!(60);
    let input = d.path().join("입력 파일.json");
    std::fs::write(
        &input,
        json!({"config":cfg,"expectedRevision":0}).to_string(),
    )
    .unwrap();
    let output = cli(
        e.data_dir(),
        &[
            "documents",
            "save-config",
            "--input",
            input.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["revision"], 1);
    assert_eq!(
        e.knowledge().get_document_config().unwrap()["nasIntervalSec"],
        60
    );
    assert!(!cli(
        e.data_dir(),
        &[
            "documents",
            "save-config",
            "--input",
            input.to_str().unwrap()
        ]
    )
    .status
    .success());
}
#[test]
fn invalid_input_and_missing_revision_have_nonzero_exit() {
    let d = tempfile::tempdir().unwrap();
    let output = cli(d.path(), &["documents", "save-config", r#"{"config":{}}"#]);
    assert!(!output.status.success());
    let output = cli(
        d.path(),
        &["documents", "config", r#"{"executeCommand":"forbidden"}"#],
    );
    assert!(!output.status.success());
}
#[test]
fn preview_refuses_writes_and_read_never_applies_import() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let input = json!({"command":"save_document_config","config":e.knowledge().get_document_config().unwrap(),"expectedRevision":0});
    assert!(e
        .knowledge()
        .execute_command(input.clone(), true, true)
        .is_err());
    let output = cli(d.path(), &["knowledge-readonly", &input.to_string()]);
    assert!(!output.status.success());
    let v = e
        .knowledge()
        .execute_command(json!({"command":"get_document_config"}), true, true)
        .unwrap();
    assert_eq!(v["revision"], 0);
    let db = rusqlite::Connection::open(d.path().join("devdock.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

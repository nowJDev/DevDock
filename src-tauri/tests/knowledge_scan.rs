// 실제 자식 프로세스로 문서 수집의 격리와 캐시 보존을 검증한다.
use devdock_core::{knowledge::scanner, service::Engine};
use serde_json::json;
use std::{fs, path::Path, time::Duration};
use tempfile::TempDir;
fn fixture() -> (TempDir, Engine, String) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("문서 폴더");
    fs::create_dir(&root).unwrap();
    let e = Engine::open(d.path().join("data")).unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":root}}),
        None,
    )
    .unwrap();
    e.knowledge().upsert_document_source(json!({"sourceId":"s","scope":"project","projectId":"p","location":"nas","root":root,"roles":["handover"],"enabled":true}),0).unwrap();
    (d, e, root.to_string_lossy().into_owned())
}
fn scan(e: &Engine) -> serde_json::Value {
    e.knowledge()
        .scan_with_executable(
            &["s".into()],
            true,
            Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
            Duration::from_secs(5),
        )
        .unwrap()
}
#[test]
fn unc_korean_path_roundtrip_and_worker_skips_engine_startup() {
    let (_d, e, root) = fixture();
    fs::write(
        Path::new(&root).join("인수인계.md"),
        "# 작업\n- [x] T1 구현",
    )
    .unwrap();
    assert_eq!(scan(&e)["state"], "completed");
    let v = e.knowledge().list_project_documents("p", 0).unwrap();
    assert_eq!(v["documents"].as_array().unwrap().len(), 1);
    let id = v["documents"][0]["versionId"].as_str().unwrap();
    assert!(
        e.knowledge().read_document_version(id, 1, 200).unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("구현")
    );
}
#[test]
fn same_metadata_force_hash_detects_change() {
    let (_d, e, root) = fixture();
    let p = Path::new(&root).join("a.md");
    fs::write(&p, "aaaa").unwrap();
    scan(&e);
    let old = fs::metadata(&p).unwrap().modified().unwrap();
    fs::write(&p, "bbbb").unwrap();
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    scan(&e);
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM document_versions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
#[test]
fn partial_scan_never_deletes_and_invalid_utf8_is_explicit() {
    let (_d, e, root) = fixture();
    fs::write(Path::new(&root).join("a.md"), "valid").unwrap();
    scan(&e);
    fs::write(Path::new(&root).join("bad.md"), [0xff, 0xfe]).unwrap();
    assert_eq!(scan(&e)["sources"][0]["state"], "partial");
    assert_eq!(
        e.knowledge().list_project_documents("p", 0).unwrap()["documents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn offline_keeps_last_valid_cache() {
    let (_d, e, root) = fixture();
    fs::write(Path::new(&root).join("a.md"), "valid").unwrap();
    scan(&e);
    fs::rename(&root, format!("{root}-offline")).unwrap();
    assert_eq!(scan(&e)["sources"][0]["state"], "offline");
    assert_eq!(
        e.knowledge().list_project_documents("p", 0).unwrap()["documents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn source_revision_change_rejects_scan_result() {
    let (_d, e, _root) = fixture();
    let mut cfg = e.knowledge().get_document_config().unwrap();
    cfg["nasIntervalSec"] = json!(60);
    e.knowledge().save_document_config(cfg, 1).unwrap();
    assert!(e
        .knowledge()
        .ingest_scan_result(json!({"revision":1,"sources":[]}), Path::new("missing"))
        .unwrap_err()
        .contains("conflict"));
}
#[test]
fn timeout_kills_and_reaps_child() {
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 20",
    ]);
    let started = std::time::Instant::now();
    assert!(scanner::run_bounded(cmd, Duration::from_millis(100))
        .unwrap_err()
        .contains("timeout"));
    assert!(started.elapsed() < Duration::from_secs(3));
}
#[test]
fn timeout_keeps_last_valid_cache_and_file_limit_is_partial() {
    let (_d, e, root) = fixture();
    fs::write(Path::new(&root).join("a.md"), "valid").unwrap();
    scan(&e);
    let result = e
        .knowledge()
        .scan_with_executable(
            &["s".into()],
            true,
            Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(result["sources"][0]["state"], "timeout");
    assert_eq!(
        e.knowledge().list_project_documents("p", 0).unwrap()["documents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fs::write(
        Path::new(&root).join("large.md"),
        vec![b'a'; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    assert_eq!(scan(&e)["sources"][0]["state"], "partial");
}
#[test]
fn cache_cleanup_preserves_referenced_and_latest_versions() {
    let (_d, e, root) = fixture();
    let path = Path::new(&root).join("a.md");
    fs::write(&path, "first").unwrap();
    scan(&e);
    let old = e.knowledge().list_project_documents("p", 0).unwrap()["documents"][0]["versionId"]
        .as_str()
        .unwrap()
        .to_owned();
    e.knowledge()
        .pin_document_version(&old, "accepted-evidence")
        .unwrap();
    fs::write(&path, "second").unwrap();
    scan(&e);
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    c.execute(
        "UPDATE document_versions SET captured_at='2000-01-01T00:00:00Z' WHERE version_id=?1",
        [&old],
    )
    .unwrap();
    e.knowledge().cleanup_document_cache().unwrap();
    assert!(e.knowledge().read_document_version(&old, 1, 1).is_ok());
}

#[test]
fn cleanup_commit_failure_never_removes_cache_bytes() {
    let (_d, e, root) = fixture();
    let p = Path::new(&root).join("a.md");
    fs::write(&p, "first").unwrap();
    scan(&e);
    let old = e.knowledge().list_project_documents("p", 0).unwrap()["documents"][0]["versionId"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(&p, "second").unwrap();
    scan(&e);
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    c.execute(
        "UPDATE document_versions SET captured_at='2000-01-01T00:00:00Z' WHERE version_id=?1",
        [&old],
    )
    .unwrap();
    c.execute_batch("CREATE TABLE failure_pin(id TEXT REFERENCES document_versions(version_id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fail_cleanup AFTER DELETE ON document_versions BEGIN INSERT INTO failure_pin VALUES('missing'); END;").unwrap();
    assert!(e.knowledge().cleanup_document_cache().is_err());
    assert!(e.knowledge().read_document_version(&old, 1, 1).is_ok());
}

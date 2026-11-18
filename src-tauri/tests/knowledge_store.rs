// 문서 관리 저장의 이행·충돌·출처 경계를 검증합니다.
use devdock_core::service::Engine;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn migrate_preserves_legacy_bytes_and_runs() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let settings = std::fs::read(d.path().join("settings.json")).unwrap();
    let registry = std::fs::read(d.path().join("registry.json")).unwrap();
    let run = e.claim_run("2026-01-01", "checkIn").unwrap().unwrap();
    e.finish_run(&run, json!({"state":"recorded"})).unwrap();
    let k = e.knowledge();
    assert_eq!(k.get_document_config().unwrap()["nasIntervalSec"], 30);
    drop(k);
    let e = Engine::connect(d.path().into()).unwrap();
    assert_eq!(e.knowledge().get_document_config().unwrap()["revision"], 0);
    assert_eq!(
        std::fs::read(d.path().join("settings.json")).unwrap(),
        settings
    );
    assert_eq!(
        std::fs::read(d.path().join("registry.json")).unwrap(),
        registry
    );
    let c = rusqlite::Connection::open(d.path().join("devdock.sqlite3")).unwrap();
    assert_eq!(
        c.query_row("SELECT state FROM runs", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "recorded"
    );
}

#[test]
fn document_config_cas_conflict() {
    let d = tempdir().unwrap();
    let k = Engine::open(d.path().into()).unwrap().knowledge();
    let mut config = k.get_document_config().unwrap();
    config["nasIntervalSec"] = json!(60);
    assert_eq!(
        k.save_document_config(config.clone(), 0).unwrap()["revision"],
        1
    );
    assert!(k
        .save_document_config(config, 0)
        .unwrap_err()
        .contains("conflict"));
    assert_eq!(k.get_document_config().unwrap()["nasIntervalSec"], 60);
}

#[test]
fn source_scope_requires_project_and_preserves_multiple_roots() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let k = e.knowledge();
    let source = json!({"sourceId":"one","scope":"project","projectId":"missing","location":"nas","root":"\\\\fileserver\\docs\\Project","roles":["dailyLog"],"enabled":true});
    assert!(k.upsert_document_source(source, 0).is_err());
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":d.path()}}),
        None,
    )
    .unwrap();
    for (i, path) in [
        "\\\\fileserver\\docs\\Project",
        "\\\\fileserver\\docs\\Other",
    ]
    .iter()
    .enumerate()
    {
        k.upsert_document_source(json!({"sourceId":format!("s{i}"),"scope":"project","projectId":"p","location":"nas","root":path,"roles":["dailyLog"],"enabled":true}),i as u64).unwrap();
    }
    assert_eq!(
        k.get_document_config().unwrap()["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(k.get_document_config().unwrap()["revision"], 2);
}

#[test]
fn invalid_intervals_and_unknown_settings_are_rejected() {
    let d = tempdir().unwrap();
    let k = Engine::open(d.path().into()).unwrap().knowledge();
    let mut config = k.get_document_config().unwrap();
    config["nasIntervalSec"] = json!(0);
    assert!(k.save_document_config(config, 0).is_err());
    let mut config = k.get_document_config().unwrap();
    config["executeCommand"] = json!("forbidden");
    assert!(k.save_document_config(config, 0).is_err());
}

#[test]
fn migrations_are_idempotent_and_constraints_reject_invalid_data() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    assert!(e.claim_prompt("2026-01-02").unwrap());
    Engine::connect(d.path().into()).unwrap();
    Engine::connect(d.path().into()).unwrap();
    let c = rusqlite::Connection::open(d.path().join("devdock.sqlite3")).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM prompts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(c
        .execute("UPDATE document_config SET revision=-1", [])
        .is_err());
    assert!(c.execute("INSERT INTO document_versions VALUES('v','missing','hash','cache',0,'now',NULL,'reference','{}')",[]).is_err());
    assert!(c
        .execute(
            "INSERT INTO project_progress_control VALUES('p',0,'illegal','{}')",
            []
        )
        .is_err());
    assert_eq!(c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('document_config','document_sources','documents','document_versions','document_project_links','progress_bases','basis_tasks','basis_task_events','project_progress_control','analysis_jobs','import_candidates','import_batches','import_batch_items','project_work_entries','project_progress_points','project_report_profiles','report_requests','document_publish_jobs','document_publish_items')",[],|r|r.get::<_,i64>(0)).unwrap(),19);
}

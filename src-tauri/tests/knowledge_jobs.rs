// 분석 요청의 배타 lease와 출처·revision 검증을 확인한다.
use devdock_core::service::Engine;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
fn fixture() -> (tempfile::TempDir, Engine, String) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("docs");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("a.md"),
        "# 기록\n2026-01-01\n## 결과\n- 메뉴 구현",
    )
    .unwrap();
    let e = Engine::open(d.path().join("data")).unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":root}}),
        None,
    )
    .unwrap();
    e.knowledge().upsert_document_source(json!({"sourceId":"s","scope":"project","projectId":"p","location":"local","root":root,"roles":["handover"],"enabled":true}),0).unwrap();
    e.knowledge()
        .scan_with_executable(
            &["s".into()],
            true,
            Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
            Duration::from_secs(5),
        )
        .unwrap();
    let id = e.knowledge().list_project_documents("p", 0).unwrap()["documents"][0]["versionId"]
        .as_str()
        .unwrap()
        .to_owned();
    (d, e, id)
}
#[test]
fn analysis_claim_is_exclusive() {
    let (_d, e, id) = fixture();
    let k = e.knowledge();
    let job = k
        .create_analysis_job(json!({"projectId":"p","inputVersionIds":[id]}))
        .unwrap();
    let j = job["jobId"].as_str().unwrap();
    let claim = k.claim_analysis_job(j, "codex").unwrap();
    assert!(claim["leaseToken"].is_string());
    assert!(k.claim_analysis_job(j, "other").is_err());
    assert!(k.heartbeat_analysis_job(j, "wrong").is_err());
    assert!(k
        .heartbeat_analysis_job(j, claim["leaseToken"].as_str().unwrap())
        .is_ok());
}
#[test]
fn archive_duplicate_is_not_extra_work() {
    let (_d, e, id) = fixture();
    let k = e.knowledge();
    let job = k
        .create_analysis_job(json!({"projectId":"p","inputVersionIds":[id.clone(),id]}))
        .unwrap();
    assert_eq!(job["inputVersionIds"].as_array().unwrap().len(), 1);
}
#[test]
fn stale_analysis_result_requires_review() {
    let (_d, e, id) = fixture();
    let k = e.knowledge();
    let job = k
        .create_analysis_job(json!({"projectId":"p","inputVersionIds":[id]}))
        .unwrap();
    let j = job["jobId"].as_str().unwrap();
    let claim = k.claim_analysis_job(j, "codex").unwrap();
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    c.execute(
        "UPDATE project_progress_control SET revision=revision+1 WHERE project_id='p'",
        [],
    )
    .unwrap();
    let result = k.submit_analysis_result(
        j,
        claim["leaseToken"].as_str().unwrap(),
        json!({"candidates":[]}),
    );
    assert!(result.unwrap_err().contains("conflict"));
}
#[test]
fn analysis_rejects_foreign_sources_commands_and_mixed_job_kinds() {
    let (_d, e, id) = fixture();
    let k = e.knowledge();
    let job = k
        .create_analysis_job(json!({"projectId":"p","inputVersionIds":[id]}))
        .unwrap();
    let j = job["jobId"].as_str().unwrap();
    let claim = k.claim_analysis_job(j, "codex").unwrap();
    let token = claim["leaseToken"].as_str().unwrap();
    assert!(k
        .submit_analysis_result(
            j,
            token,
            json!({"candidates":[],"executeCommand":"echo hi"})
        )
        .is_err());
    let v: Value = json!({"candidates":[{"kind":"workEntry","workDate":"2026-01-01","sourceSpans":[{"versionId":"foreign","sha256":"bad","locator":{"lines":{"start":1,"end":2}}}],"proposedChanges":{"text":"test"}}]});
    assert!(k.submit_analysis_result(j, token, v).is_err());
    assert!(k
        .submit_analysis_result(j, token, json!({"body":"report"}))
        .is_err());
}
#[test]
fn claim_cannot_reset_sixty_minute_total_limit() {
    let (_d, e, id) = fixture();
    let k = e.knowledge();
    let job = k
        .create_analysis_job(json!({"projectId":"p","inputVersionIds":[id]}))
        .unwrap();
    let j = job["jobId"].as_str().unwrap();
    k.claim_analysis_job(j, "first").unwrap();
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    c.execute("UPDATE analysis_jobs SET payload=json_set(payload,'$.firstClaimAt','2000-01-01T00:00:00Z','$.leaseExpiresAt','2000-01-01T00:00:00Z') WHERE job_id=?1",[j]).unwrap();
    assert!(k.claim_analysis_job(j, "retry").is_err());
}

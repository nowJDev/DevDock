// 과거 반영의 원자성·중복 방지·현재 상태 분리와 의존성 있는 취소를 검증한다.
use devdock_core::service::Engine;
use serde_json::{json, Value};
fn fixture() -> (tempfile::TempDir, Engine, String) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("docs");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("a.md"),
        "# 기록\n작성일: 2026-01-17\n업무일: 2026-01-13\n## 결과\n- 메뉴 구현",
    )
    .unwrap();
    let e = Engine::open(d.path().join("data")).unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":root}}),
        None,
    )
    .unwrap();
    let k = e.knowledge();
    k.upsert_document_source(json!({"sourceId":"s","scope":"project","projectId":"p","location":"local","root":root,"roles":["handover"],"enabled":true}),0).unwrap();
    k.scan_with_executable(
        &[],
        true,
        std::path::Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    k.save_progress_basis(json!({"basisId":"dev","version":1,"projectId":"p","name":"개발","kind":"development","tasks":[{"taskId":"T1","title":"메뉴","revision":1,"state":"inProgress"}]}),0).unwrap();
    let v = k.list_project_documents("p", 0).unwrap()["documents"][0]["versionId"]
        .as_str()
        .unwrap()
        .to_owned();
    (d, e, v)
}
fn candidate(e: &Engine, version: &str, kind: &str, changes: Value) -> (String, String) {
    let k = e.knowledge();
    let job = k
        .create_analysis_job(json!({"projectId":"p","basisId":"dev","inputVersionIds":[version]}))
        .unwrap();
    let id = job["jobId"].as_str().unwrap();
    let claim = k.claim_analysis_job(id, "test").unwrap();
    let doc = k.read_document_version(version, 1, 200).unwrap();
    let preview=k.submit_analysis_result(id,claim["leaseToken"].as_str().unwrap(),json!({"candidates":[{"kind":kind,"workDate":"2026-01-13","authoredDate":"2026-01-17","effectiveDate":"2026-01-13","sourceSpans":[{"versionId":version,"sha256":doc["sha256"],"locator":{"lines":{"start":1,"end":5}}}],"proposedChanges":changes}]})).unwrap();
    let cid = preview["candidates"][0]["candidateId"].as_str().unwrap();
    k.review_import_candidate(
        id,
        cid,
        "accepted",
        None,
        "원문 확인",
        job["expectedProjectRevision"].as_i64().unwrap(),
    )
    .unwrap();
    (id.to_owned(), cid.to_owned())
}
fn apply(e: &Engine, job: &str, cid: &str, key: &str, mode: &str) -> Value {
    let rev = e.knowledge().get_project_progress("p", None).unwrap()["revision"]
        .as_i64()
        .unwrap();
    e.knowledge().apply_import(json!({"jobId":job,"candidateIds":[cid],"idempotencyKey":key,"mode":mode,"expectedProjectRevision":rev})).unwrap()
}
#[test]
fn historical_import_does_not_change_current_task_and_date_is_not_shifted() {
    let (_d, e, v) = fixture();
    let (job, c) = candidate(
        &e,
        &v,
        "workEntry",
        json!({"text":"메뉴 구현","coverage":"partial"}),
    );
    apply(&e, &job, &c, "one", "history");
    assert_eq!(
        e.knowledge()
            .get_project_progress("p", Some("dev"))
            .unwrap()["counts"]["inProgress"],
        1
    );
    let h = e
        .knowledge()
        .get_project_history("p", "dev", "2026-01-01", "2026-01-31")
        .unwrap();
    assert!(h["points"].as_array().unwrap().is_empty());
    assert_eq!(h["workEntries"][0]["workDate"], "2026-01-13");
}
#[test]
fn duplicate_apply_is_idempotent() {
    let (_d, e, v) = fixture();
    let (job, c) = candidate(&e, &v, "workEntry", json!({"text":"완료"}));
    let a = apply(&e, &job, &c, "same", "history");
    let b = apply(&e, &job, &c, "same", "history");
    assert_eq!(a["batchId"], b["batchId"]);
}
#[test]
fn undo_keeps_later_manual_change() {
    let (_d, e, v) = fixture();
    let (job, c) = candidate(
        &e,
        &v,
        "currentTaskUpdate",
        json!({"taskId":"T1","state":"reportedComplete","expectedTaskRevision":1}),
    );
    let a = apply(&e, &job, &c, "change", "current");
    e.knowledge()
        .update_progress_task(
            "p",
            "dev",
            1,
            "T1",
            json!({"state":"blocked","expectedTaskRevision":1}),
            2,
        )
        .unwrap();
    e.knowledge()
        .undo_import(a["batchId"].as_str().unwrap(), 3)
        .unwrap();
    assert_eq!(
        e.knowledge()
            .get_project_progress("p", Some("dev"))
            .unwrap()["counts"]["blocked"],
        1
    );
}
#[test]
fn undo_dependency_is_rejected() {
    let (_d, e, v) = fixture();
    let (job, c) = candidate(
        &e,
        &v,
        "basisDefinition",
        json!({"basisId":"new","version":1,"name":"신규","kind":"acceptance","tasks":[{"taskId":"U1","title":"실기기","revision":1,"state":"pending"}]}),
    );
    let a = apply(&e, &job, &c, "basis", "basis");
    e.knowledge()
        .update_progress_task(
            "p",
            "new",
            1,
            "U1",
            json!({"state":"blocked","expectedTaskRevision":1}),
            2,
        )
        .unwrap();
    assert!(e
        .knowledge()
        .undo_import(a["batchId"].as_str().unwrap(), 3)
        .unwrap_err()
        .contains("undo_dependency_conflict"));
}
#[test]
fn incomplete_snapshot_is_rejected_and_range_is_bounded() {
    let (_d, e, v) = fixture();
    let (job, c) = candidate(
        &e,
        &v,
        "historicalSnapshot",
        json!({"coverage":"partial","basisVersion":1,"counts":{"total":1,"verified":0,"reportedComplete":1}}),
    );
    let k = e.knowledge();
    assert!(k.apply_import(json!({"jobId":job,"candidateIds":[c],"idempotencyKey":"partial","mode":"history","expectedProjectRevision":1})).is_err());
    assert!(k
        .get_project_history("p", "dev", "2025-01-01", "2026-01-02")
        .is_err());
}
#[test]
fn import_failure_rolls_back_all_rows() {
    let (_d, e, v) = fixture();
    let (job, c) = candidate(&e, &v, "workEntry", json!({"text":"완료"}));
    let db = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON project_work_entries BEGIN SELECT RAISE(ABORT,'disk failure fixture'); END;").unwrap();
    assert!(e.knowledge().apply_import(json!({"jobId":job,"candidateIds":[c],"idempotencyKey":"failed","expectedProjectRevision":1})).is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        e.knowledge().get_project_progress("p", None).unwrap()["revision"],
        1
    );
}

#[test]
fn complete_snapshot_survives_retention_and_observed_is_default() {
    let (_d, e, v) = fixture();
    let (job, c) = candidate(
        &e,
        &v,
        "historicalSnapshot",
        json!({"coverage":"complete","basisVersion":1,"tasks":[{"taskId":"T1","revision":1,"state":"reportedComplete"}]}),
    );
    apply(&e, &job, &c, "snapshot", "history");
    e.maintain_history("2026-11-18T12:00:00+09:00").unwrap();
    let k = e.knowledge();
    let h = k
        .get_project_history("p", "dev", "2026-01-01", "2026-01-31")
        .unwrap();
    assert_eq!(h["points"][0]["origin"], "reconstructed");
    let db = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    let point = json!({"date":"2026-01-13","basisVersion":1,"counts":{"total":1,"verified":0,"reportedComplete":0,"inProgress":1,"blocked":0},"origin":"observed","coverage":"complete","sourceSpans":[],"capturedAt":"2026-01-13T10:00:00Z"});
    db.execute("INSERT INTO project_progress_points VALUES('observed','p','dev',1,'2026-01-13',1,0,0,'observed',NULL,?1)",[point.to_string()]).unwrap();
    let h = k
        .get_project_history("p", "dev", "2026-01-01", "2026-01-31")
        .unwrap();
    assert_eq!(h["points"][0]["origin"], "observed");
    assert_eq!(h["comparisons"].as_array().unwrap().len(), 2);
}
#[test]
fn explicit_legacy_archive_survives_old_observation_cleanup() {
    let (_d, e, _v) = fixture();
    let db = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    db.execute("INSERT INTO daily_progress VALUES('p','2026-01-01','2026-01-01T01:00:00Z',8,0,6,NULL,NULL)",[]).unwrap();
    let k = e.knowledge();
    let preview = k
        .preview_legacy_archive("p", "2026-01-01", "2026-01-31")
        .unwrap();
    k.archive_legacy_history(
        "p",
        "2026-01-01",
        "2026-01-31",
        1,
        preview["previewHash"].as_str().unwrap(),
    )
    .unwrap();
    e.maintain_history("2026-11-18T12:00:00+09:00").unwrap();
    let h = k
        .get_project_history("p", "legacy:p", "2026-01-01", "2026-01-31")
        .unwrap();
    assert_eq!(h["points"][0]["counts"]["reportedComplete"], 6);
    assert_eq!(h["points"][0]["coverage"], "legacyUnknown");
}

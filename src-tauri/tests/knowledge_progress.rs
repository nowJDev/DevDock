// 진행 기준별 집계와 근거 만료 및 관리 전환의 기존 원장 경계를 검증한다.
use devdock_core::service::Engine;
use serde_json::{json, Value};
fn fixture() -> (tempfile::TempDir, Engine) {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":d.path()}}),
        None,
    )
    .unwrap();
    (d, e)
}
fn basis(id: &str, kind: &str, n: usize, done: usize) -> Value {
    json!({"basisId":id,"version":1,"projectId":"p","kind":kind,"name":id,"authoritativeSourceIds":[],"tasks":(0..n).map(|i|json!({"taskId":format!("T{i}"),"title":format!("기능 {i}"),"revision":1,"state":if i<done{"reportedComplete"}else{"pending"}})).collect::<Vec<_>>()})
}
#[test]
fn unlinked_project_has_no_fabricated_basis() {
    let (_d, e) = fixture();
    assert!(e.knowledge().get_project_progress("p", None).unwrap()["basis"].is_null());
}
#[test]
fn summary_is_not_product_percent_and_zero_is_null() {
    let (_d, e) = fixture();
    let k = e.knowledge();
    k.save_progress_basis(basis("summary", "summary", 8, 6), 0)
        .unwrap();
    let v = k.get_project_progress("p", Some("summary")).unwrap();
    assert_eq!(v["counts"]["total"], 8);
    assert_eq!(v["reportRatio"], 0.75);
    assert_eq!(v["basis"]["kind"], "summary");
    k.save_progress_basis(basis("empty", "acceptance", 0, 0), 1)
        .unwrap();
    assert!(k.get_project_progress("p", Some("empty")).unwrap()["reportRatio"].is_null());
}
#[test]
fn development_and_uat_stay_separate() {
    let (_d, e) = fixture();
    let k = e.knowledge();
    k.save_progress_basis(basis("dev", "development", 87, 28), 0)
        .unwrap();
    k.save_progress_basis(basis("uat", "acceptance", 22, 0), 1)
        .unwrap();
    assert_eq!(
        k.get_project_progress("p", Some("dev")).unwrap()["counts"]["reportedComplete"],
        28
    );
    assert_eq!(
        k.get_project_progress("p", Some("uat")).unwrap()["counts"]["total"],
        22
    );
    assert_eq!(
        k.get_project_progress("p", Some("uat")).unwrap()["counts"]["reportedComplete"],
        0
    );
}
#[test]
fn parent_child_not_double_counted_and_cancelled_excluded() {
    let (_d, e) = fixture();
    let k = e.knowledge();
    let mut b = basis("dev", "development", 4, 4);
    b["tasks"][1]["parentId"] = json!("T0");
    b["tasks"][3]["state"] = json!("cancelled");
    k.save_progress_basis(b, 0).unwrap();
    assert_eq!(
        k.get_project_progress("p", Some("dev")).unwrap()["counts"]["total"],
        2
    );
}
#[test]
fn managed_cutover_stops_legacy_progress_writer() {
    let (_d, e) = fixture();
    let k = e.knowledge();
    k.save_progress_basis(basis("dev", "development", 2, 1), 0)
        .unwrap();
    k.activate_managed_progress("p", "dev", 1).unwrap();
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    let before: i64 = c
        .query_row("SELECT COUNT(*) FROM progress", [], |r| r.get(0))
        .unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s["projects"][0]["total"], 2);
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM progress", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert!(e.mutate(json!({"entity":"task","action":"upsert","value":{"id":"x","projectId":"p","title":"기능","revision":"1","state":"pending"}}),None).unwrap_err().contains("managed"));
}
#[test]
fn manual_state_changes_are_absolute_and_revision_checked() {
    let (_d, e) = fixture();
    let k = e.knowledge();
    k.save_progress_basis(basis("dev", "development", 1, 1), 0)
        .unwrap();
    k.update_progress_task(
        "p",
        "dev",
        1,
        "T0",
        json!({"state":"inProgress","expectedTaskRevision":1}),
        1,
    )
    .unwrap();
    assert_eq!(
        k.get_project_progress("p", Some("dev")).unwrap()["counts"]["reportedComplete"],
        0
    );
    assert!(k
        .update_progress_task(
            "p",
            "dev",
            1,
            "T0",
            json!({"state":"pending","expectedTaskRevision":1}),
            1
        )
        .is_err());
}
#[test]
fn changed_semantics_requires_new_basis_and_changed_title_expires_evidence() {
    let (_d, e) = fixture();
    let k = e.knowledge();
    k.save_progress_basis(basis("dev", "development", 1, 1), 0)
        .unwrap();
    let mut b = basis("dev", "acceptance", 1, 1);
    b["version"] = json!(2);
    assert!(k.save_progress_basis(b, 1).is_err());
    k.update_progress_task(
        "p",
        "dev",
        1,
        "T0",
        json!({"title":"수락 조건 변경","expectedTaskRevision":1}),
        1,
    )
    .unwrap();
    let v = k.get_project_progress("p", Some("dev")).unwrap();
    assert_eq!(v["basis"]["tasks"][0]["revision"], 2);
    assert_eq!(v["counts"]["verified"], 0);
}

#[test]
fn evidence_revision_expires_and_unapplied_source_never_creates_observation() {
    let (d, e) = fixture();
    let root = d.path().join("docs");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("proof.md");
    std::fs::write(&path, "메뉴 시험 통과").unwrap();
    let k = e.knowledge();
    k.upsert_document_source(json!({"sourceId":"s","scope":"project","projectId":"p","location":"local","root":root,"roles":["review"],"enabled":true}),0).unwrap();
    let scan = || {
        k.scan_with_executable(
            &[],
            true,
            std::path::Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
            std::time::Duration::from_secs(5),
        )
        .unwrap()
    };
    scan();
    let doc = k.list_project_documents("p", 0).unwrap()["documents"][0].clone();
    let mut b = basis("dev", "development", 1, 1);
    b["authoritativeSourceIds"] = json!(["s"]);
    b["sourceVersionIds"] = json!([doc["versionId"]]);
    b["tasks"][0]["verificationEvidence"] = json!([{"subjectRevision":1,"success":true,"kind":"automated","fixture":false,"sourceSpan":{"versionId":doc["versionId"],"sha256":doc["sha256"],"locator":{"lines":{"start":1,"end":1}}}}]);
    k.save_progress_basis(b, 0).unwrap();
    k.activate_managed_progress("p", "dev", 1).unwrap();
    assert_eq!(
        k.get_project_progress("p", Some("dev")).unwrap()["counts"]["verified"],
        1
    );
    scan();
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    let original: String = c
        .query_row("SELECT payload FROM project_progress_points", [], |r| {
            r.get(0)
        })
        .unwrap();
    let point: Value = serde_json::from_str(&original).unwrap();
    assert_eq!(point["sourceSpans"][0]["versionId"], doc["versionId"]);
    k.update_progress_task(
        "p",
        "dev",
        1,
        "T0",
        json!({"title":"조건 변경","expectedTaskRevision":1}),
        2,
    )
    .unwrap();
    assert_eq!(
        k.get_project_progress("p", Some("dev")).unwrap()["counts"]["verified"],
        0
    );
    std::fs::write(path, "새 근거 미반영").unwrap();
    scan();
    assert_eq!(
        k.get_project_progress("p", Some("dev")).unwrap()["sourceFreshness"],
        "unappliedChange"
    );
    assert_eq!(
        c.query_row("SELECT payload FROM project_progress_points", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        original
    );
}

#[test]
fn missing_required_file_or_disabled_source_never_counts_as_fresh() {
    let (d, e) = fixture();
    let root = d.path().join("docs");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("work.md");
    std::fs::write(&path, "T0 완료").unwrap();
    let k = e.knowledge();
    let mut source = json!({"sourceId":"s","scope":"project","projectId":"p","location":"local","root":root,"roles":["checklist"],"enabled":true});
    k.upsert_document_source(source.clone(), 0).unwrap();
    let scan = || {
        k.scan_with_executable(
            &[],
            true,
            std::path::Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
            std::time::Duration::from_secs(5),
        )
        .unwrap()
    };
    scan();
    let doc = k.list_project_documents("p", 0).unwrap()["documents"][0].clone();
    let mut b = basis("dev", "development", 1, 1);
    b["authoritativeSourceIds"] = json!(["s"]);
    b["sourceVersionIds"] = json!([doc["versionId"]]);
    k.save_progress_basis(b, 0).unwrap();
    k.activate_managed_progress("p", "dev", 1).unwrap();
    scan();
    std::fs::remove_file(&path).unwrap();
    scan();
    assert_ne!(
        k.get_project_progress("p", Some("dev")).unwrap()["sourceFreshness"],
        "fresh"
    );
    assert!(k
        .read_document_version(doc["versionId"].as_str().unwrap(), 1, 200)
        .is_ok());
    std::fs::write(&path, "T0 완료").unwrap();
    scan();
    assert_eq!(
        k.get_project_progress("p", Some("dev")).unwrap()["sourceFreshness"],
        "fresh"
    );
    source["enabled"] = json!(false);
    k.upsert_document_source(source.clone(), 1).unwrap();
    assert_ne!(
        k.get_project_progress("p", Some("dev")).unwrap()["sourceFreshness"],
        "fresh"
    );
    let other = d.path().join("other");
    std::fs::create_dir(&other).unwrap();
    source["enabled"] = json!(true);
    source["root"] = json!(other);
    k.upsert_document_source(source, 2).unwrap();
    assert_ne!(
        k.get_project_progress("p", Some("dev")).unwrap()["sourceFreshness"],
        "fresh"
    );
    scan();
    assert_ne!(
        k.get_project_progress("p", Some("dev")).unwrap()["sourceFreshness"],
        "fresh"
    );
}

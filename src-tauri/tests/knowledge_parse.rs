// 문서 분류와 원장 파싱이 목표·검증·유사 프로젝트를 혼동하지 않는지 확인한다.
use devdock_core::knowledge::parser::*;
use serde_json::json;
#[test]
fn similar_project_names_do_not_cross_link() {
    let ps = json!([{"id":"Alpha","name":"Alpha","root":"C:/work/Alpha"},{"id":"AlphaVR","name":"AlphaVR","root":"C:/work/AlphaVR"}]);
    assert_eq!(
        match_projects(
            "HANDOVER_AlphaVR_20260101.md",
            "프로젝트 C:/work/AlphaVR",
            ps.as_array().unwrap()
        ),
        vec!["AlphaVR"]
    );
    assert!(match_projects(
        "HANDOVER_Alpha_20260101.md",
        "프로젝트 C:/work/AlphaVR",
        ps.as_array().unwrap()
    )
    .is_empty());
}
#[test]
fn company_daily_log_separates_goals_and_results() {
    let text="# 일일 작업기록\n작성일: 2026-01-17\n업무일: 2026-01-13\n## 목표\n- 기능 99개 완료 목표\n## 진행 내용\n- T1 메뉴 구현\n## 결과\n- 자동 시험 2186개 통과\n## 다음 작업\n- 전체 커밋 및 푸시\n";
    let cs = parse_document("v", "abc", text, "note.md").unwrap();
    assert!(cs.iter().all(|v| v["kind"] == "workEntry"));
    assert_eq!(cs[0]["workDate"], "2026-01-13");
    assert!(!serde_json::to_string(&cs).unwrap().contains("99개"));
    assert!(!serde_json::to_string(&cs).unwrap().contains("전체 커밋"));
}
#[test]
fn receipt_is_not_product_acceptance() {
    let cs = parse_document(
        "v",
        "abc",
        r#"{"schema":5,"run_id":"run","status":"published_verified","files":[]}"#,
        "receipt.json",
    )
    .unwrap();
    assert!(cs.is_empty());
}
#[test]
fn delivery_and_markdown_keep_reported_distinct_from_verified() {
    let data = json!({"schema_version":1,"tasks":[{"id":"T1","title":"설비","status":"done","checks":{"automated":{"status":"passed","evidence":[]},"review":{"status":"passed","evidence":[]}}},{"id":"T2","title":"입력","status":"waiting"}]});
    let cs = parse_document("v", "abc", &data.to_string(), "delivery-tasks.json").unwrap();
    let tasks = cs[0]["proposedChanges"]["tasks"].as_array().unwrap();
    assert_eq!(tasks[0]["state"], "reportedComplete");
    assert_ne!(tasks[0]["verified"], true);
    assert_eq!(tasks[1]["state"], "blocked");
    let cs = parse_document(
        "v",
        "abc",
        "- [x] UAT-01 화면\n```\n- [x] UAT-02 예시\n```",
        "check.md",
    )
    .unwrap();
    assert_eq!(
        cs[0]["proposedChanges"]["tasks"].as_array().unwrap().len(),
        1
    );
}
#[test]
fn completed_receipt_pointer_is_resolved_without_claiming_full_audit() {
    let map = json!({"run_id":"a","source_head":"head","publication_status":"pending_final_verification","completion_receipt":"checks/done.json"});
    let done = json!({"run_id":"a","source_head":"head","status":"published_verified"});
    let v = publication_metadata(&map, Some(&done));
    assert_eq!(v["receiptState"], "published_verified");
    assert_eq!(v["fullAudit"], false);
    let mismatch = json!({"run_id":"b","source_head":"head","status":"published_verified"});
    assert_eq!(
        publication_metadata(&map, Some(&mismatch))["receiptState"],
        "conflict"
    );
}

#[test]
fn shared_handover_source_links_only_cross_checked_project() {
    use devdock_core::service::Engine;
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().join("data")).unwrap();
    let docs = d.path().join("docs");
    std::fs::create_dir(&docs).unwrap();
    for id in ["Alpha", "AlphaVR"] {
        let root = d.path().join(id);
        e.mutate(
            json!({"entity":"project","action":"upsert","value":{"id":id,"name":id,"root":root}}),
            None,
        )
        .unwrap();
    }
    std::fs::write(
        docs.join("HANDOVER_AlphaVR_20260101.md"),
        format!("# 프로젝트\n{}", d.path().join("AlphaVR").display()),
    )
    .unwrap();
    e.knowledge().upsert_document_source(json!({"sourceId":"global","scope":"global","location":"local","root":docs,"roles":["handover"],"enabled":true}),0).unwrap();
    e.knowledge()
        .scan_with_executable(
            &[],
            true,
            std::path::Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(
        e.knowledge().list_project_documents("AlphaVR", 0).unwrap()["documents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        e.knowledge().list_project_documents("Alpha", 0).unwrap()["documents"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

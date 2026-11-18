// 보고 화면이 실제 로컬 핸드오버 폴더의 이름과 날짜만 조회하는지 검증한다.
use devdock_core::service::Engine;
use serde_json::json;
use std::fs;

#[test]
fn report_picker_lists_live_handover_names_and_dates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("handovers");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    for filename in [
        "HANDOVER_20261116_변경.md",
        "HANDOVER_20261117_수정.snapshot.json",
        "HANDOVER_20261117_실제.md",
        "artifacts.zip",
        "nested/HANDOVER_20261118_hidden.md",
    ] {
        fs::write(root.join(filename), [0xff]).unwrap();
    }
    let engine = Engine::open(temp.path().join("data")).unwrap();
    engine
        .mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"보고","root":temp.path()}}),None)
        .unwrap();
    let k = engine.knowledge();
    for (revision, (id, location, role, enabled, folder)) in [
        ("shared", "local", "handover", true, root.clone()),
        ("nas", "nas", "handover", true, root.clone()),
        ("reference", "local", "reference", true, root.clone()),
        ("disabled", "local", "handover", false, root.clone()),
        (
            "unavailable",
            "local",
            "handover",
            true,
            temp.path().join("missing"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        k.upsert_document_source(json!({"sourceId":id,"scope":"global","projectId":null,"root":folder,"location":location,"roles":[role],"enabled":enabled}),revision as u64)
            .unwrap();
    }
    let listing = k
        .execute_command(
            json!({"command":"list_report_handover_files","projectId":"p"}),
            true,
            true,
        )
        .unwrap();
    let folders = listing["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0]["sourceId"], "shared");
    assert_eq!(
        folders[0]["files"],
        json!([
            {"name":"HANDOVER_20261117_수정.snapshot.json","date":"2026-11-17"},
            {"name":"HANDOVER_20261117_실제.md","date":"2026-11-17"},
            {"name":"HANDOVER_20261116_변경.md","date":"2026-11-16"}
        ])
    );
    assert_eq!(folders[1]["sourceId"], "unavailable");
    assert!(folders[1]["error"].is_string());
}

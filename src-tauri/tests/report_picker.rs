// 보고 화면이 연결된 프로젝트 원문과 해당 스냅샷만 읽기 전용으로 나열하는지 검증한다.
use devdock_core::service::Engine;
use serde_json::json;
use std::{fs, path::Path, time::Duration};

#[test]
fn report_picker_lists_project_handovers_and_their_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("handovers");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    let recent = "HANDOVER_20261117_수정.md";
    let older = "HANDOVER_20261116_변경.md";
    let ignored_snapshot = "HANDOVER_20261116_변경.snapshot.json";
    let other = "HANDOVER_20261117_다른.md";
    for filename in [
        recent,
        "HANDOVER_20261117_수정.snapshot.json",
        older,
        ignored_snapshot,
        other,
        "HANDOVER_20261117_다른.snapshot.json",
        "HANDOVER_20261118_미연결.md",
        "artifacts.zip",
        "nested/HANDOVER_20261118_hidden.md",
    ] {
        fs::write(root.join(filename), "# 일일 기록").unwrap();
    }
    let engine = Engine::open(temp.path().join("data")).unwrap();
    for (id, name) in [("p", "개발 프로젝트"), ("q", "별도 프로젝트")] {
        let project_root = temp.path().join(id);
        fs::create_dir(&project_root).unwrap();
        engine.mutate(json!({"entity":"project","action":"upsert","value":{"id":id,"name":name,"root":project_root}}),None).unwrap();
    }
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
        k.upsert_document_source(json!({"sourceId":id,"scope":"global","projectId":null,"root":folder,"location":location,"roles":[role],"enabled":enabled}),revision as u64).unwrap();
    }
    k.scan_with_executable(
        &["shared".into()],
        true,
        Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
        Duration::from_secs(5),
    )
    .unwrap();
    let unlinked = k.list_unlinked_documents(0).unwrap();
    let document_id = |name: &str| {
        unlinked["documents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|doc| doc["path"] == name)
            .unwrap()["documentId"]
            .as_str()
            .unwrap()
    };
    k.set_document_project_link("p", document_id(recent), true, 0)
        .unwrap();
    k.set_document_project_link("p", document_id(older), true, 1)
        .unwrap();
    k.set_document_project_link("p", document_id(ignored_snapshot), false, 2)
        .unwrap();
    k.set_document_project_link("q", document_id(other), true, 0)
        .unwrap();
    // 조회는 문서 본문을 다시 읽지 않는다.
    fs::write(root.join(recent), [0xff]).unwrap();
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
            {"name":"HANDOVER_20261117_수정.md","date":"2026-11-17"},
            {"name":"HANDOVER_20261117_수정.snapshot.json","date":"2026-11-17"},
            {"name":"HANDOVER_20261116_변경.md","date":"2026-11-16"}
        ])
    );
    assert_eq!(folders[1]["sourceId"], "unavailable");
    assert!(folders[1]["error"].is_string());
}

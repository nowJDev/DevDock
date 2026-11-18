// 미분류 문서의 명시적 연결과 오래된 프로젝트 revision의 거절을 검증한다.
use devdock_core::service::Engine;
use serde_json::json;
#[test]
fn ambiguous_document_needs_explicit_project_link_and_cas() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("docs");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("note.md"), "# 프로젝트 미확인 기록").unwrap();
    let e = Engine::open(d.path().join("data")).unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":root}}),
        None,
    )
    .unwrap();
    let k = e.knowledge();
    k.upsert_document_source(json!({"sourceId":"s","scope":"global","location":"local","root":root,"roles":["handover"],"enabled":true}),0).unwrap();
    k.scan_with_executable(
        &[],
        true,
        std::path::Path::new(env!("CARGO_BIN_EXE_devdock-cli")),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let docs = k.list_unlinked_documents(0).unwrap();
    let id = docs["documents"][0]["documentId"].as_str().unwrap();
    assert!(k.list_project_documents("p", 0).unwrap()["documents"]
        .as_array()
        .unwrap()
        .is_empty());
    k.set_document_project_link("p", id, true, 0).unwrap();
    assert_eq!(
        k.list_project_documents("p", 0).unwrap()["documents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(k.set_document_project_link("p", id, false, 0).is_err());
    k.set_document_project_link("p", id, false, 1).unwrap();
    assert!(k.list_project_documents("p", 0).unwrap()["documents"]
        .as_array()
        .unwrap()
        .is_empty());
}

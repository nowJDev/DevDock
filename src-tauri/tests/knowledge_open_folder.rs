// NAS 탐색기는 등록한 프로젝트의 지정 하위 폴더만 해석해야 한다.
#[cfg(windows)]
#[test]
fn nas_open_only_uses_registered_folder() {
    use devdock_core::service::Engine;
    use serde_json::json;
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().join("data")).unwrap();
    for id in ["p", "q"] {
        engine
            .mutate(
                json!({"entity":"project","action":"upsert","value":{
                    "id":id,"name":id,"root":data.path().join(id),
                }}),
                None,
            )
            .unwrap();
    }
    let service = engine.knowledge();
    service.upsert_document_source(json!({"sourceId":"nas","scope":"project","projectId":"p",
        "location":"nas","root":r"\\example.invalid\한글 공유\업무 문서","enabled":true,"roles":["dailyLog"],
    }),0).unwrap();
    let target = service
        .resolve_project_nas_folder("p", "nas", "dailyLog")
        .unwrap();
    assert!(target.ends_with("일자별 작업기록"));
    assert!(service
        .resolve_project_nas_folder("q", "nas", "dailyLog")
        .is_err());
    assert!(service
        .resolve_project_nas_folder("p", "nas", "../다른 자료")
        .is_err());
    assert!(service
        .resolve_project_nas_folder("p", "unknown", "root")
        .is_err());
}

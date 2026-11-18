// NAS 게시 설정은 등록 범위와 안전 모드를 공유 서비스에서 강제해야 한다.
use devdock_core::service::Engine;
use serde_json::json;

#[test]
fn publish_requires_a_project_nas_source_with_explicit_targets() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().join("data")).unwrap();
    engine
        .mutate(
            json!({"entity":"project","action":"upsert","value":{
                "id":"p","name":"샘플","root":data.path().join("project"),
            }}),
            None,
        )
        .unwrap();
    let service = engine.knowledge();
    let source = json!({
        "sourceId":"a","scope":"project","projectId":"p","location":"local",
        "root":data.path().join("project"),"enabled":true,"roles":["checklist"],
        "publishTargets":[{"role":"checklist","path":"개발 진행 체크리스트/작업.md"}],
    });
    assert!(service.upsert_document_source(source.clone(), 0).is_err());
    #[cfg(windows)]
    {
        let mut nas = source;
        nas["root"] = json!(r"\\example.invalid\share");
        nas["location"] = json!("nas");
        let mut invalid = nas.clone();
        invalid["publishTargets"][0]["path"] = json!("../작업.md");
        assert!(service.upsert_document_source(invalid, 0).is_err());
        assert!(service.upsert_document_source(nas, 0).is_ok());
    }
}

#[test]
fn publish_commands_cannot_write_from_preview_or_safe_mode() {
    let data = tempfile::tempdir().unwrap();
    let service = Engine::open(data.path().join("data")).unwrap().knowledge();
    for command in ["apply_document_publish", "restore_document_publish"] {
        let request =
            json!({"command":command,"runId":"run","planHash":"hash","paths":["작업.md"]});
        assert!(service
            .execute_command(request.clone(), false, true)
            .unwrap_err()
            .contains("안전 모드"));
        assert!(service
            .execute_command(request, true, false)
            .unwrap_err()
            .contains("읽기 전용"));
    }
}

fn fake_share() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Vec<devdock_core::knowledge::publish_files::Target>,
) {
    use devdock_core::knowledge::publish_files::Target;
    use std::fs;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("share");
    let targets = vec![
        Target {
            path: "개발 진행 체크리스트/진행.md".into(),
            role: "checklist".into(),
        },
        Target {
            path: "일자별 작업기록/일지.md".into(),
            role: "dailyLog".into(),
        },
    ];
    let files=[
        ("프로젝트_문서정리_실행지침.md", "규약 v1.3.0".to_owned()),
        ("프로젝트 개발 지침/문서 배치표.json",json!({
            "schema_version":3,"run_id":"original","source_head":"source","publication_status":"pending_final_verification",
            "entries":targets.iter().map(|t|json!({"target":t.path})).collect::<Vec<_>>(),
        }).to_string()),
        ("검토·검증 결과/문서 정리 검증/게시 영수증.json",json!({
            "schema_version":5,"run_id":"original","source_head":"source","status":"published_verified","files":[],
        }).to_string()),
    ];
    for (path, content) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    for target in &targets {
        let path = root.join(&target.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "로컬 원문 보존\n").unwrap();
    }
    (temp, root, targets)
}

#[test]
fn publish_and_recover_only_unchanged_owned_files() {
    use devdock_core::knowledge::{publish_files as ops, scanner::sha256};
    use std::fs;
    let (dir, root, targets) = fake_share();
    let stage = dir.path().join("stage");
    fs::create_dir(&stage).unwrap();
    let (protocol, prior) = ops::inspect(&root, &targets).unwrap();
    let changes: Vec<_> = prior
        .into_iter()
        .map(|p| {
            let new = ops::tag("승인한 게시문", "test");
            ops::FileChange {
                path: p.target.path,
                role: p.target.role,
                before: p.sha256,
                after: sha256(new.as_bytes()),
                old_text: p.body,
                new_text: new,
            }
        })
        .collect();
    let selected = vec![targets[0].path.clone(), targets[1].path.clone()];
    fs::write(root.join(&selected[0]), "미리보기 뒤 수정").unwrap();
    assert!(ops::apply(&root, &stage, "test", &protocol, &changes, &selected).is_err());
    assert_eq!(
        fs::read_to_string(root.join(&selected[1])).unwrap(),
        changes[1].old_text
    );
    fs::write(root.join(&selected[0]), &changes[0].old_text).unwrap();
    let published = ops::apply(&root, &stage, "test", &protocol, &changes, &selected).unwrap();
    assert_eq!(published.state, "filesVerified");
    assert!(ops::generated(
        &fs::read_to_string(root.join(&selected[1])).unwrap()
    ));
    fs::write(root.join(&selected[0]), "게시 후 사람의 수정").unwrap();
    let recovered = ops::restore(&root, &stage, "test", &changes, &selected, &selected).unwrap();
    assert_eq!(recovered.state, "restoreConflicts");
    assert_eq!(recovered.files[0].state, "conflict");
    assert_eq!(recovered.files[1].state, "restored");
    assert_eq!(
        fs::read_to_string(root.join(&selected[0])).unwrap(),
        "게시 후 사람의 수정"
    );
    assert_eq!(
        fs::read_to_string(root.join(&selected[1])).unwrap(),
        changes[1].old_text
    );
}

#[cfg(windows)]
#[test]
fn partial_publish_is_not_verified() {
    use devdock_core::knowledge::{publish_files as ops, scanner::sha256};
    use std::{fs, os::windows::fs::OpenOptionsExt};
    let (dir, root, targets) = fake_share();
    let stage = dir.path().join("stage");
    fs::create_dir(&stage).unwrap();
    let (protocol, prior) = ops::inspect(&root, &targets).unwrap();
    let changes: Vec<_> = prior
        .into_iter()
        .map(|p| {
            let new = "새 본문".to_owned();
            ops::FileChange {
                path: p.target.path,
                role: p.target.role,
                before: p.sha256,
                after: sha256(new.as_bytes()),
                old_text: p.body,
                new_text: new,
            }
        })
        .collect();
    let selected: Vec<_> = targets.iter().map(|t| t.path.clone()).collect();
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(root.join(&selected[1]))
        .unwrap();
    let published = ops::apply(&root, &stage, "partial", &protocol, &changes, &selected).unwrap();
    assert_eq!(published.state, "recoveryRequired");
    assert_eq!(published.files[0].state, "verified");
    assert_ne!(published.files[1].state, "verified");
    drop(handle);
    assert_eq!(
        ops::restore(&root, &stage, "partial", &changes, &selected, &selected)
            .unwrap()
            .state,
        "restored"
    );
}

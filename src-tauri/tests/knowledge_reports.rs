// 보고 요청이 정확한 원문·스킬·lease를 사용하며 초안을 자동 저장하지 않는지 검증한다.
use devdock_core::{knowledge::KnowledgeService, service::Engine};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn fixture() -> (tempfile::TempDir, Engine, KnowledgeService, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    fs::create_dir(&root).unwrap();
    let input = root.join("일일 기록.md");
    fs::write(&input,"# 기록\n보관하지않는초안문구\n메뉴 1 구현\n메뉴 2 구현\n메뉴 3 구현\n메뉴 4 구현\n메뉴 5 구현\n메뉴 6 구현").unwrap();
    let skill = d.path().join("skill");
    fs::create_dir_all(skill.join("references")).unwrap();
    fs::create_dir(skill.join("assets")).unwrap();
    for p in [
        "SKILL.md",
        "references/workflow.md",
        "references/source-analysis.md",
        "references/report-rules.md",
        "references/templates.md",
        "assets/default-report-template.txt",
    ] {
        fs::write(
            skill.join(p),
            "# 보고 규칙\n선택한 자료만 읽고 근거가 부족하면 내용을 만들지 않는다.",
        )
        .unwrap();
    }
    let e = Engine::open(d.path().join("data")).unwrap();
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"샘플 프로젝트","root":root}}),None).unwrap();
    let k = e
        .knowledge()
        .with_worker_executable(PathBuf::from(env!("CARGO_BIN_EXE_devdock-cli")));
    let mut p = k.get_report_profile("p").unwrap();
    p["skillPath"] = json!(skill.join("SKILL.md"));
    k.save_report_profile(p, 0).unwrap();
    (d, e, k, input)
}
fn request(k: &KnowledgeService, input: &Path) -> Value {
    k.create_report_request(json!({"projectId":"p","date":"2026-01-01","heading":"샘플 프로젝트","selectedFiles":[input],"templateMode":"default"})).unwrap()
}
fn body() -> String {
    "(1/1) 일일 업무\n1. 주요 업무내용\n  샘플 프로젝트\n    - 보관하지않는초안문구\n      = 메뉴 1 구현\n      = 메뉴 2 구현\n      = 메뉴 3 구현\n      = 메뉴 4 구현\n      = 메뉴 5 구현\n      = 메뉴 6 구현".into()
}
fn result() -> Value {
    json!({"body":body(),"needsMoreSources":false,"grounding":(4..=10).map(|line|json!({"reportLine":line,"sourceIndex":0,"startLine":if line==4{2}else{line-2},"endLine":if line==4{2}else{line-2},"classification":"implementation"})).collect::<Vec<_>>()})
}
fn prepared(k: &KnowledgeService, input: &Path) -> (Value, String, Value) {
    let r = request(k, input);
    let lease = k
        .claim_analysis_job(r["jobId"].as_str().unwrap(), "codex")
        .unwrap();
    let token = lease["leaseToken"].as_str().unwrap().to_owned();
    let proof = k
        .read_report_source(r["requestId"].as_str().unwrap(), 0, &token)
        .unwrap();
    assert!(proof["text"].as_str().unwrap().contains("메뉴 6"));
    (r, token, proof["proof"].clone())
}
#[test]
fn report_reads_only_explicit_md_txt() {
    let (d, _e, k, input) = fixture();
    let bad = d.path().join("state.json");
    fs::write(&bad, "{}").unwrap();
    assert!(k
        .create_report_request(json!({"projectId":"p","heading":"제목","selectedFiles":[bad]}))
        .is_err());
    assert!(k.create_report_request(json!({"projectId":"p","heading":"제목","selectedFiles":[input.with_file_name("비슷한 파일.md")]})).is_err());
    let r = request(&k, &input);
    assert_eq!(r["sources"].as_array().unwrap().len(), 1);
}
#[test]
fn report_changed_input_requires_reread() {
    let (_d, _e, k, input) = fixture();
    let (r, token, proof) = prepared(&k, &input);
    fs::write(&input, "새로운 내용").unwrap();
    assert!(k
        .submit_report_result(
            r["requestId"].as_str().unwrap(),
            &token,
            result(),
            json!([proof])
        )
        .unwrap_err()
        .contains("source_changed"));
}
#[test]
fn report_never_invents_metadata_or_accepts_unread_sources() {
    let (_d, _e, k, input) = fixture();
    assert!(k
        .create_report_request(json!({"projectId":"p","heading":"","selectedFiles":[input]}))
        .is_err());
    let r = request(&k, &input);
    let lease = k
        .claim_analysis_job(r["jobId"].as_str().unwrap(), "codex")
        .unwrap();
    assert!(k
        .submit_report_result(
            r["requestId"].as_str().unwrap(),
            lease["leaseToken"].as_str().unwrap(),
            result(),
            json!([])
        )
        .is_err());
    let context = k
        .get_report_context(r["requestId"].as_str().unwrap())
        .unwrap();
    assert_eq!(context["heading"], "샘플 프로젝트");
    assert!(context.get("progressRatio").is_none());
    assert!(context.get("deadline").is_none());
}
#[test]
fn report_text_is_not_auto_persisted_or_published() {
    let (_d, e, k, input) = fixture();
    let (r, token, proof) = prepared(&k, &input);
    k.submit_report_result(
        r["requestId"].as_str().unwrap(),
        &token,
        result(),
        json!([proof]),
    )
    .unwrap();
    assert_eq!(
        k.get_report_draft(r["requestId"].as_str().unwrap())
            .unwrap()["draft"]["body"],
        body()
    );
    for file in fs::read_dir(e.data_dir()).unwrap().flatten() {
        if file.file_type().unwrap().is_file() {
            assert!(!String::from_utf8_lossy(&fs::read(file.path()).unwrap())
                .contains("보관하지않는초안문구"));
        }
    }
    let c = rusqlite::Connection::open(e.data_dir().join("devdock.sqlite3")).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM document_publish_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn report_restart_expires_draft_but_cli_connect_preserves_it() {
    let (_d, e, k, input) = fixture();
    let (r, token, proof) = prepared(&k, &input);
    k.submit_report_result(
        r["requestId"].as_str().unwrap(),
        &token,
        result(),
        json!([proof]),
    )
    .unwrap();
    let id = r["requestId"].as_str().unwrap();
    Engine::connect(e.data_dir().into()).unwrap();
    assert!(k.get_report_draft(id).unwrap()["draft"]["body"].is_string());
    let restarted = Engine::open(e.data_dir().into()).unwrap();
    assert_eq!(
        restarted.knowledge().get_report_draft(id).unwrap()["state"],
        "resultExpired"
    );
}
#[test]
fn explicit_local_save_preserves_originals_and_detects_overwrite_conflict() {
    let (d, _e, k, input) = fixture();
    let (r, token, proof) = prepared(&k, &input);
    k.submit_report_result(
        r["requestId"].as_str().unwrap(),
        &token,
        result(),
        json!([proof]),
    )
    .unwrap();
    let id = r["requestId"].as_str().unwrap();
    let out = d.path().join("reports").join("보고.txt");
    let p = k.preview_report_save(id, &out).unwrap();
    assert_eq!(p["exists"], false);
    k.save_report_draft(id, &body(), &out, None, false).unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), body());
    let p = k.preview_report_save(id, &out).unwrap();
    fs::write(&out, "다른 사람이 수정").unwrap();
    assert!(k
        .save_report_draft(id, &body(), &out, p["sha256"].as_str(), true)
        .is_err());
    assert!(k
        .save_report_draft(id, &body(), &input, None, false)
        .is_err());
    assert!(k
        .preview_report_save(id, Path::new("\\\\server\\docs\\report.md"))
        .is_err());
}
#[test]
fn rule_package_changes_and_report_paths_are_rejected() {
    let (_d, _e, k, input) = fixture();
    let (r, token, proof) = prepared(&k, &input);
    let p = k.get_report_profile("p").unwrap();
    let rules = Path::new(p["skillPath"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("references/report-rules.md");
    fs::write(rules, "규칙 변경").unwrap();
    assert!(k
        .submit_report_result(
            r["requestId"].as_str().unwrap(),
            &token,
            result(),
            json!([proof])
        )
        .is_err());
}

#[cfg(windows)]
#[test]
fn cli_submits_directly_to_desktop_memory() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let (_dir, engine, knowledge, input) = fixture();
    let (request, token, proof) = prepared(&knowledge, &input);
    let id = request["requestId"].as_str().unwrap();
    let _host = devdock_core::knowledge::report_ipc::start(knowledge.clone()).unwrap();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_devdock-cli"))
        .env("DEVDOCK_DATA_DIR", engine.data_dir())
        .args(["reports", "submit", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    cli.stdin
        .take()
        .unwrap()
        .write_all(
            json!({
                "requestId": id, "leaseToken": token, "draft": result(), "proofs": [proof]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = cli.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        knowledge.get_report_draft(id).unwrap()["draft"]["body"],
        body()
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("보관하지않는초안문구"));
}

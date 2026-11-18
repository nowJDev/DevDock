// Codex 초안 검토와 NAS 파일 적용을 별도 단계로 관리한다.
use super::{
    err, jobs,
    models::SourceSpan,
    now,
    publish_files::{self as files, FileChange, Prior, Protocol, Target},
    scanner::sha256,
    KnowledgeService, Result,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    project_id: String,
    source_id: String,
    input_version_ids: Vec<String>,
    basis_id: Option<String>,
    paths: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    patches: Vec<Patch>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Patch {
    path: String,
    before_sha256: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    task_changes: Vec<TaskChange>,
    #[serde(default)]
    source_spans: Vec<SourceSpan>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskChange {
    task_id: String,
    expected_checked: bool,
    state: String,
    task_revision: i64,
    source_spans: Vec<SourceSpan>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    run_id: String,
    job_id: String,
    project_id: String,
    source_id: String,
    config_revision: u64,
    protocol: Protocol,
    files: Vec<FileChange>,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Worker {
    Inspect {
        project: String,
        source: String,
        paths: Vec<String>,
    },
    Apply {
        run: String,
    },
    Restore {
        run: String,
    },
}

fn config_targets(
    c: &Connection,
    project: &str,
    source: &str,
) -> Result<(u64, PathBuf, Vec<Target>)> {
    let config = super::store::config(c)?;
    let value = config["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| {
            s["sourceId"] == source
                && s["projectId"] == project
                && s["scope"] == "project"
                && s["location"] == "nas"
                && s["enabled"] == true
                && s["archived"] != true
        })
        .ok_or("이 프로젝트에 사용 가능한 NAS 폴더를 설정해 주세요.")?;
    let root = PathBuf::from(value["root"].as_str().ok_or("NAS 경로가 없습니다.")?);
    if !files::is_unc(&root)
        || root.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("등록된 UNC NAS 경로가 필요합니다.".into());
    }
    let targets: Vec<Target> = serde_json::from_value(value["publishTargets"].clone())
        .map_err(|_| "설정에서 NAS 게시 대상을 먼저 지정해 주세요.")?;
    files::validate_targets(&targets)?;
    if targets.is_empty() {
        return Err("등록된 NAS 게시 대상이 없습니다.".into());
    }
    Ok((config["revision"].as_u64().unwrap(), root, targets))
}
fn selected_targets(targets: &[Target], paths: &[String]) -> Result<Vec<Target>> {
    let selected: Vec<_> = targets
        .iter()
        .filter(|t| paths.contains(&t.path))
        .cloned()
        .collect();
    if selected.is_empty() || selected.len() != paths.len() {
        return Err("등록한 NAS 게시 대상만 선택해 주세요.".into());
    }
    Ok(selected)
}
fn read_run(c: &Connection, id: &str) -> Result<(String, Value)> {
    Uuid::parse_str(id).map_err(|_| "잘못된 게시 실행 ID입니다.")?;
    let (state, text): (String, String) = c
        .query_row(
            "SELECT state,payload FROM document_publish_jobs WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    Ok((state, serde_json::from_str(&text).map_err(err)?))
}
fn stage(service: &KnowledgeService, id: &str) -> PathBuf {
    service
        .engine
        .data_dir()
        .join("document-publications")
        .join(id)
}
fn load_plan(input: &Value) -> Result<Plan> {
    serde_json::from_value(input["plan"].clone()).map_err(err)
}
fn public_run(state: &str, value: &Value, result: Value) -> Value {
    json!({"runId":value["plan"]["runId"],"jobId":value["plan"]["jobId"],"state":state,"planHash":value["planHash"],
        "files":value["plan"]["files"],"selectedPaths":value["selectedPaths"],"result":result,
        "sourceRunId":value["plan"]["protocol"]["runId"],"fullAudit":false,"error":value["error"]})
}

fn status_body(basis: &Value) -> Result<String> {
    if basis.is_null() {
        return Err("현재 상태 문서를 게시하려면 승인된 진행 기준을 선택해 주세요.".into());
    }
    let counts = super::progress::counts(basis);
    Ok(format!("### DevDock 기준 집계\n\n기준 버전 {} · 전체 {} · 검증 완료 {} · 완료 보고 {} · 진행 중 {} · 막힘 {}",
        basis["version"], counts["total"],counts["verified"],counts["reportedComplete"],counts["inProgress"],counts["blocked"]))
}
fn index_body(prior: &[Prior], path: &str) -> String {
    let mut links = vec!["### DevDock 게시 문서".to_owned()];
    for entry in prior.iter().filter(|entry| entry.target.path != path) {
        let name = entry.target.path.replace('\\', "/");
        links.push(format!("- [{}](<{}>)", name.replace(['[', ']'], ""), name));
    }
    links.join("\n")
}
fn managed_body(original: &str, body: &str) -> Result<String> {
    const OPEN: &str = "<!-- devdock-managed:start -->";
    const CLOSE: &str = "<!-- devdock-managed:end -->";
    if body.trim().is_empty()
        || body.len() > 64 * 1024
        || body.contains("<!--")
        || body.contains("![")
        || body.contains('\\')
        || body.contains("/Users/")
        || body.contains("/home/")
        || body.contains("file:")
        || body
            .as_bytes()
            .windows(3)
            .any(|b| b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/')
    {
        return Err("게시 본문에 내부 경로, HTML 표식 또는 이미지를 넣을 수 없습니다.".into());
    }
    let eol = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let text = body.lines().collect::<Vec<_>>().join(eol);
    match (
        original.match_indices(OPEN).collect::<Vec<_>>().as_slice(),
        original.match_indices(CLOSE).collect::<Vec<_>>().as_slice(),
    ) {
        ([], []) => Ok(format!(
            "{original}{eol}{eol}{OPEN}{eol}{text}{eol}{CLOSE}{eol}"
        )),
        ([(start, _)], [(end, _)]) if end > start => Ok(format!(
            "{}{eol}{text}{eol}{}",
            &original[..start + OPEN.len()],
            &original[*end..]
        )),
        _ => Err("관리 구간 표식이 여러 개이거나 닫히지 않았습니다.".into()),
    }
}

fn checklist(original: &str, change: &TaskChange) -> Result<String> {
    let mut result = String::with_capacity(original.len());
    let mut matches = 0;
    let after = if change.state == "reportedComplete" {
        'x'
    } else {
        ' '
    };
    for line in original.split_inclusive('\n') {
        let content = line.trim_start();
        let marker = content
            .strip_prefix("- [")
            .or_else(|| content.strip_prefix("* ["));
        if let Some(marker) = marker {
            let id = marker.get(3..).unwrap_or("").trim_start();
            let has_id = id.strip_prefix(&change.task_id).is_some_and(|rest| {
                rest.is_empty()
                    || rest.starts_with(|c: char| c.is_whitespace() || ".)：:|".contains(c))
            });
            let checked = marker.get(..3);
            if has_id && matches!(checked, Some("x] " | "X] " | " ] ")) {
                matches += 1;
                if (checked != Some(" ] ")) != change.expected_checked {
                    return Err("체크리스트 기존 상태가 초안과 다릅니다.".into());
                }
                let offset = line.len() - content.len() + 3;
                result.push_str(&line[..offset]);
                result.push(after);
                result.push_str(&line[offset + 1..]);
                continue;
            }
        }
        result.push_str(line);
    }
    // Table ledgers track deployment and acceptance in separate columns. Publish
    // the selected DevDock state next to the ledger without replacing those columns.
    if matches == 0
        && !change.expected_checked
        && original
            .lines()
            .filter(|line| {
                let mut parts = line.trim().split('|');
                parts.next() == Some("")
                    && parts.next().is_some_and(|id| id.trim() == change.task_id)
            })
            .count()
            == 1
    {
        let eol = if original.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let heading = "## DevDock 진행 요약 (제품 수락과 별도).";
        if original.matches(heading).count() > 1 {
            return Err("DevDock 진행 요약 제목이 중복됩니다.".into());
        }
        let mut updated = original.to_owned();
        if !updated.ends_with('\n') {
            updated.push_str(eol);
        }
        if !updated.contains(heading) {
            updated.push_str(&format!("{eol}{heading}{eol}{eol}"));
        }
        let position = updated.find(heading).unwrap() + heading.len() + eol.len();
        updated.insert_str(
            position,
            &format!("- [{after}] {} · 승인된 DevDock 상태{eol}", change.task_id),
        );
        return Ok(updated);
    }
    if matches != 1 {
        return Err(format!(
            "체크리스트에서 작업 ID를 한 번만 찾을 수 있어야 합니다. {}",
            change.task_id
        ));
    }
    Ok(result)
}

fn task_value(c: &Connection, job: &Value, change: &TaskChange) -> Result<()> {
    if change.task_id.is_empty()
        || !matches!(
            change.state.as_str(),
            "pending" | "inProgress" | "blocked" | "reportedComplete"
        )
    {
        return Err("지원하지 않는 체크리스트 상태입니다.".into());
    }
    let basis = job["basisId"]
        .as_str()
        .ok_or("체크리스트를 게시하려면 승인한 진행 기준을 선택해 주세요.")?;
    let version = job["basisVersion"]
        .as_i64()
        .ok_or("진행 차수가 없습니다.")?;
    let (revision,state):(i64,String)=c.query_row(
        "SELECT revision,state FROM basis_tasks WHERE basis_id=?1 AND version=?2 AND task_id=?3",
        params![basis,version,change.task_id],|r|Ok((r.get(0)?,r.get(1)?)),
    ).map_err(err)?;
    if revision != change.task_revision || state != change.state {
        return Err("DevDock에서 승인된 작업 상태와 게시 제안이 다릅니다.".into());
    }
    Ok(())
}

impl KnowledgeService {
    fn publish_worker(&self, input: &Worker, secs: u64) -> Result<Value> {
        let mut child = Command::new(self.worker_executable()?);
        child
            .arg("--document-publish-worker")
            .arg(self.engine.data_dir());
        let bytes = serde_json::to_vec(input).map_err(err)?;
        let output = super::report_worker::capture(
            child,
            Some(bytes),
            32 * 1024 * 1024,
            Duration::from_secs(secs),
        )?;
        let mut response: Value = serde_json::from_slice(&output.bytes)
            .map_err(|_| "NAS 문서 작업 프로세스의 응답을 읽지 못했습니다.")?;
        if output.success && response["ok"] == true {
            Ok(response["data"].take())
        } else {
            Err(response["error"]
                .as_str()
                .unwrap_or("NAS 문서 작업이 중단되었습니다.")
                .into())
        }
    }
    pub fn create_publish_draft_request(&self, request: Value) -> Result<Value> {
        let r: Request = serde_json::from_value(request).map_err(err)?;
        self.require_project(&r.project_id)?;
        let (revision, _, targets) = config_targets(&self.db()?, &r.project_id, &r.source_id)?;
        selected_targets(&targets, &r.paths)?;
        let inspected = self.publish_worker(
            &Worker::Inspect {
                project: r.project_id.clone(),
                source: r.source_id.clone(),
                paths: r.paths,
            },
            30,
        )?;
        if inspected["configRevision"] != revision {
            return Err("NAS 폴더 설정이 바뀌었습니다.".into());
        }
        let progress = self.get_project_progress(&r.project_id, r.basis_id.as_deref())?;
        let context = json!({"sourceId":r.source_id,"configRevision":revision,
            "protocol":inspected["protocol"],"prior":inspected["prior"],"basis":progress["basis"],"basisViewRevision":progress["revision"],
            "instructions":"선택한 원문 버전만 읽고 문서별 근거 행과 함께 패치를 제안합니다. 체크리스트는 승인 기준의 작업 ID, 예상 기존 체크 여부, 승인된 현재 상태, 작업 revision과 원문 범위를 사용합니다. 셸 명령, 배포, NAS 직접 쓰기, 새 수락 증거 생성을 수행하지 않습니다."});
        self.create_job(json!({"projectId":r.project_id,"inputVersionIds":r.input_version_ids,"basisId":r.basis_id}),
            "documentPublishDraft",Some(context))
    }
    fn render_patches(
        &self,
        job: &Value,
        patches: &[Patch],
        run: &str,
        accepted_tasks: Option<&[String]>,
    ) -> Result<Vec<FileChange>> {
        let context = &job["publishContext"];
        let prior: Vec<Prior> = serde_json::from_value(context["prior"].clone()).map_err(err)?;
        if patches.is_empty() || patches.len() > prior.len() {
            return Err("등록 대상에 대한 패치를 제출해 주세요.".into());
        }
        let c = self.db()?;
        let mut unique = HashSet::new();
        let mut rendered = Vec::new();
        for patch in patches {
            let previous = prior
                .iter()
                .find(|p| p.target.path == patch.path)
                .ok_or("등록한 대상이 아닌 문서의 패치입니다.")?;
            if !unique.insert(&patch.path) || patch.before_sha256 != previous.sha256 {
                return Err("패치의 기존 파일 해시가 다릅니다.".into());
            }
            let body = if previous.target.role == "checklist" {
                if patch.body.is_some()
                    || !patch.source_spans.is_empty()
                    || patch.task_changes.is_empty()
                    || patch.task_changes.len() > 200
                {
                    return Err("체크리스트는 개별 작업 상태 변경으로 제안해 주세요.".into());
                }
                let mut text = previous.body.clone();
                let mut ids = HashSet::new();
                let mut selected = 0;
                for change in &patch.task_changes {
                    if !ids.insert(&change.task_id) {
                        return Err("중복 작업 ID입니다.".into());
                    }
                    self.publish_spans(job, &change.source_spans)?;
                    task_value(&c, job, change)?;
                    if accepted_tasks.is_none_or(|ids| {
                        ids.contains(&format!("{}::{}", patch.path, change.task_id))
                    }) {
                        text = checklist(&text, change)?;
                        selected += 1;
                    }
                }
                if selected == 0 {
                    continue;
                }
                text
            } else {
                if !patch.task_changes.is_empty() {
                    return Err("일반 문서에는 작업 상태 패치를 사용할 수 없습니다.".into());
                }
                self.publish_spans(job, &patch.source_spans)?;
                let content = match previous.target.role.as_str() {
                    "status" => status_body(&context["basis"])?,
                    "index" => index_body(&prior, &patch.path),
                    _ => patch
                        .body
                        .as_deref()
                        .ok_or("게시할 본문을 입력해 주세요.")?
                        .to_owned(),
                };
                managed_body(&previous.body, &content)?
            };
            let new_text = files::tag(&body, run);
            if new_text.len() > 2 * 1024 * 1024 {
                return Err("게시 결과의 파일 크기가 한도를 넘었습니다.".into());
            }
            rendered.push(FileChange {
                path: patch.path.clone(),
                role: previous.target.role.clone(),
                before: previous.sha256.clone(),
                after: sha256(new_text.as_bytes()),
                old_text: previous.body.clone(),
                new_text,
            });
        }
        if rendered.is_empty() {
            return Err("반영할 문서나 체크리스트 항목을 선택해 주세요.".into());
        }
        Ok(rendered)
    }
    fn publish_spans(&self, job: &Value, spans: &[SourceSpan]) -> Result<()> {
        self.validate_spans(job, spans)?;
        for span in spans {
            if files::generated(&self.document_text(&span.version_id)?) {
                return Err(
                    "DevDock이 게시한 문서를 자기 게시의 근거로 사용할 수 없습니다.".into(),
                );
            }
        }
        Ok(())
    }
    pub fn submit_publish_draft_result(
        &self,
        id: &str,
        token: &str,
        result: Value,
    ) -> Result<Value> {
        let draft: Draft = serde_json::from_value(result).map_err(err)?;
        let job = jobs::load_job(&self.db()?, id)?;
        if job["kind"] != "documentPublishDraft" {
            return Err("게시 초안 요청이 아닙니다.".into());
        }
        jobs::check_lease(&job, token)?;
        self.render_patches(&job, &draft.patches, &Uuid::nil().to_string(), None)?;
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut current = jobs::load_job(&tx, id)?;
        jobs::check_lease(&current, token)?;
        self.validate_job_current(&tx, &current)?;
        current["patches"] = serde_json::to_value(draft.patches).map_err(err)?;
        current.as_object_mut().unwrap().remove("leaseToken");
        tx.execute(
            "UPDATE analysis_jobs SET state='completed',payload=?1 WHERE job_id=?2",
            params![current.to_string(), id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(json!({"jobId":id,"state":"completed"}))
    }
    pub fn prepare_document_publish(
        &self,
        id: &str,
        paths: Vec<String>,
        tasks: Vec<String>,
        inputs: Vec<String>,
    ) -> Result<Value> {
        let mut c = self.db()?;
        let job = jobs::load_job(&c, id)?;
        if job["kind"] != "documentPublishDraft"
            || job["state"] != "completed"
            || job["inputVersionIds"] != json!(inputs)
        {
            return Err("검토한 게시 초안과 원문을 다시 선택해 주세요.".into());
        }
        let context = &job["publishContext"];
        let source = context["sourceId"].as_str().unwrap();
        let project = job["projectId"].as_str().unwrap();
        let (revision, _, targets) = config_targets(&c, project, source)?;
        selected_targets(&targets, &paths)?;
        if context["configRevision"] != revision {
            return Err("NAS 게시 설정이 변경되었습니다.".into());
        }
        let patches: Vec<Patch> = serde_json::from_value(job["patches"].clone()).map_err(err)?;
        if paths
            .iter()
            .any(|path| !patches.iter().any(|p| p.path == *path))
            || tasks.iter().any(|key| {
                !patches.iter().any(|p| {
                    paths.contains(&p.path)
                        && p.task_changes
                            .iter()
                            .any(|t| *key == format!("{}::{}", p.path, t.task_id))
                })
            })
        {
            return Err("초안에 있는 파일과 항목만 선택해 주세요.".into());
        }
        let patches: Vec<_> = patches
            .into_iter()
            .filter(|p| paths.contains(&p.path))
            .collect();
        let run = Uuid::new_v4().to_string();
        let files = self.render_patches(&job, &patches, &run, Some(&tasks))?;
        let plan = Plan {
            run_id: run.clone(),
            job_id: id.into(),
            project_id: project.into(),
            source_id: source.into(),
            config_revision: revision,
            protocol: serde_json::from_value(context["protocol"].clone()).map_err(err)?,
            files,
        };
        let plan_hash = sha256(&serde_json::to_vec(&plan).map_err(err)?);
        let value = json!({"plan":plan,"planHash":plan_hash,"createdAt":now()});
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        self.validate_job_current(&tx, &job)?;
        tx.execute(
            "INSERT INTO document_publish_jobs VALUES(?1,?2,'previewReady',?3)",
            params![run, project, value.to_string()],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(public_run("previewReady", &value, Value::Null))
    }
    pub fn list_document_publishes(&self, project: &str) -> Result<Value> {
        self.require_project(project)?;
        let c = self.db()?;
        let mut query=c.prepare("SELECT job_id,state,json_extract(payload,'$.plan.sourceId') FROM document_publish_jobs
            WHERE project_id=?1 ORDER BY rowid DESC LIMIT 20").map_err(err)?;
        let rows = query
            .query_map([project], |row| {
                Ok(json!({"runId":row.get::<_,String>(0)?,
            "state":row.get::<_,String>(1)?,"sourceId":row.get::<_,String>(2)?}))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        Ok(json!({"runs":rows}))
    }
    pub fn get_document_publish_status(&self, id: &str) -> Result<Value> {
        let (state, value) = read_run(&self.db()?, id)?;
        let journal = files::result(&stage(self, id).join("status.json")).unwrap_or(Value::Null);
        Ok(public_run(&state, &value, journal))
    }
    pub fn execute_publish(
        &self,
        id: &str,
        hash: &str,
        paths: Vec<String>,
        restore: bool,
        safe: bool,
    ) -> Result<Value> {
        if safe {
            return Err("안전 모드에서는 NAS 게시와 복구를 실행할 수 없습니다.".into());
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let (state, mut payload) = read_run(&tx, id)?;
        let plan = load_plan(&payload)?;
        if payload["planHash"] != hash || sha256(&serde_json::to_vec(&plan).map_err(err)?) != hash {
            return Err("게시 미리보기 해시가 다릅니다.".into());
        }
        let (revision, root, targets) = config_targets(&tx, &plan.project_id, &plan.source_id)?;
        selected_targets(&targets, &paths)?;
        if revision != plan.config_revision && !restore {
            return Err("NAS 게시 설정이 바뀌었습니다.".into());
        }
        if !restore {
            if state != "previewReady" {
                return Err("이 게시 미리보기는 이미 실행되었습니다.".into());
            }
            let job = jobs::load_job(&tx, &plan.job_id)?;
            self.validate_job_current(&tx, &job)?;
            if paths.is_empty()
                || paths
                    .iter()
                    .any(|path| !plan.files.iter().any(|f| f.path == *path))
            {
                return Err("게시 미리보기에서 파일을 골라 주세요.".into());
            }
            let selected: Vec<_> = plan
                .files
                .iter()
                .filter(|f| paths.contains(&f.path))
                .map(|f| &f.path)
                .collect();
            payload["selectedPaths"] = json!(selected);
            payload["originalRoot"] = json!(root);
        } else {
            if !matches!(
                state.as_str(),
                "filesVerified" | "recoveryRequired" | "restoreConflicts" | "partiallyRestored"
            ) || payload["originalRoot"] != json!(root)
            {
                return Err("원래 NAS 경로의 완료 또는 복구 대기 작업을 선택해 주세요.".into());
            }
            if paths.is_empty()
                || paths.iter().any(|p| {
                    !payload["selectedPaths"]
                        .as_array()
                        .is_some_and(|ps| ps.contains(&json!(p)))
                })
            {
                return Err("이 실행에서 게시한 파일만 복구할 수 있습니다.".into());
            }
            payload["restorePaths"] = json!(paths);
        }
        let active:Option<String>=tx.query_row("SELECT job_id FROM document_publish_jobs
            WHERE project_id=?1 AND job_id<>?2 AND state IN ('applying','restoring','recoveryRequired') LIMIT 1",
            params![plan.project_id,id],|r|r.get(0)).optional().map_err(err)?;
        if active.is_some() {
            return Err("같은 프로젝트에 진행 중이거나 복구가 필요한 게시가 있습니다.".into());
        }
        let operation = if restore { "restoring" } else { "applying" };
        tx.execute(
            "UPDATE document_publish_jobs SET state=?1,payload=?2 WHERE job_id=?3",
            params![operation, payload.to_string(), id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        let request = if restore {
            Worker::Restore { run: id.into() }
        } else {
            Worker::Apply { run: id.into() }
        };
        payload.as_object_mut().unwrap().remove("error");
        let outcome = self.publish_worker(&request, 120);
        let final_state = match &outcome {
            Ok(value) => value["state"].as_str().unwrap_or("recoveryRequired"),
            Err(error) => {
                payload["error"] = json!(error);
                let status =
                    files::result(&stage(self, id).join("status.json")).unwrap_or(Value::Null);
                if restore || status["state"].as_str().is_some_and(|s| s != "unknown") {
                    "recoveryRequired"
                } else {
                    "failed"
                }
            }
        };
        self.db()?
            .execute(
                "UPDATE document_publish_jobs SET state=?1,payload=?2 WHERE job_id=?3 AND state=?4",
                params![final_state, payload.to_string(), id, operation],
            )
            .map_err(err)?;
        self.get_document_publish_status(id)
    }
}

pub(crate) fn recover(service: &KnowledgeService) -> Result<()> {
    let c = service.db()?;
    let mut query = c
        .prepare("SELECT job_id FROM document_publish_jobs WHERE state IN ('applying','restoring')")
        .map_err(err)?;
    let rows = query
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(query);
    for id in rows {
        Uuid::parse_str(&id).map_err(err)?;
        let journal = files::result(&stage(service, &id).join("status.json"));
        let state = if journal.is_ok_and(|value| value["state"] == "unknown") {
            "failed"
        } else {
            "recoveryRequired"
        };
        c.execute(
            "UPDATE document_publish_jobs SET state=?1 WHERE job_id=?2",
            params![state, id],
        )
        .map_err(err)?;
    }
    Ok(())
}

pub fn worker_main(data: &Path) -> Result<Value> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(131073)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > 131072 {
        return Err("게시 worker 요청 크기가 너무 큽니다.".into());
    }
    let request: Worker = serde_json::from_slice(&bytes).map_err(err)?;
    let engine = crate::service::Engine::connect(data.to_owned())?;
    let service = engine.knowledge();
    let c = service.db()?;
    match request {
        Worker::Inspect {
            project,
            source,
            paths,
        } => {
            let (revision, root, targets) = config_targets(&c, &project, &source)?;
            let (protocol, prior) = files::inspect(&root, &selected_targets(&targets, &paths)?)?;
            Ok(json!({"configRevision":revision,"protocol":protocol,"prior":prior}))
        }
        Worker::Apply { run } | Worker::Restore { run } => {
            let (state, value) = read_run(&c, &run)?;
            let plan = load_plan(&value)?;
            let (revision, root, targets) = config_targets(&c, &plan.project_id, &plan.source_id)?;
            if value["originalRoot"] != json!(root) {
                return Err("원래 NAS 경로가 바뀌었습니다.".into());
            }
            let selected: Vec<String> =
                serde_json::from_value(value["selectedPaths"].clone()).map_err(err)?;
            selected_targets(&targets, &selected)?;
            let stage = stage(&service, &run);
            let outcome = match state.as_str() {
                "applying" if revision == plan.config_revision => {
                    fs::create_dir_all(&stage).map_err(err)?;
                    files::apply(&root, &stage, &run, &plan.protocol, &plan.files, &selected)?
                }
                "restoring" => {
                    let restore: Vec<String> =
                        serde_json::from_value(value["restorePaths"].clone()).map_err(err)?;
                    files::restore(
                        &root,
                        &stage,
                        &run,
                        &plan.protocol,
                        &plan.files,
                        &selected,
                        &restore,
                    )?
                }
                _ => return Err("승인된 게시 실행이 없습니다.".into()),
            };
            serde_json::to_value(outcome).map_err(err)
        }
    }
}
pub fn write_worker_response(path: &Path) {
    let result = match worker_main(path) {
        Ok(value) => json!({"ok":true,"data":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    println!("{result}");
}

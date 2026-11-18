// 진행 기준을 분리해 leaf 상태와 유효 근거를 집계하고 현재 변경 원장을 남긴다.
use super::{err, jobs, models::SourceSpan, KnowledgeService, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationEvidence {
    pub subject_revision: i64,
    pub success: bool,
    pub kind: String,
    pub source_span: SourceSpan,
    #[serde(default)]
    pub fixture: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasisTask {
    pub task_id: String,
    pub title: String,
    pub revision: i64,
    pub state: String,
    pub parent_id: Option<String>,
    pub source_span: Option<SourceSpan>,
    #[serde(default)]
    pub verification_evidence: Vec<VerificationEvidence>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgressBasis {
    pub basis_id: String,
    pub version: i64,
    pub project_id: String,
    pub kind: String,
    pub name: String,
    pub tasks: Vec<BasisTask>,
    #[serde(default)]
    pub authoritative_source_ids: Vec<String>,
    #[serde(default)]
    pub source_version_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Change {
    expected_task_revision: i64,
    state: Option<String>,
    title: Option<String>,
    verification_evidence: Option<Vec<VerificationEvidence>>,
}
pub(crate) fn valid_state(s: &str) -> bool {
    matches!(
        s,
        "pending" | "inProgress" | "blocked" | "reportedComplete" | "cancelled"
    )
}
pub(crate) fn bump(c: &Connection, project: &str, expected: i64) -> Result<()> {
    if c.execute("UPDATE project_progress_control SET revision=revision+1 WHERE project_id=?1 AND revision=?2",params![project,expected]).map_err(err)?!=1{return Err("project_progress_revision_conflict".into())}
    Ok(())
}
pub(crate) fn event(
    c: &Connection,
    project: &str,
    basis: &str,
    version: i64,
    task: &Value,
    batch: Option<&str>,
) -> Result<()> {
    let seq: i64 = c
        .query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM basis_task_events WHERE project_id=?1",
            [project],
            |r| r.get(0),
        )
        .map_err(err)?;
    c.execute(
        "INSERT INTO basis_task_events VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8)",
        params![
            Uuid::new_v4().to_string(),
            project,
            seq,
            basis,
            version,
            task["taskId"].as_str().ok_or("missing task")?,
            batch,
            task.to_string()
        ],
    )
    .map_err(err)?;
    c.execute("UPDATE basis_tasks SET revision=?1,state=?2,payload=?3 WHERE basis_id=?4 AND version=?5 AND task_id=?6",params![task["revision"].as_i64(),task["state"].as_str(),task.to_string(),basis,version,task["taskId"].as_str()]).map_err(err)?;
    Ok(())
}
pub(crate) fn load_basis(c: &Connection, project: &str, id: &str) -> Result<Value> {
    let (version,text):(i64,String)=c.query_row("SELECT version,payload FROM progress_bases WHERE project_id=?1 AND basis_id=?2 AND COALESCE(json_extract(payload,'$.inactive'),0)=0 ORDER BY version DESC LIMIT 1",params![project,id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
    let mut basis: Value = serde_json::from_str(&text).map_err(err)?;
    let mut q = c
        .prepare("SELECT payload FROM basis_tasks WHERE basis_id=?1 AND version=?2 ORDER BY rowid")
        .map_err(err)?;
    let tasks = q
        .query_map(params![id, version], |r| r.get::<_, String>(0))
        .map_err(err)?
        .map(|v| serde_json::from_str::<Value>(&v.map_err(err)?).map_err(err))
        .collect::<Result<Vec<_>>>()?;
    basis["tasks"] = json!(tasks);
    Ok(basis)
}
pub fn counts(basis: &Value) -> Value {
    let tasks = basis["tasks"].as_array().cloned().unwrap_or_default();
    let active: Vec<_> = tasks.iter().filter(|t| t["state"] != "cancelled").collect();
    let leaves: Vec<_> = active
        .iter()
        .filter(|t| !tasks.iter().any(|child| child["parentId"] == t["taskId"]))
        .collect();
    let verified = |t: &Value| {
        t["state"] == "reportedComplete"
            && t["verificationEvidence"].as_array().is_some_and(|es| {
                es.iter().any(|e| {
                    e["success"] == true
                        && e["fixture"] != true
                        && e["subjectRevision"] == t["revision"]
                })
            })
    };
    json!({"total":leaves.len(),"verified":leaves.iter().filter(|t|verified(t)).count(),"reportedComplete":leaves.iter().filter(|t|t["state"]=="reportedComplete"&&!verified(t)).count(),"inProgress":leaves.iter().filter(|t|t["state"]=="inProgress").count(),"blocked":leaves.iter().filter(|t|t["state"]=="blocked").count()})
}
impl KnowledgeService {
    pub fn is_managed_project(&self, id: &str) -> Result<bool> {
        self.db()?
            .query_row(
                "SELECT mode='managed' FROM project_progress_control WHERE project_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map(|v| v.unwrap_or(false))
            .map_err(err)
    }
    pub fn save_progress_basis(&self, value: Value, expected: i64) -> Result<Value> {
        let basis: ProgressBasis = serde_json::from_value(value).map_err(err)?;
        let project = basis.project_id.clone();
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        self.require_project(&project)?;
        jobs::control(&tx, &project)?;
        self.insert_basis(&tx, basis, None)?;
        bump(&tx, &project, expected)?;
        tx.commit().map_err(err)?;
        self.get_project_progress(&project, None)
    }
    pub(crate) fn insert_basis(
        &self,
        c: &Connection,
        mut basis: ProgressBasis,
        batch: Option<&str>,
    ) -> Result<()> {
        if basis.version < 1
            || basis.basis_id.trim().is_empty()
            || basis.name.trim().is_empty()
            || basis.tasks.len() > 5000
            || !matches!(
                basis.kind.as_str(),
                "summary" | "development" | "feature" | "acceptance" | "legacy"
            )
        {
            return Err("invalid progress basis".into());
        }
        self.require_project(&basis.project_id)?;
        let old:Option<(String,i64,String)>=c.query_row("SELECT project_id,version,kind FROM progress_bases WHERE basis_id=?1 ORDER BY version DESC LIMIT 1",[&basis.basis_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        if let Some((p, version, kind)) = old {
            if p != basis.project_id || kind != basis.kind {
                return Err("changed semantics requires a new basisId".into());
            }
            if basis.version != version + 1 {
                return Err("basis_version_conflict".into());
            }
        } else if basis.version != 1 {
            return Err("first basis version must be 1".into());
        }
        let ids: HashSet<_> = basis.tasks.iter().map(|t| t.task_id.clone()).collect();
        if ids.len() != basis.tasks.len() {
            return Err("duplicate basis task".into());
        }
        let parents: HashMap<_, _> = basis
            .tasks
            .iter()
            .map(|t| (t.task_id.clone(), t.parent_id.clone()))
            .collect();
        for task in &basis.tasks {
            if task.task_id.trim().is_empty()
                || task.title.trim().is_empty()
                || task.revision < 1
                || !valid_state(&task.state)
            {
                return Err("invalid basis task".into());
            }
            let mut seen = HashSet::new();
            let mut parent = task.parent_id.as_ref();
            while let Some(id) = parent {
                if !ids.contains(id) || id == &task.task_id || !seen.insert(id) {
                    return Err("invalid task hierarchy".into());
                }
                parent = parents[id].as_ref();
            }
        }
        for source in &basis.authoritative_source_ids {
            let payload: String = c
                .query_row(
                    "SELECT payload FROM document_sources WHERE source_id=?1",
                    [source],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let s: Value = serde_json::from_str(&payload).map_err(err)?;
            if s["archived"] == true
                || s["enabled"] != true
                || (s["scope"] == "project" && s["projectId"] != basis.project_id)
            {
                return Err("basis source unavailable".into());
            }
        }
        for version in &basis.source_version_ids {
            let ok:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM document_versions v JOIN document_project_links l USING(document_id) WHERE v.version_id=?1 AND l.project_id=?2 AND l.review_state='accepted')",params![version,basis.project_id],|r|r.get(0)).map_err(err)?;
            if !ok {
                return Err("foreign basis source version".into());
            }
        }
        for task in &mut basis.tasks {
            let old:Option<String>=c.query_row("SELECT payload FROM basis_tasks WHERE basis_id=?1 AND task_id=?2 ORDER BY version DESC LIMIT 1",params![basis.basis_id,task.task_id],|r|r.get(0)).optional().map_err(err)?;
            if let Some(old) = old {
                let old: Value = serde_json::from_str(&old).map_err(err)?;
                let rev = old["revision"].as_i64().ok_or("invalid task revision")?;
                if old["title"] != task.title && task.revision <= rev {
                    task.revision = rev + 1
                } else if task.revision < rev {
                    return Err("task revision cannot decrease".into());
                }
            }
            self.validate_task_evidence(c, &basis.project_id, task)?;
        }
        let mut value = serde_json::to_value(&basis).map_err(err)?;
        value["createdByBatch"] = json!(batch);
        c.execute(
            "INSERT INTO progress_bases VALUES(?1,?2,?3,?4,?5)",
            params![
                basis.basis_id,
                basis.version,
                basis.project_id,
                basis.kind,
                value.to_string()
            ],
        )
        .map_err(err)?;
        for version in &basis.source_version_ids {
            c.execute(
                "INSERT OR IGNORE INTO document_version_pins VALUES(?1,?2)",
                params![
                    version,
                    format!("basis:{}:{}", basis.basis_id, basis.version)
                ],
            )
            .map_err(err)?;
        }
        for task in &basis.tasks {
            let value = serde_json::to_value(task).map_err(err)?;
            c.execute(
                "INSERT INTO basis_tasks VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    basis.basis_id,
                    basis.version,
                    task.task_id,
                    task.revision,
                    task.state,
                    value.to_string()
                ],
            )
            .map_err(err)?;
            event(
                c,
                &basis.project_id,
                &basis.basis_id,
                basis.version,
                &value,
                batch,
            )?;
        }
        Ok(())
    }
    pub(crate) fn validate_task_evidence(
        &self,
        c: &Connection,
        project: &str,
        task: &BasisTask,
    ) -> Result<()> {
        let spans: Vec<_> = task
            .verification_evidence
            .iter()
            .map(|e| e.source_span.clone())
            .chain(task.source_span.clone())
            .collect();
        for span in spans {
            let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM document_versions v JOIN document_project_links l USING(document_id) WHERE v.version_id=?1 AND v.sha256=?2 AND l.project_id=?3 AND l.review_state='accepted' AND v.kind!='generatedPublication')",params![span.version_id,span.sha256,project],|r|r.get(0)).map_err(err)?;
            if !valid {
                return Err("foreign evidence".into());
            }
            let job = json!({"inputs":[{"versionId":span.version_id,"sha256":span.sha256}]});
            self.validate_spans(&job, std::slice::from_ref(&span))?;
            c.execute(
                "INSERT OR IGNORE INTO document_version_pins VALUES(?1,?2)",
                params![
                    span.version_id,
                    format!("task:{project}:{}:{}", task.task_id, task.revision)
                ],
            )
            .map_err(err)?;
        }
        for evidence in &task.verification_evidence {
            if evidence.subject_revision < 1
                || !matches!(evidence.kind.as_str(), "automated" | "manual" | "review")
            {
                return Err("invalid verification evidence".into());
            }
        }
        Ok(())
    }
    pub fn update_progress_task(
        &self,
        project: &str,
        basis: &str,
        version: i64,
        task_id: &str,
        change: Value,
        expected: i64,
    ) -> Result<Value> {
        let change: Change = serde_json::from_value(change).map_err(err)?;
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        self.require_project(project)?;
        let b = load_basis(&tx, project, basis)?;
        if b["version"] != version {
            return Err("basis_version_conflict".into());
        }
        let mut task: BasisTask = serde_json::from_value(
            b["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["taskId"] == task_id)
                .cloned()
                .ok_or("task missing")?,
        )
        .map_err(err)?;
        if task.revision != change.expected_task_revision {
            return Err("task_revision_conflict".into());
        }
        if let Some(state) = change.state {
            if !valid_state(&state) {
                return Err("invalid task state".into());
            }
            task.state = state
        }
        if let Some(title) = change.title {
            if title.trim().is_empty() {
                return Err("empty task title".into());
            }
            if title != task.title {
                task.revision += 1;
                task.title = title
            }
        }
        if let Some(evidence) = change.verification_evidence {
            task.verification_evidence = evidence
        }
        self.validate_task_evidence(&tx, project, &task)?;
        let value = serde_json::to_value(task).map_err(err)?;
        event(&tx, project, basis, version, &value, None)?;
        bump(&tx, project, expected)?;
        tx.commit().map_err(err)?;
        self.get_project_progress(project, Some(basis))
    }
    pub fn get_project_progress(&self, project: &str, basis_id: Option<&str>) -> Result<Value> {
        self.require_project(project)?;
        let c = self.db()?;
        let ctl = jobs::control(&c, project)?;
        let mut q=c.prepare("SELECT DISTINCT basis_id FROM progress_bases WHERE project_id=?1 AND COALESCE(json_extract(payload,'$.inactive'),0)=0 ORDER BY rowid").map_err(err)?;
        let ids = q
            .query_map([project], |r| r.get::<_, String>(0))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let bases = ids
            .iter()
            .map(|id| load_basis(&c, project, id))
            .collect::<Result<Vec<_>>>()?;
        let selected = basis_id
            .or_else(|| ctl["defaultBasisId"].as_str())
            .or_else(|| ids.first().map(String::as_str));
        let basis = selected
            .and_then(|id| bases.iter().find(|b| b["basisId"] == id))
            .cloned()
            .unwrap_or(Value::Null);
        if basis_id.is_some() && basis.is_null() {
            return Err("basis missing".into());
        }
        let count = counts(&basis);
        let total = count["total"].as_f64().unwrap_or(0.);
        let ratio = |n: f64| {
            if total > 0. {
                json!(n / total)
            } else {
                Value::Null
            }
        };
        let (freshness, at) = self.basis_freshness(&c, &basis)?;
        let mut basis = basis;
        if let Some(tasks) = basis.get_mut("tasks").and_then(Value::as_array_mut) {
            for t in tasks {
                t["verified"] = json!(
                    t["state"] == "reportedComplete"
                        && t["verificationEvidence"]
                            .as_array()
                            .is_some_and(|es| es.iter().any(|e| e["success"] == true
                                && e["fixture"] != true
                                && e["subjectRevision"] == t["revision"]))
                );
            }
        }
        Ok(
            json!({"projectId":project,"revision":ctl["revision"],"mode":ctl["mode"],"basis":basis,"bases":bases,"counts":count,"reportRatio":ratio(count["verified"].as_f64().unwrap_or(0.)+count["reportedComplete"].as_f64().unwrap_or(0.)),"verifiedRatio":ratio(count["verified"].as_f64().unwrap_or(0.)),"sourceFreshness":freshness,"sourceCheckedAt":at}),
        )
    }
    fn basis_freshness(
        &self,
        c: &Connection,
        basis: &Value,
    ) -> Result<(&'static str, Option<String>)> {
        let Some(sources) = basis["authoritativeSourceIds"]
            .as_array()
            .filter(|s| !s.is_empty())
        else {
            return Ok(("unlinked", None));
        };
        let Some(versions) = basis["sourceVersionIds"]
            .as_array()
            .filter(|v| !v.is_empty())
        else {
            return Ok(("unlinked", None));
        };
        let states = self.source_states()?;
        let mut checked = vec![];
        let mut covered = HashSet::new();
        for id in versions {
            let (source,latest,hash,seen):(String,String,String,Option<String>)=c.query_row("SELECT d.source_id,json_extract(d.payload,'$.sha256'),v.sha256,json_extract(d.payload,'$.lastSeenAt') FROM document_versions v JOIN documents d USING(document_id) WHERE v.version_id=?1",[id.as_str().ok_or("invalid version")?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?;
            if latest != hash {
                return Ok(("unappliedChange", None));
            }
            let state = states["sources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["sourceId"] == source);
            if seen.is_none()
                || state.is_none_or(|s| {
                    s["state"] != "online" || s["lastSuccessAt"].as_str() != seen.as_deref()
                })
            {
                return Ok(("stale", None));
            }
            covered.insert(source);
        }
        for source in sources {
            let Some(id) = source.as_str() else {
                return Ok(("unlinked", None));
            };
            let payload: String = c
                .query_row(
                    "SELECT payload FROM document_sources WHERE source_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let config: Value = serde_json::from_str(&payload).map_err(err)?;
            if config["enabled"] != true || config["archived"] == true {
                return Ok(("stale", None));
            }
            if !covered.contains(id) {
                return Ok(("unlinked", None));
            }
            let Some(state) = states["sources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["sourceId"] == id)
            else {
                return Ok(("stale", None));
            };
            if state["state"] != "online" {
                return Ok(("stale", None));
            }
            let Some(at) = state["lastSuccessAt"].as_str() else {
                return Ok(("stale", None));
            };
            let interval = if config["location"] == "nas" {
                super::store::config(c)?["nasIntervalSec"]
                    .as_i64()
                    .unwrap_or(30)
            } else {
                5
            };
            let valid_time = chrono::DateTime::parse_from_rfc3339(at)
                .ok()
                .is_some_and(|time| {
                    let age = chrono::Utc::now().signed_duration_since(time).num_seconds();
                    (-2..=interval.saturating_mul(3).max(30)).contains(&age)
                });
            if !valid_time {
                return Ok(("stale", None));
            }
            checked.push(at.to_owned());
        }
        checked.sort();
        Ok(("fresh", checked.first().cloned()))
    }
    pub fn activate_managed_progress(
        &self,
        project: &str,
        basis: &str,
        expected: i64,
    ) -> Result<Value> {
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        self.require_project(project)?;
        load_basis(&tx, project, basis)?;
        let mut ctl = jobs::control(&tx, project)?;
        bump(&tx, project, expected)?;
        ctl["defaultBasisId"] = json!(basis);
        ctl["autoSync"] = json!(false);
        tx.execute(
            "UPDATE project_progress_control SET mode='managed',payload=?1 WHERE project_id=?2",
            params![ctl.to_string(), project],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        self.get_project_progress(project, None)
    }
    pub fn record_observed_progress(&self) -> Result<Value> {
        let reg = self.engine.registry()?;
        let mut written = 0;
        for project in reg["projects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["archived"] != true)
        {
            let id = project["id"].as_str().ok_or("invalid project")?;
            if !self.is_managed_project(id)? {
                continue;
            }
            let view = self.get_project_progress(id, None)?;
            for b in view["bases"].as_array().unwrap() {
                let mut c = self.db()?;
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(err)?;
                let b = load_basis(&tx, id, b["basisId"].as_str().ok_or("basis missing")?)?;
                let (fresh, at) = self.basis_freshness(&tx, &b)?;
                if fresh != "fresh" {
                    continue;
                }
                let at = at.ok_or("missing source checked time")?;
                let time = chrono::DateTime::parse_from_rfc3339(&at).map_err(err)?;
                let date = time.with_timezone(&chrono::Local).date_naive().to_string();
                let count = counts(&b);
                let mut spans = Vec::new();
                for version in b["sourceVersionIds"].as_array().into_iter().flatten() {
                    let version = version.as_str().ok_or("invalid source version")?;
                    let hash: String = tx
                        .query_row(
                            "SELECT sha256 FROM document_versions WHERE version_id=?1",
                            [version],
                            |r| r.get(0),
                        )
                        .map_err(err)?;
                    let lines = self.document_text(version)?.lines().count();
                    if lines > 0 {
                        spans.push(json!({"versionId":version,"sha256":hash,"locator":{"lines":{"start":1,"end":lines}}}));
                    }
                }
                let point_id = format!(
                    "observed:{id}:{}:{}:{date}",
                    b["basisId"].as_str().unwrap(),
                    b["version"]
                );
                let point = json!({"date":date,"basisVersion":b["version"],"counts":count,"origin":"observed","coverage":"complete","sourceSpans":spans,"capturedAt":at});
                tx.execute("INSERT INTO project_progress_points VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'observed',NULL,?9) ON CONFLICT(point_id) DO UPDATE SET total=excluded.total,verified=excluded.verified,reported=excluded.reported,payload=excluded.payload WHERE julianday(json_extract(excluded.payload,'$.capturedAt'))>julianday(json_extract(project_progress_points.payload,'$.capturedAt'))",params![point_id,id,b["basisId"].as_str(),b["version"].as_i64(),date,count["total"].as_i64(),count["verified"].as_i64(),count["reportedComplete"].as_i64(),point.to_string()]).map_err(err)?;
                tx.commit().map_err(err)?;
                written += 1;
            }
        }
        Ok(json!({"written":written}))
    }
}

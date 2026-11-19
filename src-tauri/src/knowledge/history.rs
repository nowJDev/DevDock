// 과거 업무의 선택 반영과 취소를 감사 기록과 SQLite 트랜잭션으로 처리한다.
use super::{
    err, jobs,
    models::SourceSpan,
    now, parser,
    progress::{self, ProgressBasis},
    KnowledgeService, Result,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    job_id: String,
    candidate_ids: Vec<String>,
    expected_project_revision: i64,
    idempotency_key: String,
    #[serde(default = "history_mode")]
    mode: String,
    #[serde(default)]
    correction: bool,
}
fn history_mode() -> String {
    "history".into()
}
pub(crate) fn validate_changes(kind: &str, changes: &Value) -> Result<()> {
    let fields: &[&str] = match kind {
        "workEntry" => &["text", "coverage", "workItems", "result", "unfinished"],
        "historicalSnapshot" => &["coverage", "basisVersion", "counts", "tasks"],
        "currentTaskUpdate" => &["taskId", "state", "expectedTaskRevision"],
        "basisDefinition" => &[
            "basisId",
            "version",
            "projectId",
            "kind",
            "name",
            "tasks",
            "authoritativeSourceIds",
            "sourceVersionIds",
        ],
        "projectLink" => &["documentId", "linked"],
        _ => return Err("invalid candidate kind".into()),
    };
    if changes
        .as_object()
        .is_none_or(|o| o.keys().any(|k| !fields.contains(&k.as_str())))
    {
        return Err("unknown candidate change property".into());
    }
    Ok(())
}
fn candidate(c: &Connection, job: &str, id: &str) -> Result<Value> {
    let(text,state):(String,String)=c.query_row("SELECT payload,review_state FROM import_candidates WHERE candidate_id=?1 AND job_id=?2",params![id,job],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
    let mut v: Value = serde_json::from_str(&text).map_err(err)?;
    v["reviewState"] = json!(state);
    Ok(v)
}
impl KnowledgeService {
    pub fn review_import_candidate(
        &self,
        job_id: &str,
        id: &str,
        state: &str,
        edited: Option<Value>,
        reason: &str,
        expected: i64,
    ) -> Result<Value> {
        if !matches!(state, "pending" | "accepted" | "rejected") || reason.len() > 4096 {
            return Err("invalid review".into());
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let job = jobs::load_job(&tx, job_id)?;
        self.validate_job_current(&tx, &job)?;
        if job["expectedProjectRevision"] != expected {
            return Err("review_revision_conflict".into());
        }
        let mut v = candidate(&tx, job_id, id)?;
        let applied:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM import_batch_items i JOIN import_batches b USING(batch_id) WHERE i.candidate_id=?1 AND b.active=1)",[id],|r|r.get(0)).map_err(err)?;
        if applied {
            return Err("candidate already applied".into());
        }
        if let Some(edited) = edited {
            if reason.trim().is_empty() {
                return Err("edit reason required".into());
            }
            if edited.as_object().is_none_or(|o| {
                o.keys().any(|k| {
                    !["workDate", "effectiveDate", "proposedChanges"].contains(&k.as_str())
                })
            }) {
                return Err("invalid review edit".into());
            }
            let original = v.clone();
            for (key, value) in edited.as_object().unwrap() {
                if key.ends_with("Date") {
                    parser::date(value.as_str().ok_or("invalid edited date")?)?;
                }
                v[key] = value.clone();
            }
            if v.get("original").is_none() {
                v["original"] = original
            }
        }
        validate_changes(
            v["kind"].as_str().ok_or("kind missing")?,
            &v["proposedChanges"],
        )?;
        v["reviewState"] = json!(state);
        v["reviewReason"] = json!(reason);
        v["reviewedAt"] = json!(now());
        tx.execute(
            "UPDATE import_candidates SET review_state=?1,payload=?2 WHERE candidate_id=?3",
            params![state, v.to_string(), id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        self.get_import_preview(job_id)
    }
    pub fn apply_import(&self, input: Value) -> Result<Value> {
        let selection: Selection = serde_json::from_value(input).map_err(err)?;
        if selection.idempotency_key.trim().is_empty()
            || selection.idempotency_key.len() > 128
            || selection.candidate_ids.is_empty()
            || selection.candidate_ids.len() > 5000
        {
            return Err("invalid import selection".into());
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let previous: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT batch_id,payload,active FROM import_batches WHERE idempotency_key=?1",
                [&selection.idempotency_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(err)?;
        if let Some((id, text, active)) = previous {
            let old: Value = serde_json::from_str(&text).map_err(err)?;
            if old["jobId"] != selection.job_id
                || old["candidateIds"] != json!(selection.candidate_ids)
                || old["mode"] != selection.mode
                || old["correction"] != selection.correction
            {
                return Err("idempotency_key_conflict".into());
            }
            return Ok(json!({"batchId":id,"active":active==1,"duplicate":true}));
        }
        let job = jobs::load_job(&tx, &selection.job_id)?;
        if job["state"] != "completed" || job["kind"] != "progressAnalysis" {
            return Err("analysis not review ready".into());
        }
        self.validate_job_current(&tx, &job)?;
        if job["expectedProjectRevision"] != selection.expected_project_revision {
            return Err("import_revision_conflict".into());
        }
        let project = job["projectId"].as_str().ok_or("project missing")?;
        let batch_id = Uuid::new_v4().to_string();
        let mut metadata = json!({"batchId":batch_id,"jobId":selection.job_id,"candidateIds":selection.candidate_ids,"mode":selection.mode,"correction":selection.correction,"createdAt":now(),"audit":[{"action":"apply","at":now()}],"createdBases":[],"linkChanges":[]});
        tx.execute(
            "INSERT INTO import_batches VALUES(?1,?2,?3,1,?4)",
            params![
                batch_id,
                project,
                selection.idempotency_key,
                metadata.to_string()
            ],
        )
        .map_err(err)?;
        let mut seen = HashSet::new();
        for id in &selection.candidate_ids {
            if !seen.insert(id) {
                return Err("duplicate selected candidate".into());
            }
            let v = candidate(&tx, &selection.job_id, id)?;
            if v["reviewState"] != "accepted" {
                return Err("only selected accepted candidates may apply".into());
            }
            let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM import_batch_items i JOIN import_batches b USING(batch_id) WHERE i.candidate_id=?1 AND b.active=1)",[id],|r|r.get(0)).map_err(err)?;
            if duplicate {
                return Err("candidate already applied".into());
            }
            let kind = v["kind"].as_str().ok_or("kind missing")?;
            let changes = &v["proposedChanges"];
            validate_changes(kind, changes)?;
            let spans: Vec<SourceSpan> =
                serde_json::from_value(v["sourceSpans"].clone()).map_err(err)?;
            self.validate_spans(&job, &spans)?;
            match (selection.mode.as_str(), kind) {
                ("history", "workEntry") => {
                    let date = parser::date(v["workDate"].as_str().ok_or("work date missing")?)?;
                    if changes["text"].as_str().is_none_or(|s| s.trim().is_empty()) {
                        return Err("work text required".into());
                    }
                    tx.execute(
                        "INSERT INTO project_work_entries VALUES(?1,?2,?3,?4,?5)",
                        params![
                            Uuid::new_v4().to_string(),
                            project,
                            date,
                            batch_id,
                            v.to_string()
                        ],
                    )
                    .map_err(err)?;
                }
                ("history", "historicalSnapshot") => {
                    let basis_id = job["basisId"].as_str().ok_or("snapshot requires basis")?;
                    let version = job["basisVersion"]
                        .as_i64()
                        .ok_or("snapshot basis version required")?;
                    let basis =
                        progress::load_basis_version(&tx, project, basis_id, Some(version))?;
                    if changes["coverage"] != "complete"
                        || changes["basisVersion"] != basis["version"]
                        || basis["version"] != job["basisVersion"]
                    {
                        return Err("complete snapshot and matching basis required".into());
                    }
                    let states = changes["tasks"]
                        .as_array()
                        .ok_or("complete task states required")?;
                    let original = basis["tasks"].as_array().unwrap();
                    if states.len() != original.len() {
                        return Err("incomplete snapshot task set".into());
                    }
                    let mut snapshot = basis.clone();
                    let mut ids = HashSet::new();
                    for state in states {
                        if state.as_object().is_none_or(|o| {
                            o.keys().any(|k| {
                                !["taskId", "revision", "state", "verificationEvidence"]
                                    .contains(&k.as_str())
                            })
                        }) {
                            return Err("invalid snapshot state".into());
                        }
                        let task_id = state["taskId"].as_str().ok_or("task missing")?;
                        if !ids.insert(task_id) {
                            return Err("duplicate snapshot task".into());
                        }
                        let task = snapshot["tasks"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|t| t["taskId"] == task_id)
                            .ok_or("unknown snapshot task")?;
                        let known_revision=state["revision"].as_i64().is_some_and(|seen| task["revision"]==seen) ||
                            tx.query_row("SELECT EXISTS(SELECT 1 FROM basis_task_events WHERE basis_id=?1 AND version=?2 AND task_id=?3 AND active=1 AND json_extract(payload,'$.revision')=?4) OR EXISTS(SELECT 1 FROM progress_bases, json_each(progress_bases.payload,'$.tasks') WHERE basis_id=?1 AND version=?2 AND json_extract(value,'$.taskId')=?3 AND json_extract(value,'$.revision')=?4)",
                                params![basis_id,version,task_id,state["revision"].as_i64()],|row|row.get::<_,bool>(0)).map_err(err)?;
                        if !known_revision
                            || !progress::valid_state(state["state"].as_str().unwrap_or(""))
                        {
                            return Err("snapshot revision or state conflict".into());
                        }
                        task["revision"] = state["revision"].clone();
                        task["state"] = state["state"].clone();
                        task["verificationEvidence"] = state
                            .get("verificationEvidence")
                            .cloned()
                            .unwrap_or(json!([]));
                        self.validate_task_evidence(
                            &tx,
                            project,
                            &serde_json::from_value(task.clone()).map_err(err)?,
                        )?;
                    }
                    let counts = progress::counts(&snapshot);
                    if changes.get("counts").is_some()
                        && ["total", "verified", "reportedComplete"]
                            .iter()
                            .any(|key| changes["counts"][key] != counts[key])
                    {
                        return Err("snapshot count mismatch".into());
                    }
                    let date = parser::date(
                        v["effectiveDate"]
                            .as_str()
                            .or_else(|| v["workDate"].as_str())
                            .ok_or("snapshot date required")?,
                    )?;
                    let origin = if selection.correction {
                        "correction"
                    } else {
                        "reconstructed"
                    };
                    let point = json!({"date":date,"basisVersion":basis["version"],"counts":counts,"origin":origin,"coverage":"complete","sourceSpans":spans,"capturedAt":now()});
                    tx.execute("INSERT INTO project_progress_points VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![Uuid::new_v4().to_string(),project,basis_id,basis["version"].as_i64(),date,counts["total"].as_i64(),counts["verified"].as_i64(),counts["reportedComplete"].as_i64(),origin,batch_id,point.to_string()]).map_err(err)?;
                }
                ("current", "currentTaskUpdate") => {
                    let basis_id = job["basisId"]
                        .as_str()
                        .ok_or("current task requires basis")?;
                    let basis = progress::load_basis(&tx, project, basis_id)?;
                    if basis["version"] != job["basisVersion"] {
                        return Err("basis_version_conflict".into());
                    }
                    let mut task = basis["tasks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|t| t["taskId"] == changes["taskId"])
                        .cloned()
                        .ok_or("unknown current task")?;
                    if task["revision"] != changes["expectedTaskRevision"]
                        || !progress::valid_state(changes["state"].as_str().unwrap_or(""))
                    {
                        return Err("current task revision or state conflict".into());
                    }
                    task["state"] = changes["state"].clone();
                    progress::event(
                        &tx,
                        project,
                        basis_id,
                        basis["version"].as_i64().unwrap(),
                        &task,
                        Some(&batch_id),
                    )?;
                }
                ("basis", "basisDefinition") => {
                    let mut value = changes.clone();
                    if value.get("projectId").is_some() && value["projectId"] != project {
                        return Err("foreign basis project".into());
                    }
                    value["projectId"] = json!(project);
                    let basis: ProgressBasis = serde_json::from_value(value).map_err(err)?;
                    let reference = json!({"basisId":basis.basis_id,"version":basis.version});
                    self.insert_basis(&tx, basis, Some(&batch_id))?;
                    metadata["createdBases"]
                        .as_array_mut()
                        .unwrap()
                        .push(reference);
                }
                ("links", "projectLink") => {
                    let document = changes["documentId"].as_str().ok_or("document missing")?;
                    if !job["inputs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|i| i["documentId"] == document)
                    {
                        return Err("foreign document link".into());
                    }
                    let linked = changes["linked"]
                        .as_bool()
                        .ok_or("linked boolean required")?;
                    let old:Option<String>=tx.query_row("SELECT review_state FROM document_project_links WHERE document_id=?1 AND project_id=?2",params![document,project],|r|r.get(0)).optional().map_err(err)?;
                    tx.execute("INSERT INTO document_project_links VALUES(?1,?2,'manual',?3) ON CONFLICT(document_id,project_id) DO UPDATE SET method='manual',review_state=excluded.review_state",params![document,project,if linked{"accepted"}else{"rejected"}]).map_err(err)?;
                    metadata["linkChanges"].as_array_mut().unwrap().push(json!({"documentId":document,"before":old,"after":if linked{"accepted"}else{"rejected"}}));
                }
                _ => return Err("candidate kind is not allowed in this import mode".into()),
            }
            for span in &spans {
                tx.execute(
                    "INSERT OR IGNORE INTO import_batch_items VALUES(?1,?2,?3)",
                    params![batch_id, id, span.version_id],
                )
                .map_err(err)?;
            }
        }
        tx.execute(
            "UPDATE import_batches SET payload=?1 WHERE batch_id=?2",
            params![metadata.to_string(), batch_id],
        )
        .map_err(err)?;
        progress::bump(&tx, project, selection.expected_project_revision)?;
        // This job advanced the revision itself; remaining candidates still use the same inputs.
        tx.execute("UPDATE analysis_jobs SET payload=json_set(payload,'$.expectedProjectRevision',?1) WHERE job_id=?2",
            params![selection.expected_project_revision+1,selection.job_id]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(json!({"batchId":batch_id,"active":true,"applied":seen.len()}))
    }
    pub fn list_import_batches(&self, project: &str) -> Result<Value> {
        self.require_project(project)?;
        let c = self.db()?;
        let mut q=c.prepare("SELECT payload,active FROM import_batches WHERE project_id=?1 ORDER BY rowid DESC LIMIT 100").map_err(err)?;
        let rows = q
            .query_map([project], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })
            .map_err(err)?
            .map(|r| {
                let (text, active) = r.map_err(err)?;
                let mut v: Value = serde_json::from_str(&text).map_err(err)?;
                v["active"] = json!(active);
                Ok(v)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({"batches":rows}))
    }
    pub fn undo_import(&self, batch: &str, expected: i64) -> Result<Value> {
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let (project, text, active): (String, String, bool) = tx
            .query_row(
                "SELECT project_id,payload,active FROM import_batches WHERE batch_id=?1",
                [batch],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        self.require_project(&project)?;
        if !active {
            return Err("batch already inactive".into());
        }
        let mut metadata: Value = serde_json::from_str(&text).map_err(err)?;
        for basis in metadata["createdBases"].as_array().unwrap() {
            let id = basis["basisId"].as_str().unwrap();
            let version = basis["version"].as_i64().unwrap();
            let depends:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM basis_task_events WHERE basis_id=?1 AND version>=?2 AND active=1 AND (batch_id IS NULL OR batch_id<>?3)) OR EXISTS(SELECT 1 FROM project_progress_points p JOIN import_batches b USING(batch_id) WHERE p.basis_id=?1 AND p.version>=?2 AND b.active=1 AND p.batch_id<>?3)",params![id,version,batch],|r|r.get(0)).map_err(err)?;
            let ctl = jobs::control(&tx, &project)?;
            if depends || ctl["defaultBasisId"] == id {
                return Err(format!("undo_dependency_conflict: {id}@{version}"));
            }
        }
        for link in metadata["linkChanges"].as_array().unwrap() {
            let document = link["documentId"].as_str().unwrap();
            let current:Option<String>=tx.query_row("SELECT review_state FROM document_project_links WHERE document_id=?1 AND project_id=?2",params![document,project],|r|r.get(0)).optional().map_err(err)?;
            if json!(current) != link["after"] {
                return Err("undo_dependency_conflict: document link".into());
            }
            if let Some(before) = link["before"].as_str() {
                tx.execute("UPDATE document_project_links SET review_state=?1 WHERE document_id=?2 AND project_id=?3",params![before,document,project]).map_err(err)?;
            } else {
                tx.execute(
                    "DELETE FROM document_project_links WHERE document_id=?1 AND project_id=?2",
                    params![document, project],
                )
                .map_err(err)?;
            }
        }
        tx.execute(
            "UPDATE import_batches SET active=0 WHERE batch_id=?1",
            [batch],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE basis_task_events SET active=0 WHERE batch_id=?1",
            [batch],
        )
        .map_err(err)?;
        for basis in metadata["createdBases"].as_array().unwrap() {
            tx.execute("UPDATE progress_bases SET payload=json_set(payload,'$.inactive',1) WHERE basis_id=?1 AND version=?2",params![basis["basisId"].as_str(),basis["version"].as_i64()]).map_err(err)?;
        }
        replay(&tx, &project)?;
        progress::bump(&tx, &project, expected)?;
        metadata["audit"]
            .as_array_mut()
            .unwrap()
            .push(json!({"action":"undo","at":now(),"expectedProjectRevision":expected}));
        tx.execute(
            "UPDATE import_batches SET payload=?1 WHERE batch_id=?2",
            params![metadata.to_string(), batch],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(json!({"batchId":batch,"active":false}))
    }
    pub fn preview_legacy_archive(
        &self,
        project: &str,
        from: &str,
        through: &str,
    ) -> Result<Value> {
        let history = self.get_project_history(project, "legacy", from, through)?;
        let hash = legacy_archive_hash(&history["points"]);
        let revision = jobs::control(&self.db()?, project)?["revision"].clone();
        Ok(
            json!({"projectId":project,"from":from,"through":through,"points":history["points"],"previewHash":hash,"expectedProjectRevision":revision,"basisId":format!("legacy:{project}")}),
        )
    }
    pub fn archive_legacy_history(
        &self,
        project: &str,
        from: &str,
        through: &str,
        expected: i64,
        hash: &str,
    ) -> Result<Value> {
        let preview = self.preview_legacy_archive(project, from, through)?;
        if preview["previewHash"] != hash {
            return Err("legacy_archive_conflict".into());
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        self.require_project(project)?;
        let points = legacy_points(&tx, project, from, through)?;
        if points.is_empty() || legacy_archive_hash(&json!(points)) != hash {
            return Err("legacy_archive_conflict or no points".into());
        }
        let key = format!("legacy:{project}:{hash}");
        let duplicate: Option<String> = tx
            .query_row(
                "SELECT batch_id FROM import_batches WHERE idempotency_key=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if let Some(batch) = duplicate {
            return Ok(json!({"batchId":batch,"duplicate":true}));
        }
        let id = Uuid::new_v4().to_string();
        let basis = format!("legacy:{project}");
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM progress_bases WHERE basis_id=?1)",
                [&basis],
                |r| r.get(0),
            )
            .map_err(err)?;
        let created = if exists {
            vec![]
        } else {
            vec![json!({"basisId":basis,"version":1})]
        };
        let metadata = json!({"batchId":id,"kind":"legacyArchive","createdAt":now(),"createdBases":created,"linkChanges":[],"audit":[{"action":"apply","at":now()}],"previewHash":hash});
        tx.execute(
            "INSERT INTO import_batches VALUES(?1,?2,?3,1,?4)",
            params![id, project, key, metadata.to_string()],
        )
        .map_err(err)?;
        if !exists {
            let definition = json!({"basisId":basis,"version":1,"projectId":project,"name":"이전 집계 (기준 미기록)","kind":"legacy","tasks":[]});
            self.insert_basis(
                &tx,
                serde_json::from_value(definition).map_err(err)?,
                Some(&id),
            )?;
        }
        for mut point in points {
            point["basisVersion"] = json!(1);
            let counts = &point["counts"];
            tx.execute("INSERT INTO project_progress_points VALUES(?1,?2,?3,1,?4,?5,?6,?7,'observed',?8,?9)",params![Uuid::new_v4().to_string(),project,basis,point["date"].as_str(),counts["total"].as_i64(),counts["verified"].as_i64(),counts["reportedComplete"].as_i64(),id,point.to_string()]).map_err(err)?;
        }
        progress::bump(&tx, project, expected)?;
        tx.commit().map_err(err)?;
        Ok(json!({"batchId":id,"basisId":basis,"active":true}))
    }
    pub fn get_project_history(
        &self,
        project: &str,
        basis: &str,
        from: &str,
        through: &str,
    ) -> Result<Value> {
        self.require_project(project)?;
        let start =
            chrono::NaiveDate::parse_from_str(&parser::date(from)?, "%Y-%m-%d").map_err(err)?;
        let end =
            chrono::NaiveDate::parse_from_str(&parser::date(through)?, "%Y-%m-%d").map_err(err)?;
        if end < start || (end - start).num_days() >= 366 {
            return Err("history range must be 1..366 days".into());
        }
        let c = self.db()?;
        let mut raw = if basis == "legacy" {
            legacy_points(&c, project, from, through)?
        } else {
            let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM progress_bases WHERE project_id=?1 AND basis_id=?2)",params![project,basis],|r|r.get(0)).map_err(err)?;
            if !exists {
                return Err("basis missing".into());
            }
            let mut q=c.prepare("SELECT p.payload FROM project_progress_points p LEFT JOIN import_batches b USING(batch_id) WHERE p.project_id=?1 AND p.basis_id=?2 AND p.date BETWEEN ?3 AND ?4 AND (p.batch_id IS NULL OR b.active=1) ORDER BY p.rowid").map_err(err)?;
            let rows = q
                .query_map(params![project, basis, from, through], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(err)?
                .map(|v| serde_json::from_str::<Value>(&v.map_err(err)?).map_err(err))
                .collect::<Result<Vec<_>>>()?;
            rows
        };
        for point in &mut raw {
            if point["coverage"] == "legacyUnknown" && point["inProgressRecorded"] != true {
                point["counts"]["inProgress"] = Value::Null;
            }
        }
        let mut selected = BTreeMap::<(String, i64), Value>::new();
        let rank = |v: &Value| match v["origin"].as_str() {
            Some("correction") => 3,
            Some("observed") => 2,
            _ => 1,
        };
        for point in &raw {
            let key = (
                point["date"]
                    .as_str()
                    .ok_or("point date missing")?
                    .to_owned(),
                point["basisVersion"]
                    .as_i64()
                    .ok_or("point basis version missing")?,
            );
            if selected.get(&key).is_none_or(|old| {
                rank(point) > rank(old)
                    || (rank(point) == rank(old)
                        && point["capturedAt"].as_str() > old["capturedAt"].as_str())
            }) {
                selected.insert(key, point.clone());
            }
        }
        let mut q=c.prepare("SELECT e.payload FROM project_work_entries e JOIN import_batches b USING(batch_id) WHERE e.project_id=?1 AND e.date BETWEEN ?2 AND ?3 AND b.active=1 ORDER BY e.date,e.rowid").map_err(err)?;
        let work = q
            .query_map(params![project, from, through], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|v| serde_json::from_str::<Value>(&v.map_err(err)?).map_err(err))
            .collect::<Result<Vec<_>>>()?;
        raw.sort_by_key(|p| p["date"].as_str().unwrap_or("").to_owned());
        Ok(
            json!({"projectId":project,"basisId":basis,"from":from,"through":through,"points":selected.into_values().collect::<Vec<_>>(),"comparisons":raw,"workEntries":work}),
        )
    }
}
fn replay(c: &Connection, project: &str) -> Result<()> {
    let mut q=c.prepare("SELECT basis_id,version,payload FROM progress_bases WHERE project_id=?1 AND COALESCE(json_extract(payload,'$.inactive'),0)=0").map_err(err)?;
    let bases = q
        .query_map([project], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(q);
    for (id, version, text) in bases {
        let v: Value = serde_json::from_str(&text).map_err(err)?;
        for task in v["tasks"].as_array().ok_or("missing initial tasks")? {
            c.execute("UPDATE basis_tasks SET revision=?1,state=?2,payload=?3 WHERE basis_id=?4 AND version=?5 AND task_id=?6",params![task["revision"].as_i64(),task["state"].as_str(),task.to_string(),id,version,task["taskId"].as_str()]).map_err(err)?;
        }
    }
    let mut q=c.prepare("SELECT basis_id,version,payload FROM basis_task_events WHERE project_id=?1 AND active=1 ORDER BY sequence").map_err(err)?;
    let rows = q
        .query_map([project], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(q);
    for (id, version, text) in rows {
        let task: Value = serde_json::from_str(&text).map_err(err)?;
        c.execute("UPDATE basis_tasks SET revision=?1,state=?2,payload=?3 WHERE basis_id=?4 AND version=?5 AND task_id=?6",params![task["revision"].as_i64(),task["state"].as_str(),text,id,version,task["taskId"].as_str()]).map_err(err)?;
    }
    Ok(())
}
fn legacy_archive_hash(points: &Value) -> String {
    let mut normalized = points.clone();
    for point in normalized.as_array_mut().into_iter().flatten() {
        // 이전 보관 요청과 같은 기록은 기존 중복 확인 키를 유지한다.
        if point["inProgressRecorded"] != true {
            point["counts"]["inProgress"] = json!(0);
        }
    }
    super::scanner::sha256(normalized.to_string().as_bytes())
}

fn legacy_points(c: &Connection, project: &str, from: &str, through: &str) -> Result<Vec<Value>> {
    let mut q=c.prepare("SELECT date,at,total,verified,reported,in_progress FROM daily_progress WHERE project_id=?1 AND date BETWEEN ?2 AND ?3 UNION ALL SELECT collected_date,at,total,verified,reported,in_progress FROM progress WHERE project_id=?1 ORDER BY 2").map_err(err)?;
    let rows = q
        .query_map(params![project, from, through], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    let mut points = BTreeMap::new();
    for (date, at, total, verified, reported, in_progress) in rows {
        let Ok(time) = chrono::DateTime::parse_from_rfc3339(&at) else {
            continue;
        };
        let date =
            date.unwrap_or_else(|| time.with_timezone(&chrono::Local).date_naive().to_string());
        if date.as_str() < from || date.as_str() > through {
            continue;
        }
        let old = points
            .get(&date)
            .map(|v: &(chrono::DateTime<chrono::FixedOffset>, Value)| v.0);
        if old.is_none_or(|old| time >= old) {
            let mut point = json!({"date":date,"basisVersion":0,"counts":{"total":total,"verified":verified,"reportedComplete":reported,"inProgress":in_progress,"blocked":0},"origin":"observed","coverage":"legacyUnknown","sourceSpans":[],"capturedAt":at});
            if in_progress.is_some() {
                point["inProgressRecorded"] = json!(true);
            }
            points.insert(date, (time, point));
        }
    }
    Ok(points.into_values().map(|(_, v)| v).collect())
}

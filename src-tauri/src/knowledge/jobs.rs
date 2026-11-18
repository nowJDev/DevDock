// Codex 연결 요청을 고정된 입력·출처·만료 lease로 검증하고 자동 실행하지 않는다.
use super::{err, models::SourceSpan, now, parser, scanner::sha256, KnowledgeService, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    project_id: String,
    input_version_ids: Vec<String>,
    basis_id: Option<String>,
    basis_version: Option<i64>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CandidateInput {
    pub kind: String,
    pub work_date: Option<String>,
    pub authored_date: Option<String>,
    pub effective_date: Option<String>,
    pub source_spans: Vec<SourceSpan>,
    pub proposed_changes: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalysisResult {
    candidates: Vec<CandidateInput>,
}
pub(crate) fn control(c: &Connection, project: &str) -> Result<Value> {
    c.execute(
        "INSERT OR IGNORE INTO project_progress_control VALUES(?1,0,'legacy','{}')",
        [project],
    )
    .map_err(err)?;
    let (rev, mode, text): (i64, String, String) = c
        .query_row(
            "SELECT revision,mode,payload FROM project_progress_control WHERE project_id=?1",
            [project],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(err)?;
    let mut v: Value = serde_json::from_str(&text).map_err(err)?;
    v["revision"] = json!(rev);
    v["mode"] = json!(mode);
    Ok(v)
}
pub(crate) fn load_job(c: &Connection, id: &str) -> Result<Value> {
    let (state, text): (String, String) = c
        .query_row(
            "SELECT state,payload FROM analysis_jobs WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    let mut job: Value = serde_json::from_str(&text).map_err(err)?;
    job["state"] = json!(state);
    Ok(job)
}
pub(crate) fn check_lease(job: &Value, token: &str) -> Result<()> {
    let expiry = job["leaseExpiresAt"]
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .ok_or("lease missing")?;
    if job["state"] != "claimed" || job["leaseToken"] != token || chrono::Utc::now() >= expiry {
        return Err("analysis_lease_expired_or_conflict".into());
    }
    Ok(())
}
impl KnowledgeService {
    pub fn create_analysis_job(&self, request: Value) -> Result<Value> {
        self.create_job(request, "progressAnalysis", None)
    }
    pub(crate) fn create_job(
        &self,
        request: Value,
        kind: &str,
        extra: Option<Value>,
    ) -> Result<Value> {
        let r: Request = serde_json::from_value(request).map_err(err)?;
        self.require_project(&r.project_id)?;
        let mut ids = vec![];
        let mut hashes = HashSet::new();
        let mut bytes = 0i64;
        let mut inputs = vec![];
        let c = self.db()?;
        for id in &r.input_version_ids {
            let(hash,size,doc):(String,i64,String)=c.query_row("SELECT v.sha256,v.bytes,v.document_id FROM document_versions v JOIN document_project_links l USING(document_id) WHERE v.version_id=?1 AND l.project_id=?2 AND l.review_state='accepted' AND (l.method!='crossChecked' OR json_extract(v.payload,'$.crossCheckedProject')=l.project_id)",params![id,r.project_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(err)?;
            if hashes.insert(hash.clone()) {
                ids.push(id.clone());
                bytes += size;
                inputs.push(json!({"versionId":id,"sha256":hash,"documentId":doc}));
            }
        }
        if ids.is_empty() || ids.len() > 16 || bytes > 8 * 1024 * 1024 {
            return Err("analysis input limit: 1..16 documents, 8MiB".into());
        }
        for id in &ids {
            self.document_text(id)?;
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        self.require_project(&r.project_id)?;
        let ctl = control(&tx, &r.project_id)?;
        let basis = if let Some(id) = &r.basis_id {
            super::progress::load_basis_version(&tx, &r.project_id, id, r.basis_version)?
        } else {
            Value::Null
        };
        let id = Uuid::new_v4().to_string();
        let mut job = json!({"jobId":id,"projectId":r.project_id,"kind":kind,"state":"pending","basisId":r.basis_id,"basisVersion":basis["version"],"expectedProjectRevision":ctl["revision"],"inputVersionIds":ids,"inputs":inputs,"allowedTaskIds":basis["tasks"].as_array().map(|ts|ts.iter().map(|t|t["taskId"].clone()).collect::<Vec<_>>()).unwrap_or_default(),"createdAt":now(),"analysisVersion":1});
        if let Some(extra) = extra {
            if extra["basisViewRevision"] != ctl["revision"] {
                return Err("게시 진행 기준이 준비 중 변경되었습니다.".into());
            }
            job["publishContext"] = extra;
        }
        tx.execute(
            "INSERT INTO analysis_jobs VALUES(?1,?2,?3,'pending',?4)",
            params![id, r.project_id, kind, job.to_string()],
        )
        .map_err(err)?;
        for version in &ids {
            tx.execute(
                "INSERT OR IGNORE INTO document_version_pins VALUES(?1,?2)",
                params![version, format!("job:{id}")],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(job)
    }
    pub(crate) fn document_text(&self, id: &str) -> Result<String> {
        let c = self.db()?;
        let (hash, path): (String, String) = c
            .query_row(
                "SELECT sha256,cache_path FROM document_versions WHERE version_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        let bytes = std::fs::read(path).map_err(err)?;
        if sha256(&bytes) != hash {
            return Err("document cache hash mismatch".into());
        }
        String::from_utf8(bytes)
            .map(|s| s.trim_start_matches('\u{feff}').to_owned())
            .map_err(err)
    }
    pub fn get_analysis_job(&self, id: &str) -> Result<Value> {
        let mut v = load_job(&self.db()?, id)?;
        v.as_object_mut().unwrap().remove("leaseToken");
        Ok(v)
    }
    pub fn list_analysis_jobs(&self, project: &str) -> Result<Value> {
        self.require_project(project)?;
        let c = self.db()?;
        let mut q=c.prepare("SELECT json_object('jobId',job_id,'projectId',project_id,'kind',kind,'state',state,
            'requestId',json_extract(payload,'$.requestId'),'basisId',json_extract(payload,'$.basisId'),
            'expectedProjectRevision',json_extract(payload,'$.expectedProjectRevision'),
            'createdAt',json_extract(payload,'$.createdAt'),'inputVersionIds',json_extract(payload,'$.inputVersionIds'))
            FROM analysis_jobs WHERE project_id=?1 ORDER BY rowid DESC LIMIT 100").map_err(err)?;
        let ids = q
            .query_map([project], |r| r.get::<_, String>(0))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        Ok(
            json!({"jobs":ids.iter().map(|row|serde_json::from_str::<Value>(row).map_err(err)).collect::<Result<Vec<_>>>()?}),
        )
    }
    pub fn claim_analysis_job(&self, id: &str, owner: &str) -> Result<Value> {
        if owner.trim().is_empty() || owner.len() > 128 {
            return Err("invalid analysis owner".into());
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut job = load_job(&tx, id)?;
        let time = chrono::Utc::now();
        if job["state"] != "pending"
            && !(job["state"] == "claimed"
                && job["leaseExpiresAt"]
                    .as_str()
                    .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                    .is_some_and(|expiry| time >= expiry))
        {
            return Err("analysis_claim_conflict".into());
        }
        let first = job["firstClaimAt"]
            .as_str()
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(err)?
            .map(|at| at.with_timezone(&chrono::Utc))
            .unwrap_or(time);
        if time >= first + chrono::Duration::minutes(60) {
            return Err("analysis_total_lease_expired; create a new request".into());
        }
        job["firstClaimAt"] = json!(first.to_rfc3339());
        job["state"] = json!("claimed");
        job["owner"] = json!(owner);
        job["leaseToken"] = json!(Uuid::new_v4().to_string());
        job["claimedAt"] = json!(time.to_rfc3339());
        job["leaseExpiresAt"] = json!((time + chrono::Duration::minutes(10))
            .min(first + chrono::Duration::minutes(60))
            .to_rfc3339());
        tx.execute(
            "UPDATE analysis_jobs SET state='claimed',payload=?1 WHERE job_id=?2",
            params![job.to_string(), id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(job)
    }
    pub fn heartbeat_analysis_job(&self, id: &str, token: &str) -> Result<Value> {
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut job = load_job(&tx, id)?;
        check_lease(&job, token)?;
        let start = chrono::DateTime::parse_from_rfc3339(
            job["firstClaimAt"].as_str().ok_or("missing claim time")?,
        )
        .map_err(err)?;
        let max = start + chrono::Duration::minutes(60);
        let expiry = (chrono::Utc::now() + chrono::Duration::minutes(10))
            .min(max.with_timezone(&chrono::Utc));
        if expiry <= chrono::Utc::now() {
            return Err("analysis_lease_expired".into());
        }
        job["leaseExpiresAt"] = json!(expiry.to_rfc3339());
        tx.execute(
            "UPDATE analysis_jobs SET payload=?1 WHERE job_id=?2",
            params![job.to_string(), id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(json!({"jobId":id,"leaseExpiresAt":job["leaseExpiresAt"]}))
    }
    pub(crate) fn validate_job_current(&self, c: &Connection, job: &Value) -> Result<()> {
        let project = job["projectId"].as_str().ok_or("missing project")?;
        self.require_project(project)?;
        if control(c, project)?["revision"] != job["expectedProjectRevision"] {
            return Err("analysis_project_revision_conflict".into());
        }
        for input in job["inputs"].as_array().ok_or("missing inputs")? {
            let hash: String = c
                .query_row(
                    "SELECT json_extract(payload,'$.sha256') FROM documents WHERE document_id=?1",
                    [input["documentId"].as_str().ok_or("missing document")?],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if json!(hash) != input["sha256"] {
                return Err("analysis_source_version_conflict".into());
            }
        }
        Ok(())
    }
    pub(crate) fn validate_spans(&self, job: &Value, spans: &[SourceSpan]) -> Result<()> {
        if spans.is_empty() || spans.len() > 64 {
            return Err("source spans required".into());
        }
        for span in spans {
            if !job["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["versionId"] == span.version_id && i["sha256"] == span.sha256)
            {
                return Err("foreign source span".into());
            }
            let text = self.document_text(&span.version_id)?;
            if super::publish_files::generated(&text) {
                return Err("DevDock 게시 결과는 검증·진행 근거로 사용할 수 없습니다.".into());
            }
            let object = span.locator.as_object().ok_or("invalid locator")?;
            if object.len() != 1 {
                return Err("invalid locator".into());
            }
            if let Some(lines) = object.get("lines") {
                let start = lines["start"].as_u64().ok_or("invalid source start")?;
                let end = lines["end"].as_u64().ok_or("invalid source end")?;
                if lines.as_object().is_none_or(|o| o.len() != 2)
                    || start == 0
                    || end < start
                    || end as usize > text.lines().count()
                {
                    return Err("source line range invalid".into());
                }
            } else if let Some(pointer) = object.get("jsonPointer").and_then(Value::as_str) {
                let v: Value = serde_json::from_str(&text).map_err(err)?;
                if v.pointer(pointer).is_none() {
                    return Err("source JSON pointer invalid".into());
                }
            } else {
                return Err("unknown source locator".into());
            }
        }
        Ok(())
    }
    pub fn submit_analysis_result(&self, id: &str, token: &str, result: Value) -> Result<Value> {
        if result.to_string().len() > 8 * 1024 * 1024 {
            return Err("result byte limit".into());
        }
        let result: AnalysisResult = serde_json::from_value(result).map_err(err)?;
        if result.candidates.len() > 5000 {
            return Err("candidate limit".into());
        }
        let c = self.db()?;
        let job = load_job(&c, id)?;
        check_lease(&job, token)?;
        if job["kind"] != "progressAnalysis" {
            return Err("analysis job kind mismatch".into());
        }
        self.validate_job_current(&c, &job)?;
        let mut candidates = vec![];
        for input in result.candidates {
            super::history::validate_changes(&input.kind, &input.proposed_changes)?;
            if !matches!(
                input.kind.as_str(),
                "workEntry"
                    | "historicalSnapshot"
                    | "currentTaskUpdate"
                    | "basisDefinition"
                    | "projectLink"
            ) {
                return Err("invalid candidate kind".into());
            }
            for d in [
                &input.work_date,
                &input.authored_date,
                &input.effective_date,
            ]
            .into_iter()
            .flatten()
            {
                parser::date(d)?;
            }
            self.validate_spans(&job, &input.source_spans)?;
            if matches!(
                input.kind.as_str(),
                "historicalSnapshot" | "currentTaskUpdate"
            ) && job["basisId"].is_null()
            {
                return Err("progress candidate requires basis".into());
            }
            if input.kind == "currentTaskUpdate"
                && !job["allowedTaskIds"]
                    .as_array()
                    .unwrap()
                    .contains(&input.proposed_changes["taskId"])
            {
                return Err("foreign task id".into());
            }
            if input.kind == "workEntry"
                && (input.work_date.is_none()
                    || input.proposed_changes["text"]
                        .as_str()
                        .is_none_or(|s| s.trim().is_empty()))
            {
                return Err("dated work text required".into());
            }
            let mut v = serde_json::to_value(input).map_err(err)?;
            v.as_object_mut().unwrap().retain(|_, v| !v.is_null());
            let fingerprint =
                sha256(format!("{}:{}:{}", job["projectId"], job["basisId"], v).as_bytes());
            v["candidateId"] = json!(Uuid::new_v4().to_string());
            v["fingerprint"] = json!(fingerprint);
            v["reviewState"] = json!("pending");
            candidates.push(v);
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let job = load_job(&tx, id)?;
        check_lease(&job, token)?;
        self.validate_job_current(&tx, &job)?;
        for candidate in candidates {
            tx.execute(
                "INSERT OR IGNORE INTO import_candidates VALUES(?1,?2,?3,?4,'pending',?5)",
                params![
                    candidate["candidateId"].as_str(),
                    id,
                    candidate["fingerprint"].as_str(),
                    candidate["kind"].as_str(),
                    candidate.to_string()
                ],
            )
            .map_err(err)?;
        }
        tx.execute(
            "UPDATE analysis_jobs SET state='completed' WHERE job_id=?1",
            [id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        self.get_import_preview(id)
    }
    pub fn get_import_preview(&self, id: &str) -> Result<Value> {
        let c = self.db()?;
        let job = load_job(&c, id)?;
        let mut q = c
            .prepare(
                "SELECT c.payload,c.review_state,EXISTS(SELECT 1 FROM import_batch_items i JOIN import_batches b USING(batch_id) WHERE i.candidate_id=c.candidate_id AND b.active=1) FROM import_candidates c WHERE c.job_id=?1 ORDER BY c.rowid",
            )
            .map_err(err)?;
        let cs = q
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })
            .map_err(err)?
            .map(|r| {
                let (text, state, applied) = r.map_err(err)?;
                let mut v: Value = serde_json::from_str(&text).map_err(err)?;
                v["reviewState"] = json!(state);
                v["applied"] = json!(applied);
                Ok(v)
            })
            .collect::<Result<Vec<_>>>()?;
        let conflicts = self
            .validate_job_current(&c, &job)
            .err()
            .into_iter()
            .collect::<Vec<_>>();
        Ok(
            json!({"jobId":id,"projectId":job["projectId"],"basisId":job["basisId"],"expectedProjectRevision":job["expectedProjectRevision"],"inputVersionIds":job["inputVersionIds"],"candidates":cs,"conflicts":conflicts}),
        )
    }
    pub fn parse_analysis_job(&self, id: &str) -> Result<Value> {
        if self.get_analysis_job(id)?["kind"] != "progressAnalysis" {
            return Err("진행 분석 요청이 아닙니다.".into());
        }
        let claim = self.claim_analysis_job(id, "structured-parser")?;
        let result = (|| {
            let mut cs = vec![];
            for input in claim["inputs"].as_array().unwrap() {
                let version = input["versionId"].as_str().unwrap();
                let path:String=self.db()?.query_row("SELECT json_extract(payload,'$.path') FROM document_versions WHERE version_id=?1",[version],|r|r.get(0)).map_err(err)?;
                let mut parsed = parser::parse_document(
                    version,
                    input["sha256"].as_str().unwrap(),
                    &self.document_text(version)?,
                    &path,
                )?;
                for v in &mut parsed {
                    v.as_object_mut().unwrap().remove("fingerprint");
                    v.as_object_mut().unwrap().remove("reviewState");
                }
                cs.extend(parsed)
            }
            self.submit_analysis_result(
                id,
                claim["leaseToken"].as_str().unwrap(),
                json!({"candidates":cs}),
            )
        })();
        if result.is_err() {
            self.db()?.execute(
                "UPDATE analysis_jobs SET state='failed',payload=json_set(json_remove(payload,'$.leaseToken'),'$.failure','structuredParser') WHERE job_id=?1 AND state='claimed' AND json_extract(payload,'$.leaseToken')=?2",
                params![id,claim["leaseToken"].as_str()],
            ).map_err(err)?;
        }
        result
    }
}

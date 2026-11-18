// 기존 설정 파일을 보존하며 문서 관리 원장과 충돌 방지 저장을 제공한다.
use super::{err, models::*, KnowledgeService, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};
pub(crate) fn initialize(c: &mut Connection) -> Result<()> {
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    tx.execute_batch(r#"
CREATE TABLE IF NOT EXISTS document_config(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision>=0),payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS document_sources(source_id TEXT PRIMARY KEY,payload TEXT NOT NULL CHECK(json_valid(payload)),connection_state TEXT NOT NULL DEFAULT 'unknown',last_success_at TEXT,last_error TEXT);
CREATE TABLE IF NOT EXISTS documents(document_id TEXT PRIMARY KEY,source_id TEXT NOT NULL REFERENCES document_sources(source_id) ON DELETE RESTRICT,normalized_path TEXT NOT NULL,payload TEXT NOT NULL CHECK(json_valid(payload)),UNIQUE(source_id,normalized_path));
CREATE TABLE IF NOT EXISTS document_versions(version_id TEXT PRIMARY KEY,document_id TEXT NOT NULL REFERENCES documents(document_id) ON DELETE RESTRICT,sha256 TEXT NOT NULL,cache_path TEXT NOT NULL,bytes INTEGER NOT NULL CHECK(bytes>=0),captured_at TEXT NOT NULL,authored_date TEXT,kind TEXT NOT NULL,payload TEXT NOT NULL CHECK(json_valid(payload)),UNIQUE(document_id,sha256));
CREATE TABLE IF NOT EXISTS document_project_links(document_id TEXT NOT NULL REFERENCES documents(document_id) ON DELETE RESTRICT,project_id TEXT NOT NULL,method TEXT NOT NULL,review_state TEXT NOT NULL CHECK(review_state IN ('pending','accepted','rejected','conflict')),PRIMARY KEY(document_id,project_id));
CREATE TABLE IF NOT EXISTS document_version_pins(version_id TEXT NOT NULL REFERENCES document_versions(version_id) ON DELETE RESTRICT,owner TEXT NOT NULL,PRIMARY KEY(version_id,owner));
CREATE TABLE IF NOT EXISTS progress_bases(basis_id TEXT NOT NULL,version INTEGER NOT NULL CHECK(version>=1),project_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('summary','development','feature','acceptance','legacy')),payload TEXT NOT NULL CHECK(json_valid(payload)),PRIMARY KEY(basis_id,version));
CREATE TABLE IF NOT EXISTS basis_tasks(basis_id TEXT NOT NULL,version INTEGER NOT NULL,task_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>=0),state TEXT NOT NULL CHECK(state IN ('pending','inProgress','blocked','reportedComplete','cancelled')),payload TEXT NOT NULL CHECK(json_valid(payload)),PRIMARY KEY(basis_id,version,task_id),FOREIGN KEY(basis_id,version) REFERENCES progress_bases(basis_id,version) ON DELETE RESTRICT);
CREATE TABLE IF NOT EXISTS import_batches(batch_id TEXT PRIMARY KEY,project_id TEXT NOT NULL,idempotency_key TEXT NOT NULL UNIQUE,active INTEGER NOT NULL CHECK(active IN (0,1)),payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS basis_task_events(event_id TEXT PRIMARY KEY,project_id TEXT NOT NULL,sequence INTEGER NOT NULL CHECK(sequence>=0),basis_id TEXT NOT NULL,version INTEGER NOT NULL,task_id TEXT NOT NULL,batch_id TEXT REFERENCES import_batches(batch_id) ON DELETE RESTRICT,active INTEGER NOT NULL CHECK(active IN (0,1)),payload TEXT NOT NULL CHECK(json_valid(payload)),UNIQUE(project_id,sequence),FOREIGN KEY(basis_id,version,task_id) REFERENCES basis_tasks(basis_id,version,task_id) ON DELETE RESTRICT);
CREATE TABLE IF NOT EXISTS project_progress_control(project_id TEXT PRIMARY KEY,revision INTEGER NOT NULL CHECK(revision>=0),mode TEXT NOT NULL CHECK(mode IN ('legacy','managed')),payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS analysis_jobs(job_id TEXT PRIMARY KEY,project_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('progressAnalysis','dailyReport','documentPublishDraft')),state TEXT NOT NULL CHECK(state IN ('pending','claimed','completed','expired','failed','resultExpired')),payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS import_candidates(candidate_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES analysis_jobs(job_id) ON DELETE RESTRICT,fingerprint TEXT NOT NULL UNIQUE,kind TEXT NOT NULL CHECK(kind IN ('workEntry','historicalSnapshot','currentTaskUpdate','basisDefinition','projectLink')),review_state TEXT NOT NULL CHECK(review_state IN ('pending','accepted','rejected','conflict')),payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS import_batch_items(batch_id TEXT NOT NULL REFERENCES import_batches(batch_id) ON DELETE RESTRICT,candidate_id TEXT NOT NULL REFERENCES import_candidates(candidate_id) ON DELETE RESTRICT,version_id TEXT NOT NULL REFERENCES document_versions(version_id) ON DELETE RESTRICT,PRIMARY KEY(batch_id,candidate_id,version_id));
CREATE TABLE IF NOT EXISTS project_work_entries(entry_id TEXT PRIMARY KEY,project_id TEXT NOT NULL,date TEXT NOT NULL,batch_id TEXT NOT NULL REFERENCES import_batches(batch_id) ON DELETE RESTRICT,payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS project_progress_points(point_id TEXT PRIMARY KEY,project_id TEXT NOT NULL,basis_id TEXT NOT NULL,version INTEGER NOT NULL,date TEXT NOT NULL,total INTEGER NOT NULL CHECK(total>=0),verified INTEGER NOT NULL CHECK(verified>=0),reported INTEGER NOT NULL CHECK(reported>=0),origin TEXT NOT NULL CHECK(origin IN ('observed','reconstructed','correction')),batch_id TEXT REFERENCES import_batches(batch_id) ON DELETE RESTRICT,payload TEXT NOT NULL CHECK(json_valid(payload)),FOREIGN KEY(basis_id,version) REFERENCES progress_bases(basis_id,version) ON DELETE RESTRICT);
CREATE TABLE IF NOT EXISTS project_report_profiles(project_id TEXT PRIMARY KEY,payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS report_requests(request_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES analysis_jobs(job_id) ON DELETE RESTRICT,payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS document_publish_jobs(job_id TEXT PRIMARY KEY,project_id TEXT NOT NULL,state TEXT NOT NULL,payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE IF NOT EXISTS document_publish_items(job_id TEXT NOT NULL REFERENCES document_publish_jobs(job_id) ON DELETE RESTRICT,relative_path TEXT NOT NULL,payload TEXT NOT NULL CHECK(json_valid(payload)),PRIMARY KEY(job_id,relative_path));
CREATE INDEX IF NOT EXISTS knowledge_versions_document ON document_versions(document_id,captured_at);
CREATE INDEX IF NOT EXISTS knowledge_history_project ON project_progress_points(project_id,date);
"#).map_err(err)?;
    tx.execute(
        "INSERT OR IGNORE INTO document_config VALUES(1,0,?1)",
        [json!({"nasIntervalSec":30,"localIntervalSec":5}).to_string()],
    )
    .map_err(err)?;
    tx.commit().map_err(err)
}
pub(crate) fn config(c: &Connection) -> Result<Value> {
    let (rev, text): (i64, String) = c
        .query_row(
            "SELECT revision,payload FROM document_config WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    let mut v: Value = serde_json::from_str(&text).map_err(err)?;
    v["revision"] = json!(rev);
    let mut q = c
        .prepare("SELECT payload FROM document_sources ORDER BY source_id")
        .map_err(err)?;
    let sources = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(err)?
        .map(|r| serde_json::from_str::<Value>(&r.map_err(err)?).map_err(err))
        .collect::<Result<Vec<_>>>()?;
    v["sources"] = json!(sources);
    Ok(v)
}
pub(crate) fn bump(c: &Connection, expected: u64) -> Result<()> {
    if c.execute(
        "UPDATE document_config SET revision=revision+1 WHERE id=1 AND revision=?1",
        [i64::try_from(expected).map_err(err)?],
    )
    .map_err(err)?
        != 1
    {
        return Err("document_config_conflict".into());
    }
    Ok(())
}
impl KnowledgeService {
    pub fn save_document_config(&self, value: Value, expected: u64) -> Result<Value> {
        let v: DocumentConfigView = serde_json::from_value(value).map_err(err)?;
        if !(5..=3600).contains(&v.nas_interval_sec)
            || v.local_interval_sec != 5
            || v.revision != expected
        {
            return Err("invalid document intervals or revision".into());
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let current = config(&tx)?;
        if serde_json::to_value(&v.sources).map_err(err)? != current["sources"] {
            return Err("edit sources through upsert_document_source".into());
        }
        bump(&tx, expected)?;
        tx.execute(
            "UPDATE document_config SET payload=?1 WHERE id=1",
            [json!({"nasIntervalSec":v.nas_interval_sec,"localIntervalSec":5}).to_string()],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        self.get_document_config()
    }
    pub fn upsert_document_source(&self, value: Value, expected: u64) -> Result<Value> {
        let s: DocumentSource = serde_json::from_value(value).map_err(err)?;
        if s.source_id.trim().is_empty()
            || s.source_id.len() > 128
            || !['/', '\\'].iter().all(|x| !s.source_id.contains(*x))
            || s.root.trim().is_empty()
            || s.root.contains('\0')
            || !std::path::Path::new(&s.root).is_absolute()
            || !matches!(s.location.as_str(), "local" | "nas")
            || s.roles.is_empty()
            || s.roles.iter().any(|r| {
                !matches!(
                    r.as_str(),
                    "handover"
                        | "dailyLog"
                        | "checklist"
                        | "review"
                        | "reference"
                        | "publicationReceipt"
                )
            })
        {
            return Err("invalid document source".into());
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        match (s.scope.as_str(), s.project_id.as_deref()) {
            ("project", Some(id)) => self.require_project(id)?,
            ("global", None) => {}
            _ => return Err("invalid source scope".into()),
        }
        bump(&tx, expected)?;
        tx.execute("INSERT INTO document_sources(source_id,payload) VALUES(?1,?2) ON CONFLICT(source_id) DO UPDATE SET payload=excluded.payload",params![s.source_id,serde_json::to_string(&s).map_err(err)?]).map_err(err)?;
        tx.commit().map_err(err)?;
        self.get_document_config()
    }
    pub fn archive_document_source(&self, id: &str, expected: u64) -> Result<Value> {
        let mut s = self.get_document_config()?["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["sourceId"] == id)
            .cloned()
            .ok_or("source missing")?;
        s["archived"] = json!(true);
        s["enabled"] = json!(false);
        self.upsert_document_source(s, expected)
    }
}

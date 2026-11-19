// 등록 정보, 관측 및 근태 원장을 관리한다.
use chrono::{
    DateTime, Days, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat,
    Timelike, Utc,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
type R<T> = Result<T, String>;
const DAILY_PROGRESS_RETENTION_DAYS: u32 = 90;
const HOURS_PER_DAY: u32 = 24;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn required<'a>(v: &'a Value, k: &str) -> R<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing {k}"))
}
fn now() -> String {
    Local::now().to_rfc3339()
}
#[derive(Clone)]
pub struct Engine {
    data_dir: PathBuf,
}
impl Engine {
    pub fn open(data_dir: PathBuf) -> R<Self> {
        Self::initialize(data_dir, true)
    }
    pub fn connect(data_dir: PathBuf) -> R<Self> {
        Self::initialize(data_dir, false)
    }
    fn initialize(data_dir: PathBuf, recover: bool) -> R<Self> {
        fs::create_dir_all(&data_dir).map_err(err)?;
        let e = Self { data_dir };
        let mut c = e.db()?;
        let schema = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        schema.execute_batch("CREATE TABLE IF NOT EXISTS runs(attempt_id TEXT PRIMARY KEY,date TEXT NOT NULL,action TEXT NOT NULL,state TEXT NOT NULL,result TEXT,UNIQUE(date,action)); CREATE TABLE IF NOT EXISTS prompts(date TEXT PRIMARY KEY,answer INTEGER); CREATE TABLE IF NOT EXISTS checkin_prompts(date TEXT PRIMARY KEY,answer INTEGER); CREATE TABLE IF NOT EXISTS observations(id INTEGER PRIMARY KEY,daemon_id TEXT NOT NULL,at TEXT NOT NULL,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY,daemon_id TEXT NOT NULL,at TEXT NOT NULL,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS progress(id INTEGER PRIMARY KEY,at TEXT NOT NULL,project_id TEXT NOT NULL,total INTEGER,verified INTEGER,reported INTEGER,collected_date TEXT,in_progress INTEGER); CREATE TABLE IF NOT EXISTS fingerprints(project_id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL); CREATE TABLE IF NOT EXISTS observation_minutes(daemon_id TEXT NOT NULL,minute TEXT NOT NULL,sample_count INTEGER NOT NULL,success_count INTEGER NOT NULL,failure_count INTEGER NOT NULL,unknown_count INTEGER NOT NULL,latency_sum REAL NOT NULL,latency_count INTEGER NOT NULL,PRIMARY KEY(daemon_id,minute)); CREATE INDEX IF NOT EXISTS idx_observations_source_time ON observations(daemon_id,julianday(at)); CREATE INDEX IF NOT EXISTS idx_events_source_time ON events(daemon_id,julianday(at)); CREATE INDEX IF NOT EXISTS idx_progress_project_time ON progress(project_id,julianday(at)); CREATE INDEX IF NOT EXISTS idx_events_time ON events(julianday(at));").map_err(err)?;
        schema
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS daily_progress(
                project_id TEXT NOT NULL,
                date TEXT NOT NULL,
                at TEXT NOT NULL,
                total INTEGER NOT NULL,
                verified INTEGER NOT NULL,
                reported INTEGER NOT NULL,
                tracking_basis TEXT,
                source_captured_at TEXT,
                in_progress INTEGER,
                PRIMARY KEY(project_id,date)
            );
            CREATE INDEX IF NOT EXISTS idx_daily_progress_date ON daily_progress(date);",
            )
            .map_err(err)?;
        let has_collected_date = schema.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('progress') WHERE name='collected_date')",
            [],
            |r| r.get::<_, bool>(0),
        ).map_err(err)?;
        if !has_collected_date {
            schema
                .execute("ALTER TABLE progress ADD COLUMN collected_date TEXT", [])
                .map_err(err)?;
        }
        for table in ["progress", "daily_progress"] {
            let has_in_progress = schema
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name='in_progress')",
                    [table],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(err)?;
            if !has_in_progress {
                schema
                    .execute(
                        &format!("ALTER TABLE {table} ADD COLUMN in_progress INTEGER"),
                        [],
                    )
                    .map_err(err)?;
            }
        }
        schema.commit().map_err(err)?;
        crate::knowledge::store::initialize(&mut c)?;
        if recover {
            crate::knowledge::reports::expire_on_start(&e.knowledge())?;
            crate::knowledge::publisher::recover(&e.knowledge())?;
            c.execute(
                "UPDATE runs SET state='uncertain' WHERE state='pending'",
                [],
            )
            .map_err(err)?;
        }
        for (file, default) in [
            (
                "registry.json",
                json!({"schemaVersion":1,"revision":0,"projects":[],"monitors":[{"id":"daemon-A","name":"A","kind":"daemon","source":"A","enabled":true},{"id":"daemon-B","name":"B","kind":"daemon","source":"B","enabled":true}],"tasks":[],"evidence":[]}),
            ),
            ("settings.json", Self::default_settings()),
        ] {
            let p = e.data_dir.join(file);
            if !p.exists() {
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(p)
                    .map_err(err)?;
                f.write_all(default.to_string().as_bytes()).map_err(err)?;
                f.sync_all().map_err(err)?;
            }
        }
        Ok(e)
    }
    pub fn knowledge(&self) -> crate::knowledge::KnowledgeService {
        crate::knowledge::KnowledgeService::new(self.clone())
    }
    pub(crate) fn db(&self) -> R<Connection> {
        let c = Connection::open(self.data_dir.join("devdock.sqlite3")).map_err(err)?;
        c.busy_timeout(Duration::from_secs(5)).map_err(err)?;
        c.pragma_update(None, "foreign_keys", true).map_err(err)?;
        Ok(c)
    }
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    fn read(&self, name: &str) -> R<Value> {
        serde_json::from_slice(&fs::read(self.data_dir.join(name)).map_err(err)?).map_err(err)
    }
    fn write(&self, name: &str, v: &Value) -> R<()> {
        let tmp = self
            .data_dir
            .join(format!(".{name}.{}.tmp", Uuid::new_v4()));
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(err)?;
        f.write_all(serde_json::to_string_pretty(v).map_err(err)?.as_bytes())
            .map_err(err)?;
        f.sync_all().map_err(err)?;
        drop(f);
        if let Err(e) = fs::rename(&tmp, self.data_dir.join(name)) {
            let _ = fs::remove_file(&tmp);
            return Err(err(e));
        }
        Ok(())
    }
    fn default_settings() -> Value {
        json!({"schemaVersion":1,"revision":0,"attendanceEnabled":true,"startTime":"08:10","endTime":"08:30","checkoutTime":"17:30","smartoneScript":"","autostartEnabled":false,"retentionDays":7})
    }
    pub fn registry(&self) -> R<Value> {
        let v = self.read("registry.json")?;
        if v["schemaVersion"] != 1
            || v["revision"].as_u64().is_none()
            || ["projects", "monitors", "tasks", "evidence"]
                .iter()
                .any(|k| !v[*k].is_array())
        {
            return Err("invalid registry".into());
        }
        Ok(v)
    }
    pub fn settings(&self) -> R<Value> {
        let mut v = self.read("settings.json")?;
        if v.get("revision").is_none() {
            v["revision"] = json!(0);
        }
        Self::validate_settings(&v)?;
        Ok(v)
    }
    fn validate_settings(v: &Value) -> R<()> {
        if v["schemaVersion"] != 1
            || v["revision"].as_u64().is_none()
            || !v["attendanceEnabled"].is_boolean()
            || !v["autostartEnabled"].is_boolean()
            || !v["smartoneScript"].is_string()
            || v["retentionDays"].as_u64().filter(|x| *x > 0).is_none()
        {
            return Err("invalid settings".into());
        }
        let parse = |k| NaiveTime::parse_from_str(required(v, k)?, "%H:%M").map_err(err);
        let a = parse("startTime")?;
        let b = parse("endTime")?;
        parse("checkoutTime")?;
        if a >= b {
            return Err("invalid attendance window".into());
        }
        Ok(())
    }
    pub fn save_settings(&self, v: Value) -> R<Value> {
        self.save_settings_with(v, |_, _| Ok(()))
    }
    pub fn save_settings_with<F>(&self, mut v: Value, mut apply: F) -> R<Value>
    where
        F: FnMut(&Value, &Value) -> R<()>,
    {
        Self::validate_settings(&v)?;
        let expected = v["revision"].as_u64().ok_or("missing settings revision")?;
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let old = self.settings()?;
        if old["revision"].as_u64() != Some(expected) {
            return Err("settings revision conflict".into());
        }
        v["revision"] = json!(expected
            .checked_add(1)
            .ok_or("settings revision overflow")?);
        apply(&old, &v)?;
        if let Err(error) = self.write("settings.json", &v) {
            return match apply(&v, &old) {
                Ok(()) => Err(error),
                Err(rollback) => Err(format!("{error}; rollback failed: {rollback}")),
            };
        }
        if let Err(error) = tx.commit() {
            let file_rollback = self.write("settings.json", &old);
            let external_rollback = apply(&v, &old);
            return Err(format!(
                "{error}; file rollback: {:?}; external rollback: {:?}",
                file_rollback.err(),
                external_rollback.err()
            ));
        }
        Ok(v)
    }
    pub fn mutate(&self, op: Value, expected_revision: Option<u64>) -> R<Value> {
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut reg = self.registry()?;
        let revision = reg["revision"].as_u64().ok_or("invalid revision")?;
        if expected_revision.is_some_and(|x| x != revision) {
            return Err("revision conflict".into());
        }
        let entity = required(&op, "entity")?;
        let action = required(&op, "action")?;
        let value = op.get("value").cloned().unwrap_or(json!({}));
        if matches!(entity, "task" | "status" | "evidence") {
            let project = value["projectId"].as_str().or_else(|| {
                reg["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["id"] == op["id"] || t["id"] == value["taskId"])
                    .and_then(|t| t["projectId"].as_str())
            });
            if let Some(project) = project {
                let managed: bool = tx
                    .query_row(
                        "SELECT mode='managed' FROM project_progress_control WHERE project_id=?1",
                        [project],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(err)?
                    .unwrap_or(false);
                if managed {
                    return Err("managed project: use documents progress/task commands".into());
                }
            }
        }
        if entity == "status" && action == "publish" {
            Self::validate_status(&value)?;
            let project = required(&value, "projectId")?.to_owned();
            if !reg["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == project.as_str() && p["archived"] != true)
            {
                return Err("project not found".into());
            }
            if !reg["statuses"].is_object() {
                reg["statuses"] = json!({});
            }
            reg["statuses"][&project] = value;
        } else {
            let key = match entity {
                "project" => "projects",
                "monitor" => "monitors",
                "task" => "tasks",
                "evidence" => "evidence",
                _ => return Err("unsupported entity".into()),
            };
            let id = op
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.get("id").and_then(Value::as_str))
                .filter(|s| !s.is_empty())
                .ok_or("missing id")?
                .to_owned();
            if action == "upsert" {
                Self::validate_entity(entity, &value)?;
                if entity == "monitor" {
                    let kind = required(&value, "kind")?;
                    if kind == "daemon" && !["A", "B"].contains(&required(&value, "source")?) {
                        return Err("unsupported daemon source".into());
                    }
                    if kind != "daemon" {
                        let project = required(&value, "projectId")?;
                        if !reg["projects"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|p| p["id"] == project && p["archived"] != true)
                        {
                            return Err("monitor project not found".into());
                        }
                    }
                }

                if op
                    .get("id")
                    .is_some_and(|id| id.as_str() != value["id"].as_str())
                {
                    return Err("id mismatch".into());
                }
            }
            let items = reg[key].as_array_mut().ok_or("invalid registry")?;
            let mut ids = std::collections::HashSet::new();
            if items
                .iter()
                .any(|v| v["id"].as_str().is_none_or(|id| !ids.insert(id.to_owned())))
            {
                return Err("duplicate or missing id".into());
            }
            let index = items.iter().position(|x| x["id"] == id);
            match action {
                "upsert" => {
                    if let Some(i) = index {
                        items[i] = value
                    } else {
                        items.push(value)
                    }
                }
                "archive" | "remove" => {
                    let i = index.ok_or("not found")?;
                    items[i]["archived"] = json!(true)
                }
                _ => return Err("unsupported action".into()),
            }
        }
        reg["revision"] = json!(revision + 1);
        self.write("registry.json", &reg)?;
        tx.commit().map_err(err)?;
        self.snapshot()?;
        Ok(reg)
    }
    fn validate_entity(entity: &str, v: &Value) -> R<()> {
        required(v, "id")?;
        match entity {
            "project" => {
                required(v, "name")?;
                required(v, "root")?;
            }
            "monitor" => {
                required(v, "name")?;
                required(v, "source")?;
                if !["daemon", "checklist", "structuredStatus"].contains(&required(v, "kind")?)
                    || !v["enabled"].is_boolean()
                {
                    return Err("invalid monitor".into());
                }
            }
            "task" => {
                required(v, "projectId")?;
                required(v, "title")?;
                required(v, "revision")?;
                if ![
                    "pending",
                    "inProgress",
                    "blocked",
                    "reportedComplete",
                    "cancelled",
                ]
                .contains(&required(v, "state")?)
                {
                    return Err("invalid task state".into());
                }
            }
            "evidence" => {
                for k in ["taskId", "kind", "subjectRevision", "reference"] {
                    required(v, k)?;
                }
                if !v["success"].is_boolean() {
                    return Err("invalid evidence".into());
                }
            }
            _ => return Err("invalid entity".into()),
        }
        Ok(())
    }
    fn validate_status(v: &Value) -> R<()> {
        let project = required(v, "projectId")?;
        if let Some(x) = v.get("currentTask") {
            if !x.is_string() {
                return Err("invalid currentTask".into());
            }
        }
        if let Some(tasks) = v.get("tasks") {
            for task in tasks.as_array().ok_or("invalid status tasks")? {
                Self::validate_entity("task", task)?;
                if task["projectId"] != project {
                    return Err("status project mismatch".into());
                }
            }
        }
        if let Some(metrics) = v.get("metrics") {
            for m in metrics.as_array().ok_or("invalid metrics")? {
                for k in ["id", "unit", "state"] {
                    required(m, k)?;
                }
                if m.get("value").is_none() || (!m["value"].is_null() && !m["value"].is_number()) {
                    return Err("invalid metric value".into());
                }
            }
        }
        Ok(())
    }
    pub fn evaluate_schedule(&self, iso: &str) -> R<Value> {
        let t = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S")
            .or_else(|_| chrono::DateTime::parse_from_rfc3339(iso).map(|x| x.naive_local()))
            .map_err(err)?;
        let s = self.settings()?;
        let p = |k| NaiveTime::parse_from_str(s[k].as_str().unwrap_or(""), "%H:%M").map_err(err);
        let enabled = s["attendanceEnabled"] == true;
        Ok(
            json!({"date":t.date().to_string(),"checkIn":enabled && t.time()>=p("startTime")? && t.time()<p("endTime")?,"checkoutPrompt":enabled && t.time()>=p("checkoutTime")?}),
        )
    }
    pub fn claim_run(&self, date: &str, action: &str) -> R<Option<String>> {
        NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(err)?;
        if !["checkIn", "checkOut"].contains(&action) {
            return Err("invalid action".into());
        }
        let id = Uuid::new_v4().to_string();
        let c = self.db()?;
        let count=c.execute("INSERT OR IGNORE INTO runs(attempt_id,date,action,state) VALUES(?1,?2,?3,'pending')",params![id,date,action]).map_err(err)?;
        Ok(if count == 1 { Some(id) } else { None })
    }
    pub fn finish_run(&self, id: &str, result: Value) -> R<Value> {
        let state = required(&result, "state")?;
        if ![
            "recorded",
            "alreadyIn",
            "alreadyOut",
            "refused",
            "uncertain",
            "failed",
        ]
        .contains(&state)
        {
            return Err("invalid run result".into());
        }
        let c = self.db()?;
        if c.execute(
            "UPDATE runs SET state=?1,result=?2 WHERE attempt_id=?3 AND state='pending'",
            params![state, result.to_string(), id],
        )
        .map_err(err)?
            != 1
        {
            return Err("run not pending".into());
        }
        Ok(result)
    }
    pub fn claim_prompt(&self, date: &str) -> R<bool> {
        self.claim_attendance_prompt(
            date,
            "checkOut",
            "INSERT OR IGNORE INTO prompts(date) VALUES(?1)",
        )
    }
    pub fn claim_checkin_prompt(&self, date: &str) -> R<bool> {
        self.claim_attendance_prompt(
            date,
            "checkIn",
            "INSERT OR IGNORE INTO checkin_prompts(date) VALUES(?1)",
        )
    }
    fn claim_attendance_prompt(&self, date: &str, action: &str, insert: &str) -> R<bool> {
        NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(err)?;
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let existing: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE date=?1 AND action=?2)",
                params![date, action],
                |r| r.get(0),
            )
            .map_err(err)?;
        let claimed = if existing {
            false
        } else {
            tx.execute(insert, [date]).map_err(err)? == 1
        };
        tx.commit().map_err(err)?;
        Ok(claimed)
    }
    pub fn resolve_prompt(&self, date: &str, answer: bool) -> R<Value> {
        self.resolve_prompt_at(date, answer, &Local::now().date_naive().to_string())
    }
    pub fn resolve_prompt_at(&self, date: &str, answer: bool, current_date: &str) -> R<Value> {
        self.resolve_attendance_prompt(
            date,
            answer,
            current_date,
            true,
            "UPDATE prompts SET answer=?1 WHERE date=?2 AND answer IS NULL",
        )
    }
    pub fn resolve_checkin_prompt_at(&self, date: &str, answer: bool, now: &str) -> R<Value> {
        let schedule = self.evaluate_schedule(now)?;
        self.resolve_attendance_prompt(
            date,
            answer,
            required(&schedule, "date")?,
            schedule["checkIn"] == true,
            "UPDATE checkin_prompts SET answer=?1 WHERE date=?2 AND answer IS NULL",
        )
    }
    fn resolve_attendance_prompt(
        &self,
        date: &str,
        answer: bool,
        current_date: &str,
        eligible: bool,
        update: &str,
    ) -> R<Value> {
        let target = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(err)?;
        let current = NaiveDate::parse_from_str(current_date, "%Y-%m-%d").map_err(err)?;
        if target > current {
            return Err("future prompt date".into());
        }
        let expired = target < current || !eligible;
        let answer = answer && !expired;
        let c = self.db()?;
        if c.execute(update, params![answer, date]).map_err(err)? != 1 {
            return Err("prompt not pending".into());
        }
        Ok(json!({"date":date,"answer":answer,"expired":expired}))
    }
    pub fn record_observation(&self, v: Value) -> R<Value> {
        let id = required(&v, "id")?;
        if !["A", "B"].contains(&id) {
            return Err("invalid daemon".into());
        }
        let at = required(&v, "observedAt")?;
        let state = required(&v, "transportState")?;
        if !["healthy", "slow", "disconnected", "unknown"].contains(&state) {
            return Err("invalid transport".into());
        }
        self.db()?
            .execute(
                "INSERT INTO observations(daemon_id,at,data) VALUES(?1,?2,?3)",
                params![id, at, v.to_string()],
            )
            .map_err(err)?;
        Ok(v)
    }
    pub fn record_event(&self, v: Value) -> R<Value> {
        let daemon = required(&v, "daemonId")?;
        if !["A", "B"].contains(&daemon) {
            return Err("invalid daemon".into());
        }
        let typ = v
            .get("method")
            .or_else(|| v.get("type"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("missing type")?;
        if ![
            "turn/started",
            "turn.started",
            "turn/completed",
            "turn.completed",
            "item/started",
            "item.started",
            "item/completed",
            "item.completed",
            "thread/status/changed",
            "turn/plan/updated",
            "tool.started",
            "approval",
            "input",
            "turn.failed",
        ]
        .contains(&typ)
        {
            return Err("unsupported event type".into());
        }
        let p = v.get("params").unwrap_or(&v);
        let thread = p
            .get("threadId")
            .or_else(|| v.get("threadId"))
            .and_then(Value::as_str);
        let turn = p
            .get("turnId")
            .or_else(|| p.pointer("/turn/id"))
            .or_else(|| v.get("turnId"))
            .and_then(Value::as_str);
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if v.get("at").is_some_and(|at| !at.is_string()) {
            return Err("invalid event timestamp".into());
        }
        let at = v
            .get("at")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(now);
        let at = chrono::DateTime::parse_from_rfc3339(&at)
            .map_err(err)?
            .with_timezone(&chrono::Utc)
            .to_rfc3339();
        let item = p
            .pointer("/item/type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let status = p
            .pointer("/status/type")
            .or_else(|| p.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let normalized = match typ {
            "turn/started" | "turn.started" => "working",
            "turn/completed" | "turn.completed" => {
                if p.pointer("/turn/status").and_then(Value::as_str) == Some("failed") {
                    "failed"
                } else {
                    "idle"
                }
            }
            "item/started" | "item.started" => {
                if [
                    "commandExecution",
                    "fileChange",
                    "mcpToolCall",
                    "webSearch",
                    "dynamicToolCall",
                ]
                .contains(&item)
                {
                    "toolRunning"
                } else {
                    "working"
                }
            }
            "item/completed" | "item.completed" => "working",
            "thread/status/changed" if p["status"].is_object() => {
                crate::daemon::activity_from_status(&p["status"])
            }
            "thread/status/changed" => match status {
                "active" => "working",
                "idle" => "idle",
                "systemError" => "failed",
                _ => "unknown",
            },
            "turn/plan/updated" => "plan",
            "tool.started" => "toolRunning",
            "approval" => "approval",
            "input" => "input",
            "turn.failed" => "failed",
            _ => "unknown",
        };
        let plan = if typ == "turn/plan/updated" {
            p.get("plan")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|step| {
                            let text = step["step"].as_str()?;
                            let status = step["status"].as_str()?;
                            if text.len() > 1024
                                || !["pending", "inProgress", "completed"].contains(&status)
                            {
                                return None;
                            }
                            Some(json!({"step":text,"status":status}))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let safe = json!({"id":id,"daemonId":daemon,"threadId":thread,"turnId":turn,"type":typ,"message":typ,"at":at,"activityState":normalized,"plan":plan});
        self.db()?
            .execute(
                "INSERT OR IGNORE INTO events(id,daemon_id,at,data) VALUES(?1,?2,?3,?4)",
                params![id, daemon, at, safe.to_string()],
            )
            .map_err(err)?;
        Ok(safe)
    }
    pub fn maintain_history(&self, now_iso: &str) -> R<Value> {
        let now = chrono::DateTime::parse_from_rfc3339(now_iso)
            .map_err(err)?
            .with_timezone(&chrono::Utc);
        let days = i64::try_from(
            self.settings()?["retentionDays"]
                .as_u64()
                .ok_or("invalid retention")?,
        )
        .map_err(err)?;
        let raw_age = chrono::Duration::try_days(days).ok_or("invalid retention")?;
        let raw_cutoff = now
            .checked_sub_signed(raw_age)
            .ok_or("invalid retention")?
            .to_rfc3339();
        let long_time = now
            .checked_sub_signed(chrono::Duration::days(90))
            .ok_or("invalid date")?;
        let long_cutoff = long_time.to_rfc3339();
        let daily_cutoff = now
            .with_timezone(&Local)
            .date_naive()
            .checked_sub_days(Days::new(u64::from(DAILY_PROGRESS_RETENTION_DAYS - 1)))
            .ok_or("invalid date")?
            .to_string();
        let minute_cutoff = long_time
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .ok_or("invalid date")?
            .to_rfc3339();
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let groups=tx.execute("INSERT INTO observation_minutes(daemon_id,minute,sample_count,success_count,failure_count,unknown_count,latency_sum,latency_count) SELECT daemon_id,strftime('%Y-%m-%dT%H:%M:00Z',at),COUNT(*),SUM(CASE WHEN json_extract(data,'$.transportState') IN ('healthy','slow') THEN 1 ELSE 0 END),SUM(CASE WHEN json_extract(data,'$.transportState')='disconnected' THEN 1 ELSE 0 END),SUM(CASE WHEN COALESCE(json_extract(data,'$.transportState'),'unknown') NOT IN ('healthy','slow','disconnected') THEN 1 ELSE 0 END),COALESCE(SUM(CASE WHEN json_type(data,'$.latencyMs') IN ('integer','real') THEN json_extract(data,'$.latencyMs') ELSE 0 END),0),SUM(CASE WHEN json_type(data,'$.latencyMs') IN ('integer','real') THEN 1 ELSE 0 END) FROM observations WHERE julianday(at)<julianday(?1) AND julianday(at)>=julianday(?2) GROUP BY daemon_id,strftime('%Y-%m-%dT%H:%M:00Z',at) ON CONFLICT(daemon_id,minute) DO UPDATE SET sample_count=sample_count+excluded.sample_count,success_count=success_count+excluded.success_count,failure_count=failure_count+excluded.failure_count,unknown_count=unknown_count+excluded.unknown_count,latency_sum=latency_sum+excluded.latency_sum,latency_count=latency_count+excluded.latency_count",params![raw_cutoff,long_cutoff]).map_err(err)?;
        let raw = tx
            .execute(
                "DELETE FROM observations WHERE julianday(at)<julianday(?1)",
                [raw_cutoff],
            )
            .map_err(err)?;
        let events = tx
            .execute(
                "DELETE FROM events WHERE julianday(at)<julianday(?1)",
                [&long_cutoff],
            )
            .map_err(err)?;
        let progress = tx
            .execute(
                "DELETE FROM progress WHERE julianday(at)<julianday(?1)",
                [&long_cutoff],
            )
            .map_err(err)?;
        tx.execute("DELETE FROM daily_progress WHERE date<?1", [&daily_cutoff])
            .map_err(err)?;
        tx.execute("DELETE FROM project_progress_points WHERE origin='observed' AND batch_id IS NULL AND date<?1", [&daily_cutoff]).map_err(err)?;
        let minutes = tx
            .execute(
                "DELETE FROM observation_minutes WHERE julianday(minute)<julianday(?1)",
                [&minute_cutoff],
            )
            .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(
            json!({"rawDeleted":raw,"eventsDeleted":events,"progressDeleted":progress,"minuteGroupsUpdated":groups,"minutesDeleted":minutes}),
        )
    }

    pub fn history(&self, id: &str, hours: u32) -> R<Value> {
        let c = self.db()?;
        let now = Local::now();
        // 요청 범위가 SQLite의 시간 범위보다 넓으면 저장된 원시 시각을 모두 조회한다.
        let cutoff = now
            .to_utc()
            .checked_sub_signed(chrono::Duration::hours(i64::from(hours)))
            .map(|at| at.to_rfc3339());
        let mut st = c
            .prepare("SELECT data FROM observations WHERE daemon_id=?1 AND julianday(at)>=COALESCE(julianday(?2),0) ORDER BY julianday(at),id")
            .map_err(err)?;
        let points=st.query_map(params![id,cutoff],|r|r.get::<_,String>(0)).map_err(err)?.filter_map(Result::ok).filter_map(|s|serde_json::from_str::<Value>(&s).ok()).map(|v|json!({"at":v["observedAt"],"latencyMs":v["latencyMs"],"value":v["value"],"unit":v["unit"],"metricId":v["metricId"],"state":v["transportState"]})).collect::<Vec<_>>();
        let mut ps=c.prepare("SELECT at,project_id,total,verified,reported,in_progress FROM progress WHERE project_id=?1 AND julianday(at)>=COALESCE(julianday(?2),0) ORDER BY julianday(at),id").map_err(err)?;
        let progress=ps.query_map(params![id,cutoff],|r|Ok(json!({"at":r.get::<_,String>(0)?,"projectId":r.get::<_,String>(1)?,"total":r.get::<_,i64>(2)?,"verified":r.get::<_,i64>(3)?,"reportedComplete":r.get::<_,i64>(4)?,"inProgress":r.get::<_,Option<i64>>(5)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
        let mut ms=c.prepare("SELECT minute,sample_count,success_count,failure_count,unknown_count,latency_sum,latency_count FROM observation_minutes WHERE daemon_id=?1 AND julianday(minute)>=COALESCE(julianday(?2),0) ORDER BY julianday(minute)").map_err(err)?;
        let minutes=ms.query_map(params![id,cutoff],|r|Ok(json!({"at":r.get::<_,String>(0)?,"sampleCount":r.get::<_,i64>(1)?,"successCount":r.get::<_,i64>(2)?,"failureCount":r.get::<_,i64>(3)?,"unknownCount":r.get::<_,i64>(4)?,"latencySum":r.get::<_,f64>(5)?,"latencyCount":r.get::<_,i64>(6)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
        let days = hours
            .div_ceil(HOURS_PER_DAY)
            .clamp(1, DAILY_PROGRESS_RETENTION_DAYS);
        let daily_progress = Self::daily_progress(&c, id, now, days)?;
        Ok(
            json!({"monitorId":id,"points":points,"progress":progress,"minutes":minutes,"dailyProgress":daily_progress}),
        )
    }

    fn daily_progress(c: &Connection, id: &str, now: DateTime<Local>, days: u32) -> R<Vec<Value>> {
        let today = now.date_naive();
        let first_day = today
            .checked_sub_days(Days::new(u64::from(days - 1)))
            .ok_or("invalid date")?;
        // 오프셋이 다른 기존 기록은 넉넉히 조회하고 SQLite가 해석하지 못하는 시각도 Rust에서 확인한다.
        let legacy_cutoff =
            (first_day.and_time(NaiveTime::MIN).and_utc() - Days::new(1)).to_rfc3339();
        let mut stmt = c
            .prepare(
                "SELECT collected_date,at,total,verified,reported,NULL,NULL,in_progress,id,0 AS is_daily
                   FROM progress
                  WHERE project_id=?1 AND
                        (collected_date BETWEEN ?3 AND ?4 OR
                         (collected_date IS NULL AND (julianday(at)>=julianday(?2) OR julianday(at) IS NULL)))
                 UNION ALL
                 SELECT date,at,total,verified,reported,tracking_basis,source_captured_at,in_progress,NULL,1
                   FROM daily_progress
                  WHERE project_id=?1 AND date BETWEEN ?3 AND ?4
                 ORDER BY is_daily,id",
            )
            .map_err(err)?;
        let mut rows = stmt
            .query(params![
                id,
                legacy_cutoff,
                first_day.to_string(),
                today.to_string()
            ])
            .map_err(err)?;
        let mut latest = BTreeMap::<NaiveDate, (DateTime<FixedOffset>, Value)>::new();
        while let Some(row) = rows.next().map_err(err)? {
            let stored_date: Option<String> = row.get(0).map_err(err)?;
            let timestamp: String = row.get(1).map_err(err)?;
            let Ok(at) = DateTime::parse_from_rfc3339(&timestamp) else {
                continue;
            };
            let date = match stored_date {
                Some(date) => NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(err)?,
                None => at.with_timezone(&Local).date_naive(),
            };
            if date < first_day || date > today || at > now {
                continue;
            }
            if latest
                .get(&date)
                .is_some_and(|(existing, _)| *existing > at)
            {
                continue;
            }
            latest.insert(
                date,
                (
                    at,
                    json!({
                        "projectId": id, "date": date.to_string(), "at": timestamp,
                        "total": row.get::<_, i64>(2).map_err(err)?,
                        "verified": row.get::<_, i64>(3).map_err(err)?,
                        "reportedComplete": row.get::<_, i64>(4).map_err(err)?,
                        "trackingBasis": row.get::<_, Option<String>>(5).map_err(err)?,
                        "sourceCapturedAt": row.get::<_, Option<String>>(6).map_err(err)?,
                        "inProgress": row.get::<_, Option<i64>>(7).map_err(err)?,
                    }),
                ),
            );
        }
        Ok(latest.into_values().map(|(_, point)| point).collect())
    }
    pub fn snapshot(&self) -> R<Value> {
        let reg = self.registry()?;
        let settings = self.settings()?;
        let mut c = self.db()?;
        let mut daemons = Vec::new();
        for id in ["A", "B"] {
            let obs:Option<String>=c.query_row("SELECT data FROM observations WHERE daemon_id=?1 ORDER BY julianday(at) DESC,id DESC LIMIT 1",[id],|r|r.get(0)).optional().map_err(err)?;
            let ob = obs
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .unwrap_or(json!({}));
            let last_disconnect:Option<String>=c.query_row("SELECT at FROM observations WHERE daemon_id=?1 AND json_extract(data,'$.transportState')='disconnected' ORDER BY julianday(at) DESC LIMIT 1",[id],|r|r.get(0)).optional().map_err(err)?;
            let mut query = c
                .prepare("SELECT data FROM events WHERE daemon_id=?1 ORDER BY julianday(at),id")
                .map_err(err)?;
            let events = query
                .query_map([id], |r| r.get::<_, String>(0))
                .map_err(err)?
                .filter_map(Result::ok)
                .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
                .collect::<Vec<_>>();
            let mut turns = std::collections::HashMap::<String, String>::new();
            let mut seen = std::collections::HashSet::<(String, String)>::new();
            let mut activity = "unknown".to_owned();
            let mut last = Value::Null;
            let mut activity_at: Option<chrono::DateTime<chrono::FixedOffset>> = None;
            let mut plan = Vec::<Value>::new();
            for event in &events {
                let at = event["at"].as_str().unwrap_or("");
                let event_time = chrono::DateTime::parse_from_rfc3339(at).ok();
                if last_disconnect
                    .as_deref()
                    .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
                    .is_some_and(|d| event_time.is_none_or(|t| t <= d))
                {
                    continue;
                }
                let thread = event["threadId"].as_str().unwrap_or("");
                let turn = event["turnId"].as_str().unwrap_or("");
                let typ = event["type"].as_str().unwrap_or("");
                if (typ == "turn/started" || typ == "turn.started")
                    && !thread.is_empty()
                    && !turn.is_empty()
                {
                    if seen.contains(&(thread.to_owned(), turn.to_owned()))
                        && turns.get(thread).is_some_and(|active| active != turn)
                    {
                        continue;
                    }
                    seen.insert((thread.to_owned(), turn.to_owned()));
                    turns.insert(thread.to_owned(), turn.to_owned());
                    plan.clear();
                } else if !thread.is_empty()
                    && !turn.is_empty()
                    && turns.get(thread).is_some_and(|current| current != turn)
                {
                    continue;
                }
                let state = event["activityState"].as_str().unwrap_or("unknown");
                if state == "plan" {
                    plan = event["plan"].as_array().cloned().unwrap_or_default();
                } else {
                    activity = state.to_owned();
                    activity_at = event_time;
                }
                last = event.clone();
            }
            let transport = ob["transportState"].as_str().unwrap_or("unknown");
            if transport == "disconnected" || transport == "unknown" {
                activity = "unknown".into();
                plan.clear();
            } else {
                let observed = ob["observedAt"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
                if observed.is_none() || activity_at.is_none() || observed >= activity_at {
                    let state = ob["activityState"].as_str().unwrap_or("unknown");
                    activity = if [
                        "working",
                        "toolRunning",
                        "approval",
                        "input",
                        "idle",
                        "failed",
                        "unknown",
                    ]
                    .contains(&state)
                    {
                        state
                    } else {
                        "unknown"
                    }
                    .to_owned();
                }
            }
            let age = ob["observedAt"]
                .as_str()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .map(|at| chrono::Utc::now().signed_duration_since(at));
            let enabled = reg["monitors"].as_array().unwrap().iter().any(|m| {
                m["kind"] == "daemon"
                    && m["source"] == id
                    && m["enabled"] == true
                    && m["archived"] != true
            });
            let freshness = if !enabled {
                "unavailable"
            } else {
                match age {
                    None => "unavailable",
                    Some(age) if age > chrono::Duration::seconds(15) => "stale",
                    Some(_) => "fresh",
                }
            };
            let last_known = json!({"transportState":transport,"activityState":activity,"latencyMs":ob["latencyMs"],"checkedAt":ob["observedAt"]});
            if freshness != "fresh" {
                plan.clear();
            }
            daemons.push(json!({"id":id,"name":id,"transportState":if freshness=="fresh" {transport}else{"unknown"},"activityState":if freshness=="fresh" {activity.as_str()}else{"unknown"},"latencyMs":ob["latencyMs"],"checkedAt":ob["observedAt"],"lastActivityAt":last["at"],"reason":ob["reason"].as_str().unwrap_or("관측 정보 없음"),"plan":plan,"freshness":freshness,"lastKnown":last_known}));
        }
        let mut statuses = reg
            .get("statuses")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut monitor_statuses = serde_json::Map::new();
        let mut project_source_errors = HashMap::<&str, Vec<String>>::new();
        let collection_time = Local::now();
        let collection_date = collection_time.date_naive().to_string();
        // 고정 정밀도의 UTC 시각은 오프셋이 바뀌어도 SQLite에서 정확히 비교할 수 있다.
        let collected_at = collection_time
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Nanos, true);
        for m in reg["monitors"].as_array().unwrap() {
            let mid = m["id"].as_str().unwrap_or("");
            if m["archived"] == true {
                continue;
            }
            if m["enabled"] != true {
                monitor_statuses.insert(mid.into(),json!({"state":"disabled","value":null,"unit":null,"reason":"비활성","checkedAt":null}));
                continue;
            }
            if m["kind"] == "daemon" {
                let source = m["source"].as_str().unwrap_or("");
                let daemon = daemons.iter().find(|d| d["id"] == source);
                monitor_statuses.insert(mid.into(),daemon.map(|d|json!({"state":d["transportState"],"value":d["latencyMs"],"unit":"ms","reason":d["reason"],"checkedAt":d["checkedAt"]})).unwrap_or(json!({"state":"unknown","value":null,"unit":null,"reason":"대상 확인 불가","checkedAt":null})));
                continue;
            }
            if m["kind"] != "structuredStatus" {
                continue;
            }
            let source = m["source"].as_str().unwrap_or("");
            let project = reg["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == m["projectId"] && p["archived"] != true);
            let result = project
                .ok_or("project unavailable".to_owned())
                .and_then(|p| {
                    let path = PathBuf::from(source);
                    let path = if path.is_absolute() {
                        path
                    } else {
                        PathBuf::from(p["root"].as_str().unwrap_or("")).join(path)
                    };
                    fs::read_to_string(path).map_err(err)
                })
                .and_then(|s| serde_json::from_str::<Value>(&s).map_err(err))
                .and_then(|v| {
                    Self::validate_status(&v)?;
                    if v["projectId"] != m["projectId"] {
                        return Err("monitor project mismatch".into());
                    }
                    Ok(v)
                });
            match result {
                Ok(v) => {
                    let metric = v["metrics"].as_array().and_then(|a| a.first());
                    let checked = now();
                    monitor_statuses.insert(mid.into(),json!({"state":metric.and_then(|m|m["state"].as_str()).unwrap_or("healthy"),"value":metric.map(|m|&m["value"]),"unit":metric.map(|m|&m["unit"]),"reason":"","checkedAt":checked}));
                    for metric in v["metrics"].as_array().into_iter().flatten() {
                        let data = json!({"observedAt":checked,"value":metric["value"],"unit":metric["unit"],"metricId":metric["id"],"transportState":metric["state"]});
                        c.execute(
                            "INSERT INTO observations(daemon_id,at,data) VALUES(?1,?2,?3)",
                            params![mid, checked, data.to_string()],
                        )
                        .map_err(err)?;
                    }
                    statuses.insert(v["projectId"].as_str().unwrap().to_owned(), v);
                }
                Err(_) => {
                    let reason = "상태 파일을 읽거나 검증하지 못했습니다.";
                    if let Some(project_id) = m["projectId"].as_str() {
                        project_source_errors
                            .entry(project_id)
                            .or_default()
                            .push(format!("{reason} ({source})"));
                    }
                    let checked = now();
                    monitor_statuses.insert(mid.into(),json!({"state":"unknown","value":null,"unit":null,"reason":reason,"checkedAt":checked}));
                    let data =
                        json!({"observedAt":checked,"value":null,"transportState":"unknown"});
                    c.execute(
                        "INSERT INTO observations(daemon_id,at,data) VALUES(?1,?2,?3)",
                        params![mid, checked, data.to_string()],
                    )
                    .map_err(err)?;
                }
            }
        }
        let mut projects = Vec::new();
        for p in reg["projects"].as_array().unwrap() {
            if p["archived"] == true {
                continue;
            }
            let id = p["id"].as_str().unwrap_or("");
            if self.knowledge().is_managed_project(id)? {
                let view = self.knowledge().get_project_progress(id, None)?;
                let mut out = p.clone();
                for key in [
                    "total",
                    "verified",
                    "reportedComplete",
                    "inProgress",
                    "blocked",
                ] {
                    out[key] = view["counts"][key].clone();
                }
                out["tasks"] = json!(view["basis"]["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| {
                        let mut t = t.clone();
                        t["id"] = t["taskId"].clone();
                        t
                    })
                    .collect::<Vec<_>>());
                out["trackingBasis"] = view["basis"]["name"].clone();
                out["progressBasisId"] = view["basis"]["basisId"].clone();
                out["progressBasisVersion"] = view["basis"]["version"].clone();
                out["progressMode"] = json!("managed");
                out["sourceFreshness"] = view["sourceFreshness"].clone();
                out["sourceCapturedAt"] = view["sourceCheckedAt"].clone();
                out["sourceErrors"] = json!([]);
                out["currentTask"] = json!(view["basis"]["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|t| t["state"] == "inProgress")
                    .and_then(|t| t["title"].as_str())
                    .unwrap_or(""));
                projects.push(out);
                continue;
            }
            let status = statuses.get(id);
            let registered = reg["tasks"].as_array().unwrap();
            let tombstones = registered
                .iter()
                .filter(|t| {
                    t["projectId"] == id && (t["archived"] == true || t["state"] == "cancelled")
                })
                .filter_map(|t| t["id"].as_str().map(str::to_owned))
                .collect::<std::collections::HashSet<_>>();
            let mut paths = Vec::<(Option<String>, PathBuf)>::new();
            if let Some(source) = p["checklistPath"].as_str().filter(|s| !s.trim().is_empty()) {
                paths.push((None, PathBuf::from(source)))
            }
            for m in reg["monitors"].as_array().unwrap().iter().filter(|m| {
                m["kind"] == "checklist"
                    && m["enabled"] == true
                    && m["archived"] != true
                    && m["projectId"] == id
            }) {
                paths.push((
                    m["id"].as_str().map(str::to_owned),
                    PathBuf::from(m["source"].as_str().unwrap_or("")),
                ))
            }
            let mut seen_paths = std::collections::HashSet::new();
            let mut tasks = Vec::<Value>::new();
            let mut content = String::new();
            let mut errors = project_source_errors.remove(id).unwrap_or_default();
            for (mid, path) in paths {
                let path = if path.is_absolute() {
                    path
                } else {
                    PathBuf::from(p["root"].as_str().unwrap_or("")).join(path)
                };
                let checked = now();
                let result = fs::canonicalize(&path).and_then(|canonical| {
                    if !seen_paths.insert(canonical.clone()) {
                        return Ok(None);
                    }
                    fs::read_to_string(canonical).map(Some)
                });
                match result {
                    Err(_) => {
                        errors.push(format!("체크리스트를 읽지 못했습니다: {}", path.display()));
                        if let Some(mid) = mid {
                            monitor_statuses.insert(mid,json!({"state":"unknown","value":null,"unit":null,"reason":"체크리스트를 읽지 못했습니다.","checkedAt":checked}));
                        }
                    }
                    Ok(text) => {
                        if let Some(mid) = mid {
                            monitor_statuses.insert(mid,json!({"state":"healthy","value":null,"unit":null,"reason":"","checkedAt":checked}));
                        }
                        let Some(text) = text else { continue };
                        content.push_str(&text);
                        let mut source_hash = std::collections::hash_map::DefaultHasher::new();
                        text.hash(&mut source_hash);
                        let source_revision = format!("checklist:{:016x}", source_hash.finish());
                        let mut fence: Option<(char, usize)> = None;
                        let mut stack = Vec::<(usize, String)>::new();
                        for line in text.lines() {
                            let trimmed = line.trim_start();
                            let indent = line.len() - trimmed.len();
                            let ch = trimmed.chars().next().unwrap_or(' ');
                            let n = trimmed.chars().take_while(|c| *c == ch).count();
                            if (ch == '~' || ch == '\x60') && n >= 3 {
                                if fence.is_none() {
                                    fence = Some((ch, n))
                                } else if fence.is_some_and(|(c, len)| c == ch && n >= len) {
                                    fence = None
                                }
                                continue;
                            }
                            if fence.is_some() {
                                continue;
                            }
                            let Some(item) = trimmed
                                .strip_prefix("- ")
                                .or_else(|| trimmed.strip_prefix("* "))
                                .or_else(|| trimmed.strip_prefix("+ "))
                            else {
                                continue;
                            };
                            let parsed = [
                                ("[ ] ", "pending"),
                                ("[x] ", "reportedComplete"),
                                ("[X] ", "reportedComplete"),
                                ("[-] ", "inProgress"),
                                ("[?] ", "blocked"),
                            ]
                            .iter()
                            .find_map(|(prefix, state)| {
                                item.strip_prefix(prefix)
                                    .map(|title| (title.trim(), *state))
                            });
                            let Some((title, state)) = parsed else {
                                continue;
                            };
                            let token = title
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim_end_matches('.');
                            let explicit = !token.is_empty()
                                && token
                                    .bytes()
                                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                                && token.bytes().any(|c| c.is_ascii_uppercase())
                                && token.bytes().any(|c| c.is_ascii_digit());
                            let mut hash = std::collections::hash_map::DefaultHasher::new();
                            title.hash(&mut hash);
                            let tid = if explicit {
                                token.to_owned()
                            } else {
                                format!("checklist-{:016x}", hash.finish())
                            };
                            while stack.last().is_some_and(|(depth, _)| *depth >= indent) {
                                stack.pop();
                            }
                            let parent = stack.last().map(|(_, id)| id.clone());
                            stack.push((indent, tid.clone()));
                            if tombstones.contains(&tid) || tasks.iter().any(|t| t["id"] == tid) {
                                continue;
                            }
                            let mut t = json!({"id":tid,"projectId":id,"title":title,"state":state,"revision":source_revision});
                            if let Some(parent) = parent {
                                t["parentId"] = json!(parent);
                            }
                            tasks.push(t);
                        }
                    }
                }
            }
            for t in registered
                .iter()
                .filter(|t| {
                    t["projectId"] == id && t["archived"] != true && t["state"] != "cancelled"
                })
                .chain(
                    status
                        .and_then(|s| s["tasks"].as_array())
                        .into_iter()
                        .flatten(),
                )
            {
                if t["id"].as_str().is_some_and(|tid| tombstones.contains(tid)) {
                    continue;
                }
                if let Some(i) = tasks.iter().position(|x| x["id"] == t["id"]) {
                    let parent = tasks[i].get("parentId").cloned();
                    tasks[i] = t.clone();
                    if tasks[i].get("parentId").is_none() {
                        if let Some(parent) = parent {
                            tasks[i]["parentId"] = parent;
                        }
                    }
                } else {
                    tasks.push(t.clone());
                }
            }
            tasks.retain(|t| t["archived"] != true && t["state"] != "cancelled");
            for t in &mut tasks {
                let evidence=reg["evidence"].as_array().unwrap().iter().filter(|e|e["taskId"]==t["id"]&&e["success"]==true&&e["subjectRevision"]==t["revision"]&&e["archived"]!=true).map(|e|{let mut safe=json!({"id":e["id"],"kind":e["kind"],"success":true,"subjectRevision":e["subjectRevision"],"reference":e["reference"]});if let Some(at)=e["verifiedAt"].as_str(){safe["verifiedAt"]=json!(at);}safe}).collect::<Vec<_>>();
                t["verified"] = json!(t["state"] == "reportedComplete" && !evidence.is_empty());
                t["verificationEvidence"] = json!(evidence);
            }
            let leaves = tasks
                .iter()
                .filter(|t| !tasks.iter().any(|child| child["parentId"] == t["id"]))
                .collect::<Vec<_>>();
            let total = leaves.len();
            let verified = leaves.iter().filter(|t| t["verified"] == true).count();
            let reported = leaves
                .iter()
                .filter(|t| t["state"] == "reportedComplete" && t["verified"] != true)
                .count();
            let mut out = p.clone();
            out["total"] = json!(total);
            out["verified"] = json!(verified);
            out["reportedComplete"] = json!(reported);
            let in_progress = leaves.iter().filter(|t| t["state"] == "inProgress").count();
            out["inProgress"] = json!(in_progress);
            out["blocked"] = json!(leaves.iter().filter(|t| t["state"] == "blocked").count());
            out["updatedAt"] = Value::Null;
            out["currentTask"] = json!(status
                .and_then(|s| s["currentTask"].as_str())
                .unwrap_or_else(|| leaves
                    .iter()
                    .find(|t| t["state"] == "inProgress")
                    .and_then(|t| t["title"].as_str())
                    .unwrap_or("")));
            let valid = errors.is_empty();
            out["sourceErrors"] = json!(errors);
            out["tasks"] = json!(tasks);
            if valid {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(err)?;
                let accepted = tx.execute(
                    "INSERT INTO daily_progress(project_id,date,at,total,verified,reported,tracking_basis,source_captured_at,in_progress)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
                     ON CONFLICT(project_id,date) DO UPDATE SET
                        at=excluded.at,total=excluded.total,verified=excluded.verified,reported=excluded.reported,
                        tracking_basis=excluded.tracking_basis,source_captured_at=excluded.source_captured_at,in_progress=excluded.in_progress
                     WHERE excluded.at>=daily_progress.at",
                    params![id,collection_date,collected_at,total as i64,verified as i64,reported as i64,
                        p["trackingBasis"].as_str(),p["sourceCapturedAt"].as_str(),in_progress as i64],
                ).map_err(err)?;
                if accepted > 0 {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    "progress in_progress recorded".hash(&mut hasher);
                    out.to_string().hash(&mut hasher);
                    content.hash(&mut hasher);
                    let fingerprint = format!("{:016x}", hasher.finish());
                    let changed=tx.execute("INSERT INTO fingerprints(project_id,fingerprint) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET fingerprint=excluded.fingerprint WHERE fingerprint<>excluded.fingerprint",params![id,fingerprint]).map_err(err)?;
                    if changed > 0 {
                        tx.execute(
                            "INSERT INTO progress(at,project_id,total,verified,reported,collected_date,in_progress) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                            params![collected_at,id,total as i64,verified as i64,reported as i64,collection_date,in_progress as i64],
                        ).map_err(err)?;
                    }
                }
                tx.commit().map_err(err)?;
            }
            projects.push(out);
        }
        let mut stmt = c
            .prepare("SELECT attempt_id,date,action,state,result FROM runs ORDER BY date DESC")
            .map_err(err)?;
        let runs=stmt.query_map([],|r|{let raw:Option<String>=r.get(4)?;let parsed=raw.and_then(|s|serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object);let clean=parsed.as_ref().map(|v|json!({"state":v["state"],"code":v["code"].as_str(),"date":v["date"].as_str(),"time":v["time"].as_str(),"message":v["message"].as_str()}));Ok(json!({"attemptId":r.get::<_,String>(0)?,"requestedDate":r.get::<_,String>(1)?,"date":r.get::<_,String>(1)?,"action":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,"code":clean.as_ref().and_then(|v|v["code"].as_str()),"recordDate":clean.as_ref().and_then(|v|v["date"].as_str()),"time":clean.as_ref().and_then(|v|v["time"].as_str()),"message":clean.as_ref().and_then(|v|v["message"].as_str()).unwrap_or(""),"result":clean}))}).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
        let today = Local::now().date_naive().to_string();
        let checkin = runs.iter().find(|r| r["action"] == "checkIn");
        let checkout = runs.iter().find(|r| r["action"] == "checkOut");
        let time = |run: Option<&Value>| -> Value {
            run.filter(|r| {
                r["requestedDate"] == today
                    && r["recordDate"] == today
                    && ["recorded", "alreadyIn", "alreadyOut"]
                        .contains(&r["state"].as_str().unwrap_or(""))
            })
            .map(|r| r["time"].clone())
            .unwrap_or(Value::Null)
        };
        let latest = checkout
            .filter(|r| r["requestedDate"] == today)
            .or_else(|| checkin.filter(|r| r["requestedDate"] == today));
        let mut st = c
            .prepare("SELECT data FROM events ORDER BY julianday(at) DESC,id DESC LIMIT 100")
            .map_err(err)?;
        let events = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .filter_map(Result::ok)
            .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
            .collect::<Vec<_>>();
        Ok(
            json!({"now":now(),"registryRevision":reg["revision"],"daemons":daemons,"projects":projects,"attendance":{"date":today,"state":latest.map(|r|r["state"].clone()).unwrap_or(json!("unknown")),"serverState":"unknown","checkInTime":time(checkin),"checkOutTime":time(checkout),"lastCode":latest.map(|r|r["code"].clone()).unwrap_or(Value::Null),"message":latest.and_then(|r|r["message"].as_str()).filter(|s|!s.is_empty()).unwrap_or("서버 상태 미확인"),"lastRunsByAction":{"checkIn":checkin,"checkOut":checkout},"policy":{"startTime":settings["startTime"],"endTime":settings["endTime"],"checkoutTime":settings["checkoutTime"]},"runs":runs},"events":events,"settings":settings,"monitorStatuses":monitor_statuses}),
        )
    }
}

// 문서를 별도 자식 프로세스로 읽고 불변 해시 캐시와 연결 상태를 보존한다.
use super::{err, now, KnowledgeService, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_SCAN: u64 = 64 * 1024 * 1024;
const MAX_CACHE: u64 = 256 * 1024 * 1024;
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn relative_path(text: &str) -> Result<PathBuf> {
    let p = Path::new(text);
    if text.is_empty()
        || text.contains(':')
        || text.contains('\0')
        || p.is_absolute()
        || p.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("invalid relative path".into());
    }
    Ok(p.into())
}
pub fn run_bounded(mut command: Command, timeout: Duration) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let (mut child, _owner) = crate::child_process::spawn(&mut command).map_err(err)?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("worker failed: {status}"))
                }
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err(e));
            }
            Ok(None) => {}
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("document_worker_timeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn read_json(p: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(p).map_err(err)?).map_err(err)
}
fn write_json(p: &Path, v: &Value) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(p)
        .map_err(err)?;
    f.write_all(v.to_string().as_bytes()).map_err(err)?;
    f.sync_all().map_err(err)
}
fn excluded(path: &Path, source: &Value) -> bool {
    path.components().any(|c| {
        [
            ".git",
            "node_modules",
            "Library",
            "Builds",
            "Logs",
            "temp",
            ".superpowers",
            "게시 이전본",
        ]
        .iter()
        .any(|s| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(s))
    }) || source["exclude"].as_array().is_some_and(|xs| {
        xs.iter().any(|x| {
            x.as_str().is_some_and(|s| {
                path.to_string_lossy()
                    .replace('\\', "/")
                    .starts_with(&s.replace('\\', "/"))
            })
        })
    })
}
fn modified(meta: &fs::Metadata) -> String {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_default()
}
struct Walker<'a> {
    root: &'a Path,
    stage: &'a Path,
    source: &'a Value,
    files: Vec<Value>,
    errors: Vec<String>,
    entries: usize,
    bytes: u64,
    force: bool,
    known: &'a Value,
}
impl Walker<'_> {
    fn walk(&mut self, dir: &Path, depth: usize) {
        if depth > 8 {
            self.errors.push("depth limit".into());
            return;
        }
        let listing = match fs::read_dir(dir) {
            Ok(v) => v,
            Err(e) => {
                self.errors.push(err(e));
                return;
            }
        };
        for item in listing {
            self.entries += 1;
            if self.entries > 5000 {
                self.errors.push("entry limit".into());
                return;
            }
            let item = match item {
                Ok(v) => v,
                Err(e) => {
                    self.errors.push(err(e));
                    continue;
                }
            };
            let path = item.path();
            let relative = match path.strip_prefix(self.root) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if excluded(relative, self.source) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else {
                self.errors.push("metadata unavailable".into());
                continue;
            };
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    continue;
                }
            }
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                self.walk(&path, depth + 1);
                continue;
            }
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !["md", "txt", "json"].contains(&ext.as_str()) {
                continue;
            }
            let rel = relative.to_string_lossy().replace('\\', "/");
            if self.source["include"].as_array().is_some_and(|xs| {
                !xs.is_empty()
                    && !xs.iter().any(|x| {
                        x.as_str()
                            .is_some_and(|s| rel.starts_with(&s.replace('\\', "/")))
                    })
            }) {
                continue;
            }
            if meta.len() > MAX_FILE {
                self.errors.push(format!("{rel}: file limit"));
                continue;
            }
            let mtime = modified(&meta);
            if !self.force
                && self.known[&rel]["bytes"] == meta.len()
                && self.known[&rel]["mtime"] == mtime
            {
                self.files.push(json!({"path":rel,"unchanged":true,"sha256":self.known[&rel]["sha256"],"bytes":meta.len(),"mtime":mtime}));
                continue;
            }
            if self.bytes + meta.len() > MAX_SCAN {
                self.errors.push("scan byte limit".into());
                return;
            }
            match fs::read(&path) {
                Ok(bytes) => {
                    if bytes.len() as u64 > MAX_FILE
                        || fs::metadata(&path).ok().is_none_or(|after| {
                            after.len() != meta.len() || modified(&after) != mtime
                        })
                    {
                        self.errors.push(format!("{rel}: changing file"));
                        continue;
                    }
                    if std::str::from_utf8(&bytes).is_err() {
                        self.errors.push(format!("{rel}: invalid UTF-8"));
                        continue;
                    }
                    let hash = sha256(&bytes);
                    self.bytes += bytes.len() as u64;
                    if let Err(e) = fs::write(self.stage.join(&hash), &bytes) {
                        self.errors.push(err(e));
                        continue;
                    }
                    self.files
                        .push(json!({"path":rel,"sha256":hash,"bytes":bytes.len(),"mtime":mtime}));
                }
                Err(e) => self.errors.push(format!("{rel}: {e}")),
            }
        }
    }
}
pub fn run_scan_worker(request_path: &Path) -> Result<()> {
    let request = read_json(request_path)?;
    let stage = request_path.parent().ok_or("invalid staging")?;
    let mut sources = Vec::new();
    for source in request["sources"].as_array().ok_or("invalid sources")? {
        let root = Path::new(source["root"].as_str().ok_or("invalid root")?);
        let state = match fs::symlink_metadata(root) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err("reparse root unavailable".into());
                    }
                }
                if !meta.is_dir() || meta.file_type().is_symlink() {
                    Some("invalidRoot")
                } else {
                    None
                }
            }
            Err(e) => Some(if e.kind() == std::io::ErrorKind::PermissionDenied {
                "denied"
            } else {
                "offline"
            }),
        };
        if let Some(state) = state {
            sources.push(
                json!({"sourceId":source["sourceId"],"state":state,"files":[],"checkedAt":now()}),
            );
            continue;
        }
        let known = &request["known"][source["sourceId"].as_str().unwrap_or("")];
        let mut w = Walker {
            root,
            stage,
            source,
            known,
            files: vec![],
            errors: vec![],
            entries: 0,
            bytes: 0,
            force: request["force"] == true,
        };
        w.walk(root, 0);
        sources.push(json!({"sourceId":source["sourceId"],"state":if w.errors.is_empty(){"online"}else{"partial"},"files":w.files,"errors":w.errors,"checkedAt":now()}));
    }
    write_json(
        &stage.join("result.json"),
        &json!({"jobId":request["jobId"],"revision":request["revision"],"sources":sources}),
    )
}
struct ScanLock {
    path: PathBuf,
    token: String,
}
impl Drop for ScanLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).ok().as_deref() == Some(&self.token) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn acquire(data: &Path, location: &str) -> Result<ScanLock> {
    let path = data.join(format!("document-scan-{location}.lock"));
    if fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > Duration::from_secs(300))
    {
        let _ = fs::remove_file(&path);
    }
    let token = Uuid::new_v4().to_string();
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "scan already running")?;
    f.write_all(token.as_bytes()).map_err(err)?;
    Ok(ScanLock { path, token })
}
impl KnowledgeService {
    pub fn list_unlinked_documents(&self, page: u32) -> Result<Value> {
        let c = self.db()?;
        let mut q=c.prepare("SELECT v.version_id,v.document_id,d.source_id,json_extract(v.payload,'$.path'),v.sha256,v.bytes,v.captured_at,v.kind FROM document_versions v JOIN documents d USING(document_id) JOIN document_sources s USING(source_id) WHERE json_extract(s.payload,'$.scope')='global' AND json_extract(s.payload,'$.archived') IS NOT 1 AND v.sha256=json_extract(d.payload,'$.sha256') AND NOT EXISTS(SELECT 1 FROM document_project_links l WHERE l.document_id=d.document_id AND l.review_state='accepted') ORDER BY v.captured_at DESC LIMIT 100 OFFSET ?1").map_err(err)?;
        let rows=q.query_map([i64::from(page)*100],|r|Ok(json!({"versionId":r.get::<_,String>(0)?,"documentId":r.get::<_,String>(1)?,"sourceId":r.get::<_,String>(2)?,"path":r.get::<_,String>(3)?,"sha256":r.get::<_,String>(4)?,"bytes":r.get::<_,i64>(5)?,"capturedAt":r.get::<_,String>(6)?,"kind":r.get::<_,String>(7)?}))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
        Ok(json!({"documents":rows,"page":page,"pageSize":100}))
    }
    pub fn set_document_project_link(
        &self,
        project: &str,
        document: &str,
        linked: bool,
        expected: i64,
    ) -> Result<Value> {
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        self.require_project(project)?;
        let source:String=tx.query_row("SELECT s.payload FROM document_sources s JOIN documents d USING(source_id) WHERE document_id=?1",[document],|r|r.get(0)).map_err(err)?;
        let source: Value = serde_json::from_str(&source).map_err(err)?;
        if source["scope"] == "project" && source["projectId"] != project {
            return Err("document belongs to another project source".into());
        }
        super::jobs::control(&tx, project)?;
        super::progress::bump(&tx, project, expected)?;
        tx.execute("INSERT INTO document_project_links VALUES(?1,?2,'manual',?3) ON CONFLICT(document_id,project_id) DO UPDATE SET method='manual',review_state=excluded.review_state",params![document,project,if linked{"accepted"}else{"rejected"}]).map_err(err)?;
        tx.commit().map_err(err)?;
        self.get_project_progress(project, None)
    }
    pub fn pin_document_version(&self, id: &str, owner: &str) -> Result<()> {
        self.db()?
            .execute(
                "INSERT OR IGNORE INTO document_version_pins VALUES(?1,?2)",
                params![id, owner],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn cleanup_document_cache(&self) -> Result<Value> {
        let _cache_lock = acquire(self.engine.data_dir(), "cache")?;
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut q=tx.prepare("SELECT v.version_id,v.cache_path FROM document_versions v WHERE julianday(v.captured_at)<julianday('now','-30 days') AND NOT EXISTS(SELECT 1 FROM document_version_pins p WHERE p.version_id=v.version_id) AND NOT EXISTS(SELECT 1 FROM import_batch_items i WHERE i.version_id=v.version_id) AND EXISTS(SELECT 1 FROM document_versions newer WHERE newer.document_id=v.document_id AND julianday(newer.captured_at)>julianday(v.captured_at))").map_err(err)?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        drop(q);
        let mut removed = 0;
        let mut unused = Vec::new();
        for (id, path) in rows {
            tx.execute("DELETE FROM document_versions WHERE version_id=?1", [&id])
                .map_err(err)?;
            let references: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM document_versions WHERE cache_path=?1)",
                    [&path],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !references {
                unused.push(path);
            }
            removed += 1;
        }
        tx.commit().map_err(err)?;
        for path in unused {
            let p = Path::new(&path);
            if p.parent() == Some(self.engine.data_dir().join("document-cache").as_path()) {
                let _ = fs::remove_file(p);
            }
        }
        let cache = self.engine.data_dir().join("document-cache");
        let mut orphaned=0;
        if let Ok(entries)=fs::read_dir(cache) {
            for entry in entries.flatten() {
                let name=entry.file_name();
                let Some(name)=name.to_str() else{continue;};
                if name.len()!=64 || !name.bytes().all(|byte|byte.is_ascii_hexdigit()) {continue;}
                if !entry.file_type().is_ok_and(|kind|kind.is_file()) {continue;}
                let old=entry.metadata().and_then(|info|info.modified()).ok()
                    .and_then(|time|time.elapsed().ok()).is_some_and(|age|age.as_secs()>30*86400);
                if !old {continue;}
                let referenced:bool=c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM document_versions WHERE sha256=?1)",[name],|row|row.get(0),
                ).map_err(err)?;
                if !referenced && fs::remove_file(entry.path()).is_ok() {orphaned+=1;}
            }
        }
        Ok(json!({"removed":removed,"orphaned":orphaned}))
    }
    pub fn scan_document_sources(&self, ids: &[String], force: bool) -> Result<Value> {
        self.scan_with_executable(
            ids,
            force,
            &std::env::current_exe().map_err(err)?,
            Duration::from_secs(30),
        )
    }
    pub fn scan_with_executable(
        &self,
        ids: &[String],
        force: bool,
        executable: &Path,
        timeout: Duration,
    ) -> Result<Value> {
        let cfg = self.get_document_config()?;
        let job = Uuid::new_v4().to_string();
        let sources: Vec<_> = cfg["sources"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| {
                s["enabled"] == true
                    && s["archived"] != true
                    && (ids.is_empty() || ids.iter().any(|id| s["sourceId"] == *id))
            })
            .cloned()
            .collect();
        if ids
            .iter()
            .any(|id| !sources.iter().any(|s| s["sourceId"] == *id))
        {
            return Err("source unavailable".into());
        }
        let all = std::thread::scope(|scope| {
            let handles:Vec<_>=["local","nas"].iter().map(|location|{let subset:Vec<_>=sources.iter().filter(|s|s["location"]==*location).cloned().collect();let cfg=&cfg;let job=&job;scope.spawn(move||->Result<Vec<Value>>{
        if subset.is_empty(){return Ok(vec![])}let _lock=acquire(self.engine.data_dir(),location)?;let mut result=vec![];
        for s in subset {
          let stage=self.engine.data_dir().join("document-staging").join(Uuid::new_v4().to_string());fs::create_dir_all(&stage).map_err(err)?;
          let mut known=json!({});let c=self.db()?;let mut q=c.prepare("SELECT normalized_path,payload FROM documents WHERE source_id=?1").map_err(err)?;
          for row in q.query_map([s["sourceId"].as_str().unwrap()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(err)? {let(p,v)=row.map_err(err)?;let mut v:Value=serde_json::from_str(&v).map_err(err)?;let key=v["path"].as_str().unwrap_or(&p).to_owned();known[s["sourceId"].as_str().unwrap()][key]=v.take();}
          drop(q);drop(c);
          let last=self.source_states()?["sources"].as_array().unwrap().iter().find(|v|v["sourceId"]==s["sourceId"]).cloned().unwrap_or(json!({}));
          let rehash=force||last["lastHashAt"].as_str().and_then(|v|chrono::DateTime::parse_from_rfc3339(v).ok()).is_none_or(|at|chrono::Utc::now().signed_duration_since(at).num_seconds()>=300);
          let request=json!({"jobId":job,"revision":cfg["revision"],"sources":[s],"known":known,"force":rehash});write_json(&stage.join("request.json"),&request)?;
          let mut cmd=Command::new(executable);cmd.arg("--document-scan-worker").arg(stage.join("request.json"));
          match run_bounded(cmd,timeout) {
             Ok(())=>{let v=read_json(&stage.join("result.json"))?;if v["jobId"]!=*job{return Err("scan job mismatch".into())}self.ingest_scan_result(v.clone(),&stage)?;if rehash{self.update_source_state(s["sourceId"].as_str().unwrap(),None,Some(now()))?;}result.extend(v["sources"].as_array().unwrap().iter().cloned());},
             Err(error)=>{let state=if error.contains("timeout"){"timeout"}else{"error"};self.update_source_state(s["sourceId"].as_str().unwrap(),Some(json!({"state":state,"error":error,"checkedAt":now()})),None)?;result.push(json!({"sourceId":s["sourceId"],"state":state}));}
          }
          // staging에는 이번 worker가 만든 개별 파일만 존재한다. 다른 캐시·문서에는 접근하지 않는다.
          if let Ok(entries)=fs::read_dir(&stage){for item in entries.flatten(){if item.file_type().is_ok_and(|t|t.is_file()){let _=fs::remove_file(item.path());}}}let _=fs::remove_dir(&stage);
        }Ok(result)
      })}).collect();
            handles
                .into_iter()
                .map(|h| h.join().map_err(|_| "scan thread failed".to_string())?)
                .collect::<Result<Vec<_>>>()
        })?;
        Ok(
            json!({"jobId":job,"state":"completed","sources":all.into_iter().flatten().collect::<Vec<_>>()}),
        )
    }
    pub fn ingest_scan_result(&self, v: Value, stage: &Path) -> Result<Value> {
        let cfg = self.get_document_config()?;
        if v["revision"] != cfg["revision"] {
            return Err("scan_config_conflict".into());
        }
        let _cache_lock = acquire(self.engine.data_dir(), "cache")?;
        let cache = self.engine.data_dir().join("document-cache");
        fs::create_dir_all(&cache).map_err(err)?;
        let mut cache_size = fs::read_dir(&cache)
            .map_err(err)?
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum::<u64>();
        let mut prepared = vec![];
        let mut seen = HashSet::new();
        for s in v["sources"].as_array().ok_or("invalid scan result")? {
            let sid = s["sourceId"].as_str().ok_or("missing source")?;
            let source = cfg["sources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["sourceId"] == sid && x["enabled"] == true)
                .ok_or("source unavailable")?;
            if !matches!(
                s["state"].as_str(),
                Some("online" | "partial" | "offline" | "denied" | "invalidRoot")
            ) {
                return Err("invalid source state".into());
            }
            for f in s["files"].as_array().ok_or("invalid scan files")? {
                let path = f["path"].as_str().ok_or("missing relative path")?;
                relative_path(path)?;
                if !seen.insert((sid.to_owned(), path.to_owned())) {
                    return Err("duplicate scan path".into());
                }
                let hash = f["sha256"].as_str().ok_or("missing hash")?;
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("invalid hash".into());
                }
                let target = cache.join(hash);
                if f["unchanged"] != true {
                    let bytes = fs::read(stage.join(hash)).map_err(err)?;
                    if bytes.len() as u64 > MAX_FILE
                        || json!(bytes.len()) != f["bytes"]
                        || sha256(&bytes) != hash
                        || std::str::from_utf8(&bytes).is_err()
                    {
                        return Err("scan hash or encoding mismatch".into());
                    }
                    if !target.exists() {
                        if cache_size + bytes.len() as u64 > MAX_CACHE {
                            return Err("document_cache_quota; retained evidence preserved".into());
                        }
                        let mut out = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&target)
                            .map_err(err)?;
                        out.write_all(&bytes).map_err(err)?;
                        out.sync_all().map_err(err)?;
                        cache_size += bytes.len() as u64;
                    }
                } else if !target.is_file() {
                    return Err("unchanged cache missing".into());
                }
                let mut record = f.clone();
                record["lastSeenAt"] = s["checkedAt"].clone();
                prepared.push((sid.to_owned(), source.clone(), record, target));
            }
        }
        let mut c = self.db()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        if super::store::config(&tx)?["revision"] != v["revision"] {
            return Err("scan_config_conflict".into());
        }
        for (sid, source, f, target) in prepared {
            let path = f["path"].as_str().unwrap();
            let normalized = path.replace('\\', "/").to_lowercase();
            let id: Option<String> = tx
                .query_row(
                    "SELECT document_id FROM documents WHERE source_id=?1 AND normalized_path=?2",
                    params![sid, normalized],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            let id = id.unwrap_or_else(|| Uuid::new_v4().to_string());
            tx.execute("INSERT INTO documents VALUES(?1,?2,?3,?4) ON CONFLICT(source_id,normalized_path) DO UPDATE SET payload=excluded.payload",params![id,sid,normalized,f.to_string()]).map_err(err)?;
            let hash = f["sha256"].as_str().unwrap();
            let body = fs::read_to_string(&target).map_err(err)?;
            let kind = if super::publish_files::generated(&body) {
                "generatedPublication"
            } else if path.ends_with(".json") {
                "structured"
            } else if path.to_lowercase().contains("handover") {
                "handover"
            } else {
                "document"
            };
            tx.execute(
                "INSERT OR IGNORE INTO document_versions VALUES(?1,?2,?3,?4,?5,?6,NULL,?7,?8)",
                params![
                    Uuid::new_v4().to_string(),
                    id,
                    hash,
                    target.to_string_lossy(),
                    f["bytes"].as_i64().ok_or("invalid bytes")?,
                    now(),
                    kind,
                    json!({"sourceId":sid,"path":path}).to_string()
                ],
            )
            .map_err(err)?;
            if let Some(project) = source["projectId"].as_str() {
                self.require_project(project)?;
                tx.execute("INSERT OR IGNORE INTO document_project_links VALUES(?1,?2,'source','accepted')",params![id,project]).map_err(err)?;
            } else if source["scope"] == "global" {
                let reg = self.engine.registry()?;
                let projects =
                    super::parser::match_projects(path, &body, reg["projects"].as_array().unwrap());
                if projects.len() == 1 {
                    tx.execute("INSERT OR IGNORE INTO document_project_links VALUES(?1,?2,'crossChecked','accepted')",params![id,projects[0]]).map_err(err)?;
                }
            }
        }
        tx.commit().map_err(err)?;
        for s in v["sources"].as_array().unwrap() {
            self.update_source_state(s["sourceId"].as_str().unwrap(), Some(s.clone()), None)?;
        }
        self.record_observed_progress()?;
        Ok(v)
    }
    fn update_source_state(
        &self,
        id: &str,
        value: Option<Value>,
        hash_at: Option<String>,
    ) -> Result<()> {
        let c = self.db()?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS document_scan_state(source_id TEXT PRIMARY KEY REFERENCES document_sources(source_id) ON DELETE RESTRICT,payload TEXT NOT NULL)").map_err(err)?;
        let old: Option<String> = c
            .query_row(
                "SELECT payload FROM document_scan_state WHERE source_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let mut state: Value = old
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(err)?
            .unwrap_or(json!({"sourceId":id}));
        if let Some(v) = value {
            state["state"] = v["state"].clone();
            state["checkedAt"] = v["checkedAt"].clone();
            state["errors"] = v
                .get("errors")
                .cloned()
                .unwrap_or_else(|| json!([v["error"]]));
            if v["state"] == "online" {
                state["lastSuccessAt"] = v["checkedAt"].clone();
                state["failures"] = json!(0);
            } else {
                let n = state["failures"].as_u64().unwrap_or(0) + 1;
                state["failures"] = json!(n);
                state["nextRetryAt"] = json!((chrono::Utc::now()
                    + chrono::Duration::seconds([30, 60, 120, 300][(n as usize - 1).min(3)]))
                .to_rfc3339());
            }
        }
        if let Some(at) = hash_at {
            state["lastHashAt"] = json!(at)
        }
        c.execute("INSERT INTO document_scan_state VALUES(?1,?2) ON CONFLICT(source_id) DO UPDATE SET payload=excluded.payload",params![id,state.to_string()]).map_err(err)?;
        Ok(())
    }
    pub fn source_states(&self) -> Result<Value> {
        let c = self.db()?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS document_scan_state(source_id TEXT PRIMARY KEY REFERENCES document_sources(source_id) ON DELETE RESTRICT,payload TEXT NOT NULL)").map_err(err)?;
        let mut q = c
            .prepare("SELECT payload FROM document_scan_state")
            .map_err(err)?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|v| serde_json::from_str::<Value>(&v.map_err(err)?).map_err(err))
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({"sources":rows}))
    }
    pub fn list_project_documents(&self, project: &str, page: u32) -> Result<Value> {
        self.require_project(project)?;
        let c = self.db()?;
        let mut q=c.prepare("SELECT v.version_id,v.document_id,d.source_id,json_extract(v.payload,'$.path'),v.sha256,v.bytes,v.captured_at,v.kind FROM document_versions v JOIN documents d USING(document_id) JOIN document_project_links l USING(document_id) WHERE l.project_id=?1 AND l.review_state='accepted' ORDER BY v.captured_at DESC,v.version_id LIMIT 100 OFFSET ?2").map_err(err)?;
        let rows=q.query_map(params![project,i64::from(page)*100],|r|Ok(json!({"versionId":r.get::<_,String>(0)?,"documentId":r.get::<_,String>(1)?,"sourceId":r.get::<_,String>(2)?,"path":r.get::<_,String>(3)?,"sha256":r.get::<_,String>(4)?,"bytes":r.get::<_,i64>(5)?,"capturedAt":r.get::<_,String>(6)?,"kind":r.get::<_,String>(7)?}))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
        Ok(json!({"documents":rows,"page":page,"pageSize":100}))
    }
    pub fn read_document_version(&self, id: &str, start: usize, limit: usize) -> Result<Value> {
        if start == 0 || limit == 0 || limit > 1000 {
            return Err("invalid line range".into());
        }
        let c = self.db()?;
        let (hash, path): (String, String) = c
            .query_row(
                "SELECT sha256,cache_path FROM document_versions WHERE version_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        let bytes = fs::read(path).map_err(err)?;
        if sha256(&bytes) != hash {
            return Err("cache hash mismatch".into());
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(err)?
            .trim_start_matches('\u{feff}');
        let lines: Vec<_> = text.lines().collect();
        let excerpt = lines
            .iter()
            .skip(start - 1)
            .take(limit)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        Ok(
            json!({"versionId":id,"sha256":hash,"start":start,"totalLines":lines.len(),"text":excerpt}),
        )
    }
    pub fn start_document_scheduler(&self) {
        for location in ["local", "nas"] {
            let k = self.clone();
            std::thread::spawn(move || loop {
                let tick = Instant::now();
                if let (Ok(cfg), Ok(states)) = (k.get_document_config(), k.source_states()) {
                    let time = chrono::Utc::now();
                    let ids: Vec<_> = cfg["sources"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|s| {
                            s["enabled"] == true
                                && s["archived"] != true
                                && s["location"] == location
                        })
                        .filter(|s| {
                            states["sources"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .find(|v| v["sourceId"] == s["sourceId"])
                                .is_none_or(|v| {
                                    v["state"] == "online"
                                        || v["nextRetryAt"]
                                            .as_str()
                                            .and_then(|v| {
                                                chrono::DateTime::parse_from_rfc3339(v).ok()
                                            })
                                            .is_none_or(|at| time >= at)
                                })
                        })
                        .filter_map(|s| s["sourceId"].as_str().map(str::to_owned))
                        .collect();
                    if !ids.is_empty() {
                        let _ = k.scan_document_sources(&ids, false);
                    }
                    let wait = if location == "local" {
                        5
                    } else {
                        cfg["nasIntervalSec"].as_u64().unwrap_or(30)
                    };
                    std::thread::sleep(Duration::from_secs(wait).saturating_sub(tick.elapsed()));
                } else {
                    std::thread::sleep(Duration::from_secs(5));
                }
            });
        }
    }
}

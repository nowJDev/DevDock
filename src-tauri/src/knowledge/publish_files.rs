// NAS 게시 대상의 규약, 해시, 이전본과 실행 원장을 검증한다.
use super::{err, scanner::sha256, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf, Prefix},
};
use uuid::Uuid;

const FILE_LIMIT: u64 = 2 * 1024 * 1024;
const MAP: &str = "프로젝트 개발 지침/문서 배치표.json";
const RECEIPT: &str = "검토·검증 결과/문서 정리 검증/게시 영수증.json";
const GUIDE: &str = "프로젝트_문서정리_실행지침.md";
const LOCK: &str = ".devdock-publish.pending";
const GENERATED: &str = "<!-- devdock-publication ";

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub role: String,
    pub path: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Protocol {
    pub run_id: String,
    pub source_head: String,
    pub map_hash: String,
    pub receipt_path: String,
    pub receipt_hash: String,
    pub guide_hash: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prior {
    pub target: Target,
    pub sha256: String,
    pub body: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub role: String,
    pub before: String,
    pub after: String,
    pub old_text: String,
    pub new_text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileResult {
    pub path: String,
    pub state: String,
    pub message: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub state: String,
    pub files: Vec<FileResult>,
    pub message: Option<String>,
}

pub fn is_unc(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::Prefix(p))
        if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)))
}
fn relative(text: &str) -> Result<PathBuf> {
    let path = super::scanner::relative_path(text)?;
    if text.split(['/', '\\']).any(|part| {
        part.is_empty()
            || part.starts_with('.')
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c))
            || matches!(
                part.split('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_uppercase()
                    .as_str(),
                "CON"
                    | "NUL"
                    | "AUX"
                    | "PRN"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
    }) {
        return Err("잘못된 게시 상대 경로입니다.".into());
    }
    Ok(path)
}
fn path_key(text: &str) -> String {
    text.replace('\\', "/").to_lowercase()
}

pub fn validate_targets(targets: &[Target]) -> Result<()> {
    if targets.len() > 12 {
        return Err("NAS 게시 대상은 최대 12개입니다.".into());
    }
    let mut seen = HashSet::new();
    for target in targets {
        relative(&target.path)?;
        let key = path_key(&target.path);
        let allowed = match target.role.as_str() {
            "checklist" => key.starts_with("개발 진행 체크리스트/"),
            "dailyLog" => key.starts_with("일자별 작업기록/"),
            "status" => {
                key.starts_with("개발 진행 체크리스트/") || key.starts_with("일자별 작업기록/")
            }
            "index" => key == "프로젝트_문서안내.md",
            _ => false,
        };
        if !allowed || !key.ends_with(".md") || !seen.insert(key) {
            return Err("문서 역할에 맞는 서로 다른 Markdown 게시 경로를 선택해 주세요.".into());
        }
    }
    Ok(())
}
fn ordinary(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(err)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("연결 경로는 게시에 사용할 수 없습니다.".into());
        }
    }
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err("일반 파일과 폴더만 게시에 사용할 수 있습니다.".into());
    }
    Ok(())
}
fn checked(root: &Path, name: &str) -> Result<PathBuf> {
    ordinary(root, true)?;
    let relative = relative(name)?;
    let components: Vec<_> = relative.components().collect();
    let mut path = root.to_owned();
    for (index, component) in components.iter().enumerate() {
        path.push(component.as_os_str());
        ordinary(&path, index + 1 < components.len())?;
    }
    Ok(path)
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(err)?
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err("게시 문서가 파일 크기 한도를 넘었습니다.".into());
    }
    Ok(bytes)
}
fn json_bytes(bytes: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(bytes)
        .map_err(err)?
        .trim_start_matches('\u{feff}');
    serde_json::from_str(text).map_err(err)
}
fn absent(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(err(error)),
    }
}
pub fn inspect(root: &Path, targets: &[Target]) -> Result<(Protocol, Vec<Prior>)> {
    validate_targets(targets)?;
    if !absent(&root.join(LOCK))? {
        return Err("이 공유 폴더에 진행 중이거나 복구가 필요한 DevDock 게시가 있습니다.".into());
    }
    let guide = read(&checked(root, GUIDE)?)?;
    let declarations: Vec<_> = std::str::from_utf8(&guide)
        .map_err(err)?
        .lines()
        .take(12)
        .filter_map(|line| line.trim().strip_prefix("- 지침 버전. "))
        .collect();
    if declarations != ["1.3.0."] {
        return Err("지원하는 NAS 문서 규약 v1.3.0을 확인할 수 없습니다.".into());
    }
    let map_bytes = read(&checked(root, MAP)?)?;
    let map = json_bytes(&map_bytes)?;
    if map["schema_version"] != 3 {
        return Err("지원하지 않는 NAS 문서 배치표입니다.".into());
    }
    let receipt_path = map["completion_receipt"].as_str().unwrap_or(RECEIPT);
    let receipt_bytes = read(&checked(root, receipt_path)?)?;
    let receipt = json_bytes(&receipt_bytes)?;
    let meta = super::parser::publication_metadata(&map, Some(&receipt));
    if receipt["schema_version"] != 5
        || meta["receiptState"] != "published_verified"
        || !receipt["files"].is_array()
    {
        return Err(
            "같은 실행의 완료 영수증을 찾지 못했습니다. 기존 게시 상태를 확인해 주세요.".into(),
        );
    }
    let fields = |key: &str| {
        map[key]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "NAS 게시 기준이 비어 있습니다.".to_owned())
    };
    let protocol = Protocol {
        run_id: fields("run_id")?,
        source_head: fields("source_head")?,
        map_hash: sha256(&map_bytes),
        receipt_path: receipt_path.to_owned(),
        receipt_hash: sha256(&receipt_bytes),
        guide_hash: sha256(&guide),
    };
    let entries = map["entries"]
        .as_array()
        .ok_or("NAS 문서 배치 목록을 확인해 주세요.")?;
    let mut prior = Vec::new();
    for target in targets {
        if !entries.iter().any(|e| {
            e["target"]
                .as_str()
                .is_some_and(|p| path_key(p) == path_key(&target.path))
        }) && !receipt["files"].as_array().unwrap().iter().any(|e| {
            e["path"]
                .as_str()
                .is_some_and(|p| path_key(p) == path_key(&target.path))
        }) {
            return Err(format!(
                "기존 게시 문서 목록에 대상이 없습니다. {}",
                target.path
            ));
        }
        let bytes = read(&checked(root, &target.path)?)?;
        let body = std::str::from_utf8(&bytes).map_err(err)?.to_owned();
        prior.push(Prior {
            target: target.clone(),
            sha256: sha256(&bytes),
            body,
        });
    }
    Ok((protocol, prior))
}
fn recheck_protocol(root: &Path, protocol: &Protocol) -> Result<()> {
    for (name, hash) in [
        (GUIDE, &protocol.guide_hash),
        (MAP, &protocol.map_hash),
        (protocol.receipt_path.as_str(), &protocol.receipt_hash),
    ] {
        if sha256(&read(&checked(root, name)?)?) != *hash {
            return Err("NAS 문서 규약이나 게시 기준이 미리보기 이후 변경되었습니다.".into());
        }
    }
    Ok(())
}

fn owned_lock(path: &Path, run: &str) -> Result<()> {
    ordinary(path, false)?;
    if read(path)? != run.as_bytes() {
        return Err("다른 실행이 사용하는 NAS 게시 잠금입니다.".into());
    }
    Ok(())
}
fn create(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(err)
}
fn replace(path: &Path, text: &[u8], expected: &str) -> Result<()> {
    let name = path.with_file_name(format!(".devdock-publish-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        create(&name, text)?;
        ordinary(path, false)?;
        if sha256(&read(path)?) != expected {
            return Err("대상 문서가 다른 곳에서 수정되었습니다.".into());
        }
        fs::rename(&name, path).map_err(err)
    })();
    let _ = fs::remove_file(name);
    result
}
fn save_result(stage: &Path, result: &RunResult) -> Result<()> {
    let temporary = stage.join(format!("status-{}.tmp", Uuid::new_v4()));
    let path = stage.join("status.json");
    create(&temporary, &serde_json::to_vec(result).map_err(err)?)?;
    fs::rename(temporary, path).map_err(err)
}

pub fn apply(
    root: &Path,
    stage: &Path,
    run: &str,
    protocol: &Protocol,
    files: &[FileChange],
    paths: &[String],
) -> Result<RunResult> {
    ordinary(root, true)?;
    let lock = root.join(LOCK);
    recheck_protocol(root, protocol)?;
    let selected: Vec<_> = files.iter().filter(|f| paths.contains(&f.path)).collect();
    if selected.is_empty() || selected.len() != paths.len() {
        return Err("미리보기에서 게시 파일을 선택해 주세요.".into());
    }
    // Check every target before taking the lock and back up all selected files before replacing any.
    for file in &selected {
        if sha256(&read(&checked(root, &file.path)?)?) != file.before {
            return Err(format!(
                "미리보기 이후 대상이 변경되었습니다. {}",
                file.path
            ));
        }
    }
    let mut result = RunResult {
        state: "applying".into(),
        files: selected
            .iter()
            .map(|f| FileResult {
                path: f.path.clone(),
                state: "pending".into(),
                message: None,
            })
            .collect(),
        message: None,
    };
    for (i, file) in selected.iter().enumerate() {
        if sha256(file.old_text.as_bytes()) != file.before
            || sha256(file.new_text.as_bytes()) != file.after
        {
            return Err("게시 계획의 해시가 일치하지 않습니다.".into());
        }
        create(
            &stage.join(format!("before-{i}.md")),
            file.old_text.as_bytes(),
        )?;
    }
    save_result(stage, &result)?;
    if let Err(error) = create(&lock, run.as_bytes()) {
        if owned_lock(&lock, run).is_err() {
            result.state = "failed".into();
            result.message = Some(error);
            save_result(stage, &result)?;
            return Ok(result);
        }
        return Err(error);
    }
    for (i, file) in selected.iter().enumerate() {
        let write = (|| {
            owned_lock(&lock, run)?;
            recheck_protocol(root, protocol)?;
            let path = checked(root, &file.path)?;
            replace(&path, file.new_text.as_bytes(), &file.before)?;
            if sha256(&read(&path)?) != file.after {
                return Err("게시 파일을 다시 읽은 해시가 다릅니다.".into());
            }
            Ok(())
        })();
        match write {
            Ok(()) => result.files[i].state = "verified".into(),
            Err(error) => {
                result.state = "recoveryRequired".into();
                result.files[i].state = "uncertain".into();
                result.files[i].message = Some(error);
                save_result(stage, &result)?;
                return Ok(result);
            }
        }
        save_result(stage, &result)?;
    }
    for (i, file) in selected.iter().enumerate() {
        let verified = checked(root, &file.path)
            .and_then(|p| read(&p))
            .is_ok_and(|data| sha256(&data) == file.after);
        if !verified {
            result.files[i].state = "conflict".into();
            result.files[i].message =
                Some("전체 선택 파일을 다시 확인하는 중 다른 변경을 발견했습니다.".into());
            result.state = "recoveryRequired".into();
            save_result(stage, &result)?;
            return Ok(result);
        }
    }
    result.state = "filesVerified".into();
    save_result(stage, &result)?;
    owned_lock(&lock, run)?;
    fs::remove_file(lock).map_err(err)?;
    Ok(result)
}

pub fn restore(
    root: &Path,
    stage: &Path,
    run: &str,
    protocol: &Protocol,
    files: &[FileChange],
    selected: &[String],
    paths: &[String],
) -> Result<RunResult> {
    ordinary(root, true)?;
    recheck_protocol(root, protocol)?;
    let mut journal: RunResult =
        serde_json::from_slice(&read(&stage.join("status.json"))?).map_err(err)?;
    if paths.is_empty() || paths.iter().any(|p| !selected.contains(p)) {
        return Err("이 실행의 복구 파일을 선택해 주세요.".into());
    }
    let lock = root.join(LOCK);
    if absent(&lock)? {
        create(&lock, run.as_bytes())?;
    }
    owned_lock(&lock, run)?;
    let was_fully_published = matches!(
        journal.state.as_str(),
        "filesVerified" | "partiallyRestored" | "restoreConflicts"
    );
    journal.state = "restoring".into();
    save_result(stage, &journal)?;
    for (i, path) in selected.iter().enumerate() {
        if !paths.contains(path) {
            continue;
        }
        let result = (|| {
            owned_lock(&lock, run)?;
            recheck_protocol(root, protocol)?;
            let file = files
                .iter()
                .find(|f| f.path == *path)
                .ok_or("게시 파일 정보가 없습니다.")?;
            let target = checked(root, path)?;
            let current = sha256(&read(&target)?);
            if current == file.before {
                return Ok(true);
            }
            if current != file.after {
                return Ok(false);
            }
            let backup = read(&stage.join(format!("before-{i}.md")))?;
            if sha256(&backup) != file.before {
                return Err("이전본 해시가 일치하지 않습니다.".into());
            }
            replace(&target, &backup, &file.after)?;
            if sha256(&read(&target)?) != file.before {
                return Err("복구 결과 해시가 일치하지 않습니다.".into());
            }
            Ok(true)
        })();
        let item = journal
            .files
            .iter_mut()
            .find(|f| f.path == *path)
            .ok_or("복구 원장이 손상되었습니다.")?;
        match result {
            Ok(true) => {
                item.state = "restored".into();
                item.message = None;
            }
            Ok(false) => {
                item.state = "conflict".into();
                item.message = Some("게시 이후의 수동 수정이 있어 보존했습니다.".into());
            }
            Err(error) => {
                item.state = "uncertain".into();
                item.message = Some(error);
            }
        }
        save_result(stage, &journal)?;
    }
    if journal.files.iter().all(|f| f.state == "restored") {
        journal.state = "restored".into();
        save_result(stage, &journal)?;
        owned_lock(&lock, run)?;
        fs::remove_file(lock).map_err(err)?;
    } else if journal.files.iter().all(|f| {
        f.state == "restored"
            || f.state == "conflict"
            || (was_fully_published && f.state == "verified")
    }) {
        journal.state = if journal.files.iter().any(|f| f.state == "conflict") {
            "restoreConflicts"
        } else {
            "partiallyRestored"
        }
        .into();
        save_result(stage, &journal)?;
        owned_lock(&lock, run)?;
        fs::remove_file(lock).map_err(err)?;
    } else {
        journal.state = "recoveryRequired".into();
        save_result(stage, &journal)?;
    }
    Ok(journal)
}

pub fn generated(text: &str) -> bool {
    text.lines().any(|line| line.starts_with(GENERATED))
}
pub fn tag(text: &str, run: &str) -> String {
    let mut content: String = text
        .split_inclusive('\n')
        .filter(|line| !line.starts_with(GENERATED))
        .collect();
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    if !content.ends_with('\n') {
        content.push_str(eol);
    }
    content.push_str(&format!("{GENERATED}{run} -->{eol}"));
    content
}
pub fn result(path: &Path) -> Result<Value> {
    if absent(path)? {
        return Ok(json!({"state":"unknown","files":[]}));
    }
    json_bytes(&read(path)?)
}

// 배치표와 동일 실행 영수증의 표기를 읽고 문서 전체 감사 여부와 구분한다.
pub fn receipt_status(root: &Path) -> Value {
    if !absent(&root.join(LOCK)).unwrap_or(true) {
        return json!({"receiptState":"devdockPending","fullAudit":false});
    }
    let load = || -> Result<Value> {
        let map = json_bytes(&read(&checked(root, MAP)?)?)?;
        if map["schema_version"] != 3 {
            return Ok(json!({"receiptState":"unsupported","fullAudit":false}));
        }
        let path = map["completion_receipt"].as_str().unwrap_or(RECEIPT);
        let completion = checked(root, path)
            .and_then(|file| read(&file))
            .and_then(|bytes| json_bytes(&bytes))
            .ok();
        let valid = completion
            .as_ref()
            .filter(|receipt| receipt["schema_version"] == 5);
        let mut metadata = super::parser::publication_metadata(&map, valid);
        if valid.is_none() && metadata["receiptState"] == "published_verified" {
            metadata["receiptState"] = json!("unverifiedReceipt");
        }
        Ok(metadata)
    };
    load().unwrap_or(json!({"receiptState":"unknown","fullAudit":false}))
}

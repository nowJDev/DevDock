// 명시한 보고 자료만 별도 프로세스에서 전체 읽고 본문을 메모리로 반환한다.
use super::{err, scanner::sha256, KnowledgeService, Result};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub(crate) struct Output {
    pub bytes: Vec<u8>,
    pub success: bool,
    pub code: Option<i32>,
}
pub(crate) fn capture(
    mut command: Command,
    input: Option<Vec<u8>>,
    limit: usize,
    timeout: Duration,
) -> Result<Output> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(err)?;
    let stdout = child.stdout.take().ok_or("worker stdout missing")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let writer = input.map(|bytes| {
        let mut pipe = child.stdin.take().unwrap();
        std::thread::spawn(move || pipe.write_all(&bytes))
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(e) => break Err(err(e)),
            Ok(None) => {}
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            break Err("report_worker_timeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(w) = writer {
        let _ = w.join();
    }
    let bytes = reader
        .join()
        .map_err(|_| "worker reader failed")?
        .map_err(err)?;
    let status = status?;
    if bytes.len() > limit {
        return Err("worker output limit".into());
    }
    Ok(Output {
        bytes,
        success: status.success(),
        code: status.code(),
    })
}
pub fn run_report_source_worker(request: &Path) -> Result<Value> {
    let v: Value = serde_json::from_slice(&fs::read(request).map_err(err)?).map_err(err)?;
    let mut files = vec![];
    let mut total = 0;
    for path in v["paths"].as_array().ok_or("invalid selected files")? {
        let path = Path::new(path.as_str().ok_or("invalid selected path")?);
        if !path.is_absolute()
            || !matches!(
                path.extension()
                    .and_then(|s| s.to_str())
                    .map(str::to_lowercase)
                    .as_deref(),
                Some("md" | "txt")
            )
        {
            return Err("only explicit md/txt files".into());
        }
        let before = fs::symlink_metadata(path).map_err(err)?;
        if !before.is_file() || before.file_type().is_symlink() || before.len() > 2 * 1024 * 1024 {
            return Err("invalid report file or size".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if before.file_attributes() & 0x400 != 0 {
                return Err("report reparse file refused".into());
            }
        }
        let bytes = fs::read(path).map_err(err)?;
        total += bytes.len();
        if total > 8 * 1024 * 1024 {
            return Err("report inputs exceed 8MiB".into());
        }
        let after = fs::metadata(path).map_err(err)?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("report source changed while reading".into());
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(err)?
            .trim_start_matches('\u{feff}');
        let mut meta = json!({"path":path,"canonicalPath":fs::canonicalize(path).map_err(err)?,"sha256":sha256(&bytes),"bytes":bytes.len(),"lines":text.lines().count(),"nonEmpty":!text.trim().is_empty()});
        if v["includeText"] == true {
            meta["text"] = json!(text)
        }
        files.push(meta);
    }
    Ok(json!({"files":files}))
}
impl KnowledgeService {
    pub(crate) fn report_files(&self, paths: &[String], include_text: bool) -> Result<Vec<Value>> {
        if paths.is_empty() || paths.len() > 16 {
            return Err("select 1..16 report files".into());
        }
        let stage = self
            .engine
            .data_dir()
            .join("report-staging")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&stage).map_err(err)?;
        let request = stage.join("request.json");
        fs::write(
            &request,
            json!({"paths":paths,"includeText":include_text}).to_string(),
        )
        .map_err(err)?;
        let result = (|| {
            let mut command = Command::new(self.worker_executable()?);
            command.arg("--report-source-worker").arg(&request);
            let out = capture(command, None, 20 * 1024 * 1024, Duration::from_secs(30))?;
            if !out.success {
                return Err(
                    "report_source_read_failed: 정확한 파일·UTF-8·접근 권한을 확인해 주세요."
                        .into(),
                );
            }
            let value: Value = serde_json::from_slice(&out.bytes).map_err(err)?;
            Ok(value["files"]
                .as_array()
                .ok_or("invalid source worker result")?
                .clone())
        })();
        let _ = fs::remove_file(request);
        let _ = fs::remove_dir(stage);
        result
    }
}

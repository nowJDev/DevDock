// 기존 스마트원 스크립트 브리지를 제한 시간 안에 실행한다.
use serde_json::{json, Value};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
pub fn execute(script_path: &Path, bridge_path: &Path, action: &str) -> Result<Value, String> {
    execute_with_timeout(script_path, bridge_path, action, Duration::from_secs(480))
}
pub fn execute_with_timeout(
    script_path: &Path,
    bridge_path: &Path,
    action: &str,
    timeout: Duration,
) -> Result<Value, String> {
    if !["checkIn", "checkOut"].contains(&action) {
        return Err("invalid action".into());
    }
    if !script_path.is_file() || !bridge_path.is_file() {
        return Err("smartone script unavailable".into());
    }
    let file = std::env::temp_dir().join(format!("devdock-{}.json", uuid::Uuid::new_v4()));
    let output = fs::File::create(&file).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ])
    .arg(bridge_path)
    .arg("-ScriptPath")
    .arg(script_path)
    .arg("-Action")
    .arg(if action == "checkIn" {
        "CheckIn"
    } else {
        "CheckOut"
    })
    .stdout(Stdio::from(output))
    .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(s);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(100))
    };
    let raw = fs::read(&file).unwrap_or_default();
    let _ = fs::remove_file(file);
    if status.is_none() {
        return Ok(
            json!({"state":"uncertain","message":"응답 시간이 초과되어 스마트원 기록을 확인해야 합니다."}),
        );
    }
    let v: Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(_) => {
            return Ok(json!({"state":"uncertain","message":"처리 결과를 확인할 수 없습니다."}))
        }
    };
    let code = v["code"].as_str().unwrap_or("");
    let state = match code {
        "recorded" => "recorded",
        "already-in" => "alreadyIn",
        "already-out" => "alreadyOut",
        "uncertain" => "uncertain",
        "refused" | "previous-shift" | "not-in" | "rejected" => "refused",
        _ => "failed",
    };
    if !status.unwrap().success() && state != "failed" {
        return Ok(json!({"state":"uncertain","message":"처리 결과를 확인할 수 없습니다."}));
    }
    Ok(
        json!({"state":state,"code":code,"date":v["date"],"time":v["time"],"message":"스마트원 결과를 확인했습니다."}),
    )
}

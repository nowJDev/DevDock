// 기존 Codex 데몬을 읽기 전용 프록시로 관측한다.
use chrono::Local;
use serde_json::{json, Value};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    env,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub fn activity_from_status(s: &Value) -> &'static str {
    match s["type"].as_str().unwrap_or("") {
        "active" => {
            let flags = s["activeFlags"].as_array();
            if flags.is_some_and(|a| a.iter().any(|v| v == "waitingOnApproval")) {
                "approval"
            } else if flags.is_some_and(|a| a.iter().any(|v| v == "waitingOnUserInput")) {
                "input"
            } else {
                "working"
            }
        }
        "idle" => "idle",
        "systemError" => "failed",
        _ => "unknown",
    }
}
pub fn observation_from_error(id: &str, reason: &str) -> Value {
    json!({"id":id,"observedAt":Local::now().to_rfc3339(),"transportState":"unknown","activityState":"unknown","latencyMs":null,"reason":reason})
}
fn find_exe(dir: &Path, depth: u8) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_file()
            && p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("codex.exe"))
        {
            return Some(p);
        }
        if p.is_dir() {
            if let Some(found) = find_exe(&p, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}
pub fn locate_cli() -> Option<PathBuf> {
    if let Some(p) = env::var_os("CODEX_CLI")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            let p = dir.join("codex.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let home = PathBuf::from(env::var_os("USERPROFILE")?);
    let local = home.join(".local/bin/codex.exe");
    if local.is_file() {
        return Some(local);
    }
    let npm = home.join("AppData/Roaming/npm/node_modules/@openai");
    for entry in std::fs::read_dir(npm).ok()?.flatten() {
        if entry.file_name().to_string_lossy().starts_with("codex") {
            if let Some(p) = find_exe(&entry.path().join("node_modules").join("@openai"), 6) {
                return Some(p);
            }
        }
    }
    None
}
fn rpc_write(
    w: &mut impl Write,
    id: Option<u64>,
    method: &str,
    params: Value,
) -> Result<(), String> {
    if ![
        "initialize",
        "initialized",
        "thread/loaded/list",
        "thread/read",
        "thread/list",
    ]
    .contains(&method)
    {
        return Err("method forbidden".into());
    }
    let mut v = json!({"method":method,"params":params});
    if let Some(id) = id {
        v["id"] = json!(id);
    }
    writeln!(w, "{}", v)
        .and_then(|_| w.flush())
        .map_err(|e| e.to_string())
}
struct ProxyPipe {
    stdout: std::process::ChildStdout,
    stdin: std::process::ChildStdin,
}
impl Read for ProxyPipe {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.stdout.read(b)
    }
}
impl Write for ProxyPipe {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.stdin.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stdin.flush()
    }
}
fn websocket_send<S: Read + Write>(
    ws: &mut tungstenite::WebSocket<S>,
    id: Option<u64>,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let mut wire = Vec::new();
    rpc_write(&mut wire, id, method, params)?;
    let text = String::from_utf8(wire).map_err(|_| "invalid request")?;
    ws.send(tungstenite::Message::Text(text.into()))
        .map_err(|_| "WebSocket send failed".into())
}
fn websocket_response<S: Read + Write>(
    ws: &mut tungstenite::WebSocket<S>,
    id: u64,
) -> Result<Value, String> {
    loop {
        match ws.read().map_err(|_| "WebSocket read failed")? {
            tungstenite::Message::Text(text) => {
                let v: Value = serde_json::from_str(&text).map_err(|_| "invalid RPC response")?;
                if v["id"] == id {
                    if !v["error"].is_null() {
                        return Err("RPC request failed".into());
                    }
                    return Ok(v["result"].clone());
                }
            }
            tungstenite::Message::Ping(_) => {
                ws.flush().map_err(|_| "WebSocket pong failed")?;
            }
            tungstenite::Message::Close(_) => return Err("proxy closed".into()),
            _ => {}
        }
    }
}
fn loaded_states<S: Read + Write>(
    ws: &mut tungstenite::WebSocket<S>,
) -> Result<HashMap<String, &'static str>, String> {
    websocket_send(
        ws,
        Some(1),
        "initialize",
        json!({"clientInfo":{"name":"devdock","version":"0.1.0"},"capabilities":{}}),
    )?;
    websocket_response(ws, 1)?;
    websocket_send(ws, None, "initialized", json!({}))?;
    websocket_send(ws, Some(2), "thread/loaded/list", json!({"limit":100}))?;
    let loaded = websocket_response(ws, 2)?;
    let ids = loaded["data"].as_array().ok_or("invalid loaded list")?;
    let mut states = HashMap::new();
    for (i, item) in ids.iter().take(100).enumerate() {
        let tid = item
            .as_str()
            .or_else(|| item["id"].as_str())
            .ok_or("invalid thread id")?;
        websocket_send(
            ws,
            Some(i as u64 + 3),
            "thread/read",
            json!({"threadId":tid,"includeTurns":false}),
        )?;
        let r = websocket_response(ws, i as u64 + 3)?;
        states.insert(tid.to_owned(), activity_from_status(&r["thread"]["status"]));
    }
    Ok(states)
}
fn aggregate(states: &HashMap<String, &'static str>) -> &'static str {
    ["approval", "input", "working", "failed", "idle"]
        .into_iter()
        .find(|state| states.values().any(|current| current == state))
        .unwrap_or("unknown")
}
fn query_websocket<S: Read + Write>(ws: &mut tungstenite::WebSocket<S>) -> Result<String, String> {
    loaded_states(ws).map(|states| aggregate(&states).to_owned())
}
fn passive_event(value: &Value, states: &mut HashMap<String, &'static str>) -> Option<Value> {
    if value.get("id").is_some() || value["method"] != "thread/status/changed" {
        return None;
    }
    let thread = value.pointer("/params/threadId")?.as_str()?;
    if thread.is_empty()
        || thread.len() > 128
        || (!states.contains_key(thread) && states.len() >= 200)
    {
        return None;
    }
    let state = activity_from_status(value.pointer("/params/status")?);
    if state == "unknown" {
        states.remove(thread);
    } else {
        states.insert(thread.into(), state);
    }
    let status = match aggregate(states) {
        "approval" => json!({"type":"active","activeFlags":["waitingOnApproval"]}),
        "input" => json!({"type":"active","activeFlags":["waitingOnUserInput"]}),
        "working" => json!({"type":"active","activeFlags":[]}),
        "failed" => json!({"type":"systemError"}),
        "idle" => json!({"type":"idle"}),
        _ => json!({"type":"notLoaded"}),
    };
    Some(json!({"threadId":thread,"status":status}))
}
pub fn observe_status_stream(
    cli: &Path,
    home: &Path,
    mut keep: impl FnMut() -> bool,
    mut receive: impl FnMut(Value),
) {
    enum Message {
        Ready,
        Status(Value),
    }
    let mut command = Command::new(cli);
    command
        .args(["app-server", "proxy"])
        .env("CODEX_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let Ok((mut child, _owner)) = crate::child_process::spawn(&mut command) else {
        return;
    };
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (send, read) = mpsc::sync_channel(64);
    let reader = thread::spawn(move || {
        let Ok((mut ws, _)) = tungstenite::client(
            "ws://localhost/",
            ProxyPipe {
                stdin: input,
                stdout: output,
            },
        ) else {
            return;
        };
        let Ok(mut states) = loaded_states(&mut ws) else {
            return;
        };
        if send.send(Message::Ready).is_err() {
            return;
        }
        while let Ok(message) = ws.read() {
            match message {
                tungstenite::Message::Text(text) => {
                    if let Ok(data) = serde_json::from_str(&text) {
                        if let Some(event) = passive_event(&data, &mut states) {
                            if send.try_send(Message::Status(event)).is_err() {
                                break;
                            }
                        }
                    }
                }
                tungstenite::Message::Ping(_) => {
                    if ws.flush().is_err() {
                        break;
                    }
                }
                tungstenite::Message::Close(_) => break,
                _ => {}
            }
        }
    });
    let start = Instant::now();
    let mut ready = false;
    while keep() && start.elapsed() < Duration::from_secs(300) {
        match read.recv_timeout(Duration::from_secs(1)) {
            Ok(Message::Ready) => ready = true,
            Ok(Message::Status(value)) if ready => receive(value),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout)
                if ready || start.elapsed() < Duration::from_secs(3) => {}
            Err(_) => break,
        }
    }
    drop(read);
    let _ = child.kill();
    let _ = child.wait();
    drop(_owner);
    let _ = reader.join();
}
fn wait_for_proxy(
    rx: &mpsc::Receiver<Result<String, String>>,
    timeout: Duration,
) -> Result<String, String> {
    rx.recv_timeout(timeout)
        .map_err(|_| "proxy timeout or closed".to_string())
        .and_then(|r| r)
}
pub fn observe(id: &str, cli: &Path, home: &Path) -> Value {
    let started = Instant::now();
    let result = (|| -> Result<String, String> {
        let mut cmd = Command::new(cli);
        cmd.args(["app-server", "proxy"])
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let (mut child, _owner) =
            crate::child_process::spawn(&mut cmd).map_err(|_| "proxy spawn failed")?;
        let input = child.stdin.take().ok_or("missing stdin")?;
        let output = child.stdout.take().ok_or("missing stdout")?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = tungstenite::client(
                "ws://localhost/",
                ProxyPipe {
                    stdin: input,
                    stdout: output,
                },
            )
            .map_err(|_| "WebSocket handshake failed".to_string())
            .and_then(|(mut ws, _)| query_websocket(&mut ws));
            let _ = tx.send(result);
        });
        let result = wait_for_proxy(&rx, Duration::from_secs(3));
        let _ = child.kill();
        let _ = child.wait();
        result
    })();
    match result {
        Ok(activity) => {
            json!({"id":id,"observedAt":Local::now().to_rfc3339(),"transportState":if started.elapsed() > Duration::from_secs(2) {"slow"} else {"healthy"},"activityState":activity,"latencyMs":started.elapsed().as_millis() as u64,"reason":"읽기 전용 스냅샷과 지원되는 상태 알림"})
        }
        Err(reason) => observation_from_error(id, &format!("프록시 상태 조회 불가: {reason}")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rpc_never_sends_forbidden_method() {
        let mut wire = Vec::new();
        assert!(rpc_write(&mut wire, Some(1), "turn/start", json!({})).is_err());
        assert!(wire.is_empty());
    }
}
#[cfg(test)]
mod websocket_tests {
    use super::*;
    use std::io::{self, Read, Write};
    struct Endpoint {
        tx: mpsc::Sender<Vec<u8>>,
        rx: mpsc::Receiver<Vec<u8>>,
        pending: std::io::Cursor<Vec<u8>>,
    }
    impl Read for Endpoint {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.pending.position() as usize == self.pending.get_ref().len() {
                self.pending =
                    std::io::Cursor::new(self.rx.recv().map_err(|_| io::ErrorKind::UnexpectedEof)?);
            }
            self.pending.read(out)
        }
    }
    impl Write for Endpoint {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.tx
                .send(b.to_vec())
                .map_err(|_| io::ErrorKind::BrokenPipe)?;
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn endpoints() -> (Endpoint, Endpoint) {
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        (
            Endpoint {
                tx: a_tx,
                rx: b_rx,
                pending: std::io::Cursor::new(vec![]),
            },
            Endpoint {
                tx: b_tx,
                rx: a_rx,
                pending: std::io::Cursor::new(vec![]),
            },
        )
    }
    #[test]
    fn proxy_websocket_queries_loaded_thread_without_resuming() {
        let (client, server) = endpoints();
        let server = std::thread::spawn(move || {
            let mut ws = tungstenite::accept(server).unwrap();
            for (method, result) in [
                ("initialize", json!({})),
                ("initialized", Value::Null),
                ("thread/loaded/list", json!({"data":["thread-id"]})),
                (
                    "thread/read",
                    json!({"thread":{"status":{"type":"active","activeFlags":["waitingOnApproval"]}}}),
                ),
            ] {
                let msg = ws.read().unwrap().into_text().unwrap();
                let v: Value = serde_json::from_str(&msg).unwrap();
                assert_eq!(v["method"], method);
                if !v["id"].is_null() {
                    ws.send(tungstenite::Message::Text(
                        json!({"id":v["id"],"result":result}).to_string().into(),
                    ))
                    .unwrap();
                }
            }
        });
        let (mut ws, _) = tungstenite::client("ws://localhost/", client).unwrap();
        assert_eq!(query_websocket(&mut ws).unwrap(), "approval");
        server.join().unwrap();
    }
    #[test]
    fn proxy_wait_enforces_deadline_when_child_stalls() {
        let (_tx, rx) = mpsc::channel();
        let started = Instant::now();
        assert!(wait_for_proxy(&rx, Duration::from_millis(20)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

#[cfg(test)]
#[test]
fn passive_status_preserves_other_active_threads_and_ignores_server_requests() {
    use std::collections::HashMap;
    let mut states = HashMap::from([("other".to_owned(), "approval")]);
    let mut update = json!({"method":"thread/status/changed","params":{"threadId":"worker","status":{"type":"idle"}}});
    let event = passive_event(&update, &mut states).unwrap();
    assert_eq!(event["status"]["activeFlags"][0], "waitingOnApproval");
    update["id"] = json!(52);
    assert!(passive_event(&update, &mut states).is_none());
}

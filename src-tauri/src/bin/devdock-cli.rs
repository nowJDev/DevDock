// 공통 핵심 서비스를 이용해 등록 정보와 관측을 조작한다.
use devdock_core::service::Engine;
use serde_json::{json, Value};
use std::{env, path::PathBuf, process};

fn parse_json(s: &str) -> Result<Value, String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}
fn revision(s: Option<&String>) -> Result<Option<u64>, String> {
    s.map(|v| v.parse::<u64>().map_err(|e| e.to_string()))
        .transpose()
}
fn run(engine: &Engine, args: &[String]) -> Result<Value, String> {
    if let Some((request, preview)) = devdock_core::knowledge::commands::cli_request(args)? {
        return engine.knowledge().execute_command(request, preview, true);
    }
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["snapshot"] => engine.snapshot(),
        ["registry"] => engine.registry(),
        ["settings"] => engine.settings(),
        ["settings", value] => engine.save_settings_with(parse_json(value)?, |old,new| { if old["autostartEnabled"] != new["autostartEnabled"] { Err("자동 시작 변경은 DevDock 앱에서 수행해야 합니다.".into()) } else { Ok(()) } }),
        ["history", id, hours] => engine.history(id, hours.parse::<u32>().map_err(|e| e.to_string())?),
        ["event", value] => engine.record_event(parse_json(value)?),
        ["validate"] => {engine.registry()?; engine.settings()?; Ok(json!({"valid":true}))},
        ["mutate", value] => engine.mutate(parse_json(value)?, None),
        ["mutate", value, _] => engine.mutate(parse_json(value)?, revision(args.get(2))?),
        [entity @ ("project" | "monitor"), "list"] => {
            let key = if *entity == "project" {"projects"} else {"monitors"};
            Ok(engine.registry()?[key].clone())
        },
        [entity @ ("project" | "monitor"), "add" | "update", value] =>
            engine.mutate(json!({"entity":entity,"action":"upsert","value":parse_json(value)?}), None),
        [entity @ ("project" | "monitor"), "remove", id] =>
            engine.mutate(json!({"entity":entity,"action":"remove","id":id}), None),
        ["task", "upsert", value] => engine.mutate(json!({"entity":"task","action":"upsert","value":parse_json(value)?}), None),
        ["task", "archive", id] => engine.mutate(json!({"entity":"task","action":"archive","id":id}), None),
        ["status", "publish", value] => engine.mutate(json!({"entity":"status","action":"publish","value":parse_json(value)?}), None),
        ["evidence", "add", value] => engine.mutate(json!({"entity":"evidence","action":"upsert","value":parse_json(value)?}), None),
        _ => Err("usage: devdock-cli snapshot|registry|settings [JSON]|history ID HOURS|event JSON|validate|mutate JSON [revision]|project/monitor list/add/update/remove|task upsert/archive|status publish|evidence add".into()),
    }
}
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let [flag, path] = args.as_slice() {
        if flag == "--document-scan-worker" {
            if devdock_core::knowledge::scanner::run_scan_worker(std::path::Path::new(path))
                .is_err()
            {
                process::exit(1)
            }
            return;
        }
    }
    if let [group, command, path] = args.as_slice() {
        if group == "documents" && command == "scan-worker" {
            if devdock_core::knowledge::scanner::run_scan_worker(std::path::Path::new(path))
                .is_err()
            {
                process::exit(1)
            }
            return;
        }
    }
    let result = (|| -> Result<Value, String> {
        let dir = env::var_os("DEVDOCK_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("DevDock")))
            .ok_or("data directory unavailable")?;
        run(&Engine::connect(dir)?, &args)
    })();
    match result {
        Ok(v) => println!(
            "{}",
            json!({"ok":true,"revision":v.get("revision"),"data":v})
        ),
        Err(e) => {
            eprintln!(
                "{}",
                json!({"ok":false,"errorCode":"operation_failed","message":e})
            );
            process::exit(1)
        }
    }
}

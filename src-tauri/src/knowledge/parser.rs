// 정형 문서에서 출처가 있는 검토 후보만 추출하고 게시 표기를 분리한다.
use super::{
    err,
    scanner::{relative_path, sha256},
    Result,
};
use serde_json::{json, Value};
pub fn date(value: &str) -> Result<String> {
    let parsed = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(err)?;
    if parsed.to_string() != value {
        return Err("invalid date".into());
    }
    Ok(value.into())
}
fn dates(text: &str) -> Option<String> {
    text.as_bytes()
        .windows(10)
        .filter_map(|b| std::str::from_utf8(b).ok())
        .find_map(|s| date(s).ok())
}
pub fn match_projects(filename: &str, body: &str, projects: &[Value]) -> Vec<String> {
    let normalized = filename.replace('\\', "/").to_lowercase();
    let leaf = normalized.rsplit('/').next().unwrap_or(&normalized);
    let tokenize = |value: &str| {
        value
            .split(|c: char| !c.is_alphanumeric())
            .filter(|part| !part.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
    };
    let tokens = tokenize(leaf);
    let explicit: Vec<_> = projects
        .iter()
        .filter(|p| p["archived"] != true)
        .filter(|p| {
            p["referenceDocuments"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|v| {
                    v.as_str()
                        .is_some_and(|s| s.replace('\\', "/").to_lowercase() == normalized)
                })
        })
        .filter_map(|p| p["id"].as_str().map(str::to_owned))
        .collect();
    if !explicit.is_empty() {
        return explicit;
    }
    let body = body.replace('\\', "/").to_lowercase();
    projects
        .iter()
        .filter(|p| p["archived"] != true)
        .filter(|p| {
            let id = p["id"].as_str().unwrap_or("").to_lowercase();
            let name = p["name"].as_str().unwrap_or("").to_lowercase();
            let root = p["root"]
                .as_str()
                .unwrap_or("")
                .replace('\\', "/")
                .trim_end_matches('/')
                .to_lowercase();
            let token = [id, name].iter().map(|alias| tokenize(alias)).any(|label| {
                !label.is_empty()
                    && tokens
                        .windows(label.len())
                        .any(|sequence| sequence == label)
            });
            token
                && !root.is_empty()
                && body.match_indices(&root).any(|(at, _)| {
                    let boundary = |c: char| c.is_whitespace() || "`'\"()[]<>,;".contains(c);
                    body[..at].chars().next_back().is_none_or(boundary)
                        && body[at + root.len()..]
                            .chars()
                            .next()
                            .is_none_or(|c| c == '/' || boundary(c))
                })
        })
        .filter_map(|p| p["id"].as_str().map(str::to_owned))
        .collect()
}
fn span(version: &str, hash: &str, locator: Value) -> Value {
    json!({"versionId":version,"sha256":hash,"locator":locator})
}
fn candidate(
    kind: &str,
    change: Value,
    spans: Vec<Value>,
    work: Option<&str>,
    authored: Option<&str>,
) -> Value {
    let mut v =
        json!({"kind":kind,"proposedChanges":change,"sourceSpans":spans,"reviewState":"pending"});
    if let Some(d) = work {
        v["workDate"] = json!(d)
    }
    if let Some(d) = authored {
        v["authoredDate"] = json!(d)
    }
    v["fingerprint"] = json!(sha256(v.to_string().as_bytes()));
    v
}
fn task_state(state: &str) -> Result<&'static str> {
    match state {
        "done" | "completed" | "reportedComplete" => Ok("reportedComplete"),
        "waiting" | "blocked" => Ok("blocked"),
        "planned" | "pending" => Ok("pending"),
        "inProgress" => Ok("inProgress"),
        "cancelled" => Ok("cancelled"),
        _ => Err(format!("unsupported task state: {state}")),
    }
}
pub fn parse_document(version: &str, hash: &str, text: &str, path: &str) -> Result<Vec<Value>> {
    let text = text.trim_start_matches('\u{feff}');
    if path.to_lowercase().ends_with(".json") {
        let v: Value = serde_json::from_str(text).map_err(err)?;
        if v.get("run_id").is_some() || v.get("publication_status").is_some() {
            return Ok(vec![]);
        }
        let Some(tasks) = v["tasks"].as_array() else {
            return Ok(vec![]);
        };
        let mut out = vec![];
        for (i, t) in tasks.iter().enumerate() {
            let id = t["id"].as_str().ok_or("task id missing")?;
            let state = t
                .get("status")
                .or_else(|| t.get("state"))
                .and_then(Value::as_str)
                .ok_or("task state missing")?;
            out.push(json!({"taskId":id,"title":t.get("title").or_else(||t.get("name")).and_then(Value::as_str).unwrap_or(id),"revision":t["revision"].as_i64().filter(|v|*v>0).unwrap_or(1),"state":task_state(state)?,"parentId":t["parentId"].as_str().or_else(||t["parent_id"].as_str()),"sourceSpan":span(version,hash,json!({"jsonPointer":format!("/tasks/{i}")}))}));
        }
        return Ok(vec![candidate(
            "basisDefinition",
            json!({"name":path,"kind":if v["schema_version"]==1{"development"}else{"summary"},"tasks":out}),
            vec![span(version, hash, json!({"jsonPointer":"/tasks"}))],
            None,
            None,
        )]);
    }
    let authored = text.lines().find(|s| s.contains("작성일")).and_then(dates);
    let mut work = text
        .lines()
        .find(|s| s.contains("업무일") || s.contains("작업일"))
        .and_then(dates)
        .or_else(|| authored.clone());
    let mut actual = false;
    let mut fenced = false;
    let mut result = vec![];
    let mut tasks = vec![];
    let mut parents: Vec<(usize, String)> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if line.starts_with('#') {
            parents.clear();
            if let Some(d) = dates(line) {
                work = Some(d)
            }
            actual = ["진행 내용", "수행 내용", "결과", "완료 내역", "작업 내용"]
                .iter()
                .any(|s| line.contains(s));
            if [
                "목표",
                "계획",
                "다음",
                "미완료",
                "이전 snapshot",
                "과거 snapshot",
            ]
            .iter()
            .any(|s| line.contains(s))
            {
                actual = false
            }
            continue;
        }
        let checkbox = line
            .strip_prefix("- [")
            .or_else(|| line.strip_prefix("* ["));
        if let Some(check) = checkbox {
            let Some((mark, title)) = check.split_once("] ") else {
                continue;
            };
            let id = title
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(['.', ':']);
            if id.is_empty() {
                continue;
            }
            let state = match mark {
                "x" | "X" => "reportedComplete",
                " " => "pending",
                ">" => "inProgress",
                "!" => "blocked",
                _ => continue,
            };
            let indent = raw
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum();
            while parents.last().is_some_and(|(depth, _)| *depth >= indent) {
                parents.pop();
            }
            tasks.push(json!({"taskId":id,"title":title,"revision":1,"state":state,
                "parentId":parents.last().map(|(_,parent)|parent),
                "sourceSpan":span(version,hash,json!({"lines":{"start":i+1,"end":i+1}}))}));
            parents.push((indent, id.into()));
            continue;
        }
        if actual && !line.is_empty() && work.is_some() {
            result.push(candidate(
                "workEntry",
                json!({"text":line.trim_start_matches(['-','*',' ']),"coverage":"partial"}),
                vec![span(
                    version,
                    hash,
                    json!({"lines":{"start":i+1,"end":i+1}}),
                )],
                work.as_deref(),
                authored.as_deref(),
            ));
        }
    }
    if !tasks.is_empty() {
        result.push(candidate(
            "basisDefinition",
            json!({"name":path,"kind":if tasks.iter().all(|t|t["taskId"].as_str().is_some_and(|s|s.to_ascii_uppercase().starts_with("UAT-"))){"acceptance"}else{"development"},"tasks":tasks}),
            vec![span(
                version,
                hash,
                json!({"lines":{"start":1,"end":text.lines().count().max(1)}}),
            )],
            None,
            authored.as_deref(),
        ))
    }
    Ok(result)
}
pub fn publication_metadata(map: &Value, completion: Option<&Value>) -> Value {
    let mut state = map
        .get("publication_status")
        .or_else(|| map.get("status"))
        .cloned()
        .unwrap_or(json!("unknown"));
    if let Some(receipt) = completion {
        if map["run_id"] == receipt["run_id"]
            && map["source_head"] == receipt["source_head"]
            && map["run_id"].is_string()
        {
            state = receipt["status"].clone()
        } else {
            state = json!("conflict")
        }
    }
    if map["completion_receipt"]
        .as_str()
        .is_some_and(|p| relative_path(p).is_err())
    {
        state = json!("conflict")
    }
    json!({"runId":map["run_id"],"sourceHead":map["source_head"],"receiptState":state,"fullAudit":false,"verification":"receiptClaimOnly"})
}

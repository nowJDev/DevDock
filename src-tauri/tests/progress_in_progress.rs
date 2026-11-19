// 진행 중 수치가 새 일별 기록에 저장되고 이전 기록은 미기록으로 유지되는지 검증한다.
use chrono::{Duration, Local};
use devdock_core::service::Engine;
use rusqlite::{params, Connection};
use serde_json::json;
use std::fs;

#[test]
fn keeps_unknown_previous_count_and_records_in_progress_changes() {
    let dir = tempfile::tempdir().unwrap();
    let db = Connection::open(dir.path().join("devdock.sqlite3")).unwrap();
    let old = Local::now() - Duration::days(1);
    db.execute_batch(
        "CREATE TABLE progress(id INTEGER PRIMARY KEY,at TEXT NOT NULL,project_id TEXT NOT NULL,total INTEGER,verified INTEGER,reported INTEGER,collected_date TEXT);
         CREATE TABLE daily_progress(project_id TEXT NOT NULL,date TEXT NOT NULL,at TEXT NOT NULL,total INTEGER NOT NULL,verified INTEGER NOT NULL,reported INTEGER NOT NULL,tracking_basis TEXT,source_captured_at TEXT,PRIMARY KEY(project_id,date));"
    ).unwrap();
    db.execute("INSERT INTO daily_progress(project_id,date,at,total,verified,reported) VALUES('p',?1,?2,2,0,1)", params![old.date_naive().to_string(),old.to_rfc3339()]).unwrap();
    let e = Engine::open(dir.path().into()).unwrap();
    let path = dir.path().join("tasks.md");
    fs::write(&path, "- [x] one\n- [-] two\n").unwrap();
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":dir.path(),"checklistPath":"tasks.md"}}), None).unwrap();
    let snapshot = e.snapshot().unwrap();
    assert_eq!(snapshot["projects"][0]["inProgress"], 1);
    let counts = e.history("p", 72).unwrap();
    assert_eq!(
        counts["dailyProgress"][0]["inProgress"],
        serde_json::Value::Null
    );
    assert_eq!(counts["dailyProgress"][1]["inProgress"], 1);
    assert_eq!(counts["progress"][0]["inProgress"], 1);

    fs::write(&path, "- [x] one\n- [ ] two\n").unwrap();
    e.snapshot().unwrap();
    let counts = e.history("p", 72).unwrap();
    assert_eq!(counts["dailyProgress"][1]["inProgress"], 0);
    assert_eq!(counts["progress"][1]["inProgress"], 0);
    let range = e
        .knowledge()
        .get_project_history(
            "p",
            "legacy",
            &old.date_naive().to_string(),
            &Local::now().date_naive().to_string(),
        )
        .unwrap();
    assert_eq!(
        range["points"][0]["counts"]["inProgress"],
        serde_json::Value::Null
    );
    assert_eq!(range["points"][1]["counts"]["inProgress"], 0);
}

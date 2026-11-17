// 핵심 상태와 원장의 외부 계약을 검증한다.
use devdock_core::service::Engine;
use serde_json::json;
use tempfile::tempdir;

fn recent_at(value: &str) -> String {
    static BASE: std::sync::OnceLock<chrono::DateTime<chrono::Utc>> = std::sync::OnceLock::new();
    let base = *BASE.get_or_init(|| chrono::Utc::now() - chrono::Duration::seconds(2));
    let anchor = chrono::DateTime::parse_from_rfc3339("2026-11-17T10:00:00Z").unwrap();
    let original = chrono::DateTime::parse_from_rfc3339(value).unwrap();
    (base + original.signed_duration_since(anchor))
        .with_timezone(original.offset())
        .to_rfc3339()
}

#[test]
fn schedule_boundaries_and_daily_policy() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    for (t, expected) in [
        ("2026-11-22T08:09:59", false),
        ("2026-11-22T08:10:00", true),
        ("2026-11-22T08:29:59", true),
        ("2026-11-22T08:30:00", false),
    ] {
        assert_eq!(e.evaluate_schedule(t).unwrap()["checkIn"], expected);
    }
    assert_eq!(
        e.evaluate_schedule("2026-11-22T17:29:59").unwrap()["checkoutPrompt"],
        false
    );
    assert_eq!(
        e.evaluate_schedule("2026-11-22T17:30:00").unwrap()["checkoutPrompt"],
        true
    );
}
#[test]
fn ledger_prevents_duplicate_and_recovers_uncertain() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let id = e.claim_run("2026-11-17", "checkIn").unwrap().unwrap();
    assert!(e.claim_run("2026-11-17", "checkIn").unwrap().is_none());
    drop(e);
    let e = Engine::open(d.path().into()).unwrap();
    assert!(e.claim_run("2026-11-17", "checkIn").unwrap().is_none());
    let snap = e.snapshot().unwrap();
    assert!(snap["attendance"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["attemptId"] == id && x["state"] == "uncertain"));
    assert!(e.claim_prompt("2026-11-17").unwrap());
    assert!(!e.claim_prompt("2026-11-17").unwrap());
}
#[test]
fn registry_conflict_and_corruption_are_not_overwritten() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let op = json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":"."}});
    e.mutate(op.clone(), Some(0)).unwrap();
    assert!(e.mutate(op, Some(0)).is_err());
    std::fs::write(d.path().join("registry.json"), "{").unwrap();
    assert!(e.registry().is_err());
    assert_eq!(
        std::fs::read_to_string(d.path().join("registry.json")).unwrap(),
        "{"
    );
}
#[test]
fn verified_requires_matching_evidence_and_parent_is_not_double_counted() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    for (entity, value) in [
        ("project", json!({"id":"p","name":"P","root":"."})),
        (
            "task",
            json!({"id":"parent","projectId":"p","title":"Parent","state":"reportedComplete","revision":"1"}),
        ),
        (
            "task",
            json!({"id":"child","projectId":"p","parentId":"parent","title":"Child","state":"reportedComplete","revision":"1"}),
        ),
    ] {
        e.mutate(
            json!({"entity":entity,"action":"upsert","value":value}),
            None,
        )
        .unwrap();
    }
    let s = e.snapshot().unwrap();
    assert_eq!(s["projects"][0]["total"], 1);
    assert_eq!(s["projects"][0]["reportedComplete"], 1);
    assert_eq!(s["projects"][0]["verified"], 0);
    e.mutate(json!({"entity":"evidence","action":"upsert","value":{"id":"ev","taskId":"child","kind":"test","success":true,"subjectRevision":"1","reference":"test"}}),None).unwrap();
    assert_eq!(e.snapshot().unwrap()["projects"][0]["verified"], 1);
}
#[test]
fn checklist_ignores_fences() {
    let d = tempdir().unwrap();
    let path = d.path().join("tasks.md");
    std::fs::write(&path, "- [x] done\n~~~md\n- [x] example\n~~~\n- [ ] todo\n").unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":".","checklistPath":path}}),None).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s["projects"][0]["total"], 2);
    assert_eq!(s["projects"][0]["verified"], 0);
}
#[test]
fn events_do_not_revive_old_activity_after_disconnect() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"unknown","observedAt":recent_at("2026-11-17T09:59:59Z")})).unwrap();
    e.record_event(
        json!({"id":"new","daemonId":"A","type":"turn.started","at":recent_at("2026-11-17T10:00:02Z")}),
    )
    .unwrap();
    e.record_event(
        json!({"id":"old","daemonId":"A","type":"turn.completed","at":recent_at("2026-11-17T10:00:01Z")}),
    )
    .unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "working"
    );
    e.record_observation(
        json!({"id":"A","transportState":"disconnected","observedAt":recent_at("2026-11-17T10:00:03Z")}),
    )
    .unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "unknown"
    );
}

#[test]
fn concurrent_registry_writes_respect_revision() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles=(0..2).map(|i|{let e=e.clone();let gate=gate.clone();std::thread::spawn(move || {gate.wait();e.mutate(json!({"entity":"project","action":"upsert","value":{"id":format!("p{i}"),"name":"P","root":"."}}),Some(0))})}).collect::<Vec<_>>();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|r| r.is_ok())
            .count(),
        1
    );
    assert_eq!(e.registry().unwrap()["revision"], 1);
}
#[test]
fn reconnect_does_not_restore_activity_from_before_disconnect() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_event(
        json!({"id":"a","daemonId":"A","type":"turn.started","at":recent_at("2026-11-17T10:00:00Z")}),
    )
    .unwrap();
    e.record_observation(
        json!({"id":"A","transportState":"disconnected","observedAt":recent_at("2026-11-17T10:00:01Z")}),
    )
    .unwrap();
    e.record_observation(
        json!({"id":"A","transportState":"healthy","observedAt":recent_at("2026-11-17T10:00:02Z")}),
    )
    .unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "unknown"
    );
}
#[test]
fn cli_connect_does_not_recover_live_attempt() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let id = e.claim_run("2026-11-17", "checkIn").unwrap().unwrap();
    let _reader = Engine::connect(d.path().into()).unwrap();
    assert!(e.finish_run(&id, json!({"state":"recorded"})).is_ok());
}

#[test]
fn project_history_tracks_change_in_verified_progress() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":"."}}),
        None,
    )
    .unwrap();
    e.mutate(json!({"entity":"task","action":"upsert","value":{"id":"t","projectId":"p","title":"T","state":"reportedComplete","revision":"1"}}),None).unwrap();
    e.mutate(json!({"entity":"evidence","action":"upsert","value":{"id":"e","taskId":"t","kind":"test","success":true,"subjectRevision":"1","reference":"test"}}),None).unwrap();
    let h = e.history("p", 24).unwrap();
    assert!(h["progress"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["total"] == 1 && x["verified"] == 1));
}
#[cfg(windows)]
#[test]
fn fake_bridge_result_and_malformed_output_are_distinguished() {
    use devdock_core::smartone::execute;
    let d = tempdir().unwrap();
    let script = d.path().join("fake.ps1");
    let bridge = d.path().join("bridge.ps1");
    std::fs::write(&script, "# fake\n").unwrap();
    std::fs::write(&bridge,"param($ScriptPath,$Action)\nWrite-Output '{\"code\":\"already-in\",\"date\":\"2026-11-17\",\"time\":\"08:15\"}'\n").unwrap();
    assert_eq!(
        execute(&script, &bridge, "checkIn").unwrap()["state"],
        "alreadyIn"
    );
    std::fs::write(
        &bridge,
        "param($ScriptPath,$Action)\nWrite-Output 'broken'\n",
    )
    .unwrap();
    assert_eq!(
        execute(&script, &bridge, "checkIn").unwrap()["state"],
        "uncertain"
    );
}

#[test]
fn published_and_file_status_validate_tasks_and_metrics() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":"."}}),
        None,
    )
    .unwrap();
    let bad = json!({"entity":"status","action":"publish","value":{"projectId":"p","tasks":[{"id":"t","projectId":"other","title":"T","state":"inProgress","revision":"1"}]}});
    assert!(e.mutate(bad, None).is_err());
    e.mutate(json!({"entity":"status","action":"publish","value":{"projectId":"p","currentTask":"Review","tasks":[{"id":"t","projectId":"p","title":"T","state":"inProgress","revision":"1"}]}}),None).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["projects"][0]["currentTask"],
        "Review"
    );
    assert_eq!(e.snapshot().unwrap()["projects"][0]["total"], 1);
    let path = d.path().join("status.json");
    std::fs::write(&path,json!({"projectId":"p","currentTask":"File task","tasks":[],"metrics":[{"id":"latency","value":12,"unit":"ms","state":"healthy"}]}).to_string()).unwrap();
    e.mutate(json!({"entity":"monitor","action":"upsert","value":{"id":"m","projectId":"p","kind":"structuredStatus","name":"M","source":path,"enabled":true}}),None).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["projects"][0]["currentTask"],
        "File task"
    );
    assert!(e.history("m", 24).unwrap()["points"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["value"] == 12));
    std::fs::write(&path, "bad").unwrap();
    let s = e.snapshot().unwrap();
    assert_ne!(s["projects"][0]["currentTask"], "File task");
    assert!(e.history("m", 24).unwrap()["points"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["state"] == "unknown" && v["value"].is_null()));
}
#[test]
fn rpc_events_ignore_late_old_turn_and_hide_raw_text() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"unknown","observedAt":recent_at("2026-11-17T09:59:59Z")})).unwrap();
    for (method, turn, at) in [
        ("turn/started", "t1", "2026-11-17T10:00:00Z"),
        ("turn/started", "t2", "2026-11-17T10:00:01Z"),
        ("turn/completed", "t1", "2026-11-17T10:00:02Z"),
        ("item/started", "t2", "2026-11-17T10:00:03Z"),
    ] {
        e.record_event(json!({"daemonId":"A","at":recent_at(at),"method":method,"params":{"threadId":"th","turnId":turn,"turn":{"id":turn},"item":{"type":"commandExecution","command":"SECRET"}}})).unwrap();
    }
    let s = e.snapshot().unwrap();
    assert_eq!(s["daemons"][0]["activityState"], "toolRunning");
    assert!(!s.to_string().contains("SECRET"));
    e.record_event(json!({"daemonId":"A","at":recent_at("2026-11-17T10:00:04Z"),"method":"item/completed","params":{"threadId":"th","turnId":"t2","item":{"type":"commandExecution"}}})).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "working"
    );
}
#[test]
fn checklist_history_records_only_content_changes() {
    let d = tempdir().unwrap();
    let path = d.path().join("tasks.md");
    std::fs::write(&path, "- [ ] one\n").unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":".","checklistPath":path}}),None).unwrap();
    let before = e.history("p", 24).unwrap()["progress"]
        .as_array()
        .unwrap()
        .len();
    e.snapshot().unwrap();
    e.snapshot().unwrap();
    assert_eq!(
        e.history("p", 24).unwrap()["progress"]
            .as_array()
            .unwrap()
            .len(),
        before
    );
    std::fs::write(&path, "- [x] one\n").unwrap();
    e.snapshot().unwrap();
    let h = e.history("p", 24).unwrap();
    assert_eq!(h["progress"].as_array().unwrap().len(), before + 1);
    assert_eq!(
        h["progress"].as_array().unwrap().last().unwrap()["reportedComplete"],
        1
    );
}
#[cfg(windows)]
#[test]
fn fake_slow_bridge_is_killed_and_marked_uncertain() {
    let d = tempdir().unwrap();
    let script = d.path().join("fake.ps1");
    let bridge = d.path().join("slow.ps1");
    std::fs::write(&script, "# fake").unwrap();
    std::fs::write(
        &bridge,
        "param($ScriptPath,$Action)\nStart-Sleep -Seconds 10\n",
    )
    .unwrap();
    let start = std::time::Instant::now();
    let v = devdock_core::smartone::execute_with_timeout(
        &script,
        &bridge,
        "checkIn",
        std::time::Duration::from_millis(200),
    )
    .unwrap();
    assert_eq!(v["state"], "uncertain");
    assert!(start.elapsed() < std::time::Duration::from_secs(4));
}

#[test]
fn old_turn_restart_and_completion_cannot_replace_new_turn_or_plan() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"unknown","observedAt":recent_at("2026-11-17T09:59:59Z")})).unwrap();
    for (typ, turn, at) in [
        ("turn/started", "one", "2026-11-17T10:00:00Z"),
        ("turn/started", "two", "2026-11-17T10:00:01Z"),
        ("turn/started", "one", "2026-11-17T10:00:02Z"),
        ("turn/completed", "one", "2026-11-17T10:00:03Z"),
    ] {
        e.record_event(
            json!({"daemonId":"A","method":typ,"at":recent_at(at),"params":{"threadId":"th","turnId":turn}}),
        )
        .unwrap();
    }
    e.record_event(json!({"daemonId":"A","method":"turn/plan/updated","at":recent_at("2026-11-17T10:00:04Z"),"params":{"threadId":"th","turnId":"two","plan":[{"step":"Implement","status":"inProgress"}]}})).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s["daemons"][0]["activityState"], "working");
    assert_eq!(s["daemons"][0]["plan"][0]["step"], "Implement");
    e.record_event(json!({"daemonId":"A","method":"thread/status/changed","at":recent_at("2026-11-17T10:00:05Z"),"params":{"threadId":"th","status":{"type":"idle"}}})).unwrap();
    assert_eq!(e.snapshot().unwrap()["daemons"][0]["activityState"], "idle");
}

#[test]
fn latest_observation_and_event_choose_current_activity_safely() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"working","observedAt":recent_at("2026-11-17T19:00:00+09:00")})).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "working"
    );
    e.record_event(
        json!({"id":"new","daemonId":"A","type":"turn.completed","at":recent_at("2026-11-17T10:00:01Z")}),
    )
    .unwrap();
    assert_eq!(e.snapshot().unwrap()["daemons"][0]["activityState"], "idle");
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"toolRunning","observedAt":recent_at("2026-11-17T19:00:02+09:00")})).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "toolRunning"
    );
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"unknown","observedAt":recent_at("2026-11-17T19:00:03+09:00")})).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "unknown"
    );
    e.record_event(
        json!({"id":"later","daemonId":"A","type":"turn.started","at":recent_at("2026-11-17T10:00:04Z")}),
    )
    .unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "working"
    );
    e.record_observation(json!({"id":"A","transportState":"unknown","activityState":"working","observedAt":"2026-11-17T19:00:05+09:00"})).unwrap();
    e.record_event(
        json!({"id":"later2","daemonId":"A","type":"turn.started","at":recent_at("2026-11-17T10:00:06Z")}),
    )
    .unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "unknown"
    );
    assert_eq!(e.snapshot().unwrap()["events"].as_array().unwrap().len(), 3);
}

#[test]
fn review_f1_status_cannot_revive_archived_or_cancelled_tasks() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let task =
        json!({"id":"t","projectId":"p","title":"T","state":"reportedComplete","revision":"1"});
    for op in [
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":"."}}),
        json!({"entity":"task","action":"upsert","value":task}),
        json!({"entity":"evidence","action":"upsert","value":{"id":"ev","taskId":"t","kind":"test","success":true,"subjectRevision":"1","reference":"test"}}),
        json!({"entity":"status","action":"publish","value":{"projectId":"p","tasks":[task]}}),
    ] {
        e.mutate(op, None).unwrap();
    }
    e.mutate(json!({"entity":"task","action":"archive","id":"t"}), None)
        .unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s["projects"][0]["total"], 0);
    assert_eq!(s["projects"][0]["verified"], 0);
    e.mutate(
        json!({"entity":"task","action":"upsert","value":task}),
        None,
    )
    .unwrap();
    let mut cancelled = task.clone();
    cancelled["state"] = json!("cancelled");
    cancelled["revision"] = json!("2");
    e.mutate(
        json!({"entity":"status","action":"publish","value":{"projectId":"p","tasks":[cancelled]}}),
        None,
    )
    .unwrap();
    assert_eq!(e.snapshot().unwrap()["projects"][0]["total"], 0);
}
#[test]
fn review_f2_event_timestamps_are_valid_utc_and_ordered() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"unknown","observedAt":recent_at("2026-11-17T09:59:59Z")})).unwrap();
    let start=e.record_event(json!({"id":"start","daemonId":"A","method":"turn/started","at":recent_at("2026-11-17T19:00:00+09:00"),"params":{"threadId":"th","turnId":"t"}})).unwrap();
    assert_eq!(
        start["at"],
        chrono::DateTime::parse_from_rfc3339(&recent_at("2026-11-17T19:00:00+09:00"))
            .unwrap()
            .with_timezone(&chrono::Utc)
            .to_rfc3339()
    );
    e.record_event(json!({"id":"end","daemonId":"A","method":"turn/completed","at":recent_at("2026-11-17T10:00:01Z"),"params":{"threadId":"th","turnId":"t"}})).unwrap();
    assert_eq!(e.snapshot().unwrap()["daemons"][0]["activityState"], "idle");
    assert!(e
        .record_event(json!({"id":"bad","daemonId":"A","method":"turn/started","at":"invalid"}))
        .is_err());
}
#[cfg(windows)]
#[test]
fn review_f3_original_refusal_codes_are_preserved_and_classified() {
    let d = tempdir().unwrap();
    let script = d.path().join("fake.ps1");
    let bridge = d.path().join("bridge.ps1");
    std::fs::write(&script, "# fake").unwrap();
    for code in ["previous-shift", "not-in", "rejected"] {
        std::fs::write(
            &bridge,
            format!("param($ScriptPath,$Action)\nWrite-Output '{{\"code\":\"{code}\"}}'\n"),
        )
        .unwrap();
        let v = devdock_core::smartone::execute(&script, &bridge, "checkOut").unwrap();
        assert_eq!(v["state"], "refused");
        assert_eq!(v["code"], code);
    }
}
#[test]
fn review_f4_mismatched_or_duplicate_ids_are_rejected() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let task = json!({"id":"t","projectId":"p","title":"T","state":"pending","revision":"1"});
    e.mutate(
        json!({"entity":"task","action":"upsert","value":task}),
        None,
    )
    .unwrap();
    assert!(e
        .mutate(
            json!({"entity":"task","action":"upsert","id":"other","value":task}),
            None
        )
        .is_err());
    assert_eq!(e.registry().unwrap()["tasks"].as_array().unwrap().len(), 1);
    let path = d.path().join("registry.json");
    let mut reg = e.registry().unwrap();
    reg["tasks"].as_array_mut().unwrap().push(task.clone());
    std::fs::write(path, reg.to_string()).unwrap();
    assert!(e
        .mutate(
            json!({"entity":"task","action":"upsert","value":task}),
            None
        )
        .is_err());
}
#[test]
fn checkout_prompt_checks_running_and_uncertain_runs_in_atomic_transaction() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    for (i, state) in ["pending", "recorded", "alreadyOut", "uncertain"]
        .iter()
        .enumerate()
    {
        let date = format!("2026-11-{:02}", i + 1);
        let id = e.claim_run(&date, "checkOut").unwrap().unwrap();
        if *state != "pending" {
            e.finish_run(&id, json!({"state":state})).unwrap();
        }
        assert!(!e.claim_prompt(&date).unwrap());
    }
    let date = "2026-11-10";
    let conn = rusqlite::Connection::open(d.path().join("devdock.sqlite3")).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn.execute(
        "INSERT INTO runs(attempt_id,date,action,state) VALUES('manual',?1,'checkOut','pending')",
        [date],
    )
    .unwrap();
    let e2 = e.clone();
    let worker = std::thread::spawn(move || e2.claim_prompt(date).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));
    conn.execute_batch("COMMIT").unwrap();
    assert!(!worker.join().unwrap());
}

#[test]
fn legacy_mixed_offset_events_replay_by_absolute_time() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"unknown","observedAt":recent_at("2026-11-17T09:59:59Z")})).unwrap();
    let db = rusqlite::Connection::open(d.path().join("devdock.sqlite3")).unwrap();
    for (id, at, typ, state) in [
        ("s", "2026-11-17T19:00:00+09:00", "turn/started", "working"),
        ("e", "2026-11-17T10:00:01Z", "turn/completed", "idle"),
    ] {
        let v = json!({"id":id,"daemonId":"A","at":recent_at(at),"type":typ,"threadId":"th","turnId":"t","activityState":state});
        db.execute(
            "INSERT INTO events(id,daemon_id,at,data) VALUES(?1,'A',?2,?3)",
            rusqlite::params![id, at, v.to_string()],
        )
        .unwrap();
    }
    let s = e.snapshot().unwrap();
    assert_eq!(s["daemons"][0]["activityState"], "idle");
    assert_eq!(
        s["daemons"][0]["lastActivityAt"],
        recent_at("2026-11-17T10:00:01Z")
    );
    assert_eq!(s["events"][0]["id"], "e");
}
#[test]
fn manual_checkout_claim_and_prompt_can_race_without_losing_prior_claim() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a = e.clone();
    let b = barrier.clone();
    let done = completed.clone();
    let run = std::thread::spawn(move || {
        b.wait();
        let id = a.claim_run("2026-11-12", "checkOut").unwrap();
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        id
    });
    barrier.wait();
    let already = completed.load(std::sync::atomic::Ordering::SeqCst);
    let prompt = e.claim_prompt("2026-11-12").unwrap();
    assert!(run.join().unwrap().is_some());
    if already {
        assert!(!prompt)
    }
}

#[test]
fn event_rejects_non_string_timestamp() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    assert!(e
        .record_event(json!({"id":"x","daemonId":"A","method":"turn/started","at":42}))
        .is_err());
}

#[test]
fn checkout_prompt_is_suppressed_for_refused_and_failed_runs() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    for (date, state) in [("2026-11-13", "refused"), ("2026-11-14", "failed")] {
        let id = e.claim_run(date, "checkOut").unwrap().unwrap();
        e.finish_run(&id, json!({"state":state})).unwrap();
        assert!(!e.claim_prompt(date).unwrap());
    }
}

#[test]
fn checklist_partial_and_decision_items_are_visible_without_fenced_examples() {
    let d = tempdir().unwrap();
    let path = d.path().join("list.md");
    std::fs::write(&path,"- [x] DOC01. Done\n- [-] T01. In progress\n- [?] D03. Decide\n- [ ] AC01. Pending\n~~~md\n- [-] T99. Example\n- [?] D99. Example\n~~~\n").unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":".","checklistPath":path}}),None).unwrap();
    let s = e.snapshot().unwrap();
    let p = &s["projects"][0];
    assert_eq!(p["total"], 4);
    assert_eq!(p["reportedComplete"], 1);
    assert_eq!(p["inProgress"], 1);
    assert_eq!(p["blocked"], 1);
    assert_eq!(p["verified"], 0);
    assert!(p["currentTask"].as_str().unwrap().contains("T01"));
    let tasks = p["tasks"].as_array().unwrap();
    assert!(tasks
        .iter()
        .any(|t| t["id"] == "T01" && t["state"] == "inProgress"));
    assert!(tasks
        .iter()
        .any(|t| t["id"] == "D03" && t["state"] == "blocked"));
}

#[test]
fn retention_aggregates_real_samples_and_preserves_ledger() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    for (at, state, latency) in [
        ("2026-11-09T19:00:01+09:00", "healthy", json!(12)),
        ("2026-11-09T10:00:30Z", "disconnected", json!(null)),
        ("2026-11-09T10:00:40Z", "unknown", json!(null)),
        ("2026-11-10T12:00:00Z", "slow", json!(20)),
        ("2026-07-01T00:00:00Z", "healthy", json!(5)),
    ] {
        e.record_observation(
            json!({"id":"A","observedAt":at,"transportState":state,"latencyMs":latency}),
        )
        .unwrap();
    }
    e.record_event(
        json!({"id":"old","daemonId":"A","type":"turn.started","at":"2026-08-18T11:59:59Z"}),
    )
    .unwrap();
    e.record_event(
        json!({"id":"edge","daemonId":"A","type":"turn.started","at":"2026-08-19T12:00:00Z"}),
    )
    .unwrap();
    let run = e.claim_run("2026-01-01", "checkOut").unwrap().unwrap();
    e.claim_prompt("2026-01-02").unwrap();
    let db = rusqlite::Connection::open(d.path().join("devdock.sqlite3")).unwrap();
    db.execute("INSERT INTO progress(at,project_id,total,verified,reported) VALUES('2026-08-18T11:59:59Z','p',1,0,0)",[]).unwrap();
    db.execute("INSERT INTO progress(at,project_id,total,verified,reported) VALUES('2026-08-19T12:00:00Z','p',1,0,0)",[]).unwrap();
    let v = e.maintain_history("2026-11-17T12:00:00Z").unwrap();
    assert_eq!(v["rawDeleted"], 4);
    assert_eq!(v["eventsDeleted"], 1);
    assert_eq!(v["progressDeleted"], 1);
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    let x:(i64,i64,i64,i64,f64,i64)=db.query_row("SELECT sample_count,success_count,failure_count,unknown_count,latency_sum,latency_count FROM observation_minutes WHERE daemon_id='A' AND minute='2026-11-09T10:00:00Z'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(x, (3, 1, 1, 1, 12.0, 1));
    assert_eq!(
        e.maintain_history("2026-11-17T12:00:00Z").unwrap()["rawDeleted"],
        0
    );
    let n: i64 = db
        .query_row("SELECT sample_count FROM observation_minutes", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 3);
    assert!(e.finish_run(&run, json!({"state":"uncertain"})).is_ok());
    assert!(!e.claim_prompt("2026-01-02").unwrap());
    let indexes:i64=db.query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name IN ('idx_observations_source_time','idx_events_source_time','idx_progress_project_time')",[],|r|r.get(0)).unwrap();
    assert_eq!(indexes, 3);
    assert!(e.maintain_history("invalid").is_err());
}

#[test]
fn retained_minute_samples_are_queryable_and_gaps_are_not_filled() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let now = chrono::Utc::now();
    let old = now - chrono::Duration::days(8);
    e.record_observation(
        json!({"id":"A","observedAt":old.to_rfc3339(),"transportState":"healthy","latencyMs":12}),
    )
    .unwrap();
    e.maintain_history(&now.to_rfc3339()).unwrap();
    let h = e.history("A", 24 * 10).unwrap();
    let minutes = h["minutes"].as_array().unwrap();
    assert_eq!(minutes.len(), 1);
    assert_eq!(minutes[0]["sampleCount"], 1);
    assert_eq!(minutes[0]["latencySum"], 12.0);
}

#[test]
fn retention_rolls_back_aggregates_if_raw_delete_fails() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","observedAt":"2026-11-09T10:00:00Z","transportState":"healthy","latencyMs":1})).unwrap();
    let db = rusqlite::Connection::open(d.path().join("devdock.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON observations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(e.maintain_history("2026-11-17T12:00:00Z").is_err());
    let raw: i64 = db
        .query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    let agg: i64 = db
        .query_row("SELECT COUNT(*) FROM observation_minutes", [], |r| r.get(0))
        .unwrap();
    assert_eq!((raw, agg), (1, 0));
}

#[test]
fn final_f1_f6_monitors_and_tasks_merge_with_evidence() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    assert!(e.registry().unwrap()["monitors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "daemon-A" && m["source"] == "A"));
    std::fs::write(
        d.path().join("tasks.md"),
        "- [ ] P01. Parent\n  - [x] T01. Child\n",
    )
    .unwrap();
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":d.path()}}),
        None,
    )
    .unwrap();
    let bad = json!({"entity":"monitor","action":"upsert","value":{"id":"invalid","name":"bad","kind":"daemon","source":"C","enabled":true}});
    assert!(e.mutate(bad, None).is_err());
    e.mutate(json!({"entity":"monitor","action":"upsert","value":{"id":"m","name":"M","kind":"checklist","projectId":"p","source":"tasks.md","enabled":true}}),None).unwrap();
    e.mutate(json!({"entity":"task","action":"upsert","value":{"id":"T01","projectId":"p","title":"Structured","state":"reportedComplete","revision":"v1"}}),None).unwrap();
    e.mutate(json!({"entity":"evidence","action":"upsert","value":{"id":"ev","taskId":"T01","kind":"test","success":true,"subjectRevision":"v1","reference":"test"}}),None).unwrap();
    let snap = e.snapshot().unwrap();
    let p = &snap["projects"][0];
    assert_eq!(p["total"], 1);
    assert_eq!(p["verified"], 1);
    assert_eq!(p["reportedComplete"], 0);
    let t = p["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "T01")
        .unwrap();
    assert_eq!(t["verified"], true);
    assert_eq!(t["verificationEvidence"].as_array().unwrap().len(), 1);
    assert_eq!(snap["monitorStatuses"]["m"]["state"], "healthy");
    std::fs::remove_file(d.path().join("tasks.md")).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["monitorStatuses"]["m"]["state"],
        "unknown"
    );
}
#[test]
fn final_f2_attendance_reports_ledger_but_not_live_server() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let date = chrono::Local::now().date_naive().to_string();
    let id = e.claim_run(&date, "checkIn").unwrap().unwrap();
    e.finish_run(
        &id,
        json!({"state":"recorded","code":"recorded","date":date,"time":"08:15","message":"확인"}),
    )
    .unwrap();
    let s = e.snapshot().unwrap();
    let a = &s["attendance"];
    assert_eq!(a["serverState"], "unknown");
    assert_eq!(a["checkInTime"], "08:15");
    assert_eq!(a["lastCode"], "recorded");
    assert_eq!(
        a["lastRunsByAction"]["checkIn"]["result"]["state"],
        "recorded"
    );
}
#[test]
fn final_f3_old_observation_is_stale_with_last_known() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let old = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    e.record_observation(
        json!({"id":"A","transportState":"healthy","activityState":"working","observedAt":old}),
    )
    .unwrap();
    let a = &e.snapshot().unwrap()["daemons"][0];
    assert_eq!(a["freshness"], "stale");
    assert_eq!(a["transportState"], "unknown");
    assert_eq!(a["activityState"], "unknown");
    assert_eq!(a["lastKnown"]["activityState"], "working");
}
#[test]
fn final_f5_settings_revision_detects_stale_write() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let mut first = e.settings().unwrap();
    let mut second = first.clone();
    first["attendanceEnabled"] = json!(false);
    let saved = e.save_settings(first).unwrap();
    assert_eq!(saved["revision"], 1);
    second["retentionDays"] = json!(14);
    assert!(e.save_settings(second).is_err());
    assert_eq!(e.settings().unwrap()["attendanceEnabled"], false);
}

#[test]
fn final_f8_expired_prompt_cannot_resolve_new_day() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    assert!(e.claim_prompt("2026-11-16").unwrap());
    assert!(e.claim_prompt("2026-11-17").unwrap());
    let old = e
        .resolve_prompt_at("2026-11-16", true, "2026-11-17")
        .unwrap();
    assert_eq!(
        old,
        json!({"date":"2026-11-16","answer":false,"expired":true})
    );
    let current = e
        .resolve_prompt_at("2026-11-17", true, "2026-11-17")
        .unwrap();
    assert_eq!(
        current,
        json!({"date":"2026-11-17","answer":true,"expired":false})
    );
}
#[test]
fn final_f5_apply_failure_and_file_failure_roll_back() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let mut settings = e.settings().unwrap();
    settings["autostartEnabled"] = json!(true);
    assert!(e
        .save_settings_with(settings.clone(), |_, _| Err("fixture".into()))
        .is_err());
    assert_eq!(e.settings().unwrap()["autostartEnabled"], false);
    let path = d.path().join("settings.json");
    let mut calls = Vec::new();
    let result = e.save_settings_with(settings, |old, new| {
        calls.push((
            old["autostartEnabled"].clone(),
            new["autostartEnabled"].clone(),
        ));
        if calls.len() == 1 {
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(
        calls,
        vec![(json!(false), json!(true)), (json!(true), json!(false))]
    );
}
#[test]
fn final_f1_structured_status_failure_is_visible_and_project_required() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    assert!(e.mutate(json!({"entity":"monitor","action":"upsert","value":{"id":"bad","name":"bad","kind":"structuredStatus","source":"status.json","enabled":true}}),None).is_err());
    e.mutate(
        json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":d.path()}}),
        None,
    )
    .unwrap();
    e.mutate(json!({"entity":"monitor","action":"upsert","value":{"id":"m","name":"M","kind":"structuredStatus","projectId":"p","source":"missing.json","enabled":true}}),None).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s["monitorStatuses"]["m"]["state"], "unknown");
    assert!(!s["monitorStatuses"]["m"]["reason"]
        .as_str()
        .unwrap()
        .is_empty());
}
#[test]
fn final_f6_markdown_evidence_and_duplicate_paths() {
    let d = tempdir().unwrap();
    let path = d.path().join("tasks.md");
    std::fs::write(&path, "- [x] T01. Done\n").unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":d.path(),"checklistPath":"tasks.md"}}),None).unwrap();
    e.mutate(json!({"entity":"monitor","action":"upsert","value":{"id":"m","name":"M","kind":"checklist","projectId":"p","source":path,"enabled":true}}),None).unwrap();
    let revision = e.snapshot().unwrap()["projects"][0]["tasks"][0]["revision"].clone();
    e.mutate(json!({"entity":"evidence","action":"upsert","value":{"id":"ev","taskId":"T01","kind":"test","success":true,"subjectRevision":revision,"reference":"test"}}),None).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s["projects"][0]["total"], 1);
    assert_eq!(s["projects"][0]["verified"], 1);
    assert_eq!(s["projects"][0]["reportedComplete"], 0);
}

#[test]
fn final_f1_disabled_daemon_masks_latest_working_observation() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.record_observation(json!({"id":"A","transportState":"healthy","activityState":"working","observedAt":chrono::Utc::now().to_rfc3339()})).unwrap();
    assert_eq!(
        e.snapshot().unwrap()["daemons"][0]["activityState"],
        "working"
    );
    e.mutate(json!({"entity":"monitor","action":"upsert","value":{"id":"daemon-A","name":"A","kind":"daemon","source":"A","enabled":false}}),None).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s["daemons"][0]["activityState"], "unknown");
    assert_eq!(s["daemons"][0]["lastKnown"]["activityState"], "working");
    assert_eq!(s["monitorStatuses"]["daemon-A"]["state"], "disabled");
}

#[test]
fn final_f2_different_record_date_does_not_show_today_time() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    let today = chrono::Local::now().date_naive().to_string();
    let id = e.claim_run(&today, "checkOut").unwrap().unwrap();
    e.finish_run(
        &id,
        json!({"state":"recorded","code":"recorded","date":"2020-01-01","time":"17:33"}),
    )
    .unwrap();
    let a = e.snapshot().unwrap()["attendance"].clone();
    assert!(a["checkOutTime"].is_null());
    assert_eq!(
        a["lastRunsByAction"]["checkOut"]["recordDate"],
        "2020-01-01"
    );
}

#[test]
fn final_f1_missing_checklist_is_explicit_and_existing_registry_is_not_reseeded() {
    let d = tempdir().unwrap();
    Engine::open(d.path().into()).unwrap();
    let old = json!({"schemaVersion":1,"revision":7,"projects":[],"monitors":[],"tasks":[],"evidence":[]});
    std::fs::write(d.path().join("registry.json"), old.to_string()).unwrap();
    let e = Engine::connect(d.path().into()).unwrap();
    assert!(e.registry().unwrap()["monitors"]
        .as_array()
        .unwrap()
        .is_empty());
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":d.path(),"checklistPath":"missing.md"}}),None).unwrap();
    let s = e.snapshot().unwrap();
    assert!(!s["projects"][0]["sourceErrors"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn final_f6_markdown_evidence_expires_after_source_changes() {
    let d = tempdir().unwrap();
    let path = d.path().join("tasks.md");
    std::fs::write(&path, "- [x] T01. Done\n").unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"p","name":"P","root":d.path(),"checklistPath":"tasks.md"}}),None).unwrap();
    let before = e.snapshot().unwrap();
    let rev = before["projects"][0]["tasks"][0]["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    e.mutate(json!({"entity":"evidence","action":"upsert","value":{"id":"ev","taskId":"T01","kind":"test","success":true,"subjectRevision":rev,"reference":"test"}}),None).unwrap();
    assert_eq!(e.snapshot().unwrap()["projects"][0]["verified"], 1);
    std::fs::write(&path, "- [x] T01. Done and changed\n").unwrap();
    let p = e.snapshot().unwrap()["projects"][0].clone();
    assert_eq!(p["verified"], 0);
    assert_eq!(p["reportedComplete"], 1);
    assert_eq!(p["tasks"][0]["id"], "T01");
}

#[test]
fn blank_optional_checklist_does_not_create_source_error() {
    let d = tempdir().unwrap();
    let e = Engine::open(d.path().into()).unwrap();
    for (id, path) in [("empty", ""), ("spaces", "   ")] {
        e.mutate(json!({"entity":"project","action":"upsert","value":{"id":id,"name":id,"root":d.path(),"checklistPath":path}}),None).unwrap();
    }
    e.mutate(json!({"entity":"project","action":"upsert","value":{"id":"missing","name":"missing","root":d.path(),"checklistPath":"missing.md"}}),None).unwrap();
    let s = e.snapshot().unwrap();
    let projects = s["projects"].as_array().unwrap();
    for id in ["empty", "spaces"] {
        let p = projects.iter().find(|p| p["id"] == id).unwrap();
        assert!(p["sourceErrors"].as_array().unwrap().is_empty());
        assert_eq!(p["total"], 0);
    }
    assert!(
        !projects.iter().find(|p| p["id"] == "missing").unwrap()["sourceErrors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// 읽기 전용 런타임 상태 해석과 실패 경계를 검증한다.
use devdock_core::daemon::{activity_from_status, observation_from_error};
use serde_json::json;
#[test]
fn active_flags_take_precedence_over_working() {
    assert_eq!(
        activity_from_status(&json!({"type":"active","activeFlags":["waitingOnApproval"]})),
        "approval"
    );
    assert_eq!(
        activity_from_status(&json!({"type":"active","activeFlags":["waitingOnUserInput"]})),
        "input"
    );
    assert_eq!(
        activity_from_status(&json!({"type":"active","activeFlags":[]})),
        "working"
    );
}
#[test]
fn not_loaded_and_access_denied_are_unknown() {
    assert_eq!(
        activity_from_status(&json!({"type":"notLoaded"})),
        "unknown"
    );
    let o = observation_from_error("A", "access denied");
    assert_eq!(o["transportState"], "unknown");
    assert_eq!(o["activityState"], "unknown");
}
#[test]
fn verification_mode_never_permits_side_effects() {
    assert!(devdock_core::runtime::verification_mode(true, false));
    assert!(devdock_core::runtime::verification_mode(false, true));
    assert!(!devdock_core::runtime::verification_mode(false, false));
}
#[test]
fn data_directory_uses_override_then_local_app_data() {
    use std::{ffi::OsString, path::PathBuf};
    let base = tempfile::Builder::new()
        .prefix("devdock-native-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let explicit = base.path().join("fixture");
    let local = base.path().join("local");
    assert_eq!(
        devdock_core::runtime::data_directory(
            Some(explicit.clone().into_os_string()),
            Some(local.clone().into_os_string())
        )
        .unwrap(),
        explicit
    );
    assert_eq!(
        devdock_core::runtime::data_directory(None, Some(local.clone().into_os_string())).unwrap(),
        local.join("DevDock")
    );
    assert!(devdock_core::runtime::data_directory(None::<OsString>, None).is_err());
    let _ = PathBuf::new();
}
#[test]
fn concurrent_poll_gate_releases_when_guard_drops() {
    let gate = devdock_core::runtime::PollGate::default();
    let guard = gate.try_acquire().expect("first poll");
    assert!(gate.try_acquire().is_none());
    drop(guard);
    assert!(gate.try_acquire().is_some());
}
#[test]
fn checkout_prompt_skips_existing_result_for_same_day() {
    use devdock_core::runtime::checkout_prompt_allowed;
    for state in ["recorded", "alreadyOut", "uncertain", "pending"] {
        let v = json!({"attendance":{"runs":[{"date":"2026-11-17","action":"checkOut","state":state}]}});
        assert!(!checkout_prompt_allowed(&v, "2026-11-17"));
        assert!(checkout_prompt_allowed(&v, "2026-11-18"));
    }
    assert!(checkout_prompt_allowed(
        &json!({"attendance":{"runs":[]}}),
        "2026-11-17"
    ));
}
#[test]
fn data_directory_reads_isolated_environment() {
    if std::env::var_os("DEVDOCK_TEST_DATA_CHILD").is_some() {
        let expected = std::path::PathBuf::from(std::env::var_os("DEVDOCK_TEST_EXPECTED").unwrap());
        assert_eq!(
            devdock_core::runtime::data_directory_from_env().unwrap(),
            expected
        );
        return;
    }
    let tmp = tempfile::Builder::new()
        .prefix("devdock-native-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let exe = std::env::current_exe().unwrap();
    for override_enabled in [false, true] {
        let local = tmp.path().join("local");
        let explicit = tmp.path().join("explicit");
        let expected = if override_enabled {
            explicit.clone()
        } else {
            local.join("DevDock")
        };
        let mut child = std::process::Command::new(&exe);
        child
            .args(["--exact", "data_directory_reads_isolated_environment"])
            .env("DEVDOCK_TEST_DATA_CHILD", "1")
            .env("DEVDOCK_TEST_EXPECTED", expected)
            .env("LOCALAPPDATA", local);
        if override_enabled {
            child.env("DEVDOCK_DATA_DIR", explicit);
        } else {
            child.env_remove("DEVDOCK_DATA_DIR");
        }
        assert!(child.status().unwrap().success());
    }
}
#[test]
fn maintenance_runs_immediately_then_only_after_each_wait() {
    use std::cell::Cell;
    let completed = Cell::new(0);
    let waits = Cell::new(0);
    devdock_core::runtime::maintenance_worker(
        || completed.set(completed.get() + 1),
        || {
            assert_eq!(completed.get(), waits.get() + 1);
            waits.set(waits.get() + 1);
            waits.get() < 2
        },
    );
    assert_eq!(completed.get(), 2);
}
#[test]
fn daemon_sources_respect_active_registry_and_deduplicate() {
    let registry = json!({"monitors":[
        {"kind":"daemon","source":"A","enabled":true},
        {"kind":"daemon","source":"A","enabled":true},
        {"kind":"daemon","source":"B","enabled":false},
        {"kind":"daemon","source":"B","enabled":true,"archived":true},
        {"kind":"daemon","source":"other","enabled":true},
        {"kind":"checklist","source":"B","enabled":true}
    ]});
    assert_eq!(
        devdock_core::runtime::daemon_sources(&registry).unwrap(),
        vec!["A"]
    );
    assert!(devdock_core::runtime::daemon_sources(&json!({})).is_err());
}
#[test]
fn checkout_response_never_executes_expired_or_rejected_prompt() {
    use devdock_core::runtime::checkout_response;
    let calls = std::cell::Cell::new(0);
    let mut action = || {
        calls.set(calls.get() + 1);
        Ok(json!({}))
    };
    let expired = checkout_response(
        "2026-11-16",
        true,
        "2026-11-17",
        |date, answer, now| Ok(json!({"date":date,"answer":false,"expired":date!=now && answer})),
        &mut action,
    )
    .unwrap();
    assert_eq!(expired["expired"], true);
    assert_eq!(calls.get(), 0);
    assert!(checkout_response(
        "invalid",
        true,
        "2026-11-17",
        |_, _, _| Ok(json!({})),
        &mut action
    )
    .is_err());
    checkout_response(
        "2026-11-17",
        false,
        "2026-11-17",
        |date, _, _| Ok(json!({"date":date,"answer":false,"expired":false})),
        &mut action,
    )
    .unwrap();
    assert_eq!(calls.get(), 0);
    checkout_response(
        "2026-11-17",
        true,
        "2026-11-17",
        |date, _, _| Ok(json!({"date":date,"answer":true,"expired":false})),
        &mut action,
    )
    .unwrap();
    assert_eq!(calls.get(), 1);
}
#[test]
fn safe_settings_allow_other_edits_but_reject_autostart() {
    use devdock_core::runtime::apply_autostart_change;
    let calls = std::cell::Cell::new(0);
    let mut apply = |_: bool| {
        calls.set(calls.get() + 1);
        Ok(())
    };
    assert!(apply_autostart_change(
        true,
        &json!({"autostartEnabled":false}),
        &json!({"autostartEnabled":false}),
        &mut apply
    )
    .is_ok());
    assert!(apply_autostart_change(
        true,
        &json!({"autostartEnabled":false}),
        &json!({"autostartEnabled":true}),
        &mut apply
    )
    .is_err());
    assert_eq!(calls.get(), 0);
    assert!(apply_autostart_change(
        false,
        &json!({"autostartEnabled":false}),
        &json!({"autostartEnabled":true}),
        &mut apply
    )
    .is_ok());
    assert_eq!(calls.get(), 1);
}
#[test]
fn disabled_or_invalid_registry_never_calls_daemon_observer() {
    let calls = std::sync::Mutex::new(Vec::new());
    let registry = json!({"monitors":[{"kind":"daemon","source":"A","enabled":true},{"kind":"daemon","source":"A","enabled":true},{"kind":"daemon","source":"B","enabled":false}]});
    let values = devdock_core::runtime::observe_daemons(Ok(registry), |id| {
        calls.lock().unwrap().push(id.to_string());
        json!({"id":id,"transportState":"healthy"})
    });
    assert_eq!(*calls.lock().unwrap(), vec!["A"]);
    assert_eq!(values[1]["transportState"], "unknown");
    let bad = devdock_core::runtime::observe_daemons(Err("invalid".into()), |_| {
        panic!("must not observe on registry error")
    });
    assert!(bad.iter().all(|v| v["transportState"] == "unknown"));
}
#[test]
fn stale_settings_do_not_touch_startup_and_safe_other_changes_save() {
    let dir = tempfile::tempdir().unwrap();
    let engine = devdock_core::service::Engine::open(dir.path().to_path_buf()).unwrap();
    let old = engine.settings().unwrap();
    let mut newer = old.clone();
    newer["retentionDays"] = json!(14);
    engine.save_settings(newer).unwrap();
    let mut stale = old;
    stale["autostartEnabled"] = json!(true);
    let calls = std::cell::Cell::new(0);
    let error = devdock_core::runtime::save_settings_with_startup(&engine, stale, false, |_| {
        calls.set(calls.get() + 1);
        Ok(())
    });
    assert!(error.is_err());
    assert_eq!(calls.get(), 0);
    let mut fresh = engine.settings().unwrap();
    fresh["retentionDays"] = json!(21);
    assert!(
        devdock_core::runtime::save_settings_with_startup(&engine, fresh, true, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .is_ok()
    );
    assert_eq!(calls.get(), 0);
}
#[test]
fn popup_claim_waits_for_frontend_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let engine = devdock_core::service::Engine::open(dir.path().to_path_buf()).unwrap();
    assert!(
        !devdock_core::runtime::claim_ready_prompt(&engine, false, false, "2026-11-17").unwrap()
    );
    assert!(!devdock_core::runtime::claim_ready_prompt(&engine, true, true, "2026-11-17").unwrap());
    assert!(devdock_core::runtime::claim_ready_prompt(&engine, false, true, "2026-11-17").unwrap());
    assert!(
        !devdock_core::runtime::claim_ready_prompt(&engine, false, true, "2026-11-17").unwrap()
    );
}
#[test]
fn previous_day_popup_expires_only_its_own_ledger_entry() {
    let dir = tempfile::tempdir().unwrap();
    let engine = devdock_core::service::Engine::open(dir.path().to_path_buf()).unwrap();
    assert!(engine.claim_prompt("2026-11-16").unwrap());
    assert!(engine.claim_prompt("2026-11-17").unwrap());
    let calls = std::cell::Cell::new(0);
    let result = devdock_core::runtime::checkout_response(
        "2026-11-16",
        true,
        "2026-11-17",
        |date, answer, now| engine.resolve_prompt_at(date, answer, now),
        &mut || {
            calls.set(calls.get() + 1);
            Ok(json!({}))
        },
    )
    .unwrap();
    assert_eq!(result["expired"], true);
    assert_eq!(calls.get(), 0);
    let today = engine
        .resolve_prompt_at("2026-11-17", false, "2026-11-17")
        .unwrap();
    assert_eq!(today["expired"], false);
    assert_eq!(today["date"], "2026-11-17");
}
#[test]
fn recorded_checkout_answer_returns_action_failure_separately() {
    let dir = tempfile::tempdir().unwrap();
    let engine = devdock_core::service::Engine::open(dir.path().to_path_buf()).unwrap();
    assert!(engine.claim_prompt("2026-11-17").unwrap());
    let result = devdock_core::runtime::checkout_response(
        "2026-11-17",
        true,
        "2026-11-17",
        |date, answer, now| engine.resolve_prompt_at(date, answer, now),
        &mut || Err("스마트원 스크립트가 설정되지 않았습니다.".into()),
    )
    .unwrap();
    assert_eq!(result["date"], "2026-11-17");
    assert_eq!(result["answer"], true);
    assert_eq!(result["expired"], false);
    assert_eq!(
        result["attendanceError"],
        "스마트원 스크립트가 설정되지 않았습니다."
    );
    let calls = std::cell::Cell::new(0);
    let retry = devdock_core::runtime::checkout_response(
        "2026-11-17",
        false,
        "2026-11-17",
        |date, answer, now| engine.resolve_prompt_at(date, answer, now),
        &mut || {
            calls.set(calls.get() + 1);
            Ok(json!({}))
        },
    );
    assert!(retry.is_err());
    assert_eq!(calls.get(), 0);
}

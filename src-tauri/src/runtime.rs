// 안전 모드와 데스크톱 명령의 실행 경계를 관리한다.
pub fn verification_mode(debug: bool, safe: bool) -> bool {
    debug || safe
}
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
pub fn data_directory(
    override_dir: Option<OsString>,
    local_app_data: Option<OsString>,
) -> Result<PathBuf, String> {
    override_dir
        .map(PathBuf::from)
        .or_else(|| local_app_data.map(|p| PathBuf::from(p).join("DevDock")))
        .ok_or_else(|| "data directory unavailable".into())
}
pub fn data_directory_from_env() -> Result<PathBuf, String> {
    data_directory(
        std::env::var_os("DEVDOCK_DATA_DIR"),
        std::env::var_os("LOCALAPPDATA"),
    )
}
#[derive(Default)]
pub struct PollGate(AtomicBool);
pub struct PollGuard<'a>(&'a AtomicBool);
impl PollGate {
    pub fn try_acquire(&self) -> Option<PollGuard<'_>> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| PollGuard(&self.0))
    }
}
impl Drop for PollGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
pub fn checkout_prompt_allowed(snapshot: &serde_json::Value, date: &str) -> bool {
    snapshot["attendance"]["runs"]
        .as_array()
        .is_some_and(|runs| {
            !runs
                .iter()
                .any(|run| run["date"] == date && run["action"] == "checkOut")
        })
}
pub fn claim_ready_prompt(
    engine: &crate::service::Engine,
    safe: bool,
    ready: bool,
    date: &str,
) -> Result<bool, String> {
    if safe || !ready {
        return Ok(false);
    }
    engine.claim_prompt(date)
}
pub fn maintenance_worker(mut run: impl FnMut(), mut wait: impl FnMut() -> bool) {
    loop {
        run();
        if !wait() {
            break;
        }
    }
}
pub fn daemon_sources(registry: &serde_json::Value) -> Result<Vec<&'static str>, String> {
    let monitors = registry["monitors"]
        .as_array()
        .ok_or("invalid monitor registry")?;
    Ok(["A", "B"]
        .into_iter()
        .filter(|id| {
            monitors.iter().any(|m| {
                m["kind"] == "daemon"
                    && m["enabled"] == true
                    && m["archived"] != true
                    && m["source"] == *id
            })
        })
        .collect())
}
pub fn observe_daemons(
    registry: Result<serde_json::Value, String>,
    observe: impl Fn(&str) -> serde_json::Value + Sync,
) -> Vec<serde_json::Value> {
    let selected = registry.and_then(|r| daemon_sources(&r));
    std::thread::scope(|scope| {
        let jobs: Vec<_> = ["A", "B"]
            .into_iter()
            .map(|id| {
                if selected.as_ref().is_ok_and(|ids| ids.contains(&id)) {
                    let observe = &observe;
                    Some(scope.spawn(move || observe(id)))
                } else {
                    None
                }
            })
            .collect();
        jobs.into_iter()
            .zip(["A", "B"])
            .map(|(job, id)| match job {
                Some(job) => job.join().unwrap_or_else(|_| {
                    crate::daemon::observation_from_error(id, "관측을 확인할 수 없습니다.")
                }),
                None => crate::daemon::observation_from_error(
                    id,
                    if selected.is_err() {
                        "데몬 등록 정보를 확인할 수 없습니다."
                    } else {
                        "활성 데몬 모니터가 없습니다."
                    },
                ),
            })
            .collect()
    })
}
pub fn apply_autostart_change(
    safe: bool,
    old: &serde_json::Value,
    new: &serde_json::Value,
    apply: &mut impl FnMut(bool) -> Result<(), String>,
) -> Result<(), String> {
    let old = old["autostartEnabled"]
        .as_bool()
        .ok_or("invalid previous autostart")?;
    let new = new["autostartEnabled"]
        .as_bool()
        .ok_or("invalid autostart")?;
    if old != new {
        if safe {
            return Err("검증 모드에서는 자동 시작을 변경하지 않습니다.".into());
        }
        apply(new)?;
    }
    Ok(())
}
pub fn save_settings_with_startup(
    engine: &crate::service::Engine,
    settings: serde_json::Value,
    safe: bool,
    mut apply: impl FnMut(bool) -> Result<(), String>,
) -> Result<serde_json::Value, String> {
    engine.save_settings_with(settings, |old, new| {
        apply_autostart_change(safe, old, new, &mut apply)
    })
}
pub fn checkout_response(
    date: &str,
    answer: bool,
    now: &str,
    resolve: impl FnOnce(&str, bool, &str) -> Result<serde_json::Value, String>,
    action: &mut impl FnMut() -> Result<serde_json::Value, String>,
) -> Result<serde_json::Value, String> {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "invalid prompt date")?;
    chrono::NaiveDate::parse_from_str(now, "%Y-%m-%d").map_err(|_| "invalid current date")?;
    let mut result = resolve(date, answer, now)?;
    if result["date"] != date || !result["expired"].is_boolean() || !result["answer"].is_boolean() {
        return Err("invalid prompt response".into());
    }
    if date == now && answer && result["answer"] == true && result["expired"] == false {
        if let Err(error) = action() {
            result["attendanceError"] = serde_json::Value::String(error);
        }
    }
    Ok(result)
}
#[cfg(feature = "desktop")]
pub mod desktop {
    use crate::{daemon, service::Engine, smartone};
    use chrono::Local;
    use serde_json::{json, Value};
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tauri::{AppHandle, Emitter, Manager, State};
    #[derive(Clone)]
    pub struct Runtime {
        pub engine: Engine,
        pub safe: bool,
        pub running: Arc<AtomicBool>,
        pub bridge: PathBuf,
        pub poll_gate: Arc<super::PollGate>,
        pub ui_ready: Arc<AtomicBool>,
    }
    impl Runtime {
        pub fn attendance(&self, action: &str) -> Result<Value, String> {
            self.attendance_for_date(action, None)
        }
        fn attendance_for_date(
            &self,
            action: &str,
            expected_date: Option<&str>,
        ) -> Result<Value, String> {
            if self.safe {
                return Err("검증 모드에서는 근태를 실행하지 않습니다.".into());
            }
            if !["checkIn", "checkOut"].contains(&action) {
                return Err("invalid action".into());
            }
            let settings = self.engine.settings()?;
            let script = settings["smartoneScript"].as_str().unwrap_or("");
            if script.is_empty() || !PathBuf::from(script).is_file() || !self.bridge.is_file() {
                return Err("스마트원 스크립트가 설정되지 않았습니다.".into());
            }
            if self
                .running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return Err("근태 처리 중입니다.".into());
            }
            let result = (|| {
                let date = Local::now().date_naive().to_string();
                if expected_date.is_some_and(|expected| expected != date) {
                    return Err("퇴근 알림 날짜가 만료되었습니다.".into());
                }
                let id = self
                    .engine
                    .claim_run(&date, action)?
                    .ok_or("오늘 이미 처리했거나 확인이 필요합니다.")?;
                if expected_date
                    .is_some_and(|expected| expected != Local::now().date_naive().to_string())
                {
                    return self.engine.finish_run(
                        &id,
                        json!({"state":"refused","message":"퇴근 알림 날짜가 만료되었습니다."}),
                    );
                }
                let result = smartone::execute(
                    PathBuf::from(script).as_path(),
                    &self.bridge,
                    action,
                )
                .unwrap_or_else(
                    |_| json!({"state":"uncertain","message":"실행 결과를 확인할 수 없습니다."}),
                );
                self.engine.finish_run(&id, result)
            })();
            self.running.store(false, Ordering::SeqCst);
            result
        }
    }
    fn changed(app: &AppHandle) {
        let _ = app.emit("dashboard-updated", ());
    }
    async fn blocking<F>(f: F) -> Result<Value, String>
    where
        F: FnOnce() -> Result<Value, String> + Send + 'static,
    {
        tauri::async_runtime::spawn_blocking(f)
            .await
            .map_err(|e| e.to_string())?
    }
    #[tauri::command]
    pub async fn knowledge_command(
        app: AppHandle,
        s: State<'_, Runtime>,
        operation: Value,
    ) -> Result<Value, String> {
        let r = s.inner().clone();
        let read_only = crate::knowledge::commands::read_only(&operation);
        let value = blocking(move || {
            r.engine
                .knowledge()
                .execute_command(operation, false, r.safe)
        })
        .await?;
        if !read_only {
            changed(&app);
        }
        Ok(value)
    }
    #[tauri::command]
    pub async fn get_snapshot(s: State<'_, Runtime>) -> Result<Value, String> {
        let r = s.inner().clone();
        blocking(move || {
            let mut v = r.engine.snapshot()?;
            v["verificationMode"] = json!(r.safe);
            Ok(v)
        })
        .await
    }
    #[tauri::command]
    pub async fn get_registry(s: State<'_, Runtime>) -> Result<Value, String> {
        let e = s.engine.clone();
        blocking(move || e.registry()).await
    }
    #[tauri::command]
    pub async fn mutate_registry(
        app: AppHandle,
        s: State<'_, Runtime>,
        operation: Value,
        expected_revision: Option<u64>,
    ) -> Result<Value, String> {
        let e = s.engine.clone();
        let v = blocking(move || e.mutate(operation, expected_revision)).await?;
        changed(&app);
        Ok(v)
    }
    #[tauri::command]
    pub async fn save_settings(
        app: AppHandle,
        s: State<'_, Runtime>,
        settings: Value,
    ) -> Result<Value, String> {
        let r = s.inner().clone();
        let v = blocking(move || {
            super::save_settings_with_startup(&r.engine, settings, r.safe, set_startup_entry)
        })
        .await?;
        changed(&app);
        Ok(v)
    }
    #[tauri::command]
    pub async fn get_history(
        s: State<'_, Runtime>,
        monitor_id: String,
        hours: u32,
    ) -> Result<Value, String> {
        let e = s.engine.clone();
        blocking(move || e.history(&monitor_id, hours)).await
    }
    #[tauri::command]
    pub async fn run_attendance(
        app: AppHandle,
        s: State<'_, Runtime>,
        action: String,
    ) -> Result<Value, String> {
        let r = s.inner().clone();
        let v = blocking(move || r.attendance(&action)).await?;
        changed(&app);
        Ok(v)
    }
    #[tauri::command]
    pub async fn answer_checkout(
        app: AppHandle,
        s: State<'_, Runtime>,
        answer: bool,
        prompt_date: String,
    ) -> Result<Value, String> {
        let r = s.inner().clone();
        let v = blocking(move || {
            if r.safe {
                return Err("검증 모드에서는 퇴근 처리를 실행하지 않습니다.".into());
            }
            let today = Local::now().date_naive().to_string();
            super::checkout_response(
                &prompt_date,
                answer,
                &today,
                |date, answer, current| r.engine.resolve_prompt_at(date, answer, current),
                &mut || r.attendance_for_date("checkOut", Some(&prompt_date)),
            )
        })
        .await?;
        changed(&app);
        Ok(v)
    }
    #[cfg(windows)]
    fn set_startup_entry(enabled: bool) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
            System::Registry::{
                RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER,
                KEY_SET_VALUE, REG_SZ,
            },
        };
        let path: Vec<u16> = r"Software\Microsoft\Windows\CurrentVersion\Run"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let name: Vec<u16> = "DevDock".encode_utf16().chain(Some(0)).collect();
        let mut key = std::ptr::null_mut();
        let open =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key) };
        if open == ERROR_FILE_NOT_FOUND && !enabled {
            return Ok(());
        }
        if open != ERROR_SUCCESS {
            return Err(format!("자동 시작 열기 실패 ({open})"));
        }
        let result = if enabled {
            let exe = match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => {
                    unsafe {
                        RegCloseKey(key);
                    };
                    return Err(e.to_string());
                }
            };
            let command: Vec<u16> = format!("\"{}\" --background", exe.display())
                .encode_utf16()
                .chain(Some(0))
                .collect();
            unsafe {
                RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        } else {
            unsafe { RegDeleteValueW(key, name.as_ptr()) }
        };
        unsafe {
            RegCloseKey(key);
        }
        if result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(format!("자동 시작 설정 실패 ({result})"))
        }
    }
    #[cfg(not(windows))]
    fn set_startup_entry(_enabled: bool) -> Result<(), String> {
        Err("자동 시작은 Windows에서만 지원합니다.".into())
    }
    #[tauri::command]
    pub async fn set_autostart(
        app: AppHandle,
        s: State<'_, Runtime>,
        enabled: bool,
    ) -> Result<Value, String> {
        let r = s.inner().clone();
        let v = blocking(move || {
            let mut settings = r.engine.settings()?;
            settings["autostartEnabled"] = json!(enabled);
            super::save_settings_with_startup(&r.engine, settings, r.safe, set_startup_entry)
        })
        .await?;
        changed(&app);
        Ok(v)
    }
    #[tauri::command]
    pub fn frontend_ready(s: State<'_, Runtime>) {
        s.ui_ready.store(true, Ordering::Release);
    }
    #[tauri::command]
    pub async fn refresh_now(app: AppHandle, s: State<'_, Runtime>) -> Result<Value, String> {
        let r = s.inner().clone();
        let v = blocking(move || {
            poll(&r);
            r.engine.snapshot()
        })
        .await?;
        changed(&app);
        Ok(v)
    }
    fn poll(r: &Runtime) {
        let Some(_guard) = r.poll_gate.try_acquire() else {
            return;
        };
        let cli = daemon::locate_cli();
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        let observations = super::observe_daemons(r.engine.registry(), |id| match (&cli, &home) {
            (Some(c), Some(h)) => daemon::observe(
                id,
                c,
                &h.join(if id == "A" { ".codex" } else { ".codex-B" }),
            ),
            _ => daemon::observation_from_error(id, "Codex CLI 또는 사용자 홈을 찾지 못했습니다."),
        });
        let current = r.engine.registry().and_then(|v| super::daemon_sources(&v));
        for mut observation in observations {
            let id = observation["id"].as_str().unwrap_or("").to_owned();
            if !current.as_ref().is_ok_and(|ids| ids.contains(&id.as_str())) {
                observation = daemon::observation_from_error(
                    &id,
                    if current.is_err() {
                        "데몬 등록 정보를 확인할 수 없습니다."
                    } else {
                        "활성 데몬 모니터가 없습니다."
                    },
                );
            }
            let _ = r.engine.record_observation(observation);
        }
    }
    pub async fn refresh_background(app: AppHandle, r: Runtime) -> Result<(), String> {
        blocking(move || {
            poll(&r);
            Ok(json!({}))
        })
        .await?;
        changed(&app);
        Ok(())
    }
    pub fn background(app: AppHandle, r: Runtime) {
        let maintenance_engine = r.engine.clone();
        std::thread::spawn(move || {
            super::maintenance_worker(
                || {
                    let _ = maintenance_engine.maintain_history(&Local::now().to_rfc3339());
                },
                || {
                    std::thread::sleep(Duration::from_secs(3600));
                    true
                },
            )
        });
        std::thread::spawn(move || loop {
            let tick = std::time::Instant::now();
            poll(&r);
            if !r.safe {
                if let Ok(s) = r.engine.evaluate_schedule(&Local::now().to_rfc3339()) {
                    let date = s["date"].as_str().unwrap_or("");
                    if s["checkIn"] == true {
                        let worker = r.clone();
                        std::thread::spawn(move || {
                            let _ = worker.attendance("checkIn");
                        });
                    }
                    if s["checkoutPrompt"] == true
                        && r.engine
                            .snapshot()
                            .is_ok_and(|snapshot| super::checkout_prompt_allowed(&snapshot, date))
                        && super::claim_ready_prompt(
                            &r.engine,
                            r.safe,
                            r.ui_ready.load(Ordering::Acquire),
                            date,
                        )
                        .unwrap_or(false)
                    {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                        let _ = app.emit("checkout-prompt", json!({"date":date}));
                    }
                }
            }
            if let Ok(snapshot) = r.engine.snapshot() {
                if let Some(tray) = app.tray_by_id("devdock") {
                    let state = |i: usize| {
                        snapshot["daemons"][i]["activityState"]
                            .as_str()
                            .unwrap_or("unknown")
                    };
                    let _ = tray.set_tooltip(Some(format!(
                        "DevDock · A: {} · B: {}",
                        state(0),
                        state(1)
                    )));
                }
            }
            changed(&app);
            std::thread::sleep(Duration::from_secs(5).saturating_sub(tick.elapsed()));
        });
    }
}

// DevDock의 단일 인스턴스와 데스크톱 수명을 시작한다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod tray;
use devdock_core::{
    runtime::{
        data_directory_from_env,
        desktop::{self, Runtime},
        verification_mode, PollGate,
    },
    service::Engine,
};
use std::sync::{atomic::AtomicBool, Arc};
use tauri::{Manager, WindowEvent};
#[cfg(windows)]
fn claim_instance() -> Option<usize> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attributes: *const std::ffi::c_void,
            owner: i32,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn CreateEventW(
            attributes: *const std::ffi::c_void,
            manual: i32,
            initial: i32,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
        fn SetEvent(event: *mut std::ffi::c_void) -> i32;
    }
    let mutex: Vec<u16> = "Local\\DevDock.SingleInstance"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let signal: Vec<u16> = "Local\\DevDock.RestoreWindow"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, mutex.as_ptr());
        if handle.is_null() {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let event = OpenEventW(0x0002, 0, signal.as_ptr());
            if !event.is_null() {
                SetEvent(event);
            }
            return None;
        }
        let event = CreateEventW(std::ptr::null(), 0, 0, signal.as_ptr());
        if event.is_null() {
            return None;
        }
        Some(event as usize)
    }
}
#[cfg(not(windows))]
fn claim_instance() -> Option<usize> {
    Some(0)
}
#[cfg(windows)]
fn listen_restore(app: tauri::AppHandle, event: usize) {
    use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    std::thread::spawn(move || loop {
        if unsafe { WaitForSingleObject(event as *mut std::ffi::c_void, INFINITE) } != 0 {
            break;
        }
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
}
#[cfg(not(windows))]
fn listen_restore(_app: tauri::AppHandle, _event: usize) {}
fn main() {
    let worker_args:Vec<_>=std::env::args_os().skip(1).collect();
    if let [flag,path]=worker_args.as_slice(){if flag=="--report-source-worker"{match devdock_core::knowledge::report_worker::run_report_source_worker(std::path::Path::new(path)){Ok(v)=>println!("{}",v),Err(_)=>std::process::exit(1)}return;}}
    if let Some(path) = std::env::args_os()
        .skip_while(|a| a != "--document-scan-worker")
        .nth(1)
    {
        if devdock_core::knowledge::scanner::run_scan_worker(std::path::Path::new(&path)).is_err() {
            std::process::exit(1);
        }
        return;
    }
    let Some(restore_event) = claim_instance() else {
        return;
    };
    let data = data_directory_from_env().expect("DevDock 데이터 경로를 찾을 수 없습니다.");
    let engine = Engine::open(data).expect("DevDock 데이터 초기화 실패");
    let safe = verification_mode(
        cfg!(debug_assertions),
        std::env::args().any(|a| a == "--safe"),
    );
    let background = std::env::args().any(|a| a == "--background");
    tauri::Builder::default()
        .setup(move |app| {
            let bridge = app
                .path()
                .resource_dir()?
                .join("scripts/smartone-bridge.ps1");
            let runtime = Runtime {
                engine: engine.clone(),
                safe,
                running: Arc::new(AtomicBool::new(false)),
                bridge,
                poll_gate: Arc::new(PollGate::default()),
                ui_ready: Arc::new(AtomicBool::new(false)),
            };
            #[cfg(windows)]
            if let Ok(host) = devdock_core::knowledge::report_ipc::start(engine.knowledge()) {
                app.manage(host);
            }
            app.manage(runtime.clone());
            engine.knowledge().start_document_scheduler();
            tray::setup(app.handle())?;
            listen_restore(app.handle().clone(), restore_event);
            desktop::background(app.handle().clone(), runtime);
            if !background {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Resized(_) => {
                if window.is_minimized().unwrap_or(false) {
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            desktop::frontend_ready,
            desktop::knowledge_command,
            desktop::get_snapshot,
            desktop::get_registry,
            desktop::mutate_registry,
            desktop::save_settings,
            desktop::get_history,
            desktop::run_attendance,
            desktop::answer_checkout,
            desktop::set_autostart,
            desktop::refresh_now
        ])
        .run(tauri::generate_context!())
        .expect("DevDock 실행 실패");
}

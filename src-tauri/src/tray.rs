// 트레이 메뉴와 창 표시 및 종료 동작을 연결한다.
use devdock_core::runtime::desktop::Runtime;
use std::sync::atomic::Ordering;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};
fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
fn restore_on_event(event: &TrayIconEvent) -> bool {
    matches!(event, TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    })
}
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "열기", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "새로고침", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "설정", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &refresh, &settings, &exit])?;
    let mut builder = TrayIconBuilder::with_id("devdock")
        .tooltip("DevDock · A/B 상태 확인 중")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if restore_on_event(&event) {
                show(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show(app),
            "settings" => {
                show(app);
                use tauri::Emitter;
                let _ = app.emit("open-settings", ());
            }
            "refresh" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let r = app.state::<Runtime>().inner().clone();
                    let handle = app.clone();
                    let _ = devdock_core::runtime::desktop::refresh_background(handle, r).await;
                });
            }
            "exit" => {
                let r = app.state::<Runtime>();
                if !r.running.load(Ordering::SeqCst) {
                    app.exit(0);
                }
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};

    fn click(button: MouseButton, button_state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: "devdock".into(),
            position: tauri::PhysicalPosition::new(1.0, 2.0),
            rect: tauri::Rect {
                position: tauri::PhysicalPosition::new(0, 0).into(),
                size: tauri::PhysicalSize::new(16, 16).into(),
            },
            button,
            button_state,
        }
    }

    #[test]
    fn left_release_restores_once() {
        assert!(restore_on_event(&click(MouseButton::Left, MouseButtonState::Up)));
        assert!(!restore_on_event(&click(MouseButton::Left, MouseButtonState::Down)));
    }

    #[test]
    fn right_and_down_do_not_restore() {
        assert!(!restore_on_event(&click(MouseButton::Right, MouseButtonState::Up)));
        assert!(!restore_on_event(&click(MouseButton::Middle, MouseButtonState::Up)));
        assert!(!restore_on_event(&TrayIconEvent::DoubleClick {
            id: "devdock".into(),
            position: tauri::PhysicalPosition::new(1.0, 2.0),
            rect: tauri::Rect {
                position: tauri::PhysicalPosition::new(0, 0).into(),
                size: tauri::PhysicalSize::new(16, 16).into(),
            },
            button: MouseButton::Left,
        }));
    }
}

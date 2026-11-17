use crate::state::Core;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;

pub const AUTOSTART_ARG: &str = "--autostart";
const HINT_MARKER: &str = "tray-hint-shown";

static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn launched_by_autostart<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .skip(1)
        .any(|arg| arg.as_ref() == AUTOSTART_ARG)
}

pub fn keep_running_on_close(start_on_login: bool, flow_active: bool, remap_running: bool) -> bool {
    !QUITTING.load(Ordering::Acquire) && (start_on_login || flow_active || remap_running)
}

pub fn status_text(flow_active: bool, remap_running: bool) -> String {
    format!(
        "Flow {} · Remapping {}",
        if flow_active { "on" } else { "off" },
        if remap_running { "on" } else { "off" }
    )
}

fn live_state(core: &Core) -> (bool, bool, bool) {
    let start_on_login = core
        .data
        .lock()
        .map(|d| d.settings.start_on_login)
        .unwrap_or(false);
    let flow_active = !crate::flow::status(core).peers.is_empty();
    let remap_running = crate::remap::status(core).running;
    (start_on_login, flow_active, remap_running)
}

pub fn should_hide_on_close(core: &Core) -> bool {
    let (start_on_login, flow_active, remap_running) = live_state(core);
    keep_running_on_close(start_on_login, flow_active, remap_running)
}

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn autostart_enabled<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

pub fn set_autostart<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<bool, String> {
    let launcher = app.autolaunch();
    let current = launcher.is_enabled().unwrap_or(false);
    if current != enabled {
        let result = if enabled {
            launcher.enable()
        } else {
            launcher.disable()
        };
        result.map_err(|e| format!("Cannot change start on login: {e}"))?;
    }
    launcher
        .is_enabled()
        .map_err(|e| format!("Cannot read start on login: {e}"))
}

pub fn notify_hidden_once<R: Runtime>(app: &AppHandle<R>, core: &Core) {
    let marker = core.directory.join(HINT_MARKER);
    if marker.exists() {
        return;
    }
    let shown = app
        .notification()
        .builder()
        .title("TrapLink is still running")
        .body("It stays in the system tray to keep switching and remapping active. Choose Quit in the tray menu to exit.")
        .show();
    if shown.is_ok() {
        let _ = std::fs::write(marker, b"1");
    }
}

pub fn quit<R: Runtime>(app: &AppHandle<R>) {
    QUITTING.store(true, Ordering::Release);
    if let Some(core) = app.try_state::<Arc<Core>>() {
        crate::release_on_exit(&core);
    }
    app.exit(0);
}

pub fn install_tray<R: Runtime>(app: &AppHandle<R>, core: Arc<Core>) -> tauri::Result<()> {
    let (_, flow_active, remap_running) = live_state(&core);
    let open = MenuItem::with_id(app, "open", "Open TrapLink", true, None::<&str>)?;
    let status = MenuItem::with_id(
        app,
        "status",
        status_text(flow_active, remap_running),
        false,
        None::<&str>,
    )?;
    let stop = MenuItem::with_id(app, "stop-input", "Stop input sharing", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &status,
            &stop,
            &PredefinedMenuItem::separator(app)?,
            &exit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("TrapLink")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "stop-input" => {
                if let Some(core) = app.try_state::<Arc<Core>>() {
                    crate::emergency_stop_core(&core);
                }
            }
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    tauri::async_runtime::spawn(async move {
        let mut last = String::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let (_, flow_active, remap_running) = live_state(&core);
            let text = status_text(flow_active, remap_running);
            if text != last {
                if status.set_text(&text).is_err() {
                    break;
                }
                last = text;
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_flag_is_exact_and_ignores_the_executable_path() {
        assert!(launched_by_autostart(["traplink.exe", "--autostart"]));
        assert!(launched_by_autostart(["traplink", "--foo", "--autostart"]));
        assert!(!launched_by_autostart(["traplink.exe"]));
        assert!(!launched_by_autostart(["--autostart"]));
        assert!(!launched_by_autostart(["traplink", "--autostart=1"]));
        assert!(!launched_by_autostart(["traplink", "--AUTOSTART"]));
    }

    #[test]
    fn closing_quits_unless_something_must_keep_running() {
        assert!(!keep_running_on_close(false, false, false));
        assert!(keep_running_on_close(true, false, false));
        assert!(keep_running_on_close(false, true, false));
        assert!(keep_running_on_close(false, false, true));
    }

    #[test]
    fn tray_status_reports_both_features() {
        assert_eq!(
            status_text(true, false),
            "Flow on · Remapping off"
        );
        assert_eq!(
            status_text(false, true),
            "Flow off · Remapping on"
        );
    }
}

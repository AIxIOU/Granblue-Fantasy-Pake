use crate::app::window::{
    hide_all_app_windows, open_additional_window_safe, show_all_app_windows, toggle_all_app_windows,
};
use crate::cancel_startup_reveal;
use std::str::FromStr;
use std::sync::{atomic::AtomicBool, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

pub fn set_system_tray(
    app: &AppHandle,
    show_system_tray: bool,
    tray_icon_path: &str,
    _init_fullscreen: bool,
    allow_multi_window: bool,
    startup_revealed: Arc<AtomicBool>,
) -> tauri::Result<()> {
    if !show_system_tray {
        app.remove_tray_by_id("pake-tray");
        return Ok(());
    }

    // Menu events are broadcast to every handler in Tauri v2, so the tray item
    // must not share the "new_window" id with the app menu accelerator
    // (Cmd/Ctrl+N), or one click opens two windows.
    let new_window = MenuItemBuilder::with_id("tray_new_window", "New Window").build(app)?;
    let hide_app = MenuItemBuilder::with_id("hide_app", "Hide").build(app)?;
    let show_app = MenuItemBuilder::with_id("show_app", "Show").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = if allow_multi_window {
        MenuBuilder::new(app)
            .items(&[&new_window, &hide_app, &show_app, &quit])
            .build()?
    } else {
        MenuBuilder::new(app)
            .items(&[&hide_app, &show_app, &quit])
            .build()?
    };

    app.app_handle().remove_tray_by_id("pake-tray");

    let menu_revealed = startup_revealed.clone();
    let click_revealed = startup_revealed;
    let mut tray_builder = TrayIconBuilder::with_id("pake-tray")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "tray_new_window" => {
                open_additional_window_safe(app);
            }
            "hide_app" => {
                // Hide every webview (main + multi-window clones), not only "pake".
                cancel_startup_reveal(&menu_revealed);
                hide_all_app_windows(app);
            }
            "show_app" => {
                cancel_startup_reveal(&menu_revealed);
                show_all_app_windows(app, _init_fullscreen);
            }
            "quit" => {
                crate::app::window::persist_window_geometry(app);
                crate::app::sidebar::persist_layout_state(app);
                let flags = if _init_fullscreen {
                    StateFlags::all()
                } else {
                    StateFlags::all() & !StateFlags::FULLSCREEN
                };
                let _ = app.save_window_state(flags);
                crate::app::window::persist_window_geometry(app);
                app.exit(0);
            }
            _ => (),
        })
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                // Windows emits Click twice per physical click (Down then Up).
                // Reacting to both runs the toggle twice, so a hidden window is
                // shown and immediately re-hidden and the tray looks dead (#1343).
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    // Any tray toggle claims visibility control from startup reveal.
                    cancel_startup_reveal(&click_revealed);
                    toggle_all_app_windows(tray.app_handle(), _init_fullscreen);
                }
            }
        });

    let resolved_icon = if tray_icon_path.is_empty() {
        app.default_window_icon().cloned()
    } else {
        tauri::image::Image::from_path(tray_icon_path)
            .ok()
            .or_else(|| app.default_window_icon().cloned())
    };

    if let Some(icon) = resolved_icon {
        tray_builder = tray_builder.icon(icon);
    } else {
        eprintln!("[Pake] No tray icon available; tray will build without an icon.");
    }

    let tray = tray_builder.build(app)?;

    tray.set_icon_as_template(false)?;
    Ok(())
}

/// Values `gbf_set_tray` needs to rebuild the tray after startup.
#[derive(Clone)]
pub struct TrayRuntime {
    pub icon_path: String,
    pub init_fullscreen: bool,
    pub multi_window: bool,
    pub startup_revealed: Arc<AtomicBool>,
}

/// Show or hide the system tray. Off by default; closing the window then quits.
#[tauri::command]
pub fn gbf_set_tray(app: AppHandle, on: bool) -> Result<String, String> {
    app.state::<crate::app::sidebar::SidebarState>()
        .set_tray_enabled(on);
    crate::app::sidebar::persist_layout_state(&app);

    let (icon_path, init_fullscreen, multi_window, startup_revealed) = {
        let rt = app.state::<TrayRuntime>();
        (
            rt.icon_path.clone(),
            rt.init_fullscreen,
            rt.multi_window,
            rt.startup_revealed.clone(),
        )
    };

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let report = match set_system_tray(
            &handle,
            on,
            &icon_path,
            init_fullscreen,
            multi_window,
            startup_revealed,
        ) {
            Ok(()) => {
                if let Some(host) = handle.get_window("pake") {
                    let _ = crate::app::sidebar::layout(&host);
                }
                format!(
                    "tray={on} tray_icon={}",
                    handle.tray_by_id("pake-tray").is_some()
                )
            }
            Err(error) => format!("tray ERR {error}"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    if report.starts_with("tray ERR") {
        Err(report)
    } else {
        Ok(report)
    }
}

pub fn set_global_shortcut(
    app: &AppHandle,
    shortcut: String,
    _init_fullscreen: bool,
    startup_revealed: Arc<AtomicBool>,
) -> tauri::Result<()> {
    if shortcut.is_empty() {
        return Ok(());
    }

    let app_handle = app.clone();
    let shortcut_hotkey = match Shortcut::from_str(&shortcut) {
        Ok(s) => s,
        Err(error) => {
            eprintln!("[Pake] Invalid activation shortcut '{shortcut}': {error}");
            return Ok(());
        }
    };
    let last_triggered = Arc::new(Mutex::new(Instant::now()));

    if let Err(error) = app_handle.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler({
                let last_triggered = Arc::clone(&last_triggered);
                let startup_revealed = startup_revealed.clone();
                move |app, event, _shortcut| {
                    let Ok(mut last_triggered) = last_triggered.lock() else {
                        return;
                    };
                    if Instant::now().duration_since(*last_triggered) < Duration::from_millis(300) {
                        return;
                    }
                    *last_triggered = Instant::now();

                    if shortcut_hotkey.eq(event) {
                        cancel_startup_reveal(&startup_revealed);
                        toggle_all_app_windows(app, _init_fullscreen);
                    }
                }
            })
            .build(),
    ) {
        eprintln!(
            "[Pake] Failed to register global shortcut plugin '{shortcut}': {error}; continuing without it."
        );
        return Ok(());
    }

    if let Err(error) = app.global_shortcut().register(shortcut_hotkey) {
        eprintln!("[Pake] Failed to bind global shortcut '{shortcut}': {error}");
    }

    Ok(())
}

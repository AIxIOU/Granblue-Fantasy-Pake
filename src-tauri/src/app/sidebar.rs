//! EXPERIMENT ONLY -- native sidebar as a sibling webview.
//!
//! Not on `main`, not in any release.
//! See EXPERIMENT-native-sidebar/GBF_Pake_EXPERIMENT_NATIVE_SIDEBAR.md.
//!
//! # What this is testing
//!
//! On `main`, the sidebar is DOM inside Granblue's own document, drawn by
//! `gbf-scaler.js`. That costs us three standing problems:
//!
//!   1. it is destroyed and rebuilt on every game navigation (the reload flash)
//!   2. it has to track the game's right edge every frame to place itself
//!   3. it lives on the far side of Pake's page zoom, so CSS pixels and
//!      physical pixels disagree and window fitting drifts
//!
//! Here the sidebar is a SECOND WEBVIEW in the same window, beside the game
//! rather than inside it. The game webview is narrowed to `window - sidebar`,
//! so Granblue simply sees a smaller viewport -- exactly what it would see in a
//! narrower browser window. All three problems above stop existing rather than
//! being worked around.
//!
//! # Rule 0
//!
//! Stronger here than on main, not weaker. The game webview gets a viewport and
//! nothing else: no injected chrome, no edge measurement, no DOM writes. The one
//! thing we send into it is `location.hash = "..."` on a nav click, which is
//! what clicking Granblue's own menu does.
//!
//! # WINDOW LOOKUP: use `get_window`, never `get_webview_window`
//!
//! Measured 2026-09-05, and it cost the first three iterations of this module.
//!
//! `WebviewWindow` is Tauri's convenience type for a window holding EXACTLY ONE
//! webview. The moment `add_child` puts a second webview in our window, that
//! window stops being one: `get_webview_window("pake")` returns `None` and
//! `webview_windows()` no longer lists it.
//!
//! It failed in the worst possible way -- silently. The toggle command reported
//! success and the state really did flip, but the relayout was skipped, so
//! nothing moved and it read like a broken IPC call. Everything here works on
//! `Window` and its `webviews()` list instead.
//!
//! **This breaks Pake's own code too**, which is a real cost of the approach
//! rather than a detail of this module: `hide_all_app_windows`,
//! `show_all_app_windows` and `any_app_window_visible` in `window.rs` all
//! enumerate `app.webview_windows()`, which is now empty. Tray Hide/Show and the
//! activation shortcut are therefore expected to be dead in this build. They
//! would need porting to `app.windows()` before any of this could ship.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    webview::WebviewBuilder, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl,
    WebviewWindow, Window, WindowEvent,
};

/// Expanded width, matching `SIDEBAR_W` in gbf-scaler.js so the two builds are
/// visually comparable.
pub const SIDEBAR_W: f64 = 250.0;
/// Collapsed rail width, matching `SIDEBAR_W_COLLAPSED` in gbf-scaler.js.
pub const SIDEBAR_W_COLLAPSED: f64 = 52.0;
/// Granblue's narrowest layout (320 x zoom 1). Never squeeze the game below it.
const MIN_GAME_WIDTH: f64 = 320.0;

/// One sidebar per window, so `--multi-window` keeps working: each window gets
/// its own game webview and its own sidebar beside it.
pub fn sidebar_label(window_label: &str) -> String {
    format!("{window_label}--gbf-sidebar")
}

#[derive(Default)]
pub struct SidebarState {
    collapsed: AtomicBool,
}

impl SidebarState {
    pub fn is_collapsed(&self) -> bool {
        self.collapsed.load(Ordering::Relaxed)
    }

    /// Flip and return the NEW value.
    pub fn toggle(&self) -> bool {
        !self.collapsed.fetch_xor(true, Ordering::Relaxed)
    }
}

/// The game webview is the one whose label matches the window's own label --
/// `WebviewWindowBuilder` gives them the same name.
fn game_webview(host: &Window) -> Option<Webview> {
    let label = host.label().to_string();
    host.webviews().into_iter().find(|w| w.label() == label)
}

fn sidebar_webview(host: &Window) -> Option<Webview> {
    let label = sidebar_label(host.label());
    host.webviews().into_iter().find(|w| w.label() == label)
}

/// Widths for the current window size, in logical pixels: (game, sidebar, height).
fn split(host: &Window, collapsed: bool) -> tauri::Result<(f64, f64, f64)> {
    let scale = host.scale_factor()?;
    let size = host.inner_size()?.to_logical::<f64>(scale);
    let want = if collapsed {
        SIDEBAR_W_COLLAPSED
    } else {
        SIDEBAR_W
    };
    // The game always wins a fight for space. If the window is too narrow to
    // hold both, the sidebar gives up width rather than crushing the game --
    // the reverse of the in-page version, where the sidebar could not be
    // narrower than its own content.
    let sidebar_w = want.min((size.width - MIN_GAME_WIDTH).max(0.0));
    let game_w = (size.width - sidebar_w).max(1.0);
    Ok((game_w, sidebar_w, size.height))
}

/// Place both webviews side by side across the window's client area.
///
/// Everything is in LOGICAL pixels. This is the whole point of moving the
/// sidebar out of the page: logical pixels are the window's own coordinate
/// space, so there is no CSS-pixel/physical-pixel conversion to get wrong and
/// no page zoom folded into `devicePixelRatio` to correct for. The drift bug
/// that Patch 2 in GBF_Pake_UPSTREAM_PATCHES.md exists to fix cannot occur here.
pub fn layout(host: &Window) -> tauri::Result<()> {
    let collapsed = host.app_handle().state::<SidebarState>().is_collapsed();
    let (game_w, sidebar_w, height) = split(host, collapsed)?;
    if height <= 0.0 {
        return Ok(()); // minimized; the next Resized event carries real numbers
    }

    if let Some(game) = game_webview(host) {
        game.set_position(LogicalPosition::new(0.0, 0.0))?;
        game.set_size(LogicalSize::new(game_w, height))?;
    }
    if let Some(bar) = sidebar_webview(host) {
        bar.set_position(LogicalPosition::new(game_w, 0.0))?;
        bar.set_size(LogicalSize::new(sidebar_w, height))?;
        // Rust owns the collapsed flag; the page only renders it. One source of
        // truth, so the two can never disagree.
        let _ = bar.eval(format!(
            "window.__gbfSidebar && window.__gbfSidebar.setCollapsed({collapsed})"
        ));
    }
    Ok(())
}

/// `layout()`, narrating every call it makes.
///
/// Kept permanently rather than deleted after the bug it found. Nothing here is
/// observable from outside -- the sidebar webview is not exposed as a CDP
/// target, and a skipped relayout prints nothing and returns Ok -- so this is
/// the only way to see what actually happened.
fn layout_verbose(host: &Window) -> String {
    let collapsed = host.app_handle().state::<SidebarState>().is_collapsed();
    let (game_w, sidebar_w, height) = match split(host, collapsed) {
        Ok(v) => v,
        Err(e) => return format!("split ERR {e}"),
    };

    let mut out = format!("want game={game_w:.0} bar={sidebar_w:.0} h={height:.0}\n");

    match game_webview(host) {
        None => out.push_str("game webview NOT FOUND\n"),
        Some(game) => {
            let p = game.set_position(LogicalPosition::new(0.0, 0.0));
            let z = game.set_size(LogicalSize::new(game_w, height));
            out.push_str(&format!(
                "game pos={} size={} now={}\n",
                result_word(&p),
                result_word(&z),
                bounds_word(&game),
            ));
        }
    }

    match sidebar_webview(host) {
        None => out.push_str("sidebar webview NOT FOUND\n"),
        Some(bar) => {
            let p = bar.set_position(LogicalPosition::new(game_w, 0.0));
            let z = bar.set_size(LogicalSize::new(sidebar_w, height));
            let e = bar.eval(format!(
                "window.__gbfSidebar && window.__gbfSidebar.setCollapsed({collapsed})"
            ));
            out.push_str(&format!(
                "bar pos={} size={} eval={} now={}\n",
                result_word(&p),
                result_word(&z),
                result_word(&e),
                bounds_word(&bar),
            ));
        }
    }

    out
}

fn result_word<T>(r: &tauri::Result<T>) -> String {
    match r {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("ERR({e})"),
    }
}

fn bounds_word(w: &Webview) -> String {
    match (w.position(), w.size()) {
        (Ok(p), Ok(s)) => format!("{},{} {}x{}", p.x, p.y, s.width, s.height),
        _ => "unreadable".to_string(),
    }
}

/// Add the sidebar webview beside the game and keep it laid out.
pub fn attach(window: &WebviewWindow) -> tauri::Result<()> {
    let label = sidebar_label(window.label());
    let host = window.as_ref().window();

    let (game_w, sidebar_w, height) = split(&host, false)?;

    // Served from the bundled assets as tauri://localhost/gbf-sidebar.html, so
    // it is trusted local content with a working IPC bridge. It is deliberately
    // NOT part of Granblue's origin -- that separation is the feature.
    //
    // No auto_resize(): this webview is positioned by layout() alone. Letting
    // Tauri grow it with the window would put it back on top of the game.
    let builder = WebviewBuilder::new(&label, WebviewUrl::App("gbf-sidebar.html".into()));

    host.add_child(
        builder,
        LogicalPosition::new(game_w, 0.0),
        LogicalSize::new(sidebar_w, height),
    )?;

    layout(&host)?;

    // The game webview no longer follows the window on its own, so every resize
    // has to re-run the split.
    let on_resize = host.clone();
    host.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Err(error) = layout(&on_resize) {
                eprintln!("[Pake][gbf] sidebar relayout failed: {error}");
            }
        }
    });

    Ok(())
}

/// Route a nav click to the game webview.
///
/// `location.hash = ...` is deliberate, and `navigate()` would be wrong:
/// Granblue is a hash-routed single-page app, so assigning the hash is exactly
/// what clicking its own menu does and it does NOT reload. A real navigation
/// would tear the game down and reload it on every sidebar click.
#[tauri::command]
pub fn gbf_nav(window: Window, hash: String) -> Result<(), String> {
    let game = game_webview(&window)
        .ok_or_else(|| format!("no game webview labelled '{}'", window.label()))?;
    let encoded = serde_json::to_string(&hash).map_err(|e| e.to_string())?;
    game.eval(format!("location.hash = {encoded}"))
        .map_err(|e| e.to_string())
}

/// Collapse or expand. Returns a narrated report of the relayout.
///
/// The relayout is marshalled onto the main thread: Tauri runs command handlers
/// on the async runtime, and moving a webview from off the UI thread is ignored
/// on Windows without erroring.
#[tauri::command]
pub fn gbf_toggle_sidebar(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let collapsed = app.state::<SidebarState>().toggle();

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let label = window.label().to_string();

    app.run_on_main_thread(move || {
        // get_window, NOT get_webview_window -- see the module docs.
        let report = match handle.get_window(&label) {
            Some(host) => layout_verbose(&host),
            None => format!("get_window({label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    // A timeout here means the main thread never ran the closure, which is a
    // completely different bug from the closure running and being ignored.
    let report = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));

    Ok(format!("collapsed={collapsed}\n{report}"))
}

/// Read-only snapshot of everything `layout()` bases its decisions on.
#[tauri::command]
pub fn gbf_debug(window: Window) -> Result<String, String> {
    let bounds: Vec<String> = window
        .webviews()
        .into_iter()
        .map(|w| format!("{} @{}", w.label(), bounds_word(&w)))
        .collect();

    let scale = window.scale_factor().unwrap_or(-1.0);
    let phys = window.inner_size().map_err(|e| e.to_string())?;
    let logical = phys.to_logical::<f64>(if scale > 0.0 { scale } else { 1.0 });
    let collapsed = window.app_handle().state::<SidebarState>().is_collapsed();

    Ok(format!(
        "window={}\nsidebar={}\nwebviews:\n  {}\nscale={scale:.3}\nphysical={}x{}\nlogical={:.0}x{:.0}\ncollapsed={collapsed}",
        window.label(),
        sidebar_label(window.label()),
        bounds.join("\n  "),
        phys.width,
        phys.height,
        logical.width,
        logical.height,
    ))
}

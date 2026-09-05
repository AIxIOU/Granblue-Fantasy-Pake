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
//! **This also broke Pake's own code**, which is a real cost of the approach
//! rather than a detail of this module: `hide_all_app_windows`,
//! `show_all_app_windows` and `any_app_window_visible` used to enumerate
//! `app.webview_windows()`. They, plus the matching `get_webview_window`
//! lookups, were ported to `app.windows()` / `get_window` in this experiment
//! so tray Hide/Show and the activation shortcut can work. That port is an
//! upstream-file patch and must be documented if it ever ships.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    webview::WebviewBuilder, AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalSize, Url,
    Webview, WebviewUrl, WebviewWindow, Window, WindowEvent,
};

/// Expanded width, matching `SIDEBAR_W` in gbf-scaler.js so the two builds are
/// visually comparable.
pub const SIDEBAR_W: f64 = 250.0;
/// Collapsed rail width, matching `SIDEBAR_W_COLLAPSED` in gbf-scaler.js.
pub const SIDEBAR_W_COLLAPSED: f64 = 52.0;
/// Granblue's narrowest layout (320 x zoom 1). Never squeeze the game below it.
const MIN_GAME_WIDTH: f64 = 320.0;
/// Preferred reading width for the wiki panel. 960 left Extra Drop Raids
/// cramped against the article edge; 1200 gives that widget a full row.
const WIKI_W: f64 = 1200.0;
/// Fallback wiki width: still no overlap or horizontal scroll on gbf.wiki.
const WIKI_MIN_W: f64 = 800.0;
/// Accept a preferred tier a few pixels short rather than dropping to 800.
const WIKI_TIER_SLACK: f64 = 48.0;
/// Extra inner width when growing so rounding cannot miss a whole tier.
const WIKI_WIDEN_BUFFER: f64 = 2.0;
/// About is a page of text, so it asks for less than the wiki and can open
/// in windows the wiki refuses.
const ABOUT_W: f64 = 470.0;
const ABOUT_MIN_W: f64 = 300.0;
/// The wiki is loaded directly, as a real page in a real webview.
const WIKI_URL: &str = "https://gbf.wiki/";
/// Id of the one style element locked mode adds to Granblue's document.
const LOCK_STYLE_ID: &str = "gbf-native-locked";
/// Granblue's own chat column. Hiding it is RULE 0 EXCEPTION 2.
const LOCK_SELECTOR: &str = "#submenu,#general-chat{display:none !important;}";

/// One sidebar per window, so `--multi-window` keeps working: each window gets
/// its own game webview and its own sidebar beside it.
pub fn sidebar_label(window_label: &str) -> String {
    format!("{window_label}--gbf-sidebar")
}

/// The wiki gets its own webview too, one per window.
pub fn wiki_label(window_label: &str) -> String {
    format!("{window_label}--gbf-wiki")
}

pub fn about_label(window_label: &str) -> String {
    format!("{window_label}--gbf-about")
}

pub fn options_label(window_label: &str) -> String {
    format!("{window_label}--gbf-options")
}

/// Separate from `.window-state.json` so extra keys cannot break the plugin's
/// restore parser. Per-window, same as SidebarState.
const LAYOUT_STATE_FILE: &str = "gbf-layout.json";

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct SavedLayout {
    locked: bool,
    /// Wiki sits to the right of the sidebar. False = between game and sidebar.
    #[serde(default)]
    wiki_outside: bool,
    /// System tray icon. Process-wide; stored on each window entry.
    /// Default false: no tray, and closing the window quits.
    #[serde(default)]
    tray: bool,
}

fn layout_state_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(LAYOUT_STATE_FILE))
}

pub fn persisted_layout_state_path(app: &AppHandle) -> Option<PathBuf> {
    layout_state_path(app)
}

fn load_layout_states(app: &AppHandle) -> HashMap<String, SavedLayout> {
    layout_state_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write lock/unlock (and later layout flags) so the next launch matches.
pub fn persist_layout_state(app: &AppHandle) {
    let Some(path) = layout_state_path(app) else {
        return;
    };
    let mut states = load_layout_states(app);
    let sidebar = app.state::<SidebarState>();
    for label in app.windows().keys() {
        states.insert(
            label.clone(),
            SavedLayout {
                locked: sidebar.is_locked(label),
                wiki_outside: sidebar.is_wiki_outside(label),
                tray: sidebar.is_tray_enabled(),
            },
        );
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&states) {
        if let Err(error) = std::fs::write(&path, bytes) {
            eprintln!(
                "[Pake][gbf] failed to save layout state to {}: {error}",
                path.display()
            );
        }
    }
}

fn restore_layout_locked(app: &AppHandle, label: &str) -> bool {
    load_layout_states(app)
        .get(label)
        .map(|s| s.locked)
        .unwrap_or(false)
}

fn restore_layout_wiki_outside(app: &AppHandle, label: &str) -> bool {
    load_layout_states(app)
        .get(label)
        .map(|s| s.wiki_outside)
        .unwrap_or(false)
}

/// Tray is process-wide. Prefer the main window's saved flag; else any entry.
pub fn restore_layout_tray(app: &AppHandle) -> bool {
    let states = load_layout_states(app);
    states
        .get("pake")
        .map(|s| s.tray)
        .or_else(|| states.values().next().map(|s| s.tray))
        .unwrap_or(false)
}

#[derive(Default, Clone, Copy)]
struct WindowFlags {
    collapsed: bool,
    wiki_open: bool,
    about_open: bool,
    options_open: bool,
    /// Wiki/About/Options sit to the right of the sidebar (live-client order).
    /// False = between the game and the sidebar.
    wiki_outside: bool,
    locked: bool,
    lock_was_auto: bool,
    /// True when we collapsed the sidebar ourselves to free width for a panel.
    /// Closing the last panel restores it; a manual toggle takes ownership.
    collapsed_for_panel: bool,
    /// #wrapper's right edge in the game webview, CSS pixels. 0 = unknown.
    game_edge: f64,
    /// Visible submenu overlay right edge (collapsed rail or expanded chat).
    /// CSS pixels. 0 = hidden / unknown. Used when unlocked.
    game_overlay: f64,
    /// devicePixelRatio of the game webview. CSS px × this / window scale
    /// is window logical px. 0 = not yet reported.
    game_dpr: f64,
    /// Inner width to restore when unlocking, after a lock hug. 0 = none.
    hug_saved_w: f64,
    /// True between our set_size and the Resized layout that follows it.
    hug_busy: bool,
    /// GBF Automatic Resizing (`mobage_fixwindowsize === 0`).
    automatic: bool,
    /// Generation for delayed Automatic hugs so a drag does not snap mid-pull.
    hug_gen: u64,
    /// Layout may hug an Automatic window (set after the settle timer).
    auto_hug_due: bool,
    /// Automatic: one hug per user resize. Stops the walk-down after a snap.
    auto_hug_allowed: bool,
    /// Physical inner width of the last Automatic hug, to ignore its Resized echo.
    last_hug_phys_w: u32,
    /// Locked + Automatic: do not shrink the game webview below this. The OS
    /// window may hug; Granblue's viewport must not.
    game_keep_w: f64,
    /// Wiki panel width in use (960 or 800). 0 = closed / unset.
    wiki_panel_w: f64,
    /// Inner width before we grew the window for the wiki. 0 = none.
    panel_before_w: f64,
    /// Inner width we left after growing for the wiki. 0 = none.
    panel_after_w: f64,
    /// One hug after a panel open/close, even if Automatic already snapped.
    panel_hug_due: bool,
    /// User dragged the window while a panel was open; do not restore borrow.
    panel_user_resized: bool,
}

/// Per-window flags. `--multi-window` must not share collapsed/wiki/lock
/// across clones.
#[derive(Default)]
pub struct SidebarState {
    by_window: Mutex<HashMap<String, WindowFlags>>,
    /// Process-wide. Not per `--multi-window` clone.
    tray: AtomicBool,
}

impl SidebarState {
    fn with<R>(&self, label: &str, f: impl FnOnce(&WindowFlags) -> R) -> R {
        let map = self.by_window.lock().unwrap_or_else(|e| e.into_inner());
        f(map.get(label).unwrap_or(&WindowFlags::default()))
    }

    fn update(&self, label: &str, f: impl FnOnce(&mut WindowFlags)) {
        let mut map = self.by_window.lock().unwrap_or_else(|e| e.into_inner());
        f(map.entry(label.to_string()).or_default());
    }

    pub fn is_tray_enabled(&self) -> bool {
        self.tray.load(Ordering::Relaxed)
    }

    pub fn set_tray_enabled(&self, on: bool) {
        self.tray.store(on, Ordering::Relaxed);
    }

    pub fn is_collapsed(&self, label: &str) -> bool {
        self.with(label, |f| f.collapsed)
    }

    pub fn toggle(&self, label: &str) -> bool {
        let mut out = false;
        self.update(label, |f| {
            f.collapsed = !f.collapsed;
            f.collapsed_for_panel = false;
            out = f.collapsed;
        });
        out
    }

    pub fn wiki_is_open(&self, label: &str) -> bool {
        self.with(label, |f| f.wiki_open)
    }

    pub fn set_wiki_open(&self, label: &str, open: bool) {
        self.update(label, |f| {
            f.wiki_open = open;
            if open {
                f.about_open = false;
                f.options_open = false;
            }
        });
    }

    pub fn about_is_open(&self, label: &str) -> bool {
        self.with(label, |f| f.about_open)
    }

    pub fn set_about_open(&self, label: &str, open: bool) {
        self.update(label, |f| {
            f.about_open = open;
            if open {
                f.wiki_open = false;
                f.options_open = false;
            }
        });
    }

    pub fn options_is_open(&self, label: &str) -> bool {
        self.with(label, |f| f.options_open)
    }

    pub fn set_options_open(&self, label: &str, open: bool) {
        self.update(label, |f| {
            f.options_open = open;
            if open {
                f.wiki_open = false;
                f.about_open = false;
            }
        });
    }

    pub fn panel_is_open(&self, label: &str) -> bool {
        self.with(label, |f| f.wiki_open || f.about_open || f.options_open)
    }

    pub fn is_locked(&self, label: &str) -> bool {
        self.with(label, |f| f.locked)
    }

    pub fn is_wiki_outside(&self, label: &str) -> bool {
        self.with(label, |f| f.wiki_outside)
    }

    pub fn set_wiki_outside(&self, label: &str, on: bool) {
        self.update(label, |f| f.wiki_outside = on);
    }

    pub fn set_locked(&self, label: &str, on: bool) {
        self.update(label, |f| f.locked = on);
    }

    pub fn lock_was_auto(&self, label: &str) -> bool {
        self.with(label, |f| f.lock_was_auto)
    }

    pub fn set_lock_was_auto(&self, label: &str, on: bool) {
        self.update(label, |f| f.lock_was_auto = on);
    }

    pub fn game_edge(&self, label: &str) -> f64 {
        self.with(label, |f| f.game_edge)
    }

    pub fn set_game_edge(&self, label: &str, right: f64) {
        self.update(label, |f| f.game_edge = right);
    }

    pub fn game_overlay(&self, label: &str) -> f64 {
        self.with(label, |f| f.game_overlay)
    }

    pub fn set_game_overlay(&self, label: &str, right: f64) {
        self.update(label, |f| f.game_overlay = right);
    }

    pub fn game_dpr(&self, label: &str) -> f64 {
        self.with(label, |f| f.game_dpr)
    }

    pub fn set_game_dpr(&self, label: &str, dpr: f64) {
        self.update(label, |f| f.game_dpr = dpr);
    }

    pub fn is_automatic(&self, label: &str) -> bool {
        self.with(label, |f| f.automatic)
    }

    pub fn set_automatic(&self, label: &str, on: bool) {
        self.update(label, |f| f.automatic = on);
    }

    pub fn bump_hug_gen(&self, label: &str) -> u64 {
        let mut out = 0;
        self.update(label, |f| {
            f.hug_gen = f.hug_gen.wrapping_add(1);
            out = f.hug_gen;
        });
        out
    }

    pub fn hug_gen(&self, label: &str) -> u64 {
        self.with(label, |f| f.hug_gen)
    }

    pub fn set_auto_hug_due(&self, label: &str, on: bool) {
        self.update(label, |f| f.auto_hug_due = on);
    }

    pub fn take_auto_hug_due(&self, label: &str) -> bool {
        let mut out = false;
        self.update(label, |f| {
            out = f.auto_hug_due;
            f.auto_hug_due = false;
        });
        out
    }

    pub fn auto_hug_allowed(&self, label: &str) -> bool {
        self.with(label, |f| f.auto_hug_allowed)
    }

    pub fn set_auto_hug_allowed(&self, label: &str, on: bool) {
        self.update(label, |f| f.auto_hug_allowed = on);
    }

    pub fn last_hug_phys_w(&self, label: &str) -> u32 {
        self.with(label, |f| f.last_hug_phys_w)
    }

    pub fn set_last_hug_phys_w(&self, label: &str, w: u32) {
        self.update(label, |f| f.last_hug_phys_w = w);
    }

    pub fn game_keep_w(&self, label: &str) -> f64 {
        self.with(label, |f| f.game_keep_w)
    }

    pub fn raise_game_keep_w(&self, label: &str, w: f64) {
        self.update(label, |f| {
            if w > f.game_keep_w {
                f.game_keep_w = w;
            }
        });
    }

    pub fn set_game_keep_w(&self, label: &str, w: f64) {
        self.update(label, |f| f.game_keep_w = w);
    }

    pub fn wiki_panel_w(&self, label: &str) -> f64 {
        self.with(label, |f| f.wiki_panel_w)
    }

    pub fn set_wiki_panel_w(&self, label: &str, w: f64) {
        self.update(label, |f| f.wiki_panel_w = w);
    }

    fn save_panel_before_w(&self, label: &str, width: f64) {
        self.update(label, |f| {
            if f.panel_before_w <= 1.0 {
                f.panel_before_w = width;
            }
        });
    }

    fn set_panel_after_w(&self, label: &str, width: f64) {
        self.update(label, |f| f.panel_after_w = width);
    }

    fn take_panel_restore(&self, label: &str) -> (f64, f64) {
        let mut before = 0.0;
        let mut after = 0.0;
        self.update(label, |f| {
            before = f.panel_before_w;
            after = f.panel_after_w;
            f.panel_before_w = 0.0;
            f.panel_after_w = 0.0;
            f.wiki_panel_w = 0.0;
        });
        (before, after)
    }

    fn take_panel_hug_due(&self, label: &str) -> bool {
        let mut out = false;
        self.update(label, |f| {
            out = f.panel_hug_due;
            f.panel_hug_due = false;
        });
        out
    }

    fn set_panel_hug_due(&self, label: &str, on: bool) {
        self.update(label, |f| f.panel_hug_due = on);
    }

    fn take_panel_user_resized(&self, label: &str) -> bool {
        let mut out = false;
        self.update(label, |f| {
            out = f.panel_user_resized;
            f.panel_user_resized = false;
        });
        out
    }

    /// A user width-drag (not our own hug) may have one Automatic snap.
    /// A drag while wiki/About is open means we must not restore the borrowed width.
    pub fn arm_auto_hug_if_user_resize(&self, label: &str, phys_w: u32) {
        self.update(label, |f| {
            if f.hug_busy {
                return;
            }
            if f.wiki_open || f.about_open || f.options_open {
                f.panel_user_resized = true;
            }
            if !f.locked || !f.automatic {
                return;
            }
            let last = f.last_hug_phys_w;
            if last == 0 || phys_w.abs_diff(last) > 8 {
                f.auto_hug_allowed = true;
            }
        });
    }

    pub fn hug_busy(&self, label: &str) -> bool {
        self.with(label, |f| f.hug_busy)
    }

    pub fn set_hug_busy(&self, label: &str, on: bool) {
        self.update(label, |f| f.hug_busy = on);
    }

    pub fn save_hug_width_once(&self, label: &str, width: f64) {
        self.update(label, |f| {
            if f.hug_saved_w <= 1.0 {
                f.hug_saved_w = width;
            }
        });
    }

    pub fn take_hug_saved_w(&self, label: &str) -> f64 {
        let mut out = 0.0;
        self.update(label, |f| {
            out = f.hug_saved_w;
            f.hug_saved_w = 0.0;
            f.hug_busy = false;
        });
        out
    }

    fn collapse_for_panel(&self, label: &str) {
        self.update(label, |f| {
            f.collapsed = true;
            f.collapsed_for_panel = true;
        });
    }

    fn restore_collapse_for_panel(&self, label: &str) {
        self.update(label, |f| {
            if f.collapsed_for_panel && !f.wiki_open && !f.about_open && !f.options_open {
                f.collapsed = false;
                f.collapsed_for_panel = false;
            }
        });
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

fn wiki_webview(host: &Window) -> Option<Webview> {
    let label = wiki_label(host.label());
    host.webviews().into_iter().find(|w| w.label() == label)
}

fn about_webview(host: &Window) -> Option<Webview> {
    let label = about_label(host.label());
    host.webviews().into_iter().find(|w| w.label() == label)
}

fn options_webview(host: &Window) -> Option<Webview> {
    let label = options_label(host.label());
    host.webviews().into_iter().find(|w| w.label() == label)
}

/// The window's client area divided between the three webviews.
///
/// Left to right: game | wiki | sidebar. The wiki sits next to the sidebar
/// rather than next to the game, matching where main's slide-out panel appears.
struct Split {
    game_w: f64,
    wiki_w: f64,
    sidebar_w: f64,
    height: f64,
}

/// Widths for the current window size, in logical pixels.
///
/// Priority when space is short: the game keeps `MIN_GAME_WIDTH` first, the
/// sidebar takes what it needs second, and the wiki gets whatever is left. The
/// wiki is the only one that can be squeezed to nothing, which is why
/// `gbf_wiki_toggle` refuses to open below `WIKI_MIN_W` instead of showing an
/// unreadable column.
fn split(
    host: &Window,
    collapsed: bool,
    wiki_open: bool,
    about_open: bool,
    options_open: bool,
) -> tauri::Result<Split> {
    let scale = host.scale_factor()?;
    let size = host.inner_size()?.to_logical::<f64>(scale);

    let want_bar = if collapsed {
        SIDEBAR_W_COLLAPSED
    } else {
        SIDEBAR_W
    };
    let sidebar_w = want_bar.min((size.width - MIN_GAME_WIDTH).max(0.0));

    let room_for_panel = (size.width - MIN_GAME_WIDTH - sidebar_w).max(0.0);
    let slim = about_open || options_open;
    let panel_open = wiki_open || slim;
    let want_panel = if panel_open {
        let chosen = host
            .app_handle()
            .state::<SidebarState>()
            .wiki_panel_w(host.label());
        let fallback = if slim { ABOUT_W } else { WIKI_W };
        if chosen > 1.0 {
            chosen
        } else {
            fallback
        }
    } else {
        0.0
    };
    let wiki_w = if panel_open {
        want_panel.min(room_for_panel)
    } else {
        0.0
    };

    let game_w = (size.width - sidebar_w - wiki_w).max(1.0);
    Ok(Split {
        game_w,
        wiki_w,
        sidebar_w,
        height: size.height,
    })
}

/// Ignore leftover smaller than this when hugging (rounding / DPI).
const HUG_SLACK: f64 = 8.0;

/// CSS pixels in the game webview → window logical pixels.
///
/// WebView2's `devicePixelRatio` is not the OS window `scale_factor`. Treating
/// `#wrapper.right` as logical placed the sidebar ~14px into the game
/// (measured 2026-09-05: dpr 1.126, scale 1.104, wrapper 640 → 653 logical).
fn css_to_window_logical(host: &Window, css: f64, dpr: f64) -> f64 {
    let scale = host.scale_factor().unwrap_or(1.0).max(0.05);
    let dpr = if dpr > 0.05 { dpr } else { 1.0 };
    css * dpr / scale
}

/// CSS pixels of the edge the sidebar should sit on.
/// Locked: `#wrapper`. Unlocked: the submenu overlay (collapsed rail or
/// expanded chat panel).
fn snap_css(state: &SidebarState, label: &str) -> f64 {
    if state.is_locked(label) {
        state.game_edge(label)
    } else {
        state.game_overlay(label)
    }
}

/// X origin of the wiki/About + sidebar column.
///
/// Locked: `#wrapper`'s right edge. Unlocked: the submenu overlay's right
/// edge so the sidebar sits on Chat/Settings, collapsed or expanded.
fn column_x(host: &Window, s: &Split) -> f64 {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label();
    let css = snap_css(&state, label);
    if css <= 1.0 {
        return s.game_w;
    }
    css_to_window_logical(host, css, state.game_dpr(label)).max(0.0)
}

/// Locked: shrink/grow the OS window so its right edge sits on the sidebar.
///
/// Fixed Window Size hugs immediately. Automatic waits for
/// `schedule_automatic_hug` so a width drag can let GBF grow up to its cap
/// first; leftover past that cap is then hugged away (the dead strip between
/// game and sidebar).
fn maybe_hug_window(host: &Window, col: f64, s: &Split) -> tauri::Result<bool> {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    if snap_css(&state, &label) <= 1.0 {
        return Ok(false);
    }
    if state.hug_busy(&label) {
        return Ok(false);
    }
    let panel_due = state.take_panel_hug_due(&label);
    if state.is_automatic(&label) {
        let auto_due = state.take_auto_hug_due(&label);
        if !panel_due && !auto_due {
            return Ok(false);
        }
        // One snap per user drag. A panel/sidebar hug does not spend that token.
        if auto_due && !panel_due {
            state.set_auto_hug_allowed(&label, false);
        }
    }
    let scale = host.scale_factor()?;
    let phys = host.inner_size()?;
    let inner_w = phys.to_logical::<f64>(scale).width;
    // Prefer the rail we asked for, not the squeezed leftover. After collapse
    // the window is too narrow for 250, so split() reports ~52–59 and a hug
    // to that width is a no-op — expand would never grow back.
    // Same for an open panel: hug to the reserved width, not whatever
    // leftover split() carved from the current frame. Collapse-then-expand
    // at Small was eating Options/About/wiki instead of growing the window.
    let want_bar = sidebar_want(state.is_collapsed(&label));
    let want_w = col + reserved_panel_w(&state, &label, s.wiki_w) + want_bar;
    if state.is_automatic(&label) && !panel_due {
        // Close leftover to the right of the sidebar only. Growing would
        // fight a shrink, and a second shrink after GBF reflows is the walk-down.
        if inner_w <= want_w + HUG_SLACK {
            return Ok(false);
        }
    } else if (inner_w - want_w).abs() <= HUG_SLACK {
        return Ok(false);
    }
    let want_phys = (want_w * scale).round() as u32;
    if want_phys == 0 || want_phys == phys.width {
        return Ok(false);
    }
    state.save_hug_width_once(&label, inner_w);
    state.set_hug_busy(&label, true);
    if state.is_automatic(&label) {
        state.set_last_hug_phys_w(&label, want_phys);
    }
    if let Err(e) = host.set_size(PhysicalSize::new(want_phys, phys.height)) {
        state.set_hug_busy(&label, false);
        if state.is_automatic(&label) {
            state.set_auto_hug_allowed(&label, true);
        }
        return Err(e);
    }
    Ok(true)
}

/// First leftover after a user drag is hugged away. Further hugs are skipped
/// until the next user resize, so GBF reflow cannot walk the window down.
fn schedule_automatic_hug(host: &Window, col: f64, s: &Split) {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    if !state.is_automatic(&label) || state.hug_busy(&label) {
        return;
    }
    if !state.auto_hug_allowed(&label) {
        return;
    }
    if snap_css(&state, &label) <= 1.0 {
        return;
    }
    let Ok(scale) = host.scale_factor() else {
        return;
    };
    let Ok(phys) = host.inner_size() else {
        return;
    };
    let inner_w = phys.to_logical::<f64>(scale).width;
    let want_w = col + reserved_panel_w(&state, &label, s.wiki_w) + sidebar_want(state.is_collapsed(&label));
    if inner_w <= want_w + HUG_SLACK {
        return;
    }
    let gen = state.bump_hug_gen(&label);
    let app = host.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        let state = app.state::<SidebarState>();
        if state.hug_gen(&label) != gen || !state.is_locked(&label) {
            return;
        }
        let app_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            let state = app_main.state::<SidebarState>();
            if state.hug_gen(&label) != gen {
                return;
            }
            state.set_auto_hug_due(&label, true);
            if let Some(host) = app_main.get_window(&label) {
                let _ = layout(&host);
            }
        });
    });
}

fn restore_hug_width(host: &Window) -> String {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let saved = state.take_hug_saved_w(&label);
    if saved <= 1.0 {
        return "restore=none".into();
    }
    let scale = match host.scale_factor() {
        Ok(v) => v,
        Err(e) => return format!("restore ERR {e}"),
    };
    let phys = match host.inner_size() {
        Ok(v) => v,
        Err(e) => return format!("restore ERR {e}"),
    };
    let want_phys = (saved * scale).round() as u32;
    if want_phys == 0 || want_phys == phys.width {
        return format!("restore=skip saved={saved:.0}");
    }
    state.set_hug_busy(&label, true);
    match host.set_size(PhysicalSize::new(want_phys, phys.height)) {
        Ok(()) => format!("restore={saved:.0}"),
        Err(e) => {
            state.set_hug_busy(&label, false);
            format!("restore ERR {e}")
        }
    }
}

/// How much inner width the monitor can actually hold (logical px).
///
/// Uses the work area, minus frame chrome. If the OS reports something smaller
/// than the window we already have, ignore it — same trap live hit with
/// `screen.availWidth`.
fn monitor_inner_ceiling(host: &Window) -> f64 {
    let Ok(scale) = host.scale_factor() else {
        return f64::INFINITY;
    };
    let Ok(inner) = host.inner_size() else {
        return f64::INFINITY;
    };
    let inner_log = inner.to_logical::<f64>(scale).width;
    let Ok(Some(monitor)) = host.current_monitor() else {
        return f64::INFINITY;
    };
    let work_w = monitor.work_area().size.width;
    let frame = host
        .outer_size()
        .ok()
        .map(|o| o.width.saturating_sub(inner.width))
        .unwrap_or(0);
    let max_phys = work_w.saturating_sub(frame);
    if max_phys <= inner.width {
        return f64::INFINITY;
    }
    (max_phys as f64 / scale).max(inner_log)
}

fn sidebar_want(collapsed: bool) -> f64 {
    if collapsed {
        SIDEBAR_W_COLLAPSED
    } else {
        SIDEBAR_W
    }
}

/// Width the open panel asked for, not the leftover `split()` could steal.
fn reserved_panel_w(state: &SidebarState, label: &str, split_panel: f64) -> f64 {
    if !(state.wiki_is_open(label) || state.about_is_open(label) || state.options_is_open(label)) {
        return 0.0;
    }
    let reserved = state.wiki_panel_w(label);
    if reserved > 1.0 {
        reserved
    } else {
        split_panel
    }
}

fn needed_for_panel(game_col: f64, panel: f64, collapsed: bool) -> f64 {
    game_col + panel + sidebar_want(collapsed) + WIKI_WIDEN_BUFFER
}

fn panel_no_room_notice(automatic: bool) -> String {
    if automatic {
        "Not enough room. Widen the window, or pick a fixed Window Size in Granblue's Browser Settings.".into()
    } else {
        "Not enough room. In Granblue's Browser Settings, pick a smaller Window Size.".into()
    }
}

/// Fit the OS window to `want` logical inner width. Height is echoed so it
/// cannot drift. Grows when a panel needs room; shrinks leftover when a
/// smaller panel (or a close) no longer needs the wiki's width.
/// Under Automatic a size change reloads Granblue — that is GBF's own
/// behaviour; wiki and About still have to take the width they need.
fn fit_inner_width(host: &Window, want: f64, allow_shrink: bool) -> tauri::Result<f64> {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let scale = host.scale_factor()?;
    let phys = host.inner_size()?;
    let inner_w = phys.to_logical::<f64>(scale).width;
    let ceiling = monitor_inner_ceiling(host);
    let target = want.min(ceiling).max(1.0);
    let want_phys = (target * scale).round() as u32;
    if want_phys == 0 || want_phys == phys.width {
        return Ok(inner_w);
    }
    if want_phys > phys.width {
        state.save_panel_before_w(&label, inner_w);
    } else if !allow_shrink {
        return Ok(inner_w);
    }
    state.set_hug_busy(&label, true);
    if state.is_automatic(&label) {
        state.set_last_hug_phys_w(&label, want_phys);
    }
    host.set_size(PhysicalSize::new(want_phys, phys.height))?;
    let after = host
        .inner_size()
        .ok()
        .map(|p| p.to_logical::<f64>(scale).width)
        .unwrap_or(target);
    state.set_panel_after_w(&label, after);
    Ok(after)
}

fn grow_inner_width(host: &Window, want: f64) -> tauri::Result<f64> {
    fit_inner_width(host, want, false)
}

/// After a panel open/close, hug leftover once Granblue has reflowed.
/// Does not use the Automatic user-drag hug token, so it still runs after
/// that token is spent — and it does not hug again on later GBF edge updates.
fn schedule_panel_hug(host: &Window) {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    if !state.is_locked(&label) {
        return;
    }
    let gen = state.bump_hug_gen(&label);
    let app = host.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        let state = app.state::<SidebarState>();
        if state.hug_gen(&label) != gen || !state.is_locked(&label) {
            return;
        }
        let app_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            let state = app_main.state::<SidebarState>();
            if state.hug_gen(&label) != gen {
                return;
            }
            state.set_panel_hug_due(&label, true);
            if let Some(host) = app_main.get_window(&label) {
                let _ = layout(&host);
            }
        });
    });
}

fn restore_panel_width(host: &Window) {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let (before, _after) = state.take_panel_restore(&label);
    let user_resized = state.take_panel_user_resized(&label);
    if user_resized {
        schedule_panel_hug(host);
        return;
    }
    let automatic = state.is_automatic(&label);
    // Automatic: do not yank back to the pre-panel size. GBF may have
    // reflowed larger; restoring a stale width clips the sidebar. Hug leftover
    // to the current column instead.
    if !automatic && before > 1.0 {
        let Ok(scale) = host.scale_factor() else {
            schedule_panel_hug(host);
            return;
        };
        let Ok(phys) = host.inner_size() else {
            schedule_panel_hug(host);
            return;
        };
        let want_phys = (before * scale).round() as u32;
        if want_phys != 0 && want_phys != phys.width {
            state.set_hug_busy(&label, true);
            state.set_last_hug_phys_w(&label, want_phys);
            let _ = host.set_size(PhysicalSize::new(want_phys, phys.height));
        }
    }
    if automatic {
        // Stale keep from the pre-wiki drag is wider than the hugged
        // window and would paint the game over the sidebar.
        state.set_game_keep_w(&label, 0.0);
        state.set_auto_hug_allowed(&label, false);
        state.set_panel_hug_due(&label, true);
        let _ = layout(host);
        schedule_panel_hug(host);
    } else {
        schedule_panel_hug(host);
    }
}

/// Pick the preferred width if the monitor can hold it, else the fallback,
/// else refuse. Collapse the sidebar when that is what makes a tier fit.
fn prepare_panel_open(
    host: &Window,
    prefer: f64,
    minimum: f64,
    name: &str,
) -> Result<String, String> {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let automatic = state.is_automatic(&label);
    let collapsed = state.is_collapsed(&label);
    let game_col = {
        let css = snap_css(&state, &label);
        if css > 1.0 {
            css_to_window_logical(host, css, state.game_dpr(&label))
        } else {
            MIN_GAME_WIDTH
        }
    };
    let ceiling = monitor_inner_ceiling(host);

    let fits = |panel: f64, bar_collapsed: bool| {
        needed_for_panel(game_col, panel, bar_collapsed) <= ceiling + WIKI_TIER_SLACK
    };

    let (chosen, need_collapse) = if fits(prefer, collapsed) {
        (prefer, false)
    } else if !collapsed && fits(prefer, true) {
        (prefer, true)
    } else if fits(minimum, collapsed) {
        (minimum, false)
    } else if !collapsed && fits(minimum, true) {
        (minimum, true)
    } else {
        return Err(panel_no_room_notice(automatic));
    };

    let mut note = String::new();
    if need_collapse {
        state.collapse_for_panel(&label);
        note.push_str(&format!("Sidebar collapsed to make room for the {name}.\n"));
    }
    let want = needed_for_panel(game_col, chosen, collapsed || need_collapse);
    let after = fit_inner_width(host, want, true).map_err(|e| e.to_string())?;
    let bar = sidebar_want(collapsed || need_collapse);
    let space = (after - game_col - bar).max(0.0);
    let reserved = if space >= prefer - WIKI_TIER_SLACK {
        space.min(prefer)
    } else if space >= minimum {
        space.min(minimum)
    } else {
        0.0
    };
    if reserved < minimum {
        if need_collapse {
            state.update(&label, |f| {
                f.collapsed = false;
                f.collapsed_for_panel = false;
            });
        }
        restore_panel_width(host);
        return Err(panel_no_room_notice(automatic));
    }
    state.set_wiki_panel_w(&label, reserved);
    schedule_panel_hug(host);
    Ok(format!(
        "{note}{name}_tier={reserved:.0} game_col={game_col:.0} ceiling={ceiling:.0} after={after:.0}\n"
    ))
}

fn prepare_wiki_open(host: &Window) -> Result<String, String> {
    prepare_panel_open(host, WIKI_W, WIKI_MIN_W, "wiki")
}

fn prepare_about_open(host: &Window) -> Result<String, String> {
    prepare_panel_open(host, ABOUT_W, ABOUT_MIN_W, "about")
}

fn prepare_options_open(host: &Window) -> Result<String, String> {
    prepare_panel_open(host, ABOUT_W, ABOUT_MIN_W, "options")
}

fn wait_for_game_edge(app: &AppHandle, label: &str) -> f64 {
    for _ in 0..25 {
        let edge = app.state::<SidebarState>().game_edge(label);
        if edge > 1.0 {
            return edge;
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
    app.state::<SidebarState>().game_edge(label)
}

/// Place both webviews side by side across the window's client area.
///
/// Everything is in LOGICAL pixels. This is the whole point of moving the
/// sidebar out of the page: logical pixels are the window's own coordinate
/// space, so there is no CSS-pixel/physical-pixel conversion to get wrong and
/// no page zoom folded into `devicePixelRatio` to correct for. The drift bug
/// that Patch 2 in GBF_Pake_UPSTREAM_PATCHES.md exists to fix cannot occur here.
pub fn layout(host: &Window) -> tauri::Result<()> {
    let _ = apply_layout(host)?;
    Ok(())
}

/// `layout()`, narrating every call it makes.
///
/// Kept permanently rather than deleted after the bug it found. Nothing here is
/// observable from outside -- the sidebar webview is not exposed as a CDP
/// target, and a skipped relayout prints nothing and returns Ok -- so this is
/// the only way to see what actually happened.
fn layout_verbose(host: &Window) -> String {
    match apply_layout(host) {
        Ok(text) => text,
        Err(e) => format!("layout ERR {e}"),
    }
}

fn apply_layout(host: &Window) -> tauri::Result<String> {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let collapsed = state.is_collapsed(&label);
    let wiki_open = state.wiki_is_open(&label);
    let about_open = state.about_is_open(&label);
    let options_open = state.options_is_open(&label);
    let locked = state.is_locked(&label);
    let automatic = state.is_automatic(&label);
    let outside = state.is_wiki_outside(&label);
    let tray = state.is_tray_enabled();
    let s = split(host, collapsed, wiki_open, about_open, options_open)?;
    if s.height <= 0.0 {
        state.set_hug_busy(&label, false);
        return Ok("layout: minimized\n".into());
    }
    let col = column_x(host, &s);
    let edge = state.game_edge(&label);
    let want_w = col + s.wiki_w + s.sidebar_w;

    let mut out = format!(
        "want game={:.0} panel={:.0} bar={:.0} col={col:.0} edge={edge:.0} locked={locked} auto={automatic} outside={outside} hug={want_w:.0} h={:.0} wiki={wiki_open} about={about_open} options={options_open}\n",
        s.game_w, s.wiki_w, s.sidebar_w, s.height
    );

    if maybe_hug_window(host, col, &s)? {
        out.push_str("hug: set_size issued\n");
        // Keep going and place the chrome. Returning here left the sidebar
        // at its pre-hug x, which is off the right edge of the smaller
        // window (recording 113543: close wiki at Automatic cap).
    }

    let panel_min = if wiki_open {
        WIKI_MIN_W
    } else if about_open || options_open {
        ABOUT_MIN_W
    } else {
        0.0
    };
    if (wiki_open || about_open || options_open) && panel_min > 0.0 && s.wiki_w + 1.0 >= panel_min {
        let scale = host.scale_factor()?;
        let inner_w = host.inner_size()?.to_logical::<f64>(scale).width;
        let want = col + s.wiki_w + s.sidebar_w + WIKI_WIDEN_BUFFER;
        let ceiling = monitor_inner_ceiling(host);
        if inner_w + HUG_SLACK < want && want <= ceiling + WIKI_TIER_SLACK {
            let after = grow_inner_width(host, want).unwrap_or(inner_w);
            if after > inner_w + 1.0 {
                out.push_str(&format!("panel grow {inner_w:.0}->{after:.0}\n"));
                return Ok(out);
            }
        }
    }

    let scale = host.scale_factor()?;
    let inner_w = host.inner_size()?.to_logical::<f64>(scale).width;
    let lock_fill = locked && edge > 1.0;
    let unlock_snap = !locked && snap_css(&state, &label) > 1.0;
    let panel_open = wiki_open || about_open || options_open;
    // Locked, no panel: game webview stays full-window so the sidebar can
    // sit on leftover without shrinking Granblue. Unlocked: tile the game
    // to the submenu overlay so Chat/Settings is flush with the sidebar.
    // A panel is always a sibling column.
    let game_w = if (lock_fill || unlock_snap) && panel_open {
        col.max(1.0)
    } else if lock_fill && automatic {
        // Overlay. Never shrink Granblue's viewport because the OS frame hugged.
        if !state.hug_busy(&label) {
            let last = state.last_hug_phys_w(&label);
            let phys = host.inner_size()?.width;
            if inner_w + HUG_SLACK < state.game_keep_w(&label)
                && (last == 0 || phys.abs_diff(last) > 8)
            {
                state.set_game_keep_w(&label, inner_w);
            } else {
                state.raise_game_keep_w(&label, inner_w);
            }
        }
        let overlay = state.game_keep_w(&label).max(inner_w).max(1.0);
        // Overlay only while a user drag still has leftover for GBF to grow
        // into. Once that hug is spent (or a panel just closed), tile to
        // `#wrapper` so the sidebar is a sibling, not under the game
        // (recording 113543).
        if state.auto_hug_allowed(&label) {
            overlay.min(inner_w).max(1.0)
        } else {
            col.max(1.0)
        }
    } else if lock_fill {
        inner_w.max(1.0)
    } else if unlock_snap {
        col.max(1.0)
    } else {
        s.game_w
    };

    match game_webview(host) {
        None => out.push_str("game webview NOT FOUND\n"),
        Some(game) => {
            let p = game.set_position(LogicalPosition::new(0.0, 0.0));
            let z = game.set_size(LogicalSize::new(game_w, s.height));
            out.push_str(&format!(
                "game pos={} size={} now={}\n",
                result_word(&p),
                result_word(&z),
                bounds_word(&game),
            ));
        }
    }

    // The wiki keeps its webview once created, so a closed panel is hidden
    // rather than destroyed. That is the point of it being its own webview:
    // your page, scroll position and history survive being closed and
    // reopened, and survive the game reloading beside it.
    let panel_w = if (wiki_open || about_open || options_open) && s.wiki_w > 0.0 {
        s.wiki_w
    } else {
        0.0
    };
    let bar_x = if outside { col } else { col + panel_w };
    let panel_x = if outside { col + s.sidebar_w } else { col };

    match wiki_webview(host) {
        None => out.push_str("wiki: not created\n"),
        Some(wiki) => {
            if wiki_open && panel_w > 0.0 {
                let p = wiki.set_position(LogicalPosition::new(panel_x, 0.0));
                let z = wiki.set_size(LogicalSize::new(panel_w, s.height));
                let v = wiki.show();
                out.push_str(&format!(
                    "wiki pos={} size={} show={} now={}\n",
                    result_word(&p),
                    result_word(&z),
                    result_word(&v),
                    bounds_word(&wiki),
                ));
            } else {
                out.push_str(&format!("wiki hide={}\n", result_word(&wiki.hide())));
            }
        }
    }

    match about_webview(host) {
        None => out.push_str("about: not created\n"),
        Some(about) => {
            if about_open && panel_w > 0.0 {
                let p = about.set_position(LogicalPosition::new(panel_x, 0.0));
                let z = about.set_size(LogicalSize::new(panel_w, s.height));
                let v = about.show();
                out.push_str(&format!(
                    "about pos={} size={} show={} now={}\n",
                    result_word(&p),
                    result_word(&z),
                    result_word(&v),
                    bounds_word(&about),
                ));
            } else {
                out.push_str(&format!("about hide={}\n", result_word(&about.hide())));
            }
        }
    }

    match options_webview(host) {
        None => out.push_str("options: not created\n"),
        Some(options) => {
            if options_open && panel_w > 0.0 {
                let p = options.set_position(LogicalPosition::new(panel_x, 0.0));
                let z = options.set_size(LogicalSize::new(panel_w, s.height));
                let v = options.show();
                out.push_str(&format!(
                    "options pos={} size={} show={} now={}\n",
                    result_word(&p),
                    result_word(&z),
                    result_word(&v),
                    bounds_word(&options),
                ));
            } else {
                out.push_str(&format!("options hide={}\n", result_word(&options.hide())));
            }
            let e = options.eval(format!(
                "window.__gbfOptions && window.__gbfOptions.setState({{wikiOutside:{outside},tray:{tray}}})"
            ));
            out.push_str(&format!("options eval={}\n", result_word(&e)));
        }
    }

    match sidebar_webview(host) {
        None => out.push_str("sidebar webview NOT FOUND\n"),
        Some(bar) => {
            let p = bar.set_position(LogicalPosition::new(bar_x, 0.0));
            let z = bar.set_size(LogicalSize::new(s.sidebar_w, s.height));
            let e = bar.eval(format!(
                "window.__gbfSidebar && window.__gbfSidebar.setState({{collapsed:{collapsed},wikiOpen:{wiki_open},aboutOpen:{about_open},optionsOpen:{options_open},locked:{locked},wikiOutside:{outside}}})"
            ));
            let _ = bar.hide();
            let v = bar.show();
            out.push_str(&format!(
                "bar pos={} size={} eval={} show={} now={}\n",
                result_word(&p),
                result_word(&z),
                result_word(&e),
                result_word(&v),
                bounds_word(&bar),
            ));
        }
    }

    schedule_automatic_hug(host, col, &s);
    state.set_hug_busy(&label, false);
    Ok(out)
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
    crate::app::window::restore_window_geometry(&host);

    let sp = split(&host, false, false, false, false)?;

    // Served from the bundled assets as tauri://localhost/gbf-sidebar.html, so
    // it is trusted local content with a working IPC bridge. It is deliberately
    // NOT part of Granblue's origin -- that separation is the feature.
    //
    // No auto_resize(): this webview is positioned by layout() alone. Letting
    // Tauri grow it with the window would put it back on top of the game.
    let builder = WebviewBuilder::new(&label, WebviewUrl::App("gbf-sidebar.html".into()));

    host.add_child(
        builder,
        LogicalPosition::new(sp.game_w, 0.0),
        LogicalSize::new(sp.sidebar_w, sp.height),
    )?;

    // Built now, empty and hidden, because add_child only works before the
    // event loop starts. See create_wiki().
    if let Err(error) = create_wiki(&host, &sp) {
        eprintln!("[Pake][gbf] could not create the wiki webview: {error}");
    }
    if let Err(error) = create_about(&host, &sp) {
        eprintln!("[Pake][gbf] could not create the about webview: {error}");
    }
    if let Err(error) = create_options(&host, &sp) {
        eprintln!("[Pake][gbf] could not create the options webview: {error}");
    }

    // Restore lock before the first layout. Do not call set_lock(false) here:
    // unlock would try to restore a hug width and fight the size we just
    // restored from disk.
    if restore_layout_locked(host.app_handle(), host.label()) {
        let _ = set_lock(&host, true);
    }
    if restore_layout_wiki_outside(host.app_handle(), host.label()) {
        host.app_handle()
            .state::<SidebarState>()
            .set_wiki_outside(host.label(), true);
    }

    layout(&host)?;
    crate::app::window::persist_window_geometry(host.app_handle());
    persist_layout_state(host.app_handle());

    // The game webview no longer follows the window on its own, so every resize
    // has to re-run the split. A user width-drag (not our hug) arms one
    // Automatic snap; GBF reflow after that snap must not arm another.
    let on_resize = host.clone();
    host.on_window_event(move |event| {
        match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let state = on_resize.app_handle().state::<SidebarState>();
                let label = on_resize.label().to_string();
                if let Ok(phys) = on_resize.inner_size() {
                    state.arm_auto_hug_if_user_resize(&label, phys.width);
                }
                if let Err(error) = layout(&on_resize) {
                    eprintln!("[Pake][gbf] sidebar relayout failed: {error}");
                }
                crate::app::window::schedule_persist_window_geometry(
                    on_resize.app_handle().clone(),
                );
            }
            WindowEvent::Moved(_) => {
                crate::app::window::schedule_persist_window_geometry(
                    on_resize.app_handle().clone(),
                );
            }
            _ => {}
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
    let collapsed = app.state::<SidebarState>().toggle(window.label());

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let label = window.label().to_string();

    app.run_on_main_thread(move || {
        // get_window, NOT get_webview_window -- see the module docs.
        let report = match handle.get_window(&label) {
            Some(host) => {
                let state = handle.state::<SidebarState>();
                if snap_css(&state, &label) > 1.0 {
                    // Snap column (lock or unlocked overlay): hug to the new rail.
                    state.set_panel_hug_due(&label, true);
                } else {
                    // Tiled: keep the game column, grow/shrink the OS window
                    // by the rail delta so Automatic leftover is removed
                    // instead of left as empty window.
                    let delta = sidebar_want(collapsed) - sidebar_want(!collapsed);
                    if let (Ok(scale), Ok(phys)) = (host.scale_factor(), host.inner_size()) {
                        let inner = phys.to_logical::<f64>(scale).width;
                        let _ = fit_inner_width(&host, inner + delta, true);
                    }
                }
                layout_verbose(&host)
            }
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

    // The two manager views side by side -- the evidence for the get_window
    // rule in the module docs.
    let mut wv: Vec<String> = window.app_handle().webview_windows().keys().cloned().collect();
    wv.sort();
    let mut wins: Vec<String> = window.app_handle().windows().keys().cloned().collect();
    wins.sort();

    let scale = window.scale_factor().unwrap_or(-1.0);
    let phys = window.inner_size().map_err(|e| e.to_string())?;
    let logical = phys.to_logical::<f64>(if scale > 0.0 { scale } else { 1.0 });
    let collapsed = window.app_handle().state::<SidebarState>().is_collapsed(window.label());
    let locked = window.app_handle().state::<SidebarState>().is_locked(window.label());
    let edge = window.app_handle().state::<SidebarState>().game_edge(window.label());
    let overlay_edge = window.app_handle().state::<SidebarState>().game_overlay(window.label());
    let automatic = window.app_handle().state::<SidebarState>().is_automatic(window.label());
    let keep = window.app_handle().state::<SidebarState>().game_keep_w(window.label());
    let hug_allowed = window.app_handle().state::<SidebarState>().auto_hug_allowed(window.label());
    let hug_busy = window.app_handle().state::<SidebarState>().hug_busy(window.label());
    let last_hug = window.app_handle().state::<SidebarState>().last_hug_phys_w(window.label());
    let wiki_open = window.app_handle().state::<SidebarState>().wiki_is_open(window.label());
    let about_open = window.app_handle().state::<SidebarState>().about_is_open(window.label());
    let options_open = window.app_handle().state::<SidebarState>().options_is_open(window.label());
    let wiki_panel = window.app_handle().state::<SidebarState>().wiki_panel_w(window.label());
    let wiki_outside = window.app_handle().state::<SidebarState>().is_wiki_outside(window.label());
    let tray = window.app_handle().state::<SidebarState>().is_tray_enabled();
    let tray_icon = window.app_handle().tray_by_id("pake-tray").is_some();
    let dpr = window.app_handle().state::<SidebarState>().game_dpr(window.label());
    let persist = crate::app::window::persisted_window_state_path(window.app_handle())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "none".into());
    let layout_persist = persisted_layout_state_path(window.app_handle())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "none".into());
    let monitor = monitor_inner_ceiling(&window);

    let report = format!(
        "window={}\nsidebar={}\nwebviews:\n  {}\nwebview_windows()={wv:?}\nwindows()={wins:?}\nscale={scale:.3}\ndpr={dpr:.3}\nphysical={}x{}\nlogical={:.0}x{:.0}\ncollapsed={collapsed}\nlocked={locked}\nautomatic={automatic}\nedge={edge:.0}\noverlay={overlay_edge:.0}\nkeep={keep:.0}\nhug_allowed={hug_allowed}\nhug_busy={hug_busy}\nlast_hug_phys={last_hug}\nwiki_open={wiki_open}\nabout_open={about_open}\noptions_open={options_open}\nwiki_panel={wiki_panel:.0}\nwiki_outside={wiki_outside}\ntray={tray}\ntray_icon={tray_icon}\nmonitor={monitor:.0}\npersist={persist}\nlayout_persist={layout_persist}",
        window.label(),
        sidebar_label(window.label()),
        bounds.join("\n  "),
        phys.width,
        phys.height,
        logical.width,
        logical.height,
    );
    // The sidebar webview is not a CDP target, and synthetic clicks often miss
    // its footer, so the same snapshot is also written where a later session
    // can read it without the overlay.
    let dump_path = std::env::temp_dir().join("gbf-native-sidebar-debug.txt");
    let _ = std::fs::write(&dump_path, &report);
    eprintln!("[Pake][gbf] gbf_debug written to {}", dump_path.display());
    Ok(report)
}

/// Drive the same Hide/Show path the tray uses, so it can be verified without
/// finding the tray icon. Experiment diagnostic only.
#[tauri::command]
pub fn gbf_toggle_app_windows(app: AppHandle) -> Result<String, String> {
    let before = crate::app::window::any_app_window_visible(&app);
    crate::app::window::toggle_all_app_windows(&app, false);
    let after = crate::app::window::any_app_window_visible(&app);
    let mut labels: Vec<String> = app.windows().keys().cloned().collect();
    labels.sort();
    Ok(format!(
        "any_visible {before} -> {after}; windows()={labels:?}"
    ))
}

/// Open a `--multi-window` clone with its own sidebar. Tray New Window uses
/// the same `open_additional_window` path; this exists so it can be verified
/// without finding the tray icon (same reason as `gbf_toggle_app_windows`).
#[tauri::command]
pub fn gbf_new_window(app: AppHandle) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let report = match crate::app::window::open_additional_window(&app) {
            Ok(window) => {
                let opened = window.label().to_string();
                let host = window.as_ref().window();
                let mut labels: Vec<String> = app.windows().keys().cloned().collect();
                labels.sort();
                let webviews: Vec<String> =
                    host.webviews().iter().map(|w| w.label().to_string()).collect();
                format!("opened={opened} windows()={labels:?} webviews={webviews:?}")
            }
            Err(error) => format!("open_additional_window ERR {error}"),
        };
        let _ = tx.send(report);
    });
    Ok(rx
        .recv_timeout(std::time::Duration::from_secs(12))
        .unwrap_or_else(|e| format!("new window never replied: {e}")))
}

/// Create the wiki webview, blank and hidden, during `attach()`.
///
/// # Why it is created here rather than on first open
///
/// Measured 2026-09-05. `add_child` called from inside a `run_on_main_thread`
/// callback never returns: the invoke stays pending forever, though the app
/// keeps running and the window keeps answering messages. Creating a webview
/// needs the event loop to turn, and a main-thread callback is itself running
/// on that loop.
///
/// `attach()` during `setup` works because the loop has not started. `attach()`
/// on a `--multi-window` clone also works, when called from the same worker
/// thread that built the window (measured 2026-09-05 02:56: `pake-1` got
/// sidebar+wiki+About). Do not move `add_child` into `run_on_main_thread`.
///
/// So the wiki is built up-front, pointed at `about:blank` and hidden. Opening
/// it the first time only navigates it, which is cheap and safe from any thread.
///
/// # Why a webview rather than an iframe
///
/// The wiki is a REAL PAGE at gbf.wiki. That removes two whole features `main`
/// needs:
///
///   - **No link interception.** Links in the wiki are just links; they
///     navigate the wiki's own webview and cannot touch the game. On `main` the
///     panel is an iframe inside Granblue's document, so every click has to be
///     inspected to stop it hijacking the game's page.
///   - **No cross-origin problems**, and the wiki keeps its own history, scroll
///     position and session -- across being closed, and across the game
///     reloading beside it.
fn create_wiki(host: &Window, sp: &Split) -> tauri::Result<()> {
    let label = wiki_label(host.label());
    let blank = Url::parse("about:blank").expect("about:blank parses");
    host.add_child(
        WebviewBuilder::new(&label, WebviewUrl::External(blank))
            .initialization_script(include_str!("../inject/gbf-keys.js")),
        LogicalPosition::new(sp.game_w, 0.0),
        LogicalSize::new(WIKI_W, sp.height),
    )?;
    if let Some(w) = wiki_webview(host) {
        let _ = w.hide();
    }
    Ok(())
}

fn create_about(host: &Window, sp: &Split) -> tauri::Result<()> {
    let label = about_label(host.label());
    host.add_child(
        WebviewBuilder::new(&label, WebviewUrl::App("gbf-about.html".into()))
            .initialization_script(include_str!("../inject/gbf-keys.js")),
        LogicalPosition::new(sp.game_w, 0.0),
        LogicalSize::new(ABOUT_W, sp.height),
    )?;
    if let Some(w) = about_webview(host) {
        let _ = w.hide();
    }
    Ok(())
}

fn create_options(host: &Window, sp: &Split) -> tauri::Result<()> {
    let label = options_label(host.label());
    host.add_child(
        WebviewBuilder::new(&label, WebviewUrl::App("gbf-options.html".into()))
            .initialization_script(include_str!("../inject/gbf-keys.js")),
        LogicalPosition::new(sp.game_w, 0.0),
        LogicalSize::new(ABOUT_W, sp.height),
    )?;
    if let Some(w) = options_webview(host) {
        let _ = w.hide();
    }
    Ok(())
}

fn apply_panel_lock(host: &Window, opening: bool) -> String {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    if opening {
        if !state.is_locked(&label) {
            state.set_lock_was_auto(&label, true);
            return format!("{}\n", set_lock(host, true));
        }
        return String::new();
    }
    if !state.panel_is_open(&label) && state.lock_was_auto(&label) {
        state.set_lock_was_auto(&label, false);
        return format!("{}\n", set_lock(host, false));
    }
    String::new()
}

/// Open or close the wiki panel. Returns a narrated report.
///
/// Created lazily on first open, then kept and hidden -- so the page you were
/// reading, its scroll position and its history all survive closing and
/// reopening, and survive the game reloading beside it. Opening About while
/// the wiki is up switches the column; the wiki webview stays loaded.
#[tauri::command]
pub fn gbf_wiki_toggle(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let was_open = app.state::<SidebarState>().wiki_is_open(&label);

    if was_open {
        app.state::<SidebarState>().set_wiki_open(&label, false);
        app.state::<SidebarState>().restore_collapse_for_panel(&label);
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let handle = app.clone();
        let win_label = label.clone();
        app.run_on_main_thread(move || {
            let report = match handle.get_window(&win_label) {
                Some(host) => {
                    restore_panel_width(&host);
                    let mut out = apply_panel_lock(&host, false);
                    out.push_str(&layout_verbose(&host));
                    out
                }
                None => format!("get_window({win_label}) -> None"),
            };
            let _ = tx.send(report);
        })
        .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
        let report = rx
            .recv_timeout(std::time::Duration::from_secs(8))
            .unwrap_or_else(|e| format!("main thread never replied: {e}"));
        return Ok(format!("wikiOpen=false\n{report}"));
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => apply_panel_lock(&host, true),
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
    let lock_report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    let _ = wait_for_game_edge(&app, &label);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => match prepare_wiki_open(&host) {
                Ok(note) => {
                    handle.state::<SidebarState>().set_wiki_open(&win_label, true);
                    let mut out = lock_report.clone();
                    out.push_str(&note);
                    if let Some(w) = wiki_webview(&host) {
                        let blank = w.url().map(|u| u.scheme() == "about").unwrap_or(true);
                        if blank {
                            let url = Url::parse(WIKI_URL).expect("wiki url parses");
                            out.push_str(&format!("navigate={}\n", result_word(&w.navigate(url))));
                        }
                    } else {
                        out.push_str("wiki webview MISSING\n");
                    }
                    out.push_str(&layout_verbose(&host));
                    out
                }
                Err(notice) => {
                    handle.state::<SidebarState>().restore_collapse_for_panel(&win_label);
                    let mut out = lock_report.clone();
                    out.push_str(&apply_panel_lock(&host, false));
                    out.push_str("REFUSE ");
                    out.push_str(&notice);
                    out.push('\n');
                    out
                }
            },
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    let opened = app.state::<SidebarState>().wiki_is_open(&label);
    if !opened {
        return Err(report.lines().find(|l| l.starts_with("REFUSE ")).map(|l| l[7..].to_string()).unwrap_or_else(|| panel_no_room_notice(true)));
    }
    Ok(format!("wikiOpen=true\n{report}"))
}

/// Open or close the About page. Own webview, so it cannot destroy the wiki.
/// Same two-tier grow as the wiki: 470 preferred, 300 fallback, refuse if
/// even 300 will not fit. Placement uses the same inside/outside flag.
#[tauri::command]
pub fn gbf_about_toggle(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let was_open = app.state::<SidebarState>().about_is_open(&label);

    if was_open {
        app.state::<SidebarState>().set_about_open(&label, false);
        app.state::<SidebarState>().restore_collapse_for_panel(&label);
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let handle = app.clone();
        let win_label = label.clone();
        app.run_on_main_thread(move || {
            let report = match handle.get_window(&win_label) {
                Some(host) => {
                    restore_panel_width(&host);
                    let mut out = apply_panel_lock(&host, false);
                    out.push_str(&layout_verbose(&host));
                    out
                }
                None => format!("get_window({win_label}) -> None"),
            };
            let _ = tx.send(report);
        })
        .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
        let report = rx
            .recv_timeout(std::time::Duration::from_secs(8))
            .unwrap_or_else(|e| format!("main thread never replied: {e}"));
        return Ok(format!("aboutOpen=false\n{report}"));
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => apply_panel_lock(&host, true),
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
    let lock_report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    let _ = wait_for_game_edge(&app, &label);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => match prepare_about_open(&host) {
                Ok(note) => {
                    handle.state::<SidebarState>().set_about_open(&win_label, true);
                    let mut out = lock_report.clone();
                    out.push_str(&note);
                    out.push_str(&layout_verbose(&host));
                    out
                }
                Err(notice) => {
                    handle.state::<SidebarState>().restore_collapse_for_panel(&win_label);
                    let mut out = lock_report.clone();
                    out.push_str(&apply_panel_lock(&host, false));
                    out.push_str("REFUSE ");
                    out.push_str(&notice);
                    out.push('\n');
                    out
                }
            },
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    let opened = app.state::<SidebarState>().about_is_open(&label);
    if !opened {
        return Err(report.lines().find(|l| l.starts_with("REFUSE ")).map(|l| l[7..].to_string()).unwrap_or_else(|| panel_no_room_notice(true)));
    }
    Ok(format!("aboutOpen=true\n{report}"))
}

/// Open or close the Options page. Same panel path as About (470 / 300).
#[tauri::command]
pub fn gbf_options_toggle(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let was_open = app.state::<SidebarState>().options_is_open(&label);

    if was_open {
        app.state::<SidebarState>().set_options_open(&label, false);
        app.state::<SidebarState>().restore_collapse_for_panel(&label);
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let handle = app.clone();
        let win_label = label.clone();
        app.run_on_main_thread(move || {
            let report = match handle.get_window(&win_label) {
                Some(host) => {
                    restore_panel_width(&host);
                    let mut out = apply_panel_lock(&host, false);
                    out.push_str(&layout_verbose(&host));
                    out
                }
                None => format!("get_window({win_label}) -> None"),
            };
            let _ = tx.send(report);
        })
        .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
        let report = rx
            .recv_timeout(std::time::Duration::from_secs(8))
            .unwrap_or_else(|e| format!("main thread never replied: {e}"));
        return Ok(format!("optionsOpen=false\n{report}"));
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => apply_panel_lock(&host, true),
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
    let lock_report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    let _ = wait_for_game_edge(&app, &label);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => match prepare_options_open(&host) {
                Ok(note) => {
                    handle.state::<SidebarState>().set_options_open(&win_label, true);
                    let mut out = lock_report.clone();
                    out.push_str(&note);
                    out.push_str(&layout_verbose(&host));
                    out
                }
                Err(notice) => {
                    handle.state::<SidebarState>().restore_collapse_for_panel(&win_label);
                    let mut out = lock_report.clone();
                    out.push_str(&apply_panel_lock(&host, false));
                    out.push_str("REFUSE ");
                    out.push_str(&notice);
                    out.push('\n');
                    out
                }
            },
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    let opened = app.state::<SidebarState>().options_is_open(&label);
    if !opened {
        return Err(report.lines().find(|l| l.starts_with("REFUSE ")).map(|l| l[7..].to_string()).unwrap_or_else(|| panel_no_room_notice(true)));
    }
    Ok(format!("optionsOpen=true\n{report}"))
}

/// Step the wiki's own history back. It has real history because it is a real
/// webview.
#[tauri::command]
pub fn gbf_wiki_back(window: Window) -> Result<(), String> {
    let wiki = wiki_webview(&window).ok_or_else(|| "the wiki is not open".to_string())?;
    wiki.eval("history.back()").map_err(|e| e.to_string())
}

/// Send the wiki back to its front page.
#[tauri::command]
pub fn gbf_wiki_home(window: Window) -> Result<(), String> {
    let wiki = wiki_webview(&window).ok_or_else(|| "the wiki is not open".to_string())?;
    let url = Url::parse(WIKI_URL).map_err(|e| e.to_string())?;
    wiki.navigate(url).map_err(|e| e.to_string())
}

/// Locked mode: hide Granblue's chat column so the wiki can use that space.
///
/// # Rule 0
///
/// This is **exception 2** in `GBF_Pake_RULES_AND_HANDOFF.md`, ported here
/// unchanged, and it stays inside the same boundary:
///
///   - it only ever adds or removes ONE `<style>` element of our own
///   - it sets `display` and nothing else
///   - **Granblue's own nodes are never touched, moved or detached**
///   - it is instantly and completely reversible
///
/// It is the only thing in this build that writes to the game's document at
/// all, and it exists for the same reason it does on `main`: without it the
/// wiki has to take its width from the game, which pushes Granblue below what
/// its own Window Size needs and clips this very column.
fn lock_js(on: bool) -> String {
    format!(
        "(function(){{var id={id:?};var el=document.getElementById(id);         if({on}){{if(!el){{el=document.createElement('style');el.id=id;         el.textContent={css:?};(document.head||document.documentElement).appendChild(el);}}}}         else if(el&&el.parentNode){{el.parentNode.removeChild(el);}}}})()",
        id = LOCK_STYLE_ID,
        on = on,
        css = LOCK_SELECTOR,
    )
}

fn hug_js(on: bool) -> String {
    format!("window.__gbfSetHug && window.__gbfSetHug({on})")
}

/// Push lock CSS into Granblue, or drop the stale lock overlay on Steam login
/// and other non-game pages so the sidebar cannot sit on top of them.
pub fn on_game_page_finished(webview: &Webview, url: &Url) {
    let host_name = url.host_str().unwrap_or("");
    let on_gbf = host_name.contains("granbluefantasy");
    if on_gbf {
        reapply_lock(webview);
        return;
    }
    let label = webview.window().label().to_string();
    webview
        .app_handle()
        .state::<SidebarState>()
        .set_game_edge(&label, 0.0);
    if let Some(host) = webview.app_handle().get_window(&label) {
        let _ = layout(&host);
    }
}

/// Push the current lock state into a game webview.
///
/// Called on every page load as well as on toggle, because Granblue rebuilds
/// its document on each navigation and takes our style element with it.
pub fn reapply_lock(webview: &Webview) {
    let label = webview.window().label().to_string();
    let locked = webview.app_handle().state::<SidebarState>().is_locked(&label);
    if let Err(error) = webview.eval(lock_js(locked)) {
        eprintln!("[Pake][gbf] could not reapply locked mode: {error}");
    }
    let _ = webview.eval(hug_js(locked));
}

fn set_lock(host: &Window, on: bool) -> String {
    host.app_handle()
        .state::<SidebarState>()
        .set_locked(host.label(), on);
    if on {
        host.app_handle()
            .state::<SidebarState>()
            .set_auto_hug_allowed(host.label(), true);
        host.app_handle()
            .state::<SidebarState>()
            .set_last_hug_phys_w(host.label(), 0);
        host.app_handle()
            .state::<SidebarState>()
            .set_game_keep_w(host.label(), 0.0);
        host.app_handle()
            .state::<SidebarState>()
            .set_panel_hug_due(host.label(), true);
    }
    let mut extra = String::new();
    if !on {
        host.app_handle()
            .state::<SidebarState>()
            .set_game_keep_w(host.label(), 0.0);
        let _ = host
            .app_handle()
            .state::<SidebarState>()
            .take_hug_saved_w(host.label());
        host.app_handle()
            .state::<SidebarState>()
            .set_panel_hug_due(host.label(), true);
        extra.push_str(" restore=overlay");
    }
    let out = match game_webview(host) {
        Some(game) => {
            let lock = result_word(&game.eval(lock_js(on)));
            // Always burst-report wrapper + overlay after a lock change.
            let hug = result_word(&game.eval(hug_js(true)));
            format!("lock({on})={lock} hug={hug}{extra}")
        }
        None => format!("lock: game webview NOT FOUND{extra}"),
    };
    persist_layout_state(host.app_handle());
    out
}

/// Toggle locked mode by hand. Hides the chat column (exception 2) and snaps
/// the sidebar's left edge to #wrapper. Returns the new state.
#[tauri::command]
pub fn gbf_toggle_lock(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let now = !app.state::<SidebarState>().is_locked(&label);
    app.state::<SidebarState>().set_lock_was_auto(&label, false);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => {
                let mut out = set_lock(&host, now);
                out.push('\n');
                out.push_str(&layout_verbose(&host));
                out
            }
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    Ok(format!("locked={now}\n{report}"))
}

/// Place the wiki/About between the game and the sidebar (`outside=false`) or
/// on the sidebar's outer right edge (`outside=true`, live-client order).
#[tauri::command]
pub fn gbf_set_wiki_outside(window: Window, outside: bool) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    app.state::<SidebarState>().set_wiki_outside(&label, outside);
    persist_layout_state(&app);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = label;
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&win_label) {
            Some(host) => layout_verbose(&host),
            None => format!("get_window({win_label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    Ok(format!("wiki_outside={outside}\n{report}"))
}

/// #wrapper and submenu overlay edges, from the game webview.
/// Locked snaps to wrapper. Unlocked snaps to the overlay (collapsed rail
/// or expanded chat). Hugging happens in `layout`.
#[tauri::command]
pub fn gbf_game_edge(
    window: Window,
    right: f64,
    automatic: bool,
    dpr: Option<f64>,
    overlay: Option<f64>,
) -> Result<(), String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let state = app.state::<SidebarState>();
    let dpr = dpr.filter(|v| *v > 0.05).unwrap_or(0.0);
    let overlay = overlay.unwrap_or(0.0);
    if right <= 1.0 {
        // Reload removes #wrapper for a moment. Keep the last edge instead
        // of snapping the sidebar to x=0 / tiling the game to Small.
        return Ok(());
    }
    let same_edge = (state.game_edge(&label) - right).abs() < 0.5;
    let same_overlay = (state.game_overlay(&label) - overlay).abs() < 0.5;
    let same_dpr = dpr <= 0.05 || (state.game_dpr(&label) - dpr).abs() < 0.01;
    let same_auto = state.is_automatic(&label) == automatic;
    let dpr_first = state.game_dpr(&label) <= 0.05 && dpr > 0.05;
    let overlay_changed = !same_overlay;
    state.set_automatic(&label, automatic);
    if dpr > 0.05 {
        state.set_game_dpr(&label, dpr);
    }
    if same_edge && same_overlay && same_auto && same_dpr {
        return Ok(());
    }
    state.set_game_edge(&label, right);
    state.set_game_overlay(&label, overlay);
    if dpr_first || overlay_changed {
        state.set_panel_hug_due(&label, true);
    }
    let handle = app.clone();
    let win_label = label;
    app.run_on_main_thread(move || {
        if let Some(host) = handle.get_window(&win_label) {
            let _ = layout(&host);
        }
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
    Ok(())
}

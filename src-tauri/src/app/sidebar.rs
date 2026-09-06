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
    utils::config::Color, webview::WebviewBuilder, AppHandle, LogicalPosition, LogicalSize, Manager,
    PhysicalSize, Url, Webview, WebviewUrl, WebviewWindow, Window, WindowEvent,
};

/// Expanded width, matching `SIDEBAR_W` in gbf-scaler.js so the two builds are
/// visually comparable.
pub const SIDEBAR_W: f64 = 250.0;
/// Collapsed rail width, matching `SIDEBAR_W_COLLAPSED` in gbf-scaler.js.
pub const SIDEBAR_W_COLLAPSED: f64 = 52.0;
/// Granblue serves an entirely different, lighter client on this user agent.
/// It has no `#submenu` chat column, lays out at a fixed 320 CSS px, and never
/// re-fits or reloads when the viewport changes -- see
/// `GBF_Pake_MOBILE_CLIENT_NOTES.md`. This is the default client.
///
/// Exception 6: this string asks the server for that client. It is not a page
/// inject and not a Rule 0 traffic rewrite. On Windows it is the HTTP document
/// header only; `navigator.userAgent` stays the desktop Chrome string from
/// `pake.json` (Thorium / Speed Tweaks), so Menu opens `#setting/pc`.
pub const MOBILE_USER_AGENT: &str =
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_7_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Mobile/15E148 Safari/604.1";

/// Shown when a panel is asked for while the desktop client is on Automatic
/// Resizing. That mode is a bare window: no sidebar, no panels.
/// The `NOTICE ` prefix tells the sidebar page to show this in its banner
/// rather than dumping it into the diagnostics pane.
const PANEL_NEEDS_FIXED_SIZE: &str =
    "NOTICE Wiki, Options and About are unavailable while the desktop client is on Automatic Resizing.";

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

/// The colour anything of ours shows before it has content -- above all the
/// HOST WINDOW's own background, which is what the panel-switch flash was.
///
/// # What the flash actually is
///
/// Shrinking the frame repaints the window's background in the strip where
/// the game and the sidebar meet, before the two webviews composite over it
/// again. Windows' default is WHITE, so it read as a white band flashing over
/// the right of the game for one to three frames.
///
/// # How that was established
///
/// By painting every surface a different colour and switching panels four
/// times. **Magenta -- the window background -- appeared in the band. Red
/// (wiki), green (game), blue (sidebar) and yellow (panel) never did.** So it
/// is not a webview's blank frame, and it is not the wiki being pushed
/// across: it is the parent window showing through a seam its children have
/// not covered yet.
///
/// # What does NOT work, so nobody retries it
///
/// - `SWP_NOREDRAW` on the resize: no change, still flashed.
/// - `WM_SETREDRAW` freeze plus one `RedrawWindow`: worse, the whole strip
///   went white instead of part of it.
/// - Colouring only the webviews: the band stayed white, which is what
///   pointed at the window itself.
///
/// Skipping the resize entirely removed it, which is what proved the resize
/// is the trigger. Wiki → About/Options now keeps the frame width and lets
/// the incoming panel cover the leftover, so that shrink does not run.
/// Closing a panel still hugs; the near-black background is for that path.
///
/// Granblue's page and the sidebar are both near-black, so a near-black
/// window background makes a remaining close-shrink repaint invisible.
///
/// Chrome themes retint this so a remaining shrink still matches the rail,
/// not a leftover midnight strip on Ember/Tide.
const WEBVIEW_BLANK: Color = Color(11, 11, 15, 255);

fn normalize_theme(theme: &str) -> String {
    match theme.trim().to_ascii_lowercase().as_str() {
        "ember" => "ember".into(),
        "tide" => "tide".into(),
        _ => "midnight".into(),
    }
}

fn theme_blank(theme: &str) -> Color {
    match normalize_theme(theme).as_str() {
        "ember" => Color(18, 12, 8, 255),
        "tide" => Color(8, 14, 16, 255),
        _ => WEBVIEW_BLANK,
    }
}

fn theme_js(theme: &str) -> String {
    serde_json::to_string(&normalize_theme(theme)).unwrap_or_else(|_| "\"midnight\"".into())
}

fn theme_boot_script(host: &Window) -> String {
    let theme = host.app_handle().state::<SidebarState>().theme();
    format!(
        "document.documentElement.setAttribute('data-theme', {});",
        theme_js(&theme)
    )
}

fn apply_theme_blank(host: &Window) {
    let theme = host.app_handle().state::<SidebarState>().theme();
    let color = theme_blank(&theme);
    let _ = host.set_background_color(Some(color));
    for wv in [
        game_webview(host),
        sidebar_webview(host),
        wiki_webview(host),
        panel_webview(host),
    ]
    .into_iter()
    .flatten()
    {
        let _ = wv.set_background_color(Some(color));
    }
}

/// One sidebar per window, so `--multi-window` keeps working: each window gets
/// its own game webview and its own sidebar beside it.
pub fn sidebar_label(window_label: &str) -> String {
    format!("{window_label}--gbf-sidebar")
}

/// The wiki gets its own webview too, one per window.
pub fn wiki_label(window_label: &str) -> String {
    format!("{window_label}--gbf-wiki")
}

/// True for the GAME webview, false for the sidebar, wiki and panel.
///
/// Every child we add is `<window>--gbf-<what>`, and the game webview is the
/// one whose label IS the window label. This matters because a check of the
/// shape `label.starts_with("pake-")` -- meant to catch `--multi-window`
/// clones like `pake-1` -- also matches `pake--gbf-wiki`, and treating one of
/// our own panels as the game is how the panel-switch flash happened: see
/// `on_game_page_finished`.
pub fn is_game_label(label: &str) -> bool {
    !label.contains("--gbf-")
}

/// About and Options SHARE one webview.
///
/// They can never be open at once -- opening either closes the other -- they
/// use the same width tier, and both are static local pages with no scroll
/// position, history or session to lose. Two webviews for that cost two
/// renderer processes, about 55 MB each measured 2026-09-05, for one visible
/// panel. So there is one, navigated between the two pages.
///
/// The wiki keeps its own: it is a real site whose article, scroll position
/// and history are worth keeping across a close.
pub fn panel_label(window_label: &str) -> String {
    format!("{window_label}--gbf-panel")
}

/// The two pages the shared panel switches between.
const ABOUT_PAGE: &str = "gbf-about.html";
const OPTIONS_PAGE: &str = "gbf-options.html";

/// Separate from `.window-state.json` so extra keys cannot break the plugin's
/// restore parser. Per-window, same as SidebarState.
const LAYOUT_STATE_FILE: &str = "gbf-layout.json";

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct SavedLayout {
    locked: bool,
    /// Panels sit to the right of the sidebar. False = between game and sidebar.
    /// Default true. Old persist files stored false because that was the old
    /// default and the option was hidden on mobile; `placement_chosen` is how
    /// we tell a real pick from that leftover.
    #[serde(default = "default_wiki_outside")]
    wiki_outside: bool,
    /// True once this build has written placement. Missing/false means use
    /// the new default (right of sidebar), ignoring a leftover wiki_outside
    /// false.
    #[serde(default)]
    placement_chosen: bool,
    /// System tray icon. Process-wide; stored on each window entry.
    /// Default false: no tray, and closing the window quits.
    #[serde(default)]
    tray: bool,
    /// Ask Granblue for its DESKTOP client. Default false = mobile.
    ///
    /// The mobile client is a fixed 320 CSS layout that never re-fits and never
    /// reloads on a resize; the desktop client reloads on every width change
    /// under Automatic Resizing. Mobile is the default for that reason.
    /// Process-wide, like `tray`, because the user agent can only be set when
    /// the webview is built.
    #[serde(default)]
    desktop_client: bool,
    /// Mobile size: false = Default (zoom 2, Granblue's full scale), true =
    /// Half (zoom 1). The mobile client is a fixed 320 CSS layout, so "size"
    /// here is simply the webview zoom we render it at.
    #[serde(default)]
    mobile_half: bool,
    /// Chrome theme for the sidebar, Options, and About. Process-wide.
    /// Missing or unknown = midnight (the original palette).
    #[serde(default)]
    theme: String,
    /// Sidebar footer diagnostics strip. Process-wide. Default false.
    #[serde(default)]
    sidebar_debug: bool,
}

fn default_wiki_outside() -> bool {
    true
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
                placement_chosen: true,
                tray: sidebar.is_tray_enabled(),
                desktop_client: sidebar.is_desktop_client(),
                mobile_half: sidebar.is_mobile_half(),
                theme: sidebar.theme(),
                sidebar_debug: sidebar.is_sidebar_debug(),
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
    let states = load_layout_states(app);
    states
        .get(label)
        .or_else(|| states.values().next())
        .map(|s| {
            // Old files stored false because that was the previous default and
            // the Options control was hidden on mobile. Treat that leftover as
            // the new default (right of sidebar) until the player picks.
            if s.placement_chosen {
                s.wiki_outside
            } else {
                true
            }
        })
        .unwrap_or(true)
}

/// Mobile size. Process-wide, like the client choice.
pub fn restore_layout_mobile_half(app: &AppHandle) -> bool {
    let states = load_layout_states(app);
    states
        .get("pake")
        .map(|s| s.mobile_half)
        .or_else(|| states.values().next().map(|s| s.mobile_half))
        .unwrap_or(false)
}

/// Which Granblue client to request. Process-wide, like the tray flag.
/// Default false = mobile, which is the one that does not reload on resize.
pub fn restore_layout_desktop_client(app: &AppHandle) -> bool {
    let states = load_layout_states(app);
    states
        .get("pake")
        .map(|s| s.desktop_client)
        .or_else(|| states.values().next().map(|s| s.desktop_client))
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

/// Chrome theme. Process-wide, like the tray flag. Default midnight.
pub fn restore_layout_theme(app: &AppHandle) -> String {
    let states = load_layout_states(app);
    let raw = states
        .get("pake")
        .map(|s| s.theme.as_str())
        .or_else(|| states.values().next().map(|s| s.theme.as_str()))
        .unwrap_or("");
    normalize_theme(raw)
}

/// Sidebar footer dump. Process-wide, like the tray flag. Default off.
pub fn restore_layout_sidebar_debug(app: &AppHandle) -> bool {
    let states = load_layout_states(app);
    states
        .get("pake")
        .map(|s| s.sidebar_debug)
        .or_else(|| states.values().next().map(|s| s.sidebar_debug))
        .unwrap_or(false)
}

#[derive(Default, Clone, Copy)]
struct WindowFlags {
    collapsed: bool,
    wiki_open: bool,
    about_open: bool,
    options_open: bool,
    /// True = panel between the game and the sidebar. Default false = the
    /// panel sits to the right of the sidebar so the rail stays flush.
    wiki_inside: bool,
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
    /// Last Game.getZoom() from the game webview. 0 = unknown.
    game_zoom: f64,
    /// True between our set_size and the Resized layout that follows it.
    hug_busy: bool,
    /// Cancels a delayed About/Options slim-hug if the player already moved on.
    panel_fit_gen: u64,
    /// Lock CSS last pushed into the game page. `None` = unknown, which is
    /// what a page load leaves it as: Granblue rebuilds its document and
    /// takes our style element with it.
    lock_css: Option<bool>,
    /// GBF Automatic Resizing (`mobage_fixwindowsize === 0`).
    automatic: bool,
    /// Granblue served its mobile client (no `#submenu` column). The mobile
    /// client is a fixed 320 CSS layout that never re-fits and never reloads
    /// on resize, and it has no Window Size settings at all.
    mobile: bool,
    /// Page zoom we last applied to the game webview. 1.0 = untouched.
    page_zoom: f64,
    /// Physical inner width of the last resize WE performed. Its `Resized`
    /// event must not be mistaken for the player dragging the frame -- doing so
    /// marks the panel width-borrow as user-owned and it is never given back.
    last_set_phys_w: u32,
    /// Wiki panel width in use (960 or 800). 0 = closed / unset.
    wiki_panel_w: f64,
    /// Inner width before we grew the window for the wiki. 0 = none.
    panel_before_w: f64,
    /// Inner width we left after growing for the wiki. 0 = none.
    panel_after_w: f64,
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
    /// Process-wide. Which Granblue client to ask for. False = mobile.
    desktop_client: AtomicBool,
    /// Process-wide. Mobile size: false = Default (zoom 2), true = Half (zoom 1).
    mobile_half: AtomicBool,
    /// Process-wide. Sidebar / Options / About palette. Empty = midnight.
    theme: Mutex<String>,
    /// Sidebar footer diagnostics. Process-wide. Default off.
    sidebar_debug: AtomicBool,
    /// Last `location.hash` read from the game webview. RAM only; used to
    /// highlight the matching sidebar nav row (same longest-prefix rule as
    /// shipping `markActive`).
    game_hash: Mutex<HashMap<String, String>>,
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

    pub fn is_desktop_client(&self) -> bool {
        self.desktop_client.load(Ordering::Relaxed)
    }

    pub fn set_desktop_client(&self, on: bool) {
        self.desktop_client.store(on, Ordering::Relaxed);
    }

    pub fn is_mobile_half(&self) -> bool {
        self.mobile_half.load(Ordering::Relaxed)
    }

    pub fn set_mobile_half(&self, on: bool) {
        self.mobile_half.store(on, Ordering::Relaxed);
    }

    pub fn theme(&self) -> String {
        let guard = self.theme.lock().unwrap_or_else(|e| e.into_inner());
        normalize_theme(guard.as_str())
    }

    pub fn set_theme(&self, theme: &str) {
        let mut guard = self.theme.lock().unwrap_or_else(|e| e.into_inner());
        *guard = normalize_theme(theme);
    }

    pub fn is_sidebar_debug(&self) -> bool {
        self.sidebar_debug.load(Ordering::Relaxed)
    }

    pub fn set_sidebar_debug(&self, on: bool) {
        self.sidebar_debug.store(on, Ordering::Relaxed);
    }

    fn game_hash(&self, label: &str) -> String {
        let guard = self.game_hash.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(label).cloned().unwrap_or_default()
    }

    /// Returns true when the stored hash changed.
    fn set_game_hash(&self, label: &str, hash: String) -> bool {
        let mut guard = self.game_hash.lock().unwrap_or_else(|e| e.into_inner());
        let changed = guard.get(label).map(|old| old != &hash).unwrap_or(true);
        guard.insert(label.to_string(), hash);
        changed
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
                f.panel_fit_gen = f.panel_fit_gen.wrapping_add(1);
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
            } else {
                f.panel_fit_gen = f.panel_fit_gen.wrapping_add(1);
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
            } else {
                f.panel_fit_gen = f.panel_fit_gen.wrapping_add(1);
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
        self.with(label, |f| !f.wiki_inside)
    }

    pub fn set_wiki_outside(&self, label: &str, on: bool) {
        self.update(label, |f| f.wiki_inside = !on);
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

    pub fn game_zoom(&self, label: &str) -> f64 {
        self.with(label, |f| f.game_zoom)
    }

    pub fn set_game_zoom(&self, label: &str, zoom: f64) {
        self.update(label, |f| f.game_zoom = zoom);
    }

    pub fn is_automatic(&self, label: &str) -> bool {
        self.with(label, |f| f.automatic)
    }

    pub fn is_mobile(&self, label: &str) -> bool {
        self.with(label, |f| f.mobile)
    }

    pub fn set_mobile(&self, label: &str, on: bool) {
        self.update(label, |f| f.mobile = on);
    }

    pub fn page_zoom(&self, label: &str) -> f64 {
        self.with(label, |f| if f.page_zoom > 0.05 { f.page_zoom } else { 1.0 })
    }

    pub fn set_page_zoom(&self, label: &str, z: f64) {
        self.update(label, |f| f.page_zoom = z);
    }

    /// Record a resize we are about to perform, so its echo is not read as a drag.
    fn note_our_resize(&self, label: &str, phys_w: u32) {
        self.update(label, |f| f.last_set_phys_w = phys_w);
    }

    pub fn set_automatic(&self, label: &str, on: bool) {
        self.update(label, |f| {
            f.automatic = on;
        });
    }

    pub fn wiki_panel_w(&self, label: &str) -> f64 {
        self.with(label, |f| f.wiki_panel_w)
    }

    pub fn set_wiki_panel_w(&self, label: &str, w: f64) {
        self.update(label, |f| f.wiki_panel_w = w);
    }

    fn bump_panel_fit_gen(&self, label: &str) -> u64 {
        let mut out = 0;
        self.update(label, |f| {
            f.panel_fit_gen = f.panel_fit_gen.wrapping_add(1);
            out = f.panel_fit_gen;
        });
        out
    }

    fn panel_fit_gen(&self, label: &str) -> u64 {
        self.with(label, |f| f.panel_fit_gen)
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

    fn take_panel_user_resized(&self, label: &str) -> bool {
        let mut out = false;
        self.update(label, |f| {
            out = f.panel_user_resized;
            f.panel_user_resized = false;
        });
        out
    }

    /// A drag while a panel is open means we must not restore the borrowed
    /// width.
    pub fn note_user_resize(&self, label: &str, phys_w: u32) {
        self.update(label, |f| {
            if f.hug_busy {
                return;
            }
            // Our own resize echoing back is not a drag.
            if f.last_set_phys_w != 0 && phys_w.abs_diff(f.last_set_phys_w) <= 8 {
                f.last_set_phys_w = 0;
                return;
            }
            f.last_set_phys_w = 0;
            if f.wiki_open || f.about_open || f.options_open {
                f.panel_user_resized = true;
            }
        });
    }

    pub fn hug_busy(&self, label: &str) -> bool {
        self.with(label, |f| f.hug_busy)
    }

    pub fn set_hug_busy(&self, label: &str, on: bool) {
        self.update(label, |f| f.hug_busy = on);
    }

    /// Record what we pushed, and say whether it was a change. `None` marks
    /// the page's copy as unknown again -- call that on every page load.
    fn note_lock_css(&self, label: &str, on: Option<bool>) -> bool {
        let mut changed = false;
        self.update(label, |f| {
            changed = f.lock_css != on;
            f.lock_css = on;
        });
        changed
    }

    fn collapse_for_panel(&self, label: &str) {
        self.update(label, |f| {
            f.collapsed = true;
            f.collapsed_for_panel = true;
        });
    }

    /// Desktop Automatic is a bare window. Drop every panel flag and the
    /// width-borrow bookkeeping; `layout` hides the webviews from these flags.
    fn close_panels_for_automatic(&self, label: &str) -> bool {
        let mut had = false;
        self.update(label, |f| {
            had = f.wiki_open || f.about_open || f.options_open;
            f.wiki_open = false;
            f.about_open = false;
            f.options_open = false;
            f.wiki_panel_w = 0.0;
            f.panel_before_w = 0.0;
            f.panel_after_w = 0.0;
            f.panel_user_resized = false;
            if f.collapsed_for_panel {
                f.collapsed = false;
                f.collapsed_for_panel = false;
            }
        });
        had
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

fn panel_webview(host: &Window) -> Option<Webview> {
    let label = panel_label(host.label());
    host.webviews().into_iter().find(|w| w.label() == label)
}


/// The window's client area divided between the three webviews.
///
/// Widths only. Left-to-right order is decided in `apply_layout`: default is
/// game | sidebar | panel (flush rail); Options can pick game | panel | sidebar.
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

/// DIAGNOSTIC 2026-09-05: when true, Granblue sizes itself (no webview zoom,
/// no hug/snap, Game size Large/Small hidden). Off again: Large/Small and the
/// hug path are back, while we match Thorium's request-only mobile UA instead.
const GAME_SIZES_ITSELF: bool = false;

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
    if GAME_SIZES_ITSELF {
        return s.game_w;
    }
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label();
    let css = snap_css(&state, label);
    if css <= 1.0 {
        return s.game_w;
    }
    css_to_window_logical(host, css, state.game_dpr(label)).max(0.0)
}

/// Locked: shrink/grow the OS window so its right edge sits on the sidebar.
fn maybe_hug_window(host: &Window, col: f64, s: &Split) -> tauri::Result<bool> {
    if GAME_SIZES_ITSELF {
        return Ok(false);
    }
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    if snap_css(&state, &label) <= 1.0 {
        return Ok(false);
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
    if (inner_w - want_w).abs() <= HUG_SLACK {
        return Ok(false);
    }
    // hug_busy blocks a grow/shrink loop while our set_size echoes. Nested
    // layout during Wiki grow used to hug back to About's width and refuse
    // ("Not enough room", recording 180531).
    if state.hug_busy(&label) {
        return Ok(false);
    }
    let want_phys = (want_w * scale).round() as u32;
    if want_phys == 0 || want_phys == phys.width {
        return Ok(false);
    }
    // A shrink in the SAME layout as Wiki → About/Options is the flash
    // (parent HWND through the game/sidebar seam). Skip that shrink while
    // the panel is still filling the leftover. After the panel has painted,
    // schedule_slim_panel_hug drops reserved to the About/Options tier and
    // hugging is allowed again. Closing a panel has reserved 0 and still hugs.
    if want_phys < phys.width && reserved_panel_w(&state, &label, s.wiki_w) > ABOUT_W + WIKI_TIER_SLACK
    {
        return Ok(false);
    }
    state.set_hug_busy(&label, true);
    state.note_our_resize(&label, want_phys);
    if let Err(e) = host.set_size(PhysicalSize::new(want_phys, phys.height)) {
        state.set_hug_busy(&label, false);
        return Err(e);
    }
    Ok(true)
}

/// The desktop client's Automatic Resizing is a bare window, so panels are
/// refused there. Do not gate this on `mobage_fixwindowsize`: with a Windows
/// navigator the mobile client can show Window Size, and that flag can move.
fn panels_unavailable(app: &AppHandle, label: &str) -> bool {
    native_mode(&app.state::<SidebarState>(), label)
}

/// Desktop client + Automatic Resizing. Nothing of ours runs in the page
/// there: no sidebar, no panels, no lock CSS, and every Rule 0 exception in
/// the injected scripts gates itself off `window.__gbfInert()`. The only
/// thing left is gbf-edge.js's 2s read of `mobage_fixwindowsize`, which is
/// how we notice the player leaving.
///
/// The mobile client is never this mode (`is_mobile` is our chosen client).
/// Matching Thorium, Menu can still open `#setting/pc` with Window Size;
/// that does not flip us into a bare window.
fn native_mode(state: &SidebarState, label: &str) -> bool {
    state.is_automatic(label) && !state.is_mobile(label)
}

/// Render the mobile client at the chosen size.
///
/// The mobile client is a fixed 320 CSS layout, so "size" is simply the webview
/// zoom we render it at: Large = 2 (Granblue's full scale, a 721px game at
/// this display) and Small = 1 (360px). Because the zoom is fixed rather than
/// fitted to the column, `#wrapper` has a stable width, and mobile can use the
/// same layout and hug path as the desktop client's fixed Sizes.
///
/// This is the webview's own zoom -- the same one Pake already applies and a
/// browser's Ctrl+ uses. It is NOT CSS `zoom` injected into the page, so
/// Exception 3's boundary is untouched.
fn apply_mobile_zoom(host: &Window) -> Option<String> {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    if !state.is_mobile(&label) {
        return None;
    }
    let game = game_webview(host)?;
    let target = if GAME_SIZES_ITSELF {
        1.0
    } else if state.is_mobile_half() {
        1.0
    } else {
        2.0
    };
    let current = state.page_zoom(&label);
    if (target - current).abs() < 0.01 {
        return None;
    }
    match game.set_zoom(target) {
        Ok(()) => {
            state.set_page_zoom(&label, target);
            Some(format!("mobile zoom {current:.2} -> {target:.2}\n"))
        }
        Err(e) => Some(format!("mobile zoom ERR {e}\n")),
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

/// Exactly the width the three columns occupy. This is the ONE definition of
/// the frame width when a panel is open, and `maybe_hug_window` targets the
/// same sum -- they must not drift apart.
///
/// They did: this used to add `WIKI_WIDEN_BUFFER` and the hug did not, so
/// opening a panel sized the frame 2px wider than the columns fill while the
/// hug wanted it 2px narrower. 2 is inside `HUG_SLACK`, so the hug never
/// corrected it and the frame simply kept whichever value the last path
/// produced -- leaving 2px of bare window down the right edge, and a 2px
/// resize (with the repaint that goes with it) on every panel switch that
/// crossed between the two paths.
fn panel_span(game_col: f64, panel: f64, collapsed: bool) -> f64 {
    game_col + panel + sidebar_want(collapsed)
}

/// The span plus headroom, for deciding whether a tier FITS. The buffer
/// belongs in the decision, never in the width we actually ask for.
fn needed_for_panel(game_col: f64, panel: f64, collapsed: bool) -> f64 {
    panel_span(game_col, panel, collapsed) + WIKI_WIDEN_BUFFER
}

fn panel_no_room_notice(mobile: bool) -> String {
    if mobile {
        "Not enough room. Widen the window.".into()
    } else {
        "Not enough room. In Granblue's Browser Settings, pick a smaller Window Size.".into()
    }
}

/// Fit the OS window to `want` logical inner width. Height is echoed so it
/// cannot drift. Grows when a panel needs room; shrinks leftover when a
/// smaller panel (or a close) no longer needs the wiki's width.
fn fit_inner_width(host: &Window, want: f64, allow_shrink: bool) -> tauri::Result<f64> {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let scale = host.scale_factor()?;
    let phys = host.inner_size()?;
    let inner_w = phys.to_logical::<f64>(scale).width;
    let ceiling = monitor_inner_ceiling(host);
    let target = want.min(ceiling).max(1.0);
    // Same tolerance the hug uses. Without it this resized the frame for a
    // SINGLE pixel of logical<->physical rounding, so switching between two
    // panels of the same tier still moved the window -- and every window
    // resize is a repaint, which is the flash. The hug ignores anything
    // under HUG_SLACK, so a resize under it can only ever undo itself.
    if (inner_w - target).abs() <= HUG_SLACK {
        return Ok(inner_w);
    }
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
    state.note_our_resize(&label, want_phys);
    // Clear hug_busy ourselves if the resize fails. Only apply_layout's tail
    // clears it, and a failed set_size fires no Resized event to get there --
    // the flag would stay set and silently refuse every later hug.
    if let Err(e) = host.set_size(PhysicalSize::new(want_phys, phys.height)) {
        state.set_hug_busy(&label, false);
        return Err(e);
    }
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

fn restore_panel_width(host: &Window) {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let (before, _after) = state.take_panel_restore(&label);
    let user_resized = state.take_panel_user_resized(&label);
    if user_resized {
        return;
    }
    if before > 1.0 {
        let Ok(scale) = host.scale_factor() else {
            return;
        };
        let Ok(phys) = host.inner_size() else {
            return;
        };
        let want_phys = (before * scale).round() as u32;
        if want_phys != 0 && want_phys != phys.width {
            state.set_hug_busy(&label, true);
            state.note_our_resize(&label, want_phys);
            let _ = host.set_size(PhysicalSize::new(want_phys, phys.height));
        }
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
    let mobile = state.is_mobile(&label);
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
        return Err(panel_no_room_notice(mobile));
    };

    let mut note = String::new();
    if need_collapse {
        state.collapse_for_panel(&label);
        note.push_str(&format!("Sidebar collapsed to make room for the {name}.\n"));
    }
    // Undo the collapse we forced. Every failure past this point has to run
    // it, not just the reserved-too-small one: leaving the rail collapsed
    // after a panel that never opened is a state the player did not ask for.
    let undo_collapse = |state: &SidebarState| {
        if need_collapse {
            state.update(&label, |f| {
                f.collapsed = false;
                f.collapsed_for_panel = false;
            });
        }
    };
    // The span, not the span+buffer: the buffer is for the fits() decision
    // above. Asking for it here is what left the 2px gap.
    let want = panel_span(game_col, chosen, collapsed || need_collapse);
    // Grow only. Shrinking here ran while the previous panel was still the
    // open flag, so Wiki → About resized the HWND with the wiki still on
    // screen. That is the parent-background flash (05ay) and what read as
    // the wiki jumping into the game. Opening from a hugged window still
    // grows; switching from a wider panel keeps the leftover and covers it.
    let after = match fit_inner_width(host, want, false) {
        Ok(v) => v,
        Err(e) => {
            undo_collapse(&state);
            return Err(e.to_string());
        }
    };
    let bar = sidebar_want(collapsed || need_collapse);
    let space = (after - game_col - bar).max(0.0);
    // Prefer the tier when the frame was grown to it. If leftover is already
    // larger (Wiki → About), keep that leftover for the FIRST paint so the
    // incoming panel covers the wiki column. schedule_slim_panel_hug then
    // hugs to the About/Options tier once they are on screen.
    let reserved = if space >= prefer - WIKI_TIER_SLACK {
        if space > prefer + WIKI_TIER_SLACK {
            space
        } else {
            prefer
        }
    } else if space >= minimum {
        minimum
    } else {
        0.0
    };
    if reserved < minimum {
        undo_collapse(&state);
        restore_panel_width(host);
        return Err(panel_no_room_notice(mobile));
    }
    state.set_wiki_panel_w(&label, reserved);
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

/// Hug About/Options to their own width after they have painted.
///
/// Wiki → About/Options must not shrink in the same layout as the switch
/// (that is the parent-background flash; confirmed in play). Once the new
/// panel is on screen the window can hug down to 470. A later shrink can
/// still show a dark seam — not the wiki jumping into the game.
const PANEL_SLIM_AFTER: std::time::Duration = std::time::Duration::from_millis(200);

fn schedule_slim_panel_hug(host: &Window) {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    if !(state.about_is_open(&label) || state.options_is_open(&label)) {
        return;
    }
    if state.wiki_panel_w(&label) <= ABOUT_W + WIKI_TIER_SLACK {
        return;
    }
    let gen = state.bump_panel_fit_gen(&label);
    let app = host.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(PANEL_SLIM_AFTER);
        let app_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            let state = app_main.state::<SidebarState>();
            if state.panel_fit_gen(&label) != gen {
                return;
            }
            if state.wiki_is_open(&label) {
                return;
            }
            if !(state.about_is_open(&label) || state.options_is_open(&label)) {
                return;
            }
            let Some(host) = app_main.get_window(&label) else {
                return;
            };
            state.set_wiki_panel_w(&label, ABOUT_W);
            let _ = layout(&host);
        });
    });
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
    let desktop_client = state.is_desktop_client();
    let mobile = state.is_mobile(&label);
    let mobile_half = state.is_mobile_half();
    let theme = state.theme();
    let theme_js = theme_js(&theme);
    let sidebar_debug = state.is_sidebar_debug();
    let hash_js = serde_json::to_string(&state.game_hash(&label)).unwrap_or_else(|_| "\"\"".into());
    let s = split(host, collapsed, wiki_open, about_open, options_open)?;
    if s.height <= 0.0 {
        state.set_hug_busy(&label, false);
        return Ok("layout: minimized\n".into());
    }
    // DESKTOP client + Automatic Resizing: hand the whole window to Granblue
    // and get out of the way. No sidebar, no panels, no hugs, no zoom.
    //
    // This is the mode every hug, settle timer and walk-down in this file was
    // written for, and none of them worked: the desktop client re-fits to its
    // viewport and reloads on every width change, so anything we place beside
    // it either strands a dead strip or fights the player's drag. The sidebar
    // is available on the fixed Sizes, and on the mobile client, which does not
    // re-fit at all. See GBF_Pake_HANDOFF_2026-09-05af.md.
    if automatic && !mobile {
        if let Some(bar) = sidebar_webview(host) {
            let _ = bar.hide();
        }
        for panel in [wiki_webview(host), panel_webview(host)] {
            if let Some(p) = panel {
                let _ = p.hide();
            }
        }
        state.close_panels_for_automatic(&label);
        if let Some(game) = game_webview(host) {
            let full = host
                .scale_factor()
                .ok()
                .and_then(|sc| host.inner_size().ok().map(|p| p.to_logical::<f64>(sc).width))
                .unwrap_or(s.game_w);
            let _ = game.set_position(LogicalPosition::new(0.0, 0.0));
            let _ = game.set_size(LogicalSize::new(full.max(1.0), s.height));
        }
        state.set_hug_busy(&label, false);
        sync_lock_css(host);
        return Ok(format!(
            "layout: desktop client on Automatic -- bare window, sidebar hidden\n"
        ));
    }

    let col = column_x(host, &s);
    let edge = state.game_edge(&label);
    let want_w = col + s.wiki_w + s.sidebar_w;

    let mut out = format!(
        "want game={:.0} panel={:.0} bar={:.0} col={col:.0} edge={edge:.0} locked={locked} auto={automatic} outside={outside} hug={want_w:.0} h={:.0} wiki={wiki_open} about={about_open} options={options_open}\n",
        s.game_w, s.wiki_w, s.sidebar_w, s.height
    );

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
        let want = col + s.wiki_w + s.sidebar_w;
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
    let lock_fill = !GAME_SIZES_ITSELF && locked && edge > 1.0;
    let unlock_snap = !GAME_SIZES_ITSELF && !locked && snap_css(&state, &label) > 1.0;
    let panel_open = wiki_open || about_open || options_open;
    // Unlocked: tile to the submenu overlay so Chat/Settings is flush with the
    // sidebar. A panel is always a sibling column.
    // The wiki keeps its webview once created, so a closed panel is hidden
    // rather than destroyed. That is the point of it being its own webview:
    // your page, scroll position and history survive being closed and
    // reopened, and survive the game reloading beside it.
    let panel_w = if (wiki_open || about_open || options_open) && s.wiki_w > 0.0 {
        s.wiki_w
    } else {
        0.0
    };
    // Park unused panels before the game/sidebar move. A shrink with the wiki
    // HWND still in the frame is what read as the wiki flashing into the game.
    if !wiki_open {
        if let Some(wiki) = wiki_webview(host) {
            out.push_str(&format!("wiki hide={}\n", hide_panel(&wiki)));
        }
    }
    if !about_open && !options_open {
        if let Some(panel) = panel_webview(host) {
            out.push_str(&format!("panel hide={}\n", hide_panel(&panel)));
        }
    }
    let game_w = if (lock_fill || unlock_snap) && panel_open {
        col.max(1.0)
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
            if let Some(note) = apply_mobile_zoom(host) {
                out.push_str(&note);
            }
        }
    }

    // Wiki / About / Options: right of the sidebar (outside) or between the
    // game and the sidebar. Default is outside so the rail stays flush on
    // the game.
    let bar_x = if outside {
        col
    } else {
        col + panel_w
    };
    let panel_x = if outside {
        col + s.sidebar_w
    } else {
        col
    };

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
                out.push_str(&format!("wiki hide={}\n", hide_panel(&wiki)));
            }
        }
    }

    // About and Options are one webview showing one of two pages.
    match panel_webview(host) {
        None => out.push_str("panel: not created\n"),
        Some(panel) => {
            if (about_open || options_open) && panel_w > 0.0 {
                let p = panel.set_position(LogicalPosition::new(panel_x, 0.0));
                let z = panel.set_size(LogicalSize::new(panel_w, s.height));
                let v = panel.show();
                out.push_str(&format!(
                    "panel({}) pos={} size={} show={} now={}\n",
                    if about_open { "about" } else { "options" },
                    result_word(&p),
                    result_word(&z),
                    result_word(&v),
                    bounds_word(&panel),
                ));
            } else {
                out.push_str(&format!("panel hide={}\n", hide_panel(&panel)));
            }
            // Theme lands even while About is showing (setAttribute). Options
            // also asks gbf_panel_state on load because this eval can race
            // the About→Options navigation.
            let e = panel.eval(format!(
                "document.documentElement.setAttribute('data-theme', {theme_js}); window.__gbfOptions && window.__gbfOptions.setState({{wikiOutside:{outside},tray:{tray},desktopClient:{desktop_client},mobile:{mobile},theme:{theme_js},sidebarDebug:{sidebar_debug}}})"
            ));
            out.push_str(&format!("panel eval={}\n", result_word(&e)));
        }
    }

    match sidebar_webview(host) {
        None => out.push_str("sidebar webview NOT FOUND\n"),
        Some(bar) => {
            let p = bar.set_position(LogicalPosition::new(bar_x, 0.0));
            let z = bar.set_size(LogicalSize::new(s.sidebar_w, s.height));
            let e = bar.eval(format!(
                "window.__gbfSidebar && window.__gbfSidebar.setState({{collapsed:{collapsed},wikiOpen:{wiki_open},aboutOpen:{about_open},optionsOpen:{options_open},locked:{locked},wikiOutside:{outside},mobile:{mobile},mobileHalf:{mobile_half},theme:{theme_js},gameSizesItself:{GAME_SIZES_ITSELF},sidebarDebug:{sidebar_debug},gameHash:{hash_js}}})"
            ));
            // show() is enough after desktop Automatic hid the rail. hide()
            // then show() blanks WebView2 white on every layout, twice when
            // a panel open both grows the window and relayouts.
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

    // HUG LAST, once every webview already holds its final bounds.
    //
    // This used to run before the placement block. On a SHRINK that meant the
    // frame narrowed while the children still held their old geometry, and
    // Windows repainted the seam between them before we moved anything: a
    // ~250px strip straddling the game/sidebar edge went white for one or two
    // frames. Measured 2026-09-06 at 60fps -- 2 frames on a shrink
    // (Wiki -> About/Options), none on a grow (About -> Wiki), none when no
    // resize happens at all (About <-> Options). Direction is the tell.
    //
    // Placing first means the shrink only ever clips space nothing is using.
    // The grow path above still runs BEFORE placement, which is the correct
    // order in that direction: widen the frame, then fill it.
    //
    // Returning early here is what left the sidebar at its pre-hug x, off the
    // right edge of the smaller window (recording 113543) -- placing first
    // removes that hazard rather than working around it.
    if maybe_hug_window(host, col, &s)? {
        out.push_str("hug: set_size issued\n");
    }

    state.set_hug_busy(&label, false);
    sync_lock_css(host);
    Ok(out)
}

fn hide_panel(wv: &Webview) -> String {
    let h = result_word(&wv.hide());
    // hide() alone leaves the old 1200px wiki HWND in the window. Switching
    // to About/Options then shows Relink through that hole (recording 175247).
    let _ = wv.set_position(LogicalPosition::new(-10000.0, 0.0));
    let _ = wv.set_size(LogicalSize::new(1.0, 1.0));
    h
}

/// Stop the parent HWND painting over its children on a resize.
///
/// WS_CLIPCHILDREN is the GDI-side half of this flash (parent erase under the
/// game/sidebar seam). It does not fix WebView2's out-of-process compositor
/// (that needs skip-resize); it does stop the parent brush from drawing where
/// a child already is.
#[cfg(windows)]
fn clip_host_children(host: &Window) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE, WS_CLIPCHILDREN,
    };
    let Ok(hwnd) = host.hwnd() else {
        return;
    };
    unsafe {
        let style = GetWindowLongPtrW(hwnd.0, GWL_STYLE);
        let _ = SetWindowLongPtrW(hwnd.0, GWL_STYLE, style | WS_CLIPCHILDREN as isize);
    }
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

/// Push the game's current hash to the sidebar so the matching nav row lights
/// up. Read-only: we never write Granblue's location here.
fn push_sidebar_game_hash(host: &Window) {
    let Some(bar) = sidebar_webview(host) else {
        return;
    };
    let hash = host
        .app_handle()
        .state::<SidebarState>()
        .game_hash(host.label());
    let encoded = serde_json::to_string(&hash).unwrap_or_else(|_| "\"\"".into());
    let _ = bar.eval(format!(
        "window.__gbfSidebar && window.__gbfSidebar.setGameHash({encoded})"
    ));
}

/// Add the sidebar webview beside the game and keep it laid out.
/// `gbf-sidebar.html` is bundled from dist at compile time.
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
    let boot = theme_boot_script(&host);
    let builder = WebviewBuilder::new(&label, WebviewUrl::App("gbf-sidebar.html".into()))
        .initialization_script(&boot);

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
    if let Err(error) = create_panel(&host, &sp) {
        eprintln!("[Pake][gbf] could not create the panel webview: {error}");
    }

    // Restore lock before the first layout. Do not call set_lock(false) here:
    // unlock would try to restore a hug width and fight the size we just
    // restored from disk.
    if restore_layout_locked(host.app_handle(), host.label()) {
        let _ = set_lock(&host, true);
    }
    host.app_handle()
        .state::<SidebarState>()
        .set_wiki_outside(
            host.label(),
            restore_layout_wiki_outside(host.app_handle(), host.label()),
        );

    // The host window is the one that matters -- see WEBVIEW_BLANK.
    apply_theme_blank(&host);
    #[cfg(windows)]
    clip_host_children(&host);

    layout(&host)?;
    crate::app::window::persist_window_geometry(host.app_handle());
    persist_layout_state(host.app_handle());

    // The game webview no longer follows the window on its own, so every resize
    // has to re-run the split.
    let on_resize = host.clone();
    host.on_window_event(move |event| match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            let state = on_resize.app_handle().state::<SidebarState>();
            let label = on_resize.label().to_string();
            if let Ok(phys) = on_resize.inner_size() {
                state.note_user_resize(&label, phys.width);
            }
            if let Err(error) = layout(&on_resize) {
                eprintln!("[Pake][gbf] sidebar relayout failed: {error}");
            }
            crate::app::window::schedule_persist_window_geometry(on_resize.app_handle().clone());
        }
        WindowEvent::Moved(_) => {
            crate::app::window::schedule_persist_window_geometry(on_resize.app_handle().clone());
        }
        _ => {}
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
        .map_err(|e| e.to_string())?;
    window
        .app_handle()
        .state::<SidebarState>()
        .set_game_hash(window.label(), hash);
    push_sidebar_game_hash(&window);
    Ok(())
}

/// Game history back. Same as the shipping sidebar's Back: `history.back()`
/// on Granblue's webview, not the sidebar's.
#[tauri::command]
pub fn gbf_game_back(window: Window) -> Result<(), String> {
    let game = game_webview(&window)
        .ok_or_else(|| format!("no game webview labelled '{}'", window.label()))?;
    game.eval("history.back()").map_err(|e| e.to_string())
}

/// Reload Granblue's page. Same as the shipping sidebar's Reload.
#[tauri::command]
pub fn gbf_game_reload(window: Window) -> Result<(), String> {
    let game = game_webview(&window)
        .ok_or_else(|| format!("no game webview labelled '{}'", window.label()))?;
    game.eval("location.reload()").map_err(|e| e.to_string())
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
                if snap_css(&state, &label) <= 1.0 {
                    // Tiled: keep the game column, grow/shrink the OS window
                    // by the rail delta so leftover is not left as empty window.
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
    let mut wv: Vec<String> = window
        .app_handle()
        .webview_windows()
        .keys()
        .cloned()
        .collect();
    wv.sort();
    let mut wins: Vec<String> = window.app_handle().windows().keys().cloned().collect();
    wins.sort();

    let scale = window.scale_factor().unwrap_or(-1.0);
    let phys = window.inner_size().map_err(|e| e.to_string())?;
    let logical = phys.to_logical::<f64>(if scale > 0.0 { scale } else { 1.0 });
    let collapsed = window
        .app_handle()
        .state::<SidebarState>()
        .is_collapsed(window.label());
    let locked = window
        .app_handle()
        .state::<SidebarState>()
        .is_locked(window.label());
    let edge = window
        .app_handle()
        .state::<SidebarState>()
        .game_edge(window.label());
    let overlay_edge = window
        .app_handle()
        .state::<SidebarState>()
        .game_overlay(window.label());
    let automatic = window
        .app_handle()
        .state::<SidebarState>()
        .is_automatic(window.label());
    let hug_busy = window
        .app_handle()
        .state::<SidebarState>()
        .hug_busy(window.label());
    // Last Game.getZoom() we were told about. Nothing acts on it any more --
    // it is here so a probe can see what GBF settled on.
    let game_zoom = window
        .app_handle()
        .state::<SidebarState>()
        .game_zoom(window.label());
    let wiki_open = window
        .app_handle()
        .state::<SidebarState>()
        .wiki_is_open(window.label());
    let about_open = window
        .app_handle()
        .state::<SidebarState>()
        .about_is_open(window.label());
    let options_open = window
        .app_handle()
        .state::<SidebarState>()
        .options_is_open(window.label());
    let wiki_panel = window
        .app_handle()
        .state::<SidebarState>()
        .wiki_panel_w(window.label());
    let wiki_outside = window
        .app_handle()
        .state::<SidebarState>()
        .is_wiki_outside(window.label());
    let tray = window
        .app_handle()
        .state::<SidebarState>()
        .is_tray_enabled();
    let tray_icon = window.app_handle().tray_by_id("pake-tray").is_some();
    let dpr = window
        .app_handle()
        .state::<SidebarState>()
        .game_dpr(window.label());
    let persist = crate::app::window::persisted_window_state_path(window.app_handle())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "none".into());
    let layout_persist = persisted_layout_state_path(window.app_handle())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "none".into());
    let monitor = monitor_inner_ceiling(&window);
    let mobile = window
        .app_handle()
        .state::<SidebarState>()
        .is_mobile(window.label());
    let mobile_half = window
        .app_handle()
        .state::<SidebarState>()
        .is_mobile_half();
    let desktop_client = window
        .app_handle()
        .state::<SidebarState>()
        .is_desktop_client();
    let theme = window.app_handle().state::<SidebarState>().theme();
    let page_zoom = window
        .app_handle()
        .state::<SidebarState>()
        .page_zoom(window.label());
    let game_hash = window
        .app_handle()
        .state::<SidebarState>()
        .game_hash(window.label());
    let sidebar_debug = window
        .app_handle()
        .state::<SidebarState>()
        .is_sidebar_debug();

    let report = format!(
        "window={}\nsidebar={}\nwebviews:\n  {}\nwebview_windows()={wv:?}\nwindows()={wins:?}\nscale={scale:.3}\ndpr={dpr:.3}\nphysical={}x{}\nlogical={:.0}x{:.0}\ncollapsed={collapsed}\nlocked={locked}\nautomatic={automatic}\nmobile={mobile}\nmobile_half={mobile_half}\ndesktop_client={desktop_client}\npage_zoom={page_zoom:.3}\nedge={edge:.0}\noverlay={overlay_edge:.0}\nhug_busy={hug_busy}\ngame_zoom={game_zoom:.3}\nwiki_open={wiki_open}\nabout_open={about_open}\noptions_open={options_open}\nwiki_panel={wiki_panel:.0}\nwiki_outside={wiki_outside}\ntray={tray}\ntray_icon={tray_icon}\ntheme={theme}\ngame_sizes_itself={GAME_SIZES_ITSELF}\ngame_hash={game_hash}\nsidebar_debug={sidebar_debug}\nmonitor={monitor:.0}\npersist={persist}\nlayout_persist={layout_persist}",
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
                let webviews: Vec<String> = host
                    .webviews()
                    .iter()
                    .map(|w| w.label().to_string())
                    .collect();
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
        let theme = host.app_handle().state::<SidebarState>().theme();
        let _ = w.set_background_color(Some(theme_blank(&theme)));
        let _ = w.hide();
    }
    Ok(())
}

fn create_panel(host: &Window, sp: &Split) -> tauri::Result<()> {
    let label = panel_label(host.label());
    let keys = include_str!("../inject/gbf-keys.js");
    let boot = theme_boot_script(host);
    let init = format!("{keys}\n{boot}");
    host.add_child(
        WebviewBuilder::new(&label, WebviewUrl::App(ABOUT_PAGE.into()))
            .initialization_script(&init),
        LogicalPosition::new(sp.game_w, 0.0),
        LogicalSize::new(ABOUT_W, sp.height),
    )?;
    if let Some(w) = panel_webview(host) {
        let theme = host.app_handle().state::<SidebarState>().theme();
        let _ = w.set_background_color(Some(theme_blank(&theme)));
        let _ = w.hide();
    }
    Ok(())
}

/// Point the shared panel at one of its two pages.
///
/// The target URL is derived from the panel's own current URL rather than
/// built by hand: the app scheme differs by platform and config, and the two
/// pages are siblings, so `join` is exact where a hardcoded `tauri://` guess
/// would not be.
fn show_panel_page(host: &Window, page: &str) -> String {
    let Some(w) = panel_webview(host) else {
        return format!("panel: not created\n");
    };
    let Ok(here) = w.url() else {
        return format!("panel url unreadable\n");
    };
    if here.path().ends_with(page) {
        return String::new();
    }
    match here.join(page) {
        Ok(u) => format!("panel -> {page} {}\n", result_word(&w.navigate(u))),
        Err(e) => format!("panel url join ERR {e}\n"),
    }
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

    // Desktop Automatic is a bare window. Opening is refused with a notice;
    // closing still works so a panel left open by a mode switch can shut.
    if !was_open && panels_unavailable(&app, &label) {
        return Ok(PANEL_NEEDS_FIXED_SIZE.to_string());
    }

    if was_open {
        app.state::<SidebarState>().set_wiki_open(&label, false);
        app.state::<SidebarState>()
            .restore_collapse_for_panel(&label);
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
                    handle
                        .state::<SidebarState>()
                        .set_wiki_open(&win_label, true);
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
                    handle
                        .state::<SidebarState>()
                        .restore_collapse_for_panel(&win_label);
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
        return Err(report
            .lines()
            .find(|l| l.starts_with("REFUSE "))
            .map(|l| l[7..].to_string())
            .unwrap_or_else(|| {
                panel_no_room_notice(app.state::<SidebarState>().is_mobile(&label))
            }));
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

    // Desktop Automatic is a bare window. Opening is refused with a notice;
    // closing still works so a panel left open by a mode switch can shut.
    if !was_open && panels_unavailable(&app, &label) {
        return Ok(PANEL_NEEDS_FIXED_SIZE.to_string());
    }

    if was_open {
        app.state::<SidebarState>().set_about_open(&label, false);
        app.state::<SidebarState>()
            .restore_collapse_for_panel(&label);
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
                    handle
                        .state::<SidebarState>()
                        .set_about_open(&win_label, true);
                    let mut out = lock_report.clone();
                    out.push_str(&note);
                    // Shared webview: point it at this page before laying out.
                    out.push_str(&show_panel_page(&host, ABOUT_PAGE));
                    out.push_str(&layout_verbose(&host));
                    schedule_slim_panel_hug(&host);
                    out
                }
                Err(notice) => {
                    handle
                        .state::<SidebarState>()
                        .restore_collapse_for_panel(&win_label);
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
        return Err(report
            .lines()
            .find(|l| l.starts_with("REFUSE "))
            .map(|l| l[7..].to_string())
            .unwrap_or_else(|| {
                panel_no_room_notice(app.state::<SidebarState>().is_mobile(&label))
            }));
    }
    Ok(format!("aboutOpen=true\n{report}"))
}

/// Open or close the Options page. Same panel path as About (470 / 300).
#[tauri::command]
pub fn gbf_options_toggle(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let was_open = app.state::<SidebarState>().options_is_open(&label);

    // Desktop Automatic is a bare window. Opening is refused with a notice;
    // closing still works so a panel left open by a mode switch can shut.
    if !was_open && panels_unavailable(&app, &label) {
        return Ok(PANEL_NEEDS_FIXED_SIZE.to_string());
    }

    if was_open {
        app.state::<SidebarState>().set_options_open(&label, false);
        app.state::<SidebarState>()
            .restore_collapse_for_panel(&label);
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
                    handle
                        .state::<SidebarState>()
                        .set_options_open(&win_label, true);
                    let mut out = lock_report.clone();
                    out.push_str(&note);
                    // Shared webview: point it at this page before laying out.
                    out.push_str(&show_panel_page(&host, OPTIONS_PAGE));
                    out.push_str(&layout_verbose(&host));
                    schedule_slim_panel_hug(&host);
                    out
                }
                Err(notice) => {
                    handle
                        .state::<SidebarState>()
                        .restore_collapse_for_panel(&win_label);
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
        return Err(report
            .lines()
            .find(|l| l.starts_with("REFUSE "))
            .map(|l| l[7..].to_string())
            .unwrap_or_else(|| {
                panel_no_room_notice(app.state::<SidebarState>().is_mobile(&label))
            }));
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
    // Only the game webview. The wiki loads gbf.wiki and the shared panel
    // navigates between two local pages -- neither is Granblue, so letting
    // either reach the branch below zeroed the GAME's edge and made the next
    // layout fall back to a window-derived column. That collapsed the game
    // column and moved the sidebar on top of it for a few frames on every
    // panel switch: the flash.
    if !is_game_label(webview.label()) {
        return;
    }
    let host_name = url.host_str().unwrap_or("");
    let on_gbf = host_name.contains("granbluefantasy");
    if on_gbf {
        if let Some(frag) = url.fragment() {
            webview
                .app_handle()
                .state::<SidebarState>()
                .set_game_hash(&webview.window().label().to_string(), format!("#{frag}"));
        }
        reapply_lock(webview);
        if let Some(host) = webview.app_handle().get_window(webview.window().label()) {
            push_sidebar_game_hash(&host);
        }
        let _ = webview.eval("window.__gbfReportEdge && window.__gbfReportEdge()");
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

/// Push the lock CSS only when the page's copy does not already match.
///
/// A one-shot eval at the Automatic transition lost a race with Granblue's
/// reload: `reapply_lock` fired while Rust still thought it was in native
/// mode and pushed the OFF form, and the ON push that followed was wiped by
/// the document swap. `layout` runs after every edge report, so syncing here
/// self-heals whatever order those land in.
fn sync_lock_css(host: &Window) {
    let state = host.app_handle().state::<SidebarState>();
    let label = host.label().to_string();
    let want = state.is_locked(&label) && !native_mode(&state, &label);
    if !state.note_lock_css(&label, Some(want)) {
        return;
    }
    if let Some(game) = game_webview(host) {
        let _ = game.eval(lock_js(want));
    }
}

/// Push the current lock state into a game webview.
///
/// Called on every page load as well as on toggle, because Granblue rebuilds
/// its document on each navigation and takes our style element with it.
pub fn reapply_lock(webview: &Webview) {
    let label = webview.window().label().to_string();
    let state = webview.app_handle().state::<SidebarState>();
    // Native mode: hiding GBF's own #submenu / #general-chat there took the
    // player's chat column away for nothing -- the sidebar that column makes
    // room for is not even shown. Push the OFF form so a stale lock style
    // from before the switch is removed too.
    let locked = state.is_locked(&label) && !native_mode(&state, &label);
    // The document swap took our style element with it, whatever we last
    // pushed. Record the fresh truth so a later sync is not skipped.
    state.note_lock_css(&label, Some(locked));
    if let Err(error) = webview.eval(lock_js(locked)) {
        eprintln!("[Pake][gbf] could not reapply locked mode: {error}");
    }
    let _ = webview.eval(hug_js(locked));
}

fn set_lock(host: &Window, on: bool) -> String {
    host.app_handle()
        .state::<SidebarState>()
        .set_locked(host.label(), on);
    let mut extra = String::new();
    if !on {
        // Unlock cannot leave a hug in flight: the sidebar stops tracking the
        // game edge, so nothing would clear the flag.
        host.app_handle()
            .state::<SidebarState>()
            .set_hug_busy(host.label(), false);
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
    if app.state::<SidebarState>().is_mobile(&label) {
        return Ok("lock skipped: Layout is desktop-only\n".into());
    }
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

/// Mobile size: Large (Granblue's full scale) or Small. No restart needed --
/// it is only a webview zoom plus the usual hug.
#[tauri::command]
pub fn gbf_set_mobile_half(window: Window, half: bool) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    app.state::<SidebarState>().set_mobile_half(half);
    persist_layout_state(&app);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let report = match handle.get_window(&label) {
            Some(host) => layout_verbose(&host),
            None => format!("get_window({label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;
    let report = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    Ok(format!("mobile_half={half}
{report}"))
}

/// Choose which Granblue client to request. Mobile is the default.
///
/// The user agent can only be set when a webview is built, so this persists the
/// choice and restarts the app. `restart()` does not return.
#[tauri::command]
pub fn gbf_set_desktop_client(window: Window, desktop: bool) -> Result<String, String> {
    let app = window.app_handle().clone();
    let state = app.state::<SidebarState>();
    if state.is_desktop_client() == desktop {
        return Ok(format!("desktop_client already {desktop}"));
    }
    state.set_desktop_client(desktop);
    persist_layout_state(&app);
    app.restart();
}

/// Place Wiki / About / Options to the right of the sidebar (`outside=true`,
/// default) or between the game and the sidebar (`outside=false`).
/// The Options page's own state, for it to ask for on load.
///
/// About and Options share one webview, so opening Options navigates it and
/// Rust's layout push races the page load. Asking cannot race.
#[tauri::command]
pub fn gbf_panel_state(window: Window) -> Result<serde_json::Value, String> {
    let state = window.app_handle().state::<SidebarState>();
    let label = window.label();
    Ok(serde_json::json!({
        "wikiOutside": state.is_wiki_outside(label),
        "tray": state.is_tray_enabled(),
        "desktopClient": state.is_desktop_client(),
        "mobile": state.is_mobile(label),
        "theme": state.theme(),
        "sidebarDebug": state.is_sidebar_debug(),
    }))
}

/// Palette for the sidebar, Options, and About. Granblue and gbf.wiki keep
/// their own look. Process-wide, like the tray flag.
#[tauri::command]
pub fn gbf_set_theme(window: Window, theme: String) -> Result<String, String> {
    let app = window.app_handle().clone();
    let theme = {
        let state = app.state::<SidebarState>();
        state.set_theme(&theme);
        state.theme()
    };
    persist_layout_state(&app);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let mut labels: Vec<String> = handle.windows().keys().cloned().collect();
        labels.sort();
        let mut out = format!("theme={theme}\n");
        for label in labels {
            match handle.get_window(&label) {
                Some(host) => {
                    apply_theme_blank(&host);
                    out.push_str(&layout_verbose(&host));
                }
                None => out.push_str(&format!("get_window({label}) -> None\n")),
            }
        }
        let _ = tx.send(out);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));
    Ok(report)
}

/// Show or hide the sidebar's bottom diagnostics strip.
#[tauri::command]
pub fn gbf_set_sidebar_debug(window: Window, on: bool) -> Result<String, String> {
    let app = window.app_handle().clone();
    app.state::<SidebarState>().set_sidebar_debug(on);
    persist_layout_state(&app);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let win_label = window.label().to_string();
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
    Ok(format!("sidebar_debug={on}\n{report}"))
}

#[tauri::command]
pub fn gbf_set_wiki_outside(window: Window, outside: bool) -> Result<String, String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    app.state::<SidebarState>()
        .set_wiki_outside(&label, outside);
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
    zoom: Option<f64>,
    mobile: Option<bool>,
    hash: Option<String>,
) -> Result<(), String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let state = app.state::<SidebarState>();
    let hash_changed = hash
        .map(|h| state.set_game_hash(&label, h))
        .unwrap_or(false);
    if let Some(m) = mobile {
        state.set_mobile(&label, m);
    }
    let dpr = dpr.filter(|v| *v > 0.05).unwrap_or(0.0);
    let overlay = overlay.unwrap_or(0.0);
    let zoom = zoom.filter(|v| *v > 0.05).unwrap_or(0.0);
    if right <= 1.0 {
        // Reload removes #wrapper for a moment. Keep the last edge instead
        // of snapping the sidebar to x=0.
        state.set_game_zoom(&label, 0.0);
        if hash_changed {
            let handle = app.clone();
            let win_label = label;
            let _ = app.run_on_main_thread(move || {
                if let Some(host) = handle.get_window(&win_label) {
                    push_sidebar_game_hash(&host);
                }
            });
        }
        return Ok(());
    }
    let current_edge = state.game_edge(&label);
    let same_edge = (current_edge - right).abs() < 0.5;
    let same_overlay = (state.game_overlay(&label) - overlay).abs() < 0.5;
    let same_dpr = dpr <= 0.05 || (state.game_dpr(&label) - dpr).abs() < 0.01;
    let prev_auto = state.is_automatic(&label);
    let same_auto = prev_auto == automatic;
    state.set_automatic(&label, automatic);
    if dpr > 0.05 {
        state.set_game_dpr(&label, dpr);
    }
    if zoom > 0.05 {
        state.set_game_zoom(&label, zoom);
    }
    if same_edge && same_overlay && same_auto && same_dpr {
        if hash_changed {
            let handle = app.clone();
            let win_label = label;
            let _ = app.run_on_main_thread(move || {
                if let Some(host) = handle.get_window(&win_label) {
                    push_sidebar_game_hash(&host);
                }
            });
        }
        return Ok(());
    }
    state.set_game_edge(&label, right);
    state.set_game_overlay(&label, overlay);
    // Crossing into or out of native mode. Mobile reports
    // mobage_fixwindowsize === 0 always, so it is never either switch.
    let desktop = !state.is_mobile(&label);
    let went_native = automatic && !prev_auto && desktop;
    let left_native = !automatic && prev_auto && desktop;
    if went_native {
        // State only -- we are not guaranteed to be on the main thread; the
        // `layout` below hides the webviews.
        let _ = state.close_panels_for_automatic(&label);
    }
    // Crossing either way is handled by `sync_lock_css` inside the `layout`
    // below: it pushes the CSS only when the page's copy does not match, so
    // it does not matter which of the reload and the report lands first.
    let _ = left_native;
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

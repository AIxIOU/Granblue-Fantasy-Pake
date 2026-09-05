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

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    webview::WebviewBuilder, AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview,
    WebviewUrl, WebviewWindow, Window, WindowEvent,
};

/// Expanded width, matching `SIDEBAR_W` in gbf-scaler.js so the two builds are
/// visually comparable.
pub const SIDEBAR_W: f64 = 250.0;
/// Collapsed rail width, matching `SIDEBAR_W_COLLAPSED` in gbf-scaler.js.
pub const SIDEBAR_W_COLLAPSED: f64 = 52.0;
/// Granblue's narrowest layout (320 x zoom 1). Never squeeze the game below it.
const MIN_GAME_WIDTH: f64 = 320.0;
/// Preferred reading width for the wiki panel.
const WIKI_W: f64 = 460.0;
/// Below this the wiki is unreadable, so we refuse to open it rather than
/// showing a squeezed column. Mirrors main's "widen the window" notice.
const WIKI_MIN_W: f64 = 320.0;
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

#[derive(Default)]
pub struct SidebarState {
    collapsed: AtomicBool,
    wiki_open: AtomicBool,
    locked: AtomicBool,
    /// True when the lock was turned on by opening the wiki rather than by the
    /// player, so closing the wiki knows whether to undo it.
    lock_was_auto: AtomicBool,
}

impl SidebarState {
    pub fn is_collapsed(&self) -> bool {
        self.collapsed.load(Ordering::Relaxed)
    }

    /// Flip and return the NEW value.
    pub fn toggle(&self) -> bool {
        !self.collapsed.fetch_xor(true, Ordering::Relaxed)
    }

    pub fn wiki_is_open(&self) -> bool {
        self.wiki_open.load(Ordering::Relaxed)
    }

    pub fn set_wiki_open(&self, open: bool) {
        self.wiki_open.store(open, Ordering::Relaxed);
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    pub fn set_locked(&self, on: bool) {
        self.locked.store(on, Ordering::Relaxed);
    }

    pub fn lock_was_auto(&self) -> bool {
        self.lock_was_auto.load(Ordering::Relaxed)
    }

    pub fn set_lock_was_auto(&self, on: bool) {
        self.lock_was_auto.store(on, Ordering::Relaxed);
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
fn split(host: &Window, collapsed: bool, wiki_open: bool) -> tauri::Result<Split> {
    let scale = host.scale_factor()?;
    let size = host.inner_size()?.to_logical::<f64>(scale);

    let want_bar = if collapsed {
        SIDEBAR_W_COLLAPSED
    } else {
        SIDEBAR_W
    };
    // The game always wins a fight for space. If the window is too narrow to
    // hold both, the sidebar gives up width rather than crushing the game --
    // the reverse of the in-page version, where the sidebar could not be
    // narrower than its own content.
    let sidebar_w = want_bar.min((size.width - MIN_GAME_WIDTH).max(0.0));

    let room_for_wiki = (size.width - MIN_GAME_WIDTH - sidebar_w).max(0.0);
    let wiki_w = if wiki_open {
        WIKI_W.min(room_for_wiki)
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

/// How much width the wiki could take right now, without opening it.
fn wiki_room(host: &Window, collapsed: bool) -> tauri::Result<f64> {
    let scale = host.scale_factor()?;
    let size = host.inner_size()?.to_logical::<f64>(scale);
    let want_bar = if collapsed {
        SIDEBAR_W_COLLAPSED
    } else {
        SIDEBAR_W
    };
    let sidebar_w = want_bar.min((size.width - MIN_GAME_WIDTH).max(0.0));
    Ok((size.width - MIN_GAME_WIDTH - sidebar_w).max(0.0))
}

/// Place both webviews side by side across the window's client area.
///
/// Everything is in LOGICAL pixels. This is the whole point of moving the
/// sidebar out of the page: logical pixels are the window's own coordinate
/// space, so there is no CSS-pixel/physical-pixel conversion to get wrong and
/// no page zoom folded into `devicePixelRatio` to correct for. The drift bug
/// that Patch 2 in GBF_Pake_UPSTREAM_PATCHES.md exists to fix cannot occur here.
pub fn layout(host: &Window) -> tauri::Result<()> {
    let state = host.app_handle().state::<SidebarState>();
    let collapsed = state.is_collapsed();
    let wiki_open = state.wiki_is_open();
    let locked = state.is_locked();
    let s = split(host, collapsed, wiki_open)?;
    if s.height <= 0.0 {
        return Ok(()); // minimized; the next Resized event carries real numbers
    }

    if let Some(game) = game_webview(host) {
        game.set_position(LogicalPosition::new(0.0, 0.0))?;
        game.set_size(LogicalSize::new(s.game_w, s.height))?;
    }

    // The wiki keeps its webview once created, so a closed panel is hidden
    // rather than destroyed. That is the point of it being its own webview:
    // your page, scroll position and history survive being closed and
    // reopened, and survive the game reloading beside it.
    if let Some(wiki) = wiki_webview(host) {
        if wiki_open && s.wiki_w > 0.0 {
            wiki.set_position(LogicalPosition::new(s.game_w, 0.0))?;
            wiki.set_size(LogicalSize::new(s.wiki_w, s.height))?;
            let _ = wiki.show();
        } else {
            let _ = wiki.hide();
        }
    }

    if let Some(bar) = sidebar_webview(host) {
        bar.set_position(LogicalPosition::new(s.game_w + s.wiki_w, 0.0))?;
        bar.set_size(LogicalSize::new(s.sidebar_w, s.height))?;
        // Rust owns both flags; the page only renders them. One source of
        // truth, so the two can never disagree.
        let _ = bar.eval(format!(
            "window.__gbfSidebar && window.__gbfSidebar.setState({{collapsed:{collapsed},wikiOpen:{wiki_open},locked:{locked}}})"
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
    let state = host.app_handle().state::<SidebarState>();
    let collapsed = state.is_collapsed();
    let wiki_open = state.wiki_is_open();
    let sp = match split(host, collapsed, wiki_open) {
        Ok(v) => v,
        Err(e) => return format!("split ERR {e}"),
    };

    let mut out = format!(
        "want game={:.0} wiki={:.0} bar={:.0} h={:.0}
",
        sp.game_w, sp.wiki_w, sp.sidebar_w, sp.height
    );

    match game_webview(host) {
        None => out.push_str("game webview NOT FOUND
"),
        Some(game) => {
            let p = game.set_position(LogicalPosition::new(0.0, 0.0));
            let z = game.set_size(LogicalSize::new(sp.game_w, sp.height));
            out.push_str(&format!(
                "game pos={} size={} now={}
",
                result_word(&p),
                result_word(&z),
                bounds_word(&game),
            ));
        }
    }

    match wiki_webview(host) {
        None => out.push_str("wiki: not created
"),
        Some(wiki) => {
            if wiki_open && sp.wiki_w > 0.0 {
                let p = wiki.set_position(LogicalPosition::new(sp.game_w, 0.0));
                let z = wiki.set_size(LogicalSize::new(sp.wiki_w, sp.height));
                let v = wiki.show();
                out.push_str(&format!(
                    "wiki pos={} size={} show={} now={}
",
                    result_word(&p),
                    result_word(&z),
                    result_word(&v),
                    bounds_word(&wiki),
                ));
            } else {
                out.push_str(&format!("wiki hide={}
", result_word(&wiki.hide())));
            }
        }
    }

    match sidebar_webview(host) {
        None => out.push_str("sidebar webview NOT FOUND
"),
        Some(bar) => {
            let p = bar.set_position(LogicalPosition::new(sp.game_w + sp.wiki_w, 0.0));
            let z = bar.set_size(LogicalSize::new(sp.sidebar_w, sp.height));
            let locked = host.app_handle().state::<SidebarState>().is_locked();
            let e = bar.eval(format!(
                "window.__gbfSidebar && window.__gbfSidebar.setState({{collapsed:{collapsed},wikiOpen:{wiki_open},locked:{locked}}})"
            ));
            out.push_str(&format!(
                "bar pos={} size={} eval={} now={}
",
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

    let sp = split(&host, false, false)?;

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

    // The two manager views side by side -- the evidence for the get_window
    // rule in the module docs.
    let mut wv: Vec<String> = window.app_handle().webview_windows().keys().cloned().collect();
    wv.sort();
    let mut wins: Vec<String> = window.app_handle().windows().keys().cloned().collect();
    wins.sort();

    let scale = window.scale_factor().unwrap_or(-1.0);
    let phys = window.inner_size().map_err(|e| e.to_string())?;
    let logical = phys.to_logical::<f64>(if scale > 0.0 { scale } else { 1.0 });
    let collapsed = window.app_handle().state::<SidebarState>().is_collapsed();

    let report = format!(
        "window={}\nsidebar={}\nwebviews:\n  {}\nwebview_windows()={wv:?}\nwindows()={wins:?}\nscale={scale:.3}\nphysical={}x{}\nlogical={:.0}x{:.0}\ncollapsed={collapsed}",
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

/// Create the wiki webview, blank and hidden, during window setup.
///
/// # Why it is created here rather than on first open
///
/// Measured 2026-09-05. `add_child` called from inside a `run_on_main_thread`
/// callback never returns: the invoke stays pending forever, though the app
/// keeps running and the window keeps answering messages. Creating a webview
/// needs the event loop to turn, and a main-thread callback is itself running
/// on that loop. During `setup` the loop has not started yet, so `add_child`
/// works there -- which is why `attach()` succeeds.
///
/// So it is built up-front, pointed at `about:blank` and hidden. Opening it the
/// first time only navigates it, which is cheap and safe from any thread. The
/// startup cost is one empty webview.
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
        WebviewBuilder::new(&label, WebviewUrl::External(blank)),
        LogicalPosition::new(sp.game_w, 0.0),
        LogicalSize::new(WIKI_W, sp.height),
    )?;
    if let Some(w) = wiki_webview(host) {
        let _ = w.hide();
    }
    Ok(())
}

/// Open or close the wiki panel. Returns a narrated report.
///
/// Created lazily on first open, then kept and hidden -- so the page you were
/// reading, its scroll position and its history all survive closing and
/// reopening, and survive the game reloading beside it.
#[tauri::command]
pub fn gbf_wiki_toggle(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let collapsed = app.state::<SidebarState>().is_collapsed();
    let was_open = app.state::<SidebarState>().wiki_is_open();

    if !was_open {
        // Refuse rather than showing an unreadable column. This is the native
        // equivalent of main's "widen the window to open the wiki" notice --
        // and unlike main we never resize the window to make room, because
        // under Automatic Resizing that would reload the game.
        let room = wiki_room(&window, collapsed).map_err(|e| e.to_string())?;
        if room < WIKI_MIN_W {
            return Err(format!(
                "Not enough room for the wiki. Widen the window by about {:.0}px, or collapse the sidebar.",
                (WIKI_MIN_W - room).ceil()
            ));
        }
    }
    app.state::<SidebarState>().set_wiki_open(!was_open);

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = app.clone();
    let label = window.label().to_string();

    app.run_on_main_thread(move || {
        let report = match handle.get_window(&label) {
            Some(host) => {
                let mut out = String::new();
                let open_now = handle.state::<SidebarState>().wiki_is_open();

                // The wiki takes the space locked mode reclaims, exactly as on
                // main. Without this it takes width from the game instead and
                // pushes Granblue below what its Window Size needs, clipping
                // the very column we are hiding.
                if open_now {
                    if !handle.state::<SidebarState>().is_locked() {
                        handle.state::<SidebarState>().set_lock_was_auto(true);
                        out.push_str(&set_lock(&host, true));
                        out.push_str("\n");
                    }
                } else if handle.state::<SidebarState>().lock_was_auto() {
                    // Only undo a lock the wiki turned on itself.
                    handle.state::<SidebarState>().set_lock_was_auto(false);
                    out.push_str(&set_lock(&host, false));
                    out.push_str("\n");
                }

                if open_now {
                    // First open: the webview exists but is still on
                    // about:blank. Navigating is safe from here; creating it
                    // would not be.
                    if let Some(w) = wiki_webview(&host) {
                        let blank = w
                            .url()
                            .map(|u| u.scheme() == "about")
                            .unwrap_or(true);
                        if blank {
                            let url = Url::parse(WIKI_URL).expect("wiki url parses");
                            out.push_str(&format!("navigate={}
", result_word(&w.navigate(url))));
                        }
                    } else {
                        out.push_str("wiki webview MISSING
");
                    }
                }
                out.push_str(&layout_verbose(&host));
                out
            }
            None => format!("get_window({label}) -> None"),
        };
        let _ = tx.send(report);
    })
    .map_err(|e| format!("run_on_main_thread failed: {e}"))?;

    let report = rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .unwrap_or_else(|e| format!("main thread never replied: {e}"));

    Ok(format!("wikiOpen={}
{report}", !was_open))
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

/// Push the current lock state into a game webview.
///
/// Called on every page load as well as on toggle, because Granblue rebuilds
/// its document on each navigation and takes our style element with it.
pub fn reapply_lock(webview: &Webview) {
    let locked = webview.app_handle().state::<SidebarState>().is_locked();
    if let Err(error) = webview.eval(lock_js(locked)) {
        eprintln!("[Pake][gbf] could not reapply locked mode: {error}");
    }
}

fn set_lock(host: &Window, on: bool) -> String {
    host.app_handle().state::<SidebarState>().set_locked(on);
    match game_webview(host) {
        Some(game) => format!("lock({on})={}", result_word(&game.eval(lock_js(on)))),
        None => "lock: game webview NOT FOUND".to_string(),
    }
}

/// Toggle locked mode by hand. Returns the new state.
#[tauri::command]
pub fn gbf_toggle_lock(window: Window) -> Result<String, String> {
    let app = window.app_handle().clone();
    let now = !app.state::<SidebarState>().is_locked();
    // A manual toggle takes ownership of the lock away from the wiki, so
    // closing the wiki will not undo what the player just asked for.
    app.state::<SidebarState>().set_lock_was_auto(false);
    let report = set_lock(&window, now);
    Ok(format!("locked={now}
{report}"))
}

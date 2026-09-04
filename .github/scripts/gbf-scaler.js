(function () {
  // ------------------------------------------------------------------
  // Top-frame guard
  // ------------------------------------------------------------------
  try {
    if (window.top !== window.self) return;
  } catch (e) {
    return;
  }

  // ------------------------------------------------------------------
  // Single-instance guard
  // ------------------------------------------------------------------
  // Nothing builds two sidebars by accident today, but this exists so a
  // second copy CAN be loaded deliberately and win.
  //
  // The script is baked into the binary by Pake's --inject at build time, so
  // testing a one-line change otherwise costs a full CI build. With this
  // guard, a development copy injected at document-start (dev-inject.mjs, over
  // the same debugging port the probe uses) claims the flag first and the
  // baked copy then bails, leaving exactly one sidebar. Seconds instead of
  // minutes per iteration.
  //
  // The flag records where the winner came from, so probe.mjs can report which
  // copy is actually running -- testing the wrong one is a mistake worth
  // making impossible to make silently.
  if (window.__gbfScaler) return;
  window.__gbfScaler = { source: window.__gbfScalerDevSource || "baked" };

  // ------------------------------------------------------------------
  // Resource hints (preconnect / dns-prefetch)
  // ------------------------------------------------------------------
  // GBF can be loaded from two different origins depending on build
  // config, and they hit completely different infrastructure:
  //   - steam.granbluefantasy.com (Steam/desktop build): its own
  //     sharded asset CDN, plus a guild/chat websocket host.
  //   - game.granbluefantasy.jp (original Mobage/browser build): a
  //     differently-named asset CDN, plus the separate Mobage
  //     login/session SDK and ad/analytics trackers that build pulls
  //     in and the Steam build does not.
  // Each preset below was captured from a real network log on that
  // specific origin (2026-07-17). The page's own origin never needs a
  // hint (the browser is already connecting there as part of
  // navigation). The matching preset is picked via location.hostname
  // at runtime and applied as the very first thing this script does,
  // so hints fire as early as possible on every execution, including
  // post-reload cases (Home, entering combat) where the script re-runs
  // from scratch. Low-volume one-off hosts (trackers, single scripts)
  // get dns-prefetch only, not a full preconnect (DNS+TCP+TLS).
  (function addResourceHints() {
    function hint(rel, href, crossorigin) {
      if (
        document.querySelector('link[rel="' + rel + '"][href="' + href + '"]')
      )
        return;
      const l = document.createElement("link");
      l.rel = rel;
      l.href = href;
      if (crossorigin) l.crossOrigin = "anonymous";
      document.head.appendChild(l);
    }

    const PRESETS = {
      // steam.granbluefantasy.com — Steam/desktop build
      "steam.granbluefantasy.com": {
        preconnect: [
          "https://prd-game-a-granbluefantasy-steam.akamaized.net", // bulk: JS/CSS/images/font
          "https://ws.game.granbluefantasy.jp:11240", // guild/chat websocket
        ],
        preconnectCrossorigin: [
          "https://fonts.fontplus.dev", // webfont CSS/woff2
        ],
        dnsPrefetch: [
          "https://www.datadoghq-browser-agent.com", // analytics, one-off
        ],
      },
      // game.granbluefantasy.jp — original Mobage/browser build
      "game.granbluefantasy.jp": {
        preconnect: [
          "https://prd-game-a-granbluefantasy.akamaized.net", // bulk: JS/CSS/images/sounds
          "https://cdn-connect.mobage.jp", // Mobage login/session SDK
          "https://connect.mobage.jp", // Mobage login/session iframes
        ],
        preconnectCrossorigin: [
          "https://fonts.fontplus.dev", // webfont CSS
        ],
        dnsPrefetch: [
          "https://event-api.analytics.mbga.jp", // analytics
          "https://app.mobage.jp", // login proxy iframe, one-off
          "https://aimg-link.gree.net", // one-off script
          "https://d-track.send.microad.jp", // ad tracker, one-off
        ],
      },
    };

    const preset = PRESETS[location.hostname];
    if (!preset) return; // unknown/unrecognized host — nothing to hint from

    (preset.preconnect || []).forEach((h) => hint("preconnect", h));
    (preset.preconnectCrossorigin || []).forEach((h) =>
      hint("preconnect", h, true),
    );
    (preset.dnsPrefetch || []).forEach((h) => hint("dns-prefetch", h));
  })();

  // ------------------------------------------------------------------
  // Config
  // ------------------------------------------------------------------
  const SIDEBAR_W = 250;
  const SIDEBAR_W_COLLAPSED = 52;
  // Sidebar gutters. The RIGHT side is deliberately not symmetric with the
  // left: everything past a button's right edge is dead space against the
  // window edge, and at 12px gutter + 8px scrollbar + 12px padding it came to
  // 32px of nothing. It is now 4 + 8 + 1 = 13px, so the scrollbar sits one
  // pixel off the window edge and the buttons run almost the full width.
  const SIDEBAR_PAD = 12; // left, top, bottom
  const SIDEBAR_PAD_COLLAPSED = 6; // left, when icon-only
  const SCROLLBAR_W = 8; // must match the ::-webkit-scrollbar width below
  const NAV_SCROLLBAR_GAP = 4; // button right edge -> scrollbar
  const SIDEBAR_EDGE_GAP = 1; // scrollbar -> window edge

  // Wiki panel. A FIXED width, deliberately: the panel is room we reserve, not
  // room we scavenge. See applyBodyInset() and openWikiPanel().
  //
  // 960 is measured, not guessed. gbf.wiki is responsive, and its own sidebar
  // is a fixed 160px, so the article column is whatever is left. Checked live
  // against the site: at a 950px viewport it renders the full desktop layout
  // (160px wiki sidebar + ~727px article, no horizontal scrolling) and the
  // main page's panels sit side by side rather than stacking. Below roughly
  // 780 the article column keeps narrowing until it is cramped. 960 is that
  // measured 950 plus a little buffer, and matches the reference screenshot
  // the maintainer supplied.
  //
  // The wiki gets this width whenever it opens. Room is found in this order:
  //   1. grow the window, so the game keeps the width it already has;
  //   2. if the screen has no more to give, take it from the GAME.
  // Step 2 is fine — GBF is responsive and simply relays out narrower, exactly
  // as it does when you drag the window smaller. This is deliberately NOT a
  // minimum window size: an earlier version refused to open unless the whole
  // app could reach ~1930px, which was wrong for that reason.
  const WIKI_PANEL_WIDTH = 960;
  // Fallback tier for people with less screen than the maintainer's 2K, who
  // may be running GBF at its Large fixed size with the sidebar out. Verified
  // against the live site: at an 800px viewport gbf.wiki still renders its
  // 160px sidebar plus a 610px article column, with no element overlap and no
  // horizontal scrolling. Below this its elements start colliding, so we do
  // not go lower — we tell the user to narrow the game instead.
  const WIKI_PANEL_WIDTH_MIN = 800;
  // How far short of a tier we still accept it. Prevents a few pixels of
  // rounding or resize slop from demoting 960 all the way to 800.
  const WIKI_TIER_SLACK = 48;
  // Asked for on top of the exact requirement when widening. Aiming for the
  // exact width is a knife edge: css-to-physical conversion, the OS rounding
  // the window size, and DPI scaling can each land a pixel or two short, and
  // a shortfall of one pixel used to cost an entire tier. A few pixels of
  // headroom costs nothing visible — it shows as a sliver of the sidebar's
  // own background beside the panel.
  const WIKI_WIDEN_BUFFER = 2;
  // The width actually in use for the current open, decided by openWikiPanel().
  let wikiPanelWidth = WIKI_PANEL_WIDTH;

  // The About page shares the wiki's panel, so it inherits the sizing, the
  // reserved inset, the auto-close and the Automatic-mode rules for free. It
  // only needs its own widths: it is a page of text, so it asks for far less
  // and can still open in windows where the wiki cannot.
  const ABOUT_PANEL_WIDTH = 470;
  const ABOUT_PANEL_WIDTH_MIN = 300;
  let panelMode = "wiki"; // "wiki" | "about"
  // Absurdity guard, not a quality bar. GBF is mobile-derived and stays
  // functional at narrow widths, so this exists only to stop the wiki
  // squeezing the game down to a sliver on a genuinely small screen.
  const MIN_GAME_WIDTH = 320;
  // Slack on the auto-close test, so rounding cannot trip it. See
  // positionSidebar().
  const WIKI_ROOM_TOLERANCE = 8;

  // "Locked" mode doesn't try to control GBF's own width/zoom math (the
  // game area is genuinely responsive, not a fixed size we can predict
  // with a formula). Instead, when locked, we:
  //   1. Force-hide #submenu / #general-chat (GBF's own chat/help panel)
  //      via CSS !important, regardless of what class GBF toggles on it.
  //   2. Reserve margin-right = sidebar width, same as always, so GBF
  //      lays out the game within (window width - sidebar width).
  //   3. Continuously measure #wrapper's *actual rendered* right edge
  //      (via getBoundingClientRect, which already accounts for whatever
  //      internal zoom GBF is using) and snap the sidebar's left edge to
  //      it. NOTE: #wrapper is the actual game viewport — its parent
  //      #mobage-game-container is an umbrella div that also contains
  //      #submenu/#general-chat as siblings of #wrapper, so it spans
  //      close to full width regardless of game content; measuring the
  //      umbrella instead of #wrapper was the bug in the previous pass.
  const GAME_CONTAINER_ID = "wrapper";

  const NAV = [
    { section: "NAVIGATION" },
    { label: "Home", hash: "#mypage", icon: "\u2302", key: "1" },
    { label: "Party", hash: "#party/index/0/npc/0", icon: "\u2694", key: "p" },
    { label: "Quests", hash: "#quest", icon: "\u2637", key: "2" },
    { label: "Raids", hash: "#quest/assist", icon: "\u2620", key: "3" },
    { label: "Co-op", hash: "#coopraid", icon: "\u21C4", key: "4" },
    { label: "Crew", hash: "#guild", icon: "\u2691", key: "5" },

    { section: "MANAGEMENT" },
    { label: "Supplies", hash: "#item", icon: "\u25C8", key: "6" },
    { label: "Inventory", hash: "#list", icon: "\u2261", key: "7" },
    { label: "Crate", hash: "#present", icon: "\u25A3", key: "c" },
    { label: "Stash", hash: "#container", icon: "\u26BF", key: "s" },
    { label: "Profile", hash: "#profile", icon: "\u263A", key: "8" },

    { section: "MORE" },
    { label: "Shop", hash: "#shop", icon: "\u26C1", key: "9" },
    {
      label: "Journey Drops",
      hash: "#shop/exchange/trajectory",
      icon: "\u2728",
      key: "0",
    },
    { label: "Arcarum", hash: "#arcarum", icon: "\u2606", key: "-" },
    {
      label: "Alchemy Lab",
      hash: "#frontier/alchemy/top",
      icon: "\u2697",
      key: "=",
    },
    { label: "Trial Battles", hash: "#trial_battle", icon: "\u2694", key: "[" },
    { label: "Casino", hash: "#casino", icon: "\u2660", key: "]" },
    { label: "Gacha", hash: "#gacha", icon: "\u2748", key: ";" },

    { section: "WIKI" },
    // Not a real GBF hash — opens the slide-out wiki panel instead of
    // navigating GBF itself. See openWikiPanel()/ensureWikiPanel() below.
    { label: "Wiki", action: () => openWikiPanel("wiki"), icon: "⊞", key: "w" },
    // Same panel, different contents — see openWikiPanel(mode).
    { label: "About", action: () => openWikiPanel("about"), icon: "ⓘ" },
  ];

  const store = {
    get(k, d) {
      try {
        const v = localStorage.getItem(k);
        return v === null ? d : v;
      } catch (e) {
        return d;
      }
    },
    set(k, v) {
      try {
        localStorage.setItem(k, v);
      } catch (e) {}
    },
  };

  let collapsed = store.get("gbfCollapsed", "0") === "1";
  let locked = store.get("gbfLocked", "1") === "1";
  // Wiki panel state. Declared HERE, with the rest of the module state, and
  // not down beside the wiki code — applyBodyInset() runs during sidebar
  // construction and calls wikiIsOpen(), which reads wikiPanelEl. A `let`
  // further down the file is still in its temporal dead zone at that point,
  // and reading it throws ReferenceError, killing the whole script: no
  // sidebar, no lock state, no listeners. That shipped once. Do not move
  // these declarations back down next to the wiki section.
  let wikiPanelEl = null;
  let wikiFrameEl = null;
  let wikiUnloadTimer = 0;
  // Window width as we left it after widening for the wiki. If it no longer
  // matches, the user resized and we must not shrink further. See
  // restoreWindowWidth().
  let wikiWidthAfterWiden = 0;
  // Window width before the panel widened anything, so the restore can aim at
  // it rather than adding back a delta. resizeWindowByCss rounds through
  // physical pixels, so a symmetric -N/+N pair loses a pixel or two each time:
  // measured 6px lost over one open/close suite.
  let wikiWidthBeforeWiden = 0;
  // Pending re-check of the auto-close decision once the grace expires. See
  // positionSidebar().
  let wikiRecheckTimer = 0;
  // True while a panel open/close is mid-flight. The hug must not run then:
  // opening WIDENS the window before the panel exists, so the hug measured
  // that new width as surplus and took it straight back -- observed shrinking
  // the window to its 320px minimum with the sidebar sitting on top of the
  // game.
  let panelBusy = false;

  let sidebarEl = null; // set once the sidebar exists (outer #gbf-sidebar-outer)
  let sidebarInnerEl = null;
  let navScrollEl = null; // the scrolling region inside the sidebar
  // css px the window was widened by to fit the expanded nav, given back when
  // it collapses again. Mirrors wikiWidenedBy.
  let navWidenedBy = 0; // inner #gbf-simple-sidebar (nav content), for auto collapse/expand
  let toggleBtnEl = null; // collapse toggle button, so its icon can be updated from auto logic too
  let sizeObserver = null;
  // Whether the sidebar has faded in yet after this script execution
  // (fresh on every real page reload, since the whole script re-runs).
  // See revealSidebar() below.
  let sidebarRevealed = false;
  // True when locked-mode available space is too tight for the full
  // sidebar, forcing icon-only mode regardless of the manual toggle.
  // See positionSidebar() for how this gets set/cleared.
  let autoNarrow = false;
  // The game's right edge as it was when auto-narrow engaged. See the long
  // note in positionSidebar() -- this is what makes auto-narrow reversible.
  let narrowRefEdge = Infinity;
  // Hoisted: applyCollapsedVisual() runs during construction and schedules a
  // refit, so this must already be initialised by then. A `let` further down
  // would be in its temporal dead zone and kill the whole script.
  let densityRefitTimer = 0;
  // Set when the wiki collapsed the nav to icons to free room for itself.
  // Deliberately NOT autoNarrow: positionSidebar() recomputes that from the
  // available space, which grows once the wiki reserves its area, so it would
  // immediately expand the nav again and undo us. Cleared on wiki close.
  let wikiForcedNarrow = false;
  // Drag-to-scroll: click-and-drag anywhere in the sidebar to scroll
  // its nav list vertically. Only engages once the pointer has moved
  // past DRAG_SCROLL_THRESHOLD, so plain button clicks are unaffected;
  // once crossed, the click that would otherwise fire on whatever's
  // under the pointer is suppressed (see the capturing click listener
  // below) so a drag never also triggers navigation.
  const DRAG_SCROLL_THRESHOLD = 4;
  // How long after a real drag ends we still swallow a click. The click a drag
  // produces arrives within a few ms of pointerup; a deliberate tap after the
  // drag needs a fresh pointerdown and realistically lands later than this.
  //
  // This is a TIME WINDOW on purpose, not a boolean flag. Two earlier attempts
  // used a flag and both leaked, in opposite directions — see the comment on
  // the click handlers below. Do not convert this back to a flag.
  const DRAG_CLICK_SUPPRESS_MS = 250;
  let dragScrolling = false;
  let dragScrollMoved = false;
  let dragScrollEndedAt = 0; // see pageDragEndedAt - same mechanism
  let dragScrollStartY = 0;
  let dragScrollStartTop = 0;

  // --- Momentum (flick) scrolling ---------------------------------
  // Shared by both the sidebar drag-scroll and the generic GBF-page
  // drag-scroll below. On release, if the pointer was moving fast
  // enough, keeps scrolling in that direction and decays the speed
  // every frame (friction) until it's near zero or the element hits
  // its scroll bounds. A fresh pointerdown cancels any in-flight coast.
  const MOMENTUM_MIN_VELOCITY = 0.15; // px/ms — below this, don't bother coasting
  const MOMENTUM_FRICTION = 0.95; // multiplier applied to velocity each frame
  const MOMENTUM_SAMPLE_WINDOW = 100; // ms — how far back we look to compute release velocity

  function createMomentumScroller(getEl) {
    let history = []; // {t, y} samples of the last ~100ms of pointer movement
    let rafId = null;

    function reset() {
      history = [];
    }

    function sample(y) {
      const now = performance.now();
      history.push({ t: now, y });
      const cutoff = now - MOMENTUM_SAMPLE_WINDOW;
      while (history.length > 1 && history[0].t < cutoff) history.shift();
    }

    function stop() {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
    }

    function velocityPxPerMs() {
      if (history.length < 2) return 0;
      const first = history[0];
      const last = history[history.length - 1];
      const dt = last.t - first.t;
      if (dt <= 0) return 0;
      // Positive delta = pointer moved down = content should keep
      // scrolling up (matches the drag convention used below:
      // scrollTop = startTop - delta), so velocity is inverted here.
      return -(last.y - first.y) / dt;
    }

    function release() {
      const el = getEl();
      const v0 = velocityPxPerMs();
      reset();
      if (!el || Math.abs(v0) < MOMENTUM_MIN_VELOCITY) return;

      let velocity = v0;
      let lastFrame = performance.now();
      stop();

      function tick(now) {
        const dt = Math.min(now - lastFrame, 48); // clamp for tab-switch pauses
        lastFrame = now;

        const maxScroll = el.scrollHeight - el.clientHeight;
        el.scrollTop = Math.max(
          0,
          Math.min(maxScroll, el.scrollTop + velocity * dt),
        );

        velocity *= Math.pow(MOMENTUM_FRICTION, dt / 16.7);

        const atBound = el.scrollTop <= 0 || el.scrollTop >= maxScroll;
        if (Math.abs(velocity) < MOMENTUM_MIN_VELOCITY || atBound) {
          rafId = null;
          return;
        }
        rafId = requestAnimationFrame(tick);
      }
      rafId = requestAnimationFrame(tick);
    }

    return { sample, release, stop, reset };
  }

  // Suppresses the browser's native "drag this image/link" gesture
  // (HTML5 draggable, on by default for <img> and <a>). Without this,
  // starting a drag-to-scroll gesture on top of an image hands the gesture to
  // the browser's own drag-and-drop instead of our pointermove-based scroll
  // handlers, so scrolling silently stops working the moment the pointer
  // starts over an image.
  //
  // NARROWED 2026-09-03: this used to preventDefault() every dragstart in the
  // page unconditionally. That is a blanket override of a native behaviour
  // inside GBF, which the project's first rule forbids — GBF must behave as it
  // does in a plain browser. It is now limited to drags originating on an
  // <img> or <a>, which is the only case that actually hijacks drag-to-scroll.
  // Anything else GBF might do with native drag is left alone.
  document.addEventListener(
    "dragstart",
    (e) => {
      const t = e.target;
      if (!t || typeof t.closest !== "function") return;
      if (!t.closest("img, a")) return;
      e.preventDefault();
    },
    true,
  );

  function isEffectivelyCollapsed() {
    return collapsed || autoNarrow || wikiForcedNarrow;
  }

  function sidebarWidth() {
    return isEffectivelyCollapsed() ? SIDEBAR_W_COLLAPSED : SIDEBAR_W;
  }

  // Applies the collapsed/expanded visual state (icon-only vs full),
  // driven by isEffectivelyCollapsed() (manual toggle OR auto-narrow).
  // Defined at top level (not nested inside the sidebar-creation block
  // below) so positionSidebar() can call it once auto-narrow changes.
  function applyCollapsedVisual() {
    if (!sidebarInnerEl) return;
    const eff = isEffectivelyCollapsed();
    sidebarInnerEl.classList.toggle("gbf-collapsed", eff);
    if (toggleBtnEl) toggleBtnEl.textContent = eff ? "\u00AB" : "\u00BB";
    sidebarInnerEl.querySelectorAll(".gbf-section-title").forEach((t) => {
      if (!t.dataset.full) t.dataset.full = t.textContent;
      t.textContent = eff ? t.dataset.full.charAt(0) : t.dataset.full;
    });
    applyBodyInset();
    scheduleDensityRefit();
  }

  // Brief visual acknowledgment that a button was clicked, independent
  // of .gbf-active (which marks the current page) and independent of
  // how long the actual navigation/reload takes to complete. Purely
  // cosmetic and self-contained — never reads or reacts to anything
  // from GBF itself, just toggles a class on the button that was
  // clicked and removes it again after the flash finishes. Safe to
  // call even right before a real page reload (e.g. Home/combat),
  // since the flash class doesn't need to persist across that — the
  // sidebar rebuilds from scratch afterward anyway.
  function flashButton(btn) {
    if (!btn) return;
    btn.classList.remove("gbf-clicked");
    // Force reflow so re-adding the class restarts the CSS
    // transition even on rapid repeat clicks of the same button.
    void btn.offsetWidth;
    btn.classList.add("gbf-clicked");
    setTimeout(() => btn.classList.remove("gbf-clicked"), 200);
  }

  // ------------------------------------------------------------------
  // Base page styles
  // ------------------------------------------------------------------
  const style = document.createElement("style");
  style.innerHTML = `
        html, body {
            margin: 0 !important;
            padding: 0 !important;
            background-color: #0a1622 !important;
        }
        body {
            overflow-x: hidden !important;
        }
        #wrapper, .wrapper {
            margin: 0 auto 0 0 !important;
        }
        /* When locked, force GBF's own wide-screen side panel closed no
           matter what class/attribute GBF itself is toggling internally. */
        html.gbf-locked #submenu,
        html.gbf-locked #general-chat {
            display: none !important;
        }
        /* Belt-and-suspenders alongside the dragstart listener below:
           stops the browser's native image/link drag affordance from
           hijacking drag-to-scroll gestures that start over an image. */
        img, a {
            -webkit-user-drag: none;
            user-drag: none;
        }
    `;
  document.head.appendChild(style);

  // ------------------------------------------------------------------
  // Sidebar
  // ------------------------------------------------------------------
  if (!document.getElementById("gbf-simple-sidebar")) {
    const sidebarStyle = document.createElement("style");
    sidebarStyle.textContent = `
            /* ---- Granblue Fantasy Themed Sidebar (flatter left edge) ----
               #gbf-sidebar-outer handles fixed positioning/left tracking;
               #gbf-simple-sidebar is a fixed-width flex child pinned to
               the outer's LEFT edge (flex-start) — i.e. immediately
               against the game — so it never drifts away from the game
               content. Any extra reclaimed space (when the window is
               wider than the game+sidebar actually need) shows as the
               outer's own themed background AFTER the sidebar, so the
               window's right edge always touches themed background
               instead of a raw gap, without ever moving the sidebar
               itself away from the game.

               z-index is set to the max signed 32-bit int so GBF's own
               fixed-position top/bottom toolbars (which ignore our
               body margin-right inset, since fixed elements aren't
               affected by margins on ancestors) can never render on
               top of us, no matter what z-index GBF itself uses. */
            #gbf-sidebar-outer {
                position: fixed;
                top: 0;
                right: 0;
                height: 100vh;
                width: ${SIDEBAR_W}px;
                display: flex;
                justify-content: flex-start;
                background: linear-gradient(180deg, #0b1a2e 0%, #061220 100%);
                background-image: 
                    linear-gradient(180deg, #0b1a2e 0%, #061220 100%),
                    repeating-linear-gradient(
                        0deg,
                        transparent,
                        transparent 2px,
                        rgba(201, 169, 110, 0.03) 2px,
                        rgba(201, 169, 110, 0.03) 4px
                    );
                border-left: 1px solid rgba(201,169,110,0.25);
                box-shadow: -1px 0 3px rgba(0,0,0,0.3);
                z-index: 2147483647;
                opacity: 1;
                transition: opacity .15s ease;
            }
            /* Applied from creation until GBF's game container reports a
               real size, so a fresh rebuild (e.g. after Home/combat's
               full reload) never pops in at a transient/wrong position
               mid-measurement — it stays invisible, then fades in once
               positionSidebar() has already placed it correctly. */
            #gbf-sidebar-outer.gbf-sidebar-hidden {
                opacity: 0;
            }
            #gbf-simple-sidebar {
                /* flex-shrink MUST stay 0. With shrink enabled the browser
                   squeezed the nav to make room for the wiki panel instead of
                   using reserved space — observed in play 2026-09-03, the nav
                   was crushed from 245px to 39px and the wiki rendered in its
                   place. The wiki gets reserved room, never the nav's room. */
                flex: 0 0 auto;
                width: ${SIDEBAR_W}px;
                max-width: 100%;
                height: 100%;
                /* The sidebar itself no longer scrolls — #gbf-nav-scroll does.
                   See the comment on #gbf-sidebar-actions for why. */
                display: flex;
                flex-direction: column;
                overflow: hidden;
                box-sizing: border-box;
                padding: ${SIDEBAR_PAD}px ${SIDEBAR_EDGE_GAP}px
                         ${SIDEBAR_PAD}px ${SIDEBAR_PAD}px;
                font-family: "Georgia", "Times New Roman", serif;
                transition: width .12s ease;
                cursor: grab;
            }
            .gbf-collapsed #gbf-nav-scroll {
                padding-right: ${NAV_SCROLLBAR_GAP}px;
            }
            #gbf-simple-sidebar.gbf-collapsed {
                width: ${SIDEBAR_W_COLLAPSED}px;
                padding: ${SIDEBAR_PAD}px ${SIDEBAR_EDGE_GAP}px
                         ${SIDEBAR_PAD}px ${SIDEBAR_PAD_COLLAPSED}px;
            }
            /* Drag-to-scroll: while an actual drag is in progress (past
               the movement threshold), force the grabbing cursor even
               over buttons, which otherwise declare their own
               cursor:pointer, and block text selection during the drag. */
            #gbf-simple-sidebar.gbf-drag-scrolling,
            #gbf-simple-sidebar.gbf-drag-scrolling * {
                cursor: grabbing !important;
                user-select: none;
            }

            /* The scrolling region. Holds the toggle, every nav section and
               the notice; NOT the actions footer. */
            #gbf-nav-scroll {
                flex: 1 1 auto;
                min-height: 0;
                overflow-y: auto;
                overflow-x: hidden;
                /* Just enough to keep the scrollbar off the buttons. Matching
                   the 12px left gutter here was tried and reverted: combined
                   with the scrollbar and the sidebar's own right padding it
                   left 32px of dead space against the window edge. */
                padding-right: ${NAV_SCROLLBAR_GAP}px;
            }

            /* Density steps. These are the FALLBACK, not the normal case:
               fitNavDensity() first tries the stylesheet's own spacing and
               grows the gaps to fill the sidebar down to ACTIONS. It only
               reaches for these when the list would otherwise run into the
               footer, and then takes the loosest step that still fits. So the
               spacing follows the window height and the number of nav items
               instead of being a number tuned to one screen. */
            /* EVERY density rule is scoped to #gbf-nav-scroll. This is not
               tidiness -- an unscoped .gbf-dense-N .gbf-nav-btn also matches
               the ACTIONS footer's Back/Reload/Lock, and that caused a
               permanent oscillation: compressing shrank the footer, which
               GREW the scroller's viewport by ~40px, which made the list fit,
               which removed the class, which grew the footer back. Traced on a
               real build flipping dense-3 <-> none every ~355ms forever, with
               the viewport swinging 1016 <-> 1056.

               The fitter's whole premise is that it changes CONTENT height
               while the VIEWPORT stays put. Anything here that can alter the
               footer breaks that premise and reintroduces the loop. */
            #gbf-nav-scroll .gbf-dense-1 .gbf-nav-btn,
            .gbf-dense-1 #gbf-nav-scroll .gbf-nav-btn { padding: 9px 12px; margin-bottom: 5px; }
            .gbf-dense-1 #gbf-nav-scroll .gbf-section-title { margin: 8px 0 4px; }
            .gbf-dense-1 #gbf-toggle { padding: 6px 0; }

            .gbf-dense-2 #gbf-nav-scroll .gbf-nav-btn { padding: 7px 12px; margin-bottom: 4px; }
            .gbf-dense-2 #gbf-nav-scroll .gbf-section-title { margin: 6px 0 3px; }
            .gbf-dense-2 #gbf-toggle { padding: 5px 0; }

            .gbf-dense-3 #gbf-nav-scroll .gbf-nav-btn { padding: 5px 12px; margin-bottom: 3px; }
            .gbf-dense-3 #gbf-nav-scroll .gbf-section-title { margin: 5px 0 2px; }
            .gbf-dense-3 #gbf-toggle { padding: 4px 0; }

            .gbf-collapsed.gbf-dense-1 #gbf-nav-scroll .gbf-nav-btn { padding: 9px 0; }
            .gbf-collapsed.gbf-dense-2 #gbf-nav-scroll .gbf-nav-btn { padding: 7px 0; }
            .gbf-collapsed.gbf-dense-3 #gbf-nav-scroll .gbf-nav-btn { padding: 5px 0; }

            #gbf-nav-scroll::-webkit-scrollbar { width: 8px; }
            #gbf-nav-scroll::-webkit-scrollbar-track {
                background: rgba(0,0,0,0.3);
                border-radius: 4px;
            }
            #gbf-nav-scroll::-webkit-scrollbar-thumb {
                background: #c9a96e;
                border-radius: 4px;
                border: 1px solid #5c4a2e;
            }

            .gbf-section-title {
                color: #e9d097;
                font-size: 12px;
                letter-spacing: 2px;
                font-weight: 700;
                /* Sized so the list fills the sidebar with exactly one row of
                   headroom — see the note on .gbf-nav-btn below. Measured at
                   16/8 the headroom was only 21px, under half a row, because
                   the taller buttons also made the actions footer taller and
                   shrank the scrolling viewport. */
                margin: 11px 0 5px;
                white-space: nowrap;
                text-transform: uppercase;
                text-shadow: 1px 1px 0 rgba(0,0,0,0.8);
                border-bottom: 1px solid #c9a96e;
                padding-bottom: 3px;
                position: relative;
            }
            .gbf-section-title::after {
                content: '';
                position: absolute;
                bottom: -2px;
                left: 0;
                width: 30px;
                height: 2px;
                background: #c9a96e;
                box-shadow: 0 0 6px rgba(201,169,110,0.6);
            }
            .gbf-collapsed .gbf-section-title {
                text-align: center;
                font-size: 10px;
                letter-spacing: 1px;
                overflow: hidden;
                border-bottom: none;
            }
            .gbf-collapsed .gbf-section-title::after {
                display: none;
            }

            .gbf-nav-btn {
                display: flex;
                align-items: center;
                gap: 10px;
                width: 100%;
                /* Spacing is tuned so the whole list fills the sidebar with
                   about one row of space left at the bottom — enough to add
                   one more nav item later without it overflowing.
                   Measured against the real build at a 1376px-tall window:
                   19 buttons and 4 headings come to 1133px of content in a
                   1186px viewport, leaving 53px — one row is 50px, so exactly
                   one more item fits. Trimming further leaves dead space;
                   adding more starts a scrollbar.

                   Note the coupling: the footer's Back/Reload/Lock buttons use
                   this same rule, so raising the padding makes the footer
                   taller and shrinks the scrolling viewport at both ends. The
                   heading margins are the safe knob — they only affect the
                   list. */
                margin-bottom: 6px;
                padding: 12px 12px;
                background: rgba(20, 30, 45, 0.7);
                border: 1px solid #5c4a2e;
                border-left: 3px solid #c9a96e;
                border-radius: 4px;
                color: #e0d3b5;
                cursor: pointer;
                text-align: left;
                font-weight: 600;
                font-size: 13px;
                font-family: inherit;
                box-sizing: border-box;
                white-space: nowrap;
                overflow: hidden;
                text-shadow: 0 1px 2px rgba(0,0,0,0.6);
                /* NOT "all". Every visual property here may animate, but
                   padding and margin must NOT: fitNavDensity() decides from
                   the measured list height, and animating those makes every
                   measurement a value that is still moving. It measured
                   mid-animation, decided from a stale height, re-measured,
                   and flipped forever -- traced on a shipped build as the
                   list oscillating between spacings ~3 times a second for as
                   long as the app ran.
                   Hover, active and click-flash all animate exactly as before;
                   only the geometry is now instant. */
                transition: background .15s ease, border-color .15s ease,
                            color .15s ease, box-shadow .15s ease,
                            border-left-color .15s ease;
                position: relative;
            }
            .gbf-nav-btn:hover {
                background: rgba(50, 70, 100, 0.7);
                border-color: #dbb867;
                color: #f5e7c6;
                box-shadow: 0 0 8px rgba(201,169,110,0.3);
            }
            .gbf-nav-btn:active {
                background: rgba(30, 45, 65, 0.9);
                transform: translateY(1px);
            }
            .gbf-nav-btn.gbf-active {
                background: linear-gradient(90deg, rgba(201,169,110,0.25) 0%, rgba(20,30,45,0.8) 80%);
                border-left-color: #e5c158;
                color: #f9eec1;
                box-shadow: 0 0 12px rgba(201,169,110,0.4);
            }
            /* Click feedback flash: a brief bright highlight the instant a
               button is clicked, independent of .gbf-active (current page)
               and independent of how long the actual navigation/reload
               takes. Uses the same .15s "all" transition already declared
               on .gbf-nav-btn above to fade back out once the class is
               removed ~200ms after being added (see flashButton()). !important
               so it visibly overrides .gbf-active's own background/border
               for the moment of the flash, even on the currently-active button. */
            .gbf-nav-btn.gbf-clicked {
                background: rgba(229, 193, 88, 0.55) !important;
                border-color: #f9eec1 !important;
                box-shadow: 0 0 14px rgba(229,193,88,0.7) !important;
            }
            .gbf-nav-btn::before {
                content: '';
                position: absolute;
                top: -1px;
                right: -1px;
                width: 6px;
                height: 6px;
                border-top: 1px solid #c9a96e;
                border-right: 1px solid #c9a96e;
                opacity: 0.6;
            }
            .gbf-collapsed .gbf-nav-btn {
                justify-content: center;
                padding: 12px 0;
                gap: 0;
                border-left: none;
                border-radius: 4px;
            }
            .gbf-collapsed .gbf-nav-btn::before {
                display: none;
            }
            .gbf-collapsed .gbf-nav-btn .gbf-label { display: none; }

            .gbf-icon {
                flex: 0 0 auto;
                width: 16px;
                text-align: center;
                font-size: 15px;
                opacity: .9;
                color: #c9a96e;
                text-shadow: 0 0 4px rgba(201,169,110,0.5);
            }
            .gbf-key {
                margin-left: auto;
                font-size: 9px;
                color: #8a7c5a;
                font-weight: 500;
                letter-spacing: 0.5px;
            }
            .gbf-collapsed .gbf-key { display: none; }

            .gbf-row { display: flex; gap: 6px; margin-bottom: 6px; }
            .gbf-row .gbf-nav-btn { margin-bottom: 0; }
            .gbf-collapsed .gbf-row { flex-direction: column; }

            /* Sticks Back/Reload/Lock to the bottom of the sidebar's
               scrollable area once scrolling would otherwise push them
               out of view, so they stay reachable at any window height
               without needing a separate non-scrolling region. Solid
               background (matching the page's own base background color)
               so nav items scrolling underneath don't show through while
               stuck. */
            /* A real flex footer, OUTSIDE the scrolling region — not
               position:sticky inside it.
               Sticky looked equivalent and was not. An opaque sticky footer
               overlays whatever scrolls under it, so the LAST item in the list
               was hidden behind it at ordinary scroll positions. The Wiki
               button is that last item, and it was invisible unless the list
               happened to be scrolled to its exact end — observed in play
               2026-09-03, measured as 0 of 41px visible at 60px from the
               bottom of the scroll range.
               Padding or margin does not fix it; it only moves which scroll
               position hides something. The footer has to be out of the
               scrolling box. Do not put it back. */
            #gbf-sidebar-actions {
                flex: 0 0 auto;
                /* The footer sits outside the scroller, so it has no scrollbar
                   to allow for. Pad it by the gap plus the scrollbar width so
                   its buttons line up with the nav buttons above. */
                padding-right: ${NAV_SCROLLBAR_GAP + SCROLLBAR_W}px;
                background: #0a1622;
                padding-top: 10px;
                margin-top: 10px;
                border-top: 1px solid rgba(201,169,110,0.25);
            }

            #gbf-toggle {
                width: 100%;
                padding: 8px 0;
                margin-bottom: 4px;
                background: transparent;
                border: 1px solid #5c4a2e;
                border-radius: 4px;
                color: #c9a96e;
                cursor: pointer;
                font-size: 15px;
                font-weight: 700;
                font-family: inherit;
                text-shadow: 0 0 5px rgba(201,169,110,0.4);
                transition: background .15s ease, color .15s ease;
            }
            #gbf-toggle:hover {
                background: rgba(201,169,110,0.15);
                color: #ecd59b;
                box-shadow: 0 0 8px rgba(201,169,110,0.3);
            }

            /* Transient message for actions that cannot run right now (e.g.
               the wiki needing locked mode or more width). Collapsed to zero
               height when idle so it never takes space in the nav list. */
            #gbf-sidebar-notice {
                max-height: 0;
                overflow: hidden;
                opacity: 0;
                margin-top: 0;
                padding: 0 8px;
                border-radius: 4px;
                background: rgba(201,169,110,0.12);
                border: 1px solid transparent;
                color: #f0e0b8;
                font-size: 11px;
                line-height: 1.35;
                transition: max-height .15s ease, opacity .15s ease,
                            margin-top .15s ease, padding .15s ease;
            }
            #gbf-sidebar-notice.gbf-notice-show {
                max-height: 80px;
                opacity: 1;
                margin-top: 8px;
                padding: 8px;
                border-color: rgba(201,169,110,0.35);
            }
            .gbf-collapsed #gbf-sidebar-notice { display: none; }
        `;
    document.head.appendChild(sidebarStyle);

    const outer = document.createElement("div");
    outer.id = "gbf-sidebar-outer";
    outer.classList.add("gbf-sidebar-hidden"); // faded in once positioning is confirmed — see revealSidebar()
    sidebarEl = outer;

    const sidebar = document.createElement("div");
    sidebar.id = "gbf-simple-sidebar";
    sidebarInnerEl = sidebar;

    // The scrolling region. Everything except the actions footer lives in
    // here, so the footer can never overlay a nav item.
    const navScroll = document.createElement("div");
    navScroll.id = "gbf-nav-scroll";
    sidebar.appendChild(navScroll);
    navScrollEl = navScroll;

    // --- collapse toggle ---
    const toggle = document.createElement("button");
    toggle.id = "gbf-toggle";
    toggle.title = "Collapse / expand sidebar (Alt+\\)";
    toggleBtnEl = toggle;
    navScroll.appendChild(toggle);

    // --- nav buttons ---
    const navButtons = [];

    NAV.forEach((item) => {
      if (item.section) {
        const t = document.createElement("div");
        t.className = "gbf-section-title";
        t.textContent = item.section;
        t.dataset.full = item.section;
        navScroll.appendChild(t);
        return;
      }

      const btn = document.createElement("button");
      btn.className = "gbf-nav-btn";
      if (item.hash) btn.dataset.hash = item.hash;
      btn._navItem = item; // used by the keyboard shortcut handler below, works for hash or action items alike

      let titleText = item.label;
      if (item.key) {
        titleText += "  (Alt+" + item.key.toUpperCase() + ")";
      }
      btn.title = titleText;

      const ic = document.createElement("span");
      ic.className = "gbf-icon";
      ic.textContent = item.icon || "\u25CF";

      const lb = document.createElement("span");
      lb.className = "gbf-label";
      lb.textContent = item.label;

      btn.appendChild(ic);
      btn.appendChild(lb);

      if (item.key) {
        const kb = document.createElement("span");
        kb.className = "gbf-key";
        kb.textContent = "Alt+" + item.key.toUpperCase();
        btn.appendChild(kb);
      }

      btn.addEventListener("click", () => {
        flashButton(btn);
        if (item.action) item.action();
        else go(item.hash);
      });
      navScroll.appendChild(btn);
      navButtons.push(btn);
    });

    // --- actions (sticky footer: stays visible regardless of sidebar
    // scroll position/height — see #gbf-sidebar-actions CSS above) ---
    const actionsFooter = document.createElement("div");
    actionsFooter.id = "gbf-sidebar-actions";

    const actTitle = document.createElement("div");
    actTitle.className = "gbf-section-title";
    actTitle.textContent = "ACTIONS";
    actionsFooter.appendChild(actTitle);

    const row = document.createElement("div");
    row.className = "gbf-row";

    const backBtn = document.createElement("button");
    backBtn.className = "gbf-nav-btn";
    backBtn.title = "Back (Alt+Left)";
    backBtn.innerHTML =
      '<span class="gbf-icon">\u2190</span><span class="gbf-label">Back</span>';
    backBtn.addEventListener("click", () => {
      flashButton(backBtn);
      history.back();
    });

    const reloadBtn = document.createElement("button");
    reloadBtn.className = "gbf-nav-btn";
    reloadBtn.title = "Reload (Alt+R)";
    reloadBtn.innerHTML =
      '<span class="gbf-icon">\u21BB</span><span class="gbf-label">Reload</span>';
    reloadBtn.addEventListener("click", () => {
      flashButton(reloadBtn);
      location.reload();
    });

    row.appendChild(backBtn);
    row.appendChild(reloadBtn);
    actionsFooter.appendChild(row);

    const lockBtn = document.createElement("button");
    lockBtn.className = "gbf-nav-btn";
    lockBtn.id = "gbf-lock-toggle";
    actionsFooter.appendChild(lockBtn);

    // Footer goes on the SIDEBAR, not in the scroller — that is the whole
    // point of the restructure. See the #gbf-sidebar-actions CSS.
    sidebar.appendChild(actionsFooter);

    outer.appendChild(sidebar);
    document.body.appendChild(outer);

    // --- behaviour ---
    function go(hash) {
      if (location.hash === hash) {
        location.hash = "";
      }
      location.hash = hash;
    }

    function markActive() {
      const h = location.hash || "";
      let best = null;
      navButtons.forEach((b) => {
        b.classList.remove("gbf-active");
        const bh = b.dataset.hash;
        if (!bh) return; // action-type buttons (e.g. Wiki) never map to a GBF hash
        if (h === bh || h.indexOf(bh + "/") === 0) {
          if (!best || bh.length > best.dataset.hash.length) best = b;
        }
      });
      if (best) best.classList.add("gbf-active");
    }

    async function toggleCollapsed() {
      // Read the EFFECTIVE state, not the manual flag. The nav can be icon-only
      // for three reasons — manual, autoNarrow (no room) and the wiki forcing
      // it — and the button shows one arrow for all of them. Toggling `collapsed`
      // blindly meant that when the nav was auto-collapsed, pressing expand set
      // collapsed = true and nothing appeared to happen, then pressing it again
      // set it back. The button did nothing, twice.
      const wantExpand = isEffectivelyCollapsed();

      if (wantExpand) {
        // Make room the same way the wiki does: grow the window rather than
        // squeeze the game. Without this, expanding in a window that has no
        // space either does nothing (autoNarrow immediately re-collapses) or
        // takes the width from the game.
        const edge = getGameRightEdge();
        const available = edge == null ? 0 : window.innerWidth - edge;
        // Aim just past AUTO_COLLAPSE_BELOW (250), NOT past
        // AUTO_EXPAND_AT_OR_ABOVE (255).
        //
        // Those are the two halves of positionSidebar()'s hysteresis: it
        // collapses below 250 and re-expands at 255. The 255 figure only
        // governs the case where autoNarrow is left set and positionSidebar()
        // has to notice the room itself. We clear autoNarrow directly below,
        // so the only threshold that can still act on us is the re-collapse
        // one at 250 — anything at or above that is stable.
        //
        // Widening to exactly 250 did fail, but because it landed *on* the
        // boundary, not because 255 was required. 4px of slack covers rounding
        // in the css-to-physical conversion without taking more of the screen
        // than the nav actually needs.
        // SIDEBAR_W, not AUTO_COLLAPSE_BELOW. These were the same number until
        // the collapse threshold was relaxed to 205 to stop the nav folding
        // over a few pixels; after that, aiming at the threshold made expand
        // ask for only 209px and the nav came back 208 wide instead of 250 --
        // visible once the window hugs the content and there is no surplus to
        // absorb the difference. The target is the width the nav actually
        // wants; the threshold is a separate question.
        const needed = SIDEBAR_W + 4 + (wikiIsOpen() ? wikiPanelWidth : 0);
        const shortfall = Math.max(0, needed - available);
        if (shortfall > 0) {
          const gained = await tryWidenWindowBy(shortfall);
          if (gained > 0) navWidenedBy += gained;
        }
        collapsed = false;
        // Clear both forced-narrow reasons here rather than waiting for
        // positionSidebar(). The user asked for this explicitly, and the
        // window resize has not reached window.innerWidth yet, so
        // positionSidebar() would still measure the old width and leave the
        // nav collapsed. If the room genuinely is not there, the next
        // positionSidebar() re-collapses it, which is the correct outcome.
        autoNarrow = false;
        wikiForcedNarrow = false;
      } else {
        collapsed = true;
      }

      store.set("gbfCollapsed", collapsed ? "1" : "0");
      applyCollapsedVisual();
      positionSidebar();
      // The OS resize lands a few frames after setSize resolves, so measure
      // again once it has, and let positionSidebar() settle the final state.
      if (wantExpand) setTimeout(positionSidebar, 160);

      // Collapsing hands back exactly what expanding took.
      if (!wantExpand && navWidenedBy > 0) {
        const give = navWidenedBy;
        navWidenedBy = 0;
        await resizeWindowByCss(-give);
      }
    }

    toggle.addEventListener("click", () => {
      flashButton(toggle);
      toggleCollapsed();
    });

    function applyLockButton() {
      lockBtn.innerHTML = locked
        ? '<span class="gbf-icon">\u{1F512}</span><span class="gbf-label">Locked</span>'
        : '<span class="gbf-icon">\u{1F513}</span><span class="gbf-label">Unlocked</span>';
      lockBtn.title =
        (locked
          ? "Locked: game stays compact, GBF panel hidden"
          : "Unlocked: GBF panel can appear at wide widths") + "  (Alt+L)";
      lockBtn.classList.toggle("gbf-active", locked);
    }

    function toggleLocked() {
      locked = !locked;
      store.set("gbfLocked", locked ? "1" : "0");
      // Unlocking gives the reclaimed area back to GBF's own chat/help panel,
      // so the wiki has nowhere to live. Close it rather than let the two
      // fight over the same region.
      if (!locked && wikiIsOpen()) closeWikiPanel();
      applyLockButton();
      applyLockState();
    }

    lockBtn.addEventListener("click", () => {
      flashButton(lockBtn);
      toggleLocked();
    });

    // --- drag-to-scroll (with momentum on release) ---
    const sidebarMomentum = createMomentumScroller(() => navScroll);

    sidebar.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return; // primary button/touch only
      sidebarMomentum.stop(); // a new touch cancels any in-flight coast
      sidebarMomentum.reset();
      dragScrolling = true;
      dragScrollMoved = false;
      dragScrollStartY = e.clientY;
      dragScrollStartTop = navScroll.scrollTop;
      sidebarMomentum.sample(e.clientY);
    });

    sidebar.addEventListener("pointermove", (e) => {
      if (!dragScrolling) return;
      const delta = e.clientY - dragScrollStartY;
      if (!dragScrollMoved && Math.abs(delta) > DRAG_SCROLL_THRESHOLD) {
        dragScrollMoved = true;
        sidebar.classList.add("gbf-drag-scrolling");
      }
      if (dragScrollMoved) {
        navScroll.scrollTop = dragScrollStartTop - delta;
        sidebarMomentum.sample(e.clientY);
        e.preventDefault();
      }
    });

    function endDragScroll() {
      if (dragScrolling && dragScrollMoved) sidebarMomentum.release();
      dragScrolling = false;
      sidebar.classList.remove("gbf-drag-scrolling");
      // Same mechanism as endPageDragScroll(): record when the drag ended and
      // let the click handler decide on elapsed time.
      if (dragScrollMoved) dragScrollEndedAt = Date.now();
      dragScrollMoved = false;
    }
    window.addEventListener("pointerup", endDragScroll);
    window.addEventListener("pointercancel", endDragScroll);

    // Capturing so this runs before a nav/action button's own click
    // handler (registered in the bubble phase) — if the pointer
    // actually dragged, swallow the click entirely so a drag never
    // also fires navigation, then clear the flag for the next gesture.
    sidebar.addEventListener(
      "click",
      (e) => {
        if (Date.now() - dragScrollEndedAt < DRAG_CLICK_SUPPRESS_MS) {
          dragScrollEndedAt = 0; // one click per drag
          e.preventDefault();
          e.stopPropagation();
        }
      },
      true,
    );

    // Keyboard shortcuts
    window.addEventListener("keydown", (e) => {
      if (!e.altKey || e.ctrlKey || e.metaKey) return;
      const key = e.key.toLowerCase();
      if (e.key === "\\") {
        flashButton(toggle);
        toggleCollapsed();
        e.preventDefault();
        return;
      }
      if (key === "l") {
        flashButton(lockBtn);
        toggleLocked();
        e.preventDefault();
        return;
      }
      if (key === "r") {
        flashButton(reloadBtn);
        location.reload();
        e.preventDefault();
        return;
      }
      if (e.key === "ArrowLeft") {
        flashButton(backBtn);
        history.back();
        e.preventDefault();
        return;
      }

      const item = NAV.filter((i) => i.key).find((i) => i.key === key);
      if (item) {
        const b = navButtons.find((nb) => nb._navItem === item);
        if (b) flashButton(b);
        if (item.action) item.action();
        else go(item.hash);
        e.preventDefault();
      }
    });

    window.addEventListener("hashchange", markActive);
    applyCollapsedVisual();
    applyLockButton();
    markActive();
  }

  // ------------------------------------------------------------------
  // Wiki panel — gbf.wiki in a plain iframe, in the reserved area
  // ------------------------------------------------------------------
  // There used to be a second "Embedded" variant that fetched articles through
  // MediaWiki's action API and rendered them into a srcdoc iframe so links
  // could be intercepted. Removed 2026-09-03: it only ever rendered the article
  // body without the wiki's own skin, so it neither looked nor behaved like the
  // site. The plain iframe does, and that is what actually matters here.
  //
  // The trade-off we accept with an ordinary cross-origin iframe: nothing
  // inside it can be scripted — no reading its DOM, no intercepting clicks, no
  // knowing which page it is on. That is the browser's same-origin policy, not
  // a Pake limitation, and there is no workaround. Internal wiki browsing is
  // just ordinary iframe navigation, and Pake's own injected link handling
  // deals with links that leave the wiki.
  //
  // Consequences worth remembering before adding features here: there can be
  // no working Back button (cross-origin history is unreadable) and no "what
  // page am I on" display. Home and Search work only because both are done by
  // SETTING src, which needs no access to the frame's contents.
  const WIKI_BASE = "https://gbf.wiki";
  const WIKI_HOME_TITLE = "Main_Page";

  // --- Resizing the OS window to make room for the wiki -----------------
  // window.__TAURI__ IS available in Pake builds (tauri.conf.json sets
  // withGlobalTauri: true), and we grant the two window capabilities this
  // needs in src-tauri/capabilities/default.json — see
  // PAKE_UPSTREAM_PATCHES.md for the patches and how to re-apply them.
  //
  // WORK IN PHYSICAL PIXELS, AND NEVER RE-DERIVE THE HEIGHT.
  //
  // setSize() takes an INNER size, so passing window.innerHeight looks like it
  // should be a no-op. It is not. window.innerHeight is CSS pixels; Tauri's
  // logical pixels are physical/scaleFactor, and Pake applies a page zoom that
  // devicePixelRatio folds in, so the two differ. Every call therefore resized
  // the window slightly — observed in play: the window lost ~25px of height on
  // each wiki click, compounding.
  //
  // Reading innerSize() gives physical pixels, and echoing that exact height
  // straight back cannot drift, whatever the zoom or DPI. The width delta is
  // converted with a ratio measured from the same reading, so it needs no
  // assumption about devicePixelRatio either.
  let wikiWidenedBy = 0; // css px we added, to give back when the wiki closes
  let wikiOpenedAt = 0; // suppresses the auto-close race right after opening

  function tauriWindowApi() {
    const t = window.__TAURI__;
    if (!t || !t.window) return null;
    const getWin = t.window.getCurrentWindow;
    if (typeof getWin !== "function") return null;
    // PhysicalSize lives in the dpi module; window re-exports only the
    // Logical* pair, so check both rather than assuming either.
    const PhysicalSize =
      (t.dpi && t.dpi.PhysicalSize) || t.window.PhysicalSize || null;
    if (typeof PhysicalSize !== "function") return null;
    return { getWin: getWin, PhysicalSize: PhysicalSize };
  }

  function wikiDebug(msg, extra) {
    if (window.localStorage && localStorage.getItem("gbfDebug") === "1") {
      console.log("[gbf-wiki] " + msg, extra === undefined ? "" : extra);
    }
  }

  // Adds deltaCssPx to the window width, preserving height exactly.
  // Resolves with the css px actually gained (0 if it could not resize).
  async function resizeWindowByCss(deltaCssPx) {
    // ONE CHOKE POINT: under Automatic Resizing we never resize the window.
    //
    // GBF reloads itself when the window changes size substantially -- a
    // marker on `window` did not survive 891 -> 1500. Every caller here
    // (wiki widen, collapse hug, nav expand) would therefore restart Granblue
    // mid-play, and the reload also wipes the state that hands the width back:
    // measured, opening the wiki at a 900px window left it at 1606 with GBF
    // reloaded, the wiki closed and widenedBy reset to 0. The window stayed
    // stretched with nothing to show for it.
    //
    // Every caller already handles "gained 0" -- the wiki falls back to a
    // narrower tier or the existing notice, and expanding just uses the room
    // that is there. So refusing here degrades cleanly instead of needing a
    // branch in each of them.
    if (gbfUsesAutomaticResizing()) return 0;
    const api = tauriWindowApi();
    if (!api) {
      wikiDebug("no Tauri window API - cannot resize");
      return 0;
    }
    try {
      const win = api.getWin();
      const size = await win.innerSize(); // PhysicalSize
      if (!size || !size.width || !size.height) return 0;

      // Physical px per css px, measured rather than assumed. Covers display
      // scaling and Pake's page zoom together.
      const ratio = size.width / window.innerWidth;
      if (!isFinite(ratio) || ratio <= 0) return 0;

      const curCssW = window.innerWidth;
      // NO screenX HERE. This used to clamp growth to
      // `screen.availWidth - window.screenX`, and that is the same unreliable
      // prediction that was removed from openWikiPanel() — leaving it here
      // meant the resize quietly under-delivered, the measured space landed a
      // tier short, and the wiki opened at 800px inside a window that had
      // room for 960. Widening by hand first made it work, because less growth
      // was then needed for the clamp to bite. Do not reintroduce it.
      //
      // availWidth alone is still a sane ceiling — a window has no business
      // being wider than the screen — but only trust it when it is actually
      // larger than the window we already have. If it reports something
      // smaller than the current window it is telling us something impossible,
      // and it must not be allowed to block growth.
      const avail = (window.screen && window.screen.availWidth) || 0;
      const ceiling = avail > curCssW ? avail : Infinity;

      let targetCssW = Math.min(curCssW + deltaCssPx, ceiling);
      targetCssW = Math.max(320, targetCssW);

      const gained = Math.round(targetCssW - curCssW);
      if (gained === 0) return 0;

      const targetPhysW = Math.round(size.width + gained * ratio);
      // size.height is echoed back untouched. This is the whole point.
      await win.setSize(new api.PhysicalSize(targetPhysW, size.height));
      return gained;
    } catch (err) {
      // Tauri rejects when a capability is missing, and that used to be
      // invisible. Surface it under the debug flag instead of assuming success.
      wikiDebug("resize failed", err);
      return 0;
    }
  }

  async function tryWidenWindowBy(deltaPx) {
    if (deltaPx <= 0) return 0;
    return await resizeWindowByCss(deltaPx);
  }

  // Give back the width the wiki borrowed -- but ONLY if the window is still
  // the size we left it.
  //
  // Handing it back unconditionally is wrong the moment the user resizes the
  // window while the wiki is open, because the borrowed width is no longer
  // there to give. Measured: opened at 884 (widened to 1852, borrowing 968),
  // then dragged the window to 1500. That is too narrow for game + sidebar +
  // panel, so the wiki auto-closed and handed back all 968 -- collapsing the
  // window to 534. A small drag by the user produced a window a third of the
  // size they asked for.
  //
  // If the width does not match what we left, the user has taken control of
  // it. Their size wins: drop the debt and change nothing.
  async function restoreWindowWidth() {
    if (wikiWidenedBy <= 0) return;
    const give =
      wikiWidthBeforeWiden > 0
        ? window.innerWidth - wikiWidthBeforeWiden
        : wikiWidenedBy;
    wikiWidenedBy = 0;
    wikiWidthBeforeWiden = 0;
    if (give <= 0) {
      wikiWidthAfterWiden = 0;
      return;
    }
    if (
      wikiWidthAfterWiden > 0 &&
      Math.abs(window.innerWidth - wikiWidthAfterWiden) > 8
    ) {
      wikiWidthAfterWiden = 0;
      return;
    }
    wikiWidthAfterWiden = 0;
    await resizeWindowByCss(-give);
  }

  // Small transient message inside the nav, so a click that cannot act is
  // never silently ignored.
  let noticeTimer = null;
  function showSidebarNotice(text) {
    const host = navScrollEl || sidebarInnerEl;
    if (!host) return;
    let el = host.querySelector("#gbf-sidebar-notice");
    if (!el) {
      el = document.createElement("div");
      el.id = "gbf-sidebar-notice";
      host.appendChild(el);
    }
    el.textContent = text;
    el.classList.add("gbf-notice-show");
    if (noticeTimer) clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => {
      el.classList.remove("gbf-notice-show");
      noticeTimer = null;
    }, 4000);
  }
  const wikiPanelStyle = document.createElement("style");
  wikiPanelStyle.textContent = `
        /* flex-shrink is 1 ON PURPOSE here, and 0 on the nav.
           If the window ends up narrower than nav + panel — a resize that
           under-delivered, or a screen with no room left — something has to
           give. Without shrink the panel overflowed the window and was clipped
           on the right, taking the close button with it, so the wiki could not
           even be dismissed. Observed in play 2026-09-03. Shrinking the panel
           is recoverable; clipping it is not. The nav must never be the thing
           that gives. */
        #gbf-wiki-panel {
            flex: 0 1 ${WIKI_PANEL_WIDTH}px; /* overridden per open, see openWikiPanel */
            min-width: 0;
            max-width: 100%;
            height: 100%;
            box-sizing: border-box;
            background: #0f1a26;
            border-left: 1px solid rgba(201,169,110,0.35);
            display: none;
            flex-direction: column;
            opacity: 0;
            transition: opacity .15s ease;
        }
        #gbf-wiki-panel.gbf-wiki-open { display: flex; }
        #gbf-wiki-panel.gbf-wiki-shown { opacity: 1; }
        #gbf-wiki-header {
            display: flex;
            align-items: center;
            gap: 6px;
            padding: 8px;
            background: #16222f;
            border-bottom: 1px solid rgba(201,169,110,0.25);
            flex: 0 0 auto;
        }
        #gbf-wiki-header button {
            background: transparent;
            border: 1px solid #5c4a2e;
            border-radius: 4px;
            color: #c9a96e;
            cursor: pointer;
            font-size: 14px;
            padding: 5px 8px;
            line-height: 1;
            font-family: inherit;
        }
        #gbf-wiki-header button:hover { background: rgba(201,169,110,0.15); }
        #gbf-wiki-search {
            flex: 1 1 auto;
            min-width: 0;
            background: #0a1622;
            border: 1px solid #3a3020;
            border-radius: 4px;
            color: #ddd;
            font-family: inherit;
            font-size: 12px;
            padding: 6px 8px;
        }
        #gbf-wiki-body {
            flex: 1 1 auto;
            min-height: 0;
            position: relative;
        }
        /* About page. Scrolls on its own so a short window still reaches the
           bottom, and caps its line length so the text stays readable when the
           panel opens at the wiki's much wider tier. */
        #gbf-about-body {
            flex: 1 1 auto;
            min-height: 0;
            overflow-y: auto;
            padding: 18px 22px 26px;
            color: #e8e2d2;
            font-family: "Georgia", "Times New Roman", serif;
            font-size: 13px;
            line-height: 1.55;
        }
        #gbf-about-body > * { max-width: 620px; }
        #gbf-about-body h1 {
            font-size: 19px;
            margin: 0 0 6px;
            color: #f2e2b6;
            letter-spacing: .5px;
        }
        #gbf-about-body h2 {
            font-size: 13px;
            letter-spacing: 2px;
            text-transform: uppercase;
            color: #e9d097;
            margin: 20px 0 6px;
            border-bottom: 1px solid rgba(201,169,110,0.35);
            padding-bottom: 3px;
        }
        #gbf-about-body p.gbf-about-lede { color: #cfc7b4; margin: 0 0 4px; }
        #gbf-about-body ul { margin: 4px 0; padding-left: 18px; }
        #gbf-about-body li { margin: 5px 0; }
        #gbf-about-body b { color: #f2e2b6; font-weight: 600; }
        #gbf-about-body table.gbf-about-keys { border-collapse: collapse; width: 100%; }
        #gbf-about-body table.gbf-about-keys td { padding: 2px 0; vertical-align: top; }
        #gbf-about-body table.gbf-about-keys td:last-child {
            text-align: right;
            white-space: nowrap;
            width: 1%;
        }
        #gbf-about-body kbd {
            display: inline-block;
            padding: 1px 5px;
            border: 1px solid #5c4a2e;
            border-bottom-width: 2px;
            border-radius: 3px;
            background: rgba(0,0,0,0.35);
            color: #f2e2b6;
            font-family: inherit;
            font-size: 11px;
        }
        #gbf-panel-title {
            flex: 1 1 auto;
            color: #f2e2b6;
            font-family: "Georgia", "Times New Roman", serif;
            font-size: 13px;
            letter-spacing: 1px;
            padding-left: 4px;
        }
        #gbf-wiki-body iframe {
            width: 100%;
            height: 100%;
            border: 0;
            background: #1b1b1b;
        }
    `;
  document.head.appendChild(wikiPanelStyle);

  function ensureWikiPanel() {
    if (wikiPanelEl) return wikiPanelEl;

    const panel = document.createElement("div");
    panel.id = "gbf-wiki-panel";
    panel.innerHTML =
      '<div id="gbf-wiki-header">' +
      '<button id="gbf-wiki-home" title="Main Page">⌂</button>' +
      '<input id="gbf-wiki-search" placeholder="Jump to a wiki page…" />' +
      '<span id="gbf-panel-title"></span>' +
      '<button id="gbf-wiki-close" title="Close">✕</button>' +
      "</div>" +
      '<div id="gbf-wiki-body"></div>' +
      '<div id="gbf-about-body"></div>';

    // Into the sidebar's outer wrapper, AFTER the nav, so flex lays it out in
    // the reserved area. Never document.body — it would draw underneath the
    // sidebar and spill over the game.
    if (!sidebarEl) return null;
    sidebarEl.appendChild(panel);
    wikiPanelEl = panel;

    panel
      .querySelector("#gbf-wiki-close")
      .addEventListener("click", closeWikiPanel);

    panel.querySelector("#gbf-wiki-home").addEventListener("click", () => {
      if (wikiFrameEl) wikiFrameEl.src = WIKI_BASE + "/" + WIKI_HOME_TITLE;
    });

    panel.querySelector("#gbf-wiki-search").addEventListener("keydown", (e) => {
      if (e.key !== "Enter") return;
      const q = e.target.value.trim();
      if (!q || !wikiFrameEl) return;
      wikiFrameEl.src =
        WIKI_BASE + "/" + encodeURIComponent(q.replace(/ /g, "_"));
    });

    panel.querySelector("#gbf-about-body").innerHTML = aboutHtml();

    return panel;
  }

  // Everything here is a thing a user cannot discover by looking at the app:
  // shortcuts, behaviour that looks like a bug but is not, and limits with a
  // reason behind them. Kept honest -- the limitations section says what does
  // not work, not what we wish worked.
  function aboutHtml() {
    const keyRows = NAV.filter((i) => i.key && i.label)
      .map(
        (i) =>
          "<tr><td>" +
          i.label +
          "</td><td><kbd>Alt</kbd>+<kbd>" +
          String(i.key).toUpperCase() +
          "</kbd></td></tr>",
      )
      .join("");

    return (
      "<h1>Granblue Fantasy Pake</h1>" +
      "<p class='gbf-about-lede'>A desktop wrapper for Granblue Fantasy with a " +
      "navigation sidebar and a built-in wiki panel. The game itself runs " +
      "exactly as it does in a browser — nothing here modifies it.</p>" +
      "<h2>Keyboard shortcuts</h2>" +
      "<table class='gbf-about-keys'>" +
      keyRows +
      "<tr><td>Collapse / expand the sidebar</td><td><kbd>Alt</kbd>+<kbd>\\</kbd></td></tr>" +
      "<tr><td>Lock / unlock the sidebar</td><td><kbd>Alt</kbd>+<kbd>L</kbd></td></tr>" +
      "<tr><td>Reload the page</td><td><kbd>Alt</kbd>+<kbd>R</kbd></td></tr>" +
      "</table>" +
      "<h2>Things you may not have noticed</h2>" +
      "<ul>" +
      "<li><b>Drag to scroll.</b> Click and drag anywhere that scrolls — the " +
      "game's own pages as well as this sidebar — and it scrolls with the " +
      "drag, with momentum. A drag never activates the link underneath it.</li>" +
      "<li><b>Locked mode.</b> Locked (the padlock, or <kbd>Alt</kbd>+<kbd>L</kbd>) " +
      "hides Granblue's own chat/help column and gives that space to the " +
      "sidebar and the wiki. Unlocked, the sidebar is a plain strip and the " +
      "wiki cannot open — there is nowhere to put it.</li>" +
      "<li><b>The sidebar collapses itself</b> when the window gets too narrow " +
      "for labels, and expands again when the room comes back.</li>" +
      "<li><b>Spacing adapts.</b> Nav items tighten or relax so the list fits " +
      "your window height without a scrollbar.</li>" +
      "<li><b>Wiki links stay in the panel.</b> Links to other wiki pages open " +
      "in place; genuinely external links open in your normal browser.</li>" +
      "</ul>" +
      "<h2>Limitations, and why</h2>" +
      "<ul>" +
      "<li><b>With Automatic Resizing on, the window is never resized for you.</b> " +
      "Granblue reloads itself whenever the window changes size, so making " +
      "room for the wiki would restart your game mid-session. The wiki " +
      "therefore only opens if the window is already wide enough — widen it " +
      "yourself, or switch to a fixed Window Size in Granblue's Browser " +
      "Settings.</li>" +
      "<li><b>Empty space beside the game under Automatic Resizing.</b> That gap " +
      "is Granblue's own chat column, which locked mode hides. It cannot be " +
      "reclaimed: the game only recalculates its zoom on a real window " +
      "resize, and resizing reloads it. A fixed Window Size has no gap — the " +
      "window hugs the game and sidebar exactly.</li>" +
      "<li><b>Closing the wiki forgets your place.</b> The page is unloaded so it " +
      "stops using memory and network while closed, so it reopens at the " +
      "main page.</li>" +
      "<li><b>The wiki needs about 800px</b> to be readable. If there is less, " +
      "it will collapse the sidebar to find the room, and refuse if there " +
      "still is not enough.</li>" +
      "</ul>" +
      "<h2>Granblue is never modified</h2>" +
      "<p>The app reads the game's layout to place the sidebar, and hides its " +
      "chat column in locked mode. It never changes the game's settings, its " +
      "code, or its network traffic. Granblue runs as it would in any " +
      "browser.</p>"
    );
  }

  // Which panel width can we actually achieve on THIS screen, with the game at
  // the width it is currently set to?
  //
  // The maintainer has a 2K monitor; someone else may be running GBF at its
  // Large fixed size on a 1080p screen, where the preferred panel simply will
  // not fit. Rather than opening something cut off, pick the widest tier that
  // fits, and if neither does, say so — the user can narrow the game in GBF's
  // own Browser Settings, which is the only lever that actually frees space.
  //
  // Returns 0 when nothing fits.
  // Waits for an OS resize to actually reach the page. setSize resolves when
  // the command is accepted, not when window.innerWidth reflects it, and GBF
  // relayouts behind a 20ms debounced ResizeObserver on top of that.
  function settleLayout() {
    return new Promise((resolve) => {
      requestAnimationFrame(() =>
        requestAnimationFrame(() => setTimeout(resolve, 120)),
      );
    });
  }

  async function openWikiPanel(mode) {
    panelBusy = true;
    try {
      return await openWikiPanelInner(mode);
    } finally {
      panelBusy = false;
    }
  }

  async function openWikiPanelInner(mode) {
    mode = mode === "about" ? "about" : "wiki";
    const isAbout = mode === "about";
    const label = isAbout ? "About page" : "wiki";
    const prefer = isAbout ? ABOUT_PANEL_WIDTH : WIKI_PANEL_WIDTH;
    const minimum = isAbout ? ABOUT_PANEL_WIDTH_MIN : WIKI_PANEL_WIDTH_MIN;

    // Each button toggles ITS OWN view. Pressing Wiki while the About page is
    // showing switches to the wiki rather than closing the panel -- closing
    // and reopening would give the window back and take it again for no
    // reason.
    if (wikiIsOpen()) {
      if (panelMode === mode) {
        closeWikiPanel();
        return;
      }
      // Switching views. If the panel is currently too narrow for the one
      // being switched TO, re-open properly instead: About opens happily at
      // 300px, and inheriting that would have handed the wiki a 300px panel.
      if (wikiPanelWidth < minimum) {
        const previous = panelMode;
        closeWikiPanel();
        await settleLayout();
        await openWikiPanelInner(mode);
        // The wiki needs far more room than the About page, so this switch can
        // legitimately fail. Do not leave the user with nothing: put back what
        // they were reading. The notice from the failed attempt still explains
        // why it did not switch.
        if (!wikiIsOpen()) {
          panelMode = previous;
          await openWikiPanelInner(previous);
        }
        return;
      }
      panelMode = mode;
      applyPanelMode();
      if (!isAbout) ensureWikiFrame();
      return;
    }

    // The panel lives in space we reserve next to the game. Unlocked, that
    // space does not exist — the outer is a plain fixed-width strip and GBF's
    // own chat/help panel occupies that region instead.
    if (!locked) {
      showSidebarNotice("Lock the sidebar (Alt+L) to open the " + label + ".");
      return;
    }

    panelMode = mode;

    // TRY, THEN MEASURE. Do not try to predict whether there is room.
    //
    // This used to decide up front from screen.availWidth and window.screenX.
    // Those are not reliable in the Pake webview: the wiki refused to open on
    // a screen with plenty of space, and manually dragging the window wider
    // fixed it — which is exactly the signature of the prediction being wrong
    // while the real limit was fine. Asking the OS to resize and then looking
    // at what we actually got needs no screen metrics at all and cannot be
    // wrong in that way.
    //
    // The target is measured against the game's width BEFORE any resize. That
    // matters in Automatic Resizing mode, where the game grows into whatever
    // the window gains until we reserve the room — measuring free space after
    // the fact would read zero and look like failure.
    const edge = getGameRightEdge();
    const gameW = edge == null ? 0 : edge;

    const reachFor = async (panelWidth) => {
      const target = gameW + sidebarWidth() + panelWidth + WIKI_WIDEN_BUFFER;
      const shortfall = target - window.innerWidth;
      if (shortfall > 0) {
        if (wikiWidenedBy === 0 && wikiWidthBeforeWiden === 0)
          wikiWidthBeforeWiden = window.innerWidth;
        const gained = await tryWidenWindowBy(shortfall);
        if (gained > 0) wikiWidenedBy += gained;
        await settleLayout();
      }
      // What we actually ended up with, for the game at its pre-resize width.
      return window.innerWidth - gameW - sidebarWidth();
    };

    let space = await reachFor(prefer);

    // Still short? Collapsing the nav frees SIDEBAR_W - SIDEBAR_W_COLLAPSED,
    // about 198px, which is often the difference between no wiki and the
    // fallback tier. Try it before giving up.
    let collapsedForWiki = false;
    if (space < minimum && !isEffectivelyCollapsed()) {
      wikiForcedNarrow = true;
      collapsedForWiki = true;
      applyCollapsedVisual();
      positionSidebar();
      await settleLayout();
      space = await reachFor(prefer);
    }

    // Pick a tier from what we really have, not from what we hoped for —
    // with a little slack, because dropping a whole tier over a few pixels is
    // a much worse outcome than the panel being marginally narrower. The panel
    // is flex-shrinkable with max-width:100%, so if the space is slightly
    // under the tier it renders a little narrower rather than overflowing.
    const tier =
      space >= prefer - WIKI_TIER_SLACK
        ? prefer
        : space >= minimum
          ? minimum
          : 0;
    // RESERVE WHAT EXISTS, NOT THE NOMINAL TIER.
    //
    // The slack above lets a 960 tier be picked when only 957px is there --
    // fine for RENDERING, since the panel flex-shrinks. But wikiPanelWidth is
    // also what applyBodyInset() reserves and what the auto-close measures
    // against, and reserving 3px that do not exist made the auto-close
    // conclude the game had been squeezed below its minimum. It then closed
    // the panel it had just opened.
    //
    // Seen in Small, where the game is exactly MIN_GAME_WIDTH and has no
    // headroom to absorb the difference: widened to 1527 of a 1532 target
    // (rounding), reserved 960 against 957 available, computed 317 for a game
    // that needs 320, and closed at ~860ms every time. The wiki was
    // unopenable at that size.
    const chosen = tier === 0 ? 0 : Math.min(tier, Math.floor(space));

    if (chosen === 0) {
      // Put everything back the way it was before we started experimenting.
      if (collapsedForWiki) {
        wikiForcedNarrow = false;
        applyCollapsedVisual();
      }
      await restoreWindowWidth();
      positionSidebar();
      wikiDebug("no room for the wiki", {
        gameW: gameW,
        sidebar: sidebarWidth(),
        innerWidth: window.innerWidth,
        spaceAchieved: space,
      });
      // Both levers, because which one helps depends on the mode: a smaller
      // fixed Window Size frees a fixed amount, while Automatic Resizing lets
      // the game shrink to whatever is left.
      showSidebarNotice(
        "Not enough room. In Granblue's Browser Settings, pick a smaller Window Size or turn on Automatic Resizing.",
      );
      return;
    }

    if (collapsedForWiki) {
      showSidebarNotice(
        "Sidebar collapsed to make room for the " + label + ".",
      );
    }
    wikiPanelWidth = chosen;
    wikiDebug("opening wiki", {
      gameWidthBefore: gameW,
      sidebar: sidebarWidth(),
      innerWidth: window.innerWidth,
      spaceAchieved: space,
      tierChosen: chosen,
      collapsedNav: collapsedForWiki,
    });

    ensureWikiPanel();
    if (!wikiPanelEl) return;

    wikiPanelEl.classList.add("gbf-wiki-open");
    wikiPanelEl.style.flexBasis = wikiPanelWidth + "px";
    applyPanelMode();
    wikiOpenedAt = Date.now();
    // Remember the width we leave the window at, so restoreWindowWidth() can
    // tell "unchanged since we widened" from "the user resized it".
    wikiWidthAfterWiden = window.innerWidth;
    // The panel is open, so the reserved margin has to grow to match and the
    // sidebar has to re-measure against the game's new edge.
    applyBodyInset();
    positionSidebar();
    // Fade in a frame later, so the browser commits the opacity:0 state first
    // — same reason revealSidebar() does it.
    requestAnimationFrame(
      () => wikiPanelEl && wikiPanelEl.classList.add("gbf-wiki-shown"),
    );

    if (panelMode === "wiki") ensureWikiFrame();
  }

  // Built lazily, and NOT only when the panel is first created: opening the
  // About page first and then switching to the wiki left the wiki body empty,
  // because creation used to happen once at panel construction.
  function ensureWikiFrame() {
    if (wikiFrameEl || !wikiPanelEl) return;
    const body = wikiPanelEl.querySelector("#gbf-wiki-body");
    if (!body) return;
    const iframe = document.createElement("iframe");
    iframe.id = "gbf-wiki-iframe";
    iframe.src = WIKI_BASE + "/" + WIKI_HOME_TITLE;
    body.appendChild(iframe);
    wikiFrameEl = iframe;
  }

  function closeWikiPanel() {
    if (!wikiPanelEl) return;
    // Held until restoreWindowWidth() has handed the width back, so the hug
    // cannot shrink the window twice for the same space.
    panelBusy = true;
    setTimeout(() => {
      panelBusy = false;
    }, 700);
    wikiPanelEl.classList.remove("gbf-wiki-shown");
    wikiPanelEl.classList.remove("gbf-wiki-open");
    // Give the nav back if the wiki was the reason it collapsed.
    if (wikiForcedNarrow) {
      wikiForcedNarrow = false;
      applyCollapsedVisual();
    }
    // Release the reserved room first, so the game reclaims it, then hand back
    // exactly the window width we took.
    applyBodyInset();
    positionSidebar();
    // Fire and forget: the layout above is already correct, and the window
    // handing width back is cosmetic from here.
    restoreWindowWidth();

    // UNLOAD THE IFRAME.
    //
    // Hiding the panel does not stop the page inside it: measured after
    // closing, the iframe was still present with src=gbf.wiki/Main_Page at
    // width 0, so its scripts, timers and network requests kept running for
    // the rest of the session. Keeping it made reopening instant, but that is
    // a poor trade for a panel that is closed most of the time -- and this
    // project's first priority is that Granblue gets the machine.
    //
    // Deferred so it cannot tear down mid-transition, and re-checked on fire
    // because the Wiki button toggles: reopening inside the delay must cancel
    // this, not race it.
    //
    // Cost: reopening returns to the wiki's home page rather than wherever you
    // had browsed to. Restoring that would mean holding the URL and navigating
    // back on open, which reloads the page anyway.
    if (wikiUnloadTimer) clearTimeout(wikiUnloadTimer);
    wikiUnloadTimer = setTimeout(() => {
      wikiUnloadTimer = 0;
      if (wikiIsOpen() || !wikiFrameEl) return;
      wikiFrameEl.remove();
      wikiFrameEl = null;
    }, 400);
  }

  // Swap the panel between the wiki and the About page. Same element, so the
  // sizing, the reserved inset and the auto-close rules do not need to know
  // which one is showing.
  function applyPanelMode() {
    if (!wikiPanelEl) return;
    const isAbout = panelMode === "about";
    const set = (sel, shown) => {
      const el = wikiPanelEl.querySelector(sel);
      if (el) el.style.display = shown ? "" : "none";
    };
    set("#gbf-wiki-home", !isAbout);
    set("#gbf-wiki-search", !isAbout);
    set("#gbf-wiki-body", !isAbout);
    set("#gbf-about-body", isAbout);
    const title = wikiPanelEl.querySelector("#gbf-panel-title");
    if (title) {
      title.textContent = isAbout ? "About this app" : "";
      title.style.display = isAbout ? "" : "none";
    }
  }

  function wikiIsOpen() {
    return !!wikiPanelEl && wikiPanelEl.classList.contains("gbf-wiki-open");
  }

  // ------------------------------------------------------------------
  // Drag-to-scroll for GBF's own content (not just our sidebar)
  // ------------------------------------------------------------------
  // Deliberately generic: it never touches anything unless the pointer
  // actually starts inside an element that already scrolls vertically
  // (found via computed overflow-y + scrollHeight > clientHeight, not
  // any GBF-specific class/id), so it can never affect non-scrolling
  // game UI (battle buttons, node maps, etc.) — those keep their
  // native click behavior completely untouched. Skips the sidebar
  // itself, which already has its own separate drag-scroll handling.
  let pageDragTarget = null;
  let pageDragging = false;
  let pageDragMoved = false;
  let pageDragEndedAt = 0; // timestamp of the last real drag; drives suppression
  let pageDragStartY = 0;
  let pageDragStartTop = 0;

  function findScrollableAncestor(el) {
    while (el && el !== document.body && el !== document.documentElement) {
      const cs = window.getComputedStyle(el);
      if (
        (cs.overflowY === "auto" || cs.overflowY === "scroll") &&
        el.scrollHeight > el.clientHeight + 1
      ) {
        return el;
      }
      el = el.parentElement;
    }
    const scroller = document.scrollingElement || document.documentElement;
    if (scroller && scroller.scrollHeight > scroller.clientHeight + 1)
      return scroller;
    return null;
  }

  const pageMomentum = createMomentumScroller(() => pageDragTarget);

  document.addEventListener(
    "pointerdown",
    (e) => {
      if (e.button !== 0) return;
      if (sidebarEl && sidebarEl.contains(e.target)) return; // sidebar handles its own drag-scroll
      const target = findScrollableAncestor(e.target);
      if (!target) return;
      pageMomentum.stop(); // a new touch cancels any in-flight coast
      pageMomentum.reset();
      pageDragTarget = target;
      pageDragging = true;
      pageDragMoved = false;
      pageDragStartY = e.clientY;
      pageDragStartTop = target.scrollTop;
      pageMomentum.sample(e.clientY);
    },
    true,
  );

  document.addEventListener(
    "pointermove",
    (e) => {
      if (!pageDragging) return;
      const delta = e.clientY - pageDragStartY;
      if (!pageDragMoved && Math.abs(delta) > DRAG_SCROLL_THRESHOLD) {
        pageDragMoved = true;
      }
      if (pageDragMoved) {
        pageDragTarget.scrollTop = pageDragStartTop - delta;
        pageMomentum.sample(e.clientY);
        e.preventDefault();
      }
    },
    true,
  );

  function endPageDragScroll() {
    if (pageDragging && pageDragMoved) pageMomentum.release();
    pageDragging = false;
    pageDragTarget = null;
    // Record WHEN the drag ended. The click handler suppresses on elapsed
    // time, not on a flag — see DRAG_CLICK_SUPPRESS_MS.
    if (pageDragMoved) pageDragEndedAt = Date.now();
    pageDragMoved = false;
  }
  window.addEventListener("pointerup", endPageDragScroll);
  window.addEventListener("pointercancel", endPageDragScroll);

  // Capturing, same reasoning as the sidebar's own click suppression:
  // if the pointer actually dragged, swallow the resulting click before
  // it reaches whatever GBF element is underneath, so a drag never also
  // fires a game action.
  // Swallow the click a real drag produces, so a scroll gesture never also
  // fires a game action. Suppression is bounded by ELAPSED TIME since the drag
  // ended, which is what makes it safe.
  //
  // Two earlier versions of this leaked, in opposite directions, and both let
  // a scroll gesture navigate GBF or eat a real click:
  //
  //   1. A boolean cleared only inside this handler. If a drag ended with no
  //      click following (released over a different element, or off-window),
  //      it stayed true. The pointerdown reset sits after an early return, so
  //      it does not always clear either — and the NEXT genuine game click got
  //      swallowed.
  //   2. Clearing that boolean from setTimeout(..., 0) on pointerup. If the
  //      browser dispatched the click in a later task than pointerup, the
  //      timeout won the race, the flag was already false, and the drag's own
  //      click went through and NAVIGATED THE GAME.
  //
  // A timestamp has neither failure mode: it cannot go stale, and there is no
  // race to lose.
  document.addEventListener(
    "click",
    (e) => {
      if (Date.now() - pageDragEndedAt < DRAG_CLICK_SUPPRESS_MS) {
        pageDragEndedAt = 0; // one click per drag
        e.preventDefault();
        e.stopPropagation();
      }
    },
    true,
  );

  // ------------------------------------------------------------------
  // Nav density — pick spacing that actually fits this window
  // ------------------------------------------------------------------
  // Spacing used to be a fixed number, tuned by measuring in a browser. That
  // kept being wrong in the app: the host page's font metrics differ, so a
  // list measured as having 53px to spare arrived with a scrollbar. Rather
  // than guess a fourth time, measure the real thing and step the spacing down
  // until it fits.
  //
  // It works in two phases, and the ORDER matters. Compression can only ever
  // remove space, so on a tall window it left the list ending far above the
  // ACTIONS footer with a large dead gap. So: first take the loosest spacing
  // that fits, then GROW the gaps to spread whatever is left over, filling the
  // sidebar down to the footer. Compression is the fallback, reached only when
  // the list would otherwise run into ACTIONS.
  //
  // If even the tightest step overflows, the tightest is used and the list
  // simply scrolls — which is correct, not a failure.
  const NAV_DENSITY_CLASSES = ["gbf-dense-1", "gbf-dense-2", "gbf-dense-3"];

  // scrollHeight, NOT the last child's rect.
  //
  // The rect version measured the last item's bottom relative to the
  // SCROLLER'S OWN top, which is the viewport top, not the content top. Once
  // the list had been scrolled at all, that came out short by exactly
  // scrollTop -- so an overflowing list measured as fitting, the fitter kept
  // the loosest spacing, and the list stayed overflowing. Seen in play as the
  // spacing flipping to wide-and-clipped after the wiki closed, since closing
  // it is what left the nav scrolled.
  function navContentHeight() {
    return navScrollEl ? navScrollEl.scrollHeight : 0;
  }

  // Buttons in the SCROLLER only. .gbf-nav-btn also styles the footer's
  // Back/Reload/Lock, and growing those would make the footer taller and eat
  // the very space we are trying to fill.
  function navButtons() {
    return navScrollEl ? [...navScrollEl.querySelectorAll(".gbf-nav-btn")] : [];
  }

  // Growth is applied inline rather than through a class or a custom property
  // deliberately. An id-scoped rule (#gbf-nav-scroll .gbf-nav-btn) would
  // outrank the .gbf-dense-N rules and silently disable compression; an inline
  // style outranks everything and needs no specificity bookkeeping.
  const NAV_EXTRA_MAX = 16; // past this it reads as a broken layout, not airy
  function clearNavExtra() {
    navButtons().forEach((b) => {
      b.style.marginBottom = "";
    });
  }

  // Collapsing/expanding animates width over .12s, and the labels are
  // display:none while collapsed. Fitting density in the same tick therefore
  // measures the OLD layout: expanding measured the icon-only list, concluded
  // the loosest spacing fit, and never looked again -- the list then grew as
  // the labels came back and simply overflowed. Reading clientHeight forces a
  // reflow of the CURRENT animated frame, not the final one, so a synchronous
  // read cannot fix this; it has to be measured again once the width has
  // settled. A plain timeout comfortably outlasts the .12s transition and,
  // unlike transitionend, still fires when no transition happens at all --
  // already at that width, or reduced motion.
  function scheduleDensityRefit() {
    if (densityRefitTimer) clearTimeout(densityRefitTimer);
    densityRefitTimer = setTimeout(() => {
      densityRefitTimer = 0;
      fitNavDensity();
      scheduleWindowHug();
    }, 200);
  }

  let densityFitting = false;
  function fitNavDensity() {
    if (!navScrollEl || !sidebarInnerEl || densityFitting) return;
    densityFitting = true;
    try {
      // Measure the stylesheet's own spacing, not last pass's result.
      clearNavExtra();

      for (let step = 0; step <= NAV_DENSITY_CLASSES.length; step++) {
        NAV_DENSITY_CLASSES.forEach((c) => sidebarInnerEl.classList.remove(c));
        if (step > 0)
          sidebarInnerEl.classList.add(NAV_DENSITY_CLASSES[step - 1]);
        // Last step is the tightest we have; take it whether it fits or not.
        if (step === NAV_DENSITY_CLASSES.length) break;
        const viewport = navScrollEl.clientHeight;
        if (viewport <= 0) break; // not laid out yet, try again later
        if (navContentHeight() <= viewport) break;
      }

      // Compression alone only ever removes space. On a tall window the list
      // fits at the loosest step and stops well short of the footer, leaving a
      // large dead gap — so spread whatever is left over the gaps between
      // buttons until the list reaches down to ACTIONS.
      const viewport = navScrollEl.clientHeight;
      const btns = navButtons();
      if (viewport <= 0 || !btns.length) return;
      const slack = viewport - navContentHeight();
      if (slack <= 0) return;
      const base =
        parseFloat(window.getComputedStyle(btns[0]).marginBottom) || 0;
      const extra = Math.min(NAV_EXTRA_MAX, Math.floor(slack / btns.length));
      if (extra <= 0) return;
      btns.forEach((b) => {
        b.style.marginBottom = base + extra + "px";
      });
    } finally {
      densityFitting = false;
    }
  }

  // ------------------------------------------------------------------
  // Body inset for sidebar (always reserve room for the sidebar itself;
  // this is unrelated to lock/unlock, just keeps content out from under it)
  // ------------------------------------------------------------------
  function applyBodyInset() {
    // Reserve the wiki's room as well while it is open.
    //
    // THIS IS THE WHOLE MECHANISM — without it, widening the window does
    // nothing useful. GBF is responsive and lays out inside
    // (window width - this margin). If the margin only covers the sidebar,
    // every pixel we add to the window is absorbed by the game growing, the
    // game's right edge moves with it, and the reclaimed area stays exactly
    // the same size. Observed in play 2026-09-03: the window was widened and
    // the game's right edge did not move at all.
    //
    // Reserving nav + wiki means the game lays out inside the smaller box, so
    // the width we add to the window becomes the wiki's room.
    const reserved = sidebarWidth() + (wikiIsOpen() ? wikiPanelWidth : 0);
    document.body.style.setProperty(
      "margin-right",
      reserved + "px",
      "important",
    );
  }

  // ------------------------------------------------------------------
  // Locked-mode sidebar edge tracking
  // ------------------------------------------------------------------
  function getGameRightEdge() {
    const container = document.getElementById(GAME_CONTAINER_ID);
    if (!container) return null;
    const rect = container.getBoundingClientRect();
    if (rect.width <= 0) return null;
    return rect.right;
  }

  // Auto icon-only thresholds for locked mode (see positionSidebar):
  // collapse when available space drops below the full sidebar width,
  // but only expand again once there's a bit more than that back — the
  // gap between the two avoids flicker right at the boundary (see
  // comment inside positionSidebar for why a boundary flicker is
  // otherwise possible).
  // These are NOT the full sidebar width, deliberately.
  //
  // Requiring the full 250 meant the sidebar collapsed whenever it was a few
  // pixels short of ideal. Measured live on a 884px window: the game takes
  // 640, leaving 244 -- six pixels under, so it started collapsed on a window
  // with obviously enough room, and only a manual click undid it.
  //
  // Six pixels do not matter, because the box is pinned between the game's
  // edge and the window's edge: at 244 it simply renders 244 wide. Nothing
  // overlaps and nothing is pushed off-screen; the labels just have slightly
  // less room. Collapsing to icons is the far bigger loss. So the threshold is
  // what the labels genuinely need, not what the layout would prefer, and the
  // squeeze is allowed.
  const AUTO_COLLAPSE_BELOW = 205;
  const AUTO_EXPAND_AT_OR_ABOVE = 215;

  // Shrink the window so its right edge sits just past the sidebar.
  //
  // The sidebar's container is pinned to the window edge but the nav is
  // left-aligned inside it, so any surplus shows as a dead strip on the right
  // -- measured in Small: game 0-320, nav 321-571, then 120px of nothing to
  // 691. It appears whenever the window is wider than game + sidebar, which is
  //常 at the smaller fixed sizes, and collapsing only makes it wider.
  //
  // NOT DONE UNDER AUTOMATIC RESIZING. There, GBF refits the game to whatever
  // width we leave it, so freeing space immediately produces the same surplus
  // again and the window would walk itself down to nothing. Under a fixed size
  // the game cannot grow, so one shrink converges. This reads GBF's mode; it
  // never sets it.
  const HUG_SLACK = 8; // ignore rounding-sized leftovers
  let hugTimer = 0;
  let hugBusy = false;

  function gbfUsesAutomaticResizing() {
    try {
      return !!(
        window.Game &&
        window.Game.setting &&
        window.Game.setting.mobage_fixwindowsize === 0
      );
    } catch (e) {
      return true;
    }
  }

  // Under Automatic, the sidebar keeps its natural width and the game takes
  // the space. Stretching the nav to fill the surplus was tried and removed:
  // on a 1499px window it produced an 858px-wide sidebar. The sidebar has a
  // size; dragging the window is meant to scale GRANBLUE, not the nav. Past
  // GBF's own zoom cap (2, i.e. a 640px game) the surplus is simply dead
  // space, and nothing we can do about it is worth the cost -- see below.
  //
  // NEVER HUG UNDER AUTOMATIC RESIZING -- it makes GBF RELOAD.
  //
  // Measured: with Automatic on, resizing the window from 891 to 1500 wiped a
  // marker set on `window` beforehand. GBF re-fits by reloading the page, so
  // every hug would restart Granblue underneath the player. That is a far
  // worse outcome than a dead strip, and it is the one thing this project does
  // not do. Fixed sizes do not reload: the Small collapse tests resized the
  // window dozens of times with the marker surviving throughout.
  //
  // Shrinking there would also cost game size whenever the game is elastic --
  // an experiment walked it from 553px down to its 320px minimum -- though on
  // a wide window the game is capped at zoom 2 and the surplus really is dead
  // (measured at 1599px: game 640, 708px of nothing). Both facts point the
  // same way: under Automatic, leave the window alone.
  function scheduleWindowHug() {
    if (hugTimer || hugBusy) return;
    if (!locked || wikiIsOpen() || navWidenedBy > 0) return;
    // A panel is opening or closing: its widen is not surplus, and the width
    // it gives back on close is handled by restoreWindowWidth().
    if (panelBusy || wikiWidenedBy > 0) return;
    if (gbfUsesAutomaticResizing()) return;
    // Debounced: positionSidebar runs on every resize frame, and the resize we
    // are about to do triggers more of them.
    hugTimer = setTimeout(async () => {
      hugTimer = 0;
      if (!locked || wikiIsOpen() || gbfUsesAutomaticResizing()) return;
      if (panelBusy || wikiWidenedBy > 0) return;
      const edge = getGameRightEdge();
      if (edge == null) return;
      // Fits the window to the content in BOTH directions.
      //
      // Shrinking alone was not enough: changing GBF's Window Size from Small
      // to Large grows the game from 320 to 640, and the window stayed at the
      // width Small had been hugged to. Measured right after such a switch --
      // window 533, game 0-640, sidebar crushed to 7px and the game
      // overflowing. Nothing widened the window because the hug only ever
      // took space away.
      //
      // Growing has the same safety story as shrinking: only in a fixed size,
      // where the game does not reflow into what we hand it, and
      // resizeWindowByCss already refuses to exceed the screen.
      // Aim at the width the user ASKED for, not the one we are currently
      // forced into. sidebarWidth() reports 52 whenever autoNarrow is latched,
      // and autoNarrow latches exactly when the window is too small -- so
      // using it here would grow the window only enough to keep the nav
      // collapsed, and it would stay collapsed forever. `collapsed` is the
      // manual preference and is the right target; once the room exists,
      // positionSidebar() clears autoNarrow on its own.
      const wantSidebar = collapsed ? SIDEBAR_W_COLLAPSED : SIDEBAR_W;
      const leftover = Math.round(window.innerWidth - edge - wantSidebar);
      if (Math.abs(leftover) <= HUG_SLACK) return;
      hugBusy = true;
      try {
        await resizeWindowByCss(-leftover);
        await settleLayout();
      } finally {
        hugBusy = false;
      }
    }, 350);
  }

  function positionSidebar() {
    if (!sidebarEl) return;

    // Outer wrapper's right edge is ALWAYS pinned to the window's right
    // edge — this alone guarantees requirement #1 unconditionally.
    sidebarEl.style.right = "0";

    if (!locked) {
      // Auto-narrow only applies to locked mode's edge-fitting logic;
      // clear it when unlocked so the sidebar always matches the
      // manual toggle exactly, same as before this feature existed.
      if (autoNarrow) {
        autoNarrow = false;
        applyCollapsedVisual();
      }
      sidebarEl.style.left = "";
      sidebarEl.style.width = sidebarWidth() + "px";
      return;
    }

    const edge = getGameRightEdge();
    if (edge == null) return; // GBF hasn't laid out yet

    // Auto icon-only mode: when the space actually available for the
    // sidebar (window width minus the game's edge) is too tight for
    // the full sidebar, force the compact icon-only layout regardless
    // of the manual toggle, so content never gets pushed past the
    // window's right edge. Reverts to the manual preference once
    // enough room is back. Collapsing/expanding changes body's
    // margin-right, which changes how much room GBF's own layout
    // leaves for the game — that in turn changes the measured edge on
    // the *next* call, so this settles to a stable state via the
    // existing debounced ResizeObserver rather than needing its own
    // polling loop here.
    // MEASURE AGAINST THE EDGE FROM BEFORE WE COLLAPSED, not the one our own
    // collapse produced.
    //
    // Collapsing drops the reserved inset from 250 to 52. In Automatic
    // Resizing mode GBF immediately grows the game into those freed 198px, so
    // `window.innerWidth - edge` snaps back to about 52 and never again
    // reaches the 255 expand threshold. Auto-narrow therefore latched on
    // permanently and only the manual toggle could undo it -- seen in play as
    // the sidebar starting collapsed on a window with ample room and needing a
    // click every launch.
    //
    // Freezing the pre-collapse edge breaks that loop: the reference cannot be
    // moved by our own collapse, so widening the window raises `available`
    // directly. It is lowered again whenever the game genuinely gets narrower,
    // which is what recovers from a too-wide edge measured during GBF's
    // first, unsettled layout at startup.
    const available = window.innerWidth - edge;
    if (autoNarrow) {
      if (edge < narrowRefEdge) narrowRefEdge = edge;
      if (window.innerWidth - narrowRefEdge >= AUTO_EXPAND_AT_OR_ABOVE) {
        autoNarrow = false;
        applyCollapsedVisual();
      }
    } else if (available < AUTO_COLLAPSE_BELOW) {
      narrowRefEdge = edge;
      autoNarrow = true;
      applyCollapsedVisual();
    }

    // Both left (game's edge) and right (window's edge) are pinned,
    // with width:auto — the box exactly fills the space between them,
    // so it can never overlap the game and never leave a gap to the
    // window edge, regardless of how wide that space ends up being.
    // THE SIDEBAR AREA IS ONLY EVER AS WIDE AS THE SIDEBAR NEEDS.
    //
    // It used to span from the game's edge to the window edge with width:auto,
    // so the area grew with the window and any surplus showed up as dead space
    // inside it. Dragging the window is meant to scale GRANBLUE, not this.
    //
    // The one thing that legitimately widens it is the wiki, which is what the
    // reclaimed area is for -- so the panel's width is added when it is open.
    //
    // Still clamped to the space beside the game, so it can never overlap:
    // when the window is too narrow for the full width, it stops at the game's
    // edge exactly as before.
    const wantW = sidebarWidth() + (wikiIsOpen() ? wikiPanelWidth : 0);
    const roomBesideGame = Math.max(0, window.innerWidth - Math.round(edge));
    sidebarEl.style.left = "";
    sidebarEl.style.width = Math.min(wantW, roomBesideGame) + "px";

    fitNavDensity();
    scheduleWindowHug();

    // The wiki occupies the reclaimed area. If the window shrinks until that
    // area is too narrow to read, close it rather than leaving an unusable
    // sliver wedged against the nav. Reopening is one click.
    // If the window is shrunk until the reserved wiki room would squeeze the
    // game below what is playable, drop the wiki rather than the game. The
    // 600ms grace stops this firing against the transient measurements our own
    // widening produces, which would close the panel we just opened.
    // THE GRACE MUST DEFER THE DECISION, NOT DISCARD IT.
    //
    // This check only runs when positionSidebar() does, i.e. on a resize. If
    // the only resize lands inside the grace, the grace skipped it and nothing
    // ever looked again -- measured: shrinking to 1400 within 600ms of opening
    // left the panel open indefinitely with 190px for a game whose minimum is
    // 320, and it took an unrelated later resize to correct itself. The grace
    // is meant to ignore our own settling measurements, not to drop a genuine
    // one that happens to arrive early.
    // A few px of tolerance. Every term here is a rounded measurement, and the
    // window resize lands within a pixel or two of the request, so an exact
    // comparison can fire on rounding alone -- which is precisely how the
    // panel used to close itself immediately after opening.
    // How much room the game actually needs, which is NOT always
    // MIN_GAME_WIDTH.
    //
    // Under Automatic the game reflows, so the minimum is the right bar. At a
    // FIXED size it cannot shrink at all: whatever it currently measures is
    // what it needs, and reserving the panel's room on top of a game that will
    // not yield means the sidebar and panel sit on top of it. Found by a test
    // that changed GBF's Window Size WHILE the wiki was open: the game grew
    // 320 -> 640, the window stayed at 1532 with 960 reserved for the panel,
    // and 322px was left for a 640px game -- overlapping, with the panel
    // happily open because 322 cleared the old 320 bar.
    const gameNeeds = gbfUsesAutomaticResizing()
      ? MIN_GAME_WIDTH
      : edge != null
        ? edge
        : MIN_GAME_WIDTH;
    const wikiRoomTooTight =
      wikiIsOpen() &&
      window.innerWidth - sidebarWidth() - wikiPanelWidth <
        gameNeeds - WIKI_ROOM_TOLERANCE;
    const graceLeft = 600 - (Date.now() - wikiOpenedAt);
    if (wikiRoomTooTight && graceLeft > 0) {
      if (!wikiRecheckTimer) {
        wikiRecheckTimer = setTimeout(() => {
          wikiRecheckTimer = 0;
          positionSidebar(); // re-decides with the grace expired
        }, graceLeft + 50);
      }
    }

    if (wikiRoomTooTight && graceLeft <= 0) {
      closeWikiPanel();
      // Same actionable wording as the pre-check: if the window could not be
      // stretched, GBF's own Window Size is the lever that actually helps.
      showSidebarNotice(
        "No room for the wiki. Try a smaller Window Size in GBF's Browser Settings.",
      );
    }

    if (window.localStorage && localStorage.getItem("gbfDebug") === "1") {
      console.log(
        "[gbf-lock] game right edge=" +
          edge +
          " -> outer left=" +
          Math.round(edge) +
          (autoNarrow ? " (auto-narrow)" : ""),
      );
    }
  }

  // GBF's own page transitions (e.g. clicking Home/Quests/etc.) can
  // cause #wrapper to briefly report an incorrect, much smaller width
  // for a frame or two while the new page lays out, before settling on
  // its real size. Reacting to that instantly makes the sidebar flash
  // into the game area for a split second. Debouncing the ResizeObserver
  // callback means we only reposition once #wrapper's size has actually
  // settled, so the transient in-between value never gets rendered.
  // (This only debounces #wrapper-driven repositioning; the OS window
  // resize handler below stays instant so dragging the window still
  // feels responsive.)
  // 20ms per user's own hands-on testing — confirmed noticeably faster
  // with no regression on their end (originally 100ms).
  let positionDebounceTimer = null;
  function debouncedPositionSidebar() {
    if (positionDebounceTimer) clearTimeout(positionDebounceTimer);
    positionDebounceTimer = setTimeout(() => {
      positionDebounceTimer = null;
      positionSidebar();
    }, 20);
  }

  function ensureSizeObserver() {
    if (sizeObserver) return;
    const container = document.getElementById(GAME_CONTAINER_ID);
    if (!container || typeof ResizeObserver === "undefined") return;
    sizeObserver = new ResizeObserver(() => debouncedPositionSidebar());
    sizeObserver.observe(container);
  }

  function applyLockState() {
    document.documentElement.classList.toggle("gbf-locked", locked);
    applyBodyInset();
    positionSidebar();
    ensureSizeObserver();
    ensureNavObserver();
  }

  // Fades the sidebar in once GBF's game container has a real size and
  // positionSidebar() has already been applied against it, so nothing
  // visible ever reflects an in-between/incorrect measurement. rAF-delayed
  // one frame so the browser has committed the initial hidden (opacity:0)
  // state before the class is removed — otherwise a same-frame add+remove
  // on a freshly-inserted element can skip the transition entirely.
  function revealSidebar() {
    if (sidebarRevealed || !sidebarEl) return;
    sidebarRevealed = true;
    requestAnimationFrame(() => {
      // REPOSITION IN THE SAME FRAME WE REVEAL.
      //
      // This is what actually causes the flash, and two earlier attempts
      // missed it by gating the reveal instead. The reveal is deferred one
      // frame (so the browser commits opacity:0 and the transition runs), but
      // nothing repositioned during that frame -- so whatever the edge was
      // when we last positioned is what gets painted. Traced on a real reload:
      // GBF was already initialised with the zoom applied, wrapper at 387, and
      // the sidebar still drawn at 320 at opacity 0.07.
      //
      // No gate on the previous frame's measurement can fix that, because the
      // stale value is the POSITION, not the reading. Measuring again here,
      // immediately before it becomes visible, is what makes the first visible
      // frame correct.
      applyBodyInset();
      positionSidebar();
      sidebarEl.classList.remove("gbf-sidebar-hidden");
      // First moment the nav has its real box. Every earlier fit ran while the
      // sidebar was hidden and GBF was still settling, so this is the first
      // measurement worth trusting.
      fitNavDensity();
    });
  }

  // Watch the SCROLLER, not the game container.
  //
  // The fitter only ever ran from positionSidebar(), which fires on game-
  // container resizes. But what the fitter needs is the nav's own viewport
  // height, and that changes with WINDOW HEIGHT, which the game container does
  // not report. At startup every one of those calls happened while the sidebar
  // was hidden and unsettled, and nothing looked again -- measured on a real
  // build: content 1117 in a 1016 viewport, overflowing by 101px with no
  // density step applied. Observing the element whose size the decision
  // actually depends on fixes that class of miss rather than one instance.
  //
  // No feedback loop: changing density changes the CONTENT height
  // (scrollHeight), while this observes the scroller's own border box, which
  // is driven by the flex column and does not move in response.
  let navObserver = null;
  function ensureNavObserver() {
    if (navObserver || !navScrollEl || typeof ResizeObserver === "undefined")
      return;
    navObserver = new ResizeObserver(() => scheduleDensityRefit());
    navObserver.observe(navScrollEl);
  }

  // GBF may finish its own initial layout slightly after our script runs,
  // so retry until the game container actually has a size. Also covers
  // the post-reload case (e.g. Home, entering combat): those are real
  // page reloads where this script re-runs on a fresh document, so this
  // same loop is what rebuilds the sidebar from scratch afterward. Since
  // a real reload means the sidebar unavoidably has to disappear and
  // redraw, the thing worth optimizing is how fast we detect #wrapper is
  // ready and apply — so this checks on every animation frame (via
  // requestAnimationFrame) instead of a fixed setTimeout interval. That's
  // roughly a 16ms worst-case detection latency once #wrapper actually
  // has a size, faster than a fixed-interval poll. Budget is tracked by
  // elapsed wall-clock time (not frame count, since rAF's actual rate
  // isn't guaranteed), capped at MAX_CAPTURE_MS so a pathological case
  // (GBF never finishing layout) doesn't poll forever.
  const MAX_CAPTURE_MS = 30000; // ~30s ceiling — generous vs. the original 5s (250ms*20)
  let captureStartTime = null;
  // The edge seen on the previous frame. Revealing requires two consecutive
  // frames to agree — see below.
  let lastCaptureEdge = null;
  function retryCaptureAndApply() {
    applyLockState();
    const edge = getGameRightEdge();

    // WAIT FOR GBF TO HAVE APPLIED ITS ZOOM, not merely for a width, and not
    // for the width to hold still.
    //
    // On a real reload (Home, entering combat) #wrapper is briefly its
    // unzoomed 320px base before GBF applies the container zoom. Revealing on
    // "has a width" put the sidebar 320px INSIDE the game and then snapped it
    // right, mid-fade -- the flash.
    //
    // Requiring the edge to repeat across two frames was tried and MEASURED TO
    // FAIL: the pre-zoom 320 is perfectly stable for several frames, so it
    // satisfied a stability test and revealed just as wrongly (traced: sidebar
    // at 320 against a 387 wrapper at opacity 0.16). Stability is not the
    // property that matters.
    //
    // Game.setting is. It populates in the same frame the zoom lands -- in the
    // trace it flipped from null to 0 exactly as the wrapper went 320 -> 387 --
    // because both come from GBF finishing its own init. So gate on GBF being
    // ready, and keep the two-frame check as a cheap extra guard for the rest.
    //
    // Note Game.setting is absent BEFORE #wrapper has a size too, so this can
    // only ever delay the reveal, never skip the wait. The MAX_CAPTURE_MS
    // budget below still bounds it, which matters on any page where GBF never
    // initialises at all.
    const gbfReady = !!(
      window.Game &&
      window.Game.setting &&
      typeof window.Game.getZoom === "function"
    );
    const settled = edge != null && edge === lastCaptureEdge && gbfReady;
    lastCaptureEdge = edge;

    if (!settled) {
      const now =
        window.performance && performance.now ? performance.now() : Date.now();
      if (captureStartTime == null) captureStartTime = now;
      if (now - captureStartTime < MAX_CAPTURE_MS) {
        requestAnimationFrame(retryCaptureAndApply);
        return;
      }
      // Ran out of budget without ever getting a stable measurement —
      // reveal anyway rather than leaving the sidebar invisible forever.
    }
    revealSidebar();
  }

  retryCaptureAndApply();
  window.addEventListener("resize", () => {
    applyBodyInset();
    positionSidebar();
  });

  // ------------------------------------------------------------------
  // Diagnostics
  // ------------------------------------------------------------------
  // State snapshot for probe.mjs, which attaches over WebView2's remote
  // debugging port. Everything interesting lives in this IIFE's closure and is
  // otherwise unreachable from outside -- and closure values are exactly what
  // the layout bugs have turned on: an expand threshold compared against a
  // stale edge, a content height short by scrollTop. Read-only, and reading it
  // must never change anything.
  window.__gbfDebug = function () {
    return {
      locked: locked,
      collapsed: collapsed,
      autoNarrow: autoNarrow,
      narrowRefEdge:
        narrowRefEdge === Infinity ? null : Math.round(narrowRefEdge),
      wikiForcedNarrow: wikiForcedNarrow,
      effectivelyCollapsed: isEffectivelyCollapsed(),
      sidebarWidth: sidebarWidth(),
      gameRightEdge: (function () {
        const e = getGameRightEdge();
        return e == null ? null : Math.round(e);
      })(),
      availableForSidebar: (function () {
        const e = getGameRightEdge();
        return e == null ? null : Math.round(window.innerWidth - e);
      })(),
      thresholds: {
        autoCollapseBelow: AUTO_COLLAPSE_BELOW,
        autoExpandAtOrAbove: AUTO_EXPAND_AT_OR_ABOVE,
      },
      wiki: {
        open: wikiIsOpen(),
        panelWidth: wikiPanelWidth,
        widenedBy: wikiWidenedBy,
        msSinceOpened: wikiOpenedAt ? Date.now() - wikiOpenedAt : null,
      },
      // GBF's own sizing model, read straight from its globals. Purely
      // observational -- we never set any of it (RULE 0).
      //
      // The size options are a CSS zoom on the game container, so the game's
      // rendered width is its 320px base layout times that zoom. Which mode is
      // active decides whether widening the window is absorbed by the game
      // (Automatic Resizing) or not (a fixed size), and that distinction is
      // behind several of the layout bugs -- so it is worth being able to see
      // it rather than infer it.
      gbf: (function () {
        try {
          const G = window.Game;
          if (!G) return null;
          const wrap = document.getElementById(GAME_CONTAINER_ID);
          return {
            fixWindowSize: G.setting ? G.setting.mobage_fixwindowsize : null,
            zoom: typeof G.getZoom === "function" ? G.getZoom() : null,
            baseWidth: wrap
              ? Math.round(parseFloat(window.getComputedStyle(wrap).width))
              : null,
          };
        } catch (e) {
          return "unreadable: " + e.message;
        }
      })(),
      nav: {
        viewport: navScrollEl ? navScrollEl.clientHeight : null,
        content: navContentHeight(),
        density: sidebarInnerEl
          ? [...sidebarInnerEl.classList].filter((c) =>
              c.startsWith("gbf-dense-"),
            )
          : null,
        extraMax: NAV_EXTRA_MAX,
      },
    };
  };

  if (window.localStorage && localStorage.getItem("gbfDebug") === "1") {
    try {
      if (!document.querySelector("iframe[data-gbf-console]")) {
        const f = document.createElement("iframe");
        f.setAttribute("data-gbf-console", "1");
        f.style.display = "none";
        document.documentElement.appendChild(f);
        window.console = f.contentWindow.console;
      }
    } catch (e) {}
    console.log("[gbf-debug] diagnostics active");
  }
})();

// EXPERIMENT ONLY. Alt-modified shortcuts (Rule 0 exception 5) so they fire
// in whichever webview has focus — game, wiki, About, or Options — not only the sidebar.
(function () {
  if (window.__gbfNativeKeys) return;
  window.__gbfNativeKeys = true;

  var HASH = {
    1: "#mypage",
    p: "#party/index/0/npc/0",
    P: "#party/index/0/npc/0",
    2: "#quest",
    3: "#quest/assist",
    4: "#coopraid",
    5: "#guild",
    6: "#item",
    7: "#list",
    c: "#present",
    C: "#present",
    s: "#container",
    S: "#container",
    8: "#profile",
    9: "#shop",
    0: "#shop/exchange/trajectory",
    // #arcarum redirects to #arcarum2; keep this the same as the sidebar
    // row so Alt+- and the click land on the hash the highlight expects.
    "-": "#arcarum2",
    "=": "#frontier/alchemy/top",
    "[": "#trial_battle",
    "]": "#casino",
    ";": "#gacha",
  };

  function invoke(cmd, args) {
    var t = window.__TAURI__;
    if (!t || !t.core || !t.core.invoke) return false;
    t.core.invoke(cmd, args || {});
    return true;
  }

  document.addEventListener(
    "keydown",
    function (e) {
      if (!e.altKey || e.ctrlKey || e.metaKey) return;
      // Desktop client + Automatic Resizing is native: no sidebar, no panels,
      // so nothing to drive and no key of the game's to shadow. __gbfInert is
      // defined only in the game webview, so the panels keep their shortcuts.
      if (window.__gbfInert && window.__gbfInert()) return;
      if (e.key === "w" || e.key === "W") {
        e.preventDefault();
        invoke("gbf_wiki_toggle");
        return;
      }
      if (e.key === "l" || e.key === "L") {
        e.preventDefault();
        invoke("gbf_toggle_lock");
        return;
      }
      if (e.key === "\\") {
        e.preventDefault();
        invoke("gbf_toggle_sidebar");
        return;
      }
      // Reload and Back drive GRANBLUE, from whichever webview has focus.
      //
      // These are the Reload and Back BUTTONS' shortcuts, and those buttons
      // drive the game, so the keys must mean the same thing everywhere. They
      // used to act on the focused document instead: from About or Options
      // that reloaded a static local page and stepped a history with nothing
      // in it, so the keys looked dead there. The sidebar had the identical
      // bug in its own copy of this handler.
      //
      // The WIKI is the one place the fallback matters. It is an external
      // site with no Tauri IPC, so `invoke` cannot reach Rust there -- and
      // reloading or stepping back the wiki itself is exactly what a reader
      // wants, so that is what it falls back to.
      if (e.key === "r" || e.key === "R") {
        e.preventDefault();
        if (!invoke("gbf_game_reload")) location.reload();
        return;
      }
      if (e.key === "ArrowLeft") {
        e.preventDefault();
        if (!invoke("gbf_game_back")) history.back();
        return;
      }
      var hash = HASH[e.key];
      if (hash) {
        e.preventDefault();
        invoke("gbf_nav", { hash: hash });
      }
    },
    true,
  );
})();

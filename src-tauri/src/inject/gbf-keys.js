// EXPERIMENT ONLY. Alt-modified shortcuts (Rule 0 exception 5) so they fire
// in whichever webview has focus — game, wiki, or About — not only the sidebar.
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
    "-": "#arcarum",
    "=": "#frontier/alchemy/top",
    "[": "#trial_battle",
    "]": "#casino",
    ";": "#gacha",
  };

  function invoke(cmd, args) {
    var t = window.__TAURI__;
    if (!t || !t.core || !t.core.invoke) return;
    t.core.invoke(cmd, args || {});
  }

  document.addEventListener(
    "keydown",
    function (e) {
      if (!e.altKey || e.ctrlKey || e.metaKey) return;
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
      var hash = HASH[e.key];
      if (hash) {
        e.preventDefault();
        invoke("gbf_nav", { hash: hash });
      }
    },
    true,
  );
})();

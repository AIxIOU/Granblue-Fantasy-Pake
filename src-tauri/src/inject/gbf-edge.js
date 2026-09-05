// EXPERIMENT ONLY. Read Granblue's #wrapper right edge so Rust can snap the
// native sidebar to it. Rule 0: this only *reads* geometry. It does not write
// to the game's document (locked-mode CSS is a separate, existing exception).
(function () {
  if (window.__gbfEdgeInit) return;
  window.__gbfEdgeInit = true;

  var timer = 0;
  function automatic() {
    try {
      if (!window.Game || !window.Game.setting) return null;
      return window.Game.setting.mobage_fixwindowsize === 0;
    } catch (e) {
      return null;
    }
  }

  window.__gbfReportEdge = function () {
    var t = window.__TAURI__;
    if (!t || !t.core || !t.core.invoke) return;
    var el = document.getElementById("wrapper");
    if (!el) {
      // Steam login and other non-game pages. Clear the stale lock edge so
      // the sidebar cannot sit on top of them.
      t.core.invoke("gbf_game_edge", { right: 0, automatic: false });
      return;
    }
    var r = el.getBoundingClientRect();
    if (r.width <= 0) {
      t.core.invoke("gbf_game_edge", { right: 0, automatic: false });
      return;
    }
    if (!window.__gbfHug) return;
    var auto = automatic();
    if (auto === null) return;
    t.core.invoke("gbf_game_edge", { right: r.right, automatic: auto });
  };

  window.__gbfSetHug = function (on) {
    window.__gbfHug = !!on;
    window.__gbfReportEdge();
    if (!on) return;
    var n = 0;
    var id = setInterval(function () {
      n += 1;
      window.__gbfReportEdge();
      if (automatic() !== null || n > 20) clearInterval(id);
    }, 100);
  };

  function schedule() {
    if (timer) return;
    timer = setTimeout(function () {
      timer = 0;
      window.__gbfReportEdge();
    }, 100);
  }

  window.addEventListener("resize", schedule);
  try {
    var ro = new ResizeObserver(schedule);
    function attach() {
      var el = document.getElementById("wrapper");
      if (el) ro.observe(el);
    }
    attach();
    document.addEventListener("DOMContentLoaded", attach);
  } catch (e) {}
})();

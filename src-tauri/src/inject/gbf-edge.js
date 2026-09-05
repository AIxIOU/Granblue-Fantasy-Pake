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
  function send(right) {
    var t = window.__TAURI__;
    if (!window.__gbfHug || !t || !t.core || !t.core.invoke) return;
    var auto = automatic();
    // Wait until GBF's setting exists so we do not hug a Fixed-size window
    // that is actually Automatic (or the reverse).
    if (auto === null) return;
    t.core.invoke("gbf_game_edge", { right: right, automatic: auto });
  }

  window.__gbfReportEdge = function () {
    var el = document.getElementById("wrapper");
    if (!el) return;
    var r = el.getBoundingClientRect();
    if (r.width <= 0) return;
    send(r.right);
  };

  window.__gbfSetHug = function (on) {
    window.__gbfHug = !!on;
    if (!on) return;
    window.__gbfReportEdge();
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

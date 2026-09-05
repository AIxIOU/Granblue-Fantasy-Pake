// EXPERIMENT ONLY. Read Granblue's #wrapper and submenu overlay so Rust can
// snap the native sidebar. Rule 0: this only *reads* geometry.
//
// Desktop client + Automatic Resizing is a NATIVE mode: nothing of ours runs
// in the page there. This file is the one exception, reduced to the smallest
// thing that can notice the player leaving that mode -- a 2s read of
// `Game.setting.mobage_fixwindowsize`. No DOM work, no observers, no
// listeners, no page change. Everything else of ours asks
// `window.__gbfInert()` and does nothing while it is true.
(function () {
  if (window.__gbfEdgeInit) return;
  window.__gbfEdgeInit = true;

  var DORMANT_POLL_MS = 2000;
  var timer = 0;
  var dormant = 0;
  var ro = null;
  var mo = null;

  function automatic() {
    try {
      if (!window.Game || !window.Game.setting) return null;
      return window.Game.setting.mobage_fixwindowsize === 0;
    } catch (e) {
      return null;
    }
  }

  // The mobile client has no #submenu column, and no Window Size settings at
  // all -- mobage_fixwindowsize is permanently 0 there, so it is not a mode.
  function desktopClient() {
    return !!document.getElementById("submenu");
  }

  // True only in the native mode: desktop client on Automatic Resizing.
  // Unknown (`Game` not up yet, or a non-game page) is NOT inert -- our
  // behaviour is the default and this gate is the exception.
  window.__gbfInert = function () {
    return automatic() === true && desktopClient();
  };

  function consider(el, best) {
    if (!el) return best;
    var r = el.getBoundingClientRect();
    var cs = window.getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden") return best;
    if (r.height < 40 || r.width < 8) return best;
    return r.right > best ? r.right : best;
  }

  // Collapsed: #cnt-submenu-navi-vertical (icon rail).
  // Expanded: #prt-submenu-contents / #chat-body (chat panel).
  function overlayRight() {
    var best = 0;
    best = consider(document.getElementById("cnt-submenu-navi-vertical"), best);
    best = consider(document.getElementById("prt-submenu-contents"), best);
    best = consider(document.getElementById("chat-body"), best);
    return best;
  }

  window.__gbfReportEdge = function () {
    var t = window.__TAURI__;
    if (!t || !t.core || !t.core.invoke) return;
    var el = document.getElementById("wrapper");
    if (!el) return;
    var r = el.getBoundingClientRect();
    if (r.width <= 0) return;
    var auto = automatic();
    if (auto === null) return;
    // Reload paints #wrapper at the unzoomed 320px base before zoom lands.
    var zoom = 1;
    try {
      if (typeof Game.getZoom === "function") zoom = Game.getZoom() || 1;
    } catch (e) {}
    if (zoom > 1.05 && r.right < 340) return;
    t.core.invoke("gbf_game_edge", {
      right: r.right,
      overlay: overlayRight(),
      automatic: auto,
      dpr: window.devicePixelRatio || 1,
      zoom: zoom,
      mobile: !desktopClient(),
    });
    // Rust now knows it is the native mode. Report nothing further and stop
    // watching the page; the poll below is all that is left of us.
    if (auto === true && desktopClient()) sleep();
  };

  function sleep() {
    if (dormant) return;
    if (timer) {
      clearTimeout(timer);
      timer = 0;
    }
    window.removeEventListener("resize", schedule);
    try {
      if (ro) ro.disconnect();
      if (mo) mo.disconnect();
    } catch (e) {}
    dormant = setInterval(function () {
      // One property read. Nothing else of ours runs until this flips.
      if (automatic() === false) wake();
    }, DORMANT_POLL_MS);
  }

  function wake() {
    if (!dormant) return;
    clearInterval(dormant);
    dormant = 0;
    window.addEventListener("resize", schedule);
    attach();
    watchSub();
    window.__gbfReportEdge();
  }

  function schedule() {
    if (timer || dormant) return;
    timer = setTimeout(function () {
      timer = 0;
      window.__gbfReportEdge();
    }, 100);
  }

  window.__gbfSetHug = function () {
    window.__gbfReportEdge();
    var n = 0;
    var id = setInterval(function () {
      n += 1;
      window.__gbfReportEdge();
      if (dormant || automatic() !== null || n > 20) clearInterval(id);
    }, 100);
  };

  function attach() {
    if (!ro) return;
    ["wrapper", "submenu", "cnt-submenu-navi-vertical", "prt-submenu-contents", "chat-body"].forEach(
      function (id) {
        var el = document.getElementById(id);
        if (el) ro.observe(el);
      },
    );
  }

  function watchSub() {
    if (!mo) return;
    var sub = document.getElementById("submenu");
    if (sub) mo.observe(sub, { attributes: true, childList: true, subtree: true });
  }

  window.addEventListener("resize", schedule);
  try {
    ro = new ResizeObserver(schedule);
    attach();
    document.addEventListener("DOMContentLoaded", attach);
    document.addEventListener("hashchange", function () {
      setTimeout(attach, 50);
      schedule();
    });
  } catch (e) {}
  try {
    mo = new MutationObserver(schedule);
    watchSub();
    document.addEventListener("DOMContentLoaded", watchSub);
  } catch (e) {}
})();

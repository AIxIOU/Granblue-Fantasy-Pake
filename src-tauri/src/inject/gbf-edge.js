// EXPERIMENT ONLY. Read Granblue's #wrapper and submenu overlay so Rust can
// snap the native sidebar. Rule 0: this only *reads* geometry.
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
    // Only skip that 320px flash, not Automatic sizes between Small and Large
    // (320*zoom would ignore wrap 388 while getZoom is still 2).
    var zoom = 1;
    try {
      if (typeof Game.getZoom === "function") zoom = Game.getZoom() || 1;
    } catch (e) {}
    if (zoom > 1.05 && r.right < 340) return;
    // The mobile client has no #submenu column at all. That is the cleanest
    // signal for which client we were served, and Rust needs it: on mobile
    // there are no Window Size settings, so "Automatic" means something else.
    t.core.invoke("gbf_game_edge", {
      right: r.right,
      overlay: overlayRight(),
      automatic: auto,
      dpr: window.devicePixelRatio || 1,
      zoom: zoom,
      mobile: !document.getElementById("submenu"),
    });
  };

  window.__gbfSetHug = function (on) {
    window.__gbfHug = !!on;
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
      ["wrapper", "submenu", "cnt-submenu-navi-vertical", "prt-submenu-contents", "chat-body"].forEach(
        function (id) {
          var el = document.getElementById(id);
          if (el) ro.observe(el);
        },
      );
    }
    attach();
    document.addEventListener("DOMContentLoaded", attach);
    document.addEventListener("hashchange", function () {
      setTimeout(attach, 50);
      schedule();
    });
  } catch (e) {}
  try {
    var mo = new MutationObserver(schedule);
    function watchSub() {
      var sub = document.getElementById("submenu");
      if (sub) mo.observe(sub, { attributes: true, childList: true, subtree: true });
    }
    watchSub();
    document.addEventListener("DOMContentLoaded", watchSub);
  } catch (e) {}
})();

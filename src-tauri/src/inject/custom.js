// EXPERIMENT ONLY. Injected into the GAME webview.
//
// Drag-to-scroll is Rule 0 exception 1, copied from gbf-scaler.js with the
// same boundary: overflow-y + scrollHeight, scrollTop only, time-bounded
// click suppression, no GBF class/id. Exception 4: dragstart on img/a only.
// Alt shortcuts: exception 5 (see gbf-keys.js, inlined below so this file
// is the single initialization_script already wired by Pake).

(function () {
  if (window.__gbfNativeDrag) return;
  window.__gbfNativeDrag = true;

  var DRAG_SCROLL_THRESHOLD = 4;
  var DRAG_CLICK_SUPPRESS_MS = 250;
  var MOMENTUM_MIN_VELOCITY = 0.15;
  var MOMENTUM_FRICTION = 0.95;
  var MOMENTUM_SAMPLE_WINDOW = 100;

  function createMomentumScroller(getEl) {
    var history = [];
    var rafId = null;
    function reset() {
      history = [];
    }
    function sample(y) {
      var now = performance.now();
      history.push({ t: now, y: y });
      var cutoff = now - MOMENTUM_SAMPLE_WINDOW;
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
      var first = history[0];
      var last = history[history.length - 1];
      var dt = last.t - first.t;
      if (dt <= 0) return 0;
      return -(last.y - first.y) / dt;
    }
    function release() {
      var el = getEl();
      var v0 = velocityPxPerMs();
      reset();
      if (!el || Math.abs(v0) < MOMENTUM_MIN_VELOCITY) return;
      var velocity = v0;
      var lastFrame = performance.now();
      stop();
      function tick(now) {
        var dt = Math.min(now - lastFrame, 48);
        lastFrame = now;
        velocity *= MOMENTUM_FRICTION;
        el.scrollTop = Math.max(
          0,
          Math.min(el.scrollHeight - el.clientHeight, el.scrollTop + velocity * dt),
        );
        var maxScroll = el.scrollHeight - el.clientHeight;
        var atBound = el.scrollTop <= 0 || el.scrollTop >= maxScroll;
        if (Math.abs(velocity) < MOMENTUM_MIN_VELOCITY || atBound) {
          rafId = null;
          return;
        }
        rafId = requestAnimationFrame(tick);
      }
      rafId = requestAnimationFrame(tick);
    }
    return { sample: sample, release: release, stop: stop, reset: reset };
  }

  document.addEventListener(
    "dragstart",
    function (e) {
      var t = e.target;
      if (!t || typeof t.closest !== "function") return;
      if (!t.closest("img, a")) return;
      e.preventDefault();
    },
    true,
  );

  var pageDragTarget = null;
  var pageDragging = false;
  var pageDragMoved = false;
  var pageDragEndedAt = 0;
  var pageDragStartY = 0;
  var pageDragStartTop = 0;

  function findScrollableAncestor(el) {
    while (el && el !== document.body && el !== document.documentElement) {
      var cs = window.getComputedStyle(el);
      if (
        (cs.overflowY === "auto" || cs.overflowY === "scroll") &&
        el.scrollHeight > el.clientHeight + 1
      ) {
        return el;
      }
      el = el.parentElement;
    }
    var scroller = document.scrollingElement || document.documentElement;
    if (scroller && scroller.scrollHeight > scroller.clientHeight + 1) return scroller;
    return null;
  }

  var pageMomentum = createMomentumScroller(function () {
    return pageDragTarget;
  });

  document.addEventListener(
    "pointerdown",
    function (e) {
      if (e.button !== 0) return;
      var target = findScrollableAncestor(e.target);
      if (!target) return;
      pageMomentum.stop();
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
    function (e) {
      if (!pageDragging) return;
      var delta = e.clientY - pageDragStartY;
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
    if (pageDragMoved) pageDragEndedAt = Date.now();
    pageDragMoved = false;
  }
  window.addEventListener("pointerup", endPageDragScroll);
  window.addEventListener("pointercancel", endPageDragScroll);

  document.addEventListener(
    "click",
    function (e) {
      if (Date.now() - pageDragEndedAt < DRAG_CLICK_SUPPRESS_MS) {
        pageDragEndedAt = 0;
        e.preventDefault();
        e.stopPropagation();
      }
    },
    true,
  );
})();

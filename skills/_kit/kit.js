/* momputer's page kit: small helpers every skill's page shares. No library, no network.
   Builds nodes with textContent only: names in the data are written by strangers.
   Motion comes from the browser itself (the Web Animations API, View Transitions), plays once,
   and steps aside when the person's system asks for less motion. Nothing loops at rest.

   Kept in skills/_kit and copied into each page by skills/_kit/sync.py. */
var Kit = (function () {
  "use strict";
  var SVGNS = "http://www.w3.org/2000/svg";
  var reduce = window.matchMedia ? window.matchMedia("(prefers-reduced-motion: reduce)") : { matches: false };

  function motion() { return !reduce.matches; }
  function $(id) { return document.getElementById(id); }

  function fill(n, attrs, kids, from) {
    if (attrs) Object.keys(attrs).forEach(function (k) {
      var v = attrs[k];
      if (v == null || v === false) return;
      if (k === "text") n.textContent = v;
      else if (k === "class") n.setAttribute("class", v);
      else if (k === "style" && typeof v === "object") Object.keys(v).forEach(function (p) { n.style.setProperty(p, v[p]); });
      else if (k.slice(0, 2) === "on" && typeof v === "function") n.addEventListener(k.slice(2), v);
      else n.setAttribute(k, v === true ? "" : v);
    });
    for (var i = from; i < kids.length; i++) {
      var c = kids[i];
      if (c == null || c === false) continue;
      if (Array.isArray(c)) c.forEach(function (x) { if (x != null && x !== false) n.appendChild(typeof x === "string" ? document.createTextNode(x) : x); });
      else n.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
    }
    return n;
  }
  /** el("div", {class: "x", text: "..."}, child, ...): never innerHTML. */
  function el(tag, attrs) { return fill(document.createElement(tag), attrs, arguments, 2); }
  function svg(tag, attrs) { return fill(document.createElementNS(SVGNS, tag), attrs, arguments, 2); }

  // ---- numbers
  function bytes(n, digits) {
    if (n == null || isNaN(n)) return "–";
    var u = ["B", "KB", "MB", "GB", "TB", "PB"], i = 0;
    n = Math.max(0, n);
    while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
    var d = digits != null ? digits : (n < 10 && i ? 1 : 0);
    return (i ? n.toFixed(d) : Math.round(n)) + " " + u[i];
  }
  /** The number and its unit apart, for a readout: ["94", "GB"]. */
  function bytesParts(n) { var s = bytes(n); var i = s.lastIndexOf(" "); return [s.slice(0, i), s.slice(i + 1)]; }
  function count(n) { return (n || 0).toLocaleString("en-US"); }
  function pct(part, whole) {
    if (!whole) return "0%";
    var p = 100 * part / whole;
    return (p < 1 && part > 0 ? "<1" : p > 99 && p < 100 ? ">99" : Math.round(p)) + "%";
  }
  function plural(n, one, many) { return count(n) + " " + (n === 1 ? one : (many || one + "s")); }

  // ---- icons: fixed paths drawn by us
  var ICONS = {
    search: "M11 4a7 7 0 1 0 4.4 12.4l4.1 4.1 1.4-1.4-4.1-4.1A7 7 0 0 0 11 4zm0 2a5 5 0 1 1 0 10 5 5 0 0 1 0-10z",
    copy: "M8 3h10a2 2 0 0 1 2 2v12h-2V5H8V3zm-3 4h10a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2zm0 2v11h10V9H5z",
    check: "M9.5 16.2 5.3 12l-1.4 1.4 5.6 5.6L20.1 8.4 18.7 7z",
    chev: "M9 6l6 6-6 6-1.4-1.4L12.2 12 7.6 7.4z",
    up: "M12 7l6 6-1.4 1.4L12 9.8l-4.6 4.6L6 13z",
    sun: "M12 7a5 5 0 1 1 0 10 5 5 0 0 1 0-10zm0-5h1v3h-2V2h1zm0 17h1v3h-2v-3h1zM2 11h3v2H2v-2zm17 0h3v2h-3v-2zM4.2 5.6l1.4-1.4 2.1 2.1-1.4 1.4-2.1-2.1zm12.1 12.1 1.4-1.4 2.1 2.1-1.4 1.4-2.1-2.1zM5.6 19.8l-1.4-1.4 2.1-2.1 1.4 1.4-2.1 2.1zM17.7 7.7l-1.4-1.4 2.1-2.1 1.4 1.4-2.1 2.1z",
    moon: "M14.5 3a8.5 8.5 0 1 0 6.4 14.1A7 7 0 0 1 14.5 3z",
    keys: "M3 6h18a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zm1 2v8h16V8H4zm2 1h2v2H6V9zm3 0h2v2H9V9zm3 0h2v2h-2V9zm3 0h3v2h-3V9zM6 13h12v2H6v-2z",
    note: "M5 3h10l4 4v14H5V3zm2 2v14h10V8h-3V5H7zm2 6h6v2H9v-2zm0 4h6v2H9v-2z"
  };
  function icon(name, cls) {
    return svg("svg", { viewBox: "0 0 24 24", "class": cls || null, "aria-hidden": "true", fill: "currentColor" }, svg("path", { d: ICONS[name] || "" }));
  }

  // ---- storage (a private window or blocked storage just means nothing is remembered)
  function store(key) {
    return {
      get: function (fallback) { try { var v = localStorage.getItem(key); return v == null ? fallback : JSON.parse(v); } catch (e) { return fallback; } },
      set: function (v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch (e) { /* not remembered */ } }
    };
  }

  // ---- motion
  function animate(node, frames, opts) {
    if (!motion() || !node || !node.animate) return null;
    try { return node.animate(frames, Object.assign({ duration: 420, easing: "cubic-bezier(0.2, 0.7, 0.2, 1)", fill: "both" }, opts || {})); }
    catch (e) { return null; }
  }
  /** Calls step(t) from 0 to 1 over ms, eased; once. */
  function tween(ms, step, done) {
    if (!motion()) { step(1); if (done) done(); return; }
    var start = null;
    function frame(now) {
      if (start == null) start = now;
      var t = Math.min(1, (now - start) / ms), e = 1 - Math.pow(1 - t, 3);
      step(e);
      if (t < 1) requestAnimationFrame(frame); else if (done) done();
    }
    requestAnimationFrame(frame);
  }
  /** Rolls a readout from its last value to `to`. fmt turns a number into its text. */
  function countUp(node, to, fmt, ms) {
    var from = node.__value != null ? node.__value : 0;
    node.__value = to;
    if (from === to) { node.textContent = fmt(to); return; }
    tween(ms || 700, function (e) { node.textContent = fmt(from + (to - from) * e); });
  }
  /** Each [data-reveal] in `root`: what's on the first screen rises in, one after another; what's
      further down is never hidden (a full-page picture or a print shows it) and lifts gently into
      place as it scrolls in. Each gets a "kit:shown" event when it plays. */
  function reveal(root) {
    var items = Array.prototype.slice.call((root || document).querySelectorAll("[data-reveal]:not(.shown)"));
    function shown(n) { n.dispatchEvent(new CustomEvent("kit:shown")); }
    if (!motion() || !("IntersectionObserver" in window)) { items.forEach(function (n) { n.classList.add("shown"); shown(n); }); return; }
    var fold = window.innerHeight, later = [], first = 0;
    items.forEach(function (n) {
      n.classList.add("shown");
      if (n.getBoundingClientRect().top < fold * 0.96) {
        animate(n, [{ opacity: 0, transform: "translateY(16px)" }, { opacity: 1, transform: "none" }], { duration: 620, delay: 80 + Math.min(first++, 10) * 60 });
        shown(n);
      } else later.push(n);
    });
    if (!later.length) return;
    var io = new IntersectionObserver(function (entries) {
      var k = 0;
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        io.unobserve(en.target);
        animate(en.target, [{ opacity: 0.4, transform: "translateY(18px)" }, { opacity: 1, transform: "none" }], { duration: 640, delay: Math.min(k++, 6) * 50, fill: "none" });
        shown(en.target);
      });
    }, { rootMargin: "0px 0px -4% 0px" });
    later.forEach(function (n) { io.observe(n); });
  }
  /** Runs fn once `node` is on screen (right away if motion is off). */
  function whenShown(node, fn) {
    if (!motion() || !("IntersectionObserver" in window)) { fn(); return; }
    var io = new IntersectionObserver(function (entries) {
      if (entries.some(function (e) { return e.isIntersecting; })) { io.disconnect(); fn(); }
    });
    io.observe(node);
  }
  /** A <details class="fold"> whose body slides open and shut. onOpen runs before the first opening. */
  function smooth(details, onOpen) {
    var summary = details.querySelector("summary"), body = details.querySelector(".fold-body"), busy = null, built = false;
    function build() { if (!built && onOpen) { built = true; onOpen(); } }
    details.addEventListener("toggle", function () { if (details.open) build(); });
    summary.addEventListener("click", function (e) {
      if (!motion() || !body) return;
      e.preventDefault();
      if (busy) busy.cancel();
      if (!details.open) {
        build();
        details.open = true;
        var h = body.scrollHeight;
        busy = body.animate([{ height: "0px", opacity: 0 }, { height: h + "px", opacity: 1 }], { duration: Math.min(520, 200 + h / 3), easing: "cubic-bezier(0.2, 0.7, 0.2, 1)" });
        busy.onfinish = function () { busy = null; };
      } else {
        var h2 = body.scrollHeight;
        busy = body.animate([{ height: h2 + "px", opacity: 1 }, { height: "0px", opacity: 0 }], { duration: Math.min(420, 160 + h2 / 4), easing: "cubic-bezier(0.4, 0, 1, 1)" });
        busy.onfinish = function () { details.open = false; busy = null; };
      }
    });
  }
  /** Swaps content with a crossfade, through a view transition where the browser has one. */
  function swap(fn) {
    if (motion() && document.startViewTransition) { try { document.startViewTransition(fn); return; } catch (e) { /* plain */ } }
    fn();
  }

  // ---- the floating tip
  var tipNode = null;
  function tip(e, title, lines) {
    if (!tipNode) { tipNode = el("div", { "class": "tip", role: "tooltip" }); document.body.appendChild(tipNode); }
    tipNode.textContent = "";
    tipNode.appendChild(el("b", { text: title }));
    (lines || []).forEach(function (l) { if (l) tipNode.appendChild(el("span", { text: l })); });
    var w = tipNode.offsetWidth, h = tipNode.offsetHeight, x = e.clientX + 14, y = e.clientY + 16;
    if (x + w > window.innerWidth - 8) x = e.clientX - w - 14;
    if (y + h > window.innerHeight - 8) y = e.clientY - h - 14;
    tipNode.style.left = Math.max(8, x) + "px";
    tipNode.style.top = Math.max(8, y) + "px";
    tipNode.classList.add("on");
  }
  function untip() { if (tipNode) tipNode.classList.remove("on"); }

  // ---- toast and copying
  var toastTimer = null;
  function toast(msg) {
    var t = $("toast");
    if (!t) { t = el("div", { id: "toast", "class": "toast", role: "status", "aria-live": "polite" }); document.body.appendChild(t); }
    t.textContent = msg;
    t.classList.add("on");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { t.classList.remove("on"); }, 2400);
  }
  /** Puts text on the clipboard. Resolves true, or false when the page has to show it instead. */
  function copy(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text).then(function () { return true; }, function () { return false; });
    }
    return Promise.resolve(false);
  }
  /** A button that says what just happened for a moment ("Copied"), then goes back. */
  function flash(btn, label, ms) {
    if (!btn.__label) btn.__label = Array.prototype.slice.call(btn.childNodes);
    btn.textContent = "";
    btn.appendChild(icon("check"));
    btn.appendChild(document.createTextNode(label));
    btn.setAttribute("data-state", "done");
    animate(btn, [{ transform: "scale(0.94)" }, { transform: "scale(1.04)" }, { transform: "none" }], { duration: 380, fill: "none" });
    clearTimeout(btn.__t);
    btn.__t = setTimeout(function () {
      btn.textContent = "";
      btn.__label.forEach(function (n) { btn.appendChild(n); });
      btn.removeAttribute("data-state");
    }, ms || 1800);
  }

  // ---- light and dark
  function theme(button, key) {
    var s = store(key || "kit-theme"), root = document.documentElement;
    var saved = s.get(null);
    if (saved === "light" || saved === "dark") root.setAttribute("data-theme", saved);
    function current() {
      var t = root.getAttribute("data-theme");
      if (t) return t;
      return window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    }
    function paint() {
      button.textContent = "";
      button.appendChild(icon(current() === "dark" ? "sun" : "moon"));
      button.setAttribute("aria-label", current() === "dark" ? "Switch to light" : "Switch to dark");
      button.title = button.getAttribute("aria-label") + " (T)";
    }
    function flip() {
      var next = current() === "dark" ? "light" : "dark";
      swap(function () { root.setAttribute("data-theme", next); paint(); });
      s.set(next);
      document.dispatchEvent(new CustomEvent("kit:theme"));
    }
    button.addEventListener("click", flip);
    paint();
    return { flip: flip, current: current };
  }

  // ---- keys
  var keymap = {};
  document.addEventListener("keydown", function (e) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    var t = e.target, typing = t && (t.tagName === "INPUT" && t.type !== "checkbox" || t.tagName === "TEXTAREA" || t.isContentEditable);
    var h = keymap[e.key];
    if (h && (!typing || h.inInput)) { if (h.fn(e) !== false) e.preventDefault(); }
  });
  function key(k, fn, inInput) { keymap[k] = { fn: fn, inInput: !!inInput }; }

  // ---- the section bar: which section is in view, with a light that slides to it
  function spy(nav) {
    var links = Array.prototype.slice.call(nav.querySelectorAll("[data-target]"));
    links.forEach(function (l) {
      l.addEventListener("click", function () {
        var t = document.getElementById(l.getAttribute("data-target"));
        if (t) t.scrollIntoView({ behavior: motion() ? "smooth" : "auto", block: "start" });
      });
    });
    var ind = el("span", { "class": "ind", "aria-hidden": "true" });
    nav.appendChild(ind);
    // sections that don't fit: an arrow at each end that has more, so nothing is hidden without a sign
    var wrap = el("div", { "class": "secnav-wrap" });
    nav.parentNode.insertBefore(wrap, nav);
    wrap.appendChild(nav);
    var prev = el("button", { "class": "nav-more prev", type: "button", tabindex: "-1", "aria-label": "Earlier sections" }, icon("chev"));
    var next = el("button", { "class": "nav-more next", type: "button", tabindex: "-1", "aria-label": "More sections" }, icon("chev"));
    wrap.appendChild(prev);
    wrap.appendChild(next);
    function edges() {
      var max = nav.scrollWidth - nav.clientWidth;
      wrap.classList.toggle("can-prev", nav.scrollLeft > 2);
      wrap.classList.toggle("can-next", nav.scrollLeft < max - 2);
    }
    function by(d) { nav.scrollBy({ left: d * nav.clientWidth * 0.7, behavior: motion() ? "smooth" : "auto" }); }
    prev.addEventListener("click", function () { by(-1); });
    next.addEventListener("click", function () { by(1); });
    nav.addEventListener("scroll", edges, { passive: true });
    window.addEventListener("resize", edges);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(edges);
    function mark(a) {
      links.forEach(function (l) { l.setAttribute("aria-current", l === a ? "true" : "false"); });
      if (!a) { ind.style.opacity = 0; return; }
      ind.style.opacity = 1;
      ind.style.width = a.offsetWidth + "px";
      ind.style.transform = "translateX(" + a.offsetLeft + "px)";
      if (nav.scrollWidth > nav.clientWidth) nav.scrollTo({ left: a.offsetLeft - 24, behavior: motion() ? "smooth" : "auto" });
    }
    var secs = links.map(function (l) { return document.getElementById(l.getAttribute("data-target")); });
    function onScroll() {
      var y = window.scrollY + 120, best = null;
      secs.forEach(function (s, i) { if (s && s.offsetTop <= y) best = links[i]; });
      mark(best);
    }
    var raf = null;
    window.addEventListener("scroll", function () { if (!raf) raf = requestAnimationFrame(function () { raf = null; onScroll(); }); }, { passive: true });
    window.addEventListener("resize", onScroll);
    onScroll();
    edges();
  }

  /** Mom blinks once, a moment after the page opens. */
  function blink(mom) {
    if (!mom) return;
    var eyes = mom.querySelectorAll(".eye");
    setTimeout(function () {
      Array.prototype.forEach.call(eyes, function (eye) {
        animate(eye, [{ transform: "scaleY(1)" }, { transform: "scaleY(0.08)" }, { transform: "scaleY(1)" }], { duration: 260, easing: "ease-in-out", fill: "none" });
      });
    }, 900);
  }

  /** Start: motion classes, then reveal. */
  function start() {
    if (motion()) document.documentElement.classList.add("motion");
    reveal(document);
  }

  return {
    $: $, el: el, svg: svg, icon: icon, bytes: bytes, bytesParts: bytesParts, count: count, pct: pct, plural: plural,
    store: store, motion: motion, animate: animate, tween: tween, countUp: countUp, reveal: reveal, whenShown: whenShown,
    smooth: smooth, swap: swap, tip: tip, untip: untip, toast: toast, copy: copy, flash: flash, theme: theme, key: key,
    spy: spy, blink: blink, start: start
  };
})();

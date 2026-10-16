/* momputer's page kit: small helpers every skill's page shares. No library, no network.
   Builds nodes with textContent only: names in the data are written by strangers.
   Motion comes from the browser itself (the Web Animations API, View Transitions). It answers
   what the person does, plus at most one moment on opening; it never hides content, never loops,
   and steps aside when the person's system asks for less motion.

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
  /** el("div", {class: "x", text: "..."}, child, ...): text goes in as text, never parsed as markup. */
  function el(tag, attrs) { return fill(document.createElement(tag), attrs, arguments, 2); }
  function svg(tag, attrs) { return fill(document.createElementNS(SVGNS, tag), attrs, arguments, 2); }

  // ---- numbers
  function bytes(n, digits) {
    if (n == null || isNaN(n)) return "–";
    var u = ["B", "KB", "MB", "GB", "TB", "PB"], i = 0;
    n = Math.max(0, n);
    while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
    var d = typeof digits === "number" ? digits : (n < 10 && i ? 1 : 0);   // a formatter may be handed a row as its second argument
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
  /** Tells each [data-reveal] in `root` when it first comes into view, with a "kit:shown" event, so
      a page can play its one moment there. Never hides anything and never animates by itself:
      content that waits for an animation is content people can't read yet. */
  function reveal(root) {
    var items = Array.prototype.slice.call((root || document).querySelectorAll("[data-reveal]:not(.shown)"));
    function shown(n) { n.classList.add("shown"); n.dispatchEvent(new CustomEvent("kit:shown")); }
    if (!("IntersectionObserver" in window)) { items.forEach(shown); return; }
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) { if (en.isIntersecting) { io.unobserve(en.target); shown(en.target); } });
    }, { rootMargin: "0px 0px -8% 0px" });
    items.forEach(function (n) { io.observe(n); });
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
        busy = body.animate([{ height: "0px", opacity: 0 }, { height: h + "px", opacity: 1 }], { duration: Math.min(360, 180 + h / 4), easing: "cubic-bezier(0.2, 0.7, 0.2, 1)" });
        busy.onfinish = function () { busy = null; };
      } else {
        var h2 = body.scrollHeight;
        busy = body.animate([{ height: h2 + "px", opacity: 1 }, { height: "0px", opacity: 0 }], { duration: Math.min(280, 140 + h2 / 5), easing: "cubic-bezier(0.4, 0, 1, 1)" });
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
    animate(btn, [{ transform: "scale(0.96)" }, { transform: "none" }], { duration: 220, fill: "none" });
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
      secs.forEach(function (s, i) { if (s && s.offsetTop <= y && s.offsetParent !== null) best = links[i]; });
      // at the very bottom a short last section can't reach the top: light the last one shown
      if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 4) {
        for (var i = secs.length - 1; i >= 0; i--) if (secs[i] && secs[i].offsetParent !== null) { best = links[i]; break; }
      }
      mark(best);
    }
    var raf = null;
    window.addEventListener("scroll", function () { if (!raf) raf = requestAnimationFrame(function () { raf = null; onScroll(); }); }, { passive: true });
    window.addEventListener("resize", onScroll);
    onScroll();
    edges();
  }

  // ---- meters: a row of dots, like the widget's
  function dotsRun(from, to) {
    var d = "";
    for (var i = from; i < to; i++) d += "M" + (i + 0.5) + " 0.5h0";
    return d;
  }
  /** A meter of n dots filled to a fraction; with warn, the last four lit go red. Lit dots take the
      CSS colour --c. Gives back { node, set(fraction) }: set it again to animate the fill. */
  function meter(fraction, opts) {
    opts = opts || {};
    var n = opts.n || 60;
    var node = svg("svg", { "class": "meter", viewBox: "0 0 " + n + " 1", preserveAspectRatio: "xMinYMid meet", "aria-hidden": "true" });
    var off = svg("path", { "class": "m-off" }), lit = svg("path", { "class": "m-on" }), hot = svg("path", { "class": "m-hot" });
    node.appendChild(off);
    node.appendChild(lit);
    node.appendChild(hot);
    var full = Math.max(fraction > 0 ? 1 : 0, Math.round(Math.max(0, Math.min(1, fraction || 0)) * n));
    function set(t) {
      var k = Math.round(full * Math.max(0, Math.min(1, t))), h = opts.warn ? Math.max(0, full - 4) : full;
      off.setAttribute("d", dotsRun(k, n));
      lit.setAttribute("d", dotsRun(0, Math.min(k, h)));
      hot.setAttribute("d", dotsRun(Math.min(k, h), k));
    }
    set(1);
    return { node: node, set: set };
  }
  /** One meter shared by several things: parts are [{ v, c }], lit in proportion to total, each in
      its own colour c. Gives back { node, set(t) } where t from 0 to 1 lights them up in turn. */
  function stack(parts, opts) {
    opts = opts || {};
    var n = opts.n || 48, sum = 0;
    parts = parts.filter(function (p) { return p.v > 0; });
    parts.forEach(function (p) { sum += p.v; });
    var total = opts.total || sum || 1;
    var lit = Math.min(n, Math.max(sum > 0 ? 1 : 0, Math.round(n * sum / total)));
    var counts = parts.map(function (p) { return Math.floor(lit * p.v / (sum || 1)); });
    var left = lit - counts.reduce(function (a, b) { return a + b; }, 0);
    parts.map(function (p, i) { return [lit * p.v / (sum || 1) - counts[i], i]; })
      .sort(function (a, b) { return b[0] - a[0]; })
      .forEach(function (r) { if (left > 0) { counts[r[1]]++; left--; } });
    var node = svg("svg", { "class": "meter", viewBox: "0 0 " + n + " 1", preserveAspectRatio: "xMinYMid meet", "aria-hidden": "true" });
    var off = svg("path", { "class": "m-off" });
    node.appendChild(off);
    var paths = parts.map(function (p) { return node.appendChild(svg("path", { "class": "m-on", style: { stroke: p.c } })); });
    function set(t) {
      var upto = Math.round(lit * Math.max(0, Math.min(1, t))), at = 0;
      paths.forEach(function (path, i) {
        path.setAttribute("d", dotsRun(at, Math.min(at + counts[i], upto)));
        at += counts[i];
      });
      off.setAttribute("d", dotsRun(Math.min(upto, lit), n));
    }
    set(1);
    return { node: node, set: set };
  }

  // ---- charts: dots, like the widget's meters. They take plain numbers and labels, build
  //      nodes with textContent only, and give every mark a tooltip. Colours are CSS values
  //      (a page's --cN lights, or ink); text stays ink, never the colour.

  /** A light: a small dot in a colour (ink-3 when none is given). */
  function led(color) {
    return el("span", { "class": color ? "led c" : "led", style: color ? { "--c": color } : null, "aria-hidden": "true" });
  }
  /** Shares of n whole dots for values, by largest remainder (so they always add up to n). */
  function share(values, n) {
    var sum = 0;
    values.forEach(function (v) { sum += Math.max(0, v || 0); });
    if (!sum || n <= 0) return values.map(function () { return 0; });
    var exact = values.map(function (v) { return n * Math.max(0, v || 0) / sum; });
    var out = exact.map(Math.floor), left = n - out.reduce(function (a, b) { return a + b; }, 0);
    exact.map(function (x, i) { return [x - out[i], i]; }).sort(function (a, b) { return b[0] - a[0]; })
      .forEach(function (r) { if (left > 0) { out[r[1]]++; left--; } });
    return out;
  }
  /** "Show all 40" for a list cut short, so nothing is hidden without a sign. */
  function moreButton(hiddenNodes, noun) {
    var b = el("button", { "class": "link-btn more", type: "button", text: "Show " + count(hiddenNodes.length) + " more" + (noun ? " " + noun : "") });
    b.addEventListener("click", function () { hiddenNodes.forEach(function (n) { n.classList.remove("hidden"); }); b.remove(); });
    return b;
  }
  /** A legend: each series' light and name, with an optional value. */
  function legend(items) {
    return el("div", { "class": "chart-legend" }, items.map(function (it) {
      return el("span", { "class": "who" }, led(it.color), el("b", { text: it.name }), it.value != null ? " " + it.value : null);
    }));
  }
  /** Columns of dots over time. labels: ["2019", ...]; series: [{ name, color, values }] stacked bottom
      up in that order. opts: { rows (dots tall, 12), fmt (value to text), label (what it shows, for
      screen readers), caption }. Gives back the chart element. */
  function columns(labels, series, opts) {
    opts = opts || {};
    var rows = opts.rows || 12, fmt = opts.fmt || String;
    var totals = labels.map(function (_, i) { return series.reduce(function (a, s) { return a + (s.values[i] || 0); }, 0); });
    var max = Math.max.apply(null, totals.concat([0])) || 1;
    var grid = el("div", { "class": "cols", style: { "--n": labels.length }, role: "img", "aria-label": opts.label || "Chart" });
    labels.forEach(function (lab, i) {
      var lit = totals[i] > 0 ? Math.max(1, Math.round(rows * totals[i] / max)) : 0;
      var parts = share(series.map(function (s) { return s.values[i] || 0; }), lit);
      var col = el("div", { "class": "col" });
      var dots = [];
      for (var k = 0; k < rows - lit; k++) dots.push(el("i"));
      for (var j = series.length - 1; j >= 0; j--) for (var m = 0; m < parts[j]; m++) dots.push(el("i", { "class": "on", style: { "--c": series[j].color || "var(--ink)" } }));
      dots.forEach(function (d) { col.appendChild(d); });
      col.addEventListener("pointermove", function (e) {
        tip(e, String(lab), series.length > 1 ? series.filter(function (s) { return s.values[i]; }).map(function (s) { return s.name + ": " + fmt(s.values[i]); }).concat(["Together: " + fmt(totals[i])]) : [fmt(totals[i])]);
      });
      col.addEventListener("pointerleave", untip);
      grid.appendChild(col);
    });
    // every nth label, fewer on a phone (class "s"), so they never run into each other
    var every = Math.max(1, Math.ceil(labels.length / 12)), small = Math.max(every, Math.ceil(labels.length / 5));
    var labs = el("div", { "class": "cols-labels", style: { "--n": labels.length }, "aria-hidden": "true" },
      labels.map(function (lab, i) {
        var w = i % every === 0, sm = i % small === 0;
        return el("span", { "class": (w ? "w" : "") + (sm ? " s" : ""), text: w || sm ? String(lab) : "" });
      }));
    return el("div", { "class": "chart" }, grid, labs,
      series.length > 1 ? legend(series.map(function (s) { return { name: s.name, color: s.color }; })) : null,
      opts.caption ? el("p", { "class": "chart-caption", text: opts.caption }) : null);
  }
  /** A grid of dots sized by value, such as weekday by hour. values[r][c]; rowLabels and colLabels
      are short text. opts: { color, fmt, label, every (show every nth column label) }. */
  function heat(rowLabels, colLabels, values, opts) {
    opts = opts || {};
    var fmt = opts.fmt || String, max = 0;
    values.forEach(function (r) { r.forEach(function (v) { if (v > max) max = v; }); });
    var grid = el("div", { "class": "heat", style: { "--cols": colLabels.length }, role: "img", "aria-label": opts.label || "Chart" });
    rowLabels.forEach(function (rl, r) {
      grid.appendChild(el("span", { "class": "heat-l", text: rl }));
      colLabels.forEach(function (cl, c) {
        var v = (values[r] || [])[c] || 0;
        var cell = el("i", { "class": v ? "on" : null, style: { "--s": v ? (0.3 + 0.7 * Math.sqrt(v / (max || 1))).toFixed(2) : "0.3", "--c": opts.color || "var(--ink)" } });
        cell.addEventListener("pointermove", function (e) { tip(e, rl + " " + cl, [fmt(v)]); });
        cell.addEventListener("pointerleave", untip);
        grid.appendChild(cell);
      });
    });
    var every = opts.every || Math.max(1, Math.ceil(colLabels.length / 8));
    grid.appendChild(el("span", { "class": "heat-l" }));
    colLabels.forEach(function (cl, c) { grid.appendChild(el("span", { "class": "heat-x", text: c % every === 0 ? cl : "" })); });
    return el("div", { "class": "chart" }, grid, opts.caption ? el("p", { "class": "chart-caption", text: opts.caption }) : null);
  }
  /** A hundred dots split between parts: [{ name, v, color }]. With its legend beside it. */
  function waffle(parts, opts) {
    opts = opts || {};
    var fmt = opts.fmt || String, total = 0;
    parts.forEach(function (p) { total += p.v || 0; });
    var counts = share(parts.map(function (p) { return p.v; }), 100);
    var grid = el("div", { "class": "waffle", role: "img", "aria-label": opts.label || "Chart" });
    parts.forEach(function (p, i) {
      for (var k = 0; k < counts[i]; k++) {
        var d = el("i", { "class": "on", style: { "--c": p.color || "var(--ink)" } });
        d.addEventListener("pointermove", function (e) { tip(e, p.name, [fmt(p.v) + " · " + pct(p.v, total)]); });
        d.addEventListener("pointerleave", untip);
        grid.appendChild(d);
      }
    });
    for (var r = grid.children.length; r < 100; r++) grid.appendChild(el("i"));
    var key = el("ul", { "class": "rows waffle-key" }, parts.map(function (p) {
      return el("li", { "class": "row" }, led(p.color), el("span", { "class": "name", text: p.name }), el("span", { "class": "num", text: fmt(p.v) }), el("span", { "class": "num2", text: pct(p.v, total) }));
    }));
    return el("div", { "class": "chart waffle-wrap" }, grid, key);
  }
  /** A ranked list with a meter on each row: [{ name, sub, v, v2, color }]. opts: { fmt, fmt2, limit, n, max }. */
  function bars(rowsIn, opts) {
    opts = opts || {};
    var fmt = opts.fmt || String, limit = opts.limit || 10, max = opts.max || 0;
    rowsIn.forEach(function (r) { if (r.v > max) max = r.v; });
    var ul = el("ul", { "class": "rows bars" }), cut = [];
    rowsIn.forEach(function (r, i) {
      var m = meter(max ? r.v / max : 0, { n: opts.n || 40 });
      m.node.style.setProperty("--c", r.color || "var(--ink)");
      var li = el("li", { "class": "row" + (i >= limit ? " hidden" : ""), "data-q": opts.searchable ? String(r.name + " " + (r.sub || "")).toLowerCase() : null },
        el("span", { "class": "name", text: r.name }), m.node, el("span", { "class": "num", text: fmt(r.v) }),
        el("span", { "class": "num2", text: r.v2 != null ? (opts.fmt2 || String)(r.v2) : "" }),
        r.sub ? el("span", { "class": "row-note", text: r.sub }) : null);
      if (i >= limit) cut.push(li);
      ul.appendChild(li);
    });
    if (cut.length) ul.appendChild(el("li", { "class": "more-line" }, moreButton(cut, opts.noun)));
    return ul;
  }
  /** A table you can sort by any column. cols: [{ key, label, num, mono, fmt, led }]; rows: objects.
      opts: { limit, sort: key, desc, q: row -> search text, noun }. */
  function table(cols, rowsIn, opts) {
    opts = opts || {};
    var limit = opts.limit || 25, sortKey = opts.sort || null, desc = opts.desc !== false;
    var body = el("tbody"), head = el("tr"), cut = [], foot = el("div", { "class": "tbl-foot" });
    function cell(c, r) {
      var v = r[c.key], text = c.fmt ? c.fmt(v, r) : (v == null ? "–" : String(v));
      return el("td", { "class": (c.num ? "n" : "") + (c.mono ? " mono" : "") }, c.led ? led(c.led(r)) : null, c.led ? " " : null, text);
    }
    function paint() {
      body.textContent = ""; foot.textContent = ""; cut = [];
      var rs = rowsIn.slice();
      if (sortKey) rs.sort(function (a, b) {
        var x = a[sortKey], y = b[sortKey];
        var o = typeof x === "number" && typeof y === "number" ? x - y : String(x == null ? "" : x).localeCompare(String(y == null ? "" : y));
        return desc ? -o : o;
      });
      rs.forEach(function (r, i) {
        var tr = el("tr", { "class": i >= limit ? "hidden" : null, "data-q": opts.q ? String(opts.q(r)).toLowerCase() : null }, cols.map(function (c) { return cell(c, r); }));
        if (i >= limit) cut.push(tr);
        body.appendChild(tr);
      });
      if (cut.length) foot.appendChild(moreButton(cut, opts.noun));
      Array.prototype.forEach.call(head.children, function (th, j) { th.setAttribute("aria-sort", cols[j].key === sortKey ? (desc ? "descending" : "ascending") : "none"); });
    }
    cols.forEach(function (c) {
      var b = el("button", { type: "button", text: c.label });
      b.addEventListener("click", function () { if (sortKey === c.key) desc = !desc; else { sortKey = c.key; desc = !!c.num; } paint(); });
      head.appendChild(el("th", { "class": c.num ? "n" : null, scope: "col" }, b));
    });
    paint();
    return el("div", { "class": "tbl-wrap" }, el("table", { "class": "tbl" }, el("thead", null, head), body), foot);
  }
  /** A few figures in a row, each a dot-matrix number over its label: [{ value, unit, label }]. */
  function facts(items) {
    return el("div", { "class": "facts" }, items.map(function (f) {
      return el("div", { "class": "fact" }, el("div", { "class": "fact-v" }, el("span", { "class": "dots", text: String(f.value) }), f.unit ? el("span", { "class": "u", text: f.unit }) : null),
        el("div", { "class": "fact-l", text: f.label }));
    }));
  }
  /** Mom's take: what the agent noticed, each with the lights of what it's about. [{ title, text, colors }]. */
  function insights(list) {
    return el("ul", { "class": "group rows insights" }, list.map(function (it) {
      var lights = (it.colors || []).length ? it.colors.map(led) : [led(null)];
      return el("li", { "class": "row" }, el("span", { "class": "ins-leds" }, lights),
        el("div", { "class": "ins-body" }, it.title ? el("b", { text: it.title }) : null, el("p", { text: it.text })));
    }));
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
    spy: spy, meter: meter, stack: stack, led: led, share: share, legend: legend, columns: columns, heat: heat,
    waffle: waffle, bars: bars, table: table, facts: facts, insights: insights, blink: blink, start: start
  };
})();

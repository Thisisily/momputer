// What's alive on the page: mom herself, a widget you can press, cards and dots that answer your
// pointer, the star count, sharing and the tip jar. The page reads the same without any of it.
// Nothing here runs on a loop: things move when you scroll, point or press, and then they rest.
(() => {
  "use strict";

  const REPO = "Thisisily/momputer";
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const fine = matchMedia("(hover: hover) and (pointer: fine)").matches;
  const root = document.documentElement;

  // ------------------------------------------------------------------ mom says

  // A short line from mom at the foot of the window, and a face to go with it if she's on the page.
  let mom = null;
  const says = $("#says");
  let saysTimer;
  function say(text, mood) {
    if (!says) return;
    $("span", says).textContent = text;
    says.hidden = false;
    requestAnimationFrame(() => says.classList.add("on"));
    clearTimeout(saysTimer);
    saysTimer = setTimeout(() => {
      says.classList.remove("on");
      saysTimer = setTimeout(() => (says.hidden = true), 300);
      if (mom) mom.set("look");
    }, 3200);
    if (mom && mood) mom.set(mood);
  }

  // ------------------------------------------------------------------ mom herself

  // The widget's own mom (its script and GSAP come with the demo): she blinks, follows your
  // pointer and squishes when you boop her. Until she's loaded, and if she can't be, the picture stays.
  const script = (src) =>
    new Promise((done, failed) => {
      const el = Object.assign(document.createElement("script"), { src, onload: done, onerror: failed });
      document.head.append(el);
    });
  async function wakeMom() {
    const screen = $("#mom");
    if (!screen) return;
    try {
      await script("demo/vendor/gsap.min.js");
      await script("demo/vendor/MorphSVGPlugin.min.js");
      await script("demo/mom.js");
    } catch {
      return;
    }
    if (!window.momFace) return;
    const label = $("img", screen).alt;
    mom = window.momFace.mount(screen);
    screen.classList.add("alive");
    screen.setAttribute("role", "img");
    screen.setAttribute("aria-label", label);
    screen.addEventListener("pointerenter", () => mom.hover(true));
    screen.addEventListener("pointerleave", () => mom.hover(false));
    const boops = ["c:", "Hello, sweetie.", "That tickles.", "I'm proud of you."];
    let n = 0;
    screen.addEventListener("click", () => {
      mom.boop();
      say(boops[n++ % boops.length], "giggle");
    });
  }

  // ------------------------------------------------------------------ things arrive as you reach them

  if (!calm && "IntersectionObserver" in window) {
    const arriving = $$(".why > *, .section-head, .tile, .pair > .card, .skills, .never-main > *, .proof, .step, .after > .card, .thank, .faq > details");
    const seen = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          e.target.classList.add("in");
          seen.unobserve(e.target);
          const count = $(".readout .dot", e.target);
          if (count) countUp(count);
        }
      },
      { rootMargin: "0px 0px -6% 0px", threshold: 0.06 },
    );
    for (const el of arriving) {
      // Neighbours come in one after the other.
      const place = [...el.parentElement.children].indexOf(el);
      el.style.setProperty("--wait", `${(place % 4) * 70}ms`);
      el.classList.add("rise");
      seen.observe(el);
    }
    root.classList.add("motion");

    // The big wordmark's letters switch on, one by one, once.
    const word = $(".wordmark-big");
    if (word) {
      word.replaceChildren(...[...word.textContent].map((ch, i) => Object.assign(document.createElement("span"), { textContent: ch, style: `--i:${i}` })));
      word.classList.add("switch-on");
    }
  }

  /** A dot-matrix number counts up to itself, once. */
  function countUp(el) {
    const to = Number(el.textContent);
    if (!Number.isInteger(to) || to < 3) return;
    const began = performance.now();
    const tick = (now) => {
      const t = Math.min(1, (now - began) / 700);
      el.textContent = String(Math.round(to * (1 - (1 - t) ** 3)));
      if (t < 1) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }

  // ------------------------------------------------------------------ the pointer: dots, cards, pictures

  if (fine && !calm) {
    // The dot grid lights up around the pointer, a card is lit where the pointer is, and a picture
    // leans a little toward it. One frame per move, nothing in between.
    let waiting = 0;
    let last = null;
    let leaning = null;
    let lit = null;
    const settle = () => {
      waiting = 0;
      const { clientX: x, clientY: y } = last;
      // (Where the pointer is on the page, not in the window: the dots scroll with the page.)
      root.style.setProperty("--px", `${x + scrollX}px`);
      root.style.setProperty("--py", `${y + scrollY}px`);
      const el = document.elementFromPoint(x, y);
      const card = el && el.closest(".card");
      if (lit && lit !== card) {
        lit.style.removeProperty("--mx");
        lit.style.removeProperty("--my");
      }
      lit = card || null;
      if (card) {
        const box = card.getBoundingClientRect();
        card.style.setProperty("--mx", `${x - box.left}px`);
        card.style.setProperty("--my", `${y - box.top}px`);
      }
      const picture = el && el.closest(".tile-shots img, .tile-shots picture, .proof img");
      const img = picture && (picture.tagName === "IMG" ? picture : $("img", picture));
      if (leaning && leaning !== img) lean(leaning, 0, 0);
      if (img) {
        const box = img.getBoundingClientRect();
        lean(img, (x - box.left) / box.width - 0.5, (y - box.top) / box.height - 0.5);
      }
      leaning = img || null;
    };
    const lean = (img, dx, dy) => {
      img.style.setProperty("--ry", `${(dx * 5).toFixed(2)}deg`);
      img.style.setProperty("--rx", `${(-dy * 4).toFixed(2)}deg`);
    };
    addEventListener(
      "pointermove",
      (e) => {
        last = e;
        if (!waiting) waiting = requestAnimationFrame(settle);
      },
      { passive: true },
    );
    // The page moved under a pointer that didn't: what's under it now is what's lit.
    addEventListener("scroll", () => last && !waiting && (waiting = requestAnimationFrame(settle)), { passive: true });
    root.addEventListener("pointerleave", () => {
      if (leaning) lean(leaning, 0, 0);
      if (lit) lit.style.removeProperty("--mx"), lit.style.removeProperty("--my");
      leaning = lit = null;
      root.style.removeProperty("--px");
      root.style.removeProperty("--py");
    });
    root.classList.add("pointer");
  }

  // ------------------------------------------------------------------ stars

  // How many stars she has, asked of GitHub once (and remembered for ten minutes). Shown once
  // there are a few: until then the button says enough.
  function showStars(n) {
    if (!(n >= 10)) return;
    const short = n >= 1000 ? `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k` : String(n);
    for (const el of $$("[data-stars]")) {
      el.textContent = short;
      el.title = `${n.toLocaleString("en")} stars`;
      el.hidden = false;
    }
  }
  (async () => {
    try {
      const kept = JSON.parse(sessionStorage.getItem("mom-stars") || "null");
      if (kept && Date.now() - kept.at < 600_000) return showStars(kept.n);
      const res = await fetch(`https://api.github.com/repos/${REPO}`, { headers: { Accept: "application/vnd.github+json" } });
      if (!res.ok) return;
      const n = Number((await res.json()).stargazers_count);
      sessionStorage.setItem("mom-stars", JSON.stringify({ n, at: Date.now() }));
      showStars(n);
    } catch {
      // No count, then: the button still goes to GitHub.
    }
  })();
  for (const a of $$("[data-star]")) {
    a.addEventListener("pointerenter", () => mom && mom.set("proud"));
    a.addEventListener("pointerleave", () => mom && mom.set("look"));
    a.addEventListener("click", () => say("A star? For me?", "proud"));
  }

  // ------------------------------------------------------------------ sharing

  const share = $("#share");
  if (share) {
    const label = $("#share-label");
    const page = { title: "momputer", text: "momputer: a mom for you, your computer and your AIs", url: "https://thisisily.github.io/momputer/" };
    if (!navigator.share) label.textContent = "Copy the link";
    share.addEventListener("click", async () => {
      try {
        if (navigator.share) {
          await navigator.share(page);
        } else {
          await navigator.clipboard.writeText(page.url);
          label.textContent = "Link copied";
          setTimeout(() => (label.textContent = "Copy the link"), 1800);
        }
        say("Tell them mom says hi.", "wink");
      } catch {
        // They changed their mind, or the browser said no: nothing to do.
      }
    });
  }

  // ------------------------------------------------------------------ the tip jar

  const addr = $("#tip-addr");
  for (const coin of $$(".coin")) {
    coin.addEventListener("click", () => {
      for (const other of $$(".coin")) other.setAttribute("aria-pressed", String(other === coin));
      addr.textContent = coin.dataset.addr;
    });
  }
  const tip = $("button.copy[data-says='coffee']");
  if (tip) tip.addEventListener("click", () => say("Oh, you shouldn't have. c:", "kiss"));
  const dl = $("#dl");
  if (dl) dl.addEventListener("click", () => say("Good choice, sweetie.", "delighted"));

  // ------------------------------------------------------------------ the live widget

  // On a wide screen with a mouse, the picture of the widget becomes the widget: its own page,
  // running on made-up data, in a frame. Its BIG MODE button works too: the page makes room the
  // way the app does, with a plain card that travels while the real thing changes size unseen.
  const slot = $("#demo");
  const W = 420;
  const H = 760;
  const BIG = [1280, 820];
  let frame = null;
  let holder = null;
  let big = false;
  let shade = null;
  let card = null;
  let back = null;
  let cameFrom = null; // what "Open it here" was pressed on, if the widget itself is out of sight

  const tell = (word) => frame && frame.contentWindow && frame.contentWindow.postMessage({ momputer: word }, location.origin);
  const inView = (box) => box.bottom > 80 && box.top < innerHeight - 80;

  function fit() {
    if (!holder) return;
    if (big) {
      const [w, h] = BIG;
      const k = Math.min(1, (innerWidth - 40) / w, (innerHeight - 96) / h);
      Object.assign(holder.style, { left: `${(innerWidth - w * k) / 2}px`, top: `${(innerHeight - h * k) / 2 + 16}px`, width: `${w}px`, height: `${h}px`, transform: `scale(${k})` });
      Object.assign(frame.style, { width: `${w}px`, height: `${h}px`, transform: "none" });
    } else {
      Object.assign(holder.style, { left: "", top: "", width: "", height: "", transform: "" });
      Object.assign(frame.style, { width: `${W}px`, height: `${H}px`, transform: `scale(${slot.clientWidth / W})` });
    }
  }

  /** How big the widget may be here: as wide as it really is, if the window is tall enough for it.
      (The page's head works the same number out before anything is drawn, so nothing jumps.) */
  function room() {
    root.style.setProperty("--demo-w", `${Math.round(Math.max(330, Math.min(W, (innerHeight - 190) * (W / H), innerWidth * 0.3)))}px`);
  }

  /** The plain card travels from one shape to the other; the real thing is swapped in underneath. */
  async function travel(from, to, between) {
    Object.assign(card.style, { left: `${to.left}px`, top: `${to.top}px`, width: `${to.width}px`, height: `${to.height}px`, opacity: "1" });
    card.hidden = false;
    const start = `translate(${from.left - to.left}px, ${from.top - to.top}px) scale(${from.width / to.width}, ${from.height / to.height})`;
    holder.style.visibility = "hidden";
    between();
    await card.animate([{ transform: start }, { transform: "none" }], { duration: calm ? 1 : 340, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" }).finished.catch(() => {});
    holder.style.visibility = "";
    tell("sized");
    await card.animate([{ opacity: 1 }, { opacity: 0 }], { duration: calm ? 1 : 200, easing: "ease-out" }).finished.catch(() => {});
    card.hidden = true;
  }

  const whereBig = () => {
    const k = Math.min(1, (innerWidth - 40) / BIG[0], (innerHeight - 96) / BIG[1]);
    return { left: (innerWidth - BIG[0] * k) / 2, top: (innerHeight - BIG[1] * k) / 2 + 16, width: BIG[0] * k, height: BIG[1] * k };
  };
  const whereSmall = () => {
    const box = slot.getBoundingClientRect();
    return inView(box) || !cameFrom ? box : cameFrom.getBoundingClientRect();
  };

  async function grow() {
    if (big) return tell("sized");
    const from = whereSmall();
    big = true;
    shade.hidden = back.hidden = false;
    root.classList.add("big-open");
    requestAnimationFrame(() => shade.classList.add("on"));
    await travel(from, whereBig(), () => (holder.classList.add("big"), fit()));
    frame.focus();
  }

  async function shrink() {
    if (!big) return tell("sized");
    const from = whereBig();
    big = false;
    shade.classList.remove("on");
    root.classList.remove("big-open");
    back.hidden = true;
    await travel(from, whereSmall(), () => (holder.classList.remove("big"), fit()));
    shade.hidden = true;
    cameFrom = null;
  }

  function startDemo() {
    if (!slot || frame || !fine || innerWidth < 900) return;
    room();
    holder = Object.assign(document.createElement("div"), { className: "demo-live" });
    frame = Object.assign(document.createElement("iframe"), { src: "demo/index.html?demo", title: "A live momputer widget, running on made-up data" });
    frame.setAttribute("scrolling", "no");
    holder.append(frame);
    slot.append(holder);
    fit();

    shade = Object.assign(document.createElement("div"), { className: "big-shade", hidden: true });
    card = Object.assign(document.createElement("div"), { className: "big-card", hidden: true });
    back = Object.assign(document.createElement("button"), { className: "big-back", type: "button", hidden: true, textContent: "Back to the page" });
    document.body.append(shade, card, back);
    const leave = () => big && tell("go-small");
    shade.addEventListener("click", leave);
    back.addEventListener("click", leave);
    addEventListener("keydown", (e) => e.key === "Escape" && leave());

    // If the demo never says it's ready (its files aren't there, say), the picture simply stays.
    const giveUp = setTimeout(() => {
      holder.remove();
      frame = holder = null;
    }, 10_000);
    addEventListener("message", (e) => {
      if (!frame || e.origin !== location.origin || e.source !== frame.contentWindow || !e.data) return;
      const word = e.data.momputer;
      if (word === "ready") {
        clearTimeout(giveUp);
        slot.classList.add("live");
        $("#live-note").hidden = false;
        $("#go-big-row").hidden = false;
      } else if (word === "big") grow();
      else if (word === "small") shrink();
    });
    addEventListener("resize", () => (room(), fit()), { passive: true });
    new ResizeObserver(() => !big && fit()).observe(slot);

    $("#go-big").addEventListener("click", (e) => {
      cameFrom = e.currentTarget;
      tell("go-big");
    });
  }

  // After everything the page needs has loaded, and when the browser has a moment.
  const later = (fn) => (window.requestIdleCallback ? requestIdleCallback(fn, { timeout: 2500 }) : setTimeout(fn, 600));
  const begin = () => later(() => (wakeMom(), startDemo()));
  if (document.readyState === "complete") begin();
  else addEventListener("load", begin, { once: true });
})();

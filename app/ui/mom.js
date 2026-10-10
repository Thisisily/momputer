/* Mom, drawn in SVG: a bun, a big white hairdo, giant round glasses and rosy cheeks.
   `momFace.svg(mood)` draws her once (the icons use it). `momFace.mount(el)` puts a live mom on
   the page, animated with GSAP: she breathes, blinks, turns her head and eyes toward your pointer,
   blushes when you hover over her, squishes when you boop her, and morphs from face to face. */
(() => {
  "use strict";

  const INK = "#000";
  const HAIR = "#fff";
  const STRAND = "#cfcfcf";
  const BLUSH = "#f6a98c";
  const RED = "#ff3b5c";
  const WATER = "#7cc8ff";
  const L = [90, 131]; // left eye
  const R = [214, 131]; // right eye

  const line = (d, w = 5, colour = INK, attrs = "") =>
    `<path ${attrs} d="${d}" fill="none" stroke="${colour}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"/>`;

  // ------------------------------------------------------------------ the parts that never change

  const BUN = `<g class="bun">
      <circle cx="152" cy="22" r="29" fill="${HAIR}" stroke="${INK}" stroke-width="5"/>
      ${line("M136 9 Q151 0 166 7", 3.2, STRAND)}
      ${line("M131 23 Q149 12 171 18", 3.2, STRAND)}
    </g>`;

  const HAIRDO = `<g class="hair">
      <path d="M40 134 C22 100 30 60 72 44 C100 33 126 30 152 30 C178 30 204 33 232 44 C274 60 282 100 264 134 C246 104 200 70 152 64 C104 70 58 104 40 134 Z"
        fill="${HAIR}" stroke="${INK}" stroke-width="5" stroke-linejoin="round"/>
      ${line("M124 40 C98 47 68 66 53 98", 3.4, STRAND)}
      ${line("M115 60 C96 68 78 84 69 106", 3.4, STRAND)}
      ${line("M180 40 C206 47 236 66 251 98", 3.4, STRAND)}
      ${line("M189 60 C208 68 226 84 235 106", 3.4, STRAND)}
      ${line("M144 32 C164 42 164 56 152 64", 5)}
    </g>`;

  const CHEEKS = `<g class="cheeks">
      <ellipse class="blush" cx="60" cy="174" rx="16" ry="9.5" fill="${BLUSH}"/>
      <ellipse class="blush" cx="244" cy="174" rx="16" ry="9.5" fill="${BLUSH}"/>
      ${line("M80 185 l-3 8 M87 185 l-3 8", 2.6)}
      ${line("M221 185 l-3 8 M228 185 l-3 8", 2.6)}
    </g>`;

  const GLASSES = `<g class="glasses">
      <circle cx="${L[0]}" cy="134" r="41" fill="#fff" stroke="${INK}" stroke-width="5"/>
      <circle cx="${R[0]}" cy="134" r="41" fill="#fff" stroke="${INK}" stroke-width="5"/>
      ${line("M131 128 Q152 117 173 128", 5)}
      <path class="glint" d="M62 112 Q70 102 82 99" fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" opacity="0"/>
      <path class="glint" d="M186 112 Q194 102 206 99" fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" opacity="0"/>
    </g>`;

  // ------------------------------------------------------------------ eyes, brows, mouths

  // Each eye sits at its lens; `.look` is what follows the pointer, `.eye` is what blinks.
  const at = ([x, y], inner, cls = "") =>
    `<g transform="translate(${x} ${y})"><g class="look"><g class="eye ${cls}">${inner}</g></g></g>`;
  const pair = (inner, cls) => at(L, inner, cls) + at(R, inner, cls);

  const OPEN = `<circle r="20" fill="url(#mom-iris)" stroke="${INK}" stroke-width="2.5"/>
      <circle cx="7" cy="-7.5" r="6.5" fill="#fff"/>
      <circle cx="-6.5" cy="7" r="2.8" fill="#fff"/>`;
  const HEART = `<path d="M0 15 C-20 2 -25 -9 -17 -16 C-10 -21 -3 -18 0 -11 C3 -18 10 -21 17 -16 C25 -9 20 2 0 15 Z" fill="${RED}" stroke="${INK}" stroke-width="3" stroke-linejoin="round"/>
    <ellipse cx="-9" cy="-9" rx="4" ry="2.6" fill="#fff" opacity=".7" transform="rotate(-30 -9 -9)"/>`;

  const EYES = {
    open: pair(OPEN, "open"),
    happy: pair(line("M-15 7 Q0 -15 15 7", 5.5)),
    shut: pair(line("M-16 -2 Q0 15 16 -2", 5.5)),
    flat: pair(line("M-17 0 L17 0", 7)),
    hearts: pair(HEART, "heart"),
    cross: pair(line("M-11 -11 L11 11 M11 -11 L-11 11", 5.5)),
    wink: at(L, line("M-15 7 Q0 -15 15 7", 5.5)) + at(R, OPEN, "open"),
  };

  const brows = (d) => `<g class="brows">${d}</g>`;
  const BROWS = {
    none: "",
    stern: brows(line("M72 99 L104 110 M232 99 L200 110", 5.5)),
    cross: brows(line("M70 94 L106 112 M234 94 L198 112", 6)),
    worried: brows(line("M76 108 L102 98 M228 108 L202 98", 5)),
    up: brows(line("M74 100 Q89 92 104 99 M200 99 Q215 92 230 100", 5)),
    hmm: brows(line("M75 101 Q90 91 105 101", 5) + line("M197 109 L231 106", 6.5)),
  };

  // Line mouths morph into each other; filled ones pop in.
  const TONGUE = "#f27a93";
  const lineMouth = (d, w) => line(d, w, INK, 'class="mouth-line"');
  const MOUTHS = {
    // Her usual face: a big "c:" smile.
    grin: lineMouth("M137 149 C137 172 167 172 167 149", 5),
    smile: lineMouth("M139 153 Q152 169 165 153", 4.8),
    uwu: lineMouth("M133 151 C133 166 151 166 152 155 C153 166 171 166 171 151", 5),
    flat: lineMouth("M142 161 L162 161", 5.5),
    sideways: lineMouth("M146 161 L165 159", 5.5),
    small: lineMouth("M144 157 Q152 165 160 157", 4.5),
    kiss: lineMouth("M146 147 Q160 149 151 157 Q160 163 146 167", 4.5),
    wobble: lineMouth("M138 160 Q145 152 152 160 Q159 168 166 160", 4.8),
    o: `<g class="mouth-fill"><ellipse cx="152" cy="160" rx="7" ry="9" fill="${INK}"/></g>`,
    open: `<g class="mouth-fill"><path d="M137 151 Q152 154 167 151 Q166 173 152 173 Q138 173 137 151 Z" fill="${INK}" stroke="${INK}" stroke-width="3" stroke-linejoin="round"/>
      <path d="M144 167 Q152 160 160 167 Q156 172 152 172 Q148 172 144 167 Z" fill="${TONGUE}"/></g>`,
    yell: `<g class="mouth-fill"><path d="M140 170 Q141 150 152 149 Q163 150 164 170 Z" fill="${INK}" stroke="${INK}" stroke-width="3" stroke-linejoin="round"/>
      <path d="M144 169 Q152 161 160 169 Z" fill="${TONGUE}"/></g>`,
  };

  // ------------------------------------------------------------------ extras

  const drop = (x, y, i) =>
    `<path class="drop" style="--i:${i}" d="M${x} ${y - 7} C${x + 3} ${y - 2} ${x + 5} ${y + 1} ${x + 5} ${y + 4} A5 5 0 1 1 ${x - 5} ${y + 4} C${x - 5} ${y + 1} ${x - 3} ${y - 2} ${x} ${y - 7} Z" fill="${WATER}"/>`;
  const zee = (x, y, s, i) => line(`M${x} ${y} h${s} l${-s} ${s * 1.15} h${s}`, 2.4 + s / 8, "#8f8f8f", `class="zee" style="--i:${i}"`);
  const heartAt = (x, y, cls) =>
    `<path class="${cls}" d="M${x} ${y + 7} C${x - 9} ${y + 1} ${x - 11} ${y - 4} ${x - 7} ${y - 7} C${x - 4} ${y - 9} ${x - 1} ${y - 8} ${x} ${y - 5} C${x + 1} ${y - 8} ${x + 4} ${y - 9} ${x + 7} ${y - 7} C${x + 11} ${y - 4} ${x + 9} ${y + 1} ${x} ${y + 7} Z" fill="${RED}"/>`;

  const EXTRAS = {
    sweat: `<g class="sweat">${drop(118, 58, 0)}${drop(186, 56, 1)}${drop(96, 78, 2)}${drop(210, 76, 3)}${drop(152, 50, 4)}</g>`,
    zzz: `<g class="zzz">${zee(230, 60, 9, 0)}${zee(248, 40, 12, 1)}${zee(268, 14, 15, 2)}</g>`,
    anger: `<g class="anger">${line("M232 42 q6 1 7 -6 M246 36 q1 6 7 6 M253 51 q-6 0 -7 7 M239 57 q0 -6 -7 -6", 3.6, RED)}</g>`,
    shades: `<g class="shades">
        <path d="M48 110 Q50 100 62 100 L116 100 Q131 100 131 114 Q129 158 90 164 Q54 160 48 122 Z" fill="#111" stroke="${INK}" stroke-width="5" stroke-linejoin="round"/>
        <path d="M256 110 Q254 100 242 100 L188 100 Q173 100 173 114 Q175 158 214 164 Q250 160 256 122 Z" fill="#111" stroke="${INK}" stroke-width="5" stroke-linejoin="round"/>
        ${line("M131 110 Q152 102 173 110", 6)}
        ${line("M72 146 L94 112 M88 148 L105 122", 6, "#fff")}
        ${line("M196 146 L218 112 M212 148 L229 122", 6, "#fff")}
      </g>`,
    kissed: heartAt(182, 146, "float-heart"),
    none: "",
  };

  // ------------------------------------------------------------------ moods

  const MOODS = {
    look: { eyes: "open", brows: "none", mouth: "grin", extra: "none" },
    proud: { eyes: "hearts", brows: "none", mouth: "uwu", extra: "none" },
    fine: { eyes: "open", brows: "none", mouth: "grin", extra: "none" },
    hmm: { eyes: "open", brows: "hmm", mouth: "smile", extra: "none" },
    unamused: { eyes: "flat", brows: "none", mouth: "flat", extra: "none" },
    stern: { eyes: "open", brows: "stern", mouth: "flat", extra: "none" },
    furious: { eyes: "open", brows: "cross", mouth: "yell", extra: "anger" },
    melting: { eyes: "open", brows: "worried", mouth: "wobble", extra: "sweat" },
    asleep: { eyes: "shut", brows: "none", mouth: "small", extra: "zzz" },
    shocked: { eyes: "open", brows: "up", mouth: "o", extra: "none" },
    kiss: { eyes: "happy", brows: "none", mouth: "kiss", extra: "kissed" },
    wink: { eyes: "wink", brows: "none", mouth: "smile", extra: "none" },
    cool: { eyes: "open", brows: "none", mouth: "smile", extra: "shades" },
    dizzy: { eyes: "cross", brows: "none", mouth: "wobble", extra: "none" },
    delighted: { eyes: "happy", brows: "none", mouth: "open", extra: "none" },
    giggle: { eyes: "happy", brows: "none", mouth: "uwu", extra: "none" },
  };

  function expression(mood) {
    const m = MOODS[mood] || MOODS.look;
    return EYES[m.eyes] + BROWS[m.brows] + `<g transform="translate(0 5)">${MOUTHS[m.mouth]}</g>` + EXTRAS[m.extra];
  }

  const DEFS = `<defs>
      <radialGradient id="mom-iris" cx="42%" cy="38%" r="68%">
        <stop offset="0" stop-color="#5a5a5a"/><stop offset=".55" stop-color="#262626"/><stop offset="1" stop-color="#050505"/>
      </radialGradient>
    </defs>`;

  /** Mom as one finished SVG: the widget's face, and the source of the app icon.
      Layers, outside in: fx (mood shakes, in CSS), squash (boops), breath, then hair and bun,
      and `turn`, the face that follows your pointer. */
  function svg(mood = "look", { label } = {}) {
    return `<svg class="mom mood-${mood}" viewBox="0 -12 304 216" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="${label || `Mom looks ${mood}`}">
      ${DEFS}
      <g class="fx"><g class="squash"><g class="breath">${BUN}${HAIRDO}<g class="turn"><g transform="translate(0 4)">${CHEEKS}${GLASSES}<g class="expr">${expression(mood)}</g></g></g></g></g></g>
    </svg>`;
  }

  // ------------------------------------------------------------------ a live mom

  const still = () => window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  function mount(host) {
    host.innerHTML = svg("look");
    const G = window.gsap;
    const root = host.querySelector("svg");
    const $ = (sel) => root.querySelector(sel);
    const $$ = (sel) => root.querySelectorAll(sel);
    const expr = $(".expr");
    let mood = "look";
    let gaze = { x: 0, y: 0 };
    let lastPointer = 0;
    if (G && window.MorphSVGPlugin) G.registerPlugin(window.MorphSVGPlugin);

    const set = (next) => {
      if (next === mood || !MOODS[next]) return;
      const oldMouth = expr.querySelector(".mouth-line");
      const from = oldMouth && oldMouth.getAttribute("d");
      mood = next;
      root.setAttribute("class", `mom mood-${next}`);
      root.setAttribute("aria-label", `Mom looks ${next}`);
      expr.innerHTML = expression(next);
      if (!G || still()) return;
      G.set(expr.querySelectorAll(".look"), { x: gaze.x * 8, y: gaze.y * 6 });
      const mouth = expr.querySelector(".mouth-line");
      if (mouth && from && window.MorphSVGPlugin) {
        const to = mouth.getAttribute("d");
        mouth.setAttribute("d", from);
        G.to(mouth, { morphSVG: to, duration: 0.42, ease: "back.out(1.7)" });
      }
      G.from(expr.querySelectorAll(".eye, .brows, .mouth-fill"), {
        scale: 0.55,
        opacity: 0,
        transformOrigin: "50% 50%",
        duration: 0.34,
        ease: "back.out(2.2)",
        stagger: 0.03,
      });
      G.fromTo($(".squash"), { scaleX: 1.04, scaleY: 0.95 }, { scaleX: 1, scaleY: 1, transformOrigin: "50% 100%", duration: 0.7, ease: "elastic.out(1, 0.45)" });
    };

    if (!G) return { set, boop() {}, hover() {}, get mood() { return mood; } };

    // Breathing, and a bun that sways a beat behind.
    const idle = [
      G.to($(".breath"), { y: -1.8, scaleY: 0.993, transformOrigin: "50% 100%", duration: 2.4, ease: "sine.inOut", yoyo: true, repeat: -1 }),
      G.to($(".bun"), { rotation: -3.5, transformOrigin: "50% 100%", duration: 2.4, ease: "sine.inOut", yoyo: true, repeat: -1, delay: 0.35 }),
    ];

    // Where she looks. Her eyes move most, her face a little, her hair barely: it reads as a head turning.
    const turnX = G.quickTo($(".turn"), "x", { duration: 0.7, ease: "power3" });
    const turnY = G.quickTo($(".turn"), "y", { duration: 0.7, ease: "power3" });
    const tilt = G.quickTo($(".turn"), "rotation", { duration: 0.9, ease: "power3" });
    const hairX = G.quickTo($(".hair"), "x", { duration: 0.9, ease: "power3" });
    const bunX = G.quickTo($(".bun"), "x", { duration: 1.1, ease: "power3" });
    G.set($(".turn"), { transformOrigin: "50% 60%" });

    function lookAt(x, y) {
      gaze = { x: clamp(x, -1, 1), y: clamp(y, -1, 1) };
      turnX(gaze.x * 4);
      turnY(gaze.y * 2.5);
      tilt(gaze.x * 2.5);
      hairX(gaze.x * 1.4);
      bunX(-gaze.x * 1.8);
      G.to(expr.querySelectorAll(".look"), { x: gaze.x * 8, y: gaze.y * 6, duration: 0.45, ease: "power3", overwrite: "auto" });
    }

    let frame = 0;
    document.addEventListener("pointermove", (e) => {
      lastPointer = Date.now();
      if (still() || frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const box = root.getBoundingClientRect();
        const cx = box.left + box.width / 2;
        const cy = box.top + box.height * 0.62;
        lookAt((e.clientX - cx) / 240, (e.clientY - cy) / 200);
      });
    });
    document.documentElement.addEventListener("mouseleave", () => G.delayedCall(0.6, () => lookAt(0, 0)));

    // When nobody's moving the pointer she glances around now and then.
    (function wander() {
      if (!still() && !document.hidden && Date.now() - lastPointer > 5000) {
        const spots = [[0, 0], [0, 0], [-0.7, 0.1], [0.7, 0.1], [-0.4, -0.5], [0.5, 0.4], [0, 0.6]];
        const [x, y] = spots[Math.floor(Math.random() * spots.length)];
        lookAt(x, y);
      }
      G.delayedCall(2 + Math.random() * 3, wander);
    })();

    (function blink() {
      const eyes = expr.querySelectorAll(".eye.open");
      if (eyes.length && !still() && !document.hidden) {
        const t = G.timeline();
        t.to(eyes, { scaleY: 0.08, transformOrigin: "50% 50%", duration: 0.07, ease: "power2.in" }).to(eyes, { scaleY: 1, duration: 0.12, ease: "power2.out" });
        if (Math.random() < 0.18) t.to(eyes, { scaleY: 0.08, duration: 0.07, ease: "power2.in" }, "+=0.08").to(eyes, { scaleY: 1, duration: 0.12 });
      }
      G.delayedCall(2.4 + Math.random() * 4.2, blink);
    })();

    // Now and then the light catches her glasses.
    (function glint() {
      if (!still() && !document.hidden) {
        G.fromTo($$(".glint"), { opacity: 0 }, { opacity: 0.9, duration: 0.18, yoyo: true, repeat: 1, ease: "sine.inOut", stagger: 0.06 });
      }
      G.delayedCall(7 + Math.random() * 9, glint);
    })();

    /** You poked her: a squish, a little hop, and pink cheeks. */
    function boop() {
      if (still()) return;
      G.timeline()
        .to($(".squash"), { scaleX: 1.08, scaleY: 0.9, transformOrigin: "50% 100%", duration: 0.09, ease: "power2.out" })
        .to($(".squash"), { scaleX: 1, scaleY: 1, duration: 0.8, ease: "elastic.out(1.1, 0.35)" });
      G.fromTo($$(".blush"), { scale: 1 }, { scale: 1.35, transformOrigin: "50% 50%", duration: 0.25, yoyo: true, repeat: 1, ease: "power2.out" });
    }

    /** The pointer is on her: she perks up and blushes. */
    function hover(on) {
      if (still()) return;
      G.to($$(".blush"), { scale: on ? 1.18 : 1, opacity: on ? 1 : 0.95, transformOrigin: "50% 50%", duration: 0.35, ease: "power2.out" });
      G.to(idle[0], { timeScale: on ? 1.6 : 1, duration: 0.3 });
    }

    document.addEventListener("visibilitychange", () => idle.forEach((t) => (document.hidden ? t.pause() : t.resume())));
    return { set, boop, hover, get mood() { return mood; } };
  }

  window.momFace = { svg, mount, moods: Object.keys(MOODS) };
})();

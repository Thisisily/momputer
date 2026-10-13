// The stand-ins for BIG MODE's change of size (see make_card in the app). The app says where the
// window is and where it's going. The shape it had shrinks a little and fades; the shape it's
// getting zooms in and fades in, the way a window opens on a well-made desktop. Both are moved by
// transform and opacity alone, which the graphics card does by itself at the screen's own rate.
(() => {
  "use strict";

  const was = document.getElementById("was");
  const will = document.getElementById("will");
  let runs = [];

  const put = (el, r, bg) => {
    el.style.left = `${r.x}px`;
    el.style.top = `${r.y}px`;
    el.style.width = `${r.w}px`;
    el.style.height = `${r.h}px`;
    el.style.background = bg;
  };
  const stop = () => {
    for (const run of runs) run.cancel();
    runs = [];
  };

  window.glide = {
    /** The old shape exactly where the window is now (`a`); the new one waiting, unseen, at `b`. */
    at(a, b, look) {
      stop();
      const bg = look.bg || "#ececec";
      put(was, a, bg);
      put(will, b, bg);
      was.style.opacity = "1";
      was.style.transform = "none";
      will.style.opacity = "0";
    },
    /** Say so once it's on the glass: two frames from now it has been drawn. */
    ack() {
      requestAnimationFrame(() => requestAnimationFrame(() => window.__TAURI__.core.invoke("glide_ready").catch(() => {})));
    },
    /** Go: the old shape closes, the new one opens. `ms` is the whole of it. */
    go(ms) {
      stop();
      const close = { duration: ms * 0.5, easing: "cubic-bezier(0.3, 0, 0.8, 0.15)", fill: "forwards" };
      const open = { duration: ms * 0.78, delay: ms * 0.22, easing: "cubic-bezier(0.05, 0.7, 0.1, 1)", fill: "both" };
      runs = [
        was.animate([{ transform: "scale(1)", opacity: 1 }, { transform: "scale(0.955)", opacity: 0 }], close),
        will.animate([{ transform: "scale(0.93)" }, { transform: "scale(1)" }], open),
        will.animate([{ opacity: 0 }, { opacity: 1 }], { ...open, duration: ms * 0.4, easing: "ease-out" }),
      ];
    },
    /** The real window is back underneath: dissolve into it. */
    away(ms) {
      runs.push(will.animate([{ opacity: 1 }, { opacity: 0 }], { duration: ms, easing: "ease-out", fill: "forwards" }));
    },
    done() {
      stop();
      was.style.opacity = "0";
      will.style.opacity = "0";
    },
  };
})();

// The top bar: a dot under the part of the page you're reading, and a Download button once the
// big one has scrolled out of sight. The page reads the same without any of this.
(() => {
  "use strict";

  const top = document.querySelector(".top");
  const links = [...document.querySelectorAll(".nav a[href^='#']")];
  const parts = links.map((a) => document.getElementById(a.hash.slice(1)));

  // The part being read is the last one whose top has come up past the upper third of the window.
  let asked = false;
  const look = () => {
    asked = false;
    const line = innerHeight * 0.35;
    let now = -1;
    parts.forEach((part, i) => {
      if (part && part.getBoundingClientRect().top <= line) now = i;
    });
    links.forEach((a, i) => {
      if (i === now) a.setAttribute("aria-current", "true");
      else a.removeAttribute("aria-current");
    });
    if (top) top.classList.toggle("scrolled", scrollY > 8);
  };
  const ask = () => {
    if (asked) return;
    asked = true;
    requestAnimationFrame(look);
  };
  addEventListener("scroll", ask, { passive: true });
  addEventListener("resize", ask, { passive: true });
  look();

  // The small Download button in the bar, for Windows PCs, while the big one is out of sight.
  const big = document.getElementById("dl");
  const small = document.getElementById("bar-dl");
  const note = document.getElementById("not-windows");
  if (big && small && note && note.hidden && "IntersectionObserver" in window) {
    new IntersectionObserver(([seen]) => {
      small.hidden = seen.isIntersecting;
    }).observe(big);
  }
})();

// Draws momputer's icons as SVG from mom.js, the same drawing the widget animates.
//
//   node scripts/icons.js
//   cd app/src-tauri && cargo tauri icon ../icons-src/icon.svg -o icons
//
// The tray icons (tray.svg, tray-alert.svg) are mom in a few bold strokes, because the tray is
// only 16 to 32 pixels. Render each with `cargo tauri icon` and keep its 64x64.png as
// app/src-tauri/icons/tray.png and tray-alert.png.
"use strict";
const fs = require("fs");
const path = require("path");

global.window = {};
require("../app/ui/mom.js");

const out = path.join(__dirname, "..", "app", "icons-src");
fs.mkdirSync(out, { recursive: true });

// The app icon: a black tile, mom on her round white screen, and a tiny red light.
const face = window.momFace
  .svg("look")
  .replace(/^<svg class="[^"]*"/, '<svg x="64" y="206" width="896" height="637"')
  .replace(/ role="img" aria-label="[^"]*"/, "");
const icon = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <defs>
    <clipPath id="screen"><circle cx="512" cy="524" r="400"/></clipPath>
    <filter id="glow" x="-1" y="-1" width="3" height="3"><feGaussianBlur stdDeviation="14"/></filter>
  </defs>
  <rect width="1024" height="1024" rx="230" fill="#000"/>
  <circle cx="512" cy="524" r="400" fill="#fff"/>
  <g clip-path="url(#screen)">${face}</g>
  <circle cx="876" cy="148" r="44" fill="#ff3b3b" opacity=".55" filter="url(#glow)"/>
  <circle cx="876" cy="148" r="34" fill="#ff3b3b"/>
</svg>
`;
fs.writeFileSync(path.join(out, "icon.svg"), icon);

// The tray: bun, head, a grey cap of hair, giant glasses, and a light that says how things are.
function tray(alert) {
  const light = alert
    ? '<circle cx="206" cy="206" r="50" fill="#000"/><circle cx="206" cy="206" r="38" fill="#ff3b3b"/>'
    : '<circle cx="214" cy="214" r="34" fill="#000"/><circle cx="214" cy="214" r="22" fill="#2fe07a"/>';
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs><clipPath id="cap"><rect width="256" height="112"/></clipPath></defs>
  <circle cx="128" cy="38" r="30" fill="#fff" stroke="#000" stroke-width="15"/>
  <circle cx="128" cy="146" r="106" fill="#fff" stroke="#000" stroke-width="15"/>
  <circle cx="128" cy="146" r="91" fill="#d4d4d4" clip-path="url(#cap)"/>
  <circle cx="82" cy="150" r="42" fill="#fff" stroke="#000" stroke-width="15"/>
  <circle cx="174" cy="150" r="42" fill="#fff" stroke="#000" stroke-width="15"/>
  <circle cx="82" cy="152" r="11"/>
  <circle cx="174" cy="152" r="11"/>
  <path d="M115 145 Q128 135 141 145" fill="none" stroke="#000" stroke-width="15" stroke-linecap="round"/>
  <path d="M114 211 Q128 224 142 211" fill="none" stroke="#000" stroke-width="11" stroke-linecap="round"/>
  ${light}
</svg>
`;
}
fs.writeFileSync(path.join(out, "tray.svg"), tray(false));
fs.writeFileSync(path.join(out, "tray-alert.svg"), tray(true));
console.log("wrote app/icons-src/icon.svg, tray.svg, tray-alert.svg");

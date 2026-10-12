// Copies the widget's browser libraries out of node_modules into app/ui/vendor.
//   npm install && npm run vendor
"use strict";
const fs = require("fs");
const path = require("path");

const root = path.join(__dirname, "..");
const out = path.join(root, "app", "ui", "vendor");
const files = {
  "gsap.min.js": "node_modules/gsap/dist/gsap.min.js",
  "MorphSVGPlugin.min.js": "node_modules/gsap/dist/MorphSVGPlugin.min.js",
  "idiomorph.min.js": "node_modules/idiomorph/dist/idiomorph.min.js",
  // three.js (MIT), only for mom's room; loaded when someone gets in.
  "three.module.min.js": "node_modules/three/build/three.module.min.js",
  // Fonts (SIL Open Font License): Geist for words, Geist Mono for paths and commands,
  // Doto, a dot-matrix face, for numbers and mom's name.
  "fonts/geist.woff2": "node_modules/@fontsource-variable/geist/files/geist-latin-wght-normal.woff2",
  "fonts/geist-mono.woff2": "node_modules/@fontsource-variable/geist-mono/files/geist-mono-latin-wght-normal.woff2",
  "fonts/doto.woff2": "node_modules/@fontsource-variable/doto/files/doto-latin-full-normal.woff2",
};
fs.mkdirSync(path.join(out, "fonts"), { recursive: true });
for (const [name, from] of Object.entries(files)) {
  fs.copyFileSync(path.join(root, from), path.join(out, name));
  console.log(`app/ui/vendor/${name}`);
}

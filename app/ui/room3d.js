/* Mom's room, an easter egg. Knock on her door (add her own folder as a room) and she lets you in
   to tidy up: six little chores in a little 3D room. Books on the shelf, the bed made, coffee on,
   laundry in the basket, her picture on the wall, the curtains open. About a minute.

   play(host, on) draws the room into `host` and returns { act(chore), close() }. Tap things in the
   room, or call act() (the checklist buttons do, for the keyboard). on.chore(id, done, of) after
   each step and on.done() at the end. Drawn with three.js, moved with GSAP. */
import * as THREE from "./vendor/three.module.min.js";

const G = window.gsap;
const calm = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const secs = (s) => (calm() ? 0.01 : s);

export const CHORES = [
  { id: "books", label: "Books", of: 6, hint: "Tap the books to put them on the shelf." },
  { id: "bed", label: "Bed", of: 1, hint: "Tap the bed to make it." },
  { id: "coffee", label: "Coffee", of: 1, hint: "Tap the coffee machine." },
  { id: "laundry", label: "Laundry", of: 3, hint: "Tap the clothes to drop them in the basket." },
  { id: "picture", label: "Picture", of: 1, hint: "Tap mom's picture to hang it up." },
  { id: "curtains", label: "Curtains", of: 1, hint: "Tap the curtains to let the light in." },
];

const C = {
  floor: 0xc79f78,
  board: 0xb48c66,
  wall: 0xf3ede4,
  wallSide: 0xe7dfd3,
  trim: 0xfbf8f2,
  wood: 0x8b5e3c,
  woodLight: 0xb98457,
  woodDark: 0x6a4630,
  white: 0xfbf8f3,
  sheet: 0xf0ebe3,
  blanket: 0xd64b3c,
  curtain: 0x5f7f9a,
  rug: 0x32363b,
  rugStripe: 0xd71921,
  ink: 0x1d1d1f,
  glass: 0xbfe3ff,
  sky: 0xa9d8ff,
  coffee: 0x4a2a17,
  wicker: 0xcfb48a,
  lamp: 0xffe2a8,
  plant: 0x4f8a5b,
  pot: 0xd8d2c8,
};
const BOOKS = [0xd71921, 0x2f5d9b, 0xe0b23a, 0x2f7d5b, 0x6b4ea0, 0x1d1d1f];

export function play(host, on = {}) {
  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  } catch {
    return null; // no 3D on this machine
  }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  host.appendChild(renderer.domElement);

  const scene = new THREE.Scene();
  const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 100);
  camera.position.set(9, 8.4, 9);
  camera.lookAt(0, 0.95, 0);

  const owned = []; // geometries, materials and textures to free on close
  const tweens = [];
  const tween = (target, vars) => {
    const t = G ? G.to(target, { ...vars, duration: secs(vars.duration ?? 0.4) }) : null;
    if (!G) Object.assign(target, vars);
    if (t) tweens.push(t);
    return t;
  };
  const timeline = () => {
    const t = G.timeline();
    tweens.push(t);
    return t;
  };

  // ---------------------------------------------------------------- light

  const hemi = new THREE.HemisphereLight(0xfff4e6, 0x9a7d63, 1.25);
  const fill = new THREE.DirectionalLight(0xffffff, 0.5);
  fill.position.set(7, 8, 9);
  // The sun, outside the window: once the curtains open, it falls through onto the floor.
  const sun = new THREE.DirectionalLight(0xffdca6, 3);
  sun.position.set(-7.5, 4.9, -1.15);
  sun.target.position.set(0.3, 0, -0.85);
  sun.castShadow = true;
  sun.shadow.mapSize.set(1024, 1024);
  Object.assign(sun.shadow.camera, { left: -5, right: 5, top: 5, bottom: -5, near: 1, far: 22 });
  sun.shadow.bias = -0.0006;
  sun.shadow.normalBias = 0.02;
  scene.add(hemi, fill, sun, sun.target);

  // ---------------------------------------------------------------- building blocks

  const material = (color, extra = {}) => {
    const m = new THREE.MeshStandardMaterial({ color, roughness: 0.85, metalness: 0, flatShading: true, ...extra });
    owned.push(m);
    return m;
  };
  const mesh = (geometry, color, extra) => {
    owned.push(geometry);
    const m = new THREE.Mesh(geometry, material(color, extra));
    m.castShadow = true;
    m.receiveShadow = true;
    return m;
  };
  /** A box sitting on `y` (its bottom), centred on x and z, added to `parent`. */
  const box = (w, h, d, color, x, y, z, parent = scene, extra) => {
    const m = mesh(new THREE.BoxGeometry(w, h, d), color, extra);
    m.position.set(x, y + h / 2, z);
    parent.add(m);
    return m;
  };
  const cylinder = (rTop, rBottom, h, color, x, y, z, parent = scene, extra, open = false) => {
    const m = mesh(new THREE.CylinderGeometry(rTop, rBottom, h, 18, 1, open), color, extra);
    m.position.set(x, y + h / 2, z);
    parent.add(m);
    return m;
  };
  const group = (x, y, z, parent = scene) => {
    const g = new THREE.Group();
    g.position.set(x, y, z);
    parent.add(g);
    return g;
  };

  // ---------------------------------------------------------------- the room

  const S = 2.5; // half the floor
  const H = 2.8; // walls
  const T = 0.12; // wall thickness
  box(2 * S, 0.12, 2 * S, C.floor, 0, -0.12, 0);
  for (let i = 1; i < 10; i++) box(0.015, 0.004, 2 * S, C.board, -S + i * 0.5, 0, 0);
  box(2 * S + T, H, T, C.wall, T / 2 - 0.0, 0, -S - T / 2); // back wall
  box(2 * S, 0.12, 0.03, C.trim, 0, 0, -S + 0.015); // skirting
  // Left wall, around the window.
  const win = { z0: -1.65, z1: -0.45, y0: 1.15, y1: 2.25 };
  const wz = (win.z0 + win.z1) / 2;
  const ww = win.z1 - win.z0;
  box(T, H, win.z0 + S, C.wallSide, -S - T / 2, 0, (-S + win.z0) / 2);
  box(T, H, S - win.z1, C.wallSide, -S - T / 2, 0, (win.z1 + S) / 2);
  box(T, win.y0, ww, C.wallSide, -S - T / 2, 0, wz);
  box(T, H - win.y1, ww, C.wallSide, -S - T / 2, win.y1, wz);
  box(0.03, 0.12, 2 * S, C.trim, -S + 0.015, 0, 0);
  const glass = box(0.02, win.y1 - win.y0, ww, C.glass, -S - T / 2, win.y0, wz, scene, {
    transparent: true,
    opacity: 0.3,
    emissive: 0x7fbfff,
    emissiveIntensity: 0.15,
  });
  glass.castShadow = false;
  box(0.05, win.y1 - win.y0, 0.05, C.trim, -S + 0.0, win.y0, wz);
  box(0.05, 0.05, ww, C.trim, -S + 0.0, (win.y0 + win.y1) / 2, wz);
  box(0.28, 0.05, ww + 0.2, C.trim, -S + 0.08, win.y0 - 0.05, wz);
  // Sky, just behind the window: the wall hides it everywhere else.
  const sky = box(0.02, win.y1 - win.y0 + 0.16, ww + 0.16, C.sky, -S - T - 0.03, win.y0 - 0.08, wz, scene, { emissive: C.sky, emissiveIntensity: 0.6 });
  sky.castShadow = false;
  // Rug.
  box(2.5, 0.012, 1.8, C.rug, 0.9, 0, 0.65);
  box(2.5, 0.014, 0.08, C.rugStripe, 0.9, 0, -0.1);
  box(2.5, 0.014, 0.08, C.rugStripe, 0.9, 0, 1.4);

  // Bed, in the back corner under the window.
  box(1.6, 0.3, 2.6, C.wood, -1.7, 0, -1.2);
  box(1.6, 0.95, 0.1, C.wood, -1.7, 0, -S + 0.05);
  box(1.5, 0.22, 2.45, C.white, -1.7, 0.3, -1.22);
  const pillow = box(1.0, 0.14, 0.42, C.sheet, -1.7, 0.52, -2.12);
  const blanket = box(1.5, 0.07, 1.75, C.blanket, -1.7, 0.52, -0.68);
  const lump = box(0.7, 0.2, 0.62, C.blanket, -1.35, 0.55, -0.25);
  const fold = box(1.5, 0.075, 0.22, C.sheet, -1.7, 0.52, -1.5);
  const home = {
    pillow: { p: pillow.position.clone(), r: pillow.rotation.clone() },
    blanket: { p: blanket.position.clone(), r: blanket.rotation.clone(), s: blanket.scale.clone() },
  };
  pillow.position.set(-0.55, 0.07, -0.75);
  pillow.rotation.set(0.15, 0.7, 0.35);
  blanket.position.set(-1.55, 0.62, -0.75);
  blanket.rotation.set(0.18, 0.42, -0.14);
  blanket.scale.set(0.82, 2.6, 0.72);
  fold.scale.set(0.001, 1, 1);
  // Bedside table and lamp.
  box(0.45, 0.5, 0.4, C.woodLight, -0.6, 0, -2.2);
  cylinder(0.06, 0.09, 0.06, C.ink, -0.6, 0.5, -2.2);
  cylinder(0.015, 0.015, 0.3, C.ink, -0.6, 0.56, -2.2);
  cylinder(0.12, 0.18, 0.2, C.lamp, -0.6, 0.82, -2.2, scene, { emissive: C.lamp, emissiveIntensity: 0.5 });

  // Bookshelf on the back wall: two shelves have gaps for the books on the floor.
  const shelf = group(0.7, 0, -S + 0.2);
  box(0.06, 2.0, 0.38, C.wood, -0.67, 0, 0, shelf);
  box(0.06, 2.0, 0.38, C.wood, 0.67, 0, 0, shelf);
  box(1.4, 2.0, 0.03, C.woodDark, 0, 0, -0.175, shelf);
  for (const y of [0, 0.5, 1.0, 1.5, 1.96]) box(1.34, 0.04, 0.36, C.wood, 0, y, 0, shelf);
  const deco = [
    [1.54, -0.55, 0.34, 0x8a8f99],
    [1.54, -0.43, 0.3, 0xc9b28d],
    [1.54, -0.31, 0.36, 0x2f5d9b],
    [1.54, -0.19, 0.28, 0xd71921],
    [0.04, 0.2, 0.36, 0x1d1d1f],
    [0.04, 0.32, 0.3, 0xe0b23a],
    [0.04, 0.44, 0.34, 0x6b4ea0],
    [0.54, 0.25, 0.3, 0xc9b28d],
    [0.54, 0.37, 0.26, 0x8a8f99],
    [1.04, -0.5, 0.32, 0x2f7d5b],
  ];
  for (const [y, x, h, c] of deco) box(0.1, h, 0.26, c, x, y, 0.02, shelf);
  cylinder(0.1, 0.08, 0.16, C.pot, 0.35, 1.54, 0.02, shelf);
  for (let i = 0; i < 5; i++) {
    const leaf = box(0.05, 0.22, 0.02, C.plant, 0.35, 1.68, 0.02, shelf);
    leaf.rotation.set(0, (i * Math.PI) / 2.5, 0.5);
    leaf.castShadow = false;
  }
  box(0.4, 0.22, 0.26, C.white, 0.4, 0.04, 0.02, shelf);
  shelf.updateMatrixWorld(true);
  const slots = [
    [0.54, -0.5],
    [0.54, -0.37],
    [0.54, -0.24],
    [1.04, 0.05],
    [1.04, 0.18],
    [1.04, 0.31],
  ].map(([y, x]) => shelf.localToWorld(new THREE.Vector3(x, y + 0.18, 0.02)));

  // Books on the floor.
  const floorBooks = [
    [0.35, -0.55, 0.4],
    [0.95, -0.15, -0.6],
    [1.55, 0.45, 1.1],
    [0.25, 0.55, 2.2],
    [1.15, 1.6, -1.2],
    [2.0, 0.95, 0.3],
  ];
  const books = floorBooks.map(([x, z, yaw], i) => {
    const b = box(0.12, 0.36, 0.26, BOOKS[i], x, 0, z);
    b.rotation.set(0, yaw, Math.PI / 2);
    b.position.y = 0.06;
    return b;
  });

  // Coffee corner.
  const counter = { x: 2.05, z: -S + 0.3, y: 0.85 };
  box(0.9, 0.06, 0.56, C.woodLight, counter.x, counter.y - 0.06, counter.z);
  for (const [dx, dz] of [[-0.4, -0.23], [0.4, -0.23], [-0.4, 0.23], [0.4, 0.23]]) box(0.05, counter.y - 0.06, 0.05, C.woodDark, counter.x + dx, 0, counter.z + dz);
  const machine = group(2.28, counter.y, counter.z - 0.04);
  box(0.3, 0.44, 0.3, C.ink, 0, 0, 0, machine);
  box(0.32, 0.06, 0.34, 0x2c2c2e, 0, 0.44, 0.01, machine);
  box(0.24, 0.03, 0.12, 0x3a3a3c, 0, 0, 0.19, machine);
  const spout = cylinder(0.025, 0.02, 0.06, 0x8e8e93, 0, 0.3, 0.2, machine);
  const led = mesh(new THREE.SphereGeometry(0.022, 10, 8), C.rugStripe, { emissive: C.rugStripe, emissiveIntensity: 0.9 });
  led.position.set(0.09, 0.38, 0.152);
  machine.add(led);
  const stream = cylinder(0.012, 0.012, 0.16, C.coffee, 0, 0.13, 0.2, machine);
  stream.scale.set(1, 0.001, 1);
  stream.visible = false;
  const mug = group(1.84, counter.y, counter.z + 0.08);
  cylinder(0.075, 0.068, 0.15, C.white, 0, 0, 0, mug, { side: THREE.DoubleSide }, true);
  cylinder(0.068, 0.068, 0.01, C.white, 0, 0, 0, mug);
  const handle = mesh(new THREE.TorusGeometry(0.045, 0.012, 8, 16, Math.PI), C.white);
  handle.position.set(0.075, 0.075, 0);
  handle.rotation.z = -Math.PI / 2;
  mug.add(handle);
  const brew = cylinder(0.066, 0.066, 0.004, C.coffee, 0, 0.012, 0, mug);
  brew.visible = false;
  const steam = [0, 1, 2].map((i) => {
    const puff = mesh(new THREE.SphereGeometry(0.035, 10, 8), 0xffffff, { transparent: true, opacity: 0 });
    puff.castShadow = false;
    puff.position.set((i - 1) * 0.03, 0.2, 0);
    mug.add(puff);
    return puff;
  });

  // Window curtains.
  const rod = cylinder(0.015, 0.015, ww + 0.6, C.ink, -S + 0.1, 0, wz);
  rod.rotation.set(Math.PI / 2, 0, 0);
  rod.position.y = win.y1 + 0.14;
  const curtainW = ww / 2 + 0.1;
  const curtainL = box(0.04, 1.5, curtainW, C.curtain, -S + 0.1, 0.95, win.z0 - 0.1 + curtainW / 2);
  const curtainR = box(0.04, 1.5, curtainW, C.curtain, -S + 0.1, 0.95, win.z1 + 0.1 - curtainW / 2);

  // Laundry basket, and what didn't make it in.
  const basket = { x: -0.25, z: 1.65 };
  cylinder(0.3, 0.25, 0.46, C.wicker, basket.x, 0, basket.z, scene, { side: THREE.DoubleSide }, true);
  cylinder(0.25, 0.25, 0.02, 0x9c8463, basket.x, 0, basket.z);
  const shirt = group(1.75, 0.02, -0.45);
  box(0.42, 0.04, 0.48, 0x2f8f8a, 0, 0, 0, shirt);
  for (const s of [-1, 1]) box(0.22, 0.035, 0.14, 0x2f8f8a, s * 0.28, 0, -0.14, shirt).rotation.set(0, s * 0.5, 0);
  shirt.rotation.y = 0.5;
  shirt.userData.span = 0.98; // across the sleeves
  const sock1 = group(1.9, 0.02, 1.35);
  box(0.08, 0.05, 0.26, C.white, 0, 0, 0, sock1);
  box(0.082, 0.052, 0.05, C.rugStripe, 0, 0, -0.1, sock1);
  sock1.rotation.y = -0.7;
  sock1.userData.span = 0.27;
  const sock2 = group(0.45, 0.02, 2.05);
  box(0.08, 0.05, 0.26, C.white, 0, 0, 0, sock2);
  box(0.082, 0.052, 0.05, C.rugStripe, 0, 0, -0.1, sock2);
  sock2.rotation.y = 1.2;
  sock2.userData.span = 0.27;
  const laundry = [shirt, sock1, sock2];

  // Mom's picture, on the floor until it's hung over the bed.
  const picture = group(1.25, 0.03, 1.05);
  box(0.66, 0.82, 0.04, C.ink, 0, -0.41, 0, picture);
  const face = new THREE.Mesh(new THREE.PlaneGeometry(0.58, 0.74), new THREE.MeshStandardMaterial({ map: portrait(), roughness: 0.9 }));
  owned.push(face.geometry, face.material);
  face.position.set(0, 0, 0.022);
  picture.add(face);
  picture.rotation.set(-Math.PI / 2, 0, 0.4);
  const pictureHome = { p: new THREE.Vector3(-1.7, 1.8, -S + 0.04), r: new THREE.Euler(0, 0, 0) };

  /** Mom's face, from the widget's own drawing, on a cream mat. */
  function portrait() {
    const canvas = document.createElement("canvas");
    canvas.width = 256;
    canvas.height = 330;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#f4eee4";
    ctx.fillRect(0, 0, 256, 330);
    const tex = new THREE.CanvasTexture(canvas);
    tex.colorSpace = THREE.SRGBColorSpace;
    owned.push(tex);
    const faces = window.momFace;
    if (faces) {
      const img = new Image();
      img.onload = () => {
        ctx.drawImage(img, 10, 80, 236, 168);
        tex.needsUpdate = true;
      };
      img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(faces.svg(faces.moods.includes("proud") ? "proud" : "look"))}`;
    }
    return tex;
  }

  // ---------------------------------------------------------------- what can be tapped

  const tappable = [];
  const mark = (root, chore) => {
    root.userData.chore = chore;
    root.traverse((o) => {
      if (o.isMesh) {
        o.userData.root = root;
        tappable.push(o);
      }
    });
  };
  books.forEach((b) => mark(b, "books"));
  laundry.forEach((l) => mark(l, "laundry"));
  [pillow, blanket, lump].forEach((m) => mark(m, "bed"));
  mark(machine, "coffee");
  mark(mug, "coffee");
  mark(picture, "picture");
  [curtainL, curtainR].forEach((c) => mark(c, "curtains"));

  // ---------------------------------------------------------------- chores

  const count = Object.fromEntries(CHORES.map((c) => [c.id, 0]));
  const of = Object.fromEntries(CHORES.map((c) => [c.id, c.of]));
  const busy = new Set();
  let shelved = 0; // books given a slot, in the air or on the shelf
  let finished = false;
  let lastTouch = performance.now();

  const step = (chore) => {
    count[chore]++;
    if (on.chore) on.chore(chore, count[chore], of[chore]);
    if (!finished && CHORES.every((c) => count[c.id] >= c.of)) {
      finished = true;
      if (on.done) setTimeout(() => on.done(), calm() ? 0 : 700);
    }
  };

  /** Lifts `obj` up, carries it to `to` (turning to `rot`), and sets it down. */
  function carry(obj, to, rot, lift = 0.7, d = 0.55) {
    if (!G) {
      obj.position.copy(to);
      obj.rotation.copy(rot);
      return Promise.resolve();
    }
    G.killTweensOf(obj.position); // a hint hop, say
    return new Promise((resolve) => {
      const tl = timeline();
      const peak = Math.max(obj.position.y, to.y) + lift;
      tl.to(obj.position, { y: peak, duration: secs(d * 0.4), ease: "power2.out" })
        .to(obj.position, { x: to.x, z: to.z, duration: secs(d), ease: "power2.inOut" }, "<0.05")
        .to(obj.rotation, { x: rot.x, y: rot.y, z: rot.z, duration: secs(d), ease: "power2.inOut" }, "<")
        .to(obj.position, { y: to.y, duration: secs(d * 0.5), ease: "bounce.out" }, `-=${d * 0.25}`)
        .call(resolve);
    });
  }

  const doing = {
    books(item) {
      const book = item || books.find((b) => !b.userData.done && !busy.has(b));
      if (!book || book.userData.done) return;
      book.userData.done = true;
      busy.add(book);
      const slot = slots[shelved++];
      carry(book, slot, new THREE.Euler(0, 0, 0), 0.9).then(() => {
        busy.delete(book);
        step("books");
      });
    },
    bed() {
      if (count.bed || busy.has("bed")) return;
      busy.add("bed");
      carry(pillow, home.pillow.p, home.pillow.r, 0.6, 0.5);
      tween(blanket.position, { x: home.blanket.p.x, y: home.blanket.p.y, z: home.blanket.p.z, duration: 0.6, ease: "power2.inOut", delay: 0.15 });
      tween(blanket.rotation, { x: 0, y: 0, z: 0, duration: 0.6, ease: "power2.inOut", delay: 0.15 });
      tween(blanket.scale, { x: 1, y: 1, z: 1, duration: 0.6, ease: "power2.inOut", delay: 0.15 });
      tween(lump.scale, { x: 0.001, y: 0.001, z: 0.001, duration: 0.35, ease: "power2.in", delay: 0.1 });
      tween(fold.scale, { x: 1, duration: 0.4, ease: "power2.out", delay: 0.65, onComplete: () => step("bed") });
    },
    coffee() {
      if (count.coffee || busy.has("coffee")) return;
      busy.add("coffee");
      if (!G) {
        brew.visible = true;
        brew.position.y = 0.13;
        return step("coffee");
      }
      const under = new THREE.Vector3(machine.position.x, counter.y, machine.position.z + 0.2);
      const back = mug.position.clone();
      const tl = timeline();
      tl.to(mug.position, { x: under.x, z: under.z, duration: secs(0.45), ease: "power2.inOut" })
        .call(() => {
          led.material.color.setHex(0x2fe07a);
          led.material.emissive.setHex(0x2fe07a);
          stream.visible = true;
          brew.visible = true;
        })
        .to(stream.scale, { y: 1, duration: secs(0.2) })
        .to(brew.position, { y: 0.13, duration: secs(1.0), ease: "none" })
        .to(stream.scale, { y: 0.001, duration: secs(0.15) })
        .call(() => (stream.visible = false))
        .to(mug.position, { x: back.x, z: back.z, duration: secs(0.45), ease: "power2.inOut" })
        .call(() => {
          step("coffee");
          if (calm()) return;
          steam.forEach((puff, i) => {
            tweens.push(
              G.fromTo(
                puff.position,
                { y: 0.18 },
                { y: 0.42, duration: 1.6, repeat: -1, delay: i * 0.5, ease: "sine.out" }
              ),
              G.fromTo(puff.material, { opacity: 0.55 }, { opacity: 0, duration: 1.6, repeat: -1, delay: i * 0.5 }),
              G.fromTo(puff.scale, { x: 0.6, y: 0.6, z: 0.6 }, { x: 1.4, y: 1.4, z: 1.4, duration: 1.6, repeat: -1, delay: i * 0.5 })
            );
          });
        });
    },
    laundry(item) {
      const piece = item || laundry.find((l) => !l.userData.done && !busy.has(l));
      if (!piece || piece.userData.done) return;
      piece.userData.done = true;
      busy.add(piece);
      // Bundled up to fit: no wider than the basket, lying nearly flat, below the rim.
      const n = laundry.filter((l) => l.userData.done).length - 1;
      const fit = Math.min(0.85, 0.34 / (piece.userData.span || 0.34));
      const into = new THREE.Vector3(basket.x + [-0.04, 0.04, 0][n], 0.3 + n * 0.05, basket.z + [0.03, -0.03, 0.01][n]);
      carry(piece, into, new THREE.Euler(0.06, piece.rotation.y + 1.2, 0.04), 1.0, 0.6).then(() => {
        busy.delete(piece);
        step("laundry");
      });
      tween(piece.scale, { x: fit, y: fit, z: fit, duration: 0.5, delay: 0.15 });
    },
    picture() {
      if (count.picture || busy.has("picture")) return;
      busy.add("picture");
      carry(picture, pictureHome.p, pictureHome.r, 1.2, 0.7).then(() => {
        if (G && !calm()) tweens.push(G.fromTo(picture.rotation, { z: 0.12 }, { z: 0, duration: 0.9, ease: "elastic.out(1.2, 0.3)" }));
        step("picture");
      });
    },
    curtains() {
      if (count.curtains || busy.has("curtains")) return;
      busy.add("curtains");
      tween(curtainL.position, { z: win.z0 - 0.32, duration: 0.7, ease: "power2.inOut" });
      tween(curtainL.scale, { z: 0.32, duration: 0.7, ease: "power2.inOut" });
      tween(curtainR.position, { z: win.z1 + 0.32, duration: 0.7, ease: "power2.inOut" });
      tween(curtainR.scale, { z: 0.32, duration: 0.7, ease: "power2.inOut" });
      tween(hemi, { intensity: 1.45, duration: 0.9 });
      tween(glass.material, { emissiveIntensity: 0.6, duration: 0.9, onComplete: () => step("curtains") });
    },
  };

  const waitingFor = (chore) =>
    [...new Set(tappable.map((m) => m.userData.root))].filter(
      (r) => r.userData.chore === chore && !r.userData.done && count[chore] < of[chore] && !busy.has(r) && !busy.has(chore)
    );

  /** Where the next thing for a chore is, in CSS pixels within `host`: for pointing at it. */
  function spot(chore) {
    const root = waitingFor(chore)[0];
    if (!root) return null;
    const v = new THREE.Box3().setFromObject(root).getCenter(new THREE.Vector3()).project(camera);
    return { x: ((v.x + 1) / 2) * host.clientWidth, y: ((1 - v.y) / 2) * host.clientHeight };
  }

  /** Makes everything a chore still needs glow (hovering its checklist button). */
  function highlight(chore, on) {
    for (const root of waitingFor(chore)) glow(root, on);
  }

  /** The next step of a chore (the checklist's buttons), or a tapped thing. */
  function act(chore, item) {
    lastTouch = performance.now();
    if (doing[chore]) doing[chore](item);
  }

  // ---------------------------------------------------------------- pointer

  const ray = new THREE.Raycaster();
  const ndc = new THREE.Vector2();
  let hovered = null;
  const pick = (e) => {
    const r = renderer.domElement.getBoundingClientRect();
    ndc.set(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    ray.setFromCamera(ndc, camera);
    const hit = ray.intersectObjects(tappable, false).find((h) => {
      const root = h.object.userData.root;
      return root && !root.userData.done && !(root.userData.chore && count[root.userData.chore] >= of[root.userData.chore]);
    });
    return hit ? hit.object.userData.root : null;
  };
  const glow = (root, on) =>
    root &&
    root.traverse((o) => {
      if (o.isMesh && o.material.emissive && !o.material.transparent && o !== led) o.material.emissive.setHex(on ? 0x2a2018 : 0x000000);
    });
  const onDown = (e) => {
    const root = pick(e);
    if (!root) return;
    e.preventDefault();
    glow(root, false);
    hovered = null;
    act(root.userData.chore, root.userData.chore === "books" || root.userData.chore === "laundry" ? root : undefined);
  };
  const onMove = (e) => {
    const root = pick(e);
    if (root !== hovered) {
      glow(hovered, false);
      glow(root, true);
      hovered = root;
      renderer.domElement.style.cursor = root ? "pointer" : "";
    }
  };
  renderer.domElement.addEventListener("pointerdown", onDown);
  renderer.domElement.addEventListener("pointermove", onMove);

  // Stuck? Something that still needs doing gives a little hop.
  const hint = setInterval(() => {
    if (finished || !G || calm() || performance.now() - lastTouch < 3500) return;
    const waiting = tappable
      .map((m) => m.userData.root)
      .filter((r) => !r.userData.done && count[r.userData.chore] < of[r.userData.chore] && !busy.has(r) && !busy.has(r.userData.chore));
    const root = waiting[Math.floor(Math.random() * waiting.length)];
    if (root && root !== curtainL && root !== curtainR) tweens.push(G.to(root.position, { y: `+=0.12`, duration: 0.16, yoyo: true, repeat: 1, ease: "power1.out" }));
  }, 2200);

  // ---------------------------------------------------------------- size and drawing

  function fit() {
    const w = host.clientWidth || 1;
    const h = host.clientHeight || 1;
    renderer.setSize(w, h, false);
    renderer.domElement.style.width = "100%";
    renderer.domElement.style.height = "100%";
    const aspect = w / h;
    let vw = 7.9;
    let vh = vw / aspect;
    if (vh < 6.4) {
      vh = 6.4;
      vw = vh * aspect;
    }
    Object.assign(camera, { left: -vw / 2, right: vw / 2, top: vh / 2 + 0.2, bottom: -vh / 2 + 0.2 });
    camera.updateProjectionMatrix();
  }
  const resize = new ResizeObserver(fit);
  resize.observe(host);
  fit();
  renderer.setAnimationLoop(() => renderer.render(scene, camera));
  const pause = () => renderer.setAnimationLoop(document.hidden ? null : () => renderer.render(scene, camera));
  document.addEventListener("visibilitychange", pause);
  if (G && !calm()) {
    camera.zoom = 0.82;
    camera.updateProjectionMatrix();
    tweens.push(G.to(camera, { zoom: 1, duration: 0.9, ease: "power3.out", onUpdate: () => camera.updateProjectionMatrix() }));
  }

  function close() {
    clearInterval(hint);
    resize.disconnect();
    document.removeEventListener("visibilitychange", pause);
    renderer.setAnimationLoop(null);
    for (const t of tweens) t.kill();
    renderer.domElement.removeEventListener("pointerdown", onDown);
    renderer.domElement.removeEventListener("pointermove", onMove);
    for (const o of owned) o.dispose();
    renderer.dispose();
    renderer.domElement.remove();
  }

  return { act, close, spot, highlight, count: () => ({ ...count }) };
}

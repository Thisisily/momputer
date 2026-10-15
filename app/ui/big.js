/* BIG MODE: the widget grown into a full window, with your PC on a desk beside it. A monitor
   with mom on its screen (the same live mom as the widget, laid onto the glass), and a glass-sided
   tower whose parts you can point at and tap: processor, memory, graphics card, drives, the
   motherboard, the monitor itself, the speakers, the keyboard, the mouse, and the power button
   for Windows itself.
   Each part carries a little light: green when it's fine, red when something's wrong. The cooler
   on the processor has a dot display: mom's smile when every light is green, otherwise how many
   parts need a look.

   It's lit like a studio shot: soft boxes reflected in metal and glass, a key light that casts
   shadows, a rim light behind. Pick a part inside the tower and its glass door swings open while
   the view leans in.

   open(host, screenEl, model, on) draws it into `host`, lays `screenEl` over the monitor, and
   returns { update(model), select(id), where(id), close() }. on.pick(id) when a part is tapped
   (null for the empty desk), on.hover(id, x, y) while pointing.

   Like the widget, it costs nothing at rest: there is no animation loop. A frame is drawn when
   something changes (you look around, point at a part, a short tween runs) and at no other
   time; shadows are only recalculated when something in the scene moves. Drawn with three.js,
   moved with GSAP. */
import * as THREE from "./vendor/three.module.min.js";

const G = window.gsap;
const calm = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** The scene's colours for a light or a dark room. */
function palette(dark) {
  return dark
    ? {
        bgIn: "#26262a", bgOut: "#0b0b0c", floor: 0.5, exposure: 1.05,
        desk: 0x18181a, mat: 0x0e0e0f, alu: 0xb8b8bd, anod: 0x232326, caseBody: 0x1b1b1d, caseEdge: 0x2c2c30,
        pcb: "#0d0d0f", trace: "rgba(255,255,255,0.07)", via: "rgba(255,255,255,0.12)", plastic: 0x161618, black: 0x0c0c0d,
        white: 0xe8e8e4, key: 0x232325, keyEdge: 0x2e2e31, fabric: 0x151517, ring: 0xf4f4f4, cable: 0x1a1a1c,
        glass: 0xe2e8ee, screenGlow: 0xdfe8ff, ceramic: 0xeeeeea, leaf: 0x6f8c6b, cover: 0x3a3a3d,
      }
    : {
        bgIn: "#ffffff", bgOut: "#dcdcd8", floor: 0.2, exposure: 1.0,
        desk: 0xf1f0ec, mat: 0x2a2a2c, alu: 0xc4c4c8, anod: 0x2a2a2d, caseBody: 0xf2f2f0, caseEdge: 0xdcdcda,
        pcb: "#141416", trace: "rgba(255,255,255,0.07)", via: "rgba(255,255,255,0.12)", plastic: 0x1c1c1e, black: 0x111113,
        white: 0xf4f4f1, key: 0xf3f3f0, keyEdge: 0xdadad6, fabric: 0x2a2a2c, ring: 0xffffff, cable: 0x222224,
        glass: 0xe3e9ee, screenGlow: 0xe8efff, ceramic: 0xffffff, leaf: 0x6f8c6b, cover: 0x161618,
      };
}

const LED = { ok: 0x18c75a, warn: 0xe5202b, tip: 0x8c8c8c, none: 0x5a5a5a };
const RED = "#e5202b";

/** A box with rounded edges and corners (three.js's RoundedBoxGeometry, trimmed): soft edges
    catch the light the way real products do. Centred on the origin. */
function roundedBoxGeometry(w, h, d, r, segments = 2) {
  const s = segments * 2 + 1;
  const radius = Math.max(0.0001, Math.min(w / 2, h / 2, d / 2, r));
  const g = new THREE.BoxGeometry(1, 1, 1, s, s, s).toNonIndexed();
  const pos = g.attributes.position.array;
  const nor = g.attributes.normal.array;
  const box = new THREE.Vector3(w, h, d).divideScalar(2).subScalar(radius);
  const half = 0.5 / s;
  const p = new THREE.Vector3();
  const n = new THREE.Vector3();
  for (let i = 0; i < pos.length; i += 3) {
    p.fromArray(pos, i);
    n.copy(p);
    n.x -= Math.sign(n.x) * half;
    n.y -= Math.sign(n.y) * half;
    n.z -= Math.sign(n.z) * half;
    n.normalize();
    pos[i] = box.x * Math.sign(p.x) + n.x * radius;
    pos[i + 1] = box.y * Math.sign(p.y) + n.y * radius;
    pos[i + 2] = box.z * Math.sign(p.z) + n.z * radius;
    nor[i] = n.x;
    nor[i + 1] = n.y;
    nor[i + 2] = n.z;
  }
  return g;
}

/** A soft studio to reflect: a dim room with a few bright panels, baked once into an
    environment map (what three.js's RoomEnvironment does, shaped for a desk shot). */
function studio(renderer, dark) {
  const env = new THREE.Scene();
  const disposable = [];
  const add = (geometry, color, setup) => {
    const material = new THREE.MeshBasicMaterial({ color, side: THREE.BackSide });
    disposable.push(geometry, material);
    const m = new THREE.Mesh(geometry, material);
    setup(m);
    env.add(m);
  };
  add(new THREE.BoxGeometry(1, 1, 1), new THREE.Color(dark ? 0x141416 : 0x7c7c7c), (m) => {
    m.scale.set(26, 14, 26);
    m.position.y = 6;
  });
  const panel = (w, h, x, y, z, strength) =>
    add(new THREE.PlaneGeometry(w, h), new THREE.Color(1, 1, 1).multiplyScalar(strength), (m) => {
      m.material.side = THREE.DoubleSide;
      m.position.set(x, y, z);
      m.lookAt(0, 1, 0);
    });
  panel(7, 4, -6, 9, 6, dark ? 7 : 9); // the key soft box, above and to the left
  panel(1.6, 9, 10, 4, -2, dark ? 5 : 6); // a tall strip on the right, for the long reflections
  panel(10, 2, 0, 11, -8, dark ? 3 : 4); // behind and above, for the rims
  panel(4, 4, 2, -2, 9, dark ? 0.8 : 1.6); // a low bounce from the front
  const pmrem = new THREE.PMREMGenerator(renderer);
  const target = pmrem.fromScene(env, 0.035);
  pmrem.dispose();
  disposable.forEach((d) => d.dispose());
  return target;
}

/** Text drawn as a dot matrix, like mom's Doto numbers: the text is set small, and each pixel
    becomes a dot. `round` keeps only the dots inside a circle (a round display). */
function dotMatrix(canvas, text, { cols, rows, on, off, round = false, weight = 700 }) {
  const small = document.createElement("canvas");
  small.width = cols;
  small.height = rows;
  const s = small.getContext("2d", { willReadFrequently: true });
  s.fillStyle = "#000";
  s.fillRect(0, 0, cols, rows);
  s.fillStyle = "#fff";
  s.textAlign = "center";
  s.textBaseline = "middle";
  s.font = `${weight} ${Math.round(rows * 0.78)}px "Geist Mono", "Cascadia Mono", Consolas, monospace`;
  s.fillText(text, cols / 2, rows / 2 + 1);
  const px = s.getImageData(0, 0, cols, rows).data;
  const ctx = canvas.getContext("2d");
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  const cw = canvas.width / cols;
  const ch = canvas.height / rows;
  const rad = Math.min(cw, ch) * 0.34;
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      const cx = (x + 0.5) * cw;
      const cy = (y + 0.5) * ch;
      if (round && Math.hypot(cx - canvas.width / 2, cy - canvas.height / 2) > canvas.width / 2 - cw * 0.6) continue;
      ctx.fillStyle = px[(y * cols + x) * 4] > 110 ? on : off;
      ctx.beginPath();
      ctx.arc(cx, cy, rad, 0, Math.PI * 2);
      ctx.fill();
    }
  }
}

/** A tiny repeatable random: the board's traces come out the same every time. */
function seeded(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function open(host, screenEl, model, on = {}) {
  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: "high-performance" });
  } catch {
    return null; // no 3D on this machine: the list of parts still works
  }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  // Shadows are worked out again only when something in the scene moves, not when you look around.
  renderer.shadowMap.autoUpdate = false;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  host.appendChild(renderer.domElement);

  const css = getComputedStyle(document.documentElement);
  const lum = (hex) => {
    const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(hex.trim());
    return m ? (parseInt(m[1], 16) * 299 + parseInt(m[2], 16) * 587 + parseInt(m[3], 16) * 114) / 255000 : 1;
  };
  const dark = lum(css.getPropertyValue("--bg") || "#ececec") < 0.5;
  const P = palette(dark);
  renderer.toneMappingExposure = P.exposure;
  // The backdrop: a soft studio sweep behind the desk.
  host.style.background = `radial-gradient(120% 95% at 42% 34%, ${P.bgIn} 0%, ${P.bgOut} 78%)`;

  const scene = new THREE.Scene();
  const envTarget = studio(renderer, dark);
  scene.environment = envTarget.texture;
  scene.environmentIntensity = dark ? 0.75 : 0.9;
  const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 100);
  // Where you look from: around (az) and above (pol) a spot on the desk, nearer or further (zoom).
  const HOME = { tx: 0.15, ty: 1.12, tz: 0.3, zoom: 1, az: 0.3, pol: 1.17 };
  const view = { ...HOME };
  const owned = [envTarget];
  const tweens = new Set();

  // ---------------------------------------------------------------- drawing only when needed

  let queued = 0;
  let closed = false;
  let ready = false;
  let shadowsStale = true;
  const draw = () => {
    queued = 0;
    if (closed || !ready) return;
    if (shadowsStale) {
      renderer.shadowMap.needsUpdate = true;
      shadowsStale = false;
    }
    renderer.render(scene, camera);
    layScreen();
  };
  const invalidate = () => {
    if (!queued && !closed) queued = requestAnimationFrame(draw);
  };
  const moved = () => {
    shadowsStale = true;
    invalidate();
  };
  /** A short move. The view (where you look from) is placed again on every step of its own;
      anything else that moves casts new shadows. */
  const tween = (target, vars) => {
    const step = target === view ? follow : moved;
    if (!G || calm()) {
      Object.assign(target, Object.fromEntries(Object.entries(vars).filter(([k]) => !["duration", "ease", "delay", "overwrite"].includes(k))));
      step();
      return null;
    }
    const t = G.to(target, { ...vars, onUpdate: step, onComplete: () => tweens.delete(t) });
    tweens.add(t);
    return t;
  };

  // ---------------------------------------------------------------- light

  const key = new THREE.DirectionalLight(0xfff8f0, dark ? 2.6 : 2.3);
  key.position.set(-5.5, 9, 7);
  key.target.position.set(0.6, 0.8, 0);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  Object.assign(key.shadow.camera, { left: -7.5, right: 7.5, top: 6.5, bottom: -4.5, near: 2, far: 26 });
  key.shadow.bias = -0.0004;
  key.shadow.normalBias = 0.025;
  key.shadow.radius = 4;
  scene.add(key, key.target);
  const rim = new THREE.DirectionalLight(dark ? 0xc8d4ff : 0xffffff, dark ? 1.4 : 0.9);
  rim.position.set(6, 6, -9);
  scene.add(rim);
  scene.add(new THREE.HemisphereLight(0xffffff, dark ? 0x101012 : 0xb9b9b5, dark ? 0.25 : 0.45));

  // ---------------------------------------------------------------- building blocks

  const geometries = new Map();
  const rounded = (w, h, d, r, seg = 2) => {
    const k = `${w}|${h}|${d}|${r}|${seg}`;
    if (!geometries.has(k)) {
      const g = roundedBoxGeometry(w, h, d, r, seg);
      owned.push(g);
      geometries.set(k, g);
    }
    return geometries.get(k);
  };
  const keep = (x) => {
    owned.push(x);
    return x;
  };
  /** Materials: matte, metal, anodised (satin), glossy, or light that glows by itself. All but
      the glass are one kind of material underneath: each kind is a shader the graphics driver
      compiles on first sight, and on Windows that's slow. */
  const matte = (color, roughness = 0.78, extra = {}) => keep(new THREE.MeshStandardMaterial({ color, roughness, metalness: 0, ...extra }));
  const metal = (color, roughness = 0.32, extra = {}) => keep(new THREE.MeshStandardMaterial({ color, roughness, metalness: 1, ...extra }));
  const satin = (color, roughness = 0.42) => keep(new THREE.MeshStandardMaterial({ color, roughness, metalness: 0.6 }));
  const glossy = (color, roughness = 0.3) => keep(new THREE.MeshStandardMaterial({ color, roughness, metalness: 0 }));
  const glow = (color) => keep(new THREE.MeshBasicMaterial({ color, toneMapped: false }));

  const shadowed = (m, cast = true) => {
    m.castShadow = cast;
    m.receiveShadow = true;
    return m;
  };
  /** A rounded box whose bottom sits on `y`, centred on x and z. Each mesh gets its own
      material, so one part can light up without the others. */
  const block = (w, h, d, r, mat, x, y, z, parent) => {
    const m = shadowed(new THREE.Mesh(rounded(w, h, d, r), mat.clone()));
    owned.push(m.material);
    m.position.set(x, y + h / 2, z);
    parent.add(m);
    return m;
  };
  /** A disc or drum whose round face looks at you (down +Z). */
  const drum = (r, depth, mat, x, y, z, parent, sides = 48) => {
    const g = keep(new THREE.CylinderGeometry(r, r, depth, sides));
    const m = shadowed(new THREE.Mesh(g, mat.clone()));
    owned.push(m.material);
    m.rotation.x = Math.PI / 2;
    m.position.set(x, y, z);
    parent.add(m);
    return m;
  };
  const ring = (r, tube, mat, x, y, z, parent, arc = Math.PI * 2) => {
    const g = keep(new THREE.TorusGeometry(r, tube, 12, 64, arc));
    const m = new THREE.Mesh(g, mat.clone());
    owned.push(m.material);
    m.position.set(x, y, z);
    parent.add(m);
    return m;
  };
  const lathe = (points, mat, parent, segments = 48) => {
    const g = keep(new THREE.LatheGeometry(points.map(([x, y]) => new THREE.Vector2(x, y)), segments));
    const m = shadowed(new THREE.Mesh(g, mat.clone()));
    owned.push(m.material);
    parent.add(m);
    return m;
  };
  const cable = (points, radius, mat, parent) => {
    const curve = new THREE.CatmullRomCurve3(points.map(([x, y, z]) => new THREE.Vector3(x, y, z)));
    const g = keep(new THREE.TubeGeometry(curve, 48, radius, 10, false));
    const m = shadowed(new THREE.Mesh(g, mat.clone()));
    owned.push(m.material);
    parent.add(m);
    return m;
  };
  const group = (parent = scene) => {
    const g = new THREE.Group();
    parent.add(g);
    return g;
  };
  const canvasTexture = (canvas) => {
    const t = keep(new THREE.CanvasTexture(canvas));
    t.colorSpace = THREE.SRGBColorSpace;
    t.anisotropy = 4;
    return t;
  };

  /** A fan: a thin frame, a softly lit ring, and blades that turn when you point at its part.
      It faces down +Z; `holder` turns it to face elsewhere. Returns the part that spins. */
  const fan = (parent, r, x, y, z, face = null) => {
    const holder = group(parent);
    holder.position.set(x, y, z);
    if (face) holder.rotation.set(face[0], face[1], face[2]);
    ring(r, r * 0.07, matte(P.black, 0.6), 0, 0, 0, holder);
    ring(r * 0.9, r * 0.022, glow(P.ring), 0, 0, 0.012, holder);
    const rotor = group(holder);
    drum(r * 0.27, 0.05, satin(P.anod), 0, 0, 0.01, rotor);
    const bladeGeo = rounded(r * 0.62, r * 0.3, 0.012, 0.006, 1);
    const bladeMat = keep(matte(P.plastic, 0.5));
    for (let i = 0; i < 9; i++) {
      const hold = group(rotor);
      hold.rotation.z = (i / 9) * Math.PI * 2;
      const blade = new THREE.Mesh(bladeGeo, bladeMat);
      blade.position.set(r * 0.52, 0, 0);
      blade.rotation.x = 0.42;
      hold.add(blade);
    }
    return rotor;
  };

  // ---------------------------------------------------------------- parts you can tap

  const parts = new Map(); // id -> { group, light, glow, level, spin: [groups], inside }
  const part = (id, inside = false) => {
    const g = group();
    g.userData.part = id;
    const p = { id, group: g, light: null, glow: null, level: "none", spin: [], inside };
    parts.set(id, p);
    return p;
  };
  // A part's little light: a bead, and a halo around it that glows its colour.
  const haloCanvas = document.createElement("canvas");
  haloCanvas.width = haloCanvas.height = 64;
  {
    const c = haloCanvas.getContext("2d");
    const grad = c.createRadialGradient(32, 32, 0, 32, 32, 32);
    grad.addColorStop(0, "rgba(255,255,255,1)");
    grad.addColorStop(0.18, "rgba(255,255,255,0.55)");
    grad.addColorStop(1, "rgba(255,255,255,0)");
    c.fillStyle = grad;
    c.fillRect(0, 0, 64, 64);
  }
  const haloTexture = canvasTexture(haloCanvas);
  const beadGeo = keep(new THREE.SphereGeometry(0.038, 16, 12));
  const lamp = (p, x, y, z) => {
    const bead = new THREE.Mesh(beadGeo, glow(LED.none));
    bead.position.set(x, y, z);
    const halo = new THREE.Sprite(keep(new THREE.SpriteMaterial({ map: haloTexture, color: LED.none, blending: THREE.AdditiveBlending, depthWrite: false, transparent: true, toneMapped: false })));
    halo.scale.set(0.34, 0.34, 1);
    bead.add(halo);
    p.group.add(bead);
    p.light = bead;
    p.glow = halo;
  };

  // The floor under the desk: only the desk's shadow shows on it.
  {
    const floor = new THREE.Mesh(keep(new THREE.PlaneGeometry(40, 40)), keep(new THREE.ShadowMaterial({ opacity: P.floor })));
    floor.rotation.x = -Math.PI / 2;
    floor.position.y = -0.62;
    floor.receiveShadow = true;
    scene.add(floor);
  }

  // The desk: a thick slab with soft edges, and a felt mat for the keyboard and mouse.
  {
    const desk = block(10, 0.34, 5.1, 0.1, matte(P.desk, 0.58), 0, -0.34, 0.45, scene);
    desk.castShadow = true;
    block(4.6, 0.022, 1.4, 0.06, matte(P.mat, 0.95), -1.0, 0, 1.4, scene).castShadow = false;
  }

  // The monitor: mom lives on its screen. A slim panel on a brushed aluminium stand, with its
  // light under the screen where a power light would be. Tapping the frame or the stand picks it;
  // tapping the screen pokes mom.
  const MX = -1.95;
  const monitorPart = part("monitor");
  const monitor = monitorPart.group;
  {
    const alu = metal(P.alu, 0.38);
    block(1.3, 0.045, 0.82, 0.035, alu, MX, 0, -0.45, monitor);
    const neck = block(0.3, 1.32, 0.07, 0.03, alu, MX, 0.02, -0.6, monitor);
    neck.rotation.x = -0.08;
    block(3.42, 2.3, 0.1, 0.06, satin(P.anod, 0.45), MX, 0.78, -0.5, monitor); // its back
    block(3.4, 2.26, 0.05, 0.04, glossy(P.black, 0.2), MX, 0.8, -0.42, monitor); // the black glass around the screen
  }
  const SCREEN = { w: 3.06, h: 2.0 };
  const glassGeo = keep(new THREE.PlaneGeometry(SCREEN.w, SCREEN.h));
  const glass = new THREE.Mesh(glassGeo, glow(0xffffff));
  glass.position.set(MX, 0.8 + 1.13, -0.42 + 0.027);
  monitor.add(glass);
  lamp(monitorPart, MX + 1.46, 0.866, -0.385);
  // The screen's light falls on the keyboard and the desk in front of it.
  {
    const spill = new THREE.PointLight(P.screenGlow, dark ? 1.6 : 0.6, 4.2, 2);
    spill.position.set(MX, 1.6, 0.35);
    scene.add(spill);
  }

  // The keyboard: an aluminium case with real keys, Esc in red.
  const keyboard = part("keyboard");
  {
    const KX = MX;
    const KZ = 1.36;
    block(2.62, 0.075, 0.9, 0.04, satin(dark ? P.anod : P.alu, 0.4), KX, 0.022, KZ, keyboard.group);
    const rows = [
      [1, 0.25, 1, 1, 1, 1, 0.25, 1, 1, 1, 1, 0.25, 1, 1, 1, 1],
      [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2],
      [1.5, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1.5],
      [1.75, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2.25],
      [2.25, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2.75],
      [1.25, 1.25, 1.25, 6.25, 1.25, 1.25, 1.25, 1.25],
    ];
    const u = 2.42 / 15;
    const keyGeo = rounded(1, 1, 1, 0.12, 1);
    const caps = [];
    rows.forEach((row, r) => {
      let x = 0;
      for (const w of row) {
        const skip = w < 0.5; // a gap in the function row
        if (!skip) caps.push({ x: x + w / 2, w, r, esc: r === 0 && x === 0 });
        x += w;
      }
    });
    const capMat = matte(0xffffff, 0.5);
    const keys = new THREE.InstancedMesh(keyGeo, capMat, caps.length);
    keys.castShadow = true;
    keys.receiveShadow = true;
    const m4 = new THREE.Matrix4();
    const q = new THREE.Quaternion().setFromEuler(new THREE.Euler(-0.06, 0, 0));
    caps.forEach((c, i) => {
      const pos = new THREE.Vector3(KX - (15 * u) / 2 + c.x * u, 0.13, KZ - 0.34 + c.r * 0.135);
      m4.compose(pos, q, new THREE.Vector3(c.w * u - 0.024, 0.075, 0.115));
      keys.setMatrixAt(i, m4);
      if (c.esc) keys.setColorAt(i, new THREE.Color(RED));
      else keys.setColorAt(i, new THREE.Color(P.key));
    });
    keyboard.group.add(keys);
    lamp(keyboard, KX + 1.2, 0.12, KZ - 0.4);
  }

  // The mouse: a smooth pebble with a scroll wheel.
  const mouse = part("mouse");
  {
    const body = lathe([[0, 0], [0.18, 0.004], [0.205, 0.035], [0.2, 0.085], [0.165, 0.135], [0.1, 0.168], [0, 0.18]], glossy(P.key, 0.4), mouse.group);
    body.scale.set(0.78, 1, 1.25);
    body.position.set(MX + 2.0, 0.022, 1.45);
    const wheel = drum(0.03, 0.026, metal(P.alu, 0.3), MX + 2.0, 0.205, 1.3, mouse.group, 20);
    wheel.rotation.set(0, 0, Math.PI / 2);
    lamp(mouse, MX + 2.0, 0.21, 1.58);
  }

  // The speakers, either side of the monitor: fabric fronts, a woofer and a tweeter each.
  const sound = part("sound");
  for (const x of [MX - 2.3, MX + 2.3]) {
    block(0.62, 1.02, 0.56, 0.07, satin(P.anod, 0.5), x, 0, -0.38, sound.group);
    block(0.54, 0.94, 0.03, 0.04, matte(P.fabric, 1), x, 0.04, -0.09, sound.group);
    const woofer = lathe([[0, 0.0], [0.04, 0.004], [0.17, 0.04], [0.2, 0.048]], matte(P.black, 0.7), sound.group);
    woofer.rotation.x = Math.PI / 2;
    woofer.position.set(x, 0.36, -0.12);
    drum(0.05, 0.03, metal(P.alu, 0.25), x, 0.36, -0.06, sound.group);
    ring(0.2, 0.012, metal(P.alu, 0.3), x, 0.36, -0.07, sound.group);
    const dome = lathe([[0, 0.03], [0.04, 0.022], [0.06, 0.0]], metal(P.alu, 0.2), sound.group);
    dome.rotation.x = Math.PI / 2;
    dome.position.set(x, 0.78, -0.09);
  }
  lamp(sound, MX + 2.3 + 0.2, 0.93, -0.06);

  // The tower: an aluminium-edged case with a glass side. Inside, from the back: the
  // motherboard, the cooler, memory, a graphics card shown off fans-out, drives on the shroud.
  const TX = 3.05; // centre
  const TW = 2.7; // width
  const TH = 2.95; // height
  const TD = 1.25; // depth
  const Z0 = -0.85; // its back wall
  const shell = group();
  {
    const body = satin(P.caseBody, dark ? 0.45 : 0.5);
    const edge = metal(P.alu, 0.36);
    block(TW, 0.06, TD, 0.03, body, TX, 0.06, Z0 + TD / 2, shell); // floor
    block(TW, 0.09, TD + 0.02, 0.04, edge, TX, 0.06 + TH - 0.09, Z0 + TD / 2, shell); // roof
    block(0.08, TH, TD, 0.035, body, TX - TW / 2 + 0.04, 0.06, Z0 + TD / 2, shell); // back end
    block(0.11, TH, TD + 0.02, 0.04, edge, TX + TW / 2 - 0.055, 0.06, Z0 + TD / 2, shell); // front, brushed
    block(TW, TH, 0.05, 0.02, body, TX, 0.06, Z0, shell); // the wall behind the motherboard
    for (const x of [TX - TW / 2 + 0.28, TX + TW / 2 - 0.28]) block(0.3, 0.06, 1.0, 0.03, matte(P.black, 0.9), x, 0, Z0 + TD / 2, shell); // feet
    // A light strip down the front.
    block(0.018, TH - 0.5, 0.018, 0.008, glow(P.ring), TX + TW / 2 - 0.004, 0.31, Z0 + TD - 0.08, shell);
    // The shroud over the power supply, with mom's name in dots.
    const shroudFront = Z0 + TD - 0.12;
    block(TW - 0.18, 0.56, TD - 0.14, 0.03, body, TX, 0.12, Z0 + (TD - 0.14) / 2 + 0.02, shell);
    const word = document.createElement("canvas");
    word.width = 512;
    word.height = 64;
    dotMatrix(word, "momputer", { cols: 64, rows: 8, on: dark ? "#9a9a9a" : "#3a3a3a", off: "rgba(0,0,0,0)" });
    const wordMat = keep(new THREE.MeshBasicMaterial({ map: canvasTexture(word), transparent: true, opacity: dark ? 0.9 : 0.75, toneMapped: false }));
    wordMat.alphaTest = 0.02;
    {
      // Black stays see-through: only the dots show.
      const ctx = word.getContext("2d");
      const img = ctx.getImageData(0, 0, word.width, word.height);
      for (let i = 0; i < img.data.length; i += 4) if (img.data[i] + img.data[i + 1] + img.data[i + 2] < 30) img.data[i + 3] = 0;
      ctx.putImageData(img, 0, 0);
      wordMat.map.needsUpdate = true;
    }
    const plate = new THREE.Mesh(keep(new THREE.PlaneGeometry(1.1, 0.1375)), wordMat);
    plate.position.set(TX - 0.45, 0.4, shroudFront + 0.002);
    shell.add(plate);
    // Vents along the shroud's top edge.
    const ventGeo = rounded(0.16, 0.012, 0.03, 0.006, 1);
    const ventMat = keep(matte(P.black, 0.9));
    for (let i = 0; i < 9; i++) {
      const v = new THREE.Mesh(ventGeo, ventMat);
      v.position.set(TX + 0.35 + (i % 3) * 0.2, 0.56 - Math.floor(i / 3) * 0.06, shroudFront + 0.004);
      shell.add(v);
    }
    // Case fans: one out the back, two in at the front.
    fan(shell, 0.4, TX - TW / 2 + 0.11, 2.06, Z0 + 0.62, [0, Math.PI / 2, 0]);
    fan(shell, 0.4, TX + TW / 2 - 0.13, 2.0, Z0 + 0.62, [0, -Math.PI / 2, 0]);
    fan(shell, 0.4, TX + TW / 2 - 0.13, 1.12, Z0 + 0.62, [0, -Math.PI / 2, 0]);
  }

  {
    const inside = new THREE.PointLight(0xffffff, dark ? 3.2 : 2.2, 3.6, 1.6);
    inside.position.set(TX + 0.2, 2.35, Z0 + 0.95);
    shell.add(inside);
    const low = new THREE.PointLight(0xffffff, dark ? 1.4 : 1.0, 2.6, 1.6);
    low.position.set(TX - 0.3, 0.95, Z0 + 0.95);
    shell.add(low);
  }

  // The glass side, on a hinge at the back end: it swings open when you pick a part inside.
  const door = group(shell);
  door.position.set(TX - TW / 2 + 0.02, 0.09, Z0 + TD + 0.012);
  {
    const pane = new THREE.Mesh(
      rounded(TW - 0.06, TH - 0.08, 0.018, 0.009, 1),
      keep(new THREE.MeshPhysicalMaterial({ color: P.glass, metalness: 0, roughness: 0.04, transmission: 1, thickness: 0.02, ior: 1.45, specularIntensity: 0.7, envMapIntensity: 0.7 }))
    );
    pane.position.set((TW - 0.06) / 2, (TH - 0.08) / 2, 0);
    door.add(pane);
    const screw = rounded(0.05, 0.05, 0.012, 0.02, 1);
    const screwMat = keep(metal(P.alu, 0.25));
    for (const [x, y] of [[0.08, 0.08], [TW - 0.14, 0.08], [0.08, TH - 0.16], [TW - 0.14, TH - 0.16]]) {
      const s = new THREE.Mesh(screw, screwMat);
      s.position.set(x, y, 0.014);
      door.add(s);
    }
  }

  // The motherboard, on the back wall: a black board with fine traces, heatsinks, slots.
  const board = part("board", true);
  const BX = TX - 0.28;
  {
    const c = document.createElement("canvas");
    c.width = c.height = 512;
    const x = c.getContext("2d");
    x.fillStyle = P.pcb;
    x.fillRect(0, 0, 512, 512);
    const rnd = seeded(2610);
    x.strokeStyle = P.trace;
    x.lineCap = "round";
    for (let i = 0; i < 160; i++) {
      x.lineWidth = rnd() < 0.8 ? 1.2 : 2.4;
      let px = rnd() * 512;
      let py = rnd() * 512;
      x.beginPath();
      x.moveTo(px, py);
      for (let s = 0; s < 4; s++) {
        if (s % 2) px += (rnd() - 0.5) * 160;
        else py += (rnd() - 0.5) * 160;
        x.lineTo(px, py);
      }
      x.stroke();
    }
    x.fillStyle = P.via;
    for (let i = 0; i < 220; i++) {
      x.beginPath();
      x.arc(rnd() * 512, rnd() * 512, 1.6, 0, Math.PI * 2);
      x.fill();
    }
    const pcbMat = keep(new THREE.MeshStandardMaterial({ map: canvasTexture(c), roughness: 0.5, metalness: 0.25 }));
    const pcb = shadowed(new THREE.Mesh(keep(new THREE.BoxGeometry(1.86, 1.95, 0.035)), pcbMat));
    pcb.position.set(BX, 0.82 + 1.95 / 2, Z0 + 0.045);
    board.group.add(pcb);
    const sink = satin(P.anod, 0.38);
    block(0.34, 0.66, 0.16, 0.03, sink, BX - 0.74, 2.05, Z0 + 0.11, board.group); // the I/O cover
    block(0.7, 0.16, 0.12, 0.025, sink, BX - 0.2, 2.53, Z0 + 0.1, board.group); // power stage heatsink, top
    block(0.16, 0.6, 0.12, 0.025, sink, BX - 0.62, 1.86, Z0 + 0.1, board.group); // and left
    block(0.44, 0.34, 0.06, 0.025, metal(P.alu, 0.34), BX + 0.56, 0.9, Z0 + 0.09, board.group); // chipset
    for (const y of [1.2, 0.95]) block(1.3, 0.05, 0.05, 0.012, matte(P.black, 0.6), BX + 0.02, y, Z0 + 0.09, board.group); // card slots
    block(0.06, 0.56, 0.08, 0.02, matte(P.black, 0.6), BX + 0.9, 1.58, Z0 + 0.1, board.group); // the 24-pin socket
    drum(0.06, 0.012, metal(P.alu, 0.2), BX + 0.2, 0.98, Z0 + 0.075, board.group, 24); // the clock battery
    lamp(board, BX - 0.82, 0.94, Z0 + 0.1);
  }

  // The processor, under a liquid cooler: the pump has a round dot display.
  const processor = part("processor", true);
  const CX = BX - 0.3;
  const CY = 2.1;
  const pumpCanvas = document.createElement("canvas");
  pumpCanvas.width = pumpCanvas.height = 256;
  const pumpTexture = canvasTexture(pumpCanvas);
  let pumpSays = "";
  const pumpShows = (text, color) => {
    if (pumpSays === text + color) return;
    pumpSays = text + color;
    dotMatrix(pumpCanvas, text, { cols: 17, rows: 17, on: color, off: dark ? "#141416" : "#1a1a1c", round: true });
    pumpTexture.needsUpdate = true;
    invalidate();
  };
  {
    drum(0.31, 0.06, metal(P.alu, 0.3), CX, CY, Z0 + 0.1, processor.group); // the plate on the processor
    drum(0.29, 0.16, satin(P.anod, 0.35), CX, CY, Z0 + 0.2, processor.group); // the pump
    ring(0.29, 0.016, metal(P.alu, 0.22), CX, CY, Z0 + 0.282, processor.group);
    const face = new THREE.Mesh(keep(new THREE.CircleGeometry(0.255, 64)), keep(new THREE.MeshBasicMaterial({ map: pumpTexture, toneMapped: false })));
    face.position.set(CX, CY, Z0 + 0.283);
    processor.group.add(face);
    const hose = matte(P.cable, 0.85);
    cable([[CX - 0.12, CY + 0.26, Z0 + 0.22], [CX - 0.16, CY + 0.5, Z0 + 0.4], [CX + 0.1, 2.62, Z0 + 0.58], [TX - 0.75, 2.78, Z0 + 0.62]], 0.034, hose, processor.group);
    cable([[CX + 0.12, CY + 0.26, Z0 + 0.22], [CX + 0.2, CY + 0.46, Z0 + 0.36], [CX + 0.5, 2.6, Z0 + 0.5], [TX - 0.35, 2.78, Z0 + 0.52]], 0.034, hose, processor.group);
    // The radiator under the roof, its fans blowing up and out.
    block(2.05, 0.12, 0.66, 0.03, satin(P.anod, 0.5), TX - 0.1, 2.8, Z0 + 0.55, processor.group);
    for (const dx of [-0.48, 0.48]) processor.spin.push(fan(processor.group, 0.28, TX - 0.1 + dx, 2.74, Z0 + 0.55, [Math.PI / 2, 0, 0]));
  }
  lamp(processor, CX + 0.38, CY + 0.34, Z0 + 0.3);

  // Memory: as many sticks as the PC has, each with a heat spreader and a light bar on top.
  const memory = part("memory", true);
  const sticks = Math.max(1, Math.min(8, model.sticks || 2));
  {
    const spread = satin(P.anod, 0.32);
    const bar = glow(P.ring);
    const pitch = sticks > 4 ? 0.075 : 0.1;
    for (let i = 0; i < sticks; i++) {
      const x = BX + 0.26 + i * pitch;
      block(0.04, 0.12, 0.05, 0.01, matte(P.black, 0.6), x, 1.72, Z0 + 0.09, memory.group); // the slot
      block(0.048, 0.76, 0.24, 0.014, spread, x, 1.76, Z0 + 0.19, memory.group);
      block(0.05, 0.035, 0.24, 0.012, bar, x, 2.52, Z0 + 0.19, memory.group).castShadow = false;
    }
    lamp(memory, BX + 0.26 + ((sticks - 1) * pitch) / 2, 2.66, Z0 + 0.32);
  }

  // The graphics card(s): shown off fans-out, three fans each, a light strip along the top.
  const graphics = part("graphics", true);
  const cards = Math.max(1, Math.min(2, model.cards || 1));
  {
    const shroud = satin(P.anod, 0.4);
    const trim = metal(P.alu, 0.32);
    for (let c = 0; c < cards; c++) {
      const y = cards === 1 ? 1.08 : 1.34 - c * 0.58;
      const z = Z0 + 0.5;
      block(1.98, 0.48, 0.03, 0.012, metal(P.anod, 0.4), BX + 0.02, y + 0.01, Z0 + 0.38, graphics.group); // the backplate
      block(1.98, 0.5, 0.2, 0.06, shroud, BX + 0.02, y, z, graphics.group);
      block(1.9, 0.05, 0.21, 0.02, trim, BX + 0.02, y + 0.45, z, graphics.group);
      block(1.1, 0.012, 0.1, 0.005, glow(P.ring), BX + 0.12, y + 0.505, z + 0.02, graphics.group).castShadow = false;
      block(0.04, 0.62, 0.36, 0.01, trim, BX - 0.97, y - 0.06, Z0 + 0.36, graphics.group); // its bracket
      for (const dx of [-0.6, 0, 0.6]) graphics.spin.push(fan(graphics.group, 0.21, BX + 0.02 + dx, y + 0.23, z + 0.105));
      cable([[BX + 0.55, y + 0.5, z - 0.02], [BX + 0.7, y + 0.66, z - 0.05], [BX + 1.0, y + 0.6, z - 0.15], [TX + 1.05, 0.72, Z0 + 0.6]], 0.03, matte(P.cable, 0.85), graphics.group);
    }
    lamp(graphics, BX + 0.92, cards === 1 ? 1.68 : 1.94, Z0 + 0.62);
  }

  // The 24-pin cable from the board's edge, down behind the shroud.
  cable([[BX + 0.95, 1.6, Z0 + 0.14], [BX + 1.08, 1.5, Z0 + 0.3], [BX + 1.12, 1.0, Z0 + 0.4], [BX + 1.05, 0.66, Z0 + 0.42]], 0.05, matte(P.cable, 0.9), shell);

  // Drives: a heatsinked stick on the board for each NVMe one, the rest on the shroud.
  const disks = part("disks", true);
  const drives = (model.drives && model.drives.length ? model.drives : ["nvme"]).slice(0, 5);
  {
    let onBoard = 0;
    let onShroud = 0;
    for (const kind of drives) {
      if (kind === "nvme") {
        const y = onBoard === 0 ? 1.62 : 0.86 - (onBoard - 1) * 0.14;
        block(0.62, 0.11, 0.035, 0.012, metal(P.alu, 0.3), BX - 0.42, y, Z0 + 0.075, disks.group);
        onBoard++;
      } else {
        const x = TX + 0.95 - onShroud * 0.2;
        if (kind === "hdd") block(0.16, 0.5, 0.62, 0.03, metal(P.alu, 0.42), x, 0.68, Z0 + 0.62, disks.group);
        else block(0.06, 0.42, 0.56, 0.02, satin(P.white, 0.4), x, 0.68, Z0 + 0.62, disks.group);
        onShroud++;
      }
    }
    lamp(disks, onShroud ? TX + 0.95 : BX - 0.1, onShroud ? 1.28 : 1.7, onShroud ? Z0 + 0.95 : Z0 + 0.12);
  }

  // The power button: Windows itself. Its ring lights in Windows's colour.
  const windows = part("windows");
  let powerRing;
  {
    const bx = TX + TW / 2 - 0.3;
    const top = 0.06 + TH;
    const btn = drum(0.11, 0.04, metal(P.alu, 0.25), bx, top + 0.01, Z0 + TD - 0.25, windows.group);
    btn.rotation.x = 0;
    powerRing = ring(0.115, 0.012, glow(LED.none), bx, top + 0.032, Z0 + TD - 0.25, windows.group);
    powerRing.rotation.x = Math.PI / 2;
    lamp(windows, bx + 0.22, top + 0.03, Z0 + TD - 0.25);
  }

  // A few things that make it mom's desk: her mug, a plant, a notebook.
  {
    const mug = group();
    mug.position.set(1.18, 0, 1.62);
    lathe([[0, 0], [0.15, 0], [0.165, 0.02], [0.17, 0.33], [0.152, 0.33], [0.148, 0.04], [0, 0.04]], glossy(P.ceramic, 0.25), mug);
    const handle = ring(0.085, 0.022, glossy(P.ceramic, 0.25), 0.19, 0.17, 0, mug, Math.PI);
    handle.rotation.z = -Math.PI / 2;
    const smile = document.createElement("canvas");
    smile.width = 128;
    smile.height = 128;
    dotMatrix(smile, "c:", { cols: 9, rows: 9, on: "#1a1a1a", off: "rgba(0,0,0,0)" });
    {
      const ctx = smile.getContext("2d");
      const img = ctx.getImageData(0, 0, 128, 128);
      for (let i = 0; i < img.data.length; i += 4) if (img.data[i] + img.data[i + 1] + img.data[i + 2] < 10 && img.data[i + 3] > 0 && img.data[i] === 0) img.data[i + 3] = 0;
      ctx.putImageData(img, 0, 0);
    }
    const print = new THREE.Mesh(keep(new THREE.CylinderGeometry(0.1715, 0.1715, 0.14, 24, 1, true, -0.55, 1.1)), keep(new THREE.MeshStandardMaterial({ map: canvasTexture(smile), transparent: true, roughness: 0.3 })));
    print.position.y = 0.19;
    print.rotation.y = 0.35;
    mug.add(print);

    const plant = group();
    plant.position.set(4.55, 0, 2.05);
    lathe([[0, 0], [0.16, 0], [0.2, 0.05], [0.22, 0.34], [0.205, 0.34], [0.19, 0.06], [0, 0.06]], matte(P.ceramic, 0.6), plant);
    drum(0.2, 0.02, matte(0x2a211b, 1), 0, 0.31, 0, plant).rotation.x = 0;
    const leafGeo = keep(new THREE.SphereGeometry(0.1, 16, 10));
    const leafMat = matte(P.leaf, 0.55);
    const rnd = seeded(77);
    for (let i = 0; i < 11; i++) {
      const a = (i / 11) * Math.PI * 2 + rnd() * 0.4;
      const leaf = shadowed(new THREE.Mesh(leafGeo, leafMat));
      leaf.scale.set(0.42, 1.5 + rnd() * 0.5, 0.18);
      leaf.position.set(Math.cos(a) * 0.08, 0.48 + rnd() * 0.08, Math.sin(a) * 0.08);
      leaf.rotation.set(Math.sin(a) * 0.55, -a, Math.cos(a) * 0.55);
      plant.add(leaf);
    }

    const book = group();
    book.position.set(-4.2, 0, 1.55);
    book.rotation.y = -0.16;
    block(0.78, 0.05, 1.02, 0.02, matte(P.cover, 0.7), 0, 0, 0, book);
    block(0.76, 0.034, 0.99, 0.01, matte(0xf4f2ec, 0.95), 0.02, 0.008, 0, book);
    block(0.05, 0.056, 1.03, 0.01, matte(RED, 0.6), 0.26, 0, 0, book);
  }

  // Thin parts are hard to hit (a tap lands between two memory sticks): each part also gets an
  // unseen box around the whole of it, for the pointer only.
  const proxies = [];
  for (const p of parts.values()) {
    const bounds = new THREE.Box3().setFromObject(p.group);
    const span = bounds.getSize(new THREE.Vector3());
    const geo = keep(new THREE.BoxGeometry(span.x, span.y, Math.max(span.z, 0.05)));
    const mat = keep(new THREE.MeshBasicMaterial({ transparent: true, opacity: 0, depthWrite: false }));
    const proxy = new THREE.Mesh(geo, mat);
    bounds.getCenter(proxy.position);
    proxy.userData.part = p.id;
    proxy.userData.proxy = true;
    proxies.push(proxy);
  }

  for (const p of parts.values()) {
    p.group.traverse((o) => {
      if (o.isMesh && o !== p.light) o.userData.part = p.id;
    });
  }

  // ---------------------------------------------------------------- looking around

  let fit = 12;
  function place() {
    const r = fit * view.zoom;
    const s = Math.sin(view.pol);
    camera.position.set(view.tx + r * s * Math.sin(view.az), view.ty + r * Math.cos(view.pol), view.tz + r * s * Math.cos(view.az));
    camera.lookAt(view.tx, view.ty, view.tz);
    camera.updateMatrixWorld();
  }
  function follow() {
    place();
    invalidate();
  }
  const size = () => {
    const w = Math.max(1, host.clientWidth);
    const h = Math.max(1, host.clientHeight);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    // Far enough back for the whole desk to fit, wide or tall.
    const v = (camera.fov * Math.PI) / 360;
    const hfov = Math.atan(Math.tan(v) * camera.aspect);
    fit = Math.max(6.0 / Math.tan(hfov), 3.05 / Math.tan(v));
    camera.updateProjectionMatrix();
    place();
    invalidate();
  };
  const resize = new ResizeObserver(size);
  resize.observe(host);

  // Mom's own element, laid onto the monitor's glass with the same perspective.
  const SW = 612;
  const SH = 400;
  screenEl.style.width = SW + "px";
  screenEl.style.height = SH + "px";
  const corner = new THREE.Vector3();
  const adj = (m) => [m[4] * m[8] - m[5] * m[7], m[2] * m[7] - m[1] * m[8], m[1] * m[5] - m[2] * m[4], m[5] * m[6] - m[3] * m[8], m[0] * m[8] - m[2] * m[6], m[2] * m[3] - m[0] * m[5], m[3] * m[7] - m[4] * m[6], m[1] * m[6] - m[0] * m[7], m[0] * m[4] - m[1] * m[3]];
  const mul = (a, b) => {
    const c = new Array(9);
    for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) c[3 * i + j] = a[3 * i] * b[j] + a[3 * i + 1] * b[3 + j] + a[3 * i + 2] * b[6 + j];
    return c;
  };
  const mulv = (m, v) => [m[0] * v[0] + m[1] * v[1] + m[2] * v[2], m[3] * v[0] + m[4] * v[1] + m[5] * v[2], m[6] * v[0] + m[7] * v[1] + m[8] * v[2]];
  const basis = (x1, y1, x2, y2, x3, y3, x4, y4) => {
    const m = [x1, x2, x3, y1, y2, y3, 1, 1, 1];
    const v = mulv(adj(m), [x4, y4, 1]);
    return mul(m, [v[0], 0, 0, 0, v[1], 0, 0, 0, v[2]]);
  };
  function layScreen() {
    const w = host.clientWidth;
    const h = host.clientHeight;
    const px = [];
    for (const [sx, sy] of [[-1, 1], [1, 1], [-1, -1], [1, -1]]) {
      corner.set((sx * SCREEN.w) / 2, (sy * SCREEN.h) / 2, 0);
      glass.localToWorld(corner).project(camera);
      px.push((corner.x * 0.5 + 0.5) * w, (-corner.y * 0.5 + 0.5) * h);
    }
    const s = basis(0, 0, SW, 0, 0, SH, SW, SH);
    const d = basis(px[0], px[1], px[2], px[3], px[4], px[5], px[6], px[7]);
    let t = mul(d, adj(s));
    if (!t[8]) return;
    t = t.map((x) => x / t[8]);
    screenEl.style.transform = `matrix3d(${t[0]},${t[3]},0,${t[6]},${t[1]},${t[4]},0,${t[7]},0,0,1,0,${t[2]},${t[5]},0,${t[8]})`;
    screenEl.style.visibility = "visible";
  }

  // ---------------------------------------------------------------- pointing and tapping

  const ray = new THREE.Raycaster();
  const point = new THREE.Vector2();
  const pickable = [];
  for (const p of parts.values()) p.group.traverse((o) => o.isMesh && o !== p.light && pickable.push(o));
  // The sound's box would swallow the monitor between the speakers, so the speakers stay as they are.
  for (const proxy of proxies) {
    if (proxy.userData.part === "sound") continue;
    scene.add(proxy);
    pickable.push(proxy);
  }
  let hovered = null;
  let chosen = null;

  const lit = (p) => (p === chosen ? 0.07 : p === hovered ? 0.05 : 0);
  const glowSize = (p) => (p.level === "none" ? 0.2 : p === chosen ? 0.56 : p === hovered ? 0.44 : 0.34);
  let booted = false;
  const paint = () => {
    for (const p of parts.values()) {
      const e = lit(p);
      p.group.traverse((o) => {
        if (o.isMesh && o !== p.light && o.material.emissive) {
          o.material.emissive.setHex(dark ? 0xffffff : 0xdfe6ff);
          o.material.emissiveIntensity = dark ? e : e * 0.55;
        }
      });
      const colour = LED[p.level] || LED.none;
      p.light.material.color.setHex(colour);
      p.glow.material.color.setHex(colour);
      p.glow.material.opacity = p.level === "none" ? 0.25 : 0.95;
      if (booted) p.glow.scale.setScalar(glowSize(p));
    }
    powerRing.material.color.setHex(LED[parts.get("windows").level] || LED.none);
    const bad = [...parts.values()].filter((p) => p.level === "warn").length;
    const seen = [...parts.values()].some((p) => p.level !== "none");
    pumpShows(!seen ? "··" : bad ? String(bad) : "c:", bad ? RED : dark ? "#f2f2f2" : "#f4f4f4");
    invalidate();
  };

  const under = (e) => {
    const r = renderer.domElement.getBoundingClientRect();
    point.set(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    ray.setFromCamera(point, camera);
    const hit = ray.intersectObjects(pickable, false)[0];
    return hit ? parts.get(hit.object.userData.part) : null;
  };

  const whirl = (p) => {
    for (const s of p.spin) tween(s.rotation, { z: s.rotation.z - Math.PI * 4, duration: 1.6, ease: "power2.out", overwrite: "auto" });
  };

  let drag = null;
  let moveFrame = 0;
  const canvas = renderer.domElement;
  canvas.style.touchAction = "none";
  const down = (e) => {
    if (e.button !== 0) return;
    drag = { x: e.clientX, y: e.clientY, az: view.az, pol: view.pol, moved: false };
    canvas.setPointerCapture(e.pointerId);
  };
  const move = (e) => {
    if (drag) {
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      if (!drag.moved && Math.hypot(dx, dy) < 4) return;
      drag.moved = true;
      view.az = Math.max(-0.75, Math.min(0.9, drag.az - dx * 0.005));
      view.pol = Math.max(0.95, Math.min(1.5, drag.pol - dy * 0.004));
      place();
      invalidate();
      if (on.hover) on.hover(null);
      return;
    }
    if (moveFrame) return;
    moveFrame = requestAnimationFrame(() => {
      moveFrame = 0;
      const p = under(e);
      if (p !== hovered) {
        hovered = p;
        canvas.style.cursor = p ? "pointer" : "grab";
        if (p) whirl(p);
        paint();
      }
      if (on.hover) on.hover(p ? p.id : null, e.clientX, e.clientY);
    });
  };
  const up = (e) => {
    const was = drag;
    drag = null;
    if (!was || was.moved) return;
    const p = under(e);
    if (on.pick) on.pick(p ? p.id : null);
  };
  const leave = () => {
    if (hovered) {
      hovered = null;
      paint();
    }
    if (on.hover) on.hover(null);
  };
  const wheel = (e) => {
    e.preventDefault();
    view.zoom = Math.max(0.58, Math.min(1.3, view.zoom + Math.sign(e.deltaY) * 0.06));
    place();
    invalidate();
  };
  canvas.addEventListener("pointerdown", down);
  canvas.addEventListener("pointermove", move);
  canvas.addEventListener("pointerup", up);
  canvas.addEventListener("pointercancel", () => (drag = null));
  canvas.addEventListener("pointerleave", leave);
  canvas.addEventListener("wheel", wheel, { passive: false });
  canvas.style.cursor = "grab";

  // ---------------------------------------------------------------- what the app asks of it

  const centre = new THREE.Vector3();
  const centreOf = (p) => new THREE.Box3().setFromObject(p.group).getCenter(centre);

  function update(next) {
    for (const p of parts.values()) p.level = (next.levels && next.levels[p.id]) || "none";
    paint();
  }

  /** Picks a part (or none): it lights up, its fans turn, and the view leans toward it. For a
      part inside the tower, the glass door swings open and the view comes in close. */
  function select(id) {
    chosen = id ? parts.get(id) || null : null;
    if (chosen) {
      whirl(chosen);
      const c = centreOf(chosen);
      const near = chosen.inside;
      tween(view, {
        tx: HOME.tx + (c.x - HOME.tx) * (near ? 0.85 : 0.32),
        ty: HOME.ty + (c.y - HOME.ty) * (near ? 0.75 : 0.25),
        tz: HOME.tz + (c.z - HOME.tz) * (near ? 0.4 : 0.1),
        az: near ? 0.42 : HOME.az,
        zoom: near ? 0.54 : 0.88,
        duration: 0.9,
        ease: "power3.inOut",
        overwrite: "auto",
      });
      tween(door.rotation, { y: near ? -1.02 : 0, duration: near ? 0.85 : 0.6, ease: near ? "power2.out" : "power2.inOut", overwrite: "auto" });
    } else {
      tween(view, { ...HOME, duration: 0.8, ease: "power3.inOut", overwrite: "auto" });
      tween(door.rotation, { y: 0, duration: 0.6, ease: "power2.inOut", overwrite: "auto" });
    }
    paint();
  }

  /** Where a part is on the page (for a keyboard or a test to find it). */
  function where(id) {
    const p = parts.get(id);
    if (!p) return null;
    const r = canvas.getBoundingClientRect();
    const v = centreOf(p).clone().project(camera);
    return { x: r.left + (v.x * 0.5 + 0.5) * r.width, y: r.top + (-v.y * 0.5 + 0.5) * r.height };
  }

  function close() {
    closed = true;
    cancelAnimationFrame(queued);
    cancelAnimationFrame(moveFrame);
    tweens.forEach((t) => t.kill());
    resize.disconnect();
    canvas.removeEventListener("pointerdown", down);
    canvas.removeEventListener("pointermove", move);
    canvas.removeEventListener("pointerup", up);
    canvas.removeEventListener("pointerleave", leave);
    canvas.removeEventListener("wheel", wheel);
    owned.forEach((o) => o.dispose());
    renderer.dispose();
    canvas.remove();
    host.style.background = "";
    screenEl.style.transform = "";
    screenEl.style.visibility = "";
  }

  size();
  update(model);
  // Every shader is compiled in the background (where the driver allows) before the first frame.
  const begin = () => {
    if (closed || ready) return;
    ready = true;
    invalidate();
  };
  if (renderer.compileAsync) renderer.compileAsync(scene, camera).then(begin, begin);
  else begin();
  // Mom's fonts may still be loading: the dots are drawn again once they're in.
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(() => !closed && ((pumpSays = ""), paint()));
  // Arriving: the desk drifts in from a little further back, and the lights come on one by one.
  if (G && !calm()) {
    const z = view.zoom;
    view.zoom = z * 1.24;
    view.pol = HOME.pol - 0.06;
    place();
    tween(view, { zoom: z, pol: HOME.pol, duration: 1.1, ease: "power3.out", overwrite: "auto" });
    let i = 0;
    for (const p of parts.values()) {
      p.glow.scale.setScalar(0.001);
      tween(p.glow.scale, { x: glowSize(p), y: glowSize(p), duration: 0.5, delay: 0.35 + i++ * 0.07, ease: "back.out(2.2)" });
    }
    setTimeout(() => (booted = true), 1200);
  } else {
    booted = true;
    paint();
  }
  return { update, select, where, close };
}

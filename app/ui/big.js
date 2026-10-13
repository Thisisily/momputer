/* BIG MODE: the widget grown into a full window, with your PC on a desk beside it. A monitor
   with mom on its screen (the same live mom as the widget, laid onto the glass), and an open
   tower whose parts you can point at and tap: processor, memory, graphics card, drives, the
   motherboard, the speakers, and the power button for Windows itself. Each part carries a
   little light: green when it's fine, red when something's wrong.

   open(host, screenEl, model, on) draws it into `host`, lays `screenEl` over the monitor, and
   returns { update(model), select(id), where(id), close() }. on.pick(id) when a part is tapped
   (null for the empty desk), on.hover(id, x, y) while pointing.

   Like the widget, it costs nothing at rest: there is no animation loop. A frame is drawn when
   something changes (you look around, point at a part, a short tween runs) and at no other
   time. Drawn with three.js, moved with GSAP. */
import * as THREE from "./vendor/three.module.min.js";

const G = window.gsap;
const calm = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** The scene's colours for a light or a dark room. */
function palette(dark) {
  return dark
    ? { desk: 0x1c1c1c, shell: 0x242424, edge: 0x303030, board: 0x0d0d0d, trace: 0x2c2c2c, metal: 0xbdbdbd, card: 0xd9d9d9, stick: 0xf2f2f2, bezel: 0x2b2b2b, soft: 0x1d1d1d, key: 0x2a2a2a }
    : { desk: 0xffffff, shell: 0xe6e6e6, edge: 0xd2d2d2, board: 0x1d1d1f, trace: 0x3a3a3c, metal: 0x9a9a9a, card: 0x2a2a2a, stick: 0x111111, bezel: 0x121212, soft: 0x1b1b1b, key: 0xf2f2f2 };
}

const LED = { ok: 0x18c75a, warn: 0xe5202b, tip: 0x8c8c8c, none: 0x8c8c8c };

export function open(host, screenEl, model, on = {}) {
  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  } catch {
    return null; // no 3D on this machine: the list of parts still works
  }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  host.appendChild(renderer.domElement);

  const css = getComputedStyle(document.documentElement);
  const lum = (hex) => {
    const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(hex.trim());
    return m ? (parseInt(m[1], 16) * 299 + parseInt(m[2], 16) * 587 + parseInt(m[3], 16) * 114) / 255000 : 1;
  };
  const dark = lum(css.getPropertyValue("--bg") || "#ececec") < 0.5;
  const P = palette(dark);

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 100);
  // Where you look from: around (az) and above (pol) a spot on the desk, nearer or further (zoom).
  const HOME = { tx: 0.1, ty: 1.1, zoom: 1 };
  const view = { az: 0.27, pol: 1.2, tz: 0.25, ...HOME };
  const owned = [];
  const tweens = new Set();

  // ---------------------------------------------------------------- drawing only when needed

  let queued = 0;
  let closed = false;
  const draw = () => {
    queued = 0;
    if (closed) return;
    renderer.render(scene, camera);
    layScreen();
  };
  const invalidate = () => {
    if (!queued && !closed) queued = requestAnimationFrame(draw);
  };
  /** A short move. The view (where you look from) is placed again on every step of its own. */
  const tween = (target, vars) => {
    const step = target === view ? follow : invalidate;
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

  scene.add(new THREE.HemisphereLight(0xffffff, dark ? 0x202020 : 0xbdbdbd, dark ? 1.1 : 1.6));
  const key = new THREE.DirectionalLight(0xffffff, dark ? 2.2 : 1.9);
  key.position.set(-5, 9, 8);
  key.target.position.set(0.5, 1, 0);
  key.castShadow = true;
  key.shadow.mapSize.set(1024, 1024);
  Object.assign(key.shadow.camera, { left: -8, right: 8, top: 8, bottom: -4, near: 1, far: 30 });
  key.shadow.bias = -0.0005;
  key.shadow.normalBias = 0.03;
  scene.add(key, key.target);

  // ---------------------------------------------------------------- building blocks

  const material = (color, extra = {}) => {
    const m = new THREE.MeshStandardMaterial({ color, roughness: 0.82, metalness: 0.02, ...extra });
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
  /** A box whose bottom sits on `y`, centred on x and z. */
  const box = (w, h, d, color, x, y, z, parent = scene, extra) => {
    const m = mesh(new THREE.BoxGeometry(w, h, d), color, extra);
    m.position.set(x, y + h / 2, z);
    parent.add(m);
    return m;
  };
  /** A disc or drum whose round face looks at you (down +Z). */
  const drum = (r, depth, color, x, y, z, parent = scene, extra, sides = 40) => {
    const m = mesh(new THREE.CylinderGeometry(r, r, depth, sides), color, extra);
    m.rotation.x = Math.PI / 2;
    m.position.set(x, y, z);
    parent.add(m);
    return m;
  };
  const group = (parent = scene) => {
    const g = new THREE.Group();
    parent.add(g);
    return g;
  };

  // ---------------------------------------------------------------- parts you can tap

  const parts = new Map(); // id -> { group, light, level, spin: [meshes] }
  const part = (id) => {
    const g = group();
    g.userData.part = id;
    const p = { id, group: g, light: null, level: "none", spin: [] };
    parts.set(id, p);
    return p;
  };
  /** A part's little light: a bead that glows its colour whatever the room's light does. */
  const lamp = (p, x, y, z) => {
    const geo = new THREE.SphereGeometry(0.045, 16, 12);
    const mat = new THREE.MeshBasicMaterial({ color: LED.none });
    owned.push(geo, mat);
    const m = new THREE.Mesh(geo, mat);
    m.position.set(x, y, z);
    p.group.add(m);
    p.light = m;
  };

  // The desk: one rounded tile, with everything on it.
  {
    const [w, d, r, h] = [9.7, 4.9, 0.6, 0.26];
    const [x, y] = [-w / 2, -d / 2];
    const tile = new THREE.Shape();
    tile.moveTo(x + r, y);
    tile.lineTo(x + w - r, y);
    tile.quadraticCurveTo(x + w, y, x + w, y + r);
    tile.lineTo(x + w, y + d - r);
    tile.quadraticCurveTo(x + w, y + d, x + w - r, y + d);
    tile.lineTo(x + r, y + d);
    tile.quadraticCurveTo(x, y + d, x, y + d - r);
    tile.lineTo(x, y + r);
    tile.quadraticCurveTo(x, y, x + r, y);
    const geo = new THREE.ExtrudeGeometry(tile, { depth: h, bevelEnabled: false, curveSegments: 12 });
    geo.rotateX(Math.PI / 2); // flat on the ground, its top at 0
    const desk = mesh(geo, P.desk);
    desk.castShadow = false;
    desk.position.z = 0.45;
    scene.add(desk);
  }

  // The monitor: mom lives on its screen.
  const MX = -1.95;
  const monitor = group();
  drum(0.62, 0.05, P.bezel, MX, 0.025, -0.45, monitor).rotation.x = 0;
  box(0.16, 0.82, 0.1, P.bezel, MX, 0.04, -0.52, monitor);
  box(3.36, 2.3, 0.16, P.bezel, MX, 0.78, -0.4, monitor);
  const SCREEN = { w: 3.06, h: 2.0 };
  const glassGeo = new THREE.PlaneGeometry(SCREEN.w, SCREEN.h);
  const glassMat = new THREE.MeshBasicMaterial({ color: 0xffffff });
  owned.push(glassGeo, glassMat);
  const glass = new THREE.Mesh(glassGeo, glassMat);
  glass.position.set(MX, 0.78 + 1.15, -0.4 + 0.082);
  monitor.add(glass);

  // A keyboard, for the look of the thing.
  box(2.5, 0.07, 0.82, P.key, MX, 0, 1.35);
  box(2.34, 0.02, 0.66, P.edge, MX, 0.07, 1.35).castShadow = false;

  // The speakers, either side of the monitor.
  const sound = part("sound");
  for (const x of [MX - 2.2, MX + 2.2]) {
    box(0.56, 0.96, 0.5, P.soft, x, 0, -0.35, sound.group);
    drum(0.19, 0.03, P.edge, x, 0.34, -0.09, sound.group);
    drum(0.09, 0.03, P.edge, x, 0.72, -0.09, sound.group);
  }
  lamp(sound, MX + 2.2 + 0.19, 0.89, -0.09);

  // The tower, its side open to you.
  const TX = 3.05; // centre
  const TW = 2.7; // width
  const TH = 2.95; // height
  const TD = 1.25; // depth
  const Z0 = -0.85; // its back wall
  const shell = group();
  box(TW, TH, 0.06, P.shell, TX, 0.06, Z0, shell); // back wall
  box(TW, 0.07, TD, P.shell, TX, 0.06, Z0 + TD / 2, shell); // floor
  box(TW, 0.07, TD, P.shell, TX, 0.06 + TH - 0.07, Z0 + TD / 2, shell); // roof
  box(0.07, TH, TD, P.shell, TX - TW / 2 + 0.035, 0.06, Z0 + TD / 2, shell); // left
  box(0.07, TH, TD, P.shell, TX + TW / 2 - 0.035, 0.06, Z0 + TD / 2, shell); // right
  for (const x of [TX - TW / 2 + 0.22, TX + TW / 2 - 0.22]) box(0.22, 0.06, 0.9, P.edge, x, 0, Z0 + TD / 2, shell); // feet
  box(1.05, 0.52, 0.95, P.edge, TX - 0.72, 0.13, Z0 + 0.52, shell); // the power supply

  // The motherboard, on the back wall.
  const board = part("board");
  const BX = TX - 0.28;
  box(1.86, 1.95, 0.04, P.board, BX, 0.82, Z0 + 0.05, board.group);
  for (const [w, h, x, y] of [[0.5, 0.06, -0.55, 2.52], [0.06, 0.6, 0.78, 1.0], [0.72, 0.05, 0.3, 0.98], [0.3, 0.3, 0.6, 2.35]])
    box(w, h, 0.02, P.trace, BX + x, y, Z0 + 0.08, board.group).castShadow = false;
  lamp(board, BX - 0.82, 0.94, Z0 + 0.1);

  // The processor, under its cooler.
  const processor = part("processor");
  const CX = BX - 0.32;
  const CY = 2.12;
  box(0.62, 0.62, 0.3, P.metal, CX, CY - 0.31, Z0 + 0.22, processor.group, { metalness: 0.35, roughness: 0.5 });
  drum(0.34, 0.09, P.soft, CX, CY, Z0 + 0.42, processor.group);
  const fan = group(processor.group);
  fan.position.set(CX, CY, Z0 + 0.475);
  for (let i = 0; i < 7; i++) {
    const blade = box(0.26, 0.05, 0.012, P.edge, 0.15, -0.025, 0, fan);
    blade.castShadow = false;
    const hold = group(fan);
    hold.rotation.z = (i / 7) * Math.PI * 2;
    hold.add(blade);
  }
  drum(0.07, 0.03, P.metal, 0, 0, 0.005, fan);
  processor.spin.push(fan);
  lamp(processor, CX + 0.4, CY + 0.36, Z0 + 0.38);

  // Memory: as many sticks as the PC has.
  const memory = part("memory");
  const sticks = Math.max(1, Math.min(8, model.sticks || 2));
  for (let i = 0; i < sticks; i++) box(0.05, 0.82, 0.22, P.stick, BX + 0.26 + i * 0.1, 1.74, Z0 + 0.18, memory.group);
  lamp(memory, BX + 0.26 + (sticks - 1) * 0.05, 2.63, Z0 + 0.3);

  // The graphics card(s).
  const graphics = part("graphics");
  const cards = Math.max(1, Math.min(2, model.cards || 1));
  for (let c = 0; c < cards; c++) {
    const y = 1.3 - c * 0.36;
    box(1.9, 0.26, 0.74, P.card, BX + 0.02, y, Z0 + 0.44, graphics.group);
    for (const dx of [-0.48, 0.48]) {
      drum(0.1, 0.04, P.soft, BX + 0.02 + dx, y + 0.13, Z0 + 0.82, graphics.group);
      const ring = group(graphics.group);
      ring.position.set(BX + 0.02 + dx, y + 0.13, Z0 + 0.845);
      for (let i = 0; i < 5; i++) {
        const hold = group(ring);
        hold.rotation.z = (i / 5) * Math.PI * 2;
        box(0.085, 0.022, 0.008, P.edge, 0.05, -0.011, 0, hold).castShadow = false;
      }
      graphics.spin.push(ring);
    }
  }
  lamp(graphics, BX + 0.9, 1.5, Z0 + 0.84);

  // Drives: a stick on the board for each NVMe one, boxes in the cage for the rest.
  const disks = part("disks");
  const drives = (model.drives && model.drives.length ? model.drives : ["nvme"]).slice(0, 5);
  let onBoard = 0;
  let inCage = 0;
  for (const kind of drives) {
    if (kind === "nvme") {
      box(0.56, 0.13, 0.03, P.card, BX - 0.45 + onBoard * 0.02, 1.62 - onBoard * 0.19, Z0 + 0.1, disks.group);
      onBoard++;
    } else {
      box(0.92, kind === "hdd" ? 0.26 : 0.1, 0.72, kind === "hdd" ? P.metal : P.card, TX + 0.74, 0.16 + inCage * 0.32, Z0 + 0.48, disks.group, { metalness: kind === "hdd" ? 0.4 : 0.05, roughness: 0.55 });
      inCage++;
    }
  }
  lamp(disks, TX + 0.74 - 0.38, 0.2 + Math.max(inCage, 1) * 0.32, Z0 + 0.86);

  // The power button: Windows itself.
  const windows = part("windows");
  drum(0.11, 0.05, P.edge, TX + TW / 2 - 0.035, TH - 0.32, Z0 + TD + 0.02, windows.group);
  drum(0.06, 0.06, P.metal, TX + TW / 2 - 0.035, TH - 0.32, Z0 + TD + 0.03, windows.group);
  lamp(windows, TX + TW / 2 - 0.035, TH - 0.58, Z0 + TD + 0.03);

  // Thin parts are hard to hit (a tap lands between two memory sticks): each part also gets an
  // unseen box around the whole of it, for the pointer only.
  const proxies = [];
  for (const p of parts.values()) {
    const bounds = new THREE.Box3().setFromObject(p.group);
    const span = bounds.getSize(new THREE.Vector3());
    const geo = new THREE.BoxGeometry(span.x, span.y, Math.max(span.z, 0.05));
    const mat = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0, depthWrite: false });
    owned.push(geo, mat);
    const proxy = new THREE.Mesh(geo, mat);
    bounds.getCenter(proxy.position);
    proxy.userData.part = p.id;
    proxy.userData.proxy = true;
    proxies.push(proxy);
  }

  // Every part's meshes get their own material, so one can light up without the others.
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
    fit = Math.max(5.95 / Math.tan(hfov), 3.0 / Math.tan(v));
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

  // The picked part gets a fine line around every piece of it.
  const lineMat = new THREE.LineBasicMaterial({ color: dark ? 0xffffff : 0x000000 });
  owned.push(lineMat);
  const outlines = new Map();
  const outlineOf = (p) => {
    if (!outlines.has(p)) {
      const lines = [];
      p.group.traverse((o) => {
        if (!o.isMesh || o === p.light) return;
        const geo = new THREE.EdgesGeometry(o.geometry, 30);
        owned.push(geo);
        const l = new THREE.LineSegments(geo, lineMat);
        o.add(l);
        lines.push(l);
      });
      outlines.set(p, lines);
    }
    return outlines.get(p);
  };

  const lit = (p) => (p === chosen ? 0.34 : p === hovered ? 0.16 : 0);
  const paint = () => {
    for (const p of parts.values()) {
      const e = lit(p);
      p.group.traverse((o) => {
        if (o.isMesh && o !== p.light && o.material.emissive) {
          o.material.emissive.setHex(0xffffff);
          o.material.emissiveIntensity = dark ? e : e * 0.6;
        }
      });
      p.light.material.color.setHex(LED[p.level] || LED.none);
      p.light.scale.setScalar(p === chosen ? 1.5 : 1);
      if (p === chosen || outlines.has(p)) for (const l of outlineOf(p)) l.visible = p === chosen;
    }
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
    for (const s of p.spin) tween(s.rotation, { z: s.rotation.z - Math.PI * 4, duration: 1.5, ease: "power2.out", overwrite: "auto" });
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
      view.az = Math.max(-0.7, Math.min(0.85, drag.az - dx * 0.005));
      view.pol = Math.max(0.98, Math.min(1.5, drag.pol - dy * 0.004));
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
    view.zoom = Math.max(0.72, Math.min(1.3, view.zoom + Math.sign(e.deltaY) * 0.06));
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

  /** Picks a part (or none): it lights up, its fans turn, and the view leans toward it. */
  function select(id) {
    chosen = id ? parts.get(id) || null : null;
    if (chosen) {
      whirl(chosen);
      const c = centreOf(chosen);
      tween(view, { tx: HOME.tx + (c.x - HOME.tx) * 0.3, ty: HOME.ty + (c.y - HOME.ty) * 0.25, zoom: 0.93, duration: 0.7, ease: "power3.out", overwrite: "auto" });
    } else {
      tween(view, { ...HOME, duration: 0.7, ease: "power3.out", overwrite: "auto" });
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
    screenEl.style.transform = "";
    screenEl.style.visibility = "";
  }

  size();
  update(model);
  // Arriving: the desk drifts in from a little further back.
  if (G && !calm()) {
    const z = view.zoom;
    view.zoom = z * 1.22;
    place();
    tween(view, { zoom: z, duration: 0.9, ease: "power3.out", overwrite: "auto" });
  }
  return { update, select, where, close };
}

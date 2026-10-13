/* momputer widget. Plain JS, no build step; the libraries it uses are copied into vendor/.
   Pages are re-drawn by morphing the DOM (idiomorph), so a button never disappears from under
   your pointer when fresh numbers arrive, and motion is GSAP. */
(() => {
  "use strict";

  const T = window.__TAURI__;
  const bridge = T ? { invoke: T.core.invoke, listen: T.event.listen } : window.momMock;
  const invoke = (cmd, args) => bridge.invoke(cmd, args);
  const G = window.gsap;
  // The widget should cost nothing while you're not looking at it. 60 frames a second is plenty
  // (GSAP would draw at the screen's rate, 240 on some), and once the last tween ends GSAP stops
  // asking for frames within a sixth of a second instead of after two.
  if (G) {
    G.ticker.fps(60);
    G.config({ autoSleep: 10 });
    // Sitting on the desktop while you work elsewhere, half that is plenty for a blink.
    addEventListener("blur", () => G.ticker.fps(30));
    addEventListener("focus", () => G.ticker.fps(60));
  }
  const appWindow = T && T.window && T.window.getCurrentWindow ? T.window.getCurrentWindow() : null;
  const calm = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  // No animation when it can't be seen: a hidden window doesn't run them, and nothing should wait on one.
  const moving = () => G && !calm() && !document.hidden;

  const state = {
    snap: null,
    tab: "running",
    runningTab: "agents", // Running's two views: "agents" or "speed"
    speed: null, // what's slowing the PC down (get_speed)
    speedAt: 0,
    speedLoading: false,
    programs: null, // every program running (get_programs)
    programsAt: 0,
    programsLoading: false,
    progView: { sort: "cpu", q: "", rest: false }, // how Programs is sorted and filtered, and whether Windows' own show
    progOpen: null, // the program whose details are open
    progConfirm: null, // "id|action" waiting for its second press
    progOrder: null, // the order rows are shown in, kept between refreshes so nothing moves under the pointer
    progOrderSort: null,
    filesPlaces: null, // where the Files scan looks (files_places)
    browsers: null, // browsers, profiles and extensions (get_browsers)
    browsersAt: 0,
    browsersLoading: false,
    browserOpen: null, // "browser|profile" open to its extensions
    extOff: new Set(), // profiles whose switched-off extensions are shown
    regFixed: new Map(), // registry fixes done just now: id -> what they were, for Undo
    regConfirm: null,
    regBusy: null,
    progBusy: null, // { id, action } while mom does it
    system: null, // the PC, its protection, updates and registry (get_system)
    systemLoading: false,
    startup: null, // what starts with Windows (get_startup)
    startupAt: 0,
    startupBusy: null,
    update: null, // is there a newer momputer (get_update)
    updateBusy: false,
    updateTold: null, // the version mom has already mentioned this time
    big: false, // BIG MODE: the full window, with your PC on mom's desk
    bigPart: null, // the part of the PC that's picked there
    stopped: [], // servers mom stopped and can start again (get_stopped)
    startBusy: null, // the one starting now
    icons: new Map(), // .exe path -> its icon (a data: URI), or null when it has none (get_icons)
    more: new Set(), // long explanations that have been opened ("More")
    expanded: new Set(),
    stopping: new Set(),
    done: new Set(),
    confirm: null,
    confirmTimer: null,
    showHelpers: false,
    trash: { spots: [], busy: false },
    health: null,
    healthAt: 0,
    sheet: null, // { kind: "spot", id } or { kind: "settings" }
    picks: {}, // clean-up id -> what's ticked in its lists (see pickOf)
    cleanConfirm: null,
    cleaning: null,
    archive: null, // where old conversations go, and what's been moved there
    adminBlock: undefined, // why mom can't ask for an administrator from this copy (null: she can)
    bringing: null, // the moved folder being brought back
    deleteConfirm: null, // the moved folder whose Delete was pressed once
    uninstallConfirm: null, // the app whose Uninstall was pressed once
    uninstalled: new Set(), // apps whose uninstaller was opened
    showAllUnused: new Set(), // which of the unused lists (apps, games) are shown in full
    showAllStartup: false,
    upkeepTab: "storage", // storage | apps | windows | files
    rulesTab: "rules", // House rules' two views: rules | skills
    agents: null, // how each agent is set up (get_agents); null until mom has looked
    agentsLoading: false,
    agentsChecking: false, // Claude Code is trying its servers because "Check now" was pressed
    agentsAt: 0,
    showAllAgent: new Set(), // "<agent>:<list>": the sheet's lists shown in full
    reader: null, // the file open in BIG MODE's reader: { id, title, doc, error }
    readerWasSmall: false, // BIG MODE was switched on just to read, so closing goes back
    listView: loadView(), // how a clean-up's lists are shown: sort, grouped, filter
    groupOpen: {}, // "<spot>:<list>:<group>" -> open or shut, where the user changed it
    doesOpen: false, // the full text under "What the button does"
    live: null, // how the clean-up running now is going (from clean_progress)
    gone: new Set(), // what the clean-up running now has removed, by path
    files: null, // { report, busy }: copies and big forgotten files
    filesProgress: null,
    filesKeep: {}, // a set of copies' hash -> the copy to keep instead of mom's pick
    filesUntick: new Set(), // extra copies the user wants to keep as well
    bigTicked: new Set(), // big files the user ticked
    filesConfirm: false,
    filesWorking: false,
    showAllCopies: false,
    winsettings: null, // Windows settings that drifted from the defaults
    winLoading: false,
    winBusy: null, // the setting being put back
    winConfirm: null,
    winFixed: [], // settings put back this session that can be undone: { id, title }
    roomPaths: [],
    rooms: {},
    roomOpen: null,
    roomSel: {},
    roomBusy: false,
    roomErrors: {}, // rooms mom can't tidy, and why (a drive, Windows, gone)
    knocked: false, // knocked on mom's door
    version: "",
    tipCoin: "Solana", // which address the tip jar shows
    recycle: loadRecycle(), // clean-ups that delete send things to the Recycle Bin instead
    rules: null,
    rulesDraft: null,
    rulesEditing: false,
    settings: null,
    paused: false,
    lastFace: null,
    lastSnapAt: 0,
    renderFailed: false,
    say: null,
    ready: false,
    lastCount: null,
  };

  /** How lists are shown, kept between sessions (in the webview's own storage). */
  function loadView() {
    try {
      const v = JSON.parse(localStorage.getItem("mom.listView") || "{}");
      return { sort: ["size", "age", "name"].includes(v.sort) ? v.sort : "size", group: v.group !== false, q: "" };
    } catch {
      return { sort: "size", group: true, q: "" };
    }
  }
  function saveView() {
    try {
      localStorage.setItem("mom.listView", JSON.stringify({ sort: state.listView.sort, group: state.listView.group }));
    } catch {}
  }

  function loadRecycle() {
    try {
      return localStorage.getItem("mom.recycle") === "1";
    } catch {
      return false;
    }
  }

  function saveRecycle() {
    try {
      localStorage.setItem("mom.recycle", state.recycle ? "1" : "0");
    } catch {}
  }

  // ------------------------------------------------------------ little helpers

  const $ = (sel) => document.querySelector(sel);
  const esc = (s) =>
    String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const plural = (n, one, many = `${one}s`) => `${n.toLocaleString("en-US")} ${n === 1 ? one : many}`;
  const hash = (s) => {
    let h = 5381;
    for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
    return (h >>> 0).toString(36);
  };

  function bytes(b) {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let v = b || 0;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    if (i === 0) return `${Math.round(v)} B`;
    return v >= 100 || i <= 2 ? `${Math.round(v)} ${units[i]}` : `${v.toFixed(1)} ${units[i]}`;
  }

  function age(s) {
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    if (s < 172800) return `${Math.floor(s / 3600)}h`;
    return `${Math.floor(s / 86400)}d`;
  }

  /** "2 years", "a year", or days under that. */
  function yearsAgo(s) {
    const days = Math.max(0, s) / 86400;
    if (days >= 730) return `${Math.floor(days / 365)} years`;
    if (days >= 365) return "a year";
    return ageLong(Math.max(0, s));
  }

  function ageLong(s) {
    if (s < 60) return "a moment";
    if (s < 3600) return plural(Math.floor(s / 60), "minute");
    if (s < 172800) return plural(Math.floor(s / 3600), "hour");
    return plural(Math.floor(s / 86400), "day");
  }

  const clock = (ts) => {
    const d = new Date(ts * 1000);
    const sameDay = d.toDateString() === new Date().toDateString();
    const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
    return sameDay ? time : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
  };

  // Icons from Lucide (ISC).
  const icon = (paths) => `<svg class="ico" viewBox="0 0 24 24" aria-hidden="true">${paths}</svg>`;
  const ICON = {
    chevron: icon('<path d="m9 18 6-6-6-6"/>'),
    folder: icon('<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>'),
    close: icon('<path d="M18 6 6 18"/><path d="m6 6 12 12"/>'),
    out: icon('<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>'),
    refresh: icon('<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>'),
    trash: icon('<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/>'),
    copy: icon('<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>'),
    undo: icon('<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>'),
    archive: icon('<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>'),
    wrench: icon('<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>'),
    shield: icon('<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/>'),
    eye: icon('<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>'),
    lock: icon('<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>'),
    key: icon('<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"/><circle cx="16.5" cy="7.5" r=".5"/>'),
    check: icon('<path d="M20 6 9 17l-5-5"/>'),
    search: icon('<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>'),
    layers: icon('<path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z"/><path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65"/><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"/>'),
    stop: icon('<rect width="14" height="14" x="5" y="5" rx="2"/>'),
    image: icon('<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>'),
    film: icon('<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18"/><path d="M3 7.5h4"/><path d="M3 12h18"/><path d="M3 16.5h4"/><path d="M17 3v18"/><path d="M17 7.5h4"/><path d="M17 16.5h4"/>'),
    music: icon('<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>'),
    doc: icon('<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M16 13H8"/><path d="M16 17H8"/>'),
    box: icon('<path d="M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z"/><path d="M12 22V12"/><path d="m3.3 7 7.703 4.734a2 2 0 0 0 1.994 0L20.7 7"/>'),
    download: icon('<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/>'),
    file: icon('<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>'),
  };

  const CATEGORY = {
    "dev-server": "dev server",
    server: "server",
    watcher: "watcher",
    "headless-browser": "headless browser",
    "mcp-server": "MCP server",
    "test-runner": "test run",
    tunnel: "tunnel",
    database: "database",
    "log-follower": "log tail",
    sleeper: "sleep loop",
    build: "build",
    "media-job": "render",
    vm: "VM",
    repl: "REPL",
    script: "script",
    shell: "shell loop",
  };

  const NICK = {
    "claude-code": "Claude",
    codex: "Codex",
    cursor: "Cursor",
    gemini: "Gemini",
    copilot: "Copilot",
    windsurf: "Windsurf",
    goose: "Goose",
    amp: "Amp",
    opencode: "OpenCode",
    aider: "Aider",
    pi: "Pi",
    openclaw: "OpenClaw",
    hermes: "Hermes",
    other: "Some agent",
  };

  const isLeft = (g) => (g.status === "left-behind" || g.status === "orphaned") && !g.kept;
  const placeName = (p) => (p === "~" ? "your home folder" : p);

  /** Re-draw a region by morphing, so focus, hover and half-finished clicks survive. */
  function morph(el, html) {
    if (window.Idiomorph) {
      window.Idiomorph.morph(el, html, {
        morphStyle: "innerHTML",
        // Leave alone a field someone is typing in. (Not every focused element: a button you
        // just pressed still has to show what it's doing.)
        callbacks: { afterNodeAdded: entered, beforeNodeMorphed: (old) => !beingTyped(old) },
      });
    } else {
      el.innerHTML = html;
    }
  }

  const beingTyped = (node) =>
    node === document.activeElement && node instanceof Element && node.matches("textarea, select, input:not([type=checkbox], [type=radio], [type=button])");

  function entered(node) {
    if (!state.ready || !moving() || !(node instanceof Element) || !node.matches(".row, .check-row, .item")) return;
    G.from(node, { opacity: 0, y: 6, duration: 0.32, ease: "power2.out", clearProps: "opacity,transform" });
  }

  let toastTimer = null;
  function toast(text, isError = false) {
    const el = $("#toast");
    el.textContent = text;
    el.classList.toggle("is-error", isError);
    el.hidden = false;
    if (moving()) G.fromTo(el, { y: 24, opacity: 0 }, { y: 0, opacity: 1, duration: 0.45, ease: "back.out(1.6)" });
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      if (moving()) G.to(el, { y: 16, opacity: 0, duration: 0.25, ease: "power2.in", onComplete: () => (el.hidden = true) });
      else el.hidden = true;
    }, isError ? 6500 : 4200);
  }

  async function act(promise, okMessage) {
    try {
      const r = await promise;
      const msg = okMessage ? okMessage(r) : r && r.message ? r.message : typeof r === "string" ? r : null;
      if (msg) toast(msg);
      return r ?? true;
    } catch (e) {
      toast(String(e), true);
      return null;
    }
  }

  // ------------------------------------------------------------ mom

  let mom = null;
  let reaction = null;

  /** Mom pulls a face for a moment when you do something, then goes back to how she feels. */
  function react(face, ms = 2600) {
    reaction = { face, until: Date.now() + ms };
    renderNote();
    setTimeout(renderNote, ms + 50);
  }

  /** And sometimes she says something about it. */
  function say(text, ms = 2600) {
    state.say = { text, until: Date.now() + ms };
    renderNote();
    setTimeout(renderNote, ms + 50);
  }

  const cpuAlert = () => (state.settings ? state.settings.config.ui.cpuAlertPercent : 25);
  const melting = (s) => s.summary.leftoverCpu >= cpuAlert();
  const lookedAgo = () => (state.snap ? Math.max(0, Math.floor(Date.now() / 1000) - state.snap.takenAt) : 0);
  const stale = () => !state.paused && !!state.snap && lookedAgo() > 20;

  /** Mostly she smiles, "c:". A thoughtful look once a lot piles up, and a worried one while
      leftovers keep the processor busy. Never a cross one: she helps, she doesn't tell anyone off.
      The light by her name says if she's stuck. */
  function faceNow() {
    const s = state.snap;
    if (state.paused) return "asleep";
    if (state.renderFailed) return "dizzy";
    if (reaction && Date.now() < reaction.until) return reaction.face;
    if (!s) return "look";
    if (melting(s)) return "melting";
    switch (s.mood.face) {
      case "helping":
        return s.mood.level >= 3 ? "hmm" : "look";
      case "busy":
        return "hmm";
      default:
        return "look";
    }
  }

  function showFace() {
    const f = faceNow();
    if (!mom || f === state.lastFace) return;
    mom.set(f);
    state.lastFace = f;
  }

  // Boop her and she answers. Boop her a lot and she gets dizzy.
  const BOOPS = ["Hi, sweetie.", "That tickles.", "Yes, dear?", "I'm working here.", "Boop to you too.", "Did you eat today?", "Sit up straight."];
  let boops = [];
  function boop() {
    const now = Date.now();
    boops = boops.filter((t) => now - t < 2500).concat(now);
    if (mom) mom.boop();
    if (boops.length >= 5) {
      boops = [];
      react("dizzy", 2200);
      say("Okay, okay. I'm dizzy.", 2200);
    } else {
      react("giggle", 1300);
      say(BOOPS[Math.floor(Math.random() * BOOPS.length)], 2000);
    }
  }

  // ------------------------------------------------------------ the note at the top

  let shownCount = null;
  function setBig(n) {
    const el = $("#big");
    const draw = (v) => {
      el.innerHTML = `<span class="dot big-num">${v}</span><span class="led ${v ? "red" : "green"}"></span>`;
    };
    if (shownCount === null || !moving()) {
      shownCount = n;
      draw(n);
      return;
    }
    if (n === shownCount) return;
    const o = { v: shownCount };
    shownCount = n;
    G.to(o, { v: n, duration: Math.min(1.1, 0.25 + Math.abs(n - o.v) * 0.06), ease: "power2.out", onUpdate: () => draw(Math.round(o.v)) });
  }

  function renderNote() {
    const s = state.snap;
    showFace();
    const said = state.say && Date.now() < state.say.until ? state.say.text : null;
    if (!s) {
      if (said) $("#mom-line").textContent = said;
      return;
    }
    $("#mom-line").textContent = said || (state.paused ? "I'm on a break. Nobody's watching." : s.mood.line);
    const n = s.summary.leftovers;
    const was = state.lastCount;
    state.lastCount = n;
    if (was > 0 && n === 0) react("proud", 3200);
    setBig(n);
    $("#big-label").textContent = "left running";
    const actions = $("#note-actions");
    actions.hidden = n === 0;
    if (n > 0) {
      if (!$("#btn-clean").disabled) $("#clean-label").textContent = `Clean up ${plural(n, "leftover")}`;
      const cpu = s.summary.leftoverCpu;
      $("#note-sub").textContent = `Turns off ${plural(n, "program")} agents left running: ${bytes(s.summary.leftoverMem)} of memory${cpu >= 1 ? `, ${Math.round(cpu)}% CPU` : ""}. No files are deleted.`;
    }
    const alarm = $("#alarm");
    alarm.hidden = !melting(s);
    if (melting(s)) alarm.innerHTML = `<span class="led red blink"></span>Leftovers are using ${Math.round(s.summary.leftoverCpu)}% of your CPU.`;
    const count = n > 0 ? String(n) : "";
    if ($("#count-running").textContent !== count) {
      $("#count-running").textContent = count;
      moveThumb(false);
    }
  }

  // ------------------------------------------------------------ running

  function metaText(g) {
    const nick = NICK[g.owner.agent] || "Someone";
    const cat = CATEGORY[g.category] || g.category;
    const place = g.project ? ` in ${placeName(g.project)}` : "";
    switch (g.status) {
      case "left-behind":
        return `${nick} left this ${cat}${place}`;
      case "orphaned":
        return `A ${cat}${place} nobody's watching`;
      case "forgotten":
        return `${nick}'s ${cat}${place}, quiet for ${ageLong(g.age)}`;
      case "running":
      case "helper":
        return `${nick}'s ${cat}${place}`;
      default:
        return `Your ${cat}${place}`;
    }
  }

  function detail(g) {
    const session = g.owner.session ? `, session ${esc(g.owner.session.slice(0, 8))}` : "";
    const ports = g.ports.length ? g.ports.map((p) => `:${p.port}${p.public ? " (reachable from your network)" : ""}`).join(", ") : "none";
    const open = g.ports[0] ? `<button class="link-btn" data-act="open" data-port="${g.ports[0].port}">Open :${g.ports[0].port}</button>` : "";
    return `<dl>
        <dt>Why</dt><dd>${esc(g.why)}</dd>
        <dt>Command</dt><dd><code>${esc(g.command)}</code></dd>
        ${g.cwd ? `<dt>Folder</dt><dd>${esc(g.cwd)}</dd>` : ""}
        <dt>Started by</dt><dd>${esc(g.owner.label)}${session}${g.owner.guessed ? " (worked out from folder and timing)" : ""}</dd>
        <dt>Processes</dt><dd class="num">${g.pids.length} (main PID ${g.rootPid})</dd>
        <dt>Listening</dt><dd>${ports}</dd>
        <dt>Stop does</dt><dd>Ends ${g.pids.length === 1 ? "this process" : `these ${g.pids.length} processes`}, children first. Files and folders aren't touched.</dd>
      </dl>
      <div class="detail-actions">
        ${open}
        ${g.cwd ? `<button class="link-btn" data-act="reveal">Show folder</button>` : ""}
        <button class="link-btn" data-act="keep">${g.kept ? "Stop keeping it" : "Keep it running"}</button>
      </div>`;
  }

  function row(g, opts = {}) {
    const cls = ["row"];
    if (isLeft(g)) cls.push("is-left");
    if (state.expanded.has(g.id)) cls.push("is-open");
    if (state.stopping.has(g.id)) cls.push("is-stopping");
    if (state.done.has(g.id)) cls.push("is-done");
    const ports = g.ports
      .slice(0, 3)
      .map((p) => `<button class="port" data-act="open" data-port="${p.port}" title="Open localhost:${p.port}">:${p.port}</button>`)
      .join("");
    const badges =
      (g.dupes ? `<span class="badge" title="Same command, same folder">×${g.dupes + 1}</span>` : "") +
      (g.kept ? `<span class="badge kept">kept</span>` : "") +
      (g.owner.guessed ? `<span class="badge guess" title="Worked out from the folder and timing">probably</span>` : "");
    const cpu = g.cpu >= 0.5 ? `<span class="${g.cpu >= 10 ? "hot" : ""}">${g.cpu < 10 ? g.cpu.toFixed(1) : Math.round(g.cpu)}%</span>&nbsp; ` : "";
    let button = "";
    if (!opts.noStop) {
      const label = state.stopping.has(g.id) ? "Stopping" : state.confirm === g.id ? "Sure?" : "Stop";
      const what = `Turns off ${g.name}${g.pids.length > 1 ? ` and its ${plural(g.pids.length - 1, "helper process", "helper processes")}` : ""}. Nothing is deleted.`;
      button = `<button class="stop${state.confirm === g.id ? " confirm" : ""}" data-act="stop" title="${esc(what)}" ${state.stopping.has(g.id) ? "disabled" : ""}>${label}</button>`;
    }
    // The little light: red when nobody needs it, green when someone does, dark when it's just been sitting there.
    const led = isLeft(g) ? "red" : g.status === "forgotten" && !g.kept ? "off" : "green";
    return `<div class="${cls.join(" ")}" id="g-${hash(g.id)}" data-id="${esc(g.id)}" data-led="${led}" style="${opts.delay ? `transition-delay:${opts.delay}ms` : ""}">
      <button class="row-name" data-act="expand" aria-expanded="${state.expanded.has(g.id)}" title="${esc(g.command)}">${esc(g.name)}</button>
      <div class="row-meta"><span class="why">${esc(metaText(g))}</span>${ports}${badges}</div>
      <div class="row-nums num">${bytes(g.mem)}</div>
      <div class="row-nums2 num">${cpu}${age(g.age)}</div>
      <div class="row-act">${button}</div>
      <div class="row-detail">${state.expanded.has(g.id) ? detail(g) : ""}</div>
    </div>`;
  }

  function section(key, title, aside, body, extra = "") {
    return `<div class="section" id="sec-${key}"><div class="section-head ${extra}"><h2>${esc(title)}</h2>${aside ? `<span class="aside num">${aside}</span>` : ""}</div><div class="group">${body}</div></div>`;
  }

  const total = (list) => bytes(list.reduce((a, g) => a + g.mem, 0));

  const RUNNING_TABS = [
    ["agents", "Agents"],
    ["speed", "Speed"],
    ["programs", "Programs"],
    ["startup", "Startup"],
    ["browsers", "Browsers"],
  ];

  function runningTabs() {
    const warns = state.speed ? state.speed.findings.filter((f) => f.level === "warn").length : 0;
    const dot = (k) => (k === "speed" && warns ? `<span class="count num">${warns}</span>` : "");
    return `<div class="subtabs" role="tablist" aria-label="Running">${RUNNING_TABS.map(
      ([k, label]) => `<button class="subtab" role="tab" data-act="running-tab" data-tab="${k}" aria-selected="${state.runningTab === k}">${label}${dot(k)}</button>`
    ).join("")}</div>`;
  }

  function renderRunning() {
    const view = { speed: renderSpeed, programs: renderPrograms, startup: renderStartup, browsers: renderBrowsers }[state.runningTab] || renderAgents;
    return runningTabs() + view();
  }

  /** Opens one of Running's views, loading what it shows if it's stale. */
  function openRunningTab(tab) {
    state.runningTab = tab;
    if (tab === "agents" && Date.now() - state.agentsAt > 60000) loadAgents();
    if (tab === "speed" && Date.now() - state.speedAt > 10000) loadSpeed();
    if (tab === "programs") {
      state.progOrder = null;
      if (Date.now() - state.programsAt > 3000) loadPrograms();
    }
    if (tab === "startup" && Date.now() - state.startupAt > 15000) loadStartup();
    if (tab === "browsers" && Date.now() - state.browsersAt > 60000) loadBrowsers();
  }

  // ------------------------------------------------------------ running: speed

  /** A long explanation, kept short: its first sentence, with the rest behind "More". */
  function brief(key, text) {
    const t = String(text || "").trim();
    const end = t.search(/[.!?:](\s|$)/);
    if (t.length <= 120 || end < 0 || end + 2 >= t.length) return `<span>${esc(t)}</span>`;
    const open = state.more.has(key);
    const first = esc(t.slice(0, end + 1));
    const rest = esc(t.slice(end + 1).trim());
    return `<span>${first}${open ? ` ${rest}` : ""} <button class="more-btn" data-act="more" data-key="${esc(key)}" aria-expanded="${open}">${open ? "Less" : "More"}</button></span>`;
  }

  /** One thing mom found: its light, what it is in a sentence, a line for each thing it's about
      (a sound output, say, and whether it can go), and every button that does something about
      it. `extra` goes under the words (a chart). */
  function checkRow(c, prefix, extra = "") {
    const buttons = [c.fix, ...(c.more || [])].filter(Boolean).map(fixButton).join("");
    const items = (c.items || [])
      .map((i) => `<div class="check-item" data-led="${esc(i.led)}"><b>${esc(i.name)}</b><span>${esc(i.note)}</span></div>`)
      .join("");
    return `<div class="check-row${buttons || items ? " stack" : ""}" id="${prefix}-${hash(c.id)}" data-led="${c.level === "warn" ? "red" : c.level === "ok" ? "green" : "off"}">
        <div class="check-text"><strong>${esc(c.title)}</strong>${brief(`${prefix}:${c.id}`, c.detail)}${extra}</div>
        ${items ? `<div class="check-items">${items}</div>` : ""}
        ${buttons ? `<div class="fix-row">${buttons}</div>` : ""}
      </div>`;
  }

  /** The one thing to do about a finding: copy a page only the browser can open, open Windows
      Settings, or show another part of the widget. */
  function fixButton(f) {
    if (f.kind === "copy")
      return `<button class="secondary" data-act="copy-fix" data-text="${esc(f.value)}" data-hint="${esc(f.hint || "Copied.")}">${ICON.copy}${esc(f.label)}</button>`;
    if (f.kind === "open") return `<button class="secondary" data-act="open-uri" data-url="${esc(f.value)}">${esc(f.label)}${ICON.out}</button>`;
    if (f.kind === "tab") return `<button class="secondary" data-act="go-tab" data-to="${esc(f.value)}">${esc(f.label)}</button>`;
    if (f.kind === "read") return `<button class="secondary" data-act="open-doc" data-id="${esc(f.value)}">${esc(f.label)}</button>`;
    if (f.kind === "launch") return `<button class="secondary" data-act="open-program" data-url="${esc(f.value)}">${esc(f.label)}${ICON.out}</button>`;
    if (f.kind === "regfix") {
      const armed = state.regConfirm === f.value;
      const busy = state.regBusy === f.value;
      return `<div class="fix-stack"><button class="secondary${armed ? " sure" : ""}" data-act="reg-fix" data-id="${esc(f.value)}" ${busy ? "disabled" : ""}>${ICON.trash}${busy ? "Removing…" : armed ? "Sure? Remove" : esc(f.label)}</button>
        ${f.hint ? `<span class="fix-warn"><span class="led green"></span>${esc(f.hint)}</span>` : ""}</div>`;
    }
    if (f.kind === "end") {
      const armed = state.progConfirm === `${f.value}|end`;
      const busy = state.progBusy && state.progBusy.id === f.value;
      return `<button class="secondary${armed ? " sure" : ""}" data-act="prog-act" data-id="${esc(f.value)}" data-action="end" data-sure="1" ${busy ? "disabled" : ""}>${ICON.stop}${busy ? "Ending…" : armed ? "Sure? End it" : esc(f.label)}</button>`;
    }
    if (f.kind === "uninstall") {
      const armed = state.uninstallConfirm === f.value;
      const opened = state.uninstalled.has(f.value);
      return `<button class="secondary${armed ? " sure" : ""}" data-act="uninstall-app" data-key="${esc(f.value)}" ${opened ? "disabled" : ""}>${ICON.trash}${opened ? "Uninstaller opened" : armed ? "Sure? Uninstall" : esc(f.label)}</button>`;
    }
    if (f.kind === "program") {
      const armed = state.progConfirm === `${f.value}|restart`;
      const busy = state.progBusy && state.progBusy.id === f.value;
      return `<div class="fix-stack"><button class="secondary${armed ? " sure" : ""}" data-act="prog-act" data-id="${esc(f.value)}" data-action="restart" data-sure="1" ${busy ? "disabled" : ""}>${ICON.refresh}${busy ? "Restarting…" : armed ? "Sure? Restart it" : esc(f.label)}</button>
        ${armed && f.hint ? `<span class="fix-warn"><span class="led red"></span>${esc(f.hint)}</span>` : ""}</div>`;
    }
    return "";
  }

  /** When it happened, on a line of the last two weeks: a red dot for each crash, a tall tick
      for each update, days marked underneath. Point at a mark for its time. */
  function timeline(t) {
    const W = 320, L = 6, R = 6, Y = 30;
    const x = (at) => L + ((at - t.from) / Math.max(1, t.to - t.from)) * (W - L - R);
    const when = (at) => new Date(at * 1000).toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
    const days = Math.round((t.to - t.from) / 86400);
    const ticks = Array.from({ length: days + 1 }, (_, i) => `<line x1="${x(t.to - i * 86400).toFixed(1)}" x2="${x(t.to - i * 86400).toFixed(1)}" y1="${Y - 2}" y2="${Y + 2}" class="tl-tick"/>`).join("");
    const marks = t.marks
      .map((m) =>
        m.kind === "crash"
          ? `<circle cx="${x(m.at).toFixed(1)}" cy="${Y}" r="4" class="tl-crash"><title>Crashed ${esc(when(m.at))}</title></circle>`
          : `<g><line x1="${x(m.at).toFixed(1)}" x2="${x(m.at).toFixed(1)}" y1="13" y2="${Y + 6}" class="tl-update"/><text x="${x(m.at).toFixed(1)}" y="9" text-anchor="${x(m.at) > W / 2 ? "end" : "start"}" class="tl-label">updated</text><title>Driver updated ${esc(when(m.at))}</title></g>`
      )
      .join("");
    // Crashes too close to tell apart get a count above them.
    const groups = [];
    for (const m of t.marks.filter((m) => m.kind === "crash")) {
      const g = groups[groups.length - 1];
      if (g && x(m.at) - x(g[g.length - 1].at) < 14) g.push(m);
      else groups.push([m]);
    }
    const counts = groups
      .filter((g) => g.length > 1)
      .map((g) => {
        const cx = (x(g[0].at) + x(g[g.length - 1].at)) / 2;
        return `<text x="${Math.min(Math.max(cx, L + 20), W - R - 20).toFixed(1)}" y="${Y - 9}" text-anchor="middle" class="tl-label tl-count">${g.length} crashes</text>`;
      })
      .join("");
    const crashes = t.marks.filter((m) => m.kind === "crash").length;
    const updates = t.marks.length - crashes;
    const label = `${plural(crashes, "crash", "crashes")}${updates ? ` and ${plural(updates, "driver update")}` : ""} in the last ${days} days`;
    return `<svg class="timeline" viewBox="0 0 ${W} 50" role="img" aria-label="${esc(label)}">
        <line x1="${L}" x2="${W - R}" y1="${Y}" y2="${Y}" class="tl-base"/>${ticks}${marks}${counts}
        <text x="${L}" y="47" class="tl-label">${days} days ago</text><text x="${W - R}" y="47" text-anchor="end" class="tl-label">today</text>
      </svg>`;
  }

  function renderSpeed() {
    const sp = state.speed;
    const again = `<button class="secondary" data-act="speed-again" ${state.speedLoading ? "disabled" : ""}>${state.speedLoading ? '<span class="led green blink"></span>Looking' : "Look again"}</button>`;
    if (!sp) return `<div class="lead"><div class="lead-text"><p>Looking for what slows your PC down…</p></div>${again}</div>`;
    const order = { warn: 0, tip: 1, ok: 2 };
    const findings = [...sp.findings].sort((a, b) => (order[a.level] ?? 1) - (order[b.level] ?? 1));
    const warns = findings.filter((f) => f.level === "warn").length;
    const said = warns ? `${warns === 1 ? "thing is" : "things are"} slowing your PC down.` : "Nothing's slowing your PC down right now.";
    let html = `<div class="lead"><div class="lead-text">${warns ? `<div class="big"><span class="dot free-num">${warns}</span></div>` : ""}<p>${said}</p></div>${again}</div>`;
    if (findings.length) {
      html += section(
        "slow",
        "What mom found",
        "",
        findings.map((f) => checkRow(f, "spd", f.timeline ? timeline(f.timeline) : "")).join("")
      );
    }
    const rows = [];
    const g = sp.gpu;
    if (g) {
      const bits = [g.temp != null ? `${g.temp} °C` : "", g.powerLimit ? `power limit ${Math.round(g.powerLimit)} W` : ""].filter(Boolean).join(" · ");
      rows.push(`<div class="row" id="spd-gpu" data-led="green" style="grid-template-areas:'name nums act' 'meta nums2 act'">
          <span class="row-name">${esc(g.name)}</span>
          <div class="row-meta"><span class="why">Graphics card${bits ? ` · ${esc(bits)}` : ""}</span></div>
          <div class="row-nums num">${g.busy != null ? `${g.busy}%` : ""}</div>
          <div class="row-nums2">${g.busy != null ? "busy" : ""}</div>
          <div class="row-act"></div>
        </div>`);
    }
    for (const b of sp.busy) {
      rows.push(`<div class="row" id="spd-b-${hash(b.name)}" data-led="${b.cpu >= 10 ? "red" : "off"}" style="grid-template-areas:'name nums act' 'meta nums2 act'">
          <span class="row-name">${esc(b.name)}</span>
          <div class="row-meta"><span class="why">${plural(b.count, "process", "processes")} · ${bytes(b.mem)}</span></div>
          <div class="row-nums num"><span class="${b.cpu >= 10 ? "hot" : ""}">${b.cpu < 10 ? b.cpu.toFixed(1) : Math.round(b.cpu)}%</span></div>
          <div class="row-nums2">of the CPU</div>
          <div class="row-act"></div>
        </div>`);
    }
    html += section("busy", "Busy right now", "", rows.join("") || `<p class="empty">Nothing's busy. Your PC is resting.</p>`);
    return html;
  }

  /** Looks at what slows the PC down. The busy list changes by the second; the slow parts (the
      event log, installed tools) are read again at most every five minutes, or when asked. */
  async function loadSpeed(fresh = false) {
    if (state.speedLoading) return;
    state.speedLoading = true;
    if (state.tab === "running" && state.runningTab === "speed") render();
    try {
      state.speed = await invoke("get_speed", { fresh });
      state.speedAt = Date.now();
    } catch (e) {
      console.warn("no speed", e);
    }
    state.speedLoading = false;
    if (state.tab === "running") render();
  }

  // ------------------------------------------------------------ running: programs

  const ACTION = {
    restart: ["Restart", "Restarting…"],
    quit: ["Quit", "Quitting…"],
    end: ["End", "Ending…"],
  };
  const HANDS_OFF = {
    windows: "Part of Windows. Mom never stops it.",
    security: "It keeps your PC safe. Mom never stops it.",
    service: "It runs as a Windows service. Stop it from Windows' Services if you must.",
    agent: "An agent's work, or something an agent started. It's in the Agents tab.",
    mom: "That's me, sweetie.",
  };
  const PROG_SORT = {
    cpu: (a, b) => (b.avgSecs ? b.avgCpu : b.cpu) - (a.avgSecs ? a.avgCpu : a.cpu) || b.mem - a.mem,
    mem: (a, b) => b.mem - a.mem,
    gpu: (a, b) => (b.gpu || 0) - (a.gpu || 0) || b.mem - a.mem,
    name: (a, b) => a.name.localeCompare(b.name),
  };

  const ACTION_ICON = { restart: "refresh", quit: "close", end: "stop" };

  /** The button for one of a program's actions; restart and end ask "Sure?" first. */
  function progButton(p, action) {
    const busy = state.progBusy && state.progBusy.id === p.id;
    const armed = state.progConfirm === `${p.id}|${action}`;
    const label = busy && state.progBusy.action === action ? ACTION[action][1] : armed ? `Sure? ${ACTION[action][0]}` : ACTION[action][0];
    const title = { restart: `Close ${p.name} and start it again`, quit: `Ask ${p.name} to close, like its X button`, end: `Stop ${p.name} at once; anything unsaved is lost` }[action];
    return `<button class="secondary${armed ? " sure" : ""}" data-act="prog-act" data-id="${esc(p.id)}" data-action="${action}" data-sure="1" title="${esc(title)}" ${busy ? "disabled" : ""}>${ICON[ACTION_ICON[action]]}${label}</button>`;
  }

  function progDetail(p) {
    const what = {
      restart: `Closes ${p.name} the way its X button does (ending it if it hides in the tray), then starts it again.`,
      quit: `Asks ${p.name} to close, the way its X button does, so it can save first.`,
      end: `Stops ${p.name} at once, children first. Anything unsaved in it is lost.`,
    };
    const rows = p.can.map((a) => `<dt>${ACTION[a][0]}</dt><dd>${esc(what[a])}</dd>`).join("");
    return `<dl>
        ${p.exe ? `<dt>File</dt><dd><code>${esc(p.exe)}</code></dd>` : ""}
        ${p.inside ? `<dt>Inside</dt><dd>${esc(p.inside)} run inside it, and stop when it closes.</dd>` : ""}
        <dt>Running</dt><dd class="num">${plural(p.count, "process", "processes")} since ${esc(clock(p.started))}${p.avgSecs ? `, ${p.avgCpu.toFixed(1)}% of the processor over the last ${esc(ageLong(p.avgSecs))}` : ""}</dd>
        ${rows || `<dt>Hands off</dt><dd>${esc(HANDS_OFF[p.kind] || "Mom leaves this one alone.")}</dd>`}
      </dl>
`;
  }

  /** A program's or extension's own picture: grey until you point at its row. */
  const rowIcon = (src) => (src ? `<img class="row-icon" src="${esc(src)}" alt="" draggable="false">` : "");

  /** Fetches the icons of these .exe files that mom hasn't seen yet, then shows them. Once per
      file: they're kept for as long as the widget runs. */
  async function loadIcons(exes) {
    const missing = [...new Set(exes.filter((x) => x && !state.icons.has(x)))];
    if (!missing.length) return;
    for (const x of missing) state.icons.set(x, null);
    try {
      const got = await invoke("get_icons", { exes: missing });
      let any = false;
      for (const [x, uri] of Object.entries(got || {})) {
        state.icons.set(x, uri);
        any = any || !!uri;
      }
      if (any) render();
    } catch (e) {
      console.warn("no icons", e);
    }
  }

  function progRow(p) {
    const open = state.progOpen === p.id;
    const cpu = p.avgSecs ? p.avgCpu : p.cpu;
    const bits = [p.kind === "app" || p.kind === "background" ? "" : HANDS_OFF[p.kind] ? { windows: "Windows", security: "Security", service: "Service", agent: "Agent", mom: "momputer" }[p.kind] : ""];
    bits.push(plural(p.count, "process", "processes"));
    if (p.windows) bits.push(plural(p.windows, "window"));
    if (p.gpu >= 1) bits.push(`graphics card ${Math.round(p.gpu)}%`);
    if (p.inside) bits.push(`${p.inside} inside`);
    const buttons = p.can.map((a) => progButton(p, a)).join("") + (p.exe && p.can.length ? `<button class="link-btn" data-act="reveal-path" data-path="${esc(p.exe)}">${ICON.folder}Show folder</button>` : "");
    return `<div class="row prog${open ? " is-open" : ""}" id="pg-${hash(p.id)}" data-led="${cpu >= 10 ? "red" : p.kind === "app" ? "green" : "off"}">
      <button class="row-name" data-act="prog-expand" data-id="${esc(p.id)}" aria-expanded="${open}" title="${esc(p.exe || p.name)}">${rowIcon(state.icons.get(p.exe))}${esc(p.name)}</button>
      <div class="row-meta"><span class="why">${esc(bits.filter(Boolean).join(" · "))}</span></div>
      <div class="row-nums num">${bytes(p.mem)}</div>
      <div class="row-nums2 num">${cpu >= 0.1 ? `<span class="${cpu >= 10 ? "hot" : ""}">${cpu < 10 ? cpu.toFixed(1) : Math.round(cpu)}%</span>` : ""}</div>
      <div class="row-act"></div>
      ${buttons ? `<div class="row-btns">${buttons}</div>` : ""}
      <div class="row-detail">${open ? progDetail(p) : ""}</div>
    </div>`;
  }

  /** The programs in the order they're shown. Sorted once (when the view opens or a sort is
      tapped), then kept: what's there stays where it is and what's new joins at the end, so a
      button never slides out from under the pointer when the numbers change. */
  function orderPrograms(list) {
    const sort = state.progView.sort;
    const sorted = [...list].sort(PROG_SORT[sort] || PROG_SORT.cpu);
    if (!state.progOrder || state.progOrderSort !== sort) {
      state.progOrder = sorted.map((p) => p.id);
      state.progOrderSort = sort;
      return sorted;
    }
    const byId = new Map(list.map((p) => [p.id, p]));
    const kept = state.progOrder.filter((id) => byId.has(id)).map((id) => byId.get(id));
    const known = new Set(state.progOrder);
    const out = [...kept, ...sorted.filter((p) => !known.has(p.id))];
    state.progOrder = out.map((p) => p.id);
    return out;
  }

  function renderPrograms() {
    const list = state.programs;
    if (!list) return `<div class="lead"><div class="lead-text"><p>Looking at what's running…</p></div></div>`;
    const v = state.progView;
    const q = v.q.trim().toLowerCase();
    const shown = orderPrograms(list).filter((p) => !q || `${p.name} ${p.exe || ""}`.toLowerCase().includes(q));
    const apps = shown.filter((p) => p.kind === "app");
    const background = shown.filter((p) => p.kind === "background");
    const rest = shown.filter((p) => p.kind !== "app" && p.kind !== "background");
    const cpu = list.reduce((a, p) => a + p.cpu, 0);
    const sort = (k, label) => `<button class="seg-btn" data-act="prog-sort" data-sort="${k}" aria-pressed="${v.sort === k}">${label}</button>`;
    let html = `<div class="lead"><div class="lead-text"><p>${plural(apps.length, "app")} open, ${background.length} in the background.<small>Using ${Math.round(cpu)}% of the processor between them.</small></p></div></div>
      <div class="list-tools">
        <label class="list-search">${ICON.search}<input type="search" id="prog-q" placeholder="Filter" value="${esc(v.q)}" autocomplete="off" spellcheck="false" aria-label="Filter programs"></label>
        <div class="seg" role="group" aria-label="Sort by">${sort("cpu", "CPU")}${sort("mem", "Memory")}${sort("gpu", "Graphics")}${sort("name", "A–Z")}</div>
      </div>
      <p class="list-hint">Rows stay where they are while you look, so nothing moves under your pointer. Tap a sort to sort again.</p>`;
    html += section("apps", "Apps", apps.length ? total(apps) : "", apps.map(progRow).join("") || `<p class="empty">${q ? "No app matches that." : "No app windows open."}</p>`);
    if (background.length) html += section("background", "In the background", total(background), background.map(progRow).join(""));
    if (rest.length)
      html += section(
        "rest",
        "Windows, security and agents",
        total(rest),
        (v.rest || q ? rest.map(progRow).join("") : "") +
          `<button class="quiet-toggle" data-act="prog-rest">${v.rest ? "Hide them" : `Show all ${rest.length}`} (mom leaves these alone)</button>`
      );
    return html;
  }

  async function loadPrograms() {
    if (state.programsLoading) return;
    state.programsLoading = true;
    try {
      state.programs = await invoke("get_programs");
      state.programsAt = Date.now();
      loadIcons(state.programs.map((p) => p.exe));
    } catch (e) {
      console.warn("no programs", e);
    }
    state.programsLoading = false;
    if (state.tab === "running" && state.runningTab === "programs") render();
  }

  /** Restart, quit or end: the second press of "Sure?" for restart and end. */
  async function programAction(id, action) {
    state.progConfirm = null;
    state.progBusy = { id, action };
    render();
    try {
      const r = await invoke("program_action", { id, action });
      toast(r.message, !r.ok);
      if (r.ok) react(action === "end" ? "cool" : "proud");
    } catch (e) {
      toast(String(e), true);
    }
    state.progBusy = null;
    await loadPrograms();
    if (action === "restart") setTimeout(() => loadSpeed(true), 4000);
    render();
  }

  // ------------------------------------------------------------ running: browsers

  const agoText = (t) => {
    if (!t) return "";
    const s = Math.max(0, Math.floor(Date.now() / 1000) - t);
    return s < 86400 ? "today" : `${ageLong(s)} ago`;
  };

  function extRow(b, e) {
    const saved = e.lastSaved ? `saved something ${agoText(e.lastSaved)}` : "has never saved anything";
    const meta = [e.can[0] || "Asks for nothing special", e.on ? saved : "switched off", e.added ? `added ${agoText(e.added)}` : ""].filter(Boolean);
    const led = !e.on ? "off" : e.everywhere ? "red" : e.lastSaved ? "green" : "off";
    return `<div class="row ext${e.on ? "" : " is-off"}" id="ex-${hash(b.id + e.id)}" data-led="${led}" title="${esc(e.description || e.name)}">
      <span class="row-name">${rowIcon(e.icon)}${esc(e.name)}${e.kind ? ` <span class="badge">${esc(e.kind)}</span>` : ""}${e.app ? ` <span class="badge look">Chrome app</span>` : ""}</span>
      <div class="row-meta"><span class="why">${esc(meta.join(" · "))}</span></div>
      <div class="row-nums num">${e.size ? bytes(e.size) : ""}</div>
      <div class="row-nums2"></div>
      <div class="row-act"><button class="mini-btn wide" data-act="copy-fix" data-text="${esc(e.page)}" data-hint="${esc(`Paste it into ${b.name}'s address bar: its page has Remove and an on/off switch.`)}" title="Copy its page in ${esc(b.name)}">${ICON.copy}Link</button></div>
    </div>`;
  }

  function profileDetail(b, p) {
    const key = `${b.id}|${p.dir}`;
    const on = p.extensions.filter((e) => e.on);
    const off = p.extensions.filter((e) => !e.on);
    const cookies = p.cookies != null ? `${p.cookies.toLocaleString("en-US")} from ${plural(p.cookieSites || 0, "site")}` : `${bytes(p.cookiesBytes)} (counted when ${b.name} is closed)`;
    const stats = [
      ["Everything", bytes(p.size)],
      ["Cache", `${bytes(p.cache)} · rebuilds itself`],
      ["History", bytes(p.history)],
      ["Cookies", cookies],
    ]
      .map(([k, v]) => `<div class="pc-row"><span class="pc-key">${esc(k)}</span><span class="pc-val num">${esc(v)}</span></div>`)
      .join("");
    const notes = p.notes
      .map((n) => `<div class="check-row" data-led="${n.level === "warn" ? "red" : "off"}"><div class="check-text"><span>${esc(n.text)}</span></div></div>`)
      .join("");
    const exts = on.map((e) => extRow(b, e)).join("");
    const offRows = state.extOff.has(key) ? off.map((e) => extRow(b, e)).join("") : "";
    return `<div class="pc">${stats}</div>
      ${notes ? `<div class="notes">${notes}</div>` : ""}
      ${p.extensions.length ? `<h4 class="sub-head">Extensions <span class="dim num">${on.length} on${off.length ? `, ${off.length} off` : ""}</span></h4><div class="ext-list">${exts}${offRows}</div>` : ""}
      ${off.length ? `<button class="quiet-toggle" data-act="ext-off" data-key="${esc(key)}">${state.extOff.has(key) ? "Hide the ones switched off" : `Show the ${off.length} switched off`}</button>` : ""}
      <div class="detail-actions"><button class="link-btn" data-act="reveal-path" data-path="${esc(p.path)}">${ICON.folder}Show folder</button></div>`;
  }

  function profileRow(b, p) {
    const key = `${b.id}|${p.dir}`;
    const open = state.browserOpen === key;
    const on = p.extensions.filter((e) => e.on).length;
    const meta = [p.lastUsed ? `used ${agoText(p.lastUsed)}` : "", p.extensions.length ? `${on} of ${plural(p.extensions.length, "extension")} on` : "no extensions"].filter(Boolean);
    const idle = p.lastUsed && Date.now() / 1000 - p.lastUsed > 180 * 86400;
    return `<div class="row prof${open ? " is-open" : ""}" id="pr-${hash(key)}" data-led="${p.notes.length ? "red" : idle ? "off" : "green"}">
      <button class="row-name" data-act="profile-open" data-key="${esc(key)}" aria-expanded="${open}">${esc(p.name)}</button>
      <div class="row-meta"><span class="why">${esc(meta.join(" · "))}${p.notes.length ? ` · <span class="hot">${plural(p.notes.length, "note")}</span>` : ""}</span></div>
      <div class="row-nums num">${bytes(p.size)}</div>
      <div class="row-nums2 num">${bytes(p.cache)} cache</div>
      <div class="row-act"><span class="row-chev">${ICON.chevron}</span></div>
      <div class="row-detail">${open ? profileDetail(b, p) : ""}</div>
    </div>`;
  }

  function renderBrowsers() {
    const list = state.browsers;
    const again = `<button class="secondary" data-act="browsers-again" ${state.browsersLoading ? "disabled" : ""}>${state.browsersLoading ? '<span class="led green blink"></span>Looking' : "Look again"}</button>`;
    if (!list) return `<div class="lead"><div class="lead-text"><p>Looking at your browsers, their profiles and every extension…</p></div>${again}</div>`;
    if (!list.length) return `<div class="lead"><div class="lead-text"><p>No Chrome, Edge, Brave or Vivaldi here.</p></div>${again}</div>`;
    const profiles = list.reduce((a, b) => a + b.profiles.length, 0);
    const exts = list.reduce((a, b) => a + b.profiles.reduce((x, p) => x + p.extensions.filter((e) => e.on).length, 0), 0);
    let html = `<div class="lead"><div class="lead-text"><div class="big"><span class="dot free-num">${exts}</span></div><p>${exts === 1 ? "extension is" : "extensions are"} on.<small>Across ${plural(profiles, "profile")} in ${plural(list.length, "browser")}. Open a profile to see what each one does.</small></p></div>${again}</div>`;
    for (const b of list) {
      const state_ = b.open ? `open for ${ageLong(b.openSecs)} · ${plural(b.processes, "process", "processes")} · ${bytes(b.mem)}` : "closed";
      const buttons = `<div class="browser-acts">${b.open ? `<button class="secondary" data-act="copy-fix" data-text="${esc(b.restart)}" data-hint="${esc(`Paste it into ${b.name}'s address bar. Your tabs come back.`)}" title="Copies ${esc(b.restart)}">${ICON.refresh}Restart page</button>` : ""}
        <button class="secondary" data-act="copy-fix" data-text="${esc(b.extensionsPage)}" data-hint="${esc(`Paste it into ${b.name}'s address bar.`)}" title="Copies ${esc(b.extensionsPage)}">${ICON.copy}Extensions page</button></div>`;
      html += section(`br-${b.id}`, b.name, `<span class="led ${b.open ? "green" : ""}"></span> ${esc(state_)}`, buttons + b.profiles.map((p) => profileRow(b, p)).join(""));
    }
    return html;
  }

  async function loadBrowsers(fresh = false) {
    if (state.browsersLoading) return;
    state.browsersLoading = true;
    if (state.tab === "running" && state.runningTab === "browsers") render();
    try {
      state.browsers = await invoke("get_browsers", { fresh });
      state.browsersAt = Date.now();
    } catch (e) {
      console.warn("no browsers", e);
    }
    state.browsersLoading = false;
    if (state.tab === "running" && state.runningTab === "browsers") render();
  }

  // ------------------------------------------------------------ running: startup

  function startupRow(x) {
    const meta = [
      x.keep,
      x.yours ? "" : "Every user's",
      x.running ? "Running now" : x.enabled ? "Not running now" : "",
      x.exe ? "" : x.location,
    ].filter(Boolean);
    const control = x.yours
      ? `<button class="switch" role="switch" aria-checked="${x.enabled}" aria-label="Start ${esc(x.name)} with Windows" data-act="startup-set" data-id="${esc(x.id)}" ${state.startupBusy === x.id ? "disabled" : ""}></button>`
      : `<button class="mini-btn wide" data-act="open-uri" data-url="ms-settings:startupapps" title="It starts for every user, so only an administrator can switch it. Windows Settings can.">Settings</button>`;
    return `<div class="row" id="st-${hash(x.id)}" data-led="${x.enabled ? (x.running ? "green" : "off") : "off"}" title="${esc(x.command)}">
      <span class="row-name">${rowIcon(state.icons.get(x.exe))}${esc(x.name)}</span>
      <div class="row-meta"><span class="why">${esc(meta.join(" · "))}</span></div>
      <div class="row-nums num">${x.running ? bytes(x.mem) : ""}</div>
      <div class="row-nums2 num">${x.running && x.cpu >= 0.1 ? `${x.cpu.toFixed(1)}%` : ""}</div>
      <div class="row-act">${control}</div>
    </div>`;
  }

  function renderStartup() {
    const list = state.startup;
    if (!list) return `<div class="lead"><div class="lead-text"><p>Looking at what starts with Windows…</p></div></div>`;
    const on = list.filter((x) => x.enabled);
    const off = list.filter((x) => !x.enabled);
    let html = `<div class="lead"><div class="lead-text"><div class="big"><span class="dot free-num">${on.length}</span></div><p>of ${plural(list.length, "app")} start with Windows.<small>Switch off what you don't need the moment you sign in. It still opens when you start it.</small></p></div>
      <button class="secondary" data-act="open-uri" data-url="ms-settings:startupapps">Settings${ICON.out}</button></div>`;
    html += section("start-on", "Starts with Windows", on.length ? `${total(on.filter((x) => x.running))} now` : "", on.map(startupRow).join("") || `<p class="empty">Nothing. Windows starts clean.</p>`);
    if (off.length) html += section("start-off", "Switched off", "", off.map(startupRow).join(""));
    return html;
  }

  async function loadStartup() {
    try {
      state.startup = await invoke("get_startup");
      state.startupAt = Date.now();
      loadIcons(state.startup.map((x) => x.exe));
    } catch (e) {
      console.warn("no startup list", e);
    }
    if (state.tab === "running" && state.runningTab === "startup") render();
  }

  /** A server mom stopped: what started it, where, and a button to start it again. */
  function stoppedRow(x) {
    const busy = state.startBusy === x.id;
    const where = x.project || x.cwd.split(/[\/]/).filter(Boolean).pop() || x.cwd;
    const ago = Math.max(60, Math.floor(Date.now() / 1000) - x.at);
    return `<div class="row" id="sp-${hash(x.id)}" data-led="off" title="${esc(`${x.command}
in ${x.cwd}`)}">
      <span class="row-name">${esc(x.name)} <span class="dim">${esc(where)}</span></span>
      <div class="row-meta"><span class="why"><span class="cmd">${esc(x.command)}</span> · stopped ${esc(ageLong(ago))} ago</span></div>
      <div class="row-nums num">${x.ports.length ? `:${x.ports[0]}` : ""}</div>
      <div class="row-nums2"></div>
      <div class="row-act"><button class="mini-btn wide" data-act="start-again" data-id="${esc(x.id)}" ${state.startBusy ? "disabled" : ""} title="Opens a terminal in ${esc(x.cwd)} and runs ${esc(x.command)}">${ICON.refresh}${busy ? "Starting" : "Start"}</button></div>
    </div>`;
  }

  async function loadStopped() {
    try {
      state.stopped = (await invoke("get_stopped")) || [];
    } catch (e) {
      console.warn("no stopped list", e);
    }
    if (state.tab === "running" && state.runningTab === "agents") render();
  }

  function renderAgents() {
    const s = state.snap;
    if (!s) return section("wait", "Having a look around", "", `<p class="empty">One moment…</p>`);
    const groups = s.groups.filter((g) => !state.done.has(g.id) || state.stopping.has(g.id));
    const left = groups.filter(isLeft);
    const forgotten = groups.filter((g) => g.status === "forgotten" && !g.kept);
    const running = groups.filter((g) => g.status === "running" && !g.kept);
    const helpers = groups.filter((g) => g.status === "helper");
    const yours = groups.filter((g) => g.status === "yours" && !g.kept);
    const kept = groups.filter((g) => g.kept && g.status !== "helper");
    let html = section(
      "left",
      "Left behind",
      left.length ? total(left) : "",
      left.length ? left.map((g, i) => row(g, { delay: state.stopping.has(g.id) ? i * 70 : 0 })).join("") : `<p class="empty">Nothing left behind. Good kids.</p>`,
      left.length ? "is-left" : ""
    );
    if (forgotten.length) html += section("forgotten", "Running a long time", total(forgotten), forgotten.map((g) => row(g)).join(""));

    const sessionsByPid = new Map(s.sessions.map((x) => [x.pid, x]));
    const busy = new Map();
    const strays = [];
    for (const g of running) {
      const sess = sessionsByPid.get(g.owner.agentPid);
      if (sess) {
        if (!busy.has(sess.pid)) busy.set(sess.pid, []);
        busy.get(sess.pid).push(g);
      } else strays.push(g);
    }
    for (const [pid, list] of busy) {
      const sess = sessionsByPid.get(pid);
      const where = sess.project ? ` in ${placeName(sess.project)}` : "";
      const mine = state.showHelpers ? helpers.filter((h) => h.owner.agentPid === pid) : [];
      html += section(`s${pid}`, `${sess.label}${where}`, total(list), list.map((g) => row(g)).join("") + mine.map((g) => row(g, { noStop: true })).join(""));
    }
    if (strays.length) html += section("strays", "Other agents' things", total(strays), strays.map((g) => row(g)).join(""));

    if (s.sessions.length) {
      const lines = s.sessions
        .map((x) => {
          const where = x.project ? ` in ${placeName(x.project)}` : "";
          const from = x.host ? `, ${x.host === "Terminal" ? "from a terminal" : `in the ${x.host}`}` : "";
          const help = x.helpers ? `${plural(x.helpers, "helper")}, ${bytes(x.helperMem)}` : "";
          return `<div class="row" id="ses-${x.pid}" data-led="green" style="grid-template-areas:'name nums act' 'meta nums2 act'">
            <span class="row-name">${esc(x.label)}${esc(where)}</span>
            <div class="row-meta"><span class="why">Up ${esc(ageLong(x.age))}${esc(from)}</span></div>
            <div class="row-nums num">${bytes(x.mem + x.helperMem)}</div>
            <div class="row-nums2 num">${esc(help)}</div>
            <div class="row-act"></div>
          </div>`;
        })
        .join("");
      const threads = (s.codex && s.codex.threads) || [];
      const threadRows = threads
        .map((t) => {
          const where = t.project ? ` in ${placeName(t.project)}` : "";
          const ago = Math.max(0, Math.floor(Date.now() / 1000) - t.updated);
          const status = t.working ? `<span class="badge working">working now</span>` : `<span class="why">active ${esc(ageLong(ago))} ago</span>`;
          const auto = t.automation
            ? `<span class="thread-line">Scheduled: ${esc(t.automation.name)}, ${esc(t.automation.schedule)}${t.automation.nextRun ? `, next around ${esc(clock(t.automation.nextRun))}` : ""}</span>`
            : "";
          const subs = t.subagents ? `<span class="thread-line">${plural(t.subagents, "sub-agent")} busy</span>` : "";
          return `<div class="row" id="thr-${hash(t.id)}" data-led="${t.working ? "green" : "off"}" style="grid-template-areas:'name nums act' 'meta nums2 act'">
            <span class="row-name" title="${esc(t.cwd || "")}">Codex${esc(where)}</span>
            <div class="row-meta"><span class="why">${esc(t.name || "Untitled thread")}</span>${status}${auto}${subs}</div>
            <div class="row-nums num"></div>
            <div class="row-nums2"></div>
            <div class="row-act"><button class="secondary" data-act="open-thread" data-link="${esc(t.link)}" title="Opens this thread in the Codex app">Open</button></div>
          </div>`;
        })
        .join("");
      const helperToggle = helpers.length
        ? `<button class="quiet-toggle" data-act="toggle-helpers">${state.showHelpers ? "Hide" : "Show"} their ${helpers.length} helpers (MCP servers, ${bytes(s.summary.helperMem)})</button>`
        : "";
      html += section("awake", "Agents awake", `${s.sessions.length + threads.length}`, threadRows + lines + helperToggle);
    }
    html += agentHealth();

    if (yours.length) html += section("yours", "Started by you", total(yours), yours.map((g) => row(g)).join(""));
    if (kept.length) html += section("kept", "Kept on purpose", total(kept), kept.map((g) => row(g)).join(""));

    // Servers mom stopped, with how they were started: not the ones running again already.
    const upAgain = (x) => s.groups.some((g) => g.name === x.name && (g.cwd || "").toLowerCase() === x.cwd.toLowerCase());
    const stopped = state.stopped.filter((x) => !upAgain(x));
    if (stopped.length)
      html += section(
        "stopped",
        "Stopped recently",
        "",
        stopped.map(stoppedRow).join("") + `<button class="quiet-toggle" data-act="forget-stopped">Forget ${stopped.length === 1 ? "it" : "them"}</button>`
      );

    const m = s.machines || {};
    let machines = "";
    if (m.docker && m.docker.running) {
      machines += m.docker.containers.length
        ? m.docker.containers
            .map(
              (c) => `<div class="row" id="ctr-${hash(c.id)}" data-led="green" data-container="${esc(c.id)}">
              <span class="row-name">${esc(c.name)}</span>
              <div class="row-meta"><span class="why">${esc(c.image)}${c.project ? `, ${esc(c.project)}` : ""}</span></div>
              <div class="row-nums num"></div>
              <div class="row-nums2">${esc(c.status)}</div>
              <div class="row-act"><button class="stop" data-act="container-stop" title="Runs docker stop. The container and its data stay; start it again any time.">Stop</button></div>
            </div>`
            )
            .join("")
        : `<p class="empty">Docker is running with no containers.</p>`;
    }
    if (m.wsl) {
      const names = m.wsl.running.map((d) => d.name).join(", ") || "its VM";
      machines += `<div class="row" id="wsl" data-led="green">
          <span class="row-name">WSL is running ${esc(names)}</span>
          <div class="row-meta"><span class="why">${m.wsl.running.some((d) => d.docker) ? "Docker Desktop lives here too" : "Linux in the background"}</span></div>
          <div class="row-nums num">${bytes(m.wsl.vmMem)}</div>
          <div class="row-nums2"></div>
          <div class="row-act"><button class="stop" data-act="wsl-stop" title="Runs wsl --shutdown. Your Linux files stay; WSL starts again next time you use it.">Shut down</button></div>
        </div>`;
    }
    for (const v of m.vms || []) {
      machines += `<div class="row" id="vm-${hash(v.name)}" data-led="green"><span class="row-name">${esc(v.name)}</span>
        <div class="row-meta"><span class="why">Hyper-V VM, up ${esc(v.uptime)}</span></div>
        <div class="row-nums num">${bytes(v.mem)}</div><div class="row-nums2"></div><div class="row-act"></div></div>`;
    }
    if (machines) html += section("machines", "Containers and VMs", "", machines);
    return html;
  }

  // ------------------------------------------------------------ how the agents are set up

  async function loadAgents() {
    if (state.agentsLoading) return;
    state.agentsLoading = true;
    try {
      state.agents = await invoke("get_agents");
    } catch (e) {
      console.warn("couldn't look at the agents' set-up", e);
      state.agents = state.agents || [];
    }
    state.agentsAt = Date.now();
    state.agentsLoading = false;
    if (state.tab === "running" && state.runningTab === "agents") render();
    if (state.sheet && state.sheet.kind === "agent") renderSheet();
  }

  /** In the Agents tab: a row an agent, with a light for whether its set-up holds together. */
  function agentHealth() {
    const list = state.agents;
    if (!list) return section("setup", "Agent health", "", `<p class="empty">Looking at their servers, hooks and settings…</p>`);
    if (!list.length) return "";
    const rows = list
      .map((a) => {
        const has = [a.servers.length && plural(a.servers.length, "MCP server"), a.hooks.length && plural(a.hooks.length, "hook"), a.skills.length && plural(a.skills.length, "skill")].filter(Boolean);
        // Its servers can be asked whether they answer, and nobody has yet: not "in order".
        const unchecked = !a.problems && a.canCheck && !a.checked && a.servers.some((s) => s.level === "unknown");
        const said = a.problems ? "to look at" : unchecked ? "not checked" : "in order";
        return `<div class="row spot-row" id="setup-${esc(a.id)}" role="button" tabindex="0" data-act="open-agent" data-id="${esc(a.id)}" data-led="${a.problems ? "red" : unchecked ? "off" : "green"}"
            aria-label="${esc(`${a.name}: ${a.problems ? `${a.problems} to look at` : said}. Show details`)}">
          <span class="row-name">${esc(a.name)}</span>
          <div class="row-meta"><span class="why">${esc(has.join(" · ") || "Nothing set up yet")}</span></div>
          <div class="row-nums num">${a.problems ? `<strong>${a.problems}</strong>` : ""}</div>
          <div class="row-nums2 num">${said}</div>
          <div class="row-act">${ICON.chevron}</div>
        </div>`;
      })
      .join("");
    const worth = list.reduce((n, a) => n + a.problems, 0);
    return section("setup", "Agent health", worth ? `${worth} to look at` : "", rows);
  }

  const SETUP_LED = { warn: "red", ok: "green" };

  /** One agent's set-up: what's worth a look first, then its servers, hooks, settings, skills
      and files. Every row opens the file behind it, to read. */
  function renderAgentSheet(id) {
    const a = (state.agents || []).find((x) => x.id === id);
    const acts = `<div class="head-acts">
        <button class="icon-btn${state.agentsLoading ? " spinning" : ""}" data-act="agents-refresh" title="${state.agentsLoading ? "Looking…" : "Look again"}" aria-label="Look again" ${state.agentsLoading ? "disabled" : ""}>${ICON.refresh}</button>
        <button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button></div>`;
    if (!a) return `<div class="sheet-head"><h2 id="sheet-title">Gone</h2>${acts}</div><p class="empty">That agent has no set-up here any more.</p>`;
    const said = (f) => (f && f.label === "Copy for your agent" ? "Copied. Paste it to your agent." : "Copied.");
    // What's worth a look: a sentence each, and something to do.
    const finding = (kind, title, x) => ({
      id: `${a.id}:${kind}:${title}`,
      title,
      detail: x.problem || x.detail || "",
      level: "warn",
      fix: x.fix ? { kind: "copy", label: x.fix.label, value: x.fix.text, hint: said(x.fix) } : null,
      more: x.doc ? [{ kind: "read", label: "Read the file", value: x.doc }] : [],
    });
    const worth = [
      ...a.settings.filter((f) => f.level === "warn").map((f) => finding("set", f.title, f)),
      ...a.servers.filter((s) => s.level === "warn").map((s) => finding("mcp", `MCP server ${s.name}`, s)),
      ...a.hooks.filter((h) => h.level === "warn").map((h) => finding("hook", `Hook for ${h.event}`, h)),
    ];
    // Everything else: a row each, that opens its file.
    const row = (key, led, name, sub, doc, extra = "") =>
      `<div class="item moved${doc ? " doc-row" : ""}" id="as-${hash(`${a.id}:${key}`)}" ${doc ? `role="button" tabindex="0" data-act="open-doc" data-id="${esc(doc)}" data-title="${esc(name)}"` : ""}>
        <span class="led ${led}"></span>
        <span class="item-main"><span class="item-name">${esc(name)}${extra}</span>${sub ? `<span class="item-sub" title="${esc(sub)}">${esc(sub)}</span>` : ""}</span>
        <span class="moved-acts">${doc ? `<span class="doc-open" aria-hidden="true">${ICON.chevron}</span>` : ""}</span></div>`;
    const list = (key, title, rows, aside = "") => {
      if (!rows.length) return "";
      const k = `${a.id}:${key}`;
      const all = state.showAllAgent.has(k);
      const more = rows.length > 6 ? `<button class="quiet-toggle" data-act="toggle-agent-list" data-key="${esc(k)}">${all ? "Show fewer" : `Show all ${rows.length}`}</button>` : "";
      return section(`as-${key}`, title, aside || String(rows.length), (all ? rows : rows.slice(0, 6)).join("") + more);
    };
    const badge = (text) => ` <span class="badge look">${esc(text)}</span>`;
    // Whether its servers answer: Claude Code can be asked to try them. Nobody else can yet.
    const checking = state.agentsChecking;
    const ago = a.checked ? ageLong(Math.max(0, Date.now() / 1000 - a.checked)) : "";
    const unknown = a.servers.filter((s) => s.level === "unknown").length;
    const tried = !a.servers.length
      ? ""
      : a.canCheck
        ? `<div class="tried"><p>${
            checking ? `<span class="led green blink"></span>${esc(a.name)} is trying each server…` : ago ? `${esc(a.name)} tried its servers ${esc(ago)} ago.` : `Nobody has checked whether its servers answer yet.`
          }</p><button class="secondary" data-act="agents-check" ${checking ? "disabled" : ""}>${checking ? "Checking" : ago ? "Check again" : "Check now"}</button></div>
          <p class="list-hint">Check asks ${esc(a.name)} to try each server, the way its /mcp screen does. Mom starts nothing herself.</p>`
        : unknown
          ? `<p class="list-hint">This is what its settings say. Mom can't ask ${esc(a.name)} to try its servers, so their lights stay off unless something's plainly wrong.</p>`
          : "";
    const answering = a.servers.filter((s) => s.level === "ok").length;
    const serverCount = [answering && `${answering} answering`, unknown && `${unknown} not checked`].filter(Boolean).join(" · ") || String(a.servers.length);
    const servers = a.servers.map((s, i) => row(`mcp${i}`, SETUP_LED[s.level] || "", s.name, [s.level === "ok" ? "" : s.problem, s.what, s.from].filter(Boolean).join(" · "), s.doc, s.level === "off" ? badge("off") : ""));
    const hooks = a.hooks.map((h, i) =>
      row(`hook${i}`, SETUP_LED[h.level] || "", h.matcher ? `${h.event} (${h.matcher})` : h.event, [h.level === "ok" ? "" : h.problem, h.what, h.mine ? "mom's check-in" : h.from, h.count > 1 ? `${h.count} times` : ""].filter(Boolean).join(" · "), h.doc)
    );
    const settings = a.settings.filter((f) => f.level !== "warn").map((f, i) => row(`set${i}`, SETUP_LED[f.level] || "", f.title, f.detail, f.doc));
    const skills = a.skills.map((s, i) => row(`skill${i}`, "green", s.name, [s.what, s.from].filter(Boolean).join(" · "), s.doc));
    const listed = new Set(a.skills.map((s) => s.doc));
    const files = a.docs.filter((d) => !listed.has(d.id)).map((d, i) => row(`doc${i}`, "", d.title, d.path, d.id));
    return `<div class="sheet-head">
        <div><h2 id="sheet-title">${esc(a.name)}</h2>
          <div class="pills"><span class="pill"><span class="led ${a.problems ? "red" : unknown && a.canCheck && !a.checked ? "" : "green"}"></span>${
            a.problems ? `${a.problems} to look at` : unknown && a.canCheck && !a.checked ? "Not checked yet" : "All in order"
          }</span>
          <span class="pill">Mom only reads</span></div></div>${acts}</div>
      <p class="sheet-what">How ${esc(a.name)} is set up on this PC. Tap a row to read the file behind it. Mom blanks what looks like a key.</p>
      ${worth.length ? section("as-worth", "Worth a look", String(worth.length), worth.map((w) => checkRow(w, "asw")).join("")) : ""}
      ${list("settings", "Settings", settings)}
      ${list("servers", "MCP servers", servers, serverCount)}
      ${tried}
      ${list("hooks", "Hooks", hooks)}
      ${list("skills", "Skills", skills)}
      ${list("files", "Files", files)}`;
  }

  // ------------------------------------------------------------ the reader (BIG MODE)

  /** Opens one of an agent's files to read, in BIG MODE: there's room for a page there. */
  async function openDoc(id, title) {
    state.reader = { id, title: title || "", doc: null, error: null };
    if (!state.big) {
      state.readerWasSmall = true;
      await setBigMode(true);
    }
    renderReader();
    try {
      const doc = await invoke("read_doc", { id });
      if (state.reader && state.reader.id === id) state.reader.doc = doc;
    } catch (e) {
      if (state.reader && state.reader.id === id) state.reader.error = String(e);
    }
    renderReader();
  }

  function closeReader() {
    if (!state.reader) return;
    state.reader = null;
    renderReader();
    if (state.readerWasSmall) {
      state.readerWasSmall = false;
      setBigMode(false);
    }
  }

  function renderReader() {
    const el = $("#reader");
    const r = state.reader;
    $(".app").classList.toggle("reading", !!r);
    el.hidden = !r;
    if (!r) {
      el.innerHTML = "";
      return;
    }
    const d = r.doc;
    const notes = [];
    if (d && d.hidden) notes.push(`Mom blanked ${plural(d.hidden, "thing")} that looked like ${d.hidden === 1 ? "a key" : "keys"}. Look it over before you share your screen.`);
    if (d && d.cut) notes.push("The rest is longer than mom shows.");
    const body = r.error
      ? `<p class="empty">${esc(r.error)}</p>`
      : !d
        ? `<p class="empty">Opening…</p>`
        : d.blind
          ? `<p class="empty">Mom couldn't read this as a settings file, so she can't tell what in it is a key, and won't show it. Show folder takes you to it.</p>`
          : d.kind === "markdown"
            ? `<article class="md reader-md">${markdown(d.text, true)}</article>`
            : `<pre class="reader-code"><code>${esc(d.text)}</code></pre>`;
    morph(
      el,
      `<header class="reader-head">
        <div class="reader-title"><h2>${esc(d ? d.title : r.title || "Opening…")}</h2>${d ? `<span class="reader-path" title="${esc(d.path)}">${esc(d.path)}</span>` : ""}</div>
        <div class="head-acts">
          ${d ? `<button class="link-btn" data-act="reveal-path" data-path="${esc(d.path)}">${ICON.folder}Show folder</button>` : ""}
          <button class="icon-btn" data-act="close-reader" title="Close" aria-label="Close the reader">${ICON.close}</button>
        </div>
      </header>
      ${notes.length ? `<p class="reader-note"><span class="led green"></span>${esc(notes.join(" "))}</p>` : ""}
      <div class="reader-body" id="reader-body" tabindex="0">${body}</div>`
    );
  }

  // ------------------------------------------------------------ upkeep: the PC's health and its trash

  const GROUPS = [
    ["agents", "Agents' mess"],
    ["dev", "Developer caches and builds"],
    ["windows", "Windows and browsers"],
    ["personal", "Your files"],
  ];

  /** A row of dots filled to a fraction, like a Nothing meter. The last few turn red when it's nearly full. */
  function meter(fraction, { n = 44, warn = false } = {}) {
    const f = Math.max(0, Math.min(1, fraction || 0));
    const lit = Math.round(f * n);
    const path = (from, to) => {
      let d = "";
      for (let i = from; i < to; i++) d += `M${i + 0.5} 0.5h0`;
      return d;
    };
    const hotFrom = warn ? Math.max(0, lit - 4) : lit;
    return `<svg class="meter" viewBox="0 0 ${n} 1" preserveAspectRatio="xMinYMid meet" aria-hidden="true">
      <path class="m-off" d="${path(lit, n)}"/><path class="m-on" d="${path(0, hotFrom)}"/><path class="m-hot" d="${path(hotFrom, lit)}"/></svg>`;
  }

  const spotsOf = () => state.trash.spots.filter((x) => x.available);
  const canFree = () => spotsOf().filter((x) => x.exact).reduce((a, x) => a + x.reclaimableBytes, 0);

  // The PC checkup, split over Upkeep's tabs: drives and memory with Storage, what runs with
  // Apps, Windows' own checkups with Windows.

  function renderDrives() {
    const h = state.health;
    if (!h) return section("pc", "Your PC", "", `<p class="empty">Checking…</p>`);
    const drives = h.drives
      .map((d) => {
        const used = d.total - d.free;
        const warn = d.free < d.total * 0.1 || d.free < 15 * 1024 ** 3;
        return `<div class="gauge" id="drv-${hash(d.root)}">
          <div class="gauge-head"><span class="gauge-name">${esc(d.root.replace(/\\$/, ""))} <span class="dim">${esc(d.label)}</span></span>
            <span class="num"><strong>${bytes(d.free)}</strong> free of ${bytes(d.total)}</span></div>
          ${meter(d.total ? used / d.total : 0, { warn })}
        </div>`;
      })
      .join("");
    const mem = h.memory;
    const memPct = mem.total ? mem.used / mem.total : 0;
    const memory = `<div class="gauge" id="mem">
        <div class="gauge-head"><span class="gauge-name">Memory</span><span class="num"><strong>${Math.round(memPct * 100)}%</strong> used of ${bytes(mem.total)}</span></div>
        ${meter(memPct, { warn: memPct > 0.85 })}
      </div>`;
    return section("pc", "Your PC", `up ${ageLong(h.uptimeSecs)}`, drives + memory);
  }

  function renderChecks() {
    const h = state.health;
    if (!h) return "";
    const order = { warn: 0, tip: 1, ok: 2 };
    const checks = [...h.checks]
      .sort((a, b) => (order[a.level] ?? 1) - (order[b.level] ?? 1))
      .map(
        (c) => `<div class="check-row" id="chk-${hash(c.id)}" data-led="${c.level === "warn" ? "red" : c.level === "ok" ? "green" : "off"}">
          <div class="check-text"><strong>${esc(c.title)}</strong>${brief(`chk:${c.id}`, c.detail)}</div>
          ${c.action ? `<button class="secondary" data-act="open-uri" data-url="${esc(c.action.url)}" title="Opens Windows Settings">${esc(c.action.label)}${ICON.out}</button>` : ""}
        </div>`
      )
      .join("");
    return checks ? section("checks", "Checkups", "", checks) : "";
  }

  function renderApps() {
    const h = state.health;
    if (!h) return `<p class="empty">Checking…</p>`;
    const unusedApps = h.unusedApps || [];
    const now = Date.now() / 1000;
    // One you haven't opened in a year: how big, since when, where it is, and a way to remove it.
    const appRow = (a) => {
      const sure = state.uninstallConfirm === a.key;
      const opened = state.uninstalled.has(a.key);
      const since = `${a.from ? "played" : "opened"} ${yearsAgo(now - a.lastUsed)} ago`;
      // Where it is, in full; whoever made it when there's no folder to show.
      const where = a.dir || (a.from ? "" : a.publisher);
      const folder = a.dir ? `<button class="mini-btn" data-act="reveal-path" data-path="${esc(a.dir)}" title="${esc(`Show in File Explorer: ${a.dir}`)}" aria-label="Show ${esc(a.name)} in File Explorer">${ICON.folder}</button>` : "";
      return `<div class="item stays moved" id="un-${hash(a.key)}"><span class="led"></span>
          <span class="item-main"><span class="item-name" title="${esc(a.dir ? `${a.name} · ${a.dir}` : a.name)}">${esc(a.name)}</span>
          <span class="item-sub">${esc(`${a.size ? bytes(a.size) : "size unknown"} · ${since}`)}</span>${where ? `<span class="item-sub">${esc(where)}</span>` : ""}</span>
          <span class="moved-acts">${folder}<button class="mini-btn wide${sure ? " sure" : ""}" data-act="uninstall-app" data-key="${esc(a.key)}" ${opened ? "disabled" : ""}>${opened ? "Opened" : sure ? "Sure? Uninstall" : "Uninstall"}</button></span></div>`;
    };
    // Apps and games each get their own list: a Steam library can hold hundreds of games, and
    // the apps would be lost among them.
    const unusedList = (key, title, list, note, none) => {
      const all = state.showAllUnused.has(key);
      const total = list.reduce((sum, a) => sum + a.size, 0);
      const more = list.length > 8 ? `<button class="quiet-toggle" data-act="toggle-unused" data-which="${key}">${all ? "Show fewer" : `Show all ${list.length}`}</button>` : "";
      return list.length
        ? section(key, title, total ? bytes(total) : "", (all ? list : list.slice(0, 8)).map(appRow).join("") + more + `<p class="group-note">${note}</p>`)
        : none
          ? section(key, title, "", `<p class="empty">${none}</p>`)
          : "";
    };
    const apps = unusedApps.filter((a) => !a.from);
    const games = unusedApps.filter((a) => a.from);
    return (
      unusedList(
        "unused",
        "Apps you haven't opened in a year",
        apps,
        "Uninstall opens the app's own uninstaller. Drivers, runtimes, wallets, messengers and command-line tools are never listed.",
        "None. Everything installed gets used."
      ) +
      unusedList("unused-games", "Games you haven't played in a year", games, "Going by Steam's own record. Uninstall hands the game to Steam, which asks first.", "") +
      section(
        "what-runs",
        "What runs",
        "",
        `<p class="group-note">What starts with Windows (with a switch for each), and what's using the processor and memory right now, are in Running.</p>
        <div class="group-foot"><button class="link-btn" data-act="go-tab" data-to="running:startup">Startup apps</button><button class="link-btn" data-act="go-tab" data-to="running:programs">Programs</button></div>`
      )
    );
  }

  function spotRow(s) {
    // Empty folders free no space, but they're still clutter.
    const empties = s.reclaimableBytes === 0 && s.reclaimableItems > 0;
    const gone = s.reclaimableBytes === 0 && !empties;
    const led = gone ? "green" : s.reclaimableBytes >= 512 * 1024 ** 2 ? "red" : "off";
    const size = s.exact ? (gone ? "Clean" : empties ? `${s.reclaimableItems.toLocaleString("en-US")} empty` : bytes(s.reclaimableBytes)) : "Tool decides";
    const of = s.exact ? `of ${bytes(s.totalBytes)}` : bytes(s.totalBytes);
    return `<div class="row spot-row" id="spot-${s.id}" role="button" tabindex="0" data-act="open-spot" data-spot="${esc(s.id)}" data-led="${led}"
        aria-label="${esc(`${s.label}: ${size} can go. Show details`)}">
      <span class="row-name">${esc(s.label)}${s.safety === "review" ? ` <span class="badge look">look first</span>` : ""}${s.admin ? ` <span class="badge look">admin</span>` : ""}</span>
      <div class="row-meta"><span class="why">${esc(s.note || s.what)}</span></div>
      <div class="row-nums num"><strong>${size}</strong></div>
      <div class="row-nums2 num">${of}</div>
      <div class="row-act">${ICON.chevron}</div>
    </div>`;
  }

  /** A size as a big dot-matrix number with its unit in plain type: "15.0 GB". */
  const sizeDots = (b) => {
    const [n, unit] = bytes(b).split(" ");
    // Doto draws "." as a little cross; a plain round dot reads as a decimal point.
    const digits = esc(n).replace(".", '<i class="pt"></i>');
    return `<span class="dot free-num">${digits}</span><span class="unit">${esc(unit || "")}</span>`;
  };

  let shownFree = null;
  function drawFree() {
    const el = $("#free-big");
    if (!el) return;
    const target = canFree();
    const draw = (v) => (el.innerHTML = sizeDots(v));
    if (shownFree === null || !moving()) {
      shownFree = target;
      draw(target);
      return;
    }
    if (target === shownFree) return;
    const o = { v: shownFree };
    shownFree = target;
    G.to(o, { v: target, duration: 0.6, ease: "power2.out", onUpdate: () => draw(o.v) });
  }

  /** How long ago the oldest measurement was taken (they're saved between restarts). */
  const measuredAgo = () => {
    const at = spotsOf().map((x) => x.measuredAt).filter(Boolean);
    return at.length ? Math.max(0, Math.floor(Date.now() / 1000) - Math.min(...at)) : 0;
  };

  const UPKEEP_TABS = [
    ["storage", "Storage"],
    ["apps", "Apps"],
    ["windows", "Windows"],
    ["files", "Files"],
  ];

  function upkeepTabs() {
    const w = state.winsettings;
    const serious = w ? w.filter((f) => f.level === "fix").length : 0;
    const dot = (k) => (k === "windows" && serious ? `<span class="count num">${serious}</span>` : "");
    return `<div class="subtabs" role="tablist" aria-label="Upkeep">${UPKEEP_TABS.map(
      ([k, label]) => `<button class="subtab" role="tab" data-act="upkeep-tab" data-tab="${k}" aria-selected="${state.upkeepTab === k}">${label}${dot(k)}</button>`
    ).join("")}</div>`;
  }

  function renderUpkeep() {
    const tab = state.upkeepTab;
    if (tab === "apps") return upkeepTabs() + renderApps();
    if (tab === "windows") return upkeepTabs() + renderWindows();
    if (tab === "files") return upkeepTabs() + renderFiles();
    const t = state.trash;
    const spots = spotsOf();
    const said =
      t.busy && !spots.length
        ? "Measuring…"
        : t.busy
          ? "found so far, still looking."
          : spots.length
            ? "mom can free up."
            : "Let me look under the bed.";
    const small = spots.length && !t.busy ? `Measured ${ageLong(measuredAgo())} ago. Tap anything to see exactly what it is.` : "";
    const shown = shownFree === null ? "" : sizeDots(shownFree);
    let html = upkeepTabs();
    html += `<div class="lead" id="upkeep-lead"><div class="lead-text">${spots.length ? `<div class="big" id="free-big">${shown}</div>` : ""}<p>${esc(said)}${small ? `<small>${esc(small)}</small>` : ""}</p></div>
      <button class="secondary" data-act="measure" ${t.busy ? "disabled" : ""}>${t.busy ? `<span class="led green blink"></span>Measuring` : spots.length ? "Measure again" : "Measure"}</button></div>`;
    for (const [key, title] of GROUPS) {
      const list = spots.filter((x) => (x.group || "dev") === key).sort((a, b) => b.reclaimableBytes - a.reclaimableBytes || b.totalBytes - a.totalBytes);
      if (!list.length) continue;
      const sum = list.filter((x) => x.exact).reduce((a, x) => a + x.reclaimableBytes, 0);
      html += section(`g-${key}`, title, sum ? `${bytes(sum)} can go` : "", list.map(spotRow).join(""));
    }
    html += renderDrives();
    html += `<p class="foot">Nothing is cleaned until you open an item and press its button. Each one tells you exactly what it removes first.</p>`;
    return html;
  }

  // ------------------------------------------------------------ Upkeep: Windows settings

  const AREAS = ["Safety", "Memory", "Services", "Display", "Terminal", "Startup", "Shutdown", "Network", "Paths"];

  async function loadWinsettings(force = false) {
    if (state.winLoading || (state.winsettings && !force)) return;
    state.winLoading = true;
    render();
    try {
      state.winsettings = await invoke("get_winsettings");
    } catch (e) {
      toast(String(e), true);
    }
    state.winLoading = false;
    render();
  }

  function findingCard(f) {
    const busy = state.winBusy === f.id;
    const sure = state.winConfirm === f.id;
    const led = f.level === "fix" ? "red" : f.level === "check" ? "off" : "";
    const fix = f.fix
      ? `<button class="secondary${sure ? " sure" : ""}" data-act="win-fix" data-id="${esc(f.id)}" ${state.winBusy ? "disabled" : ""}>${
          busy ? '<span class="led green blink"></span>Working…' : sure ? (f.fix.admin ? "Sure? Windows will ask" : "Sure? Press again") : esc(f.fix.label)
        }</button>`
      : "";
    const open = f.settings
      ? `<button class="link-btn" data-act="${f.settings.startsWith("ms-settings:") ? "open-uri" : "open-program"}" data-url="${esc(f.settings)}">Open the setting${ICON.out}</button>`
      : f.command
        ? `<button class="secondary" data-act="copy-fix" data-text="${esc(f.command)}" data-hint="Paste it into PowerShell and press Enter." title="Copies ${esc(f.command)}">${ICON.copy}Copy the command</button>`
        : "";
    const after = f.fix ? [f.fix.after, f.fix.admin ? "Windows asks first." : f.fix.undo ? "You can undo it." : ""].filter(Boolean).join(" ") : "";
    return `<div class="drift" id="wf-${hash(f.id)}" data-level="${f.level}">
        <div class="drift-title">${led ? `<span class="led ${led === "red" ? "red" : ""}"></span>` : ""}<strong>${esc(f.title)}</strong>${f.level === "info" ? `<span class="badge">your call</span>` : ""}</div>
        <div class="drift-vals"><div><span>Now</span>${esc(f.now)}</div><div><span>${f.area === "Terminal" ? "Better" : "Windows' default"}</span>${esc(f.default)}</div></div>
        <p class="drift-why">${esc(f.why)}</p>
        ${fix || open ? `<div class="drift-acts">${open || "<span></span>"}${fix}</div>` : ""}
        ${after ? `<p class="drift-after">${esc(after)}</p>` : ""}
      </div>`;
  }

  const SYSTEM_AREAS = [
    ["Security", "Protection"],
    ["Updates", "Updates"],
    ["Hardware", "Hardware"],
    ["Sound", "Sound"],
    ["Performance", "Performance"],
    ["Registry", "Registry leftovers"],
  ];

  /** What the PC is, and each area of the checkup with a button where there's something to do. */
  function renderSystem() {
    const sys = state.system;
    if (!sys) {
      return section("pc", "This PC", "", `<p class="empty">${state.systemLoading ? '<span class="led green blink"></span> Looking at its hardware, protection and updates…' : "One moment…"}</p>`);
    }
    const pc = sys.pc.map(([k, v]) => `<div class="pc-row"><span class="pc-key">${esc(k)}</span><span class="pc-val">${esc(v)}</span></div>`).join("");
    let html = section("pc", "This PC", "", `<div class="pc">${pc}</div>`);
    if (state.regFixed.size) {
      const rows = [...state.regFixed]
        .map(
          ([id, title]) => `<div class="item stays moved" id="rf-${hash(id)}"><span class="led green"></span>
          <span class="item-main"><span class="item-name">${esc(title)}</span><span class="item-sub">Removed just now; mom kept a copy</span></span>
          <span class="moved-acts"><button class="mini-btn wide" data-act="reg-undo" data-id="${esc(id)}">Undo</button></span></div>`
        )
        .join("");
      html += section("reg-fixed", "Removed just now", "", `<div class="items">${rows}</div>`);
    }
    const order = { warn: 0, tip: 1, ok: 2 };
    for (const [area, title] of SYSTEM_AREAS) {
      const list = sys.checks.filter((c) => c.area === area).sort((a, b) => (order[a.level] ?? 1) - (order[b.level] ?? 1));
      if (!list.length) continue;
      html += section(
        `sys-${area.toLowerCase()}`,
        title,
        "",
        list.map((c) => checkRow(c, "sys")).join("")
      );
    }
    return html;
  }

  async function loadSystem(fresh = false) {
    if (state.systemLoading) return;
    state.systemLoading = true;
    if (state.tab === "upkeep" && state.upkeepTab === "windows") render();
    try {
      state.system = await invoke("get_system", { fresh });
    } catch (e) {
      console.warn("no system checkup", e);
    }
    state.systemLoading = false;
    if (state.tab === "upkeep" && state.upkeepTab === "windows") render();
    renderStage();
  }

  // ------------------------------------------------------------ versions

  /** Asks whether there's a newer momputer (the backend asks GitHub at most once a day, and not
      at all if that's switched off, unless `now`). A new one gets a dot on the About button and
      one mention. */
  async function loadUpdate(now = false) {
    if (state.updateBusy) return;
    state.updateBusy = true;
    if (state.sheet && state.sheet.kind === "about") renderSheet();
    try {
      state.update = await invoke("get_update", { now });
    } catch (e) {
      console.warn("no word on new versions", e);
    }
    state.updateBusy = false;
    const u = state.update;
    $("#btn-about").classList.toggle("has-news", !!(u && u.newer));
    if (u && u.newer && state.updateTold !== u.latest) {
      state.updateTold = u.latest;
      toast(`momputer ${u.latest} is out. It's under the "i".`);
    }
    if (state.sheet && state.sheet.kind === "about") renderSheet();
  }

  /** In About: whether this is the newest version, and the button that gets the newer one. */
  function updateLine() {
    const u = state.update;
    const again = `<button class="link-btn" data-act="update-check" ${state.updateBusy ? "disabled" : ""}>${state.updateBusy ? "Asking…" : "Check now"}</button>`;
    if (u && u.newer)
      return `<div class="about-update is-new"><span class="led red"></span><span><b>${esc(u.latest)} is out.</b> You have ${esc(u.current)}.</span>
        <button class="primary" data-act="open-uri" data-url="${esc(u.getIt)}">Get ${esc(u.latest)}${ICON.out}</button></div>`;
    const said = !u ? "" : u.problem && !u.latest ? esc(u.problem) : u.latest ? "This is the newest version." : "Mom hasn't asked about new versions yet.";
    return `<div class="about-update"><span>${said}</span>${again}</div>`;
  }

  // ------------------------------------------------------------ BIG MODE

  let stage = null; // the desk in three dimensions (big.js), while BIG MODE is on
  let stageShape = ""; // how many sticks, drives and cards it was built with

  /** The parts on mom's desk: what each is called, its line in "This PC", which checks are its
      own, and how mom speaks of it. */
  const PARTS = [
    ["processor", "Processor", "Processor", (c) => c.area === "Performance", "Your processor", false, ["Power settings", "ms-settings:powersleep"]],
    ["memory", "Memory", "Memory", (c) => c.id.startsWith("memory"), "Your memory", false, null],
    ["graphics", "Graphics card", "Graphics", (c) => c.id.startsWith("gpu"), "Your graphics card", false, ["Display settings", "ms-settings:display"]],
    ["disks", "Drives", "Disks", (c) => /^(ssd-health|disk-health|windows-on-hdd|trim-off|optimize-off)/.test(c.id), "Your drives", true, ["Storage settings", "ms-settings:storagesense"]],
    ["sound", "Sound", "Sound", (c) => c.area === "Sound", "Your sound", false, ["Sound settings", "ms-settings:sound"]],
    ["keyboard", "Keyboard", "Keyboard", () => false, "Your keyboard", false, ["Typing settings", "ms-settings:typing"]],
    ["mouse", "Mouse", "Mouse", () => false, "Your mouse", false, ["Mouse settings", "ms-settings:mousetouchpad"]],
    ["board", "Motherboard", "Board", (c) => c.id === "bios-old", "Your motherboard", false, null],
    ["windows", "Windows", "Windows", (c) => c.area === "Security" || c.area === "Updates", "Windows", false, ["Windows Update", "ms-settings:windowsupdate"]],
  ];

  /** Your PC as the desk shows it: each part with its checks and its light, and how many
      sticks, drives and cards to draw. */
  function bigModel() {
    const sys = state.system;
    const rows = new Map(sys ? sys.pc : []);
    const checks = sys ? sys.checks : [];
    const worst = (list) => (list.some((c) => c.level === "warn") ? "warn" : list.some((c) => c.level === "tip") ? "tip" : "ok");
    // A keyboard's or mouse's maker often leaves a helper running all day (Speed knows): that's its to answer for.
    const helpers = state.speed ? state.speed.findings.filter((f) => f.id.startsWith("extras-")) : [];
    const parts = PARTS.map(([id, label, row, mine, subject, plural, settings]) => {
      const text = rows.get(row) || "";
      let own = checks.filter(mine);
      if (id === "keyboard" || id === "mouse") {
        const makers = text.split(" · ").map((n) => n.split(" ")[0].toLowerCase()).filter((w) => w.length > 2);
        own = helpers.filter((f) => makers.some((w) => f.title.toLowerCase().includes(w)));
      }
      // Nothing plugged in that mom can see: its light stays off.
      const level = !sys ? "none" : (id === "keyboard" || id === "mouse") && !text ? "none" : worst(own);
      return { id, label, subject, plural, settings, row: text, checks: own, level };
    });
    const memory = rows.get("Memory") || "";
    const sticks = +((/(\d+) × /.exec(memory) || /(\d+) sticks/.exec(memory) || [0, 2])[1]);
    const list = (text) => text.split(" · ").filter(Boolean);
    const drives = list(rows.get("Disks") || "")
      .filter((d) => !/USB/.test(d))
      .map((d) => (/NVMe/.test(d) ? "nvme" : /hard disk/.test(d) ? "hdd" : "ssd"));
    const cards = list(rows.get("Graphics") || "").length || 1;
    return { parts, sticks, drives, cards, levels: Object.fromEntries(parts.map((p) => [p.id, p.level])) };
  }

  /** The names of the parts across the top, and what mom knows about the one that's picked. */
  function renderStage() {
    if (!state.big) return;
    const m = bigModel();
    const led = (level) => `<span class="led ${level === "warn" ? "red" : level === "ok" ? "green" : ""}"></span>`;
    morph(
      $("#stage-parts"),
      m.parts.map((p) => `<button class="part-pill" data-act="big-part" data-id="${p.id}" aria-pressed="${state.bigPart === p.id}">${led(p.level)}${esc(p.label)}</button>`).join("")
    );
    const p = m.parts.find((x) => x.id === state.bigPart);
    let html;
    if (!state.system) {
      html = `<h3><span class="led green blink"></span>Looking over your PC…</h3><p class="stage-row">Its parts light up as mom gets to them.</p>`;
    } else if (p) {
      const rows = p.checks.map((c) => checkRow(c, "big")).join("");
      const nothing = p.row ? "Nothing to say about it. That's good news." : `Mom can't see ${p.plural ? "any" : "one"} plugged in.`;
      const settings = p.settings ? `<div class="stage-acts"><button class="link-btn" data-act="open-uri" data-url="${esc(p.settings[1])}">${esc(p.settings[0])}${ICON.out}</button></div>` : "";
      html = `<h3>${led(p.level)}${esc(p.label)}</h3>${p.row ? `<p class="stage-row">${esc(p.row)}</p>` : ""}${rows || `<p class="stage-row">${nothing}</p>`}${settings}`;
    } else {
      const bad = m.parts.filter((x) => x.level === "warn").map((x) => x.label);
      const said = bad.length ? `${bad.join(", ")} ${bad.length === 1 ? "needs" : "need"} a look: the red ${bad.length === 1 ? "light" : "lights"}.` : "Every light is green.";
      html = `<h3>Your PC</h3><p class="stage-row">${esc(said)} Drag to look around, tap a part, or poke mom.</p>`;
    }
    morph($("#stage-info"), html);
    if (!stage) return;
    // More sticks or drives than it was drawn with: draw it again.
    const shape = JSON.stringify([m.sticks, m.drives, m.cards]);
    if (shape !== stageShape) openStage();
    else stage.update(m);
  }

  /** Picks a part of the PC (again to let go): the desk leans in, and mom says how it is. */
  function pickPart(id) {
    state.bigPart = id && id !== state.bigPart ? id : null;
    $("#stage-tip").hidden = true;
    if (stage) stage.select(state.bigPart);
    const p = bigModel().parts.find((x) => x.id === state.bigPart);
    if (p && state.system) {
      const trouble = p.checks.find((c) => c.level === "warn");
      react(p.level === "warn" || p.level === "none" ? "hmm" : p.level === "tip" ? "look" : "proud", 3200);
      const none = p.level === "none" ? `I can't see ${p.subject.toLowerCase().replace("your ", "a ")} from here.` : "";
      say(none || (trouble ? trouble.title + "." : p.level === "tip" ? `I have a tip for ${p.subject.toLowerCase()}.` : `${p.subject} ${p.plural ? "are" : "is"} fine, sweetie.`), 4200);
    }
    renderStage();
  }

  /** The part under the pointer, named beside it. */
  function stageTip(id, x, y) {
    const tip = $("#stage-tip");
    const p = id && bigModel().parts.find((q) => q.id === id);
    if (!p) {
      tip.hidden = true;
      return;
    }
    tip.innerHTML = `${esc(p.label)}${p.row ? `<small>${esc(p.row.length > 64 ? p.row.slice(0, 63) + "…" : p.row)}</small>` : ""}`;
    tip.style.left = x + "px";
    tip.style.top = y + "px";
    tip.hidden = false;
  }

  /** Draws the desk (again, if it was there), with mom's own face laid on its monitor. */
  async function openStage() {
    try {
      const big = await import("./big.js");
      if (!state.big) return;
      if (stage) stage.close();
      const m = bigModel();
      stageShape = JSON.stringify([m.sticks, m.drives, m.cards]);
      stage = big.open($("#stage-3d"), $("#stage-screen"), m, { pick: (id) => pickPart(id || state.bigPart), hover: stageTip });
      if (stage && state.bigPart) stage.select(state.bigPart);
      if (!T) window.__momBig = stage; // the browser preview's tests find the parts through it
    } catch (e) {
      console.warn("no 3D here", e);
    }
  }

  /** BIG MODE on or off: the window grows (or shrinks back to the widget), and mom moves between
      her card and the monitor on the desk. */
  /** A fade the browser runs by itself, at the screen's own rate (GSAP here is kept to 60 a second). */
  const fade = (el, from, to, ms) => el.animate([{ opacity: from }, { opacity: to }], { duration: ms, easing: "ease-out", fill: "forwards" }).finished.catch(() => {});
  const twoFrames = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));

  async function setBigMode(on) {
    if (state.big === on || state.bigMoving) return;
    // Smoothly, one go at a time, the way a window closes and opens on a well-made desktop. What's
    // on the page fades out; the app swaps the window for a stand-in of the same shape and takes
    // the real window off the screen; the stand-in closes and a new one opens where the window is
    // going, while the page, unseen, is laid out for it; then the window is put back under the new
    // shape, which dissolves into the finished page. Nothing is ever resized where you can see it.
    state.bigMoving = true;
    const app = $(".app");
    const smooth = moving();
    const fading = smooth ? fade(app, 1, 0, 90) : null;
    let carried = false;
    try {
      carried = (await invoke("set_big", { on, smooth, bg: getComputedStyle(document.body).backgroundColor })) === true;
    } catch (e) {
      console.warn("the window stayed as it was", e);
    }
    await fading;
    state.big = on;
    app.classList.toggle("big-mode", on);
    const button = $("#btn-big");
    button.setAttribute("aria-pressed", String(on));
    button.title = on ? "Back to the widget" : "BIG MODE: a full window, with your PC on mom's desk";
    button.setAttribute("aria-label", on ? "Back to the widget" : "BIG MODE");
    $("#big-badge").hidden = !on;
    const face = $("#face");
    let desk = null;
    if (on) {
      $("#stage-screen").appendChild(face);
      if (!state.system) loadSystem();
      // The desk is drawn once its parts have loaded; it fades in by itself (see .stage-3d canvas).
      desk = openStage();
    } else {
      state.reader = null;
      state.readerWasSmall = false;
      if (stage) stage.close();
      stage = null;
      state.bigPart = null;
      $(".note").prepend(face);
      $("#stage-tip").hidden = true;
    }
    moveThumb(false);
    renderStage();
    renderReader(); // a file opened from the small widget is on the page when it shows
    if (carried) {
      // Behind the stand-in the page is whole again: it's what the stand-in dissolves into.
      for (const run of app.getAnimations()) run.cancel();
      await invoke("big_arrived").catch(() => {});
      await twoFrames(); // the page has drawn itself at its new size
      await invoke("card_away").catch(() => {});
    } else if (smooth) {
      // No stand-in to be had: the window jumped, and what's on it fades back in.
      await fade(app, 0, 1, 180);
      for (const run of app.getAnimations()) run.cancel();
    }
    state.bigMoving = false;
    if (on) {
      await desk;
      say("BIG MODE. Look at you, with a whole desk.", 3600);
    }
  }

  function renderWindows() {
    const w = state.winsettings;
    let html;
    if (!w) {
      html = `<div class="lead"><div class="lead-text"><p>${state.winLoading ? "Looking at how Windows is set up…" : "Mom can look for settings that drifted from how Windows ships them."}</p></div>
        <button class="secondary" data-act="win-refresh" ${state.winLoading ? "disabled" : ""}>${state.winLoading ? '<span class="led green blink"></span>Looking' : "Look"}</button></div>`;
    } else {
      const serious = w.filter((f) => f.level === "fix").length;
      const odd = w.filter((f) => f.level !== "info").length;
      const said = !w.length
        ? "Windows is set up the way it shipped. Nothing to put back."
        : serious
          ? `${serious === 1 ? "setting is" : "settings are"} getting in your way. Each one says what it does now and what Windows ships with.`
          : odd
            ? `${odd === 1 ? "old tweak or leftover" : "old tweaks and leftovers"}, from years of trying things. Nothing urgent.`
            : "Only choices that are fine either way.";
      html = `<div class="lead"><div class="lead-text">${w.length && (serious || odd) ? `<div class="big"><span class="dot free-num">${serious || odd}</span></div>` : ""}<p>${esc(said)}</p></div>
        <button class="secondary" data-act="win-refresh" ${state.winLoading ? "disabled" : ""}>${state.winLoading ? '<span class="led green blink"></span>Looking' : "Look again"}</button></div>`;
      if (state.winFixed.length) {
        const rows = state.winFixed
          .map(
            (x) => `<div class="item stays moved" id="wu-${hash(x.id)}"><span class="led green"></span>
            <span class="item-main"><span class="item-name">${esc(x.title)}</span><span class="item-sub">Put back to Windows' default</span></span>
            <span class="moved-acts"><button class="mini-btn wide" data-act="win-undo" data-id="${esc(x.id)}">Undo</button></span></div>`
          )
          .join("");
        html += section("win-fixed", "Put back just now", "", `<div class="items">${rows}</div>`);
      }
      html += renderSystem();
      const areas = [...AREAS, ...new Set(w.map((f) => f.area).filter((a) => !AREAS.includes(a)))];
      for (const area of areas) {
        const list = w.filter((f) => f.area === area);
        if (list.length) html += section(`win-${area.toLowerCase()}`, `Settings: ${area}`, "", list.map(findingCard).join(""));
      }
    }
    if (!w) html += renderSystem();
    html += renderChecks();
    html += `<p class="foot">Mom only changes a setting when you press its button. Your own settings come with an undo; the ones for the whole PC need an administrator, so Windows asks first.</p>`;
    return html;
  }

  // ------------------------------------------------------------ Upkeep: copies and big forgotten files

  const KIND_ICON = { photos: "image", videos: "film", music: "music", documents: "doc", archives: "box", installers: "download" };
  const kindIcon = (k) => ICON[KIND_ICON[k]] || ICON.file;
  const cap = (t) => (t ? t.charAt(0).toUpperCase() + t.slice(1) : t);
  const baseName = (p) => String(p).split(/[\\/]/).pop();
  const parentOf = (p) => String(p).replace(/[\\/][^\\/]*$/, "");

  async function loadFiles() {
    try {
      state.files = await invoke("files_state");
    } catch {
      state.files = { report: null, busy: false };
    }
    render();
  }

  const keptPath = (g) => state.filesKeep[g.hash] || (g.files.find((f) => f.keep) || g.files[0]).path;

  /** The extra copies in a group: every one but the copy that stays and any you said to keep (Settings, "Always kept"). */
  const extrasOf = (g) => g.files.filter((f) => f.path !== keptPath(g) && !f.kept);

  /** Everything ticked: extra copies (but never the one kept) and big files. */
  function filesChosen(r) {
    const paths = [];
    let size = 0;
    for (const g of r.groups) {
      const keep = keptPath(g);
      for (const f of g.files) {
        if (f.path !== keep && !f.kept && !state.filesUntick.has(f.path)) {
          paths.push(f.path);
          size += g.size;
        }
      }
    }
    for (const b of r.big) {
      if (state.bigTicked.has(b.path) && !b.kept && !paths.includes(b.path)) {
        paths.push(b.path);
        size += b.size;
      }
    }
    return { paths, size };
  }

  function copiesList(r) {
    const now = Date.now() / 1000;
    const groups = state.showAllCopies ? r.groups : r.groups.slice(0, 20);
    const html = groups
      .map((g) => {
        const keep = keptPath(g);
        const rows = g.files
          .map((f) => {
            const isKeep = f.path === keep || !!f.kept;
            const on = !isKeep && !state.filesUntick.has(f.path);
            const sub = `${f.folder} · ${parentOf(f.path)} · changed ${ageLong(Math.max(0, now - f.modified))} ago`;
            const box = isKeep
              ? `<span class="led green" title="${f.kept ? "You said to keep this (Settings, Always kept)" : "This copy stays"}"></span>`
              : `<input type="checkbox" data-act="files-pick" data-path="${esc(f.path)}" ${on ? "checked" : ""} aria-label="Recycle this copy">`;
            const swap = f.kept
              ? `<span class="keep-tag" title="You said to keep this. Settings, Always kept, lets it go again.">Always kept</span>`
              : isKeep
                ? `<span class="keep-tag">Stays</span>`
                : `<button class="mini-btn wide" data-act="files-keep" data-hash="${esc(g.hash)}" data-path="${esc(f.path)}">Keep this</button>`;
            return `<label class="item copy${isKeep ? " keep" : ""}" id="cp-${hash(f.path)}">${box}
              <span class="item-main"><span class="item-name" title="${esc(f.path)}">${esc(baseName(f.path))}</span><span class="item-sub" title="${esc(sub)}">${esc(sub)}</span></span>
              ${swap}<button class="mini-btn" data-act="reveal-path" data-path="${esc(f.path)}" title="Show in File Explorer">${ICON.folder}</button></label>`;
          })
          .join("");
        return `<div class="copy-group" id="cg-${esc(g.hash.slice(0, 16))}">
            <div class="copy-head"><span class="kind-ico">${kindIcon(g.kind)}</span><strong>${esc(cap(g.kind))}</strong>
              <span class="dim num">${bytes(g.size)} × ${g.files.length}</span><span class="num copy-extra">${bytes(g.size * (g.files.length - 1))} extra</span></div>
            <div class="items">${rows}</div></div>`;
      })
      .join("");
    const more = r.groups.length > 20 ? `<button class="quiet-toggle" data-act="toggle-copies">${state.showAllCopies ? "Show fewer" : `Show all ${r.groups.length}`}</button>` : "";
    return html + more;
  }

  function bigList(r) {
    const now = Date.now() / 1000;
    return `<div class="items">${r.big
      .map((b) => {
        const when = r.accessTimes ? `opened ${ageLong(Math.max(0, now - b.accessed))} ago` : `changed ${ageLong(Math.max(0, now - b.modified))} ago`;
        const sub = `${b.folder} · ${parentOf(b.path)} · ${when}`;
        const ticked = !b.kept && state.bigTicked.has(b.path);
        const box = b.kept
          ? `<span class="led green" title="You said to keep this (Settings, Always kept)"></span>`
          : `<input type="checkbox" data-act="big-pick" data-path="${esc(b.path)}" ${ticked ? "checked" : ""}>`;
        return `<label class="item${ticked ? " added" : " stays"}" id="bf-${hash(b.path)}">
          ${box}
          <span class="item-main"><span class="item-name" title="${esc(b.path)}">${esc(baseName(b.path))}</span><span class="item-sub" title="${esc(sub)}">${esc(sub)}</span></span>
          ${b.kept ? `<span class="keep-tag" title="You said to keep this. Settings, Always kept, lets it go again.">Always kept</span>` : ""}<span class="item-size num">${bytes(b.size)}</span>
          <button class="mini-btn" data-act="reveal-path" data-path="${esc(b.path)}" title="Show in File Explorer">${ICON.folder}</button></label>`;
      })
      .join("")}</div>`;
  }

  function renderFiles() {
    const f = state.files;
    if (!f) return `<p class="empty">Checking…</p>`;
    const r = f.report;
    const prog = state.filesProgress;
    const now = Date.now() / 1000;
    let said;
    if (f.busy) said = prog ? `${prog.stage === "comparing" ? "Comparing" : "Looking through"} ${plural(prog.files, "file")} (${bytes(prog.bytes)})…` : "Looking…";
    else if (!r) said = "Copies of the same file, and big files nobody opens.";
    else said = r.groups.length ? "in extra copies of the same files." : "No copies of the same file.";
    const small = f.busy
      ? ""
      : !r
        ? "Mom looks through the places below. She only reads; nothing moves until you say."
        : `Looked through ${plural(r.scannedFiles, "file")} ${ageLong(Math.max(0, now - r.measuredAt))} ago${r.complete ? "" : ", but was stopped early"}.`;
    let html = `<div class="lead"><div class="lead-text">${r && r.groups.length && !f.busy ? `<div class="big">${sizeDots(r.wasted)}</div>` : ""}<p>${esc(said)}${small ? `<small>${esc(small)}</small>` : ""}</p></div>
      ${f.busy ? `<button class="secondary" data-act="files-stop">${ICON.stop}Stop</button>` : `<button class="secondary" data-act="files-scan">${r ? "Look again" : "Look"}</button>`}</div>`;
    if (f.busy && prog) html += `<div class="progress solo"><div class="progress-bar unknown"><span></span></div></div>`;
    html += placesBox();
    if (!r || f.busy) return html;
    const note = `<p class="group-note">A copy is only ever another file with exactly the same contents: the same size, the same fingerprint (BLAKE3) and, right before anything goes, the same bytes as the copy that stays. Ticked ones go to the Recycle Bin, and mom never sends the last copy of anything.</p>`;
    const extras = r.groups.flatMap((g) => extrasOf(g).map((f) => f.path));
    const allExtras = extras.every((x) => !state.filesUntick.has(x));
    const allBig = r.big.every((b) => b.kept || state.bigTicked.has(b.path));
    if (r.groups.length) html += section("copies", "Copies of the same file", tickAllButton("files-pick-all", allExtras), copiesList(r) + note);
    if (r.big.length) html += section("big", r.accessTimes ? "Big files nobody has opened in a year" : "Big files nobody has changed in a year", tickAllButton("big-pick-all", allBig), bigList(r));
    if (r.groups.length || r.big.length) {
      const c = filesChosen(r);
      const label = state.filesWorking ? "Moving…" : !c.paths.length ? "Nothing ticked" : state.filesConfirm ? `Sure? Press again to move ${plural(c.paths.length, "file")}` : `Move ${plural(c.paths.length, "file")} (${bytes(c.size)}) to the Recycle Bin`;
      html += `<div class="files-foot"><button class="primary${state.filesConfirm ? " armed" : ""}" data-act="files-recycle" ${!c.paths.length || state.filesWorking ? "disabled" : ""}>${esc(label)}</button></div>`;
    } else html += `<p class="foot">No copies, and no big files gathering dust. Tidy.</p>`;
    return html;
  }

  /** Where the Files scan looks: your usual folders, every drive, folders you added. */
  function placesBox() {
    const pl = state.filesPlaces;
    if (!pl) return "";
    const busy = state.files && state.files.busy;
    const chip = (x) => {
      const sub = x.kind === "drive" ? `${bytes(x.free)} free of ${bytes(x.total)}` : x.kind === "added" ? "added by you" : "";
      return `<button class="place${x.on ? " on" : ""}" data-act="files-place" data-path="${esc(x.path)}" aria-pressed="${x.on}" title="${esc(x.path)}" ${busy ? "disabled" : ""}>
        <span class="place-led"></span><span class="place-name">${esc(x.name)}</span>${sub ? `<span class="place-sub num">${esc(sub)}</span>` : ""}</button>`;
    };
    const folders = pl.filter((x) => x.kind !== "drive").map(chip).join("");
    const drives = pl.filter((x) => x.kind === "drive").map(chip).join("");
    return section(
      "places",
      "Where mom looks",
      `${pl.filter((x) => x.on).length} on`,
      `<div class="places">${folders}<button class="place add" data-act="files-add-place" ${busy ? "disabled" : ""}>${ICON.folder}<span class="place-name">Add a folder</span></button></div>
       ${drives ? `<h4 class="places-head">Whole drives</h4><div class="places">${drives}</div>` : ""}
       <p class="group-note">On a whole drive, mom skips Windows, your programs, game libraries and the system's own folders: copies in there are on purpose. Projects are skipped everywhere.</p>`
    );
  }

  async function loadPlaces() {
    try {
      state.filesPlaces = await invoke("files_places");
    } catch (e) {
      console.warn("no places", e);
    }
    if (state.tab === "upkeep" && state.upkeepTab === "files") render();
  }

  // The details sheet for one clean-up: what's in it, what goes, what stays and why.

  // Clean-ups that delete your own files can send them to the Recycle Bin instead. Docker's things
  // aren't files, and admin clean-ups run as another user, whose bin you wouldn't see.
  const canRecycle = (s) => s.action === "delete" && !s.admin && s.id !== "docker";
  const recycling = (s) => canRecycle(s) && state.recycle;

  const ACTION_WORD = {
    delete: "Deleted for good",
    recycle: "Goes to the Recycle Bin",
    tool: "Cleaned by its own tool",
    "empty-bin": "Emptied for good",
    move: "Moved, with a link left behind",
  };

  /** "F:" for "F:\momputer-archive". */
  const driveOf = (dir) => (dir && /^[a-z]:/i.test(dir) ? dir.slice(0, 2).toUpperCase() : "the other drive");

  async function loadAdminCheck() {
    if (state.adminBlock !== undefined) return;
    try {
      state.adminBlock = (await invoke("admin_check")) || null;
    } catch {
      state.adminBlock = null;
    }
    renderSheet();
  }

  async function loadArchive() {
    try {
      state.archive = await invoke("archive_info");
    } catch {
      state.archive = null;
    }
    renderSheet();
  }

  /** House rule for the whole widget: every list you can tick has one of these. */
  const tickAllButton = (act, allOn, attrs = "", disabled = false) =>
    `<button class="link-btn" data-act="${act}" ${attrs} ${disabled ? "disabled" : ""}>${allOn ? "Untick all" : "Tick all"}</button>`;

  // What's ticked in a clean-up's two lists. Goes starts all ticked and Stays all unticked, and
  // "Tick all" / "Untick all" flip a whole list, so they also cover the smaller items that
  // aren't listed. `ticked` and `unticked` are the listed items changed from that.
  const pickOf = (id) => state.picks[id] || (state.picks[id] = { goes: true, new: false, ticked: new Set(), unticked: new Set() });
  const tickable = (i) => i.goes || i.optional;
  const startsTicked = (pk, i) => (i.goes ? pk.goes : pk.new);
  const isTicked = (pk, i) => (startsTicked(pk, i) ? !pk.unticked.has(i.path) : pk.ticked.has(i.path));
  const allTicked = (pk, base, list) => base && list.every((i) => isTicked(pk, i));

  /** What the button takes, counting items too small to be listed. */
  function picked(s) {
    const pk = pickOf(s.id);
    const items = s.items || [];
    const goes = items.filter((i) => i.goes);
    const fresh = items.filter((i) => !i.goes && i.optional);
    const sum = (list) => list.reduce((a, i) => a + i.bytes, 0);
    // A list that starts ticked takes everything in it (listed or not) but what's unticked;
    // otherwise just what's ticked.
    const share = (list, base, n, b) => {
      const off = list.filter((i) => !isTicked(pk, i));
      const on = list.filter((i) => isTicked(pk, i));
      return base ? [Math.max(0, n - off.length), Math.max(0, b - sum(off))] : [on.length, sum(on)];
    };
    const [goN, goB] = share(goes, pk.goes, s.reclaimableItems || goes.length, s.reclaimableBytes || 0);
    const [newN, newB] = share(fresh, pk.new, s.newItems || fresh.length, s.newBytes || 0);
    return {
      pk,
      goes,
      fresh,
      goN,
      newN,
      count: goN + newN,
      bytesChosen: goB + newB,
      partial: !pk.goes || pk.new || pk.ticked.size > 0 || pk.unticked.size > 0,
    };
  }

  /** "12 items (3.4 GB)", or "(all empty)" when they're empty folders. */
  const itemsSize = (n, b) => `${plural(n, "item")} (${b ? bytes(b) : n === 1 ? "empty" : "all empty"})`;

  /** The spot's own explanation, adjusted for what the user ticked and unticked. */
  function effectNow(s, p) {
    if (!p.partial) return s.effect;
    if (p.count === 0) return "Nothing is ticked, so the button does nothing.";
    const what = itemsSize(p.count, p.bytesChosen);
    if (p.goN === 0) {
      // Only things from Stays: the spot's own text doesn't cover them, so say it plainly.
      const one = p.count === 1;
      return s.action === "recycle"
        ? `Moves the ${what} you ticked to the Recycle Bin. Changed your mind? Restore ${one ? "it" : "them"} from there.`
        : `Deletes the ${what} you ticked, permanently (${one ? "it doesn't" : "they don't"} go to the Recycle Bin).`;
    }
    const left = (s.reclaimableItems || p.goes.length) - p.goN;
    const changed = [left > 0 && `unticked ${left.toLocaleString("en-US")}`, p.newN > 0 && `added ${p.newN.toLocaleString("en-US")} from Stays`].filter(Boolean).join(" and ");
    return `${s.effect} You ${changed}, so ${what} will go.`;
  }

  function cleanLabel(s, p, confirming) {
    const drive = driveOf(state.archive && state.archive.dir);
    const toBin = s.action === "recycle" || recycling(s);
    if (state.cleaning === s.id) return toBin ? "Moving to the Recycle Bin…" : s.action === "move" ? `Moving to ${drive}…` : "Cleaning…";
    if (s.action !== "tool" && p.count === 0) return (s.items || []).some(tickable) ? "Nothing ticked" : "Nothing to clear";
    if (confirming) {
      if (s.action === "recycle") return `Sure? Press again to move ${plural(p.count, "file")}`;
      if (toBin) return `Sure? Press again to recycle ${plural(p.count, "item")}`;
      if (s.action === "move") return `Sure? Press again to move ${plural(p.count, "folder")}`;
      return s.action === "tool" ? "Sure? Press again to run it" : "Sure? This can't be undone";
    }
    switch (s.action) {
      case "recycle":
        return `Move ${plural(p.count, "file")} to the Recycle Bin`;
      case "tool":
        return s.exact && s.reclaimableBytes ? `Clean up ${bytes(s.reclaimableBytes)}` : "Clean up";
      case "empty-bin":
        return `Empty the Recycle Bin (${bytes(p.bytesChosen)})`;
      case "move":
        return `Move ${plural(p.count, "folder")} to ${drive}`;
      default:
        if (toBin) return p.bytesChosen ? `Recycle ${bytes(p.bytesChosen)}` : `Recycle ${plural(p.count, "empty item")}`;
        return p.bytesChosen ? `Delete ${bytes(p.bytesChosen)}` : `Delete ${plural(p.count, "empty item")}`;
    }
  }

  // What mom deletes or moves herself can be kept for good. A tool's things, Docker's and the
  // Recycle Bin's can't: she doesn't choose among those one by one.
  const canKeep = (s, i) => s.action !== "tool" && s.action !== "empty-bin" && !!i.path && !i.path.startsWith("docker:");

  function itemRow(s, i, picking, label) {
    // One that stays says why first: that's the question, and the end of a long line is cut off.
    const why = i.reason || (i.ageSecs ? `changed ${ageLong(i.ageSecs)} ago` : "");
    const sub = (i.goes ? [i.sub, why] : [why, i.sub]).filter(Boolean).join(" · ");
    const id = `it-${hash(i.path || i.name)}`;
    const shown = label || i.name;
    const main = `<span class="item-main"><span class="item-name"${shown !== i.name ? ` title="${esc(i.name)}"` : ""}>${esc(shown)}</span>${sub ? `<span class="item-sub" title="${esc(sub)}">${esc(sub)}</span>` : ""}</span>`;
    // Docker's things aren't files: no folder button for them.
    const reveal = i.path && s.action !== "tool" && !i.path.startsWith("docker:") ? `<button class="mini-btn" data-act="reveal-path" data-path="${esc(i.path)}" title="Show in File Explorer" aria-label="Show ${esc(i.name)} in File Explorer">${ICON.folder}</button>` : "";
    const size = `<span class="item-size num">${bytes(i.bytes)}</span>`;
    const gone = state.gone.has(i.path) ? " gone" : "";
    const keep = canKeep(s, i)
      ? `<button class="mini-btn keep" data-act="keep-item" data-spot="${esc(s.id)}" data-path="${esc(i.path)}" aria-pressed="${!!i.kept}" title="${
          i.kept ? "Kept for good. Press to let it go again" : "Always keep this"
        }" aria-label="${i.kept ? "Stop keeping" : "Always keep"} ${esc(i.name)}" ${state.cleaning ? "disabled" : ""}>${ICON.lock}</button>`
      : "";
    const acts = keep || reveal ? `<span class="item-acts">${keep}${reveal}</span>` : "";
    if (picking && tickable(i)) {
      const on = isTicked(pickOf(s.id), i);
      return `<label class="item${i.goes ? "" : " stays"}${on && !i.goes ? " added" : ""}${gone}" id="${id}">
        <input type="checkbox" data-act="pick" data-path="${esc(i.path)}" ${on ? "checked" : ""} ${state.cleaning ? "disabled" : ""}>${main}${size}${acts}</label>`;
    }
    if (!i.goes) {
      // In use, kept by you, or otherwise not allowed: no box to tick.
      return `<div class="item stays${i.kept ? " kept" : ""}" id="${id}"><span class="led green" title="Can't be cleared: ${esc(i.reason)}"></span>${main}${size}${acts}</div>`;
    }
    return `<div class="item${gone}" id="${id}"><span class="led red"></span>${main}${size}${acts}</div>`;
  }

  /** After keeping or letting go: if it didn't take, show what mom really has, not what was hoped. */
  async function keepSettled(ok) {
    // What's kept is marked in the Files list too.
    if (state.files && state.files.report && !state.files.busy) {
      invoke("files_state")
        .then((f) => {
          state.files = f;
          if (state.tab === "upkeep" && state.upkeepTab === "files") render();
        })
        .catch(() => {});
    }
    if (ok !== null) return;
    try {
      state.trash = await invoke("get_trash");
      state.settings = await invoke("get_settings");
    } catch {}
    if (state.tab === "upkeep") render();
    renderSheet();
  }

  /** "…and 123 smaller items", with the button that lists them (four times as many at a time). */
  const moreButton = (s, unlisted, listed) =>
    `<button class="link-btn" data-act="show-more" data-spot="${esc(s.id)}" ${state.trash.busy || state.cleaning ? "disabled" : ""}>${unlisted <= listed * 3 ? "Show all" : "Show more"}</button>`;

  // Lists group by what comes before the first " · " in a name ("Unity games · Lifeslide ·
  // Player.log" belongs to Unity games), or by the sort of thing (Docker's images, containers).
  const SEP = " · ";
  const KIND_NAMES = { images: "Images", "stopped containers": "Stopped containers", "build cache": "Build cache", volumes: "Volumes" };
  function groupOf(i) {
    const parts = i.name.split(SEP);
    if (parts.length >= 2) return [parts[0], parts.slice(1)];
    return [i.kind ? KIND_NAMES[i.kind] || cap(i.kind) : "", parts];
  }
  const ORDER = {
    size: (a, b) => b.bytes - a.bytes,
    age: (a, b) => (b.age || 0) - (a.age || 0),
    name: (a, b) => a.name.localeCompare(b.name),
  };
  const matches = (i, q) => !q || `${i.name} ${i.sub || ""} ${i.reason || ""}`.toLowerCase().includes(q);
  const listQuery = () => state.listView.q.trim().toLowerCase();

  /** A group's (or an item's) place in the list: its size, oldest age and name. */
  const weigh = (name, items) => ({ name, bytes: items.reduce((a, i) => a + i.bytes, 0), age: Math.max(0, ...items.map((i) => i.ageSecs || 0)) });

  function listTools() {
    const v = state.listView;
    const sort = (k, label) => `<button class="seg-btn" data-act="list-sort" data-sort="${k}" aria-pressed="${v.sort === k}">${label}</button>`;
    return `<div class="list-tools">
        <label class="list-search">${ICON.search}<input type="search" id="list-q" placeholder="Filter" value="${esc(v.q)}" autocomplete="off" spellcheck="false" aria-label="Filter the lists"></label>
        <div class="seg" role="group" aria-label="Sort by">${sort("size", "Size")}${sort("age", "Oldest")}${sort("name", "A–Z")}</div>
        <button class="seg-btn solo" data-act="list-group" aria-pressed="${v.group}" title="${v.group ? "Grouped by app" : "Group by app"}" aria-label="Group by app">${ICON.layers}</button>
      </div>`;
  }

  function listBody(s, items, picking, list) {
    const v = state.listView;
    const q = listQuery();
    const shown = items.filter((i) => matches(i, q));
    if (!shown.length) return `<p class="more">Nothing here matches “${esc(v.q.trim())}”.</p>`;
    const cmp = ORDER[v.sort] || ORDER.size;
    const one = (i) => ({ ...weigh(i.name, [i]), row: () => itemRow(s, i, picking) });
    if (!v.group) return shown.map(one).sort(cmp).map((e) => e.row()).join("");
    const groups = new Map();
    for (const i of shown) {
      const [key, rest] = groupOf(i);
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push({ i, rest });
    }
    const real = [...groups].filter(([k, m]) => k && m.length > 1);
    // One group holding everything isn't worth a header.
    if (!real.length || (real.length === 1 && real[0][1].length === shown.length)) return shown.map(one).sort(cmp).map((e) => e.row()).join("");
    const shut = shown.length > 12 && real.length > 1;
    const entries = [];
    for (const [k, members] of groups) {
      if (k && members.length > 1) entries.push({ ...weigh(k, members.map((m) => m.i)), row: () => groupBlock(s, list, k, members, picking, shut, cmp) });
      else for (const m of members) entries.push(one(m.i));
    }
    return entries.sort(cmp).map((e) => e.row()).join("");
  }

  function groupBlock(s, list, key, members, picking, shut, cmp) {
    const gid = `${s.id}:${list}:${key}`;
    const open = listQuery() ? true : gid in state.groupOpen ? state.groupOpen[gid] : !shut;
    const items = members.map((m) => m.i);
    const total = items.reduce((a, i) => a + i.bytes, 0);
    const pk = pickOf(s.id);
    const tick = items.filter(tickable);
    let box = "<span></span>";
    if (picking && tick.length) {
      const on = tick.filter((i) => isTicked(pk, i)).length;
      const st = on === 0 ? "none" : on === tick.length ? "all" : "some";
      box = `<button class="tick ${st}" data-act="pick-group" data-group="${esc(gid)}" role="checkbox" aria-checked="${st === "all" ? "true" : st === "some" ? "mixed" : "false"}" aria-label="Tick all of ${esc(key)}" ${state.cleaning ? "disabled" : ""}>${ICON.check}</button>`;
    }
    const goneAll = items.every((i) => state.gone.has(i.path));
    const head = `<div class="lgroup-head" data-act="toggle-group" data-group="${esc(gid)}" role="button" tabindex="0" aria-expanded="${open}">
        ${box}<span class="lgroup-name">${esc(key)}</span><span class="lgroup-count num">${items.length}</span><span class="item-size num">${bytes(total)}</span><span class="lgroup-chev">${ICON.chevron}</span></div>`;
    const cls = `lgroup${open ? " open" : ""}${goneAll ? " gone" : ""}`;
    if (!open) return `<div class="${cls}" id="g-${hash(gid)}">${head}</div>`;
    // Inside: by the next part of the name when several items share it (a game, a tool).
    const subs = new Map();
    for (const m of members) {
      const sk = m.rest.length >= 2 ? m.rest[0] : "";
      if (!subs.has(sk)) subs.set(sk, []);
      subs.get(sk).push(m);
    }
    const nested = members.length > 4 && subs.size > 1 && [...subs.values()].some((x) => x.length > 1);
    let body;
    if (nested) {
      const parts = [];
      for (const [sk, ms] of subs) {
        if (sk && ms.length > 1) {
          const w = weigh(sk, ms.map((m) => m.i));
          parts.push({ ...w, row: () => `<div class="subgroup"><span>${esc(sk)}</span><span class="num">${bytes(w.bytes)}</span></div>` + ms.map((m) => ({ ...weigh(m.i.name, [m.i]), m })).sort(cmp).map((x) => itemRow(s, x.m.i, picking, x.m.rest.slice(1).join(SEP))).join("") });
        } else for (const m of ms) parts.push({ ...weigh(m.i.name, [m.i]), row: () => itemRow(s, m.i, picking, m.rest.join(SEP)) });
      }
      body = parts.sort(cmp).map((x) => x.row()).join("");
    } else body = members.map((m) => ({ ...weigh(m.i.name, [m.i]), m })).sort(cmp).map((x) => itemRow(s, x.m.i, picking, x.m.rest.join(SEP))).join("");
    return `<div class="${cls}" id="g-${hash(gid)}">${head}<div class="lgroup-body">${body}</div></div>`;
  }

  /** What the button does, as a few cards, with the whole story behind "Read more". */
  function doesBox(s, p) {
    const n = p.count;
    const b = p.bytesChosen;
    const drive = driveOf(state.archive && state.archive.dir);
    const cards = [];
    const card = (ico, big, small, tone = "") => cards.push(`<div class="does-card${tone ? ` ${tone}` : ""}">${ico}<div><strong>${esc(big)}</strong><span>${esc(small)}</span></div></div>`);
    if (s.action === "tool") card(ICON.wrench, "Runs its own tool", s.exact && s.reclaimableBytes ? `up to ${bytes(s.reclaimableBytes)}` : "it decides what goes", "go");
    else if (n === 0) card(ICON.check, "Nothing to do", (s.items || []).some(tickable) ? "nothing is ticked" : "it's already clean");
    else if (s.action === "recycle") card(ICON.undo, `Recycles ${plural(n, "file")}`, `${bytes(b)}, and you can restore ${n === 1 ? "it" : "them"}`, "go");
    else if (recycling(s)) card(ICON.undo, `Recycles ${plural(n, "item")}`, `${b ? bytes(b) : "all empty"}, and you can restore ${n === 1 ? "it" : "them"}`, "go");
    else if (s.action === "move") card(ICON.archive, `Moves ${plural(n, "folder")}`, `${bytes(b)} to ${drive}; ${n === 1 ? "it still opens" : "they still open"} from here`, "go");
    else card(ICON.trash, `Deletes ${plural(n, "item")}`, `${b ? bytes(b) : "all empty"}, for good`, "go");
    const keptN = s.keptItems || 0;
    if (keptN) card(ICON.lock, `${plural(keptN, "item")} stay`, s.newItems ? `${s.newItems.toLocaleString("en-US")} you can tick` : "in use, or not old enough", "stay");
    if (s.safety === "review") card(ICON.eye, "Look first", "untick anything you want to keep");
    else if (recycling(s)) card(ICON.shield, "Safe", "anything too big for the Recycle Bin stays put");
    else card(ICON.shield, "Safe", s.action === "move" ? "nothing is deleted" : s.action === "recycle" ? "it all waits in the Recycle Bin" : "nothing you'd miss, or it comes back");
    if (s.admin) card(ICON.key, "Windows asks first", "only an administrator can");
    const choice = canRecycle(s)
      ? `<div class="seg does-choice" role="group" aria-label="Where it goes">
          <button class="seg-btn" data-act="bin-mode" data-recycle="0" aria-pressed="${!state.recycle}" ${state.cleaning ? "disabled" : ""}>Delete for good</button>
          <button class="seg-btn" data-act="bin-mode" data-recycle="1" aria-pressed="${state.recycle}" ${state.cleaning ? "disabled" : ""}>Recycle Bin</button></div>`
      : "";
    return `<div class="does"><div class="does-head"><h3>What the button does</h3>${choice}</div><div class="does-cards">${cards.join("")}</div>
        <button class="does-more" data-act="toggle-does" aria-expanded="${state.doesOpen}">${state.doesOpen ? "Less" : "Read more"}${ICON.chevron}</button>
        ${state.doesOpen ? `<p class="does-text">${esc(effectNow(s, p))}</p>` : ""}</div>`;
  }

  /** While a clean-up runs: how far along it is, what it's on, and a way to stop it. */
  function cleanProgress(s) {
    const lv = state.live || {};
    const total = lv.total || 0;
    const known = total > 0;
    const pct = lv.totalFiles ? Math.min(1, lv.files / lv.totalFiles) : known ? lv.done / total : 0;
    const now = lv.stopping
      ? "Stopping after this one…"
      : lv.current
        ? lv.current
        : s.admin
          ? "Waiting for Windows to say yes…"
          : s.action === "move"
            ? "Copying…"
            : "Starting…";
    const counts = known ? `${(lv.done || 0).toLocaleString("en-US")} of ${plural(total, "item")} · ${bytes(lv.freed || 0)}` : "";
    return `<div class="progress" role="status" aria-live="polite">
        <div class="progress-bar${known ? "" : " unknown"}"><span style="width:${(pct * 100).toFixed(1)}%"></span></div>
        <div class="progress-row"><span class="progress-now">${esc(now)}</span><span class="num">${counts}</span></div>
      </div>
      <button class="secondary" data-act="stop-clean" ${lv.stopping || !known ? "disabled" : ""}>${ICON.stop}Stop</button>`;
  }

  /** Watches a clean-up while it runs, so the list shows what's gone as it goes. */
  async function watchClean(id) {
    let since = 0;
    while (state.cleaning === id) {
      await new Promise((r) => setTimeout(r, 250));
      if (state.cleaning !== id) break;
      try {
        const now = await invoke("clean_progress", { since });
        if (state.cleaning !== id) break;
        since = now.goneCount;
        for (const g of now.gone) state.gone.add(g);
        state.live = now;
        renderSheet();
      } catch {
        break;
      }
    }
  }

  // Admin clean-ups: Windows asks first, and only from a copy installed for all users.
  function adminHint() {
    if (state.adminBlock)
      return `<p class="sheet-note"><span class="led red"></span>This copy of momputer can't ask Windows for an administrator, so the button cleans only what you can clean yourself. ${esc(state.adminBlock)}</p>`;
    return `<p class="list-hint">Only an administrator can clean this, so when you press the button Windows asks whether mom may make changes. Mom itself never runs as administrator.</p>`;
  }

  function renderSpot(id) {
    const s = state.trash.spots.find((x) => x.id === id);
    if (!s) return `<div class="sheet-head"><h2 id="sheet-title">Gone</h2><button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button></div><p class="empty">That one isn't there any more.</p>`;
    const p = picked(s);
    const pk = p.pk;
    const stays = (s.items || []).filter((i) => !i.goes);
    const picking = s.action !== "tool" && (p.goes.length > 0 || p.fresh.length > 0);
    const unlisted = Math.max(0, (s.reclaimableItems || 0) - p.goes.length);
    const keptUnlisted = Math.max(0, (s.keptItems || 0) - stays.length);
    const newUnlisted = Math.max(0, (s.newItems || 0) - p.fresh.length);
    const allButton = (list, items, base) => (picking && items.length ? tickAllButton("pick-all", allTicked(pk, base, items), `data-list="${list}"`, !!state.cleaning) : "");
    const hint =
      (p.fresh.length === stays.length ? "These are" : "The ones with a box are") +
      (s.action === "recycle"
        ? " kept only because they're new. Tick any you want gone too."
        : " kept only because they're new, and something might still want them. Tick any you're sure about.");
    const go = s.reclaimableBytes || 0;
    const kept = s.keptBytes ?? Math.max(0, s.totalBytes - go);
    const cleaning = state.cleaning === s.id;
    const confirming = !cleaning && state.cleanConfirm === s.id;
    // Empty folders are 0 bytes but still worth clearing, so this goes by count, not size.
    const disabled = cleaning || (s.action !== "tool" && p.count === 0);
    const danger = (s.action === "delete" && !recycling(s)) || s.action === "empty-bin";
    const moves = s.action === "move";
    const busy = state.trash.busy;
    const tools = (s.items || []).length > 8 ? listTools() : "";
    const lastGoes = p.goes[p.goes.length - 1];
    return `<div class="sheet-head">
        <div><h2 id="sheet-title">${esc(s.label)}</h2>
          <div class="pills"><span class="pill"><span class="led ${s.safety === "review" ? "red" : "green"}"></span>${s.safety === "review" ? "Look before you clear" : s.action === "move" ? "Nothing is deleted" : "Safe to clear"}</span>
          ${s.admin ? `<span class="pill"><span class="led red"></span>${state.adminBlock ? "Needs an all-users install" : "Windows asks first"}</span>` : ""}
          <span class="pill">${recycling(s) ? ACTION_WORD.recycle : ACTION_WORD[s.action] || "Deleted"}</span></div></div>
        <div class="head-acts">
          <button class="icon-btn${busy ? " spinning" : ""}" data-act="refresh-spot" data-spot="${esc(s.id)}" title="${busy ? "Measuring…" : "Check again"}" aria-label="Check again" ${busy || state.cleaning ? "disabled" : ""}>${ICON.refresh}</button>
          <button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button>
        </div>
      </div>
      <p class="sheet-what">${esc(s.what)}</p>
      ${
        s.exact
          ? `<div class="split"><div class="split-bar"><span class="go" style="flex:${go || 0.0001}"></span><span class="stay" style="flex:${kept || 0.0001}"></span></div>
        <div class="split-legend num"><span><span class="led red"></span><strong>${bytes(go)}</strong> ${moves ? "can move" : "can go"}</span><span><span class="led green"></span>${bytes(kept)} ${moves ? "stays here" : "stays"}</span><span class="dim">${bytes(s.totalBytes)} in all</span></div></div>`
          : `<div class="split-legend num"><span>${bytes(s.totalBytes)} in all. The tool works out what's safe to drop.</span></div>`
      }
      ${doesBox(s, p)}
      ${s.action === "move" ? archiveBox(s.id) : ""}
      ${s.admin ? adminHint() : ""}
      ${s.note ? `<p class="sheet-note"><span class="led red"></span>${esc(s.note)}</p>` : ""}
      ${tools}
      ${
        p.goes.length
          ? `<h3 class="list-head">${moves ? "Moves" : "Goes"} <span class="dim num">${plural(s.reclaimableItems || p.goes.length, "item")}</span>${allButton("goes", p.goes, pk.goes)}</h3>
        <div class="items">${listBody(s, p.goes, picking, "goes")}${
          unlisted
            ? `<p class="more"><span>…and ${plural(unlisted, lastGoes.bytes ? "smaller item" : "more", lastGoes.bytes ? "smaller items" : "more")}${
                picking ? (pk.goes ? ", ticked too" : ", not ticked") : ""
              }.</span>${moreButton(s, unlisted, p.goes.length)}</p>`
            : ""
        }</div>`
          : ""
      }
      ${
        stays.length
          ? `<h3 class="list-head">${moves ? "Stays here" : "Stays"} <span class="dim num">${plural(s.keptItems || stays.length, "item")}</span>${allButton("new", p.fresh, pk.new)}</h3>
        ${picking && p.fresh.length ? `<p class="list-hint">${hint}</p>` : ""}
        <div class="items">${listBody(s, stays, picking, "stays")}${
          keptUnlisted
            ? `<p class="more"><span>…and ${plural(keptUnlisted, "more", "more")}${
                picking && newUnlisted
                  ? `, ${newUnlisted === keptUnlisted ? "all" : `${newUnlisted.toLocaleString("en-US")} of them`} new (${pk.new ? "ticked too" : "Tick all takes them too"})`
                  : ""
              }.</span>${moreButton(s, keptUnlisted, stays.length)}</p>`
            : ""
        }</div>`
          : ""
      }
      <div class="sheet-foot${cleaning ? " cleaning" : ""}">
        ${
          cleaning
            ? cleanProgress(s)
            : `${
                confirming
                  ? `<button class="link-btn" data-act="cancel-clean">Cancel</button>`
                  : s.path
                    ? `<button class="link-btn" data-act="reveal-path" data-path="${esc(s.path)}" title="Show in File Explorer">${ICON.folder}Show folder</button>`
                    : "<span></span>"
              }
        ${
          s.action === "move" && !(state.archive && state.archive.dir)
            ? `<button class="primary" data-act="choose-archive">Choose a drive</button>`
            : `<button class="primary${confirming ? (danger ? " danger" : " armed") : ""}" id="clean-btn" data-act="clean-spot" data-spot="${esc(s.id)}" ${disabled ? "disabled" : ""}>${
                confirming ? '<span class="led red blink"></span>' : danger ? '<span class="led red"></span>' : ""
              }${esc(cleanLabel(s, p, confirming))}</button>`
        }`
        }
      </div>`;
  }

  /** Where a clean-up moves things, and what it's already moved there (with a way back for each). */
  function archiveBox(spotId) {
    const a = state.archive;
    if (!a) return "";
    const where = a.dir
      ? `<div class="archive-where"><span class="led ${a.reachable ? "green" : "red"}"></span><span class="archive-path">${esc(a.dir)}</span>
          <span class="dim num">${a.reachable ? (a.free != null ? `${bytes(a.free)} free` : "") : "not connected"}</span>
          <button class="link-btn" data-act="choose-archive">Change</button></div>`
      : `<p class="list-hint">Pick a folder on another drive (an external one is fine). Mom makes a momputer-archive folder there.</p>`;
    // Moved before mom said which clean-up moved what: those were conversations.
    const moved = (a.moved || []).filter((m) => (m.spot || "old-conversations") === spotId);
    if (!moved.length) return where;
    const rows = moved
      .slice()
      .sort((x, y) => y.movedAt - x.movedAt)
      .map((m) => {
        const busy = state.bringing === m.from;
        const sure = state.deleteConfirm === m.from;
        const why = !m.reachable ? `${driveOf(a.dir)} isn't connected, so this can't be opened right now` : !m.linked ? "Its link is gone (the app may have removed it)" : `moved ${ageLong(Math.max(0, Date.now() / 1000 - m.movedAt))} ago`;
        return `<div class="item stays moved" id="mv-${hash(m.from)}"><span class="led ${m.reachable && m.linked ? "green" : "red"}"></span>
          <span class="item-main"><span class="item-name" title="${esc(m.name)}">${esc(m.name)}</span><span class="item-sub">${esc(`${bytes(m.bytes)} · ${why}`)}</span></span>
          <span class="moved-acts"><button class="mini-btn wide" data-act="bring-back" data-from="${esc(m.from)}" ${busy || !m.reachable || state.bringing ? "disabled" : ""}>${busy ? "Bringing…" : "Bring back"}</button>
          <button class="mini-btn wide${sure ? " sure" : ""}" data-act="delete-moved" data-from="${esc(m.from)}" title="Delete it from ${esc(driveOf(a.dir))} for good" ${busy || state.bringing ? "disabled" : ""}>${sure ? "Sure? Delete" : "Delete"}</button></span></div>`;
      })
      .join("");
    const total = moved.reduce((t, m) => t + m.bytes, 0);
    return `${where}<h3 class="list-head">Moved already <span class="dim num">${plural(moved.length, "folder")}, ${bytes(total)}</span></h3><div class="items">${rows}</div>`;
  }

  // ------------------------------------------------------------ rooms

  function renderRooms() {
    if (state.roomOpen) return renderRoom(state.roomOpen);
    let html = `<div class="lead"><span class="lead-note">Folders mom keeps an eye on.</span><button class="secondary" data-act="add-room">Add a folder</button></div>`;
    if (!state.roomPaths.length) {
      return html + section("rooms", "Rooms", "", `<p class="empty">Add a project or data folder and mom will grade how tidy it is, and say what to fix.</p>`);
    }
    const rows = state.roomPaths
      .map((p) => {
        const r = state.rooms[p];
        const name = r ? r.name : p.split(/[\\/]/).filter(Boolean).pop() || p;
        const why = state.roomErrors[p];
        const meta = why ? why : r && r.moms ? "Mom's room. Knock first." : r ? `${plural(r.entries, "thing")} at the top level, ${r.findings.length} to sort out` : "Looking…";
        return `<div class="row room-row" id="room-${hash(p)}" data-room="${esc(p)}" data-act="open-room" tabindex="0" role="button">
          <span class="row-name">${esc(name)}</span>
          <div class="row-meta"><span class="why">${esc(meta)}</span></div>
          <div class="row-act">${r ? grade(r.grade, 3) : ""}</div>
        </div>`;
      })
      .join("");
    return html + section("rooms", "Rooms", "", rows);
  }

  /** A plan's title without the size on the end ("(13 MB)"): the size has its own column. */
  const sizeless = (t) => t.replace(/\s*\([\d.,]+\s?(?:bytes|[KMGT]?B)\)$/, "");
  /** Text with folder paths set apart, so a long one wraps instead of running off the edge. */
  const pathy = (t) =>
    esc(t)
      .split(" ")
      .map((w) => (/[\\/]/.test(w) && w.length > 1 ? `<span class="path">${w}</span>` : w))
      .join(" ");

  const grade = (g, pitch) =>
    `<span class="grade g-${esc(g)}${pitch > 3 ? " big" : ""}" aria-label="grade ${esc(g)}"><span class="dot">${esc(g)}</span></span>`;

  function renderRoom(path) {
    const r = state.rooms[path];
    let html = `<button class="back" data-act="close-room">All rooms</button>`;
    const why = state.roomErrors[path];
    if (why)
      return (
        html +
        section("room", "Mom can't tidy this one", "", `<p class="empty">${esc(why)}</p>`) +
        `<div class="button-row"><button class="link-btn" data-act="remove-room">Stop watching this folder</button></div>`
      );
    if (!r) return html + section("room", "Looking", "", `<p class="empty">Looking…</p>`);
    if (r.moms) return html + momsRoom();
    const sel = state.roomSel[path] || new Set();
    html += `<div class="room-title"><h2>${esc(r.name)}</h2>${grade(r.grade, 4)}<p class="said">${esc(r.line)}</p></div>`;
    const careful = [];
    if (r.codeProject) careful.push(`This folder has code in it. Moving things can break paths, so nothing is ticked. "Copy chores for an agent" lets an agent tidy and fix the paths.`);
    else if (r.projects >= 2)
      careful.push(`This folder holds ${r.projects} projects (git repos and packages). Mom never moves or groups them, since tools and agents find them by path. She only tidies inside them.`);
    if (r.personal) careful.push("This is one of your own folders, so mom only ticks what's plainly throwaway. Everything else waits for you.");
    if (r.partial) careful.push("It's too big to look through in full, so this is what mom saw in the first part of it.");
    if (careful.length) html += section("code", "Careful", "", careful.map((t) => `<p class="empty">${esc(t)}</p>`).join(""));
    const findings = r.findings.map((f) => `<div class="room-finding" id="f-${hash(f.title)}"><strong>${pathy(f.title)}</strong><p>${pathy(f.detail)}</p></div>`).join("");
    html += section("wrong", "What's wrong", "", findings || `<p class="empty">Nothing. Spotless.</p>`);
    if (r.actions.length) {
      const checks = r.actions
        .map(
          (a) => `<label class="check" id="a-${hash(a.id)}"><input type="checkbox" data-act="toggle-action" data-aid="${esc(a.id)}" ${sel.has(a.id) ? "checked" : ""}>
            <span class="check-main">${pathy(sizeless(a.title))}<small>${esc(a.reason)}</small></span><span class="num">${a.bytes ? bytes(a.bytes) : ""}</span></label>`
        )
        .join("");
      html += section("plan", "Mom's plan", tickAllButton("room-pick-all", r.actions.every((a) => sel.has(a.id))), checks);
    }
    html += `<div class="button-row">
      <button class="primary" data-act="apply-room" ${!sel.size || state.roomBusy ? "disabled" : ""}>Do the checked ones${sel.size ? ` (${sel.size})` : ""}</button>
      <button class="secondary" data-act="copy-chores">Copy chores for an agent</button>
      <button class="secondary" data-act="undo-room">Undo last tidy-up</button>
    </div>
    <div class="button-row">
      <button class="link-btn" data-act="rescan-room">Look again</button>
      <button class="link-btn" data-act="reveal-room">Show folder</button>
      <button class="link-btn" data-act="remove-room">Stop watching this folder</button>
    </div>
    <p class="foot">Moves can be undone. Deleted files go to the Recycle Bin.</p>`;
    return html;
  }

  // ------------------------------------------------------------ mom's room (an easter egg)
  //
  // Point mom at her own folder and she won't have it tidied. Knock, though, and she lets you in
  // to tidy it with her: a little 3D room (room3d.js), six chores, about a minute.

  const ROOM_BEST = "mom.roomBest";
  function roomBest() {
    try {
      return Number(localStorage.getItem(ROOM_BEST)) || 0;
    } catch {
      return 0;
    }
  }
  const mmss = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
  const CHORE_LINES = {
    books: "All in a row. Lovely.",
    bed: "Hospital corners. I raised you right.",
    coffee: "Oh, you shouldn't have. c:",
    laundry: "Right in the basket. Thank you, sweetie.",
    picture: "Is that me? What a lovely frame.",
    curtains: "Let the light in. Much better.",
  };
  let playroom = null; // { el, game, chores, started, timer, done }

  function momsRoom() {
    const best = roomBest();
    const body = !state.knocked
      ? `<div class="door"><div class="door-frame"><div class="door-sign dot">knock<br>first</div><span class="door-knob"></span><span class="door-light"></span></div>
          <p>This is mom's room: her settings, her journal and the house rules. Nobody tidies mom's room.</p>
          <button class="primary" data-act="knock">Knock</button></div>`
      : `<div class="door"><div class="door-frame"><div class="door-sign dot open">come<br>in</div><span class="door-knob"></span><span class="door-light"></span></div>
          <p>She'll let you help, just this once.${best ? ` Your best: ${mmss(best)}.` : ""}</p>
          <button class="primary" data-act="enter-room">Go in</button></div>`;
    return `<div class="room-title"><h2>Mom's room</h2>${grade("A", 4)}<p class="said">My room? Of course it's spotless c:</p></div>
      <div class="moms-room">${body}</div>`;
  }

  /** The playroom: a card over the whole widget with the room in it. */
  async function enterRoom() {
    if (playroom) return;
    const el = document.createElement("div");
    el.className = "playroom";
    el.setAttribute("role", "dialog");
    el.setAttribute("aria-label", "Mom's room");
    el.innerHTML = `<div class="pr-head">
        <span class="pr-mom" aria-hidden="true">${window.momFace ? window.momFace.svg("look") : ""}</span>
        <div class="pr-title"><strong class="dot">mom's room</strong><span class="pr-said" aria-live="polite">Help me tidy up, sweetie.</span></div>
        <span class="pr-time dot num">0:00</span>
        <button class="icon-btn" data-act="leave-room" aria-label="Leave mom's room">${ICON.close}</button>
      </div>
      <div class="pr-stage"><span class="pr-point" hidden></span></div>
      <div class="pr-foot"><p class="pr-hint">Opening the door…</p><div class="pr-chores"></div></div>
      <div class="pr-end" hidden></div>`;
    document.body.appendChild(el);
    playroom = { el, started: Date.now(), done: false };
    if (moving()) G.fromTo(el, { opacity: 0, scale: 0.97 }, { opacity: 1, scale: 1, duration: 0.35, ease: "power2.out" });
    let room;
    try {
      room = await import("./room3d.js");
    } catch (e) {
      console.error("mom's room", e);
    }
    if (!playroom || playroom.el !== el) return;
    const game = room && room.play(el.querySelector(".pr-stage"), { chore: choreDone, done: roomDone });
    if (!game) {
      leaveRoom();
      toast("Mom's room needs 3D graphics, and this PC won't draw them right now.", true);
      return;
    }
    Object.assign(playroom, { game, chores: room.CHORES, started: Date.now(), touched: Date.now(), timer: setInterval(drawClock, 250) });
    // Nothing tapped for a while: a ring points at the next thing to tidy.
    el.addEventListener("pointerdown", () => {
      if (!playroom) return;
      playroom.touched = Date.now();
      el.querySelector(".pr-point").hidden = true;
    });
    el.addEventListener("pointerover", (e) => {
      const chip = e.target.closest("[data-chore]");
      if (chip && playroom && playroom.game) playroom.game.highlight(chip.dataset.chore, true);
    });
    el.addEventListener("pointerout", (e) => {
      const chip = e.target.closest("[data-chore]");
      if (chip && playroom && playroom.game) playroom.game.highlight(chip.dataset.chore, false);
    });
    el.querySelector(".pr-chores").innerHTML = room.CHORES.map(
      (c) => `<button class="pr-chore" data-act="pr-chore" data-chore="${c.id}" aria-label="${esc(c.label)}"><span class="led"></span><span class="pr-label">${esc(c.label)}</span><span class="pr-n num"></span></button>`
    ).join("");
    drawChores();
  }

  function drawClock() {
    if (!playroom || playroom.done) return;
    const t = playroom.el.querySelector(".pr-time");
    if (t) t.textContent = mmss((Date.now() - playroom.started) / 1000);
    const ring = playroom.el.querySelector(".pr-point");
    if (Date.now() - playroom.touched > 6000 && ring.hidden) {
      const n = playroom.game.count();
      const next = playroom.chores.find((c) => n[c.id] < c.of);
      const at = next && playroom.game.spot(next.id);
      if (at) {
        ring.style.left = `${at.x}px`;
        ring.style.top = `${at.y}px`;
        ring.hidden = false;
      }
    }
  }

  function drawChores() {
    if (!playroom || !playroom.game) return;
    const n = playroom.game.count();
    let next = null;
    for (const c of playroom.chores) {
      const chip = playroom.el.querySelector(`[data-chore="${c.id}"]`);
      const done = n[c.id] >= c.of;
      if (!done && !next) next = c;
      chip.classList.toggle("done", done);
      chip.querySelector(".led").className = `led${done ? " green" : ""}`;
      chip.querySelector(".pr-n").textContent = c.of > 1 ? `${n[c.id]}/${c.of}` : "";
      chip.disabled = done;
    }
    playroom.el.querySelector(".pr-hint").textContent = next ? next.hint : "All done. Look at that.";
  }

  function momSays(text, mood) {
    if (!playroom) return;
    playroom.el.querySelector(".pr-said").textContent = text;
    const face = playroom.el.querySelector(".pr-mom");
    if (face && window.momFace && mood) {
      face.innerHTML = window.momFace.svg(mood);
      clearTimeout(playroom.faceTimer);
      playroom.faceTimer = setTimeout(() => playroom && !playroom.done && (face.innerHTML = window.momFace.svg("look")), 1800);
    }
  }

  function choreDone(id, n, of) {
    drawChores();
    if (n >= of) momSays(CHORE_LINES[id] || "Good.", "delighted");
    else if (n === 1) momSays("Keep going, sweetie.", "wink");
  }

  function roomDone() {
    if (!playroom) return;
    playroom.done = true;
    playroom.el.classList.add("finished");
    clearInterval(playroom.timer);
    const took = Math.max(1, Math.round((Date.now() - playroom.started) / 1000));
    const best = roomBest();
    const record = !best || took < best;
    if (record) {
      try {
        localStorage.setItem(ROOM_BEST, String(took));
      } catch {}
    }
    playroom.el.querySelector(".pr-time").textContent = mmss(took);
    momSays("Spotless. Beautifully done.", "kiss");
    const end = playroom.el.querySelector(".pr-end");
    end.innerHTML = `<div class="pr-end-leds" aria-hidden="true">${'<span class="led green"></span>'.repeat(6)}</div>
      <p class="pr-end-big dot num">${mmss(took)}</p>
      <p>${record && best ? "A new best. I'm framing this." : record ? "Spotless, and in record time." : `Spotless. Your best is ${mmss(best)}.`}</p>
      <div class="button-row"><button class="primary" data-act="room-again">Again</button><button class="secondary" data-act="leave-room">Back to the widget</button></div>`;
    end.hidden = false;
    if (moving()) G.fromTo(end, { y: 30, opacity: 0 }, { y: 0, opacity: 1, duration: 0.45, ease: "back.out(1.6)" });
    react("kiss", 3200);
  }

  function leaveRoom() {
    if (!playroom) return;
    const { el, game, timer, faceTimer } = playroom;
    playroom = null;
    clearInterval(timer);
    clearTimeout(faceTimer);
    if (game) game.close();
    if (moving()) G.to(el, { opacity: 0, duration: 0.2, onComplete: () => el.remove() });
    else el.remove();
    render();
  }

  async function loadRooms() {
    state.roomPaths = await invoke("room_paths");
    const cached = await invoke("get_rooms");
    for (const r of cached) state.rooms[r.path] = r;
    render();
    for (const p of state.roomPaths) {
      if (!state.rooms[p]) await inspectRoom(p);
    }
  }

  async function inspectRoom(p) {
    try {
      const r = await invoke("inspect_room", { path: p });
      if (r.moms && !(state.rooms[p] && state.rooms[p].moms)) {
        react("shocked", 1200);
        setTimeout(() => react("kiss", 2400), 1250);
      }
      state.rooms[p] = r;
      delete state.roomErrors[p];
      state.roomSel[p] = new Set(r.actions.filter((a) => a.selected).map((a) => a.id));
    } catch (e) {
      state.roomErrors[p] = String(e);
    }
    render();
  }

  // ------------------------------------------------------------ house rules (just enough markdown)

  function inline(text) {
    // Code first, so **bold `code`** works and nothing inside backticks gets formatted.
    const codes = [];
    return esc(text)
      .replace(/`([^`]+)`/g, (_, c) => `\u0000${codes.push(c) - 1}\u0000`)
      .replace(/\*\*(.+?)\*\*|__(.+?)__/g, (_, a, b) => `<strong>${a ?? b}</strong>`)
      .replace(/(^|[\s(])[*_](?=\S)(.+?)(?<=\S)[*_](?=[\s).,;:!?]|$)/g, "$1<em>$2</em>")
      .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<span class="md-link" title="$2">$1</span>')
      .replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${codes[i]}</code>`);
  }

  /** `front`: the file may start with a block of facts between two --- lines (a skill does);
      show it as a small table instead of as text. */
  function markdown(src, front = false) {
    const out = [];
    let para = [];
    let list = null;
    let fence = null;
    let table = null;
    src = String(src).replace(/\r/g, "");
    if (front && /^---\n/.test(src)) {
      const end = src.indexOf("\n---", 3);
      if (end > 0) {
        // "key: value" lines; an indented line belongs to the key above it.
        const facts = [];
        const clean = (v) => v.trim().replace(/^(["'])(.*)\1$/, "$2");
        const pair = /^([A-Za-z][\w-]*):\s*(.*)$/;
        for (const line of src.slice(4, end).split("\n")) {
          const m = line.match(pair);
          const inner = line.trim().match(pair);
          if (m) facts.push([m[1], clean(m[2])]);
          else if (facts.length && line.trim()) facts[facts.length - 1][1] += `${facts[facts.length - 1][1] ? " · " : ""}${inner ? `${inner[1]} ${clean(inner[2])}` : line.trim()}`;
        }
        if (facts.length) out.push(`<dl class="md-front">${facts.map(([k, v]) => `<div><dt>${esc(k)}</dt><dd>${esc(v)}</dd></div>`).join("")}</dl>`);
        src = src.slice(end + 4).replace(/^[^\n]*\n?/, "");
      }
    }
    const cells = (line) => line.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
    const flushTable = () => {
      if (table) {
        const [head, ...rows] = table.rows;
        const tr = (r, tag) => `<tr>${r.map((c) => `<${tag}>${inline(c)}</${tag}>`).join("")}</tr>`;
        out.push(`<div class="md-table"><table>${table.headed ? `<thead>${tr(head, "th")}</thead>` : ""}<tbody>${(table.headed ? rows : table.rows).map((r) => tr(r, "td")).join("")}</tbody></table></div>`);
      }
      table = null;
    };
    const flushPara = () => {
      if (para.length) out.push(`<p>${inline(para.join(" "))}</p>`);
      para = [];
    };
    const flushList = () => {
      if (list) {
        const start = list.type === "ol" && list.start !== 1 ? ` start="${list.start}"` : "";
        out.push(`<${list.type}${start}>${list.items.map((i) => `<li>${inline(i)}</li>`).join("")}</${list.type}>`);
      }
      list = null;
    };
    const flush = () => {
      flushPara();
      flushList();
      flushTable();
    };
    for (const raw of src.split("\n")) {
      if (fence) {
        if (/^\s*```/.test(raw)) {
          out.push(`<pre><code>${esc(fence.join("\n"))}</code></pre>`);
          fence = null;
        } else fence.push(raw);
        continue;
      }
      const line = raw.trimEnd();
      let m;
      if (/^\s*```/.test(line)) {
        flush();
        fence = [];
      } else if (!line.trim()) {
        flush();
      } else if (/^\s*\|.*\|\s*$/.test(line)) {
        // A table: rows between bars, with a row of dashes under the heading.
        flushPara();
        flushList();
        if (!table) table = { rows: [], headed: false };
        if (/^[\s|:-]+$/.test(line) && line.includes("-")) table.headed = table.rows.length === 1;
        else table.rows.push(cells(line));
      } else if ((m = line.match(/^(#{1,6})\s+(.*)$/))) {
        flush();
        const n = Math.min(6, m[1].length + 2);
        out.push(`<h${n}>${inline(m[2])}</h${n}>`);
      } else if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
        flush();
        out.push("<hr>");
      } else if ((m = line.match(/^\s*(\d+)[.)]\s+(.*)$/))) {
        flushPara();
        if (!list || list.type !== "ol") {
          flushList();
          list = { type: "ol", start: Number(m[1]), items: [] };
        }
        list.items.push(m[2]);
      } else if ((m = line.match(/^\s*[-*+]\s+(.*)$/))) {
        flushPara();
        if (!list || list.type !== "ul") {
          flushList();
          list = { type: "ul", items: [] };
        }
        list.items.push(m[1]);
      } else if ((m = line.match(/^\s*>\s?(.*)$/))) {
        flush();
        out.push(`<blockquote>${inline(m[1])}</blockquote>`);
      } else if (list && /^\s+\S/.test(raw)) {
        list.items[list.items.length - 1] += ` ${line.trim()}`;
      } else {
        flushList();
        para.push(line.trim());
      }
    }
    if (fence) out.push(`<pre><code>${esc(fence.join("\n"))}</code></pre>`);
    flush();
    return out.join("");
  }

  const RULES_TABS = [
    ["rules", "Rules"],
    ["skills", "Skills"],
  ];

  function rulesTabs() {
    const late = state.rules ? (state.rules.skills || []).filter((s) => s.behind).length : 0;
    const dot = (k) => (k === "skills" && late ? `<span class="count num">${late}</span>` : "");
    return `<div class="subtabs" role="tablist" aria-label="House rules">${RULES_TABS.map(
      ([k, label]) => `<button class="subtab" role="tab" data-act="rules-tab" data-tab="${k}" aria-selected="${state.rulesTab === k}">${label}${dot(k)}</button>`
    ).join("")}</div>`;
  }

  function renderRules() {
    const r = state.rules;
    if (!r) return section("rules", "House rules", "", `<p class="empty">Fetching the rules…</p>`);
    if (!state.rulesEditing && state.rulesTab === "skills") return rulesTabs() + renderSkills(r);
    let html;
    if (state.rulesEditing) {
      const draft = state.rulesDraft ?? r.text;
      html = `<div class="lead"><span class="lead-note">Editing the house rules</span></div>
        <textarea class="rules-editor" id="rules-text" spellcheck="true" aria-label="House rules, in Markdown">${esc(draft)}</textarea>
        <div class="button-row">
          <button class="primary" data-act="save-rules">Save rules</button>
          <button class="secondary" data-act="cancel-rules">Cancel</button>
        </div>
        <p class="foot">Markdown: # for a heading, 1. for a numbered rule, **bold**, \`code\`.</p>`;
    } else {
      html = `${rulesTabs()}<div class="lead"><span class="lead-note">${r.customized ? "Your version. " : ""}Agents get these when you teach them below.</span>
          <button class="secondary" data-act="edit-rules">Edit</button></div>
        <article class="md" id="rules-view">${markdown(r.text)}</article>
        ${r.customized ? `<div class="button-row"><button class="link-btn" data-act="reset-rules">Back to mom's rules</button></div>` : ""}`;
    }
    const targets = r.targets
      .map((t) => {
        const note = t.installed ? `In ${t.path}${t.upToDate ? "" : " (older version)"}` : t.available ? `Writes them into ${t.path}` : `${t.label} isn't set up on this computer`;
        return `<div class="switch-row ${t.available ? "" : "unavailable"}" id="tgt-${hash(t.agent)}">
          <span class="label">${esc(t.label)}</span>
          <button class="switch" role="switch" aria-checked="${t.installed}" aria-label="Teach ${esc(t.label)} the rules" data-act="teach" data-agent="${esc(t.agent)}" ${t.available ? "" : "disabled"}></button>
          <small>${esc(note)}</small>
        </div>`;
      })
      .join("");
    html += section("who", "Who knows the rules", "", targets);
    const ints = r.integrations
      .map(
        (i) => `<div class="switch-row ${i.available ? "" : "unavailable"}" id="int-${hash(i.id)}">
          <span class="label">${esc(i.label)}</span>
          <button class="switch" role="switch" aria-checked="${i.installed}" aria-label="${esc(i.label)}" data-act="integration" data-id="${esc(i.id)}" ${i.available ? "" : "disabled"}></button>
          <small>${esc(i.detail)}</small>
        </div>`
      )
      .join("");
    html += section("ints", "Check-ins", "", ints);
    html += `<p class="foot">Check-ins let mom remind an agent about leftovers before it says it's done. Everything here can be switched off again; mom backs up any file she edits.</p>`;
    return html;
  }

  /** The skills that come with momputer: what each does, what to say, and who has it. */
  function renderSkills(r) {
    const skills = r.skills || [];
    const have = skills.filter((s) => (s.has || []).length).length;
    const late = skills.filter((s) => s.behind).length;
    // Behind: what to type to catch up, and what to hand an agent for the ones copied by hand.
    const lines = r.skillsUpdate || [];
    const catchUp = late
      ? `<div class="lead is-late"><div class="lead-text"><p><span class="led red"></span>${late === 1 ? "A skill has" : `${late} skills have`} a newer version than the copy your agent has.</p></div>
          <div class="button-row">
            ${lines.length ? `<button class="secondary" data-act="copy-say" data-text="${esc(lines.join("\n"))}" data-said="Copied. Paste ${lines.length > 1 ? "them" : "it"} into a terminal, then start your agent again.">${ICON.copy}Copy the update ${lines.length > 1 ? "commands" : "command"}</button>` : ""}
            ${r.skillsCatchUp ? `<button class="secondary" data-act="copy-say" data-text="${esc(r.skillsCatchUp)}">${ICON.copy}Copy for your agent</button>` : ""}
          </div></div>`
      : "";
    return `<div class="lead"><span class="lead-note">Jobs you can hand your agent. Say it your own way, or copy a line.</span></div>
      ${catchUp}
      ${section("skills", "Skills", late ? `${late} behind` : have ? `${have} of ${skills.length} installed` : String(skills.length), skills.map(skillRow).join(""))}
      <div class="button-row">
        <button class="secondary" data-act="copy-say" data-text="${esc(r.skillsInstall)}" data-said="Copied. Paste it into Claude Code to get all ${skills.length}.">Copy the install line</button>
        <button class="link-btn" data-act="open-uri" data-url="${esc(r.skillsPage)}">On GitHub${ICON.out}</button>
      </div>
      <p class="foot">One line gets all of them in Claude Code. For Codex and other agents, copy a skill's folder from GitHub.</p>`;
  }

  /** One skill: a light for whether an agent here has it, what it does in a sentence, and what
      to say to use it. The rest (how it works, more to say, where to read it) is behind "More". */
  function skillRow(s) {
    const key = `skill:${s.name}`;
    const open = state.more.has(key);
    const has = s.has || [];
    const say = (p) => `<div class="say-row"><span class="say">&ldquo;${esc(p)}&rdquo;</span>
        <button class="link-btn" data-act="copy-say" data-text="${esc(p)}" title="Copy it, then paste it to your agent">Copy</button></div>`;
    // The copy an agent here has, to read in BIG MODE (it's one of the files in that agent's set-up).
    const files = (state.agents || []).flatMap((a) => a.docs);
    const mine = (s.copies || []).map((c) => files.find((d) => d.path.toLowerCase() === `${c.path}\\SKILL.md`.toLowerCase())).filter(Boolean).map((d) => d.id)[0];
    // A copy that's older than the newest mom knows: who has it, and which version.
    const late = (s.copies || []).filter((c) => c.behind);
    const behind = late.length
      ? `<p class="skill-late"><span class="led red"></span>${esc(
          `${[...new Set(late.map((c) => c.who))].join(" and ")} ${late.length === 1 ? "has" : "have"} ${late[0].version ? `version ${late[0].version}` : "an older copy"}. The newest is ${s.version}.`
        )}</p>`
      : "";
    return `<div class="skill" id="skill-${hash(s.name)}">
      <div class="skill-head"><span class="led ${late.length ? "red" : has.length ? "green" : ""}"></span><strong>${esc(s.title)}</strong>
        <small>${esc([has.length ? `in ${has.join(" and ")}` : "not installed", s.version ? `v${s.version}` : ""].filter(Boolean).join(" · "))}</small></div>
      ${behind}
      <p class="skill-what">${esc(s.what)}${open ? ` ${esc(s.more)}` : ""} <button class="more-btn" data-act="more" data-key="${esc(key)}" aria-expanded="${open}">${open ? "Less" : "More"}</button></p>
      ${(open ? s.prompts : s.prompts.slice(0, 1)).map(say).join("")}
      ${
        open
          ? `<div class="skill-read">${mine ? `<button class="link-btn" data-act="open-doc" data-id="${esc(mine)}" data-title="${esc(s.title)}">Read your copy</button>` : ""}
              <button class="link-btn" data-act="open-uri" data-url="${esc(s.page)}">${mine ? "On GitHub" : "Read the skill"}${ICON.out}</button></div>`
          : ""
      }
    </div>`;
  }

  async function loadRules() {
    state.rules = await invoke("get_rules");
    // The tab says how many skills are behind, so it shows without opening it.
    const late = (state.rules.skills || []).filter((s) => s.behind).length;
    const count = late ? String(late) : "";
    if ($("#count-rules").textContent !== count) {
      $("#count-rules").textContent = count;
      moveThumb(false);
    }
    render();
  }

  // ------------------------------------------------------------ settings

  // ------------------------------------------------------------ about

  // Coin and GitHub marks from Simple Icons (CC0), drawn in the ink colour like every other icon.
  const MARK = {
    Solana:
      "m23.8764 18.0313-3.962 4.1393a.9201.9201 0 0 1-.306.2106.9407.9407 0 0 1-.367.0742H.4599a.4689.4689 0 0 1-.2522-.0733.4513.4513 0 0 1-.1696-.1962.4375.4375 0 0 1-.0314-.2545.4438.4438 0 0 1 .117-.2298l3.9649-4.1393a.92.92 0 0 1 .3052-.2102.9407.9407 0 0 1 .3658-.0746H23.54a.4692.4692 0 0 1 .2523.0734.4531.4531 0 0 1 .1697.196.438.438 0 0 1 .0313.2547.4442.4442 0 0 1-.1169.2297zm-3.962-8.3355a.9202.9202 0 0 0-.306-.2106.941.941 0 0 0-.367-.0742H.4599a.4687.4687 0 0 0-.2522.0734.4513.4513 0 0 0-.1696.1961.4376.4376 0 0 0-.0314.2546.444.444 0 0 0 .117.2297l3.9649 4.1394a.9204.9204 0 0 0 .3052.2102c.1154.049.24.0744.3658.0746H23.54a.469.469 0 0 0 .2523-.0734.453.453 0 0 0 .1697-.1961.4382.4382 0 0 0 .0313-.2546.4444.4444 0 0 0-.1169-.2297zM.46 6.7225h18.7815a.9411.9411 0 0 0 .367-.0742.9202.9202 0 0 0 .306-.2106l3.962-4.1394a.4442.4442 0 0 0 .117-.2297.4378.4378 0 0 0-.0314-.2546.453.453 0 0 0-.1697-.196.469.469 0 0 0-.2523-.0734H4.7596a.941.941 0 0 0-.3658.0745.9203.9203 0 0 0-.3052.2102L.1246 5.9687a.4438.4438 0 0 0-.1169.2295.4375.4375 0 0 0 .0312.2544.4512.4512 0 0 0 .1692.196.4689.4689 0 0 0 .2518.0739z",
    Zcash:
      "M12 0A12 12 0 0 0 0 12a12.013 12.013 0 0 0 12 12 12 12 0 1 0 0-24zm-1.008 4.418h2.014v2.014l3.275-.002v1.826l-5.08 6.889h5.08v2.423h-3.275v2.006h-2.012v-2.006H7.72v-1.826l5.074-6.888H7.719V6.432h3.273V4.418z",
    Ethereum: "M11.944 17.97L4.58 13.62 11.943 24l7.37-10.38-7.372 4.35h.003zM12.056 0L4.69 12.223l7.365 4.354 7.365-4.35L12.056 0z",
    Bitcoin:
      "M23.638 14.904c-1.602 6.43-8.113 10.34-14.542 8.736C2.67 22.05-1.244 15.525.362 9.105 1.962 2.67 8.475-1.243 14.9.358c6.43 1.605 10.342 8.115 8.738 14.548v-.002zm-6.35-4.613c.24-1.59-.974-2.45-2.64-3.03l.54-2.153-1.315-.33-.525 2.107c-.345-.087-.705-.167-1.064-.25l.526-2.127-1.32-.33-.54 2.165c-.285-.067-.565-.132-.84-.2l-1.815-.45-.35 1.407s.975.225.955.236c.535.136.63.486.615.766l-1.477 5.92c-.075.166-.24.406-.614.314.015.02-.96-.24-.96-.24l-.66 1.51 1.71.426.93.242-.54 2.19 1.32.327.54-2.17c.36.1.705.19 1.05.273l-.51 2.154 1.32.33.545-2.19c2.24.427 3.93.257 4.64-1.774.57-1.637-.03-2.58-1.217-3.196.854-.193 1.5-.76 1.68-1.93h.01zm-3.01 4.22c-.404 1.64-3.157.75-4.05.53l.72-2.9c.896.23 3.757.67 3.33 2.37zm.41-4.24c-.37 1.49-2.662.735-3.405.55l.654-2.64c.744.18 3.137.524 2.75 2.084v.006z",
    GitHub:
      "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
  };
  const mark = (name, cls) => `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true"><path d="${MARK[name]}"/></svg>`;

  // Ilisium's four-circle prism, drawn in the ink colour so it suits light and dark.
  const ILISIUM = `<svg viewBox="0 0 128 128" aria-hidden="true"><g transform="translate(64 64)"><g fill="none" stroke="currentColor" stroke-width="6">
    <circle cy="-30" r="30"/><circle cx="30" r="30"/><circle cy="30" r="30"/><circle cx="-30" r="30"/></g><circle r="8" fill="currentColor"/></g></svg>`;

  const TIPS = [
    ["Solana", "5qT8edyi62wwLz9UBTiRVoN3S9UXLjbV2YVEu4GJH3zH"],
    ["Zcash", "t1cYbXWEXXc6kZYR1JjkE2qRCEmzCpMA371"],
    ["Ethereum", "0xfFa448F43896Cf8227aAceF3A72b80450e1fF86E"],
    ["Bitcoin", "bc1qmnw03rsastc08g6zutgx2sdarzapz82q5nn2kd"],
  ];
  const LINKS = [
    ["Marsell", "getmarsell.com", "https://getmarsell.com", `<img src="img/marsell.png" alt="" width="34" height="34">`],
    ["Ilisium", "ilisium.xyz", "https://ilisium.xyz", ILISIUM],
  ];
  const REPO = "https://github.com/Thisisily/momputer";

  function renderAbout() {
    const face = window.momFace ? window.momFace.svg("delighted") : "";
    const tile = (name, shown, url, logo, wide = false) => `<button class="about-tile${wide ? " wide" : ""}" data-act="open-uri" data-url="${esc(url)}" id="ab-${hash(url)}">
        <span class="about-logo">${logo}</span><span class="about-tile-name">${esc(name)}</span><span class="about-tile-sub">${esc(shown)}</span><span class="about-tile-go">${ICON.out}</span></button>`;
    const tiles = LINKS.map(([name, shown, url, logo]) => tile(name, shown, url, logo)).join("");
    const [coin, addr] = TIPS.find(([c]) => c === state.tipCoin) || TIPS[0];
    const coins = TIPS.map(
      ([c]) => `<button class="seg-btn coin" data-act="tip-coin" data-coin="${esc(c)}" aria-pressed="${c === coin}">${mark(c, "coin-ico")}${esc(c)}</button>`
    ).join("");
    return `<div class="sheet-head"><h2 id="sheet-title" class="dot sheet-dot">about</h2>
        <button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button></div>
      <div class="about-hero">
        <div class="about-face" aria-hidden="true">${face}</div>
        <div>
          <div class="about-word dot">momputer</div>
          <div class="about-tag">A mom for you, your computer and your AIs.</div>
          <div class="about-meta">${state.version ? `<span class="about-ver num">v${esc(state.version)}</span>` : ""}<span>Free for personal use</span></div>
          ${updateLine()}
        </div>
      </div>
      <h3 class="list-head">A message from the dev</h3>
      <div class="about-note">
        <p><b>Hi! I'm Ilya</b> and I built momputer because my desktop was a mess. Every AI agent I worked with made it worse: dev servers left running, screenshots everywhere, folders called thing-v2-final. So I made my computer a mom.</p>
      </div>
      <h3 class="list-head">More from me</h3>
      <div class="about-tiles">${tiles}${tile("momputer on GitHub", "Thisisily/momputer · issues and ideas welcome", REPO, mark("GitHub", "fill"), true)}</div>
      <h3 class="list-head">Buy mom a coffee</h3>
      <div class="about-tip">
        <div class="seg about-coins" role="group" aria-label="Which coin">${coins}</div>
        <div class="about-addr num" id="tip-addr" title="${esc(coin)} address">${mark(coin, "coin-ico")}<span>${esc(addr)}</span></div>
        <div class="about-tip-foot"><span>momputer is free for personal use and I'm keeping it that way. If she helps, a tip is welcome.</span>
          <button class="primary" data-act="copy-text" data-text="${esc(addr)}" data-what="${esc(coin)}">Copy</button></div>
      </div>
      <p class="about-foot">Made by Ilya Arbabi · Free for personal use (PolyForm Noncommercial).<br>Mom only goes online to ask if there's a newer version. Nothing about you or your PC is sent.<br>
        <span class="about-hint">There's one room in this house nobody tidies.</span></p>`;
  }

  function renderSettings() {
    const st = state.settings;
    if (!st) return "";
    const c = st.config;
    const sw = (key, on, label, note, off = false) => `<div class="switch-row${off ? " unavailable" : ""}" id="set-${key}">
        <span class="label">${label}</span>
        <button class="switch" role="switch" aria-checked="${on}" aria-label="${esc(label.replace(/<[^>]+>/g, ""))}" data-set="${key}" ${off ? "disabled" : ""}></button>
        <small>${note}</small>
      </div>`;
    const num = (key, value, min, max) => `<input class="inline-num num" type="number" min="${min}" max="${max}" value="${value}" data-num="${key}">`;
    const row = (key, label, note) => `<div class="switch-row" id="set-${key}"><span class="label">${label}</span><small>${note}</small></div>`;
    // What you said to keep, each with a way to let it go again.
    const kept = c.trash.keep || [];
    const keptRow = (p) => `<div class="item stays moved" id="kept-${hash(p)}"><span class="led green"></span>
        <span class="item-main"><span class="item-name" title="${esc(p)}">${esc(p.split(/[\\/]/).filter(Boolean).pop() || p)}</span><span class="item-sub" title="${esc(p)}">${esc(p)}</span></span>
        <span class="moved-acts"><button class="mini-btn wide" data-act="unkeep" data-path="${esc(p)}">Let go</button></span></div>`;
    const keptBox = kept.length
      ? section(
          "s-kept",
          "Always kept",
          String(kept.length),
          kept.map(keptRow).join("") +
            `<p class="group-note">No clean-up takes these. The lock on a row adds one.</p>` +
            (kept.length > 1 ? `<div class="group-foot"><button class="link-btn" data-act="unkeep-all">Let go of all ${kept.length}</button></div>` : "")
        )
      : "";
    return `<div class="sheet-head"><h2 id="sheet-title" class="dot sheet-dot">settings</h2>
        <button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button></div>
      ${section(
        "s-clean",
        "Cleaning up",
        "",
        sw("autoStop", c.leftovers.autoStop, "Clean up on my own", `Turn off what agents leave behind once nobody has come back for it in ${num("autoStopAfterMinutes", c.leftovers.autoStopAfterMinutes, 1, 1440)} minutes.`) +
          sw("autoStopOrphans", c.leftovers.autoStopOrphans, "Orphans too", "Also turn off runtimes nobody can be traced to.") +
          sw("includeOrphans", c.leftovers.includeUnknownOrphans, "Count orphans as leftovers", "Runtimes whose parent is gone and carry no agent's fingerprints.") +
          row(
            "keep",
            "How long things stay",
            `What agents leave behind (scratch, logs, snapshots, worktrees) stays ${num("agentScratchDays", c.trash.agentScratchDays, 1, 365)} days before mom suggests clearing it, other temp files ${num("tempMinAgeDays", c.trash.tempMinAgeDays, 1, 90)} days. Conversations nobody has opened in ${num("conversationsAfterDays", c.trash.conversationsAfterDays, 1, 3650)} days can move to another drive.`
          ) +
          (kept.length ? "" : row("kept", "Always kept", "Nothing yet. The lock on any row in a clean-up keeps it out of every clean-up."))
      )}
      ${keptBox}
      ${section(
        "s-tell",
        "Notifications",
        "",
        sw("notifications", c.ui.notifications, "Notifications", "Mom's notes in the corner of your screen. Switch this off and she keeps quiet.") +
          sw(
            "notifyLeftovers",
            c.ui.notifyLeftovers,
            "Leftovers",
            `When agents leave things running and nobody comes back for them in ${num("notifyAfterMinutes", c.ui.notifyAfterMinutes, 1, 1440)} minutes. At most one note every ${num("notifyEveryMinutes", c.ui.notifyEveryMinutes, 5, 1440)} minutes.`,
            !c.ui.notifications
          ) +
          sw("notifyMeltdown", c.ui.notifyMeltdown, "CPU alarm", `When leftovers burn more than ${num("cpuAlertPercent", c.ui.cpuAlertPercent, 5, 100)}% CPU for two minutes. At most once an hour.`, !c.ui.notifications) +
          sw("notifyCleanups", c.ui.notifyCleanups, "Clean-up reports", "When mom turns things off by herself.", !c.ui.notifications) +
          sw("notifySkills", c.ui.notifySkills !== false, "Skills", "When a skill has a newer version than the copy your agent has.", !c.ui.notifications)
      )}
      ${section(
        "s-widget",
        "The widget",
        "",
        sw("alwaysOnTop", c.ui.alwaysOnTop, "Keep on top", "Stay above other windows.") +
          sw("autostart", st.autostart, "Start with Windows", "Mom starts quietly in the tray when you log in.") +
          sw("checkUpdates", c.ui.checkUpdates, "New versions", "Ask GitHub once a day whether there's a newer momputer. Nothing about your PC is sent.") +
          sw("paused", state.paused, "Take a break", `Stop watching until you switch this off. Otherwise mom looks every ${num("scanIntervalSecs", c.ui.scanIntervalSecs, 1, 60)} seconds.`)
      )}
      <p class="foot">momputer ${esc(st.version)}. Settings live in ${esc(st.configPath)}.</p>`;
  }

  /** Changes the settings and saves them. What's shown changes only once the save has gone
      through: if it didn't (true if it did), everything stays as it was and the reason is said. */
  async function saveSettings(mutate) {
    const st = state.settings;
    const next = JSON.parse(JSON.stringify(st.config));
    mutate(next);
    const saved = (await act(invoke("set_settings", { config: next }), () => null)) !== null;
    if (saved) st.config = next;
    renderSheet();
    $("#btn-pin").setAttribute("aria-pressed", String(st.config.ui.alwaysOnTop));
    renderNote();
    return saved;
  }

  // ------------------------------------------------------------ the sheet (details and settings)

  function renderSheet() {
    if (!state.sheet) return;
    const kind = state.sheet.kind;
    const html = kind === "spot" ? renderSpot(state.sheet.id) : kind === "agent" ? renderAgentSheet(state.sheet.id) : kind === "about" ? renderAbout() : renderSettings();
    morph($("#sheet-body"), html);
  }

  async function openSheet(sheet) {
    if (sheet.kind === "settings") state.settings = await invoke("get_settings");
    if (sheet.kind === "about" && !state.version) state.version = await invoke("app_version").catch(() => "");
    const opened = sheet.kind === "spot" ? state.trash.spots.find((x) => x.id === sheet.id) || {} : {};
    if (opened.action === "move") loadArchive();
    if (opened.admin) loadAdminCheck();
    const wasOpen = !!state.sheet;
    state.sheet = sheet;
    state.cleanConfirm = null;
    renderSheet();
    $("#btn-settings").setAttribute("aria-pressed", String(sheet.kind === "settings"));
    $("#btn-about").setAttribute("aria-pressed", String(sheet.kind === "about"));
    const wrap = $("#sheet");
    wrap.hidden = false;
    $("#sheet-body").scrollTop = 0;
    if (!wasOpen && moving()) {
      G.fromTo("#sheet .backdrop", { opacity: 0 }, { opacity: 1, duration: 0.3, ease: "power1.out" });
      G.fromTo("#sheet .sheet-panel", { yPercent: 100 }, { yPercent: 0, duration: 0.5, ease: "expo.out" });
    }
    const focus = wrap.querySelector("[data-act=close-sheet]");
    if (focus) focus.focus({ preventScroll: true });
  }

  function closeSheet() {
    if (!state.sheet) return;
    state.sheet = null;
    state.cleanConfirm = null;
    $("#btn-settings").setAttribute("aria-pressed", "false");
    $("#btn-about").setAttribute("aria-pressed", "false");
    const wrap = $("#sheet");
    const done = () => {
      if (!state.sheet) wrap.hidden = true;
    };
    if (moving()) {
      G.to("#sheet .backdrop", { opacity: 0, duration: 0.25 });
      G.to("#sheet .sheet-panel", { yPercent: 100, duration: 0.3, ease: "power2.in", onComplete: done });
    } else done();
  }

  // ------------------------------------------------------------ render & events

  function render() {
    const page = $("#page");
    const typing = state.tab === "rules" && document.activeElement && document.activeElement.id === "rules-text";
    try {
      if (!typing) {
        let html = "";
        if (state.tab === "running") html = renderRunning();
        else if (state.tab === "upkeep") html = renderUpkeep();
        else if (state.tab === "rooms") html = renderRooms();
        else html = renderRules();
        morph(page, html);
        stripEdges();
        if (state.tab === "upkeep") drawFree();
      }
      state.renderFailed = false;
      renderNote();
      if (state.sheet && state.sheet.kind === "spot") renderSheet();
    } catch (e) {
      console.error("mom couldn't draw", e);
      if (!state.renderFailed) toast(`Mom tripped while drawing: ${e.message}`, true);
      state.renderFailed = true;
      try {
        showFace();
      } catch {}
    }
  }

  function applySnap(snap) {
    if (!snap) return;
    state.snap = snap;
    state.lastSnapAt = Date.now();
    if (state.tab === "running" && !document.hidden) render();
    else renderNote();
    renderChecked();
    // The light flickers once per look: off for a moment, then on. Two frames, not an animation
    // (a CSS fade would be drawn at the screen's rate, 240 frames a second on some screens).
    const live = $("#live");
    live.classList.add("blip");
    setTimeout(() => live.classList.remove("blip"), 160);
  }

  function renderChecked() {
    const el = $("#checked");
    const live = $("#live");
    if (!state.snap) {
      el.textContent = "";
      return;
    }
    const ago = lookedAgo();
    const late = stale();
    el.textContent = state.paused ? "on a break" : ago < 5 ? "looked just now" : `looked ${age(ago)} ago`;
    el.classList.toggle("stale", late);
    el.title = late ? "Mom hasn't been able to look around for a while. Press Look again." : "When mom last looked around";
    live.classList.toggle("green", !state.paused && !late);
    live.classList.toggle("red", late);
    live.title = el.title;
    showFace();
  }

  function moveThumb(animate = true) {
    const tab = document.querySelector(`.tab[data-tab="${state.tab}"]`);
    const thumb = $("#tab-thumb");
    if (!tab || !thumb) return;
    const to = { x: tab.offsetLeft, width: tab.offsetWidth };
    if (animate && moving()) G.to(thumb, { ...to, duration: 0.45, ease: "expo.out" });
    else if (G) G.set(thumb, to);
    else Object.assign(thumb.style, { transform: `translateX(${to.x}px)`, width: `${to.width}px` });
  }

  async function loadHealth(force = false) {
    if (!force && Date.now() - state.healthAt < 20000) return;
    state.healthAt = Date.now();
    try {
      state.health = await invoke("get_health");
      if (state.tab === "upkeep") render();
    } catch (e) {
      console.warn("no health", e);
    }
  }

  function selectTab(tab) {
    if (tab === state.tab) return;
    state.tab = tab;
    document.querySelectorAll(".tab").forEach((b) => b.setAttribute("aria-selected", String(b.dataset.tab === tab)));
    moveThumb();
    const page = $("#page");
    page.scrollTop = 0;
    page.innerHTML = "";
    render();
    if (moving()) G.fromTo(page, { opacity: 0, y: 8 }, { opacity: 1, y: 0, duration: 0.3, ease: "power2.out", clearProps: "opacity,transform" });
    if (tab === "upkeep") {
      // Show the last measurement straight away; look again only if there isn't one, or it's from
      // more than half a day ago (the old numbers stay up while she looks).
      if (!state.trash.busy && (!state.trash.spots.length || state.trash.complete === false || measuredAgo() > 12 * 3600)) invoke("measure_trash");
      loadHealth();
      openUpkeepTab(state.upkeepTab);
    }
    if (tab === "rooms") loadRooms();
    if (tab === "rules") loadRules();
  }

  /** What a tab of Upkeep needs the first time it's shown. */
  function openUpkeepTab(tab) {
    if (tab === "windows") {
      loadWinsettings();
      if (!state.system) loadSystem();
    }
    if (tab === "files" && !state.files) loadFiles();
    if (tab === "files") loadPlaces();
  }

  function groupById(id) {
    return state.snap && state.snap.groups.find((g) => g.id === id);
  }

  async function stopIds(ids) {
    ids.forEach((id) => state.stopping.add(id));
    render();
    const r = await act(invoke("stop_groups", { ids }));
    ids.forEach((id) => {
      state.stopping.delete(id);
      if (r) state.done.add(id);
    });
    loadStopped();
    if (r) react("kiss");
    render();
    setTimeout(() => {
      ids.forEach((id) => state.done.delete(id));
      render();
    }, 1500);
  }

  async function cleanAll() {
    const s = state.snap;
    if (!s) return;
    const ids = s.groups.filter(isLeft).map((g) => g.id);
    const btn = $("#btn-clean");
    btn.disabled = true;
    $("#clean-label").textContent = "Cleaning up…";
    if (state.tab !== "running") selectTab("running");
    ids.forEach((id) => state.stopping.add(id));
    render();
    const r = await act(invoke("stop_leftovers"));
    ids.forEach((id) => {
      state.stopping.delete(id);
      if (r) state.done.add(id);
    });
    loadStopped();
    btn.disabled = false;
    if (r) react("kiss");
    render();
    setTimeout(() => {
      ids.forEach((id) => state.done.delete(id));
      render();
    }, 1500);
  }

  let cleanTimer = null;

  let winTimer = null;
  let filesTimer = null;

  async function cleanSpot(id) {
    const s = state.trash.spots.find((x) => x.id === id);
    if (!s) return;
    if (state.cleaning) return;
    clearTimeout(cleanTimer);
    if (state.cleanConfirm !== id) {
      // First press: the button asks "Sure?" for a few seconds; a second press does it.
      state.cleanConfirm = id;
      renderSheet();
      if (moving()) G.fromTo("#clean-btn", { scale: 0.94 }, { scale: 1, duration: 0.5, ease: "back.out(3)", clearProps: "transform" });
      cleanTimer = setTimeout(() => {
        state.cleanConfirm = null;
        renderSheet();
      }, 6000);
      return;
    }
    const p = picked(s);
    state.cleanConfirm = null;
    state.cleaning = id;
    state.live = null;
    state.gone = new Set();
    renderSheet();
    watchClean(id);
    const pk = p.pk;
    const toBin = recycling(s);
    // What this list shows as kept is named too: what you see staying stays, even if mom's own
    // note of it and this list have drifted apart.
    const keep = (s.items || []).filter((i) => i.kept).map((i) => i.path);
    const pick = p.partial || toBin || keep.length ? { goes: pk.goes, new: pk.new, ticked: [...pk.ticked], unticked: [...pk.unticked], recycle: toBin, keep } : null;
    const r = await act(invoke("clean_trash", { id, pick }));
    state.cleaning = null;
    state.live = null;
    delete state.picks[id];
    if (s.action === "move") loadArchive();
    if (r && r.deleted > 0) {
      react("cool");
      // All done: close the sheet so mom (and her note about it) can be seen.
      if (!r.skipped && state.sheet && state.sheet.id === id) closeSheet();
    }
    renderSheet();
  }

  document.addEventListener("click", async (e) => {
    const el = e.target.closest("[data-act]");
    if (!el) return;
    const a = el.dataset.act;
    const rowEl = el.closest("[data-id]");
    const g = rowEl ? groupById(rowEl.dataset.id) : null;
    switch (a) {
      case "expand":
        if (!g) return;
        state.expanded.has(g.id) ? state.expanded.delete(g.id) : state.expanded.add(g.id);
        render();
        break;
      case "stop":
        if (!g) return;
        if (isLeft(g) || state.confirm === g.id) {
          state.confirm = null;
          stopIds([g.id]);
        } else {
          state.confirm = g.id;
          clearTimeout(state.confirmTimer);
          state.confirmTimer = setTimeout(() => {
            state.confirm = null;
            render();
          }, 3000);
          render();
        }
        break;
      case "open":
        act(invoke("open_url", { url: `http://localhost:${el.dataset.port}` }), () => null);
        break;
      case "bin-mode":
        if (state.cleaning) break;
        state.recycle = el.dataset.recycle === "1";
        state.cleanConfirm = null;
        saveRecycle();
        renderSheet();
        break;
      case "tip-coin":
        state.tipCoin = el.dataset.coin;
        renderSheet();
        break;
      case "rules-tab":
        state.rulesTab = el.dataset.tab;
        render();
        break;
      case "copy-say": {
        try {
          await navigator.clipboard.writeText(el.dataset.text);
          toast(el.dataset.said || "Copied. Paste it to your agent.");
        } catch {
          toast("Couldn't reach the clipboard.", true);
        }
        break;
      }
      case "copy-text": {
        try {
          await navigator.clipboard.writeText(el.dataset.text);
          toast(`Copied the ${el.dataset.what} address. Thank you, sweetie c:`);
          react("kiss");
        } catch {
          toast("Couldn't reach the clipboard.", true);
        }
        break;
      }
      case "open-uri":
        act(invoke("open_url", { url: el.dataset.url }), () => null);
        break;
      case "reveal":
        if (g && g.cwd) act(invoke("reveal", { path: g.cwd }), () => null);
        break;
      case "reveal-path":
        e.preventDefault();
        act(invoke("reveal", { path: el.dataset.path }), () => null);
        break;
      case "keep":
        if (!g) return;
        act(invoke("keep_group", { signature: g.signature, keep: !g.kept }), () => (g.kept ? "Okay, it can go when you say so." : "Got it. I'll leave that one alone."));
        if (!g.kept) react("wink");
        break;
      case "open-thread":
        act(invoke("open_url", { url: el.dataset.link }), () => null);
        break;
      case "toggle-helpers":
        state.showHelpers = !state.showHelpers;
        render();
        break;
      case "toggle-unused": {
        const which = el.dataset.which || "unused";
        if (!state.showAllUnused.delete(which)) state.showAllUnused.add(which);
        render();
        break;
      }
      case "uninstall-app": {
        const key = el.dataset.key;
        clearTimeout(cleanTimer);
        if (state.uninstallConfirm !== key) {
          state.uninstallConfirm = key;
          render();
          cleanTimer = setTimeout(() => {
            state.uninstallConfirm = null;
            render();
          }, 6000);
          break;
        }
        state.uninstallConfirm = null;
        if (await act(invoke("uninstall_app", { key }))) state.uninstalled.add(key);
        render();
        setTimeout(() => loadHealth(true), 60000);
        break;
      }
      case "toggle-startup":
        state.showAllStartup = !state.showAllStartup;
        render();
        break;
      case "container-stop": {
        const id = el.closest("[data-container]").dataset.container;
        el.disabled = true;
        await act(invoke("stop_container", { id }), () => "Container stopped.");
        break;
      }
      case "wsl-stop":
        el.disabled = true;
        await act(invoke("stop_wsl", { name: null }), () => "WSL is asleep. Memory's back.");
        break;
      case "measure":
        invoke("measure_trash");
        state.trash.busy = true;
        loadHealth(true);
        render();
        break;
      case "open-spot":
        openSheet({ kind: "spot", id: el.dataset.spot });
        break;
      case "open-agent":
        openSheet({ kind: "agent", id: el.dataset.id });
        break;
      case "agents-refresh":
        loadAgents();
        renderSheet();
        break;
      case "agents-check": {
        if (state.agentsChecking) break;
        state.agentsChecking = true;
        renderSheet();
        const found = await act(invoke("check_agents"), () => "That's how its servers are right now.");
        state.agentsChecking = false;
        if (Array.isArray(found)) {
          state.agents = found;
          state.agentsAt = Date.now();
        } else await loadAgents();
        if (state.tab === "running" && state.runningTab === "agents") render();
        renderSheet();
        break;
      }
      case "toggle-agent-list":
        if (!state.showAllAgent.delete(el.dataset.key)) state.showAllAgent.add(el.dataset.key);
        renderSheet();
        break;
      case "open-doc":
        openDoc(el.dataset.id, el.dataset.title);
        break;
      case "close-reader":
        closeReader();
        break;
      case "running-tab":
        if (state.runningTab === el.dataset.tab) break;
        openRunningTab(el.dataset.tab);
        $("#page").scrollTop = 0;
        render();
        break;
      case "profile-open":
        state.browserOpen = state.browserOpen === el.dataset.key ? null : el.dataset.key;
        render();
        break;
      case "ext-off":
        if (state.extOff.has(el.dataset.key)) state.extOff.delete(el.dataset.key);
        else state.extOff.add(el.dataset.key);
        render();
        break;
      case "browsers-again":
        loadBrowsers(true);
        break;
      case "prog-expand":
        state.progOpen = state.progOpen === el.dataset.id ? null : el.dataset.id;
        render();
        break;
      case "prog-sort":
        state.progView.sort = el.dataset.sort;
        state.progOrder = null;
        render();
        break;
      case "reg-fix": {
        const id = el.dataset.id;
        if (state.regBusy) break;
        if (state.regConfirm !== id) {
          state.regConfirm = id;
          render();
          setTimeout(() => {
            if (state.regConfirm === id) {
              state.regConfirm = null;
              render();
            }
          }, 6000);
          break;
        }
        state.regConfirm = null;
        state.regBusy = id;
        render();
        const c = state.system && state.system.checks.find((x) => x.id === id);
        try {
          toast(await invoke("fix_registry", { id }));
          state.regFixed.set(id, c ? c.title : id);
          react("proud");
        } catch (e) {
          toast(String(e), true);
        }
        state.regBusy = null;
        await loadSystem(true);
        break;
      }
      case "reg-undo": {
        const id = el.dataset.id;
        try {
          toast(await invoke("undo_registry", { id }));
          state.regFixed.delete(id);
        } catch (e) {
          toast(String(e), true);
        }
        await loadSystem(true);
        break;
      }
      case "files-place": {
        const x = state.filesPlaces && state.filesPlaces.find((y) => y.path === el.dataset.path);
        if (!x) break;
        try {
          state.filesPlaces = await invoke("set_files_place", { path: x.path, on: !x.on });
        } catch (e) {
          toast(String(e), true);
        }
        render();
        break;
      }
      case "files-add-place":
        try {
          const pl = await invoke("add_files_place");
          if (pl) state.filesPlaces = pl;
        } catch (e) {
          toast(String(e), true);
        }
        render();
        break;
      case "prog-rest":
        state.progView.rest = !state.progView.rest;
        render();
        break;
      case "prog-act": {
        if (state.progBusy) break;
        const key = `${el.dataset.id}|${el.dataset.action}`;
        if (el.dataset.sure && state.progConfirm !== key) {
          state.progConfirm = key;
          render();
          setTimeout(() => {
            if (state.progConfirm === key) {
              state.progConfirm = null;
              render();
            }
          }, 6000);
          break;
        }
        programAction(el.dataset.id, el.dataset.action);
        break;
      }
      case "startup-set": {
        const x = state.startup && state.startup.find((y) => y.id === el.dataset.id);
        if (!x || state.startupBusy) break;
        state.startupBusy = x.id;
        x.enabled = !x.enabled;
        render();
        try {
          toast(await invoke("set_startup", { id: x.id, on: x.enabled }));
        } catch (e) {
          x.enabled = !x.enabled;
          toast(String(e), true);
        }
        state.startupBusy = null;
        loadStartup();
        break;
      }
      case "speed-again":
        loadSpeed(true);
        break;
      case "copy-fix":
        try {
          await navigator.clipboard.writeText(el.dataset.text);
          toast(`${el.dataset.text.length > 40 ? "Copied." : `Copied ${el.dataset.text}.`} ${el.dataset.hint}`);
        } catch {
          toast("Couldn't reach the clipboard.", true);
        }
        break;
      case "go-tab": {
        const [tab, sub] = el.dataset.to.split(":");
        if (tab === "upkeep" && sub) {
          state.upkeepTab = sub;
          openUpkeepTab(sub);
        }
        if (tab === "running" && sub) openRunningTab(sub);
        selectTab(tab);
        $("#page").scrollTop = 0;
        render();
        break;
      }
      case "upkeep-tab":
        if (state.upkeepTab === el.dataset.tab) break;
        state.upkeepTab = el.dataset.tab;
        $("#page").scrollTop = 0;
        render();
        openUpkeepTab(state.upkeepTab);
        break;
      case "refresh-spot":
        invoke("measure_spot", { id: el.dataset.spot });
        state.trash.busy = true;
        renderSheet();
        break;
      case "show-more":
        invoke("measure_spot", { id: el.dataset.spot, more: true });
        state.trash.busy = true;
        renderSheet();
        break;
      case "keep-item": {
        e.preventDefault(); // it sits inside the row's label, which would tick the box
        const s = state.trash.spots.find((x) => x.id === el.dataset.spot);
        const i = s && (s.items || []).find((x) => x.path === el.dataset.path);
        if (!i || state.cleaning) return;
        const keep = !i.kept;
        // Shown straight away; mom measures the spot again and sends the real numbers.
        if (keep) {
          if (i.goes) {
            s.reclaimableItems = Math.max(0, (s.reclaimableItems || 0) - 1);
            s.reclaimableBytes = Math.max(0, (s.reclaimableBytes || 0) - i.bytes);
            s.keptItems = (s.keptItems || 0) + 1;
            s.keptBytes = (s.keptBytes || 0) + i.bytes;
          } else if (i.optional) {
            s.newItems = Math.max(0, (s.newItems || 0) - 1);
            s.newBytes = Math.max(0, (s.newBytes || 0) - i.bytes);
          }
          Object.assign(i, { goes: false, optional: false, kept: true, reason: "You said to keep this" });
        } else Object.assign(i, { kept: false, reason: "Looking again…" });
        const pk = pickOf(s.id);
        pk.ticked.delete(i.path);
        pk.unticked.delete(i.path);
        state.cleanConfirm = null;
        state.trash.busy = true;
        if (state.settings) {
          const list = (state.settings.config.trash.keep || []).filter((k) => k !== i.path);
          state.settings.config.trash.keep = keep ? [...list, i.path] : list;
        }
        act(invoke("keep_item", { path: i.path, keep, spot: s.id }), () => (keep ? "Got it. I'll leave that one alone." : "Okay, it can go when you say so.")).then(keepSettled);
        if (state.tab === "upkeep") render();
        renderSheet();
        break;
      }
      case "unkeep": {
        const path = el.dataset.path;
        if (state.settings) state.settings.config.trash.keep = (state.settings.config.trash.keep || []).filter((k) => k !== path);
        act(invoke("keep_item", { path, keep: false }), () => "Okay, it can go when you say so.").then(keepSettled);
        renderSheet();
        break;
      }
      case "unkeep-all":
        if (state.settings) state.settings.config.trash.keep = [];
        act(invoke("keep_nothing"), () => "Okay, they can go when you say so.").then(keepSettled);
        renderSheet();
        break;
      case "stop-clean":
        invoke("stop_clean");
        if (state.live) state.live.stopping = true;
        renderSheet();
        break;
      case "toggle-does":
        state.doesOpen = !state.doesOpen;
        renderSheet();
        break;
      case "list-sort":
        state.listView.sort = el.dataset.sort;
        saveView();
        renderSheet();
        break;
      case "list-group":
        state.listView.group = !state.listView.group;
        saveView();
        renderSheet();
        break;
      case "toggle-group": {
        const gid = el.dataset.group;
        state.groupOpen[gid] = el.getAttribute("aria-expanded") !== "true";
        renderSheet();
        break;
      }
      case "pick-group": {
        e.preventDefault();
        const id = state.sheet && state.sheet.id;
        const s = state.trash.spots.find((x) => x.id === id);
        if (!s) return;
        const gid = el.dataset.group;
        const list = gid.slice(id.length + 1).split(":")[0];
        const key = gid.slice(id.length + list.length + 2);
        const q = listQuery();
        const items = s.items.filter((i) => (list === "goes" ? i.goes : !i.goes) && tickable(i) && groupOf(i)[0] === key && matches(i, q));
        const pk = pickOf(id);
        const on = !items.every((i) => isTicked(pk, i));
        for (const i of items) {
          const changes = startsTicked(pk, i) ? pk.unticked : pk.ticked;
          on === startsTicked(pk, i) ? changes.delete(i.path) : changes.add(i.path);
        }
        state.cleanConfirm = null;
        renderSheet();
        break;
      }
      case "start-again": {
        const id = el.dataset.id;
        state.startBusy = id;
        render();
        invoke("start_again", { id })
          .then((msg) => {
            toast(msg);
            react("proud");
          })
          .catch((e) => toast(String(e), true))
          .finally(() => {
            state.startBusy = null;
            loadStopped();
          });
        break;
      }
      case "forget-stopped":
        Promise.all(state.stopped.map((x) => invoke("forget_stopped", { id: x.id }).catch(() => null))).then(loadStopped);
        break;
      case "big-part":
        pickPart(el.dataset.id);
        break;
      case "update-check":
        loadUpdate(true);
        break;
      case "more":
        if (!state.more.delete(el.dataset.key)) state.more.add(el.dataset.key);
        render();
        renderStage();
        break;
      case "open-program":
        act(invoke("open_program", { name: el.dataset.url }), () => null);
        break;
      case "win-refresh":
        loadWinsettings(true);
        loadSystem(true);
        break;
      case "win-fix": {
        const id = el.dataset.id;
        const f = (state.winsettings || []).find((x) => x.id === id);
        if (!f || state.winBusy) return;
        clearTimeout(winTimer);
        if (state.winConfirm !== id) {
          state.winConfirm = id;
          render();
          winTimer = setTimeout(() => {
            state.winConfirm = null;
            render();
          }, 6000);
          return;
        }
        state.winConfirm = null;
        state.winBusy = id;
        render();
        const msg = await act(invoke("fix_setting", { id }));
        state.winBusy = null;
        if (msg && f.fix && f.fix.undo && !f.fix.admin) state.winFixed = [{ id, title: f.title }, ...state.winFixed.filter((x) => x.id !== id)];
        if (msg) react("cool");
        await loadWinsettings(true);
        break;
      }
      case "win-undo": {
        const id = el.dataset.id;
        const r = await act(invoke("undo_setting", { id }));
        if (r) state.winFixed = state.winFixed.filter((x) => x.id !== id);
        await loadWinsettings(true);
        break;
      }
      case "files-scan":
        state.files = { ...(state.files || {}), busy: true };
        state.filesProgress = null;
        state.filesConfirm = false;
        invoke("scan_files");
        render();
        break;
      case "files-stop":
        invoke("stop_files_scan");
        break;
      case "files-keep":
        e.preventDefault();
        state.filesKeep[el.dataset.hash] = el.dataset.path;
        state.filesUntick.delete(el.dataset.path);
        state.filesConfirm = false;
        render();
        break;
      case "files-pick":
        el.checked ? state.filesUntick.delete(el.dataset.path) : state.filesUntick.add(el.dataset.path);
        state.filesConfirm = false;
        render();
        break;
      case "big-pick":
        el.checked ? state.bigTicked.add(el.dataset.path) : state.bigTicked.delete(el.dataset.path);
        state.filesConfirm = false;
        render();
        break;
      case "toggle-copies":
        state.showAllCopies = !state.showAllCopies;
        render();
        break;
      case "files-recycle": {
        const r = state.files && state.files.report;
        if (!r || state.filesWorking) return;
        const c = filesChosen(r);
        if (!c.paths.length) return;
        clearTimeout(filesTimer);
        if (!state.filesConfirm) {
          state.filesConfirm = true;
          render();
          filesTimer = setTimeout(() => {
            state.filesConfirm = false;
            render();
          }, 6000);
          return;
        }
        state.filesConfirm = false;
        state.filesWorking = true;
        render();
        const done = await act(invoke("recycle_files", { paths: c.paths }));
        state.filesWorking = false;
        if (done && done.freed > 0) react("cool");
        state.filesUntick = new Set();
        state.bigTicked = new Set();
        state.filesKeep = {};
        await loadFiles();
        break;
      }
      case "close-sheet":
        closeSheet();
        break;
      case "pick": {
        const id = state.sheet && state.sheet.id;
        const s = state.trash.spots.find((x) => x.id === id);
        const i = s && s.items.find((x) => x.path === el.dataset.path);
        if (!i) return;
        const pk = pickOf(id);
        // A change from how its list started: an untick if it started ticked, else a tick.
        const changes = startsTicked(pk, i) ? pk.unticked : pk.ticked;
        el.checked === startsTicked(pk, i) ? changes.delete(i.path) : changes.add(i.path);
        state.cleanConfirm = null;
        renderSheet();
        break;
      }
      case "pick-all": {
        const id = state.sheet && state.sheet.id;
        const s = state.trash.spots.find((x) => x.id === id);
        if (!s) return;
        const pk = pickOf(id);
        const goes = el.dataset.list === "goes";
        const list = s.items.filter((i) => (goes ? i.goes : !i.goes && i.optional));
        const on = !allTicked(pk, goes ? pk.goes : pk.new, list);
        if (goes) pk.goes = on;
        else pk.new = on;
        for (const i of list) {
          pk.ticked.delete(i.path);
          pk.unticked.delete(i.path);
        }
        state.cleanConfirm = null;
        renderSheet();
        break;
      }
      case "clean-spot":
        cleanSpot(el.dataset.spot);
        break;
      case "choose-archive": {
        const info = await act(invoke("choose_archive_dir"), () => null);
        if (info && info.dir) {
          state.archive = info;
          toast(`Old conversations will go to ${info.dir}.`);
          renderSheet();
        }
        break;
      }
      case "bring-back": {
        state.bringing = el.dataset.from;
        renderSheet();
        await act(invoke("bring_back", { from: el.dataset.from }));
        state.bringing = null;
        loadArchive();
        break;
      }
      case "delete-moved": {
        const from = el.dataset.from;
        clearTimeout(cleanTimer);
        if (state.deleteConfirm !== from) {
          // First press asks; this one can't be undone.
          state.deleteConfirm = from;
          renderSheet();
          cleanTimer = setTimeout(() => {
            state.deleteConfirm = null;
            renderSheet();
          }, 6000);
          break;
        }
        state.deleteConfirm = null;
        state.bringing = from;
        renderSheet();
        await act(invoke("delete_moved", { from }));
        state.bringing = null;
        loadArchive();
        break;
      }
      case "cancel-clean":
        clearTimeout(cleanTimer);
        state.cleanConfirm = null;
        renderSheet();
        break;
      case "add-room": {
        const p = await act(invoke("add_room"), () => null);
        if (typeof p === "string") {
          if (!state.roomPaths.includes(p)) state.roomPaths.push(p);
          state.roomOpen = p;
          render();
          inspectRoom(p);
        }
        break;
      }
      case "knock":
        state.knocked = true;
        react("shocked", 1100);
        render();
        setTimeout(() => {
          react("wink", 2600);
          say("Oh, it's you. Come in, but you're helping.", 3800);
          enterRoom();
        }, 1150);
        break;
      case "enter-room":
        enterRoom();
        break;
      case "leave-room":
        leaveRoom();
        break;
      case "room-again":
        leaveRoom();
        enterRoom();
        break;
      case "pr-chore":
        if (playroom && playroom.game) playroom.game.act(el.dataset.chore);
        break;
      case "open-room":
        state.knocked = false;
        state.roomOpen = el.dataset.room;
        render();
        if (!state.rooms[state.roomOpen]) inspectRoom(state.roomOpen);
        break;
      case "close-room":
        state.roomOpen = null;
        render();
        break;
      case "room-pick-all": {
        const r = state.rooms[state.roomOpen];
        if (!r) return;
        const sel = state.roomSel[state.roomOpen] || new Set();
        state.roomSel[state.roomOpen] = r.actions.every((a) => sel.has(a.id)) ? new Set() : new Set(r.actions.map((a) => a.id));
        render();
        break;
      }
      case "files-pick-all": {
        const r = state.files && state.files.report;
        if (!r) return;
        const extras = r.groups.flatMap((g) => extrasOf(g).map((f) => f.path));
        state.filesUntick = extras.every((x) => !state.filesUntick.has(x)) ? new Set(extras) : new Set();
        state.filesConfirm = false;
        render();
        break;
      }
      case "big-pick-all": {
        const r = state.files && state.files.report;
        if (!r) return;
        const free = r.big.filter((b) => !b.kept);
        state.bigTicked = free.every((b) => state.bigTicked.has(b.path)) ? new Set() : new Set(free.map((b) => b.path));
        state.filesConfirm = false;
        render();
        break;
      }
      case "toggle-action": {
        const sel = state.roomSel[state.roomOpen] || (state.roomSel[state.roomOpen] = new Set());
        el.checked ? sel.add(el.dataset.aid) : sel.delete(el.dataset.aid);
        render();
        break;
      }
      case "apply-room": {
        const p = state.roomOpen;
        const ids = [...(state.roomSel[p] || [])];
        state.roomBusy = true;
        render();
        if (await act(invoke("apply_tidy", { path: p, ids }))) react("delighted");
        state.roomBusy = false;
        await inspectRoom(p);
        break;
      }
      case "copy-chores": {
        const text = await act(invoke("chore_prompt", { path: state.roomOpen }), () => null);
        if (typeof text === "string") {
          try {
            await navigator.clipboard.writeText(text);
            toast("Copied. Paste it to the agent that made the mess.");
          } catch {
            toast("Couldn't reach the clipboard.", true);
          }
        }
        break;
      }
      case "undo-room":
        await act(invoke("undo_tidy"));
        if (state.roomOpen) await inspectRoom(state.roomOpen);
        break;
      case "rescan-room":
        inspectRoom(state.roomOpen);
        break;
      case "reveal-room":
        act(invoke("reveal", { path: state.roomOpen }), () => null);
        break;
      case "remove-room":
        await act(invoke("remove_room", { path: state.roomOpen }), () => "I'll stop nagging about that one.");
        state.roomPaths = state.roomPaths.filter((p) => p !== state.roomOpen);
        delete state.rooms[state.roomOpen];
        delete state.roomErrors[state.roomOpen];
        state.roomOpen = null;
        render();
        break;
      case "edit-rules":
        state.rulesEditing = true;
        state.rulesDraft = null;
        render();
        $("#rules-text").focus();
        break;
      case "cancel-rules":
        state.rulesEditing = false;
        state.rulesDraft = null;
        document.activeElement.blur();
        render();
        break;
      case "save-rules": {
        const text = $("#rules-text").value;
        if (await act(invoke("save_rules", { text }))) {
          state.rulesEditing = false;
          state.rulesDraft = null;
          document.activeElement.blur();
        }
        loadRules();
        break;
      }
      case "reset-rules":
        await act(invoke("reset_rules"));
        state.rulesDraft = null;
        loadRules();
        break;
      case "teach": {
        const on = el.getAttribute("aria-checked") !== "true";
        await act(invoke("teach_rules", { agent: el.dataset.agent, on }));
        loadRules();
        break;
      }
      case "integration": {
        const on = el.getAttribute("aria-checked") !== "true";
        await act(invoke("set_integration", { id: el.dataset.id, on }));
        loadRules();
        break;
      }
    }
  });

  document.addEventListener("keydown", (e) => {
    if ((e.key === "Enter" || e.key === " ") && e.target.matches("[role=button]")) {
      e.preventDefault();
      e.target.click();
    }
    if (e.key === "Escape" && playroom) leaveRoom();
    else if (e.key === "Escape" && state.sheet) closeSheet();
    else if (e.key === "Escape" && state.reader) closeReader();
  });

  // The scrollbar shows while you scroll, then fades away.
  for (const el of [$("#page"), $("#sheet-body")]) {
    if (!el) continue;
    let fade = 0;
    el.addEventListener(
      "scroll",
      () => {
        el.classList.add("scrolling");
        clearTimeout(fade);
        fade = setTimeout(() => el.classList.remove("scrolling"), 900);
      },
      { passive: true }
    );
  }

  /** A strip of subtabs fades at whichever edge has more, and shows the chosen one. */
  function stripEdges() {
    for (const strip of document.querySelectorAll(".subtabs")) {
      const more = strip.scrollWidth - strip.clientWidth > 2;
      const chosen = strip.querySelector('[aria-selected="true"]');
      if (more && chosen) {
        const left = chosen.offsetLeft - 8;
        const right = chosen.offsetLeft + chosen.offsetWidth + 28;
        if (left < strip.scrollLeft) strip.scrollLeft = left;
        else if (right > strip.scrollLeft + strip.clientWidth) strip.scrollLeft = right - strip.clientWidth;
      }
      strip.classList.toggle("more-left", more && strip.scrollLeft > 2);
      strip.classList.toggle("more-right", more && strip.scrollLeft < strip.scrollWidth - strip.clientWidth - 2);
    }
  }
  document.addEventListener("scroll", (e) => e.target.classList && e.target.classList.contains("subtabs") && stripEdges(), { capture: true, passive: true });

  document.addEventListener("input", (e) => {
    if (e.target.id === "rules-text") state.rulesDraft = e.target.value;
    if (e.target.id === "list-q") {
      state.listView.q = e.target.value;
      renderSheet();
    }
    if (e.target.id === "prog-q") {
      state.progView.q = e.target.value;
      render();
    }
  });

  // Settings switches and numbers live in the sheet.
  $("#sheet-body").addEventListener("click", async (e) => {
    const sw = e.target.closest("[data-set]");
    if (!sw) return;
    const key = sw.dataset.set;
    const on = sw.getAttribute("aria-checked") !== "true";
    if (key === "autostart") {
      const r = await act(invoke("set_autostart", { on }), () => (on ? "I'll be here when you log in." : "Okay, I'll wait to be asked."));
      if (r !== null) state.settings.autostart = r === true;
      renderSheet();
      return;
    }
    if (key === "paused") {
      if ((await act(invoke("set_paused", { paused: on }), () => null)) !== null) state.paused = on;
      renderSheet();
      renderNote();
      renderChecked();
      return;
    }
    const saved = await saveSettings((c) => {
      if (key === "autoStop") c.leftovers.autoStop = on;
      if (key === "autoStopOrphans") c.leftovers.autoStopOrphans = on;
      if (key === "includeOrphans") c.leftovers.includeUnknownOrphans = on;
      if (key === "notifications") c.ui.notifications = on;
      if (key === "notifyLeftovers") c.ui.notifyLeftovers = on;
      if (key === "notifyMeltdown") c.ui.notifyMeltdown = on;
      if (key === "notifyCleanups") c.ui.notifyCleanups = on;
      if (key === "notifySkills") c.ui.notifySkills = on;
      if (key === "alwaysOnTop") c.ui.alwaysOnTop = on;
      if (key === "checkUpdates") c.ui.checkUpdates = on;
    });
    if (!saved) return;
    if (key === "checkUpdates" && on) loadUpdate();
    if (key === "autoStop") toast(on ? "Deal. I'll clean up after them myself." : "Okay. I'll just tell you.");
    if (key === "notifications") toast(on ? "I'll let you know when it matters." : "Okay. Not a peep.");
  });

  $("#sheet-body").addEventListener("change", (e) => {
    const input = e.target.closest("[data-num]");
    if (!input) return;
    const v = Number(input.value);
    if (!Number.isFinite(v)) return;
    saveSettings((c) => {
      const key = input.dataset.num;
      if (key === "autoStopAfterMinutes") c.leftovers.autoStopAfterMinutes = Math.max(1, Math.round(v));
      if (key === "cpuAlertPercent") c.ui.cpuAlertPercent = Math.min(100, Math.max(5, v));
      if (key === "notifyAfterMinutes") c.ui.notifyAfterMinutes = Math.min(1440, Math.max(1, Math.round(v)));
      if (key === "notifyEveryMinutes") c.ui.notifyEveryMinutes = Math.min(1440, Math.max(5, Math.round(v)));
      if (key === "scanIntervalSecs") c.ui.scanIntervalSecs = Math.min(60, Math.max(1, Math.round(v)));
      if (key === "agentScratchDays") c.trash.agentScratchDays = Math.min(365, Math.max(1, Math.round(v)));
      if (key === "tempMinAgeDays") c.trash.tempMinAgeDays = Math.min(90, Math.max(1, Math.round(v)));
      if (key === "conversationsAfterDays") c.trash.conversationsAfterDays = Math.min(3650, Math.max(1, Math.round(v)));
    }).then((saved) => {
      // What counts as old just changed, so the clean-ups need measuring again.
      if (saved && ["agentScratchDays", "tempMinAgeDays", "conversationsAfterDays"].includes(input.dataset.num)) invoke("measure_trash");
    });
  });

  // Moving the widget: grab it by anything that isn't a control. Grab mom and she comes along;
  // click her and she reacts.
  const CONTROLS = "button, a, input, textarea, select, label, [role=button], [data-act], .md, .sheet-panel, .page";
  document.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || !appWindow || e.target.closest("#face") || e.target.closest(CONTROLS)) return;
    if (!e.target.closest(".titlebar, .note, .tabs, .pr-head")) return;
    e.preventDefault();
    appWindow.startDragging().catch(() => {});
  });

  const face = $("#face");
  let press = null;
  face.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    press = { x: e.clientX, y: e.clientY, id: e.pointerId };
    face.setPointerCapture(e.pointerId);
  });
  face.addEventListener("pointermove", (e) => {
    if (!press || Math.hypot(e.clientX - press.x, e.clientY - press.y) < 5) return;
    try {
      face.releasePointerCapture(press.id);
    } catch {}
    press = null;
    if (appWindow) appWindow.startDragging().catch(() => {});
  });
  face.addEventListener("pointerup", () => {
    if (press) boop();
    press = null;
  });
  face.addEventListener("pointercancel", () => (press = null));
  face.addEventListener("pointerenter", () => mom && mom.hover(true));
  face.addEventListener("pointerleave", () => mom && mom.hover(false));
  face.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      boop();
    }
  });

  document.querySelectorAll(".tab").forEach((b) => b.addEventListener("click", () => selectTab(b.dataset.tab)));
  $("#btn-clean").addEventListener("click", cleanAll);
  $("#btn-refresh").addEventListener("click", () => {
    invoke("refresh");
    if (state.tab === "upkeep") {
      invoke("measure_trash");
      loadHealth(true);
    }
    if (state.tab === "rooms" && state.roomOpen) inspectRoom(state.roomOpen);
    if (moving()) G.fromTo("#btn-refresh svg", { rotation: 0 }, { rotation: 360, duration: 0.6, ease: "power2.inOut", transformOrigin: "50% 50%" });
  });
  $("#btn-settings").addEventListener("click", () => (state.sheet && state.sheet.kind === "settings" ? closeSheet() : openSheet({ kind: "settings" })));
  $("#btn-about").addEventListener("click", () => (state.sheet && state.sheet.kind === "about" ? closeSheet() : openSheet({ kind: "about" })));
  $("#btn-hide").addEventListener("click", () => invoke("hide_window"));
  $("#btn-big").addEventListener("click", () => setBigMode(!state.big));
  $("#btn-pin").addEventListener("click", async () => {
    if (!state.settings) state.settings = await invoke("get_settings");
    const on = !state.settings.config.ui.alwaysOnTop;
    await saveSettings((c) => (c.ui.alwaysOnTop = on));
  });
  window.addEventListener("resize", () => moveThumb(false));

  // ------------------------------------------------------------ boot

  (async () => {
    mom = window.momFace.mount(face);
    showFace();
    moveThumb(false);
    if (moving()) {
      G.from(".note", { opacity: 0, y: 10, duration: 0.6, ease: "power3.out", clearProps: "opacity,transform" });
      G.from(".tabs", { opacity: 0, y: 8, duration: 0.6, delay: 0.08, ease: "power3.out", clearProps: "opacity,transform" });
    }
    try {
      state.settings = await invoke("get_settings");
      $("#btn-pin").setAttribute("aria-pressed", String(state.settings.config.ui.alwaysOnTop));
    } catch {}
    applySnap(await invoke("get_snapshot"));
    state.ready = true;
    loadStopped();
    loadAgents();
    loadRules().catch(() => {});
    bridge.listen("skills", () => loadRules().catch(() => {}));
    // New versions: a first look once things have settled, then now and then (she asks GitHub at
    // most once a day, however often this runs).
    setTimeout(loadUpdate, 20000);
    setInterval(loadUpdate, 6 * 3600 * 1000);
    bridge.listen("snapshot", (e) => applySnap(e.payload));
    // Watchdog: if updates stop arriving (a hidden window can miss them), go and fetch one.
    setInterval(async () => {
      if (!state.paused && !document.hidden && Date.now() - state.lastSnapAt > 8000) {
        try {
          applySnap(await invoke("get_snapshot"));
        } catch {}
      }
      renderChecked();
      if (state.tab === "upkeep" && !document.hidden) loadHealth();
      const watching = state.tab === "running" && state.runningTab === "speed" && !document.hidden;
      const age = Date.now() - state.speedAt;
      if (age > (watching ? 10000 : 300000) && !state.paused) loadSpeed();
      const here = (t) => state.tab === "running" && state.runningTab === t && !document.hidden;
      // Not while a button is waiting for its "Sure?", so the row doesn't move under the pointer.
      if (here("programs") && Date.now() - state.programsAt > 5000 && !state.progConfirm && !state.progBusy) loadPrograms();
      if (here("startup") && Date.now() - state.startupAt > 30000 && !state.startupBusy) loadStartup();
    }, 2000);
    document.addEventListener("visibilitychange", async () => {
      if (document.visibilityState !== "visible") return;
      invoke("refresh");
      try {
        applySnap(await invoke("get_snapshot"));
      } catch {}
      render();
    });
    bridge.listen("files-progress", (e) => {
      state.filesProgress = e.payload;
      if (state.tab === "upkeep" && state.upkeepTab === "files") render();
    });
    bridge.listen("files", (e) => {
      state.files = e.payload;
      state.filesProgress = null;
      if (state.tab === "upkeep") render();
    });
    bridge.listen("trash", (e) => {
      state.trash = e.payload;
      if (!state.cleaning) state.gone = new Set();
      if (state.tab === "upkeep") render();
      else if (state.sheet && state.sheet.kind === "spot") renderSheet();
    });
    state.trash = await invoke("get_trash");
    render();
  })();
})();

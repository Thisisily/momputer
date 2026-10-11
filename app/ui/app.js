/* momputer widget. Plain JS, no build step; the libraries it uses are copied into vendor/.
   Pages are re-drawn by morphing the DOM (idiomorph), so a button never disappears from under
   your pointer when fresh numbers arrive, and motion is GSAP. */
(() => {
  "use strict";

  const T = window.__TAURI__;
  const bridge = T ? { invoke: T.core.invoke, listen: T.event.listen } : window.momMock;
  const invoke = (cmd, args) => bridge.invoke(cmd, args);
  const G = window.gsap;
  const appWindow = T && T.window && T.window.getCurrentWindow ? T.window.getCurrentWindow() : null;
  const calm = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  // No animation when it can't be seen: a hidden window doesn't run them, and nothing should wait on one.
  const moving = () => G && !calm() && !document.hidden;

  const state = {
    snap: null,
    tab: "running",
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
    skip: {}, // clean-up id -> paths the user unticked
    add: {}, // clean-up id -> paths the user ticked under Stays
    cleanConfirm: null,
    cleaning: null,
    showAllStartup: false,
    roomPaths: [],
    rooms: {},
    roomOpen: null,
    roomSel: {},
    roomBusy: false,
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

  // ------------------------------------------------------------ little helpers

  const $ = (sel) => document.querySelector(sel);
  const esc = (s) =>
    String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
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

  /** Mostly she smiles, "c:". A raised eyebrow once a few things pile up, a flat look when it's a
      lot, and a stern one only when it's really bad. The light by her name says if she's stuck. */
  function faceNow() {
    const s = state.snap;
    if (state.paused) return "asleep";
    if (state.renderFailed) return "dizzy";
    if (reaction && Date.now() < reaction.until) return reaction.face;
    if (!s) return "look";
    if (melting(s)) return "melting";
    switch (s.mood.face) {
      case "disappointed":
        return s.mood.level >= 3 ? "unamused" : "hmm";
      case "furious":
        return "stern";
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
    if (melting(s)) alarm.innerHTML = `<span class="led red blink"></span>Leftovers are burning ${Math.round(s.summary.leftoverCpu)}% of your CPU.`;
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

  function renderRunning() {
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

    if (yours.length) html += section("yours", "Started by you", total(yours), yours.map((g) => row(g)).join(""));
    if (kept.length) html += section("kept", "Kept on purpose", total(kept), kept.map((g) => row(g)).join(""));

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

  function renderHealth() {
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
    const order = { warn: 0, tip: 1, ok: 2 };
    const checks = [...h.checks]
      .sort((a, b) => (order[a.level] ?? 1) - (order[b.level] ?? 1))
      .map(
        (c) => `<div class="check-row" id="chk-${hash(c.id)}" data-led="${c.level === "warn" ? "red" : c.level === "ok" ? "green" : "off"}">
          <div class="check-text"><strong>${esc(c.title)}</strong><span>${esc(c.detail)}</span></div>
          ${c.action ? `<button class="secondary" data-act="open-uri" data-url="${esc(c.action.url)}" title="Opens Windows Settings">${esc(c.action.label)}${ICON.out}</button>` : ""}
        </div>`
      )
      .join("");
    const top = h.topMemory
      .map(
        (p) => `<div class="mini-row" id="tm-${hash(p.name)}"><span class="mini-name">${esc(p.name)}${p.count > 1 ? ` <span class="dim">×${p.count}</span>` : ""}</span>
          ${meter(mem.total ? p.mem / mem.total : 0, { n: 24 })}<span class="num">${bytes(p.mem)}</span></div>`
      )
      .join("");
    const on = h.startup.filter((x) => x.enabled);
    const list = state.showAllStartup ? h.startup : h.startup.slice(0, 6);
    const startup = list
      .map(
        (x) => `<div class="mini-row" id="su-${hash(x.name + x.location)}" title="${esc(x.command)}"><span class="mini-name"><span class="led ${x.enabled ? "green" : ""}"></span>${esc(x.name)}</span>
          <span class="dim">${esc(x.location)}${x.enabled ? "" : ", off"}</span></div>`
      )
      .join("");
    const more = h.startup.length > 6 ? `<button class="quiet-toggle" data-act="toggle-startup">${state.showAllStartup ? "Show fewer" : `Show all ${h.startup.length}`}</button>` : "";
    return (
      section("pc", "Your PC", `up ${ageLong(h.uptimeSecs)}`, drives + memory) +
      (checks ? section("checks", "Checkups", "", checks) : "") +
      (top ? section("top", "Using the most memory", "", top) : "") +
      (h.startup.length
        ? section(
            "startup",
            "Starts with Windows",
            `${on.length} of ${h.startup.length} on`,
            startup + more + `<div class="group-foot"><button class="link-btn" data-act="open-uri" data-url="ms-settings:startupapps">Turn them on or off in Windows${ICON.out}</button></div>`
          )
        : "")
    );
  }

  function spotRow(s) {
    const gone = s.reclaimableBytes === 0;
    const led = gone ? "green" : s.reclaimableBytes >= 512 * 1024 ** 2 ? "red" : "off";
    const size = s.exact ? (gone ? "Clean" : bytes(s.reclaimableBytes)) : "Tool decides";
    const of = s.exact ? `of ${bytes(s.totalBytes)}` : bytes(s.totalBytes);
    return `<div class="row spot-row" id="spot-${s.id}" role="button" tabindex="0" data-act="open-spot" data-spot="${esc(s.id)}" data-led="${led}"
        aria-label="${esc(`${s.label}: ${size} can go. Show details`)}">
      <span class="row-name">${esc(s.label)}${s.safety === "review" ? ` <span class="badge look">look first</span>` : ""}</span>
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

  function renderUpkeep() {
    const t = state.trash;
    const spots = spotsOf();
    const said = t.busy && !spots.length ? "Measuring…" : t.busy ? "found so far, still looking." : spots.length ? "mom can free up. Tap anything to see exactly what it is." : "Let me look under the bed.";
    const shown = shownFree === null ? "" : sizeDots(shownFree);
    let html = `<div class="lead" id="upkeep-lead"><div class="lead-text">${spots.length ? `<div class="big" id="free-big">${shown}</div>` : ""}<p>${esc(said)}</p></div>
      <button class="secondary" data-act="measure" ${t.busy ? "disabled" : ""}>${t.busy ? `<span class="led green blink"></span>Measuring` : spots.length ? "Measure again" : "Measure"}</button></div>`;
    for (const [key, title] of GROUPS) {
      const list = spots.filter((x) => (x.group || "dev") === key).sort((a, b) => b.reclaimableBytes - a.reclaimableBytes || b.totalBytes - a.totalBytes);
      if (!list.length) continue;
      const sum = list.filter((x) => x.exact).reduce((a, x) => a + x.reclaimableBytes, 0);
      html += section(`g-${key}`, title, sum ? `${bytes(sum)} can go` : "", list.map(spotRow).join(""));
    }
    html += renderHealth();
    html += `<p class="foot">Nothing is cleaned until you open an item and press its button. Each one tells you exactly what it removes first.</p>`;
    return html;
  }

  // The details sheet for one clean-up: what's in it, what goes, what stays and why.

  const ACTION_WORD = {
    delete: "Deleted for good",
    recycle: "Goes to the Recycle Bin",
    tool: "Cleaned by its own tool",
    "empty-bin": "Emptied for good",
  };

  /** What the button takes: what goes, minus what's unticked, plus what's ticked under Stays. */
  function picked(s) {
    const skip = state.skip[s.id] || new Set();
    const add = state.add[s.id] || new Set();
    const items = s.items || [];
    const goes = items.filter((i) => i.goes);
    const unticked = goes.filter((i) => skip.has(i.path));
    const added = items.filter((i) => !i.goes && i.optional && add.has(i.path));
    const sum = (list) => list.reduce((a, i) => a + i.bytes, 0);
    return {
      goes,
      unticked,
      added,
      bytesChosen: Math.max(0, (s.reclaimableBytes || 0) - sum(unticked) + sum(added)),
      count: Math.max(0, (s.reclaimableItems || goes.length) - unticked.length + added.length),
      partial: unticked.length + added.length > 0,
    };
  }

  /** The spot's own explanation, adjusted for what the user ticked and unticked. */
  function effectNow(s, p) {
    if (!p.partial) return s.effect;
    const what = `${plural(p.count, "item")} (${bytes(p.bytesChosen)})`;
    if (p.goes.length === 0) {
      // Nothing would go by itself, so the spot's text says so; say what the ticks do instead.
      const one = p.count === 1;
      const ticked = one ? `the item you ticked (${bytes(p.bytesChosen)})` : `the ${what} you ticked`;
      return s.action === "recycle"
        ? `Moves ${ticked} to the Recycle Bin. Changed your mind? Restore ${one ? "it" : "them"} from there.`
        : `Deletes ${ticked} permanently (${one ? "it doesn't" : "they don't"} go to the Recycle Bin).`;
    }
    const changed = [p.unticked.length && `unticked ${p.unticked.length}`, p.added.length && `added ${p.added.length} from Stays`].filter(Boolean).join(" and ");
    return `${s.effect} You ${changed}, so ${what} will go.`;
  }

  function cleanLabel(s, p, confirming) {
    if (state.cleaning === s.id) return s.action === "recycle" ? "Moving to the Recycle Bin…" : "Cleaning…";
    if (s.action !== "tool" && p.count === 0) return (s.items || []).some((i) => i.optional) ? "Nothing ticked" : "Nothing to clear";
    if (confirming) {
      if (s.action === "recycle") return `Sure? Press again to move ${plural(p.count, "file")}`;
      return s.action === "tool" ? "Sure? Press again to run it" : "Sure? This can't be undone";
    }
    switch (s.action) {
      case "recycle":
        return `Move ${plural(p.count, "file")} to the Recycle Bin`;
      case "tool":
        return s.exact && s.reclaimableBytes ? `Clean up ${bytes(s.reclaimableBytes)}` : "Clean up";
      case "empty-bin":
        return `Empty the Recycle Bin (${bytes(p.bytesChosen)})`;
      default:
        return `Delete ${bytes(p.bytesChosen)}`;
    }
  }

  function itemRow(s, i, picking) {
    const skip = state.skip[s.id] || new Set();
    const sub = [i.sub, i.reason || (i.ageSecs ? `changed ${ageLong(i.ageSecs)} ago` : "")].filter(Boolean).join(" · ");
    // Tool clean-ups (Docker) list made-up paths; only real files get a folder button.
    const reveal = i.path && s.action !== "tool" ? `<button class="mini-btn" data-act="reveal-path" data-path="${esc(i.path)}" title="Show in File Explorer" aria-label="Show ${esc(i.name)} in File Explorer">${ICON.folder}</button>` : "";
    if (!i.goes && picking && i.optional) {
      // Kept only for being new: tick it to clear it anyway.
      const on = (state.add[s.id] || new Set()).has(i.path);
      return `<label class="item stays${on ? " added" : ""}" id="it-${hash(i.path || i.name)}">
        <input type="checkbox" data-act="pick" data-kind="add" data-path="${esc(i.path)}" ${on ? "checked" : ""}>
        <span class="item-main"><span class="item-name">${esc(i.name)}</span><span class="item-sub">${esc(sub)}</span></span>
        <span class="item-size num">${bytes(i.bytes)}</span>${reveal}</label>`;
    }
    if (!i.goes) {
      return `<div class="item stays" id="it-${hash(i.path || i.name)}"><span class="led green" title="Can't be cleared: ${esc(i.reason)}"></span><span class="item-main"><span class="item-name">${esc(i.name)}</span>
        <span class="item-sub">${esc(sub)}</span></span><span class="item-size num">${bytes(i.bytes)}</span>${reveal}</div>`;
    }
    return `<label class="item" id="it-${hash(i.path || i.name)}">
        ${picking ? `<input type="checkbox" data-act="pick" data-path="${esc(i.path)}" ${skip.has(i.path) ? "" : "checked"}>` : `<span class="led red"></span>`}
        <span class="item-main"><span class="item-name">${esc(i.name)}</span>${sub ? `<span class="item-sub">${esc(sub)}</span>` : ""}</span>
        <span class="item-size num">${bytes(i.bytes)}</span>${reveal}</label>`;
  }

  function renderSpot(id) {
    const s = state.trash.spots.find((x) => x.id === id);
    if (!s) return `<div class="sheet-head"><h2 id="sheet-title">Gone</h2><button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button></div><p class="empty">That one isn't there any more.</p>`;
    const p = picked(s);
    const stays = (s.items || []).filter((i) => !i.goes);
    const optional = stays.filter((i) => i.optional).length;
    const picking = s.action !== "tool" && s.action !== "empty-bin" && (p.goes.length > 1 || optional > 0);
    const unlisted = Math.max(0, (s.reclaimableItems || 0) - p.goes.length);
    const keptUnlisted = Math.max(0, (s.keptItems || 0) - stays.length);
    const go = s.reclaimableBytes || 0;
    const kept = s.keptBytes ?? Math.max(0, s.totalBytes - go);
    const cleaning = state.cleaning === s.id;
    const confirming = !cleaning && state.cleanConfirm === s.id;
    const disabled = cleaning || (s.action !== "tool" && (p.bytesChosen === 0 || p.count === 0));
    const danger = s.action === "delete" || s.action === "empty-bin";
    return `<div class="sheet-head">
        <div><h2 id="sheet-title">${esc(s.label)}</h2>
          <div class="pills"><span class="pill"><span class="led ${s.safety === "review" ? "red" : "green"}"></span>${s.safety === "review" ? "Look before you clear" : "Safe to clear"}</span>
          <span class="pill">${ACTION_WORD[s.action] || "Deleted"}</span></div></div>
        <button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button>
      </div>
      <p class="sheet-what">${esc(s.what)}</p>
      ${
        s.exact
          ? `<div class="split"><div class="split-bar"><span class="go" style="flex:${go || 0.0001}"></span><span class="stay" style="flex:${kept || 0.0001}"></span></div>
        <div class="split-legend num"><span><span class="led red"></span><strong>${bytes(go)}</strong> can go</span><span><span class="led green"></span>${bytes(kept)} stays</span><span class="dim">${bytes(s.totalBytes)} in all</span></div></div>`
          : `<div class="split-legend num"><span>${bytes(s.totalBytes)} in all. The tool works out what's safe to drop.</span></div>`
      }
      <div class="callout"><h3>What the button does</h3><p>${esc(effectNow(s, p))}</p></div>
      ${s.note ? `<p class="sheet-note"><span class="led red"></span>${esc(s.note)}</p>` : ""}
      ${
        p.goes.length
          ? `<h3 class="list-head">Goes <span class="dim num">${plural(s.reclaimableItems || p.goes.length, "item")}</span>${
              picking && p.goes.length > 1 ? `<button class="link-btn" data-act="pick-all">${p.unticked.length ? "Tick all" : "Untick all"}</button>` : ""
            }</h3>
        <div class="items">${p.goes.map((i) => itemRow(s, i, picking)).join("")}${unlisted ? `<p class="more">…and ${plural(unlisted, "smaller item")}.</p>` : ""}</div>`
          : ""
      }
      ${
        stays.length
          ? `<h3 class="list-head">Stays <span class="dim num">${plural(s.keptItems || stays.length, "item")}</span></h3>
        ${picking && optional ? `<p class="list-hint">${optional === stays.length ? "These are kept only because they're new" : "The ones with a box are kept only because they're new"}. Tick any you want gone too.</p>` : ""}
        <div class="items">${stays.map((i) => itemRow(s, i, picking)).join("")}${keptUnlisted ? `<p class="more">…and ${plural(keptUnlisted, "more")}.</p>` : ""}</div>`
          : ""
      }
      <div class="sheet-foot">
        ${
          confirming
            ? `<button class="link-btn" data-act="cancel-clean">Cancel</button>`
            : s.path
              ? `<button class="link-btn" data-act="reveal-path" data-path="${esc(s.path)}" title="Show in File Explorer">${ICON.folder}Show folder</button>`
              : "<span></span>"
        }
        <button class="primary${cleaning ? " busy" : confirming ? (danger ? " danger" : " armed") : ""}" id="clean-btn" data-act="clean-spot" data-spot="${esc(s.id)}" ${disabled ? "disabled" : ""}>${
          cleaning ? '<span class="led green blink"></span>' : confirming ? '<span class="led red blink"></span>' : danger ? '<span class="led red"></span>' : ""
        }${esc(cleanLabel(s, p, confirming))}</button>
      </div>`;
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
        const name = r ? r.name : p.split(/[\\/]/).pop();
        const meta = r ? `${plural(r.entries, "thing")} at the top level, ${r.findings.length} to sort out` : "Looking…";
        return `<div class="row room-row" id="room-${hash(p)}" data-room="${esc(p)}" data-act="open-room" tabindex="0" role="button">
          <span class="row-name">${esc(name)}</span>
          <div class="row-meta"><span class="why">${esc(meta)}</span></div>
          <div class="row-act">${r ? grade(r.grade, 3) : ""}</div>
        </div>`;
      })
      .join("");
    return html + section("rooms", "Rooms", "", rows);
  }

  const grade = (g, pitch) =>
    `<span class="grade g-${esc(g)}${pitch > 3 ? " big" : ""}" aria-label="grade ${esc(g)}"><span class="dot">${esc(g)}</span></span>`;

  function renderRoom(path) {
    const r = state.rooms[path];
    let html = `<button class="back" data-act="close-room">All rooms</button>`;
    if (!r) return html + section("room", "Looking", "", `<p class="empty">Looking…</p>`);
    const sel = state.roomSel[path] || new Set();
    html += `<div class="room-title"><h2>${esc(r.name)}</h2>${grade(r.grade, 4)}<p class="said">${esc(r.line)}</p></div>`;
    if (r.codeProject) {
      html += section("code", "Careful", "", `<p class="empty">This folder has code in it. Moving things can break paths, so nothing is ticked. "Copy chores for an agent" lets an agent tidy and fix the paths.</p>`);
    }
    const findings = r.findings.map((f) => `<div class="finding" id="f-${hash(f.title)}"><strong>${esc(f.title)}</strong><p>${esc(f.detail)}</p></div>`).join("");
    html += section("wrong", "What's wrong", "", findings || `<p class="empty">Nothing. Spotless.</p>`);
    if (r.actions.length) {
      const checks = r.actions
        .map(
          (a) => `<label class="check" id="a-${hash(a.id)}"><input type="checkbox" data-act="toggle-action" data-aid="${esc(a.id)}" ${sel.has(a.id) ? "checked" : ""}>
            <span>${esc(a.title)}<small>${esc(a.reason)}</small></span><span class="num">${a.bytes ? bytes(a.bytes) : ""}</span></label>`
        )
        .join("");
      html += section("plan", "Mom's plan", "", checks);
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
      state.rooms[p] = r;
      state.roomSel[p] = new Set(r.actions.filter((a) => a.selected).map((a) => a.id));
    } catch (e) {
      toast(String(e), true);
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

  function markdown(src) {
    const out = [];
    let para = [];
    let list = null;
    let fence = null;
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
    };
    for (const raw of String(src).replace(/\r/g, "").split("\n")) {
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

  function renderRules() {
    const r = state.rules;
    if (!r) return section("rules", "House rules", "", `<p class="empty">Fetching the rules…</p>`);
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
      html = `<div class="lead"><span class="lead-note">${r.customized ? "Your version. " : ""}Agents get these when you teach them below.</span>
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

  async function loadRules() {
    state.rules = await invoke("get_rules");
    render();
  }

  // ------------------------------------------------------------ settings

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
    return `<div class="sheet-head"><h2 id="sheet-title" class="dot sheet-dot">settings</h2>
        <button class="icon-btn" data-act="close-sheet" aria-label="Close">${ICON.close}</button></div>
      ${section(
        "s-clean",
        "Cleaning up",
        "",
        sw("autoStop", c.leftovers.autoStop, "Clean up on my own", `Turn off what agents leave behind once nobody has come back for it in ${num("autoStopAfterMinutes", c.leftovers.autoStopAfterMinutes, 1, 1440)} minutes.`) +
          sw("autoStopOrphans", c.leftovers.autoStopOrphans, "Orphans too", "Also turn off runtimes nobody can be traced to.") +
          sw("includeOrphans", c.leftovers.includeUnknownOrphans, "Count orphans as leftovers", "Runtimes whose parent is gone and carry no agent's fingerprints.")
      )}
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
          sw("notifyCleanups", c.ui.notifyCleanups, "Clean-up reports", "When mom turns things off by herself.", !c.ui.notifications)
      )}
      ${section(
        "s-widget",
        "The widget",
        "",
        sw("alwaysOnTop", c.ui.alwaysOnTop, "Keep on top", "Stay above other windows.") +
          sw("autostart", st.autostart, "Start with Windows", "Mom starts quietly in the tray when you log in.") +
          sw("paused", state.paused, "Take a break", `Stop watching until you switch this off. Otherwise mom looks every ${num("scanIntervalSecs", c.ui.scanIntervalSecs, 1, 60)} seconds.`)
      )}
      <p class="foot">momputer ${esc(st.version)}. Settings live in ${esc(st.configPath)}.</p>`;
  }

  async function saveSettings(mutate) {
    const st = state.settings;
    mutate(st.config);
    await act(invoke("set_settings", { config: st.config }), () => null);
    renderSheet();
    $("#btn-pin").setAttribute("aria-pressed", String(st.config.ui.alwaysOnTop));
    renderNote();
  }

  // ------------------------------------------------------------ the sheet (details and settings)

  function renderSheet() {
    if (!state.sheet) return;
    const html = state.sheet.kind === "spot" ? renderSpot(state.sheet.id) : renderSettings();
    morph($("#sheet-body"), html);
  }

  async function openSheet(sheet) {
    if (sheet.kind === "settings") state.settings = await invoke("get_settings");
    const wasOpen = !!state.sheet;
    state.sheet = sheet;
    state.cleanConfirm = null;
    renderSheet();
    $("#btn-settings").setAttribute("aria-pressed", String(sheet.kind === "settings"));
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
    const live = $("#live");
    live.classList.remove("blip");
    void live.offsetWidth;
    live.classList.add("blip");
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
      if (!state.trash.spots.length && !state.trash.busy) invoke("measure_trash");
      loadHealth();
    }
    if (tab === "rooms") loadRooms();
    if (tab === "rules") loadRules();
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
    btn.disabled = false;
    if (r) react("kiss");
    render();
    setTimeout(() => {
      ids.forEach((id) => state.done.delete(id));
      render();
    }, 1500);
  }

  let cleanTimer = null;

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
    renderSheet();
    const pick = p.partial ? { skip: p.unticked.map((i) => i.path), add: p.added.map((i) => i.path) } : null;
    const r = await act(invoke("clean_trash", { id, pick }));
    state.cleaning = null;
    delete state.skip[id];
    delete state.add[id];
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
        act(invoke("keep_group", { signature: g.signature, keep: !g.kept }), () => (g.kept ? "Fine, it can go when you say so." : "Got it. I'll leave that one alone."));
        if (!g.kept) react("wink");
        break;
      case "open-thread":
        act(invoke("open_url", { url: el.dataset.link }), () => null);
        break;
      case "toggle-helpers":
        state.showHelpers = !state.showHelpers;
        render();
        break;
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
      case "close-sheet":
        closeSheet();
        break;
      case "pick": {
        const id = state.sheet && state.sheet.id;
        if (!id) return;
        // Under Goes a tick is the default, so unticking skips it; under Stays, ticking adds it.
        const adding = el.dataset.kind === "add";
        const bag = adding ? state.add : state.skip;
        const set = bag[id] || (bag[id] = new Set());
        el.checked === adding ? set.add(el.dataset.path) : set.delete(el.dataset.path);
        state.cleanConfirm = null;
        renderSheet();
        break;
      }
      case "pick-all": {
        const id = state.sheet && state.sheet.id;
        const s = state.trash.spots.find((x) => x.id === id);
        if (!s) return;
        const skip = state.skip[id] || new Set();
        state.skip[id] = skip.size ? new Set() : new Set(s.items.filter((i) => i.goes).map((i) => i.path));
        renderSheet();
        break;
      }
      case "clean-spot":
        cleanSpot(el.dataset.spot);
        break;
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
      case "open-room":
        state.roomOpen = el.dataset.room;
        render();
        if (!state.rooms[state.roomOpen]) inspectRoom(state.roomOpen);
        break;
      case "close-room":
        state.roomOpen = null;
        render();
        break;
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
    if (e.key === "Escape" && state.sheet) closeSheet();
  });

  document.addEventListener("input", (e) => {
    if (e.target.id === "rules-text") state.rulesDraft = e.target.value;
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
      state.paused = on;
      await invoke("set_paused", { paused: on });
      renderSheet();
      renderNote();
      renderChecked();
      return;
    }
    await saveSettings((c) => {
      if (key === "autoStop") c.leftovers.autoStop = on;
      if (key === "autoStopOrphans") c.leftovers.autoStopOrphans = on;
      if (key === "includeOrphans") c.leftovers.includeUnknownOrphans = on;
      if (key === "notifications") c.ui.notifications = on;
      if (key === "notifyLeftovers") c.ui.notifyLeftovers = on;
      if (key === "notifyMeltdown") c.ui.notifyMeltdown = on;
      if (key === "notifyCleanups") c.ui.notifyCleanups = on;
      if (key === "alwaysOnTop") c.ui.alwaysOnTop = on;
    });
    if (key === "autoStop") toast(on ? "Deal. I'll clean up after them myself." : "Fine. I'll just tell you.");
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
    });
  });

  // Moving the widget: grab it by anything that isn't a control. Grab mom and she comes along;
  // click her and she reacts.
  const CONTROLS = "button, a, input, textarea, select, label, [role=button], [data-act], .md, .sheet-panel, .page";
  document.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || !appWindow || e.target.closest("#face") || e.target.closest(CONTROLS)) return;
    if (!e.target.closest(".titlebar, .note, .tabs")) return;
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
  $("#btn-hide").addEventListener("click", () => invoke("hide_window"));
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
    }, 2000);
    document.addEventListener("visibilitychange", async () => {
      if (document.visibilityState !== "visible") return;
      invoke("refresh");
      try {
        applySnap(await invoke("get_snapshot"));
      } catch {}
      render();
    });
    bridge.listen("trash", (e) => {
      state.trash = e.payload;
      if (state.tab === "upkeep") render();
      else if (state.sheet && state.sheet.kind === "spot") renderSheet();
    });
    state.trash = await invoke("get_trash");
    render();
  })();
})();

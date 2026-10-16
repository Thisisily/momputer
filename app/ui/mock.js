/* Pretend backend for working on the UI in a normal browser (open index.html).
   Only used when the page isn't running inside the momputer app. The data is modeled
   on a real Tuesday on a machine running far too many agents. */
(() => {
  if (window.__TAURI__) return;
  // ?demo, inside a frame: the widget as the live demo on momputer's web page (site/). It tells the
  // page around it when it wants to be BIG and waits for the page to make room, the way the real one
  // waits for its window; and its links open in a new tab.
  const DEMO = location.search.includes("demo") && window.parent !== window;
  const toPage = (word) => window.parent.postMessage({ momputer: word }, location.origin);
  const fromPage = (fn) => addEventListener("message", (e) => e.origin === location.origin && e.data && typeof e.data.momputer === "string" && fn(e.data.momputer));
  if (DEMO) {
    document.documentElement.classList.add("demo");
    addEventListener("load", () => setTimeout(() => toPage("ready"), 300));
    // The page asks for BIG MODE (its "See it live" button) or for the widget back (Esc, a click beside it).
    fromPage((word) => {
      const button = document.querySelector("#btn-big");
      const big = button && button.getAttribute("aria-pressed") === "true";
      if (button && ((word === "go-big" && !big) || (word === "go-small" && big))) button.click();
    });
  }
  const now = Math.floor(Date.now() / 1000);
  const H = 3600;
  const D = 86400;
  const MB = 1024 * 1024;
  let listeners = {};

  const owner = (agent, status, extra = {}) => ({
    kind: status === "orphaned" ? "nobody" : status === "yours" ? "you" : "agent",
    agent: status === "orphaned" || status === "yours" ? null : agent,
    label: status === "orphaned" ? "Nobody" : status === "yours" ? "You (a terminal)" : agent === "codex" ? "Codex" : "Claude Code",
    session: agent === "codex" ? null : "0191f20b-ae85-40af-b308-93c34777b8cb",
    role: status === "helper" ? "harness" : "task",
    alive: !["left-behind", "orphaned"].includes(status),
    sessionAlive: !["left-behind", "orphaned"].includes(status),
    agentPid: ["running", "forgotten", "helper"].includes(status) ? (agent === "codex" ? 22228 : 31748) : null,
    guessed: false,
    ...extra,
  });

  let gid = 0;
  const g = (name, status, o) => {
    gid++;
    const age = o.age;
    return {
      id: `${o.pid}-${now - age}`,
      rootPid: o.pid,
      rootStart: now - age,
      name,
      kind: o.kind || "node",
      category: o.category || "script",
      project: o.project ?? "storefront",
      cwd: o.cwd ?? `C:\\Workspace\\${o.project ?? "storefront"}`,
      command: o.command || name,
      pids: Array.from({ length: o.procs || 2 }, (_, i) => o.pid + i),
      ports: (o.ports || []).map((p) => ({ port: p, addr: "127.0.0.1", public: false })),
      mem: Math.round((o.mem || 50) * MB),
      cpu: o.cpu || 0,
      started: now - age,
      age,
      owner: owner(o.agent || "claude-code", status, o.owner),
      status,
      dupes: o.dupes || 0,
      signature: `${name}|${gid}`,
      kept: false,
      why: o.why || "",
    };
  };

  const groups = [
    g("bun run dev", "left-behind", { pid: 116524, age: 6 * D, mem: 916, cpu: 3.2, ports: [5173, 3001], category: "dev-server", kind: "bun", procs: 7, command: "sh /c/Users/me/AppData/Roaming/npm/bun run dev", why: "Claude left this running 6 days ago in storefront. That session is over." }),
    g("server/index.ts (watch)", "left-behind", { pid: 135184, age: 6 * D, mem: 316, cpu: 11.4, category: "watcher", kind: "bun", why: "Claude left this running 6 days ago in storefront. That session is over." }),
    g("browser-server.mjs", "left-behind", { pid: 96592, age: 6 * D, mem: 163, ports: [65221], project: "storefront (scratch)", category: "server" }),
    g("mint dev", "left-behind", { pid: 19612, age: 3 * D, mem: 59, ports: [3333], project: "storefront-docs", category: "dev-server" }),
    g("npm run dev --workspace @nimbus/frontend", "left-behind", { pid: 101428, age: 3 * D, mem: 51, ports: [5273], project: "Nimbus", category: "dev-server" }),
    g("until grep -q \"BUILD_EXIT\" build.log; do sleep 5; done", "left-behind", { pid: 52348, age: 4 * D, mem: 8, project: "games", category: "shell", kind: "shell", cpu: 0.6 }),
    g("scripts/serve-site-folder.ts", "left-behind", { pid: 59432, age: 13 * D, mem: 4, ports: [4791], category: "server", kind: "bun" }),
    g("tail -f proof-run-beta-2.log", "orphaned", { pid: 37148, age: 14 * D, mem: 1, category: "log-follower", kind: "tool" }),
    g("scripts/preview-setup.ts", "running", { pid: 18920, age: 3 * 60, mem: 309, ports: [6185], dupes: 1, category: "dev-server", kind: "bun" }),
    g("scripts/preview-setup.ts", "running", { pid: 9076, age: 6 * 60, mem: 302, ports: [6186], dupes: 1, category: "dev-server", kind: "bun" }),
    g("bun test v4 demo-reel", "running", { pid: 17540, age: 3 * 60, mem: 1530, cpu: 18.5, ports: [49934], category: "test-runner", kind: "bun" }),
    g("vite", "forgotten", { pid: 160460, age: 26 * H, mem: 397, ports: [5174], project: "atlas", category: "dev-server" }),
    g("server.mjs", "running", { pid: 34168, age: 8 * 60, mem: 123, project: "~", agent: "codex", category: "script" }),
    g("server-pdf (npx)", "helper", { pid: 32372, age: 10 * 60, mem: 269, category: "mcp-server" }),
    g("blender-mcp (uv)", "helper", { pid: 32460, age: 10 * 60, mem: 113, category: "mcp-server", kind: "python" }),
    g("codex-imagegen/index.js", "helper", { pid: 31428, age: 10 * 60, mem: 59, category: "mcp-server" }),
    g("pnpm --filter @atlas/web dev", "yours", { pid: 171104, age: 2 * H, mem: 528, ports: [3000], project: "atlas", category: "dev-server" }),
  ];

  const sessions = [
    { id: "0191f20b", agent: "claude-code", label: "Claude Code", pid: 31748, started: now - 10 * 60, age: 10 * 60, project: "storefront", projectDir: "C:\\Workspace\\storefront", entry: "cli", host: "Terminal", managed: false, mem: 210 * MB, helpers: 3, helperMem: 441 * MB, running: 3 },
    { id: "e9c3412f", agent: "claude-code", label: "Claude Code", pid: 32572, started: now - 8 * 60, age: 8 * 60, project: "momputer", projectDir: "C:\\Workspace\\momputer", entry: "claude-desktop", host: "Claude app", managed: true, mem: 180 * MB, helpers: 3, helperMem: 440 * MB, running: 0 },
    { id: "pid-22228", agent: "codex", label: "Codex", pid: 22228, started: now - 9 * 60, age: 9 * 60, project: "~", projectDir: "C:\\Users\\me", entry: null, host: "Codex app", managed: true, mem: 95 * MB, helpers: 3, helperMem: 149 * MB, running: 1 },
  ];

  const left = groups.filter((x) => x.status === "left-behind" || x.status === "orphaned");
  const snapshot = {
    takenAt: now,
    machine: { totalMem: 64 * 1024 * MB, usedMem: 41 * 1024 * MB, cpus: 24, processes: 866 },
    sessions,
    groups,
    machines: { docker: null, wsl: { running: [{ name: "Ubuntu", docker: false }], vmMem: 2150 * MB }, vms: [] },
    codex: {
      threads: [
        {
          id: "0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
          name: "Ship the launch video",
          cwd: "C:\\Workspace\\storefront",
          project: "storefront",
          created: now - 18 * H,
          updated: now - 40,
          working: true,
          subagents: 2,
          automation: { id: "publishing", name: "Daily standup notes", status: "ACTIVE", active: true, schedule: "14 times a day, 9:00 to 22:30", nextRun: now + 2 * H + 7 * 60, targetThread: "0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f" },
          link: "codex://threads/0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
        },
      ],
      automations: [],
      note: null,
    },
    summary: {
      leftovers: left.length,
      leftoverMem: left.reduce((a, x) => a + x.mem, 0),
      leftoverCpu: left.reduce((a, x) => a + x.cpu, 0),
      machineCpu: 64,
      forgotten: 1,
      running: 4,
      runtimeMem: groups.reduce((a, x) => a + x.mem, 0),
      helpers: 3,
      helperMem: 441 * MB,
      yours: 1,
      sessions: 3,
      containers: 0,
      byAgent: [],
    },
    mood: { face: "helping", level: 2, line: `${left.length} leftovers. One tap and they're gone.` },
  };

  const GB = 1024 * MB;
  const D2 = 86400;
  const item = (name, sub, bytes, goes, reason, ageSecs, files = 1) => ({ name, sub, path: `C:\\Users\\me\\AppData\\Local\\Temp\\${name.replace(/\W+/g, "-")}`, bytes, files, ageSecs, ageKnown: Number.isFinite(ageSecs), goes, optional: false, reason });
  // Kept only for being new: the user can tick it.
  const recent = (i) => ({ ...i, optional: true });
  const spot = (o) => {
    const fresh = (o.items || []).filter((i) => !i.goes && i.optional);
    const newBytes = fresh.reduce((a, i) => a + i.bytes, 0);
    return { exact: true, available: true, measuredAt: now, owner: null, note: null, path: null, keptBytes: o.totalBytes - o.reclaimableBytes, newItems: fresh.length, newBytes, ...o };
  };
  const games = [
    ["Unity games", "Lanternfall", "Player-prev.log", 506, 209],
    ["Unity games", "Lanternfall", "Player.log", 120, 209],
    ["Unity games", "Hollow Reed", "Player-prev.log", 241, 475],
    ["Unity games", "Hollow Reed", "Player.log", 88, 475],
    ["Unity games", "Copperline", "Player.log", 159, 1179],
    ["Unity games", "Quiet Harbor", "Player.log", 133, 278],
    ["Unity games", "Nimbus Drift", "Player.log", 114, 310],
    ["Unity games", "Nimbus Drift", "Player-prev.log", 97, 310],
    ["Unreal games", "Atlas Run", "Logs", 96, 365],
    ["Unreal games", "Atlas Run", "crash 2025-11-02", 76, 340],
    ["Unreal games", "Paper Moons", "Logs", 73, 448],
    ["itch games", "Small Hours", "crash reports", 41, 600],
  ].map(([who, game, what, mb, days]) => item(`${who} · ${game} · ${what}`, what.includes("crash") ? "Crash report" : "Log, 1 file", mb * MB, true, `Last changed ${days} days ago`, days * D2));

  const spots = [
    spot({
      id: "game-junk", label: "Games' logs and crash reports", group: "personal", action: "delete", safety: "safe",
      what: "Logs and crash reports games write every time they run (never your saves).",
      effect: "Deletes 12 logs and crash reports (1.7 GB) whose latest recorded change is over 67 days old, permanently (they don't go to the Recycle Bin). Games write new ones the next time they run; your saves, settings and the games themselves aren't touched.",
      totalBytes: games.reduce((a, i) => a + i.bytes, 0), reclaimableBytes: games.reduce((a, i) => a + i.bytes, 0), reclaimableItems: games.length, keptItems: 0,
      items: games,
    }),
    spot({
      id: "claude-temp", label: "Claude Code scratch", group: "agents", action: "delete", safety: "safe", owner: "claude-code",
      what: "Scratchpads, screenshots and task output Claude Code saves while it works. Each session gets its own folder.",
      effect: "Deletes 4 Claude Code scratch folders with no recorded change in the last 24 hours, permanently (they don't go to the Recycle Bin). Folders of the 2 sessions still running, recent changes and unknown ages are left alone.",
      path: "C:\\Users\\me\\AppData\\Local\\Temp\\claude", totalBytes: 15.2 * GB, reclaimableBytes: 1.2 * GB, reclaimableItems: 4, keptItems: 2,
      items: [
        item("storefront · \u201cUpload the garden video and reply to comments\u201d", "212 files, mostly screenshots", 820 * MB, true, "Last changed 3 days ago", 3 * D2, 212),
        item("atlas · \u201cFix the checkout page on mobile\u201d", "48 files, mostly logs", 260 * MB, true, "Last changed 5 days ago", 5 * D2, 48),
        item("games · Session 0191f20b", "12 files, mostly videos", 90 * MB, true, "Last changed 9 days ago", 9 * D2, 12),
        item("storefront · \u201cDraft the newsletter\u201d", "6 files", 41 * MB, true, "Last changed 2 days ago", 2 * D2, 6),
        item("momputer · \u201cMake the design nicer\u201d", "1,240 files, mostly screenshots", 13.6 * GB, false, "Session is still running", 600, 1240),
        item("storefront · \u201cSchedule tomorrow's posts\u201d", "30 files", 400 * MB, false, "Session is still running", 120, 30),
      ],
    }),
    spot({
      id: "codex-temp", label: "Codex leftovers", group: "agents", action: "delete", safety: "safe", owner: "codex", path: "C:\\Users\\me\\.codex",
      what: "Index and clipboard files Codex leaves in Temp, half-written state files and old sandbox logs.",
      effect: "Deletes 3 leftover Codex files permanently. Codex's real settings, threads and history are never touched.",
      totalBytes: 410 * MB, reclaimableBytes: 380 * MB, reclaimableItems: 3, keptItems: 1,
      items: [
        item("codex-index-8812", "Search index", 300 * MB, true, "Changed 4 days ago", 4 * D2),
        item("Sandbox logs", "14 logs", 70 * MB, true, "Older than 2 weeks", 15 * D2),
        item(".codex-global-state.json.tmp-1790", "Half-written file", 10 * MB, true, "Changed 6 days ago", 6 * D2),
        item("codex-index-9901", "Search index", 30 * MB, false, "Changed 2 hours ago (kept for 24 hours)", 7200),
      ],
    }),
    spot({
      id: "old-build-folders", label: "Old build folders", group: "dev", action: "delete", safety: "review",
      what: "node_modules and Rust target folders in projects with no recorded file activity in the last month. They're rebuilt from the project files whenever you need them.",
      effect: "Deletes the ticked folders permanently. Your code isn't touched; run npm install or cargo build in a project to get its folder back.",
      totalBytes: 9.8 * GB, reclaimableBytes: 6.1 * GB, reclaimableItems: 3, keptItems: 2,
      items: [
        item("garden-v1 · target", "Rust build output", 3.9 * GB, true, "Project activity recorded 74 days ago", 74 * D2),
        item("old-landing · node_modules", "41,200 files", 1.4 * GB, true, "Project activity recorded 45 days ago", 45 * D2),
        item("nimbus-2024 · node_modules", "28,900 files", 800 * MB, true, "Project activity recorded 120 days ago", 120 * D2),
        recent(item("storefront · node_modules", "52,000 files", 2.6 * GB, false, "Project activity recorded 2 hours ago", 7200)),
        item("momputer · target", "Rust build output", 1.1 * GB, false, "In use by a running program", 600),
      ],
    }),
    spot({
      id: "npm-cache", label: "npm cache", group: "dev", action: "delete", safety: "safe",
      what: "Copies of every package npm has downloaded, kept so installs are faster.",
      effect: "Deletes npm's download cache permanently. Nothing you've installed breaks; npm downloads packages again the next time a project needs them.",
      totalBytes: 2.1 * GB, reclaimableBytes: 2.1 * GB, reclaimableItems: 1, keptItems: 0,
      items: [item("_cacache", "18,200 files", 2.1 * GB, true, "Cache", 3 * D2, 18200)],
    }),
    spot({
      id: "uv-cache", label: "uv cache", group: "dev", action: "tool", safety: "safe", exact: false,
      what: "Python packages uv has downloaded and built.",
      effect: "Runs uv cache prune, which removes only what none of your environments use. Your virtual environments keep working.",
      totalBytes: 3.4 * GB, reclaimableBytes: 3.4 * GB, reclaimableItems: 0, keptItems: 0, items: [],
    }),
    spot({
      id: "browser-cache", label: "Browser caches", group: "windows", action: "delete", safety: "safe", note: "Close Edge to clear its cache.",
      what: "Copies of web pages, pictures and scripts your browsers saved so sites open faster.",
      effect: "Deletes only saved copies of pages, images and scripts from browsers and apps that are closed. Sites may load slower the first time. Cookies, sign-ins, site storage, session recovery data, history, bookmarks and passwords are kept.",
      totalBytes: 2.9 * GB, reclaimableBytes: 1.6 * GB, reclaimableItems: 2, keptItems: 1,
      items: [
        item("Chrome, Default profile", "Cache, Code Cache, GPUCache", 1.2 * GB, true, "Chrome is closed", 3600),
        item("Brave, Default profile", "Cache", 400 * MB, true, "Brave is closed", 7200),
        item("Edge, Default profile", "Cache, Code Cache", 1.3 * GB, false, "Edge is open", 60),
      ],
    }),
    spot({
      id: "old-conversations", label: "Old conversations", group: "agents", action: "move", safety: "safe", path: null,
      what: "Your agents' past conversations: Codex's and Claude Code's chats, images Codex made, and other agents' sessions. They add up to gigabytes. Moving the old ones to another drive frees this one, and a link in each one's place means the agents still find them where they were.",
      effect: "Moves 4 folders (3.1 GB) of Codex and Claude Code conversations whose latest recorded change is over 67 days old to a drive you choose, and leaves a link in each one's place, so they still open from where they were. Nothing is deleted, and Bring back puts any of them back here. While that drive is unplugged, those conversations can't be opened.",
      totalBytes: 4.4 * GB, reclaimableBytes: 3.1 * GB, reclaimableItems: 4, keptItems: 2,
      items: [
        item("Codex · July 2026", "2,140 files, mostly logs", 2.1 * GB, true, "Last changed 72 days ago", 72 * D2, 2140),
        item("Codex · June 2026", "610 files, mostly logs", 700 * MB, true, "Last changed 103 days ago", 103 * D2, 610),
        item("Claude Code · storefront · Fix the checkout page on mobile", "48 files, mostly code", 200 * MB, true, "Last changed 80 days ago", 80 * D2, 48),
        item("Codex · 2 August 2026", "120 files, mostly logs", 100 * MB, true, "Last changed 70 days ago", 70 * D2, 120),
        recent(item("Codex · 20 September 2026", "300 files, mostly logs", 1.1 * GB, false, "Last changed 21 days ago (kept here for 67 days)", 21 * D2, 300)),
        item("Codex · 11 October 2026", "40 files, mostly logs", 200 * MB, false, "Last changed 2 hours ago", 7200, 40),
      ],
    }),
    spot({
      id: "windows-junk", label: "Windows leftovers", group: "windows", action: "delete", safety: "review", admin: true,
      what: "Windows keeps update downloads, setup folders and error reports. Update downloads and setup folders stay while an update is pending, running, or waiting for a restart, or when update status cannot be checked. Cleaning other eligible leftovers needs an administrator, so Windows asks you first.",
      effect: "Deletes 1 old error report (100 MB) permanently. Update downloads and setup folders stay because Windows is waiting for a restart.",
      totalBytes: 1.2 * GB + 700 * MB, reclaimableBytes: 100 * MB, reclaimableItems: 1, keptItems: 2,
      items: [
        item("Windows Update · update files 8d2a1c0f9e7b", "Downloaded update, 412 files", 1.2 * GB, false, "Windows is waiting for a restart; update files stay", 88 * D2, 412),
        item("Windows setup · C:\\Windows10Upgrade", "Setup leftovers, 28 files", 600 * MB, false, "Windows is waiting for a restart; setup files stay", 1200 * D2, 28),
        item("Windows · AppCrash_atlas.exe_1f2e", "Error report", 100 * MB, true, "Last changed 90 days ago", 90 * D2, 3),
      ],
    }),
    spot({
      id: "forgotten-apps", label: "Forgotten app data", group: "personal", action: "move", safety: "review",
      what: "Big folders apps keep in AppData and your home folder: data attributed to uninstalled apps, older versions, and folders with no recent recorded changes. Review the folders first. Archive moves them to another drive with a link left behind. You can instead send selected folders to the Recycle Bin or delete them for good.",
      effect: "Moves 4 folders (6.2 GB) to a drive you choose, with a link left in each one's place: data of apps you've uninstalled. Nothing is deleted. Bring back puts any of them back, and once you're sure you don't need one, Delete for good removes it from there. Mom guesses which app a folder belongs to, so look through the list first.",
      totalBytes: 8.9 * GB, reclaimableBytes: 6.2 * GB, reclaimableItems: 4, keptItems: 2,
      items: [
        item("Nimbus Studio (AppData\\Roaming)", "41,200 files, mostly images", 3.9 * GB, true, "Its app isn't installed any more. Last changed 412 days ago", 412 * D2, 41200),
        item("Google\\AndroidStudio2021.2 (AppData\\Roaming)", "8,100 files", 1.1 * GB, true, "An older version; AndroidStudio2024.2 is installed. Last changed 705 days ago", 705 * D2, 8100),
        item("Atlas Engine (AppData\\LocalLow)", "310 files, mostly data files", 800 * MB, true, "Its app isn't installed any more. Last changed 290 days ago", 290 * D2, 310),
        item("SquirrelTemp (AppData\\Local)", "12 files", 400 * MB, true, "Its app isn't installed any more. Last changed 649 days ago", 649 * D2, 12),
        recent(item(".affinity (your home folder)", "2,900 files", 1.9 * GB, false, "Its app is still installed. Last changed 416 days ago", 416 * D2, 2900)),
        recent(item("Figma (AppData\\Local)", "600 files", 800 * MB, false, "Its app isn't installed any more. Last changed 101 days ago", 101 * D2, 600)),
      ],
    }),
    spot({
      id: "old-temp", label: "Old temp files", group: "windows", action: "delete", safety: "safe", path: "C:\\Users\\me\\AppData\\Local\\Temp",
      what: "Files programs and installers left in your Temp folder and never cleaned up.",
      effect: "Deletes 4,634 items (all empty) from your Temp folder with no recorded change in the last 7 days, permanently (they don't go to the Recycle Bin). Anything newer, in use by a running program, or with an unknown age stays. Programs make new temp files when they need them.",
      totalBytes: 1.4 * GB, reclaimableBytes: 0, reclaimableItems: 4634, keptItems: 8231, newItems: 8230, newBytes: 1.2 * GB,
      items: [
        item("tmp3F2A", "Empty folder", 0, true, "Last changed 41 days ago", 41 * D2, 0),
        item("tmp9C11", "Empty folder", 0, true, "Last changed 33 days ago", 33 * D2, 0),
        item("_MEI48212", "Empty folder", 0, true, "Last changed 12 days ago", 12 * D2, 0),
        recent(item("vscode-typescript", "1,204 files, mostly code", 410 * MB, false, "Changed 3 hours ago (kept for 7 days)", 3 * 3600, 1204)),
        item("Nimbus-updater", "88 files", 200 * MB, false, "In use by a running program", 60, 88),
        recent(item("pip-build-x81k", "310 files", 120 * MB, false, "Changed 2 days ago (kept for 7 days)", 2 * D2, 310)),
      ],
    }),
    spot({
      id: "recycle-bin", label: "Recycle Bin", group: "windows", action: "empty-bin", safety: "review",
      what: "Files you deleted. Windows keeps them here in case you change your mind.",
      effect: "Empties the Recycle Bin on every drive. This is permanent: the files can't be brought back.",
      totalBytes: 1.8 * GB, reclaimableBytes: 1.8 * GB, reclaimableItems: 312, keptItems: 0, path: "C:\\$Recycle.Bin",
      items: [
        item("render-final-v3.mp4", "from C:\\Workspace\\storefront\\out", 900 * MB, true, "Deleted 12 days ago", 12 * D2),
        item("dataset-old.zip", "from D:\\Downloads", 640 * MB, true, "Deleted 30 days ago", 30 * D2),
        item("screenshots (2).zip", "from C:\\Users\\me\\Desktop", 120 * MB, true, "Deleted 2 days ago", 2 * D2),
      ].map((i, n) => ({ ...i, path: `${n === 1 ? "D" : "C"}:\\$Recycle.Bin\\S-1-5-21-demo\\$RMOCK${n}${n ? ".zip" : ".mp4"}` })),
    }),
    spot({
      id: "crash-dumps", label: "Crash reports", group: "windows", action: "delete", safety: "safe",
      what: "Memory dumps and error reports apps saved when they crashed. Only useful to someone debugging that crash.",
      effect: "Deletes 9 crash reports permanently. Apps aren't affected.",
      totalBytes: 740 * MB, reclaimableBytes: 740 * MB, reclaimableItems: 9, keptItems: 0,
      items: [
        item("node.exe crash", "node.exe.18812.dmp", 420 * MB, true, "Saved 6 days ago", 6 * D2),
        item("Code.exe crash", "Code.exe.4410.dmp", 300 * MB, true, "Saved 20 days ago", 20 * D2),
      ],
    }),
    spot({
      id: "old-installers", label: "Old installers", group: "personal", action: "recycle", safety: "review", path: "C:\\Users\\me\\Downloads",
      what: "Setup files in Downloads whose latest recorded change is over a month old. Their dates do not establish whether you ran them.",
      effect: "Moves the ticked installers to the Recycle Bin, so you can still get them back. Installed apps aren't affected.",
      totalBytes: 1.1 * GB + 210 * MB, reclaimableBytes: 1.1 * GB, reclaimableItems: 3, keptItems: 1,
      items: [
        item("DockerDesktopInstaller.exe", "Downloads", 620 * MB, true, "Last changed 64 days ago", 64 * D2),
        item("Blender-4.2-windows-x64.msi", "Downloads", 380 * MB, true, "Last changed 90 days ago", 90 * D2),
        item("OBS-Studio-30.1-Installer.exe", "Downloads", 140 * MB, true, "Last changed 41 days ago", 41 * D2),
        recent(item("Figma-Setup.exe", "Downloads", 210 * MB, false, "Last changed 6 days ago (kept for 30 days)", 6 * D2)),
      ],
    }),
    // 0.5.0 C1
    spot({
      id: "downloads", label: "Downloads you're done with", group: "personal", action: "recycle", safety: "review", path: "C:\\Users\\me\\Downloads",
      what: "Things in your Downloads folders that have done their job: archives you've already unpacked (the folder next to each one stays), and installers for programs that are installed. Old disk images can be ticked. Exports of your own data are pointed out and stay.",
      effect: "Moves 3 files (2.6 GB) to the Recycle Bin: archives you've already unpacked (the folders stay) and installers for programs that are installed, plus any old disk images you tick. Changed your mind? Restore them from the Recycle Bin.",
      totalBytes: 2.6 * GB + 5.4 * GB + 880 * MB, reclaimableBytes: 2.6 * GB, reclaimableItems: 3, keptItems: 2,
      items: [
        { ...item("lanternfall-assets.zip", "Downloads", 1.9 * GB, true, "Already unpacked into lanternfall-assets", 40 * D2), kind: "unpacked archives" },
        { ...item("NimbusSetup.exe", "Downloads", 520 * MB, true, "Nimbus is installed", 12 * D2), kind: "installers" },
        { ...item("storefront-tools-2.1-x64.msi", "Downloads", 180 * MB, true, "storefront tools is installed", 33 * D2), kind: "installers" },
        { ...recent(item("atlas-os.iso", "Downloads", 5.4 * GB, false, "Unchanged for 140 days; tick it if you're done with it", 140 * D2)), kind: "disk images" },
        { ...item("chatgpt-export", "Downloads", 880 * MB, false, "Your ChatGPT export. It may be your only copy, so it stays", 200 * D2), kind: "data exports" },
      ],
    }),
    spot({
      id: "old-drivers", label: "Old driver versions", group: "windows", action: "delete", safety: "review", admin: true, path: "C:\\Windows\\System32\\DriverStore\\FileRepository",
      what: "Every driver update adds a copy to Windows' driver store, and Windows keeps the old ones in case the new one goes wrong. Older versions no device uses can go; the newest version of each driver, and any version a device uses, always stays.",
      effect: "Has Windows remove 2 old driver versions (1.6 GB) that no device uses. The newest version of each driver, and every version a device uses, stays. Windows asks you first, and refuses any a device still needs.",
      totalBytes: 2.4 * GB, reclaimableBytes: 1.6 * GB, reclaimableItems: 2, keptItems: 1,
      items: [
        { ...item("Lanternfall Graphics · lanternfall.inf 31.0.15.4601", "Driver package oem12.inf", 820 * MB, true, "Version 32.0.15.6094 replaced it, and no device uses it", 300 * D2), kind: "old driver versions" },
        { ...item("Lanternfall Graphics · lanternfall.inf 31.0.15.5222", "Driver package oem19.inf", 790 * MB, true, "Version 32.0.15.6094 replaced it, and no device uses it", 200 * D2), kind: "old driver versions" },
        { ...item("Lanternfall Graphics · lanternfall.inf 32.0.15.6094", "Driver package oem27.inf", 810 * MB, false, "A device uses it", 30 * D2), kind: "old driver versions" },
      ],
    }),
  ];

  // Where old conversations go (none chosen yet) and one that was moved before.
  let live = { done: 0, total: 0, freed: 0, files: 0, totalFiles: 0, current: "", gone: [], goneCount: 0, stopping: false };
  let liveStop = false;

  const mockFiles = () => {
    const f = (path, folder, days, keep = false) => ({ path, folder, modified: now - days * D2, keep });
    return {
      groups: [
        { size: 219 * MB, hash: "a1".repeat(32), kind: "videos", files: [f("C:\\Users\\me\\Videos\\launch-video-final.mp4", "Videos", 40, true), f("C:\\Users\\me\\Downloads\\launch-video-final (1).mp4", "Downloads", 39)] },
        { size: 90 * MB, hash: "b2".repeat(32), kind: "videos", files: [f("C:\\Users\\me\\Videos\\storefront-demo.mov", "Videos", 120, true), f("C:\\Users\\me\\Downloads\\storefront-demo.mov", "Downloads", 118), f("C:\\Users\\me\\Desktop\\storefront-demo.mov", "Desktop", 100)] },
        { size: 34 * MB, hash: "c3".repeat(32), kind: "archives", files: [f("C:\\Users\\me\\Documents\\nimbus-assets.zip", "Documents", 200, true), f("C:\\Users\\me\\Downloads\\nimbus-assets.zip", "Downloads", 200)] },
        { size: 4.7 * MB, hash: "d4".repeat(32), kind: "installers", files: [1, 2, 3, 4].map((n) => f(`C:\\Users\\me\\Downloads\\AtlasSetup${n === 1 ? "" : ` (${n - 1})`}.exe`, "Downloads", 300 - n, n === 1)) },
        { size: 6.1 * MB, hash: "e5".repeat(32), kind: "photos", files: [f("C:\\Users\\me\\Pictures\\Lake trip\\IMG_2041.jpg", "Pictures", 400, true), f("C:\\Users\\me\\Desktop\\IMG_2041.jpg", "Desktop", 390)] },
      ],
      wasted: 219 * MB + 2 * 90 * MB + 34 * MB + 3 * 4.7 * MB + 6.1 * MB,
      big: [{ path: "C:\\Users\\me\\Videos\\Raw footage\\atlas-run-capture.mkv", size: 14.2 * GB, accessed: now - 500 * D2, modified: now - 520 * D2, recency: now - 510 * D2, kind: "videos", folder: "Videos" }],
      scannedFiles: 8309,
      scannedBytes: 29.4 * GB,
      roots: [],
      complete: true,
      accessTimes: true,
      measuredAt: now - 2 * 3600,
    };
  };
  let filesReport = null;
  /** ?kept: one extra copy and the big file are things you said to keep (Settings, "Always kept"). */
  const markKept = (r) => {
    if (!location.search.includes("kept")) return r;
    const g = r.groups[1];
    g.files.forEach((f, i) => Object.assign(f, { kept: i === 2, keep: i === 2 }));
    r.big.forEach((b) => (b.kept = true));
    return r;
  };

  // Servers mom stopped earlier, with how they were started.
  let stopped = [
    { id: "4120-1", name: "vite", project: "lanternfall-site", command: "npm run dev", cwd: "C:\\Users\\you\\code\\lanternfall-site", ports: [5173], at: now - 2 * 3600 },
    { id: "5532-1", name: "uvicorn", project: "copperline-api", command: "uvicorn app.main:app --reload --port 8000", cwd: "C:\\Users\\you\\code\\copperline-api", ports: [8000], at: now - 26 * 3600 },
  ];

  /** A made-up icon: a coloured tile with the name's first letter. */
  const mockIcon = (name) => {
    const h = [...name].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 7);
    const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='8' fill='hsl(${h} 70% 48%)'/><text x='16' y='22' font-size='17' text-anchor='middle' fill='white' font-family='Segoe UI, sans-serif' font-weight='700'>${name.replace(/[^A-Za-z0-9]/g, "").charAt(0).toUpperCase()}</text></svg>`;
    return "data:image/svg+xml," + encodeURIComponent(svg);
  };

  let winsettings = [
    { id: "pagefile", area: "Memory", level: "check", title: "The page file has a fixed size", now: "Fixed at 8,192 MB on C:", default: "Managed by Windows",
      why: "A fixed page file can run out when memory gets tight, and apps then crash instead of slowing down. Windows sizes it well on its own.",
      fix: { label: "Let Windows manage it", admin: true, undo: false, after: "Takes effect after a restart." }, settings: "SystemPropertiesPerformance.exe" },
    { id: "paging-executive", area: "Memory", level: "check", title: "Windows is told to keep all its own code in memory", now: "On", default: "Off",
      why: "An old speed-up tip from the days of slow disks. Today it only holds memory other apps could use.",
      fix: { label: "Turn it off", admin: true, undo: false, after: "Takes effect after a restart." }, settings: null },
    { id: "myths", area: "Memory", level: "check", title: "Old speed-up tweaks that do nothing", now: "SecondLevelDataCache, IoPageLockLimit", default: "Not set",
      why: "These come from guides for Windows XP. Windows 10 and 11 ignore them.",
      fix: { label: "Remove them", admin: true, undo: false, after: null }, settings: null },
    { id: "path-user", area: "Paths", level: "check", title: "1 folder in your PATH is gone", now: "C:\\tools\\old-sdk\\bin", default: "Only folders that exist",
      why: "Every command you type looks there first, for nothing. Harmless, just clutter.",
      fix: { label: "Take it out", admin: false, undo: true, after: "Apps you start from now on see the change." }, settings: null },
    { id: "env-gone", area: "Paths", level: "check", title: "2 variables point at things that are gone", now: "NIMBUS_HOME → C:\\Nimbus, ATLAS_SDK → C:\\Atlas SDK", default: "Not set",
      why: "Left behind by apps you uninstalled. They do nothing now.", fix: null, settings: null },
    { id: "compat-orphans-user", area: "Paths", level: "info", title: "Compatibility settings for 2 programs that are gone", now: "Lanternfall.exe, Copperline.exe", default: "None",
      why: "Windows remembers 'run as administrator' and old-Windows modes per program, even after the program is gone.",
      fix: { label: "Forget them", admin: false, undo: true, after: null }, settings: null },
    { id: "powershell-7", area: "Terminal", level: "check", title: "PowerShell 7 isn't installed", now: "Windows PowerShell 5.1, line editor 2.0.0", default: "PowerShell 7 alongside it",
      why: "Windows PowerShell 5.1 is from 2016, and its line editor has known bugs: lines that wrap are drawn wrong and long sessions slow down. PowerShell 7 fixes them, speaks UTF-8, and installs alongside 5.1.",
      fix: null, settings: null, command: "winget install --id Microsoft.PowerShell --exact --source winget" },
    { id: "cmd-autorun-user", area: "Terminal", level: "check", title: "Something runs every time cmd starts", now: "\"C:\\Tools\\Nimbus\\condabin\\conda_hook.bat\"", default: "Nothing",
      why: "Usually conda's or a similar tool's set-up. It slows every cmd window and can break scripts that run cmd behind the scenes.",
      fix: { label: "Stop running it", admin: false, undo: true, after: null }, settings: null },
  ];

  const archive = {
    dir: null,
    free: null,
    reachable: false,
    moved: [
      { from: "C:\\Users\\me\\.codex\\sessions\\2026\\05", to: "F:\\momputer-archive\\C\\Users\\me\\.codex\\sessions\\2026\\05", archive: "F:\\momputer-archive", name: "Codex · May 2026", bytes: 900 * MB, files: 980, movedAt: now - 9 * D2, linked: true, reachable: true },
    ],
  };

  const health = {
    drives: [
      { root: "C:\\", label: "Windows", total: 476 * GB, free: 41 * GB, system: true, fullInDays: 23 },
      { root: "D:\\", label: "Data", total: 931 * GB, free: 512 * GB, system: false },
    ],
    memory: { total: 64 * GB, used: 47 * GB },
    uptimeSecs: 9 * D2 + 4 * 3600,
    checks: [
      { id: "restart", level: "tip", title: "Up 9 days", detail: "A restart clears out memory leaks and finishes updates.", action: null },
      { id: "unused-apps", level: "tip", title: "5 apps and games with older activity records", detail: "Their latest known activity dates are over a year old. Dates may reflect app data changes or Steam updates; apps with unknown activity are omitted. Review the records below before deciding what to uninstall.", action: null },
      { id: "storage-sense", level: "tip", title: "Storage Sense is off", detail: "Windows can empty old temp files and the Recycle Bin by itself.", action: { label: "Turn it on", url: "ms-settings:storagepolicies" } },
      { id: "startup", level: "tip", title: "11 apps start with Windows", detail: "Each one slows down logging in.", action: { label: "Startup apps", url: "ms-settings:startupapps" } },
      { id: "drive-C", level: "ok", title: "Drive C: has room", detail: "41 GB free of 476 GB.", action: null },
      { id: "memory", level: "ok", title: "Memory is fine", detail: "73% used. Chrome is using the most, 5.1 GB.", action: null },
    ],
    startup: [
      { name: "Docker Desktop", command: "Docker Desktop.exe -Autostart", location: "Run (you)", enabled: true },
      { name: "Discord", command: "Update.exe --processStart Discord.exe", location: "Run (you)", enabled: true },
      { name: "Steam", command: "steam.exe -silent", location: "Run (you)", enabled: false },
      { name: "OneDrive", command: "OneDrive.exe /background", location: "Run (you)", enabled: true },
      { name: "Spotify", command: "Spotify.exe --autostart", location: "Run (you)", enabled: true },
      { name: "Logi Options+", command: "logioptionsplus_agent.exe", location: "Run (all users)", enabled: true },
      { name: "Notion", command: "Notion.lnk", location: "Startup folder", enabled: true },
    ],
    unusedApps: [
      { key: "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\nimbus-notes", name: "Nimbus Notes 10.3", publisher: "Nimbus Labs", version: "10.3", size: 915 * MB, lastUsed: now - 1703 * D2, lastUsedSource: "activity", dir: "C:\\Program Files\\Nimbus Notes", from: null },
      { key: "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{AtlasRecorder}", name: "Atlas Recorder", publisher: "Atlas", version: "2.1", size: 400 * MB, lastUsed: now - 1067 * D2, lastUsedSource: "activity", dir: "C:\\Program Files (x86)\\Atlas Recorder", from: null },
      { key: "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\copperline-player", name: "Copperline Player", publisher: "Copperline", version: "3.3", size: 0, lastUsed: now - 2081 * D2, lastUsedSource: "activity", dir: null, from: null },
      { key: "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Steam App 900001", name: "Lanternfall Tactics", publisher: "Pixel Forge", version: "", size: 9.2 * GB, lastUsed: now - 1290 * D2, lastUsedSource: "played", dir: "E:\\SteamLibrary\\steamapps\\common\\Lanternfall Tactics", from: "Steam" },
      { key: "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Steam App 900002", name: "Retro Racer", publisher: "Pixel Forge", version: "", size: 4.7 * GB, lastUsed: now - 840 * D2, lastUsedSource: "updated", dir: "D:\\SteamLibrary\\steamapps\\common\\Retro Racer", from: "Steam" },
    ],
    topMemory: [
      { name: "chrome.exe", mem: 5.1 * GB, count: 38 },
      { name: "node.exe", mem: 3.4 * GB, count: 22 },
      { name: "Code.exe", mem: 2.2 * GB, count: 14 },
      { name: "vmmem", mem: 2.1 * GB, count: 1 },
      { name: "claude.exe", mem: 1.6 * GB, count: 6 },
      { name: "msedgewebview2.exe", mem: 900 * MB, count: 11 },
    ],
  };

  // What Speed finds: an example machine with a browser that lost the graphics card.
  const speed = {
    at: now - 20,
    findings: [
      { id: "no-gpu-chrome", level: "warn", title: "Chrome is drawing without your graphics card",
        detail: "Chrome draws video, scrolling and animation on the processor instead, which is why videos stutter. It happens when the graphics driver crashes or updates while Chrome is open, and lasts until Chrome restarts. Chrome has been open 2 days with 79 processes and 14.2 GB of memory; a restart brings every tab back.",
        fix: { label: "Copy chrome://restart", kind: "copy", value: "chrome://restart", hint: "Paste it into Chrome's address bar. Your tabs come back." } },
      { id: "no-gpu-chatgpt", level: "warn", title: "ChatGPT is drawing without your graphics card",
        detail: "ChatGPT draws video, scrolling and animation on the processor instead, which is why videos stutter. It happens when the graphics driver crashes or updates while ChatGPT is open, and lasts until ChatGPT restarts. Restarting ChatGPT fixes it.",
        fix: { label: "Restart ChatGPT", kind: "program", value: "app:chatgpt", hint: "3 Codex sessions run inside it, and stop when it closes." } },
      { id: "gpu-driver", level: "tip", title: "Your graphics driver crashed twice, then was updated",
        detail: "On 4 Oct, 21:37 and 5 Oct, 03:39. Nothing since the driver was updated 19 hours ago, so the update may well have fixed it. If it comes back, an overclock on the graphics card is the usual cause; otherwise, reinstall the driver from the maker's site.",
        timeline: { from: now - 14 * 86400, to: now, marks: [{ at: now - 8 * 86400 - 2 * 3600, kind: "crash" }, { at: now - 7 * 86400 - 20 * 3600, kind: "crash" }, { at: now - 19 * 3600, kind: "update" }] },
        fix: { label: "NVIDIA drivers", kind: "open", value: "https://www.nvidia.com/en-us/drivers/", hint: null },
        more: [{ label: "NVIDIA App", kind: "launch", value: "nvidia-app", hint: null }, { label: "Event Viewer", kind: "launch", value: "eventvwr.exe", hint: null }] },
      { id: "tuners", level: "tip", title: "GPU Tweak III is installed, not running",
        detail: "So nothing is overclocked right now. Your RTX 2070 SUPER runs at its standard power limit (215 W). If you overclocked with it when the crashes above happened, that's the likely cause: leave it at default.", fix: null },
      { id: "extras-iobit-uninstaller", level: "tip", title: "IObit Uninstaller runs all the time",
        detail: "It watches every program you install: 2 processes, 33 MB, 44% of a processor core all day. It starts with Windows by itself (through its own service), so if you end it, it's back after the next restart. Uninstalling it stops it for good.",
        fix: { label: "End IObit Uninstaller", kind: "end", value: "app:iobit uninstaller", hint: null },
        more: [{ label: "Uninstall IObit Uninstaller 16", kind: "uninstall", value: "HKLM\IObit Uninstaller_is1", hint: null }] },
    ],
    gpu: { name: "RTX 2070 SUPER", temp: 41, busy: 3, powerLimit: 215, defaultPowerLimit: 215 },
    // Thirty days: the two driver crashes above, and a few app crashes.
    stability: (() => {
      const today = now - (now % 86400);
      const days = Array.from({ length: 30 }, (_, i) => ({
        at: today - (29 - i) * 86400, blueScreens: 0, shutdowns: 0, gpuResets: i === 21 || i === 22 ? 1 : 0, hardwareErrors: 0, appCrashes: [3, 9, 15, 27].includes(i) ? 2 : 0,
      }));
      return { from: days[0].at, to: now, days, blueScreens: 0, shutdowns: 0, gpuResets: 2, hardwareErrors: 0, appCrashes: 8, apps: [["lanternfall.exe", 5], ["copperline.exe", 3]] };
    })(),
    busy: [
      { name: "Claude", cpu: 2.8, mem: 3.8 * GB, count: 23 },
      { name: "Desktop Window Manager", cpu: 0.9, mem: 230 * MB, count: 1 },
      { name: "Windows services", cpu: 0.7, mem: 1.5 * GB, count: 96 },
      { name: "UninstallMonitor", cpu: 0.7, mem: 20 * MB, count: 1 },
      { name: "Chrome", cpu: 0.4, mem: 14.2 * GB, count: 79 },
    ],
  };

  // Programs: what's running, as one line each.
  const prog = (name, kind, cpu, mem, count, extra = {}) => ({
    id: `app:${name.toLowerCase()}`, name, exe: extra.exe || `C:\\Program Files\\${name}\\${name}.exe`, cpu, avgCpu: cpu * 0.9, avgSecs: 240, mem, gpu: extra.gpu ?? null,
    count, windows: kind === "app" ? 1 : 0, kind, started: now - (extra.age || 3 * 3600), inside: extra.inside || null,
    can: kind === "app" ? ["restart", "quit", "end"] : kind === "background" ? ["restart", "end"] : [],
    procs: extra.procs || [],
  });
  // A test run's workers beside a dev server, in made-up projects.
  const line = (pid, what, place, cpu, mem, mins) => ({ pid, start: now - mins * 60, what, place, cpu, mem, canEnd: true });
  const bunLines = [
    line(4120, "bun --watch server/index.ts", "storefront", 0.2, 180 * MB, 300),
    line(4188, "bun run dev", "nimbus", 0.1, 140 * MB, 240),
    ...Array.from({ length: 6 }, (_, i) => line(5200 + i, "bun test --test-worker --isolate --timeout=60000", "atlas", 5.4 + i * 0.6, (1.6 + i * 0.2) * GB, 4)),
  ];
  const programs = [
    prog("bun", "background", 38.5, 12.4 * GB, 8, { exe: "C:\\Users\\me\\AppData\\Roaming\\npm\\node_modules\\bun\\bin\\bun.exe", procs: bunLines }),
    prog("ChatGPT", "app", 6.2, 2.4 * GB, 16, { inside: "3 Codex sessions", gpu: 2 }),
    prog("Chrome", "app", 3.1, 12.8 * GB, 70, { gpu: 4, age: 2 * 86400 }),
    prog("Claude", "app", 2.8, 3.8 * GB, 23, { inside: "2 Claude Code sessions", gpu: 7 }),
    prog("Spotify", "app", 0.4, 660 * MB, 8),
    prog("Steam", "app", 0.3, 874 * MB, 10),
    prog("Razer", "background", 1.8, 989 * MB, 21),
    prog("IObit Uninstaller", "background", 1.7, 33 * MB, 2),
    prog("Wallpaper Engine", "background", 0.1, 67 * MB, 1),
    prog("Microsoft Defender", "security", 1.2, 503 * MB, 1),
    prog("Windows services", "windows", 0.7, 1.5 * GB, 96),
    prog("Desktop Window Manager", "windows", 0.9, 230 * MB, 1, { gpu: 3 }),
    prog("Claude Code", "agent", 0.5, 993 * MB, 2),
    prog("momputer", "mom", 0.2, 120 * MB, 6),
  ];
  const places = [
    { path: "C:\\Users\\me\\Downloads", name: "Downloads", kind: "folder", on: true, total: 0, free: 0 },
    { path: "C:\\Users\\me\\Documents", name: "Documents", kind: "folder", on: true, total: 0, free: 0 },
    { path: "C:\\Users\\me\\Desktop", name: "Desktop", kind: "folder", on: true, total: 0, free: 0 },
    { path: "C:\\Users\\me\\Pictures", name: "Pictures", kind: "folder", on: true, total: 0, free: 0 },
    { path: "C:\\Users\\me\\Videos", name: "Videos", kind: "folder", on: true, total: 0, free: 0 },
    { path: "C:\\Users\\me\\OneDrive", name: "OneDrive", kind: "folder", on: false, total: 0, free: 0 },
    { path: "C:\\", name: "C: (Windows)", kind: "drive", on: false, total: 1000 * GB, free: 210 * GB },
    { path: "D:\\", name: "D: (Games)", kind: "drive", on: false, total: 2000 * GB, free: 640 * GB },
    { path: "F:\\", name: "F: (Backup)", kind: "drive", on: true, total: 4000 * GB, free: 1900 * GB },
  ];
  // Browsers: profiles and their extensions (made-up profile names).
  const ext = (name, kind, on, can, saved, extra = {}) => ({
    id: name.toLowerCase().replace(/[^a-z]/g, "").padEnd(32, "a").slice(0, 32), name, description: extra.description || "", version: "1.0", on, app: !!extra.app, theme: !!extra.theme, inUse: !!extra.inUse, site: extra.site || null, kind,
    can, everywhere: can[0] === "Reads and changes every site you visit", added: now - (extra.addedDays || 400) * 86400, lastSaved: saved == null ? null : now - saved * 86400,
    size: (extra.mb || 4) * MB, page: extra.app ? "chrome://apps" : "chrome://extensions/?id=x",
  });
  const everywhere = "Reads and changes every site you visit";
  const browsers = [
    {
      id: "chrome", name: "Chrome", open: true, openSecs: 2 * 86400 + 3 * 3600, processes: 78, mem: 13 * GB, restart: "chrome://restart", extensionsPage: "chrome://extensions",
      profiles: [
        { dir: "Default", path: "C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data\\Default", name: "Personal", lastUsed: now - 3000, size: 13.4 * GB, cache: 1.9 * GB, history: 123 * MB, cookiesBytes: 2.1 * MB, cookies: null, cookieSites: null,
          notes: [
            { level: "tip", text: "2 old Chrome apps are still installed: Desmos Graphing Calculator, Sketchpad. Chrome stopped running Chrome apps, so they can go." },
            { level: "tip", text: "2 ad blockers do the same job twice: AdBlock, AdBlocker Ultimate. One is enough, and each slows every page." },
            { level: "tip", text: "18 extensions can read and change every site you visit, your bank's included. Keep the ones you trust and use." },
          ],
          extensions: [
            ext("AdBlock", "Ad blocker", true, [everywhere, "Blocks or changes what pages load"], 0, { mb: 22 }),
            ext("AdBlocker Ultimate", "Ad blocker", true, [everywhere, "Blocks or changes what pages load"], 0, { mb: 9 }),
            ext("Grammarly", "AI assistant", true, [everywhere, "Sees your open tabs and their addresses"], 0, { mb: 31 }),
            ext("Phantom", "Crypto wallet", true, [everywhere], 0, { mb: 48 }),
            ext("VPN for Chrome", "VPN", true, ["Sends your browsing through its own servers", everywhere], 0, { mb: 6 }),
            ext("Desmos Graphing Calculator", null, true, [], null, { app: true, mb: 1, site: "desmos.com" }),
            ext("Sketchpad", null, true, [], null, { app: true, mb: 2 }),
            ext("Nami", "Crypto wallet", true, [everywhere], 321, { mb: 12 }),
            ext("Toby: Tab Manager", "Tab manager", false, ["Sees your open tabs and their addresses"], null, { mb: 3 }),
          ] },
        { dir: "Profile 3", path: "C:\\x", name: "Work", lastUsed: now - 3 * 86400, size: 6.9 * GB, cache: 5.5 * GB, history: 96 * MB, cookiesBytes: 1.2 * MB, cookies: null, cookieSites: null, notes: [],
          extensions: [ext("Claude", "AI assistant", true, [everywhere], 0, { mb: 12 })] },
        { dir: "Profile 7", path: "C:\\x", name: "Your Chrome (Profile 7)", lastUsed: now - 214 * 86400, size: 536 * MB, cache: 493 * MB, history: 320 * 1024, cookiesBytes: 400 * 1024, cookies: null, cookieSites: null, notes: [{ level: "tip", text: "The browser last recorded this profile as active 214 days ago." }], extensions: [] },
      ],
    },
    {
      id: "brave", name: "Brave", open: false, openSecs: 0, processes: 0, mem: 0, restart: "brave://restart", extensionsPage: "brave://extensions",
      profiles: [{ dir: "Default", path: "C:\\x", name: "Profile 1", lastUsed: now - 246 * 86400, size: 145 * MB, cache: 69 * MB, history: 192 * 1024, cookiesBytes: 300 * 1024, cookies: null, cookieSites: null, notes: [], extensions: [ext("uBlock Origin", "Ad blocker", true, [everywhere], 240)] }],
    },
  ];
  const startup = [
    { id: "run-user:RazerAppEngine", name: "Razer Synapse", entry: "RazerAppEngine", command: '"C:\\Program Files\\Razer\\RazerAppEngine\\RazerAppEngine.exe" --autostart', exe: "C:\\Program Files\\Razer\\RazerAppEngine\\RazerAppEngine.exe", location: "Run (you)", enabled: true, yours: true, running: true, mem: 509 * MB, cpu: 0.6, keep: null },
    { id: "run-user:Steam", name: "Steam", entry: "Steam", command: '"C:\\Program Files (x86)\\Steam\\steam.exe" -silent', exe: "C:\\Program Files (x86)\\Steam\\steam.exe", location: "Run (you)", enabled: true, yours: true, running: true, mem: 874 * MB, cpu: 0.3, keep: null },
    { id: "run-machine:BdagentApp", name: "Bitdefender Agent", entry: "BdagentApp", command: "bdagent.exe", exe: "C:\\Program Files\\Bitdefender\\bdagent.exe", location: "Run (all users)", enabled: true, yours: false, running: true, mem: 60 * MB, cpu: 0.1, keep: "Your antivirus. Keep it on." },
    { id: "run-user:slack", name: "Slack", entry: "com.squirrel.slack.slack", command: "slack.exe --process-start-args --startup", exe: "C:\\Users\\me\\AppData\\Local\\slack\\slack.exe", location: "Run (you)", enabled: true, yours: true, running: false, mem: 0, cpu: 0, keep: null },
    { id: "run-machine:RazerCortex", name: "Razer Cortex", entry: "RazerCortex", command: "RazerCortex.exe", exe: "C:\\Program Files (x86)\\Razer\\Razer Cortex\\RazerCortex.exe", location: "Run (all users)", enabled: true, yours: false, running: true, mem: 120 * MB, cpu: 0.2, keep: null },
    { id: "run-user:Discord", name: "Discord", entry: "Discord", command: "Update.exe --processStart Discord.exe", exe: null, location: "Run (you)", enabled: false, yours: true, running: false, mem: 0, cpu: 0, keep: null },
    { id: "run-user:EADM", name: "EA app", entry: "EADM", command: "EALauncher.exe", exe: "C:\\Program Files\\Electronic Arts\\EA Desktop\\EALauncher.exe", location: "Run (you)", enabled: false, yours: true, running: false, mem: 0, cpu: 0, keep: null },
  ];

  const room = {
    path: "C:\\Workspace\\storefront\\data\\field-notes",
    name: "field-notes",
    grade: "F",
    score: 59,
    line: "Let's clean this folder together c:",
    entries: 57,
    codeProject: false,
    findings: [
      { kind: "misspelled", title: "\"Docuements\" looks like a typo of \"Documents\"", detail: "Agents copy folder names exactly. A typo now means reports end up in two places later.", paths: [], bytes: 0 },
      { kind: "loose-reports", title: "2 report files lying around the top level", detail: "Reports, plans and logs belong in Documents/.", paths: [], bytes: 0 },
      { kind: "family", title: "8 look-alike folders: radio-night-show-*", detail: "Versions and variants of the same thing. One folder, radio-night-show/, keeps them together.", paths: [], bytes: 0 },
      { kind: "family", title: "7 look-alike folders: podcast-*", detail: "Dated work logs. They belong together in Documents/podcast/.", paths: [], bytes: 0 },
      { kind: "screenshot-pile", title: "40 screenshots piled up in garden/shots/", detail: "2 MB of captures. Delete them once they've done their job (video uploaded, bug fixed).", paths: [], bytes: 2 * MB },
      { kind: "screenshot-pile", title: "32 screenshots piled up in games/nimbus-drift/shots/runtime_qa_ee965b26_fixed_frames/", detail: "17 MB of captures. Delete them once they've done their job (video uploaded, bug fixed).", paths: [], bytes: 17 * MB },
      { kind: "crowded", title: "57 things at the top level", detail: "Past about 30, nobody can find anything. Group related folders.", paths: [], bytes: 0 },
    ],
    actions: [
      { id: "a1", op: "rename", targets: [], to: null, title: "Rename Docuements to Documents", reason: "Fix the folder name so everyone files reports in the same place.", bytes: 0, selected: false },
      { id: "a2", op: "move", targets: [], to: null, title: "Move 2 report files into Documents/", reason: "Reports go in one clearly named place.", bytes: 0, selected: true },
      { id: "a3", op: "move", targets: [], to: null, title: "Group 8 folders into radio-night-show/", reason: "One thing, one folder.", bytes: 0, selected: true },
      { id: "a4", op: "move", targets: [], to: null, title: "Group 7 folders into Documents/podcast/", reason: "One thing, one folder.", bytes: 0, selected: true },
      { id: "a5", op: "recycle", targets: [], to: null, title: "Recycle 40 screenshots in garden/shots/ (2 MB)", reason: "Screenshots are temporary. The Recycle Bin keeps them for a while, just in case.", bytes: 2 * MB, selected: true },
      { id: "a5b", op: "recycle", targets: [], to: null, title: "Recycle 19 screenshots in games/nimbus-drift/shots/runtime_qa_6a36ab75_frames/ (13 MB)", reason: "Screenshots are temporary. The Recycle Bin keeps them for a while, just in case.", bytes: 13 * MB, selected: true },
    ],
    scannedAt: now,
  };

  // What mom did, her Sunday letter and Autopilot's last run (made-up names).
  const day = 86400;
  const hist = (id, ago, kind, text, extra = {}) => ({ id, at: now - ago, kind, text, bytes: 0, items: 1, auto: false, undo: null, undoes: null, undone: false, action: null, inBin: false, ...extra });
  const history = {
    entries: [
      hist("h1", 40 * 60, "clean", "Cleared npm cache · 1.2 GB", { bytes: 1.2 * GB }),
      hist("h2", 3 * 3600, "copies", "Sent 14 files to the Recycle Bin · 640 MB", { bytes: 640 * MB, inBin: true }),
      hist("h3", day + 2 * 3600, "archive", "Moved old conversations to the archive · 3.1 GB", { bytes: 3.1 * GB, action: "Bring back" }),
      hist("h4", day + 5 * 3600, "tidy", "Tidied Downloads · 14 things", { action: "Undo" }),
      hist("h5", 3 * day, "clean", "Cleared old temp files · 2.3 GB", { bytes: 2.3 * GB, auto: true }),
      hist("h6", 4 * day, "stopped", "Turned off vite and 2 more · 1.4 GB of memory back", { action: "Start again" }),
      hist("h7", 5 * day, "setting", "Put back a Windows setting: Mouse pointer speed", { undone: true }),
    ],
    letter: {
      at: now - 2 * day,
      from: now - 9 * day,
      freed: 4.1 * GB,
      moved: 3.1 * GB,
      autoFreed: 2.3 * GB,
      done: 6,
      lines: ["This week I cleared 4.1 GB for you.", "3.1 GB went to your archive drive, safe and sound.", "Autopilot did 2.3 GB of that by itself.", "That's 6 things done together.", "Love, mom c:"],
      next: {
        title: "Your memory runs at 2666, but it's made for 3600",
        detail: "Switching on D.O.C.P. in the BIOS runs it at full speed. Test it afterwards.",
        fix: { label: "Copy for your agent", kind: "copy", value: "My PC's memory runs at 2666 MT/s, but its sticks (2 x Crucial BL16G36C16U4BL) are rated for 3600 MT/s.", hint: "Paste it to your agent." },
      },
    },
    letterUnread: true,
    autopilot: { at: now - 3 * day, freed: 2.3 * GB, items: 120, spots: ["Old temp files"] },
  };

  const settings = {
    config: {
      rooms: [room.path],
      leftovers: { forgottenAfterHours: 12, includeUnknownOrphans: true, autoStop: false, autoStopAfterMinutes: 20, autoStopOrphans: false },
      hooks: { enabled: true, nagProcesses: true, nagClutter: true, shareRules: true, stopOnSessionEnd: false, minAgeSecs: 5 },
      tidy: { reportsFolder: "Documents", minFamily: 3, screenshotPile: 10 },
      trash: { tempMinAgeDays: 7, agentScratchDays: 67, archiveDir: null, conversationsAfterDays: 67, keep: [], autopilot: false },
      ui: { alwaysOnTop: false, notifications: true, notifyLeftovers: true, notifyMeltdown: true, notifyCleanups: true, notifySkills: true, notifyLetter: true, notifyAfterMinutes: 10, notifyEveryMinutes: 30, scanIntervalSecs: 3, cpuAlertPercent: 25, checkUpdates: true },
      protect: [],
      kept: [],
    },
    autostart: false,
    version: "0.1.0",
    configPath: "C:\\Users\\me\\.momputer\\config.toml",
  };

  // Keys in a shell history (until "Blank them"), and work that's only on this PC.
  let secretsBlanked = false;
  const repoPrompt = (name, asks) => `My project \`${name}\` at \`C:\\Users\\me\\Workspace\\${name}\` has work that exists only on this PC.\n\nPlease ${asks}. Don't delete, reset, rebase or force-push anything, and show me each command before you run it.\n`;
  const repoCheck = (name, level, title, detail, items, asks) => ({
    id: `repo:c:\\users\\me\\workspace\\${name}`, area: "Backups", level, title, detail, items,
    fix: { label: "Copy for your agent", kind: "copy", value: repoPrompt(name, asks), hint: "Paste it to your agent." },
    more: [{ label: "Open folder", kind: "folder", value: `C:\\Users\\me\\Workspace\\${name}`, hint: null }],
  });
  const yourChecks = () => [
    ...(secretsBlanked
      ? [{ id: "secrets", area: "Security", level: "ok", title: "No keys in your shell history or the files git keeps", detail: "Mom looks for the shapes services give their keys.", fix: null }]
      : [{ id: "secrets:psreadline:ConsoleHost_history.txt", area: "Security", level: "warn", title: "Your PowerShell history has 2 lines with keys (OpenAI and GitHub)", detail: "Anything running as you can read that file. Blank them, and swap any key that still works for a new one.",
          fix: { label: "Blank them", kind: "secrets", value: "psreadline:ConsoleHost_history.txt", hint: "Mom takes each key out and keeps no copy. Close PowerShell windows first." } }]),
    repoCheck("storefront", "warn", "storefront has 3 commits that aren't pushed", "They're only on this PC until they are. Pushing keeps a copy.", [{ name: "main", note: "3 commits not pushed", led: "red" }, { name: "checkout-redesign", note: "Never pushed", led: "red" }], "push the branches with commits that aren't pushed"),
    repoCheck("copperline", "warn", "copperline is only on this PC", "It has no remote, so a failing drive would take it with it. A private remote keeps a copy.", [{ name: "No remote", note: "Last commit 2 days ago", led: "red" }], "help me make a private remote for it (GitHub, say) and push every branch to it"),
    repoCheck("atlas", "tip", "atlas has changes not committed for 12 days", "4 files with work in them, only on this PC.", [{ name: "Changes", note: "4 files not committed, last touched 12 days ago", led: "off" }], "go through the changes that aren't committed with me, and commit the ones worth keeping"),
  ];

  // How the agents are set up: made-up servers, hooks and files, with a few things worth a look.
  const askAgent = (text) => ({ label: "Copy for your agent", text });
  const agentSetups = [
    {
      id: "claude", name: "Claude Code", problems: 3, checked: null, canCheck: true,
      weight: {
        everywhere: 6640, most: 5030, mostIn: "storefront", servers: 4,
        parts: [
          { name: "List of 4 skills", tokens: 3880, scope: "every chat", doc: null },
          { name: "storefront: memory index", tokens: 4470, scope: "in storefront", doc: "d-memory" },
          { name: "Your CLAUDE.md", tokens: 780, scope: "every chat", doc: "d-memory" },
          { name: "storefront: CLAUDE.md", tokens: 560, scope: "in storefront", doc: null },
          { name: "style.md (imported)", tokens: 1980, scope: "every chat", doc: null },
        ].sort((a, b) => b.tokens - a.tokens),
        tips: [
          { title: "storefront: memory index is about 4.5k tokens", detail: "Read at the start of every chat in storefront.", level: "note", doc: "d-memory",
            fix: askAgent("My Claude Code memory index `C:\\Users\\me\\.claude\\projects\\C--Users-me-Workspace-storefront\\memory\\MEMORY.md` is read at the start of every chat in storefront, about 4.5k tokens. Help me shorten it: one short line per memory, merge the ones that say the same thing, and drop the ones that aren't true any more. Show me the new index before you change anything.") },
        ],
      },
      settings: [
        { title: "Runs everything without asking", detail: "permissions.defaultMode is bypassPermissions: commands, edits and downloads all go ahead.", level: "warn", fix: null, doc: "d-set" },
        { title: "Model", detail: "claude-opus-5-5", level: "ok", fix: null, doc: "d-set" },
        { title: "Holds a key in plain text", detail: "In settings.json, under env. Anything running as you can read it.", level: "note", fix: null, doc: "d-set" },
        { title: "Plugins", detail: "4, 3 switched on.", level: "ok", fix: null, doc: "d-set" },
      ],
      servers: [
        { name: "notes", kind: "program", what: "node C:\\Users\\me\\tools\\notes-server.js", from: "your settings", level: "unknown", problem: null, fix: null, doc: "d-mcp" },
        { name: "copperline", kind: "web", what: "https://mcp.copperline.example/v1", from: "your settings", level: "unknown", problem: null, fix: null, doc: "d-mcp" },
        { name: "old-search", kind: "program", what: "search-server.exe --index docs", from: "your settings", level: "warn", problem: "It starts search-server.exe, and that file isn't there any more.", fix: askAgent("My Claude Code MCP server `old-search` (set up in `C:\\Users\\me\\.claude.json`) may not start. It starts `search-server.exe`, and that file isn't there any more. Find out whether it still works, and either fix it or tell me how to take the server out. Show me any change before you make it."), doc: "d-mcp" },
        { name: "atlas", kind: "web", what: "https://mcp.atlas.example/mcp", from: "plugin atlas", level: "warn", problem: "It's waiting for you to sign in. Type /mcp in Claude Code and pick it.", fix: { label: "Copy /mcp", text: "/mcp" }, doc: "d-atlas" },
        { name: "shop", kind: "program", what: "nimbus-shop-mcp --token •••", from: "project storefront", level: "note", problem: "Mom can't find nimbus-shop-mcp on this PC's PATH. If it works in your agent, that's fine.", fix: null, doc: "d-mcp" },
        { name: "lanternfall", kind: "web", what: "https://mcp.lanternfall.example/sse", from: "your settings", level: "off", problem: "Switched off.", fix: null, doc: "d-mcp" },
      ],
      hooks: [
        { event: "Stop", matcher: "", what: "mom.exe hook claude stop", from: "your settings", mine: true, count: 1, level: "ok", problem: null, fix: null, doc: "d-set" },
        { event: "SessionStart", matcher: "", what: "mom.exe hook claude session-start", from: "your settings", mine: true, count: 1, level: "ok", problem: null, fix: null, doc: "d-set" },
        { event: "PostToolUse", matcher: "Bash", what: "bash hooks/guard.sh hooks/remind.py", from: "plugin guard", mine: false, count: 7, level: "ok", problem: null, fix: null, doc: "d-guard" },
        { event: "PreToolUse", matcher: "Edit|Write", what: "node scripts/lint.js", from: "plugin atlas", mine: false, count: 1, level: "ok", problem: null, fix: null, doc: "d-atlas-hooks" },
      ],
      skills: [
        { name: "tidy-folders", what: "Surveys a Windows PC's personal folders read-only", from: "plugin momputer-skills", doc: "d-skill-tidy" },
        { name: "inbox-cleanup", what: "Reviews the last two months of Gmail", from: "plugin momputer-skills", doc: "d-skill-inbox" },
        { name: "atlas-maps", what: "Draws maps of a codebase", from: "plugin atlas", doc: "d-skill-maps" },
        { name: "release-notes", what: "Writes release notes from the commits since the last tag", from: "yours", doc: "d-skill-notes" },
      ],
      docs: [
        { id: "d-set", title: "settings.json", kind: "json", path: "C:\\Users\\me\\.claude\\settings.json", bytes: 2140 },
        { id: "d-memory", title: "CLAUDE.md", kind: "markdown", path: "C:\\Users\\me\\.claude\\CLAUDE.md", bytes: 3120 },
        { id: "d-mcp", title: "MCP servers", kind: "json", path: "C:\\Users\\me\\.claude.json", bytes: 48200 },
        { id: "d-atlas", title: "atlas: MCP servers", kind: "json", path: "C:\\Users\\me\\.claude\\plugins\\cache\\shop\\atlas\\1.2.0\\.mcp.json", bytes: 180 },
        { id: "d-atlas-hooks", title: "atlas: hooks", kind: "json", path: "C:\\Users\\me\\.claude\\plugins\\cache\\shop\\atlas\\1.2.0\\hooks\\hooks.json", bytes: 420 },
        { id: "d-guard", title: "guard: hooks", kind: "json", path: "C:\\Users\\me\\.claude\\plugins\\cache\\shop\\guard\\2.0.1\\hooks\\hooks.json", bytes: 2400 },
        { id: "d-skill-tidy", title: "tidy-folders", kind: "markdown", path: "C:\\Users\\me\\.claude\\plugins\\cache\\momputer\\momputer-skills\\6dd3d270c6cd\\tidy-folders\\SKILL.md", bytes: 9100 },
        { id: "d-skill-inbox", title: "inbox-cleanup", kind: "markdown", path: "C:\\Users\\me\\.claude\\plugins\\cache\\momputer\\momputer-skills\\6dd3d270c6cd\\inbox-cleanup\\SKILL.md", bytes: 8700 },
        { id: "d-skill-maps", title: "atlas-maps", kind: "markdown", path: "C:\\Users\\me\\.claude\\plugins\\cache\\shop\\atlas\\1.2.0\\skills\\atlas-maps\\SKILL.md", bytes: 2100 },
        { id: "d-skill-notes", title: "release-notes", kind: "markdown", path: "C:\\Users\\me\\.claude\\skills\\release-notes\\SKILL.md", bytes: 1900 },
        { id: "d-agent", title: "reviewer (subagent)", kind: "markdown", path: "C:\\Users\\me\\.claude\\agents\\reviewer.md", bytes: 900 },
      ],
    },
    {
      id: "codex", name: "Codex", problems: 0, checked: null, canCheck: false,
      settings: [
        { title: "Asks, and keeps commands in a sandbox", detail: "The usual way.", level: "ok", fix: null, doc: "d-codex" },
        { title: "Model", detail: "gpt-5.5", level: "ok", fix: null, doc: "d-codex" },
        { title: "Trusted folders", detail: "6, where it works without asking as much.", level: "ok", fix: null, doc: "d-codex" },
      ],
      servers: [{ name: "docs", kind: "web", what: "https://docs.copperline.example/mcp", from: "your settings", level: "unknown", problem: null, fix: null, doc: "d-codex" }],
      hooks: [{ event: "Stop", matcher: "", what: "mom.exe hook codex stop", from: "your settings", mine: true, count: 1, level: "ok", problem: null, fix: null, doc: "d-codex-hooks" }],
      skills: [{ name: "find-skills", what: "Helps find and install agent skills", from: "shared with other agents", doc: "d-skill-find" }],
      docs: [
        { id: "d-codex", title: "config.toml", kind: "toml", path: "C:\\Users\\me\\.codex\\config.toml", bytes: 1800 },
        { id: "d-codex-hooks", title: "hooks.json", kind: "json", path: "C:\\Users\\me\\.codex\\hooks.json", bytes: 600 },
        { id: "d-skill-find", title: "find-skills", kind: "markdown", path: "C:\\Users\\me\\.agents\\skills\\find-skills\\SKILL.md", bytes: 1500 },
      ],
    },
    {
      id: "cursor", name: "Cursor", problems: 0, checked: null, canCheck: false, settings: [], hooks: [], skills: [],
      servers: [{ name: "motion", kind: "program", what: "npx -y motion-mcp", from: "your settings", level: "unknown", problem: null, fix: null, doc: "d-cursor" }],
      docs: [{ id: "d-cursor", title: "mcp.json", kind: "json", path: "C:\\Users\\me\\.cursor\\mcp.json", bytes: 300 }],
    },
  ];
  const agentDocs = {
    markdown: {
      text: "---\nname: tidy-folders\ndescription: Surveys a Windows PC's personal folders read-only, finds papers lying loose, games outside a Games folder, duplicates and old installers, and builds a tidy plan page the user ticks item by item. Use when the user asks to tidy, organize or declutter their folders.\nlicense: Apache-2.0 (complete terms in LICENSE.txt)\ncompatibility: Windows 10 or 11 with Python 3.8+ (standard library only).\nmetadata:\n  version: \"1.0.0\"\n---\n\n# Tidy folders\n\nFind what's out of place in the user's folders, show it on one page they can tick through, then move only what they approved, with an undo.\n\n## Hard rules\n\n1. **Look first.** The survey reads; it never moves, renames or deletes.\n2. **Nothing moves without a tick.** The plan page is the only way in: what isn't ticked stays where it is.\n3. **Deletions go to the Recycle Bin.** Never `del`, never `rm`.\n4. Every step is logged, and `revert.ps1` puts it back.\n\n## The flow\n\n| Step | What happens | Changes anything? |\n| --- | --- | --- |\n| Survey | `python scripts/survey.py` lists what's out of place | No |\n| Plan | One page, grouped, with charts | No |\n| Apply | Moves what was ticked, writes the log | Yes |\n\n```\npython scripts/survey.py --out plan.json\npython scripts/safe_apply.py plan.json --approved approved.json\n```\n\n> If anything looks like keys, wallets or passports, say so and leave it alone.\n\nMore in [the reference](references/rules.md).\n",
    },
    json: { text: '{\n  "mcpServers": {\n    "notes": {\n      "type": "stdio",\n      "command": "node",\n      "args": ["C:\\\\Users\\\\me\\\\tools\\\\notes-server.js"],\n      "env": {\n        "NOTES_TOKEN": "•••"\n      }\n    },\n    "copperline": {\n      "type": "http",\n      "url": "https://mcp.copperline.example/v1",\n      "headers": {\n        "Authorization": "•••"\n      }\n    }\n  }\n}', hidden: 2 },
    toml: { text: 'model = "gpt-5.5"\napproval_policy = "on-request"\nsandbox_mode = "workspace-write"\n\n[mcp_servers.docs]\nurl = "https://docs.copperline.example/mcp"\n\n[mcp_servers.docs.env_http_headers]\n"X-Api-Key" = "•••"\n', hidden: 1 },
    "d-memory": { text: "# How I like things done\n\n- Reports go in `Documents/`.\n- Ask before installing anything.\n- Stop what you start.\n" },
  };

  const rules = {
    skills: [{"name": "mom", "title": "Mom", "what": "Teaches your agent to clean up after itself.", "more": "Before it says it's done, it stops what it started, bins its own temp files and puts reports where they belong. It loads by itself; you don't have to ask.", "prompts": ["Clean up after yourself", "What did you leave running?"], "has": ["Claude Code", "Codex"], "page": "https://github.com/Thisisily/momputer/tree/main/skills/mom", "version": "1.0.0", "copies": [{"who": "Claude Code", "how": "plugin", "version": "1.0.0", "path": "C:\\Users\\me\\.claude\\plugins\\cache\\momputer\\momputer-skills\\6dd3d270c6cd\\mom", "behind": false, "update": null}, {"who": "Codex", "how": "folder", "version": "", "path": "C:\\Users\\me\\.agents\\skills\\mom", "behind": true, "update": null}], "behind": true}, {"name": "tidy-folders", "title": "Tidy folders", "what": "Puts loose papers, old installers and stray games where they belong.", "more": "It looks first, shows one page of what it found, and moves only what you tick. Every move is logged and can be undone.", "prompts": ["Tidy up my Downloads", "Where are my tax papers?", "Are all my games in the Games folder?"], "has": ["Claude Code"], "page": "https://github.com/Thisisily/momputer/tree/main/skills/tidy-folders", "version": "1.0.0", "copies": [{"who": "Claude Code", "how": "plugin", "version": "0.9.0", "path": "C:\\Users\\me\\.claude\\plugins\\cache\\momputer\\momputer-skills\\6dd3d270c6cd\\tidy-folders", "behind": true, "update": "claude plugin update momputer-skills@momputer"}], "behind": true}, {"name": "desktop-icons", "title": "Desktop icons", "what": "Clears dead shortcuts, duplicates and loose files off your desktop.", "more": "It reads shortcuts without ever running one. Shortcuts go to the Recycle Bin, files move into folders, and there's an undo.", "prompts": ["Clean up my desktop", "Which of my desktop shortcuts are broken?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/desktop-icons", "version": "1.0.0", "copies": [], "behind": false}, {"name": "rename-documents", "title": "Rename documents", "what": "Gives scans and downloads names you can find later.", "more": "scan0001.pdf becomes 2024-03-14 Lanternfall invoice 1182.pdf. You see old and new side by side and tick the ones you want; there's an undo.", "prompts": ["Rename my scans so I can find them", "Give the PDFs in Downloads proper names"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/rename-documents", "version": "1.0.0", "copies": [], "behind": false}, {"name": "disk-space-report", "title": "Disk space report", "what": "Shows where your disk space went and the safe way to get it back.", "more": "A page with what fills each drive and what each part is. It deletes nothing itself.", "prompts": ["Where did my disk space go?", "Why is C: full?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/disk-space-report", "version": "1.0.0", "copies": [], "behind": false}, {"name": "backup-checkup", "title": "Backup checkup", "what": "Tells you what you'd lose if this disk died today.", "more": "Which folders and code have a second copy somewhere, and which don't. It only looks.", "prompts": ["Are my files backed up?", "What would I lose if this disk died?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/backup-checkup", "version": "1.0.0", "copies": [], "behind": false}, {"name": "inbox-cleanup", "title": "Inbox cleanup", "what": "Reviews two months of Gmail and shows what to unsubscribe from.", "more": "Who fills your inbox, what you never open, likely phishing, and a plan to declutter. It reads only; nothing is deleted or sent.", "prompts": ["Clean up my Gmail", "What should I unsubscribe from?"], "has": ["Claude Code"], "page": "https://github.com/Thisisily/momputer/tree/main/skills/inbox-cleanup", "version": "1.0.0", "copies": [{"who": "Claude Code", "how": "plugin", "version": "1.0.0", "path": "C:\\Users\\me\\.claude\\plugins\\cache\\momputer\\momputer-skills\\6dd3d270c6cd\\inbox-cleanup", "behind": false, "update": null}], "behind": false}, {"name": "calendar-fixer", "title": "Calendar fixer", "what": "Finds double bookings, days with no breaks and stale recurring meetings.", "more": "It reviews the next four weeks of Google Calendar and fixes only what you tick.", "prompts": ["Fix my calendar", "Where am I double booked?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/calendar-fixer", "version": "1.0.0", "copies": [], "behind": false}, {"name": "drive-audit", "title": "Drive audit", "what": "Shows what you share with anyone on Google Drive, and what fills it.", "more": "Public links, the biggest files, duplicates, and files nobody touched in a year. It only looks.", "prompts": ["Audit my Google Drive", "What am I sharing publicly?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/drive-audit", "version": "1.0.0", "copies": [], "behind": false}, {"name": "subscription-audit", "title": "Subscription audit", "what": "Finds what you pay for every month and what it costs a year.", "more": "From Gmail receipts and bank statements you export: price rises, things paid twice, trials about to charge. It never cancels or pays; you do the cancelling.", "prompts": ["What am I subscribed to?", "Which free trials are about to charge me?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/subscription-audit", "version": "1.0.0", "copies": [], "behind": false}, {"name": "bookmark-cleanup", "title": "Bookmark cleanup", "what": "Finds duplicate, dead and never-opened bookmarks.", "more": "It reads a copy of your bookmarks and writes a tidy file for you to import. Your browser's own files are never touched.", "prompts": ["Clean up my bookmarks", "Which of my bookmarks are dead?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/bookmark-cleanup", "version": "1.0.0", "copies": [], "behind": false}],
    skillsInstall: "/plugin install momputer-skills --marketplace Thisisily/momputer",
    skillsPage: "https://github.com/Thisisily/momputer/tree/main/skills",
    skillsUpdate: ["claude plugin marketplace update momputer", "claude plugin update momputer-skills@momputer"],
    skillsCatchUp: "These Agent Skills on my PC are older copies. For each one, download the newest folder from the GitHub page next to it and replace the old folder with it, keeping the folder's name. Don't change anything else, and tell me what you replaced.\n\n- mom in `C:\\Users\\me\\.agents\\skills\\mom` (no version; the newest is 1.0.0): https://github.com/Thisisily/momputer/tree/main/skills/mom\n",
    text: "# Mom's house rules\n\nYou share this computer with a human and a lot of other agents. Clean up after yourself.\n\n1. **Stop what you start.** Before you finish, stop every dev server, watcher, headless browser, `tail -f`, container and VM you started, unless the user asked you to keep it running. If you leave something running on purpose, say what it is, where it lives (URL or port) and how to stop it.\n2. **Don't start a second copy.** Check whether it's already running (the port is taken, `mom ps`) before you start a server again.\n3. **Temp files are temporary.** Keep scratch work in one temp folder and delete it when you're done. Delete screenshots, renders and intermediate files once they've done their job (the video is uploaded and backed up, the bug is fixed).\n4. **Reports go in `Documents/`.** Put reports, plans, summaries and logs in a `Documents/` folder in the project, with names that say what they are. Never scatter dated folders or report files across the project root.\n5. **One thing, one folder.** Keep related work together in one clearly named folder. No sibling sprawl like `thing-v2`, `thing-v3`, `thing-trial`, `thing-final`: version inside the folder, or replace the old one.\n6. **Leave no empty folders.** If you move or delete files, remove the folders you emptied.\n7. **Label everything.** Names say what something is, not just when it was made.\n8. **If cleanup is blocked, say so.** Tell the user exactly what's left and give them a one-line command to finish it.\n",
    customized: false,
    targets: [
      { agent: "claude-code", label: "Claude Code", path: "C:\\Users\\me\\.claude\\CLAUDE.md", available: true, installed: false, upToDate: false },
      { agent: "codex", label: "Codex", path: "C:\\Users\\me\\.codex\\AGENTS.md", available: true, installed: false, upToDate: false },
      { agent: "gemini", label: "Gemini CLI", path: "C:\\Users\\me\\.gemini\\GEMINI.md", available: false, installed: false, upToDate: false },
    ],
    integrations: [
      { id: "claude-hooks", label: "Claude Code check-ins", detail: "Hooks: house rules at session start, a reminder about leftovers before Claude finishes.", available: true, installed: false, path: "" },
      { id: "codex-hooks", label: "Codex check-ins", detail: "Same hooks for Codex. Codex asks you to trust them once (/hooks in the CLI).", available: true, installed: false, path: "" },
      { id: "claude-skill", label: "mom skill for Claude", detail: "Teaches Claude how to clean up after itself and use the mom command.", available: true, installed: false, path: "" },
      { id: "codex-skill", label: "mom skill for Codex", detail: "The same skill, in ~/.agents/skills where Codex looks.", available: true, installed: false, path: "" },
      { id: "agent-waiting", label: "Tell me when an agent is waiting", detail: "A note when Claude or Codex stops or needs you. Mom adds a small hook to them.", available: true, installed: false, path: "" },
    ],
  };

  const emit = (name, payload) => (listeners[name] || []).forEach((f) => f({ payload }));
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));

  function stop(ids) {
    const gone = snapshot.groups.filter((x) => ids.includes(x.id));
    snapshot.groups = snapshot.groups.filter((x) => !ids.includes(x.id));
    const left2 = snapshot.groups.filter((x) => x.status === "left-behind" || x.status === "orphaned");
    snapshot.summary.leftovers = left2.length;
    snapshot.summary.leftoverMem = left2.reduce((a, x) => a + x.mem, 0);
    snapshot.summary.leftoverCpu = left2.reduce((a, x) => a + x.cpu, 0);
    snapshot.mood = left2.length
      ? { face: "hmm", level: 1, line: "Somebody left the lights on." }
      : { face: "proud", level: 0, line: "All tidy. I'm proud of you, sweetie." };
    const freed = gone.reduce((a, x) => a + x.mem, 0);
    setTimeout(() => emit("snapshot", JSON.parse(JSON.stringify(snapshot))), 600);
    return { stopped: [], alreadyGone: [], failed: [], freedMem: freed, groups: gone.length, message: gone.length === 1 ? "Turned it off. That's memory back for you." : `Turned off ${gone.length} things. That's memory back for you.` };
  }

  // Homework: what each agent session did this week (made-up projects).
  const homework = [
    {
      agent: "Codex", id: "0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f", cwd: "C:\\Workspace\\storefront", project: "storefront", title: "Ship the launch video",
      started: now - 3 * H, ended: now - 40, commands: 64,
      changed: ["C:\\Workspace\\storefront\\src\\hero.tsx", "C:\\Workspace\\storefront\\src\\video\\cuts.ts", "C:\\Workspace\\storefront\\README.md"], changedCount: 12,
      deleted: [{ path: "C:\\Workspace\\storefront\\renders\\draft-3.mp4", outside: false }], deletedCount: 1,
      installs: ["npm i -D vitest"], pushes: [{ command: "git push origin main", force: false }],
      lastCheck: { command: "npm test", passed: false, at: now - 120 }, flags: ["Said tests pass; the last test run failed"], partial: false,
      link: "codex://threads/0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
    },
    {
      agent: "Claude Code", id: "0191f20b", cwd: "C:\\Workspace\\atlas", project: "atlas", title: "Tidy the Atlas map tiles",
      started: now - 26 * H, ended: now - 22 * H, commands: 31,
      changed: ["C:\\Workspace\\atlas\\tiles\\index.json", "C:\\Workspace\\atlas\\scripts\\cut.py"], changedCount: 6,
      deleted: [{ path: "C:\\Users\\me\\Documents\\atlas-old", outside: true }, { path: "tiles\\cache", outside: false }], deletedCount: 2,
      installs: [], pushes: [{ command: "git push --force origin tiles", force: true }],
      lastCheck: { command: "cargo test", passed: true, at: now - 22 * H }, flags: ["Deleted something outside its project", "Force-pushed"], partial: false, link: null,
    },
    {
      agent: "Claude Code", id: "5f2a91c0", cwd: "C:\\Workspace\\nimbus", project: "nimbus", title: "Nimbus onboarding copy",
      started: now - 74 * H, ended: now - 72 * H, commands: 9,
      changed: ["C:\\Workspace\\nimbus\\copy\\welcome.md", "C:\\Workspace\\nimbus\\copy\\steps.md", "C:\\Workspace\\nimbus\\copy\\done.md"], changedCount: 3,
      deleted: [], deletedCount: 0, installs: [], pushes: [],
      lastCheck: { command: "pnpm build", passed: true, at: now - 72 * H }, flags: [], partial: false, link: null,
    },
  ];
  // "?waiting" shows an agent waiting for you (off by default so the pictures don't change).
  const waiting = [{ agent: "claude", session: "0191f20b", cwd: "C:\\Workspace\\storefront", project: "storefront", kind: "needs-you", at: now - 12 * 60, transcript: null, told: true }];

  const handlers = {
    get_snapshot: () => JSON.parse(JSON.stringify(snapshot)),
    refresh: () => null,
    stop_groups: async ({ ids }) => (await wait(500), stop(ids)),
    get_stopped: async () => (await wait(80), JSON.parse(JSON.stringify(stopped))),
    start_again: async ({ id }) => {
      await wait(400);
      const x = stopped.find((s) => s.id === id);
      stopped = stopped.filter((s) => s.id !== id);
      return `Started ${x ? x.name : "it"} again, in a new window.`;
    },
    forget_stopped: async ({ id }) => ((stopped = stopped.filter((s) => s.id !== id)), null),
    stop_leftovers: async () => (await wait(700), stop(snapshot.groups.filter((x) => x.status === "left-behind" || x.status === "orphaned").map((x) => x.id))),
    keep_group: () => null,
    get_agents: async () => (await wait(500), JSON.parse(JSON.stringify(agentSetups))),
    // "Check now": what Claude Code would say after trying each of its servers.
    check_agents: async () => {
      await wait(2600);
      const a = agentSetups[0];
      a.checked = Date.now() / 1000;
      const said = { notes: "ok", copperline: "auth", shop: "ok" };
      for (const s of a.servers) {
        if (said[s.name] === "ok") Object.assign(s, { level: "ok", problem: null, fix: null });
        if (said[s.name] === "auth") Object.assign(s, { level: "warn", problem: "It's waiting for you to sign in. Type /mcp in Claude Code and pick it.", fix: { label: "Copy /mcp", text: "/mcp" } });
      }
      a.problems = a.servers.filter((s) => s.level === "warn").length + a.settings.filter((f) => f.level === "warn").length;
      return JSON.parse(JSON.stringify(agentSetups));
    },
    read_doc: async ({ id }) => {
      await wait(250);
      const doc = agentSetups.flatMap((a) => a.docs).find((d) => d.id === id);
      if (!doc) throw "That file isn't one of your agents' any more. Look again.";
      const made = agentDocs[id] || agentDocs[doc.kind];
      return { id, title: doc.title, kind: doc.kind, path: doc.path, text: made.text, hidden: made.hidden || 0, cut: false };
    },
    get_trash: () => ({ spots: JSON.parse(JSON.stringify(spots)), busy: false }),
    measure_trash: async () => {
      const list = [];
      for (const s of spots) {
        await wait(250);
        list.push(s);
        emit("trash", { spots: JSON.parse(JSON.stringify(list)), busy: true });
      }
      emit("trash", { spots: JSON.parse(JSON.stringify(list)), busy: false });
    },
    clean_trash: async ({ id, pick }) => {
      const s = spots.find((x) => x.id === id);
      const pk = { goes: true, new: false, ticked: [], unticked: [], ...(pick || {}) };
      if (pk.discard && (id !== "forgotten-apps" || pk.goes || pk.new)) throw "Choose exact app data folders to remove.";
      const action = pk.discard ? "delete" : s.action;
      // Like mom-core's choose(), for the listed items (the mock has nothing unlisted).
      const chosen = s.items.filter((i) => {
        if ((!i.goes && !i.optional) || i.kept || (pk.keep || []).includes(i.path)) return false;
        return (i.goes ? pk.goes : pk.new) ? !pk.unticked.includes(i.path) : pk.ticked.includes(i.path);
      });
      live = { done: 0, total: chosen.length, freed: 0, files: 0, totalFiles: chosen.reduce((a, i) => a + (i.files || 1), 0), current: "", gone: [], goneCount: 0, stopping: false };
      liveStop = false;
      const goes = [];
      for (const i of chosen) {
        if (liveStop) break;
        live.current = i.name;
        await wait(Math.min(900, 160 + (i.files || 1) / 4));
        goes.push(i);
        live.done++;
        live.freed += i.bytes;
        live.files += i.files || 1;
        live.gone.push(i.path);
      }
      const sum = (list) => list.reduce((a, i) => a + i.bytes, 0);
      const freed = sum(goes);
      s.items = s.items.filter((i) => !goes.includes(i));
      s.totalBytes -= freed;
      s.reclaimableBytes = sum(s.items.filter((i) => i.goes));
      s.keptBytes = sum(s.items.filter((i) => !i.goes));
      s.reclaimableItems = s.items.filter((i) => i.goes).length;
      s.keptItems = s.items.filter((i) => !i.goes).length;
      s.newItems = s.items.filter((i) => !i.goes && i.optional).length;
      s.newBytes = sum(s.items.filter((i) => !i.goes && i.optional));
      if (!s.reclaimableItems && s.action !== "tool") s.effect = "Mom has not selected anything here. Items may be recent, in use, kept, or have unknown ages.";
      emit("trash", { spots: JSON.parse(JSON.stringify(spots)), busy: false });
      const size = `${(freed / 1024 / MB).toFixed(1)} GB`;
      if (action === "move") {
        for (const i of goes) archive.moved.push({ from: i.path, to: `${archive.dir}\\${i.name}`, archive: archive.dir, name: i.name, spot: s.id, bytes: i.bytes, files: i.files, movedAt: Date.now() / 1000, linked: true, reachable: true });
      }
      const message =
        action === "recycle" || (action === "delete" && pk.recycle)
          ? `Moved ${goes.length} files (${size}) to the Recycle Bin. Changed your mind? It's all in there.`
          : action === "move"
            ? `Moved ${goes.length} folders (${size}) to ${archive.dir}. They still open from where they were.`
            : `Took out ${size} of trash.`;
      return { id, freed, deleted: goes.length, skipped: 0, message };
    },
    admin_check: async () => (await wait(60), null),
    app_version: () => "0.5.2",
    // "?update" in the address shows what a newer version looks like.
    get_update: async () => (await wait(300), { current: "0.5.2", latest: location.search.includes("update") ? "0.5.3" : "0.5.2", newer: location.search.includes("update"), checked: now, asked: now, problem: null, getIt: "https://thisisily.github.io/momputer/" }),
    measure_spot: async ({ id, more }) => {
      await wait(700);
      const s = spots.find((x) => x.id === id);
      if (more && s) {
        // "Show all": the smaller ones that weren't listed, each a little smaller than the last.
        const goes = s.items.filter((i) => i.goes);
        const missing = Math.min((s.reclaimableItems || 0) - goes.length, goes.length * 3 || 40, 400);
        const last = goes[goes.length - 1] || { bytes: 4 * MB, reason: "Changed 30 days ago", ageSecs: 30 * 86400, ageKnown: true };
        const extra = [];
        for (let n = 1; n <= missing; n++) {
          const name = `tmp-${(n * 7919).toString(16).padStart(4, "0")}`;
          extra.push({ name, sub: `${1 + (n % 9)} ${n % 9 ? "files" : "file"}`, path: `${s.path || "C:\\Users\\me\\AppData\\Local\\Temp"}\\${name}`, bytes: Math.max(1024, Math.round(last.bytes / (1 + n / 6))), files: 1 + (n % 9), ageSecs: last.ageSecs, ageKnown: last.ageKnown, goes: true, optional: false, reason: last.reason });
        }
        s.items = [...goes, ...extra, ...s.items.filter((i) => !i.goes)];
      }
      emit("trash", { spots: JSON.parse(JSON.stringify(spots)), busy: false });
      return id;
    },
    // Keeping and letting go, the way mom-core does it: what's kept stays, with no box to tick,
    // and leads the list of what stays.
    keep_item: async ({ path, keep }) => {
      await wait(350);
      const list = (settings.config.trash.keep || []).filter((k) => k !== path);
      settings.config.trash.keep = keep ? [...list, path] : list;
      const sum = (items) => items.reduce((a, i) => a + i.bytes, 0);
      for (const s of spots) {
        const i = s.items.find((x) => x.path === path);
        if (!i) continue;
        const unlisted = Math.max(0, (s.reclaimableItems || 0) - s.items.filter((x) => x.goes).length);
        if (keep) {
          if (!i.was) i.was = { goes: i.goes, optional: !!i.optional, reason: i.reason };
          Object.assign(i, { goes: false, optional: false, kept: true, reason: "You said to keep this" });
        } else if (i.was) {
          Object.assign(i, i.was, { kept: false });
          delete i.was;
        }
        const goes = s.items.filter((x) => x.goes);
        const stays = s.items.filter((x) => !x.goes).sort((a, b) => !!b.kept - !!a.kept);
        s.items = [...goes, ...stays];
        if (s.exact !== false && !unlisted) {
          s.reclaimableBytes = sum(goes);
          s.keptBytes = sum(stays);
        }
        s.reclaimableItems = goes.length + unlisted;
        s.keptItems = Math.max(s.keptItems || 0, stays.length) + (keep && i.was && i.was.goes ? 0 : 0);
        s.newItems = Math.max(0, (s.newItems || 0) + (i.was && i.was.optional ? (keep ? -1 : 0) : 0));
      }
      emit("trash", { spots: JSON.parse(JSON.stringify(spots)), busy: false });
      return null;
    },
    keep_nothing: async () => {
      await wait(350);
      settings.config.trash.keep = [];
      for (const s of spots)
        for (const i of s.items)
          if (i.was) {
            Object.assign(i, i.was, { kept: false });
            delete i.was;
          }
      emit("trash", { spots: JSON.parse(JSON.stringify(spots)), busy: false });
      return null;
    },
    clean_progress: ({ since }) => ({ ...live, gone: live.gone.slice(since), goneCount: live.gone.length, stopping: liveStop }),
    stop_clean: () => {
      liveStop = true;
      return null;
    },
    files_state: async () => (await wait(80), { report: filesReport, busy: false }),
    scan_files: async () => {
      for (let n = 1; n <= 8; n++) {
        await wait(250);
        emit("files-progress", { files: n * 1040, bytes: n * 3.6 * GB, stage: n < 6 ? "listing" : "comparing" });
      }
      filesReport = filesReport || markKept(mockFiles());
      filesReport.measuredAt = Math.floor(Date.now() / 1000);
      emit("files", { report: JSON.parse(JSON.stringify(filesReport)), busy: false });
    },
    stop_files_scan: () => null,
    recycle_files: async ({ paths }) => {
      await wait(500);
      let freed = 0;
      for (const g of filesReport.groups) {
        for (const f of g.files) if (paths.includes(f.path)) freed += g.size;
        g.files = g.files.filter((f) => !paths.includes(f.path));
      }
      filesReport.groups = filesReport.groups.filter((g) => g.files.length > 1);
      for (const b of filesReport.big) if (paths.includes(b.path)) freed += b.size;
      filesReport.big = filesReport.big.filter((b) => !paths.includes(b.path));
      filesReport.wasted = filesReport.groups.reduce((a, g) => a + g.size * (g.files.length - 1), 0);
      return { recycled: paths, skipped: [], freed, message: `Moved ${paths.length} files (${(freed / GB).toFixed(1)} GB) to the Recycle Bin.` };
    },
    get_winsettings: async () => (await wait(300), JSON.parse(JSON.stringify(winsettings))),
    fix_setting: async ({ id }) => {
      await wait(600);
      const f = winsettings.find((x) => x.id === id);
      winsettings = winsettings.filter((x) => x.id !== id);
      return `Put back: ${f.title}. ${f.fix.after || ""}`.trim();
    },
    undo_setting: async () => (await wait(300), "Put back the way it was."),
    open_program: () => null,
    archive_info: async () => (await wait(80), JSON.parse(JSON.stringify(archive))),
    choose_archive_dir: async () => {
      await wait(200);
      archive.dir = "F:\\momputer-archive";
      archive.reachable = true;
      archive.free = 398 * GB;
      return JSON.parse(JSON.stringify(archive));
    },
    bring_back: async ({ from }) => {
      await wait(900);
      const m = archive.moved.find((x) => x.from === from);
      archive.moved = archive.moved.filter((x) => x.from !== from);
      return `${m.name} is back where it was.`;
    },
    delete_moved: async ({ from }) => {
      await wait(500);
      const m = archive.moved.find((x) => x.from === from);
      archive.moved = archive.moved.filter((x) => x.from !== from);
      return `Deleted ${m.name} for good.`;
    },
    uninstall_app: async () => (await wait(200), "Atlas Recorder's uninstaller is open. Follow it to the end."),
    get_health: async () => (await wait(150), JSON.parse(JSON.stringify(health))),
    get_system: async () => (
      await wait(900),
      {
        at: Math.floor(Date.now() / 1000),
        pc: [
          ["Processor", "AMD Ryzen 7 3800X 8-Core Processor, 8 cores, 16 threads"],
          ["Memory", "64 GB DDR4: 4 × 16 GB at 2666 MT/s"],
          ["Graphics", "NVIDIA GeForce RTX 2070 SUPER, driver from 2026, 41 °C"],
          ["Disks", "Samsung SSD 970 EVO Plus 1TB (932 GB, NVMe SSD, Windows) · WDC WD10EZEX (932 GB, hard disk)"],
          ["Screen", "Lanternfall LF32 32″, 2560×1440 at 60 Hz"],
          ["Sound", "Speakers (Lanternfall Soundbar) · Speakers (Realtek(R) Audio) · 4 more made by apps or going to a screen"],
          ["Keyboard", "Copperline Apex Pro"],
          ["Mouse", "Lanternfall Basilisk X"],
          ["Board", "ASUS TUF B450-PLUS GAMING, BIOS 4604 from 2024"],
          ["Windows", "Windows 11 Pro 25H2 (build 26200)"],
        ],
        screens: [
          {
            title: "Lanternfall LF32",
            sub: "32 inches · made for 2560×1440 · from 2021",
            rows: [
              { label: "Resolution", value: "2560×1440", low: false, open: "ms-settings:display" },
              { label: "Refresh rate", value: "60 Hz, can do 240", low: true, open: "ms-settings:display-advanced" },
              { label: "Scale", value: "100%", low: false, open: "ms-settings:display" },
              { label: "HDR", value: "Off", low: false, open: "ms-settings:display-hdr" },
              { label: "Colour", value: "8 bits", low: false, open: "ms-settings:display-advanced" },
              { label: "Plugged in by", value: "DisplayPort", low: false, open: null },
            ],
          },
          {
            title: "Windows",
            sub: "",
            rows: [
              { label: "Night light", value: "Warmer colours at night", low: false, open: "ms-settings:nightlight" },
              { label: "Games in a window", value: "Optimized", low: false, open: "ms-settings:display-advancedgraphics" },
              { label: "Auto HDR in games", value: "On", low: false, open: "ms-settings:display-hdr" },
              { label: "Text size", value: "100%", low: false, open: "ms-settings:easeofaccess-display" },
            ],
          },
        ],
        checks: [
          { id: "antivirus", area: "Security", level: "ok", title: "Microsoft Defender protects this PC", detail: "Real-time protection is on and its virus list is up to date.", fix: null },
          { id: "av-leftover-bitdefender", area: "Security", level: "tip", title: "Bitdefender's antivirus is gone, but its helpers aren't", detail: "Bitdefender Agent still runs at every start, with nothing left for it to look after. Uninstall it from Installed apps.", fix: { label: "Installed apps", kind: "open", value: "ms-settings:appsfeatures", hint: null } },
          { id: "windows-support", area: "Updates", level: "ok", title: "Windows 11 Pro 25H2 (build 26200) gets security updates", detail: "Until 12 October 2027.", fix: null },
          { id: "memory-speed", area: "Hardware", level: "warn", title: "Your memory runs at 2666, but it's made for 3600", detail: "Switching on D.O.C.P. in the BIOS runs it at full speed. Test it afterwards. With four sticks, if it isn't stable, try one step lower.", fix: { label: "Copy for your agent", kind: "copy", value: "My PC's memory runs at 2666 MT/s, but its sticks (4 x Crucial BL16G36C16U4BL) are rated for 3600 MT/s.", hint: "Paste it to your agent." } },
          { id: "display-refresh-0", area: "Hardware", level: "warn", title: "Your Lanternfall LF32 runs at 60 Hz", detail: "It can do 240 Hz at 2560×1440. Pick it under Advanced display; everything moves smoother.", fix: { label: "Display settings", kind: "open", value: "ms-settings:display-advanced", hint: null } },
          { id: "devices-broken", area: "Hardware", level: "warn", title: "Lanternfall USB Hub stopped after reporting a problem", detail: "Unplugging it and trying another USB port or cable usually fixes it.", fix: { label: "Device Manager", kind: "launch", value: "devmgmt.msc", hint: null },
            more: [{ label: "Copy for your agent", kind: "copy", value: "Device Manager on my PC shows these devices with a problem:\n- \"Lanternfall USB Hub\": stopped after reporting a problem (Device Manager code 43)\n", hint: "Paste it to your agent." }] },
          { id: "watch-change", area: "Hardware", level: "ok", title: "Since the BIOS update to 4604 on 12 Oct: no hardware errors", detail: "Mom keeps an eye on it for a week.", fix: null },
          { id: "backup-onedrive-off", area: "Backups", level: "warn", title: "Your Documents, Desktop and Pictures aren't backed up", detail: "They're in your OneDrive folder, but OneDrive isn't running, so nothing syncs.", fix: { label: "Windows Backup", kind: "open", value: "ms-settings:backup", hint: null },
            more: [{ label: "Copy for your agent", kind: "copy", value: "My Documents, Desktop and Pictures live in my OneDrive folder, but OneDrive isn't running, so nothing syncs.", hint: "Paste it to your agent." }] },
          { id: "drive-repairs-f", area: "Backups", level: "warn", title: "Windows found damage on F: 16 times in 3 months", detail: "Usually from losing power or its connection while saving, sometimes a drive wearing out. Copy what matters somewhere else first.", fix: { label: "Show F:", kind: "reveal", value: "F:\\", hint: null },
            more: [{ label: "Copy for your agent", kind: "copy", value: "Windows found file system damage on my drive F: (\"Backup\") 16 times in the last three months.", hint: "Paste it to your agent." }] },
          { id: "ssd-health-1", area: "Hardware", level: "ok", title: "Samsung SSD 970 EVO Plus 1TB is healthy", detail: "12% of its rated life used, 59 °C, 173 TB written, switched on for 3.6 years in all. No errors.", fix: null },
          { id: "sound-outputs", area: "Sound", level: "tip", title: "6 sound outputs, 4 made by apps", detail: "To switch one off: Sound settings, open it, Don't allow.",
            fix: { label: "Sound settings", kind: "open", value: "ms-settings:sound", hint: null },
            more: [{ label: "Copy for your agent", kind: "copy", value: "Help me tidy the sound devices on this Windows PC. There are more than I use, and I don't know which are safe to remove.\n\nOutputs that are switched on:\n- Speakers (Lanternfall Soundbar)\n- Speakers (Realtek(R) Audio)\n- Steam Streaming Microphone\n- Steam Streaming Speakers\n- VB-Audio Virtual Cable\n- Voicemod Virtual Audio Device\n\nFor each device: work out what it is and what installed it, check whether anything uses it, and tell me plainly: keep it, switch it off, or remove it.\n\nRules: only look at first. Change nothing until I say yes to that item.", hint: "Paste it to Claude Code or Codex." }],
            items: [
              { name: "Speakers (Lanternfall Soundbar)", note: "Real. Keep.", led: "green" },
              { name: "Speakers (Realtek(R) Audio)", note: "Real. Keep.", led: "green" },
              { name: "Steam Streaming Microphone", note: "Steam Remote Play. Off if you don't use it.", led: "off" },
              { name: "Steam Streaming Speakers", note: "Steam Remote Play. Off if you don't use it.", led: "off" },
              { name: "VB-Audio Virtual Cable", note: "Passes sound between apps (VB-CABLE). Off if you don't use it.", led: "off" },
              { name: "Voicemod Virtual Audio Device", note: "Voicemod's voice changer. Off if you don't use it.", led: "off" },
            ] },
          { id: "sound-leftover-voicemod", area: "Sound", level: "tip", title: "Voicemod is gone, but its sound device isn't", detail: "Voicemod Virtual Audio Device is still here. In Device Manager, find it under Sound, video and game controllers, right-click it and choose Uninstall device.", fix: { label: "Device Manager", kind: "launch", value: "devmgmt.msc", hint: null } },
          { id: "broken-services", area: "Registry", level: "warn", title: "1 service is set to start with Windows, but its program is gone", detail: "ASUS Com Service. Windows tries to start it at every boot, waits, and gives up, which slows starting up. Left by a program uninstalled the hard way.", fix: null },
          { id: "broken-uninstall", area: "Registry", level: "tip", title: "1 program in Windows' app list can't be uninstalled", detail: "Anaconda3 2020.02. Its uninstaller is gone (it was deleted by hand, most likely), so Settings lists it but can't remove it.", fix: { label: "Installed apps", kind: "open", value: "ms-settings:appsfeatures", hint: null } },
          ...yourChecks(),
        ],
      }
    ),
    blank_secrets: async () => (await wait(400), (secretsBlanked = true), "Took 2 keys out of your PowerShell history."),
    files_places: () => JSON.parse(JSON.stringify(places)),
    set_files_place: ({ path, on }) => ((places.find((x) => x.path === path).on = on), JSON.parse(JSON.stringify(places))),
    add_files_place: async () => (await wait(300), places.push({ path: "E:\\Photos", name: "Photos", kind: "added", on: true, total: 0, free: 0 }), JSON.parse(JSON.stringify(places))),
    get_browsers: async () => {
      await wait(700);
      const out = JSON.parse(JSON.stringify(browsers));
      for (const b of out) for (const p of b.profiles) for (const e of p.extensions) e.icon = e.icon || mockIcon(e.name);
      return out;
    },
    get_icons: async ({ exes }) => (await wait(120), Object.fromEntries(exes.map((x) => [x, mockIcon(x.split("\\").pop())]))),
    get_programs: async () => (await wait(250), JSON.parse(JSON.stringify(programs))),
    program_end_one: async ({ id, pid }) => {
      await wait(500);
      const p = programs.find((x) => x.id === id);
      const l = p && p.procs.find((x) => x.pid === pid);
      if (!l) return { ok: false, message: "That process isn't running any more." };
      p.procs.splice(p.procs.indexOf(l), 1);
      p.count -= 1;
      p.cpu = Math.max(0, p.cpu - l.cpu);
      p.mem -= l.mem;
      return { ok: true, message: `Stopped ${l.what.slice(0, 60)}.` };
    },
    program_action: async ({ id, action }) => {
      await wait(action === "restart" ? 1800 : 700);
      const p = programs.find((x) => x.id === id);
      if (!p || !p.can.includes(action)) return { ok: false, message: "That program isn't running any more." };
      if (action === "restart") return { ok: true, message: `Restarted ${p.name}.` };
      programs.splice(programs.indexOf(p), 1);
      return { ok: true, message: action === "quit" ? `Closed ${p.name}.` : `Ended ${p.name} (${p.count} processes).` };
    },
    get_startup: async () => (await wait(250), JSON.parse(JSON.stringify(startup))),
    set_startup: async ({ id, on }) => {
      const x = startup.find((y) => y.id === id);
      x.enabled = on;
      return on ? `${x.name} will start with Windows again.` : `${x.name} won't start with Windows any more. It's still installed; open it whenever you like.`;
    },
    get_speed: async () => (await wait(400), { ...JSON.parse(JSON.stringify(speed)), at: Math.floor(Date.now() / 1000) - 5 }),
    room_paths: () => [room.path],
    get_rooms: () => [room],
    inspect_room: async () => (await wait(300), room),
    add_room: () => null,
    remove_room: () => null,
    apply_tidy: async () => (await wait(500), { moved: 18, recycled: 40, removed: 2, failed: [], journal: null, message: "Put away 18 things, recycled 40. Changed your mind? Undo is right there." }),
    undo_tidy: async () => ({ restored: 18, failed: [], message: "Put back 18 things, exactly as they were." }),
    chore_prompt: () => "Mom here. Your room got an F...",
    // "?late" in the address shows skills that are behind; otherwise every copy is the newest.
    get_homework: async () => (await wait(400), JSON.parse(JSON.stringify(homework))),
    get_waiting: () => (location.search.includes("waiting") ? waiting : []),
    get_rules: () => {
      if (location.search.includes("late")) return rules;
      const fresh = JSON.parse(JSON.stringify(rules));
      for (const s of fresh.skills) {
        s.behind = false;
        for (const c of s.copies) Object.assign(c, { behind: false, version: s.version, update: null });
      }
      return { ...fresh, skillsUpdate: [], skillsCatchUp: null };
    },
    save_rules: () => "Saved. Mom will hold everyone to it.",
    reset_rules: () => "Back to mom's original rules.",
    teach_rules: ({ agent, on }) => {
      const t = rules.targets.find((x) => x.agent === agent);
      t.installed = on;
      t.upToDate = on;
      return on ? `Taught ${t.label} the house rules.` : `Took the house rules out of ${t.path}.`;
    },
    set_integration: ({ id, on }) => {
      const i = rules.integrations.find((x) => x.id === id);
      i.installed = on;
      return on ? `Installed ${i.label}.` : `Removed ${i.label}.`;
    },
    get_settings: () => JSON.parse(JSON.stringify(settings)),
    set_settings: ({ config }) => {
      // ?nosave: the settings file can't be written, to see that nothing is shown as changed.
      if (location.search.includes("nosave")) throw "Access is denied. (os error 5)";
      settings.config = config;
      return null;
    },
    set_autostart: ({ on }) => on,
    get_diary: () => [],
    get_history: async () => (await wait(150), JSON.parse(JSON.stringify(history))),
    undo_history: async ({ id }) => {
      await wait(500);
      const e = history.entries.find((x) => x.id === id);
      if (!e || !e.action) throw "That one can't be put back now.";
      const said = { "Bring back": "Brought 2 folders back from the archive · 3.1 GB", Undo: "Put 14 things back where they were.", "Start again": "Started vite and 2 more again" }[e.action];
      e.undone = true;
      e.action = null;
      history.entries.unshift(hist(`u${id}`, 0, "undo", said, { undoes: id }));
      return said;
    },
    letter_read: () => {
      history.letterUnread = false;
      return null;
    },
    open_url: ({ url }) => {
      if (DEMO && /^https:\/\//.test(String(url))) window.open(url, "_blank", "noopener");
      return null;
    },
    reveal: () => null,
    open_recycle_bin: () => null,
    hide_window: () => null,
    // In the demo the page around the widget grows the frame; the widget waits to hear it has.
    set_big: ({ on }) =>
      !DEMO
        ? false
        : new Promise((done) => {
            const finish = () => (clearTimeout(timer), done(false));
            const timer = setTimeout(finish, 2000);
            fromPage((word) => word === "sized" && finish());
            toPage(on ? "big" : "small");
          }),
    big_arrived: () => null,
    card_away: () => null,
    set_paused: () => null,
    stop_container: () => null,
    stop_wsl: () => null,
  };

  window.momMock = {
    invoke: async (cmd, args) => {
      const h = handlers[cmd];
      if (!h) throw new Error(`mock: no handler for ${cmd}`);
      return h(args || {});
    },
    listen: async (name, f) => {
      (listeners[name] = listeners[name] || []).push(f);
      return () => {};
    },
  };
})();

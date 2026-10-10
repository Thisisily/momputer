/* Pretend backend for working on the UI in a normal browser (open index.html).
   Only used when the page isn't running inside the momputer app. The data is modeled
   on a real Tuesday on a machine running far too many agents. */
(() => {
  if (window.__TAURI__) return;
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
    mood: { face: "disappointed", level: 2, line: `${left.length} leftovers. Were you raised in a barn?` },
  };

  const GB = 1024 * MB;
  const D2 = 86400;
  const item = (name, sub, bytes, goes, reason, ageSecs, files = 1) => ({ name, sub, path: `C:\\Users\\me\\AppData\\Local\\Temp\\${name.replace(/\W+/g, "-")}`, bytes, files, ageSecs, goes, reason });
  const spot = (o) => ({ exact: true, available: true, measuredAt: now, owner: null, note: null, path: null, keptBytes: o.totalBytes - o.reclaimableBytes, ...o });
  const spots = [
    spot({
      id: "claude-temp", label: "Claude Code scratch", group: "agents", action: "delete", safety: "safe", owner: "claude-code",
      what: "Scratchpads, screenshots and task output Claude Code saves while it works. Each session gets its own folder.",
      effect: "Deletes 4 scratch folders from Claude Code sessions that have ended, permanently (they don't go to the Recycle Bin). Folders of the 2 sessions still running, and anything changed in the last 24 hours, are left alone.",
      path: "C:\\Users\\me\\AppData\\Local\\Temp\\claude", totalBytes: 15.2 * GB, reclaimableBytes: 1.2 * GB, reclaimableItems: 4, keptItems: 2,
      items: [
        item("storefront · \u201cUpload the garden video and reply to comments\u201d", "212 files, mostly screenshots", 820 * MB, true, "Session ended 3 days ago", 3 * D2, 212),
        item("atlas · \u201cFix the checkout page on mobile\u201d", "48 files, mostly logs", 260 * MB, true, "Session ended 5 days ago", 5 * D2, 48),
        item("games · Session 0191f20b", "12 files, mostly videos", 90 * MB, true, "Session ended 9 days ago", 9 * D2, 12),
        item("storefront · \u201cDraft the newsletter\u201d", "6 files", 41 * MB, true, "Session ended 2 days ago", 2 * D2, 6),
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
      what: "node_modules and Rust target folders in projects nobody has worked on for a month. They're rebuilt from the project files whenever you need them.",
      effect: "Deletes the ticked folders permanently. Your code isn't touched; run npm install or cargo build in a project to get its folder back.",
      totalBytes: 9.8 * GB, reclaimableBytes: 6.1 * GB, reclaimableItems: 3, keptItems: 2,
      items: [
        item("garden-v1 · target", "Rust build output", 3.9 * GB, true, "Last worked on 74 days ago", 74 * D2),
        item("old-landing · node_modules", "41,200 files", 1.4 * GB, true, "Last worked on 45 days ago", 45 * D2),
        item("nimbus-2024 · node_modules", "28,900 files", 800 * MB, true, "Last worked on 120 days ago", 120 * D2),
        item("storefront · node_modules", "52,000 files", 2.6 * GB, false, "Worked on 2 hours ago", 7200),
        item("momputer · target", "Rust build output", 1.1 * GB, false, "Worked on today", 600),
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
      effect: "Deletes cached web data for the browsers that are closed. Sites load a little slower the first time; logins, history, bookmarks and passwords are untouched.",
      totalBytes: 2.9 * GB, reclaimableBytes: 1.6 * GB, reclaimableItems: 2, keptItems: 1,
      items: [
        item("Chrome, Default profile", "Cache, Code Cache, GPUCache", 1.2 * GB, true, "Chrome is closed", 3600),
        item("Brave, Default profile", "Cache", 400 * MB, true, "Brave is closed", 7200),
        item("Edge, Default profile", "Cache, Code Cache", 1.3 * GB, false, "Edge is open", 60),
      ],
    }),
    spot({
      id: "recycle-bin", label: "Recycle Bin", group: "windows", action: "empty-bin", safety: "review",
      what: "Files you deleted. Windows keeps them here in case you change your mind.",
      effect: "Empties the Recycle Bin on every drive. This is permanent: the files can't be brought back.",
      totalBytes: 1.8 * GB, reclaimableBytes: 1.8 * GB, reclaimableItems: 312, keptItems: 0,
      items: [
        item("render-final-v3.mp4", "from C:\\Workspace\\storefront\\out", 900 * MB, true, "Deleted 12 days ago", 12 * D2),
        item("dataset-old.zip", "from D:\\Downloads", 640 * MB, true, "Deleted 30 days ago", 30 * D2),
        item("screenshots (2).zip", "from C:\\Users\\me\\Desktop", 120 * MB, true, "Deleted 2 days ago", 2 * D2),
      ],
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
      what: "Setup files in Downloads you ran more than a month ago. The apps they installed don't need them.",
      effect: "Moves the ticked installers to the Recycle Bin, so you can still get them back. Installed apps aren't affected.",
      totalBytes: 1.1 * GB, reclaimableBytes: 1.1 * GB, reclaimableItems: 3, keptItems: 0,
      items: [
        item("DockerDesktopInstaller.exe", "Downloads", 620 * MB, true, "Downloaded 64 days ago", 64 * D2),
        item("Blender-4.2-windows-x64.msi", "Downloads", 380 * MB, true, "Downloaded 90 days ago", 90 * D2),
        item("OBS-Studio-30.1-Installer.exe", "Downloads", 140 * MB, true, "Downloaded 41 days ago", 41 * D2),
      ],
    }),
  ];

  const health = {
    drives: [
      { root: "C:\\", label: "Windows", total: 476 * GB, free: 41 * GB, system: true },
      { root: "D:\\", label: "Data", total: 931 * GB, free: 512 * GB, system: false },
    ],
    memory: { total: 64 * GB, used: 47 * GB },
    uptimeSecs: 9 * D2 + 4 * 3600,
    checks: [
      { id: "restart", level: "tip", title: "Up 9 days", detail: "A restart clears out memory leaks and finishes updates.", action: null },
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
    topMemory: [
      { name: "chrome.exe", mem: 5.1 * GB, count: 38 },
      { name: "node.exe", mem: 3.4 * GB, count: 22 },
      { name: "Code.exe", mem: 2.2 * GB, count: 14 },
      { name: "vmmem", mem: 2.1 * GB, count: 1 },
      { name: "claude.exe", mem: 1.6 * GB, count: 6 },
      { name: "msedgewebview2.exe", mem: 900 * MB, count: 11 },
    ],
  };

  const room = {
    path: "C:\\Workspace\\storefront\\data\\field-notes",
    name: "field-notes",
    grade: "F",
    score: 59,
    line: "I'm not cleaning this. You are.",
    entries: 57,
    codeProject: false,
    findings: [
      { kind: "misspelled", title: "\"Docuements\" looks like a typo of \"Documents\"", detail: "Agents copy folder names exactly. A typo now means reports end up in two places later.", paths: [], bytes: 0 },
      { kind: "loose-reports", title: "2 report file(s) lying around the top level", detail: "Reports, plans and logs belong in Documents/.", paths: [], bytes: 0 },
      { kind: "family", title: "8 look-alike folders: radio-night-show-*", detail: "Versions and variants of the same thing. One folder, radio-night-show/, keeps them together.", paths: [], bytes: 0 },
      { kind: "family", title: "7 look-alike folders: podcast-*", detail: "Dated work logs. They belong together in Documents/podcast/.", paths: [], bytes: 0 },
      { kind: "screenshot-pile", title: "40 screenshots piled up in garden/shots/", detail: "2 MB of captures. Delete them once they've done their job (video uploaded, bug fixed).", paths: [], bytes: 2 * MB },
      { kind: "crowded", title: "57 things at the top level", detail: "Past about 30, nobody can find anything. Group related folders.", paths: [], bytes: 0 },
    ],
    actions: [
      { id: "a1", op: "rename", targets: [], to: null, title: "Rename Docuements to Documents", reason: "Fix the folder name so everyone files reports in the same place.", bytes: 0, selected: false },
      { id: "a2", op: "move", targets: [], to: null, title: "Move 2 report file(s) into Documents/", reason: "Reports go in one clearly named place.", bytes: 0, selected: true },
      { id: "a3", op: "move", targets: [], to: null, title: "Group 8 folder(s) into radio-night-show/", reason: "One thing, one folder.", bytes: 0, selected: true },
      { id: "a4", op: "move", targets: [], to: null, title: "Group 7 folder(s) into Documents/podcast/", reason: "One thing, one folder.", bytes: 0, selected: true },
      { id: "a5", op: "recycle", targets: [], to: null, title: "Recycle 40 screenshots in garden/shots/ (2 MB)", reason: "Screenshots are temporary. The Recycle Bin keeps them for a while, just in case.", bytes: 2 * MB, selected: true },
    ],
    scannedAt: now,
  };

  const settings = {
    config: {
      rooms: [room.path],
      leftovers: { forgottenAfterHours: 12, includeUnknownOrphans: true, autoStop: false, autoStopAfterMinutes: 20, autoStopOrphans: false },
      hooks: { enabled: true, nagProcesses: true, nagClutter: true, shareRules: true, stopOnSessionEnd: false, minAgeSecs: 5 },
      tidy: { reportsFolder: "Documents", minFamily: 3, screenshotPile: 10 },
      trash: { tempMinAgeDays: 7, agentTempMinAgeHours: 24 },
      ui: { alwaysOnTop: false, notifications: true, scanIntervalSecs: 3, cpuAlertPercent: 25 },
      protect: [],
      kept: [],
    },
    autostart: false,
    version: "0.1.0",
    configPath: "C:\\Users\\me\\.momputer\\config.toml",
  };

  const rules = {
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
    return { stopped: [], alreadyGone: [], failed: [], freedMem: freed, groups: gone.length, message: gone.length === 1 ? "Turned it off. You're welcome." : `Turned off ${gone.length} things. You're welcome.` };
  }

  const handlers = {
    get_snapshot: () => JSON.parse(JSON.stringify(snapshot)),
    refresh: () => null,
    stop_groups: async ({ ids }) => (await wait(500), stop(ids)),
    stop_leftovers: async () => (await wait(700), stop(snapshot.groups.filter((x) => x.status === "left-behind" || x.status === "orphaned").map((x) => x.id))),
    keep_group: () => null,
    get_trash: () => ({ spots: [], busy: false }),
    measure_trash: async () => {
      const list = [];
      for (const s of spots) {
        await wait(250);
        list.push(s);
        emit("trash", { spots: JSON.parse(JSON.stringify(list)), busy: true });
      }
      emit("trash", { spots: JSON.parse(JSON.stringify(list)), busy: false });
    },
    clean_trash: async ({ id, only }) => {
      await wait(800);
      const s = spots.find((x) => x.id === id);
      const goes = s.items.filter((i) => i.goes && (!only || only.includes(i.path)));
      const freed = only ? goes.reduce((a, i) => a + i.bytes, 0) : s.reclaimableBytes;
      s.items = s.items.filter((i) => !goes.includes(i));
      s.totalBytes -= freed;
      s.reclaimableBytes -= freed;
      s.reclaimableItems = s.items.filter((i) => i.goes).length;
      emit("trash", { spots: JSON.parse(JSON.stringify(spots)), busy: false });
      return { id, freed, deleted: goes.length, skipped: 0, message: `Took out ${(freed / 1024 / MB).toFixed(1)} GB of trash.` };
    },
    get_health: async () => (await wait(150), JSON.parse(JSON.stringify(health))),
    room_paths: () => [room.path],
    get_rooms: () => [room],
    inspect_room: async () => (await wait(300), room),
    add_room: () => null,
    remove_room: () => null,
    apply_tidy: async () => (await wait(500), { moved: 18, recycled: 40, removed: 2, failed: [], journal: null, message: "Put away 18 thing(s), recycled 40. Changed your mind? Undo is right there." }),
    undo_tidy: async () => ({ restored: 18, failed: [], message: "Put back 18 thing(s), exactly as they were." }),
    chore_prompt: () => "Mom here. Your room got an F...",
    get_rules: () => rules,
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
      settings.config = config;
      return null;
    },
    set_autostart: ({ on }) => on,
    get_diary: () => [],
    open_url: () => null,
    reveal: () => null,
    hide_window: () => null,
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

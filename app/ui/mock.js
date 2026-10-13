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
    mood: { face: "helping", level: 2, line: `${left.length} leftovers. One tap and they're gone.` },
  };

  const GB = 1024 * MB;
  const D2 = 86400;
  const item = (name, sub, bytes, goes, reason, ageSecs, files = 1) => ({ name, sub, path: `C:\\Users\\me\\AppData\\Local\\Temp\\${name.replace(/\W+/g, "-")}`, bytes, files, ageSecs, goes, optional: false, reason });
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
  ].map(([who, game, what, mb, days]) => item(`${who} · ${game} · ${what}`, what.includes("crash") ? "Crash report" : "Log, 1 file", mb * MB, true, `Untouched for ${days} days`, days * D2));

  const spots = [
    spot({
      id: "game-junk", label: "Games' logs and crash reports", group: "personal", action: "delete", safety: "safe",
      what: "Logs and crash reports games write every time they run (never your saves).",
      effect: "Deletes 12 logs and crash reports (1.7 GB) that games wrote and nobody has touched in 67 days, permanently (they don't go to the Recycle Bin). Games write new ones the next time they run; your saves, settings and the games themselves aren't touched.",
      totalBytes: games.reduce((a, i) => a + i.bytes, 0), reclaimableBytes: games.reduce((a, i) => a + i.bytes, 0), reclaimableItems: games.length, keptItems: 0,
      items: games,
    }),
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
        recent(item("storefront · node_modules", "52,000 files", 2.6 * GB, false, "Worked on 2 hours ago", 7200)),
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
      effect: "Deletes cached web data for the browsers that are closed. Sites load a little slower the first time; logins, history, bookmarks and passwords are untouched.",
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
      effect: "Moves 4 folders (3.1 GB) of Codex and Claude Code conversations nobody has opened in 67 days to a drive you choose, and leaves a link in each one's place, so they still open from where they were. Nothing is deleted, and Bring back puts any of them back here. While that drive is unplugged, those conversations can't be opened.",
      totalBytes: 4.4 * GB, reclaimableBytes: 3.1 * GB, reclaimableItems: 4, keptItems: 2,
      items: [
        item("Codex · July 2026", "2,140 files, mostly logs", 2.1 * GB, true, "Not opened for 72 days", 72 * D2, 2140),
        item("Codex · June 2026", "610 files, mostly logs", 700 * MB, true, "Not opened for 103 days", 103 * D2, 610),
        item("Claude Code · storefront · Fix the checkout page on mobile", "48 files, mostly code", 200 * MB, true, "Not opened for 80 days", 80 * D2, 48),
        item("Codex · 2 August 2026", "120 files, mostly logs", 100 * MB, true, "Not opened for 70 days", 70 * D2, 120),
        recent(item("Codex · 20 September 2026", "300 files, mostly logs", 1.1 * GB, false, "Opened 21 days ago (kept here for 67 days)", 21 * D2, 300)),
        item("Codex · 11 October 2026", "40 files, mostly logs", 200 * MB, false, "Opened 2 hours ago", 7200, 40),
      ],
    }),
    spot({
      id: "windows-junk", label: "Windows leftovers", group: "windows", action: "delete", safety: "safe", admin: true,
      what: "Windows Update keeps the files it downloaded after installing them, Windows keeps error reports about crashes, and Windows setup leaves folders at the top of whichever drive had room. Cleaning these needs an administrator, so Windows asks you first.",
      effect: "Deletes 3 items (1.9 GB) that Windows Update, Windows and Installers left behind and nobody has touched in 67 days, permanently (they don't go to the Recycle Bin): downloaded updates that are installed already, error reports and folders installers left at the root of the drive. Windows and your programs keep working; nothing they still need is in here. Anything newer, or in use, stays.",
      totalBytes: 1.9 * GB, reclaimableBytes: 1.9 * GB, reclaimableItems: 3, keptItems: 0,
      items: [
        item("Windows Update · update files 8d2a1c0f9e7b", "Downloaded update, 412 files", 1.2 * GB, true, "Untouched for 88 days", 88 * D2, 412),
        item("Installers · C:\\Windows10Upgrade", "Installer leftovers, 28 files", 600 * MB, true, "Untouched for 1200 days", 1200 * D2, 28),
        item("Windows · AppCrash_atlas.exe_1f2e", "Error report", 100 * MB, true, "Untouched for 90 days", 90 * D2, 3),
      ],
    }),
    spot({
      id: "forgotten-apps", label: "Forgotten app data", group: "personal", action: "move", safety: "review",
      what: "Big folders apps keep in AppData and your home folder: the data of apps you've since uninstalled, folders of an app's older versions, and things nobody has touched in a long time. Mom guesses which app each belongs to, so nothing here is deleted: it moves to your archive drive with a link left behind. Keys, passwords, wallets and browser profiles are never offered.",
      effect: "Moves 4 folders (6.2 GB) to a drive you choose, with a link left in each one's place: data of apps you've uninstalled. Nothing is deleted. Bring back puts any of them back, and once you're sure you don't need one, Delete for good removes it from there. Mom guesses which app a folder belongs to, so look through the list first.",
      totalBytes: 8.9 * GB, reclaimableBytes: 6.2 * GB, reclaimableItems: 4, keptItems: 2,
      items: [
        item("Nimbus Studio (AppData\\Roaming)", "41,200 files, mostly images", 3.9 * GB, true, "Its app isn't installed any more. Untouched for 412 days", 412 * D2, 41200),
        item("Google\\AndroidStudio2021.2 (AppData\\Roaming)", "8,100 files", 1.1 * GB, true, "An older version; AndroidStudio2024.2 is the one in use. Untouched for 705 days", 705 * D2, 8100),
        item("Atlas Engine (AppData\\LocalLow)", "310 files, mostly data files", 800 * MB, true, "Its app isn't installed any more. Untouched for 290 days", 290 * D2, 310),
        item("SquirrelTemp (AppData\\Local)", "12 files", 400 * MB, true, "Its app isn't installed any more. Untouched for 649 days", 649 * D2, 12),
        recent(item(".affinity (your home folder)", "2,900 files", 1.9 * GB, false, "Its app is still installed, but nobody has touched this in 416 days", 416 * D2, 2900)),
        recent(item("Figma (AppData\\Local)", "600 files", 800 * MB, false, "Its app isn't installed any more. Untouched for 101 days", 101 * D2, 600)),
      ],
    }),
    spot({
      id: "old-temp", label: "Old temp files", group: "windows", action: "delete", safety: "safe", path: "C:\\Users\\me\\AppData\\Local\\Temp",
      what: "Files programs and installers left in your Temp folder and never cleaned up.",
      effect: "Deletes 4,634 items (all empty) from your Temp folder that nobody has touched in 7 days, permanently (they don't go to the Recycle Bin). Anything newer, or in use by a running program, stays. Programs make new temp files when they need them.",
      totalBytes: 1.4 * GB, reclaimableBytes: 0, reclaimableItems: 4634, keptItems: 8231, newItems: 8230, newBytes: 1.2 * GB,
      items: [
        item("tmp3F2A", "Empty folder", 0, true, "Untouched for 41 days", 41 * D2, 0),
        item("tmp9C11", "Empty folder", 0, true, "Untouched for 33 days", 33 * D2, 0),
        item("_MEI48212", "Empty folder", 0, true, "Untouched for 12 days", 12 * D2, 0),
        recent(item("vscode-typescript", "1,204 files, mostly code", 410 * MB, false, "Changed 3 hours ago (kept for 7 days)", 3 * 3600, 1204)),
        item("Nimbus-updater", "88 files", 200 * MB, false, "In use by a running program", 60, 88),
        recent(item("pip-build-x81k", "310 files", 120 * MB, false, "Changed 2 days ago (kept for 7 days)", 2 * D2, 310)),
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
      totalBytes: 1.1 * GB + 210 * MB, reclaimableBytes: 1.1 * GB, reclaimableItems: 3, keptItems: 1,
      items: [
        item("DockerDesktopInstaller.exe", "Downloads", 620 * MB, true, "Downloaded 64 days ago", 64 * D2),
        item("Blender-4.2-windows-x64.msi", "Downloads", 380 * MB, true, "Downloaded 90 days ago", 90 * D2),
        item("OBS-Studio-30.1-Installer.exe", "Downloads", 140 * MB, true, "Downloaded 41 days ago", 41 * D2),
        recent(item("Figma-Setup.exe", "Downloads", 210 * MB, false, "Downloaded 6 days ago (kept for 30 days)", 6 * D2)),
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
      big: [{ path: "C:\\Users\\me\\Videos\\Raw footage\\atlas-run-capture.mkv", size: 14.2 * GB, accessed: now - 500 * D2, modified: now - 520 * D2, kind: "videos", folder: "Videos" }],
      scannedFiles: 8309,
      scannedBytes: 29.4 * GB,
      roots: [],
      complete: true,
      accessTimes: true,
      measuredAt: now - 2 * 3600,
    };
  };
  let filesReport = null;

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
      { root: "C:\\", label: "Windows", total: 476 * GB, free: 41 * GB, system: true },
      { root: "D:\\", label: "Data", total: 931 * GB, free: 512 * GB, system: false },
    ],
    memory: { total: 64 * GB, used: 47 * GB },
    uptimeSecs: 9 * D2 + 4 * 3600,
    checks: [
      { id: "restart", level: "tip", title: "Up 9 days", detail: "A restart clears out memory leaks and finishes updates.", action: null },
      { id: "unused-apps", level: "tip", title: "3 apps you haven't opened in a year", detail: "Nimbus Notes 10.3, Atlas Recorder, Retro Racer. Uninstalling the ones you don't need frees about 1.3 GB; they're listed below.", action: null },
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
      { key: "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\nimbus-notes", name: "Nimbus Notes 10.3", publisher: "Nimbus Labs", version: "10.3", size: 915 * MB, lastUsed: now - 1703 * D2 },
      { key: "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{AtlasRecorder}", name: "Atlas Recorder", publisher: "Atlas", version: "2.1", size: 400 * MB, lastUsed: now - 1067 * D2 },
      { key: "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\retro-racer", name: "Retro Racer", publisher: "Pixel Forge", version: "1.0", size: 0, lastUsed: now - 2081 * D2 },
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
  });
  const programs = [
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
    id: name.toLowerCase().replace(/[^a-z]/g, "").padEnd(32, "a").slice(0, 32), name, description: extra.description || "", version: "1.0", on, app: !!extra.app, kind,
    can, everywhere: can[0] === "Reads and changes every site you visit", added: now - (extra.addedDays || 400) * 86400, lastSaved: saved == null ? null : now - saved * 86400,
    size: (extra.mb || 4) * MB, page: "chrome://extensions/?id=x",
  });
  const everywhere = "Reads and changes every site you visit";
  const browsers = [
    {
      id: "chrome", name: "Chrome", open: true, openSecs: 2 * 86400 + 3 * 3600, processes: 78, mem: 13 * GB, restart: "chrome://restart", extensionsPage: "chrome://extensions",
      profiles: [
        { dir: "Default", path: "C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data\\Default", name: "Personal", lastUsed: now - 3000, size: 13.4 * GB, cache: 1.9 * GB, history: 123 * MB, cookiesBytes: 2.1 * MB, cookies: null, cookieSites: null,
          notes: [
            { level: "tip", text: "15 Chrome apps AudioSauna, Beatlab, Coggle, Desmos and 11 more: Chrome stopped running Chrome apps on Windows, so they just sit there." },
            { level: "tip", text: "2 ad blockers do the same job twice: AdBlock, AdBlocker Ultimate. One is enough, and each slows every page." },
            { level: "tip", text: "18 extensions can read and change every site you visit, your bank's included. Keep the ones you trust and use." },
          ],
          extensions: [
            ext("AdBlock", "Ad blocker", true, [everywhere, "Blocks or changes what pages load"], 0, { mb: 22 }),
            ext("AdBlocker Ultimate", "Ad blocker", true, [everywhere, "Blocks or changes what pages load"], 0, { mb: 9 }),
            ext("Grammarly", "AI assistant", true, [everywhere, "Sees your open tabs and their addresses"], 0, { mb: 31 }),
            ext("Phantom", "Crypto wallet", true, [everywhere], 0, { mb: 48 }),
            ext("VPN for Chrome", "VPN", true, ["Sends your browsing through its own servers", everywhere], 0, { mb: 6 }),
            ext("Desmos Graphing Calculator", null, true, [], null, { app: true, mb: 1 }),
            ext("Sketchpad", null, true, [], null, { app: true, mb: 2 }),
            ext("Nami", "Crypto wallet", true, [everywhere], 321, { mb: 12 }),
            ext("Toby: Tab Manager", "Tab manager", false, ["Sees your open tabs and their addresses"], null, { mb: 3 }),
          ] },
        { dir: "Profile 3", path: "C:\\x", name: "Work", lastUsed: now - 3 * 86400, size: 6.9 * GB, cache: 5.5 * GB, history: 96 * MB, cookiesBytes: 1.2 * MB, cookies: null, cookieSites: null, notes: [],
          extensions: [ext("Claude", "AI assistant", true, [everywhere], 0, { mb: 12 })] },
        { dir: "Profile 7", path: "C:\\x", name: "Your Chrome (Profile 7)", lastUsed: now - 214 * 86400, size: 536 * MB, cache: 493 * MB, history: 320 * 1024, cookiesBytes: 400 * 1024, cookies: null, cookieSites: null, notes: [{ level: "tip", text: "Not used in 214 days. If it's an old profile, removing it frees its space." }], extensions: [] },
      ],
    },
    {
      id: "brave", name: "Brave", open: false, openSecs: 0, processes: 0, mem: 0, restart: "brave://restart", extensionsPage: "brave://extensions",
      profiles: [{ dir: "Default", path: "C:\\x", name: "Profile 1", lastUsed: now - 246 * 86400, size: 145 * MB, cache: 69 * MB, history: 192 * 1024, cookiesBytes: 300 * 1024, cookies: 412, cookieSites: 63, notes: [], extensions: [ext("uBlock Origin", "Ad blocker", true, [everywhere], 240)] }],
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

  const settings = {
    config: {
      rooms: [room.path],
      leftovers: { forgottenAfterHours: 12, includeUnknownOrphans: true, autoStop: false, autoStopAfterMinutes: 20, autoStopOrphans: false },
      hooks: { enabled: true, nagProcesses: true, nagClutter: true, shareRules: true, stopOnSessionEnd: false, minAgeSecs: 5 },
      tidy: { reportsFolder: "Documents", minFamily: 3, screenshotPile: 10 },
      trash: { tempMinAgeDays: 7, agentScratchDays: 67, archiveDir: null, conversationsAfterDays: 67 },
      ui: { alwaysOnTop: false, notifications: true, notifyLeftovers: true, notifyMeltdown: true, notifyCleanups: true, notifyAfterMinutes: 10, notifyEveryMinutes: 30, scanIntervalSecs: 3, cpuAlertPercent: 25, checkUpdates: true },
      protect: [],
      kept: [],
    },
    autostart: false,
    version: "0.1.0",
    configPath: "C:\\Users\\me\\.momputer\\config.toml",
  };

  const rules = {
    skills: [{"name": "mom", "title": "Mom", "what": "Teaches your agent to clean up after itself.", "more": "Before it says it's done, it stops what it started, bins its own temp files and puts reports where they belong. It loads by itself; you don't have to ask.", "prompts": ["Clean up after yourself", "What did you leave running?"], "has": ["Claude Code", "Codex"], "page": "https://github.com/Thisisily/momputer/tree/main/skills/mom"}, {"name": "tidy-folders", "title": "Tidy folders", "what": "Puts loose papers, old installers and stray games where they belong.", "more": "It looks first, shows one page of what it found, and moves only what you tick. Every move is logged and can be undone.", "prompts": ["Tidy up my Downloads", "Where are my tax papers?", "Are all my games in the Games folder?"], "has": ["Claude Code"], "page": "https://github.com/Thisisily/momputer/tree/main/skills/tidy-folders"}, {"name": "desktop-icons", "title": "Desktop icons", "what": "Clears dead shortcuts, duplicates and loose files off your desktop.", "more": "It reads shortcuts without ever running one. Shortcuts go to the Recycle Bin, files move into folders, and there's an undo.", "prompts": ["Clean up my desktop", "Which of my desktop shortcuts are broken?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/desktop-icons"}, {"name": "rename-documents", "title": "Rename documents", "what": "Gives scans and downloads names you can find later.", "more": "scan0001.pdf becomes 2024-03-14 Lanternfall invoice 1182.pdf. You see old and new side by side and tick the ones you want; there's an undo.", "prompts": ["Rename my scans so I can find them", "Give the PDFs in Downloads proper names"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/rename-documents"}, {"name": "disk-space-report", "title": "Disk space report", "what": "Shows where your disk space went and the safe way to get it back.", "more": "A page with what fills each drive and what each part is. It deletes nothing itself.", "prompts": ["Where did my disk space go?", "Why is C: full?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/disk-space-report"}, {"name": "backup-checkup", "title": "Backup checkup", "what": "Tells you what you'd lose if this disk died today.", "more": "Which folders and code have a second copy somewhere, and which don't. It only looks.", "prompts": ["Are my files backed up?", "What would I lose if this disk died?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/backup-checkup"}, {"name": "inbox-cleanup", "title": "Inbox cleanup", "what": "Reviews two months of Gmail and shows what to unsubscribe from.", "more": "Who fills your inbox, what you never open, likely phishing, and a plan to declutter. It reads only; nothing is deleted or sent.", "prompts": ["Clean up my Gmail", "What should I unsubscribe from?"], "has": ["Claude Code"], "page": "https://github.com/Thisisily/momputer/tree/main/skills/inbox-cleanup"}, {"name": "calendar-fixer", "title": "Calendar fixer", "what": "Finds double bookings, days with no breaks and stale recurring meetings.", "more": "It reviews the next four weeks of Google Calendar and fixes only what you tick.", "prompts": ["Fix my calendar", "Where am I double booked?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/calendar-fixer"}, {"name": "drive-audit", "title": "Drive audit", "what": "Shows what you share with anyone on Google Drive, and what fills it.", "more": "Public links, the biggest files, duplicates, and files nobody touched in a year. It only looks.", "prompts": ["Audit my Google Drive", "What am I sharing publicly?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/drive-audit"}, {"name": "subscription-audit", "title": "Subscription audit", "what": "Finds what you pay for every month and what it costs a year.", "more": "From Gmail receipts and bank statements you export: price rises, things paid twice, trials about to charge. It never cancels or pays; you do the cancelling.", "prompts": ["What am I subscribed to?", "Which free trials are about to charge me?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/subscription-audit"}, {"name": "bookmark-cleanup", "title": "Bookmark cleanup", "what": "Finds duplicate, dead and never-opened bookmarks.", "more": "It reads a copy of your bookmarks and writes a tidy file for you to import. Your browser's own files are never touched.", "prompts": ["Clean up my bookmarks", "Which of my bookmarks are dead?"], "has": [], "page": "https://github.com/Thisisily/momputer/tree/main/skills/bookmark-cleanup"}],
    skillsInstall: "/plugin install momputer-skills --marketplace Thisisily/momputer",
    skillsPage: "https://github.com/Thisisily/momputer/tree/main/skills",
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
    return { stopped: [], alreadyGone: [], failed: [], freedMem: freed, groups: gone.length, message: gone.length === 1 ? "Turned it off. That's memory back for you." : `Turned off ${gone.length} things. That's memory back for you.` };
  }

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
    clean_trash: async ({ id, pick }) => {
      const s = spots.find((x) => x.id === id);
      const pk = { goes: true, new: false, ticked: [], unticked: [], ...(pick || {}) };
      // Like mom-core's choose(), for the listed items (the mock has nothing unlisted).
      const chosen = s.items.filter((i) => {
        if (!i.goes && !i.optional) return false;
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
      emit("trash", { spots: JSON.parse(JSON.stringify(spots)), busy: false });
      const size = `${(freed / 1024 / MB).toFixed(1)} GB`;
      if (s.action === "move") {
        for (const i of goes) archive.moved.push({ from: i.path, to: `${archive.dir}\\${i.name}`, archive: archive.dir, name: i.name, spot: s.id, bytes: i.bytes, files: i.files, movedAt: Date.now() / 1000, linked: true, reachable: true });
      }
      const message =
        s.action === "recycle"
          ? `Moved ${goes.length} files (${size}) to the Recycle Bin. Changed your mind? It's all in there.`
          : s.action === "move"
            ? `Moved ${goes.length} folders (${size}) to ${archive.dir}. They still open from where they were.`
            : `Took out ${size} of trash.`;
      return { id, freed, deleted: goes.length, skipped: 0, message };
    },
    admin_check: async () => (await wait(60), null),
    app_version: () => "0.2.0",
    // "?update" in the address shows what a newer version looks like.
    get_update: async () => (await wait(300), { current: "0.2.0", latest: location.search.includes("update") ? "0.3.0" : "0.2.0", newer: location.search.includes("update"), checked: now, asked: now, problem: null, getIt: "https://thisisily.github.io/momputer/" }),
    measure_spot: async ({ id }) => {
      await wait(700);
      emit("trash", { spots: JSON.parse(JSON.stringify(spots)), busy: false });
      return id;
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
      filesReport = filesReport || mockFiles();
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
          ["Sound", "Speakers (Lanternfall Soundbar) · Speakers (Realtek(R) Audio) · 4 more made by apps or going to a screen"],
          ["Keyboard", "Copperline Apex Pro"],
          ["Mouse", "Lanternfall Basilisk X"],
          ["Board", "ASUS TUF B450-PLUS GAMING, BIOS 4604 from 2024"],
          ["Windows", "Windows 11 Pro 25H2 (build 26200)"],
        ],
        checks: [
          { id: "antivirus", area: "Security", level: "ok", title: "Microsoft Defender protects this PC", detail: "Real-time protection is on and its virus list is up to date.", fix: null },
          { id: "av-leftover-bitdefender", area: "Security", level: "tip", title: "Bitdefender's antivirus is gone, but its helpers aren't", detail: "Bitdefender Agent still runs at every start, with nothing left for it to look after. Uninstall it from Installed apps.", fix: { label: "Installed apps", kind: "open", value: "ms-settings:appsfeatures", hint: null } },
          { id: "windows-support", area: "Updates", level: "ok", title: "Windows 11 Pro 25H2 (build 26200) gets security updates", detail: "Until 12 October 2027.", fix: null },
          { id: "memory-speed", area: "Hardware", level: "warn", title: "Your memory runs at 2666 MT/s, but it's made for 3600", detail: "Crucial BL16G36C16U4BL sticks run at the safe standard speed until their profile is switched on in the BIOS (it's called D.O.C.P.). On a Ryzen, part of the processor runs at memory speed too, so it's real speed. It's one setting and costs nothing. With 4 sticks, if 3600 isn't stable, the next step down is.", fix: null },
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
        ],
      }
    ),
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
    set_big: () => null,
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

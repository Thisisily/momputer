// Exercise the real cleanup selection UI with an in-memory DOM and fake IPC only.
// No test starts momputer, touches user settings, or deletes/recycles/archives real files.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../app/ui/app.js"), "utf8");
const clickEnd = source.indexOf('  document.addEventListener("keydown",');
assert.ok(clickEnd > 0, "The real delegated click handlers must be included");
const ID = "forgotten-apps";
const copy = (value) => JSON.parse(JSON.stringify(value));
const folder = (name) => `C:\\FakeAppData\\${name}`;
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const settle = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };

function spot(names = ["delete", "unticked"], { hidden = false, extras = true } = {}) {
  const items = names.map((name) => ({
    name: `Example app · ${name}`, path: folder(name), bytes: 100, files: 1,
    ageSecs: 400 * 86400, goes: true, optional: false, kept: false,
    sub: "App data", reason: "Old app data", kind: "app data",
  }));
  if (extras) items.push(
    { name: "Other app · recent", path: folder("recent"), bytes: 200, files: 2, goes: false, optional: true, kept: false, reason: "Recent" },
    { name: "Other app · protected", path: folder("protected"), bytes: 300, files: 3, goes: false, optional: false, kept: true, reason: "You said to keep this" },
  );
  const reclaimableItems = names.length + (hidden ? 20 : 0);
  const reclaimableBytes = names.length * 100 + (hidden ? 20000 : 0);
  const keptItems = (extras ? 2 : 0) + (hidden ? 10 : 0);
  const keptBytes = (extras ? 500 : 0) + (hidden ? 10000 : 0);
  return {
    id: ID, label: "Forgotten app data", detail: "Saved data of old apps", what: "Review app data before clearing it.",
    effect: "Moves app data to your archive drive and leaves links behind.", action: "move", safety: "review", admin: false,
    exact: true, available: true, measuredAt: 1, path: null, items,
    reclaimableItems, reclaimableBytes, keptItems, keptBytes, totalBytes: reclaimableBytes + keptBytes,
    newItems: (extras ? 1 : 0) + (hidden ? 10 : 0), newBytes: (extras ? 200 : 0) + (hidden ? 10000 : 0),
  };
}

function harness(initial = spot(), { enableMutationSimulation = false } = {}) {
  const id = initial.id;
  const calls = [], messages = [], renders = [], timers = new Map();
  let nextTimer = 0, click;
  let backend = async (cmd) => { throw new Error(`Unexpected fake IPC command: ${cmd}`); };
  const elements = new Map();
  const element = (selector) => {
    if (!elements.has(selector)) elements.set(selector, { hidden: false, setAttribute() {}, querySelector: () => null });
    return elements.get(selector);
  };
  const context = vm.createContext({
    console, messages, renders,
    navigator: { clipboard: { writeText: async (text) => messages.push({ clipboard: text }) } },
    window: { momMock: {
      invoke: (cmd, args) => { calls.push({ cmd, args: args && copy(args) }); return backend(cmd, args); },
      listen: () => { throw new Error("Startup listeners are excluded from this harness"); },
    } },
    localStorage: { getItem: () => null, setItem: () => {} },
    document: {
      addEventListener: (name, fn) => { if (name === "click") click = fn; },
      querySelector: element,
    },
    setTimeout: (fn, delay) => { timers.set(++nextTimer, { fn, delay }); return nextTimer; },
    clearTimeout: (id) => timers.delete(id),
  });
  // Preserve real selection, confirmation, rendering and click-handler code. Only replace
  // DOM drawing and face/toast animation; truncate startup, so every IPC is test-driven.
  // Only fake-IPC lifecycle tests opt into an in-memory copy with the release hold lifted.
  const simulatedSource = enableMutationSimulation ? source.replace("const FILE_CHANGES_AVAILABLE = false;", "const FILE_CHANGES_AVAILABLE = true;") : source;
  vm.runInContext(simulatedSource.slice(0, simulatedSource.indexOf('  document.addEventListener("keydown",')) + `
    renderSheet = () => { if (state.sheet) renders.push(renderSpot(state.sheet.id)); };
    render = () => renderSheet();
    react = () => {};
    toast = (text, error) => messages.push({ text, error: !!error });
    globalThis.ui = { state, picked, cleanSpot, renderSpot, effectNow, pickOf, invoke, renderRoom, renderFiles, archiveBox, renderSettings, applyFiles };
  })();`, context, { filename: "app/ui/app.js" });
  const ui = context.ui;
  ui.state.tab = "upkeep";
  ui.state.sheet = { kind: "spot", id };
  ui.state.trash = { spots: [copy(initial)], busy: false };
  const current = () => ui.state.trash.spots.find((s) => s.id === id);
  return {
    ...ui, calls, messages, renders, timers, current,
    backend: (fn) => { backend = fn; },
    html: () => ui.renderSpot(id),
    selected: () => current().items.filter(i => !i.kept && (i.goes || i.optional)).filter(i => { const p = ui.pickOf(id); return (i.goes ? p.goes : p.new) ? !p.unticked.has(i.path) : p.ticked.has(i.path); }).map(i => i.path).sort(),
    reopen: () => { ui.state.sheet = { kind: "spot", id }; },
    action: (act, data = {}, checked = false) => {
      const el = { dataset: { act, ...data }, checked, closest: () => null, getAttribute: (name) => name === "aria-expanded" ? (data.expanded || "true") : null };
      return click({ target: { closest: () => el }, preventDefault() {} });
    },
    expireConfirmation: () => {
      for (const [id, timer] of timers) if (timer.delay === 6000) { timers.delete(id); timer.fn(); }
    },
  };
}

const cleanCalls = (h) => h.calls.filter((c) => c.cmd === "clean_trash");
const hasAct = (html, act) => html.includes(`data-act="${act}"`);
const tags = (html, act) => html.match(new RegExp(`<[^>]+data-act="${act}"[^>]*>`, "g")) || [];
const allDisabled = (html, act) => tags(html, act).length > 0 && tags(html, act).every((tag) => /\bdisabled\b/.test(tag));
const trash = (s) => ({ spots: [s], busy: false });
const done = (deleted = 1, skipped = 0) => ({ id: ID, deleted, skipped, freed: deleted * 100, message: "Fake cleanup finished." });
function successfulBackend(h, refreshed = spot([], { extras: false }), archive = null) {
  h.backend(async (cmd) => {
    if (cmd === "clean_trash") return done();
    if (cmd === "get_trash") return trash(refreshed);
    if (cmd === "archive_info") return archive;
    throw new Error(`Unexpected fake IPC command: ${cmd}`);
  });
}
const mode = (h, value) => h.action("forgotten-mode", { mode: value });
const clean = (h) => h.action("clean-spot", { spot: h.current().id });
const pick = (h, name, checked) => h.action("pick", { path: folder(name) }, checked);


const simulation = (initial) => harness(initial, { enableMutationSimulation: true });
test("ordinary cleanup never reselects unticked survivors or new entries after a partial delete", async () => {
  const ordinary = (names, hidden = false) => ({
    ...spot(names, { hidden, extras: false }),
    id: "old-temp", label: "Old temp files", action: "delete", safety: "safe",
  });
  const h = simulation(ordinary(["delete", "unticked"]));
  const deleting = deferred();
  h.backend((cmd) => {
    if (cmd === "clean_trash") return deleting.promise;
    throw new Error(`Unexpected fake IPC command: ${cmd}`);
  });
  await pick(h, "unticked", false);
  await clean(h);
  const pending = clean(h);
  assert.deepEqual(cleanCalls(h)[0].args.pick.unticked, [folder("unticked")]);
  // A scan event can reach the UI before the cleanup IPC reply. Its new and hidden entries
  // must not gain permission from the completed action's default selection.
  h.state.trash = trash(ordinary(["unticked", "newly-found"], true));
  deleting.resolve({ ...done(1, 1), id: "old-temp" });
  await pending;
  assert.deepEqual(h.selected(), []);
  assert.equal(h.picked(h.current()).count, 0, "hidden aggregate entries also remain unticked");
  await clean(h);
  await clean(h);
  assert.equal(cleanCalls(h).length, 1);
  // A later scan and reopening the sheet keep that choice, while an explicit tick works.
  h.state.trash = trash(ordinary(["unticked", "newly-found", "later-found"], true));
  h.reopen();
  assert.equal(h.picked(h.current()).count, 0);
  await pick(h, "unticked", true);
  assert.deepEqual(h.selected(), [folder("unticked")]);
  await clean(h);
  assert.equal(cleanCalls(h).length, 1, "choosing a survivor still needs a new confirmation");
});

test("failed, stopped, recycled, admin and archive cleanups consume selections safely", async () => {
  for (const scenario of [
    { id: "old-temp", action: "delete", failed: true },
    { id: "old-temp", action: "delete", stopped: true },
    { id: "old-temp", action: "delete", recycle: true },
    { id: "windows-junk", action: "delete", admin: true },
    { id: "recycle-bin", action: "empty-bin" },
    { id: ID, action: "move" },
  ]) {
    const initial = { ...spot(["delete", "unticked"], { extras: false }), ...scenario };
    const h = simulation(initial);
    h.state.recycle = !!scenario.recycle;
    h.backend(async (cmd) => {
      if (cmd === "archive_info") return null;
      if (cmd !== "clean_trash") throw new Error(`Unexpected fake IPC command: ${cmd}`);
      if (scenario.failed) throw new Error("Fake cleanup failure");
      return { ...done(0, 1), id: scenario.id, message: scenario.stopped ? "Stopped." : "Fake cleanup finished." };
    });
    await pick(h, "unticked", false);
    await clean(h);
    await clean(h);
    h.reopen();
    assert.equal(h.state.cleaning, null);
    assert.deepEqual(h.selected(), [], JSON.stringify(scenario));
    assert.equal(h.picked(h.current()).count, 0);
    // No scan event has arrived: even the old selected row must no longer be selected.
    assert.equal(h.current().items.length, 2);
    await clean(h);
    await clean(h);
    assert.equal(cleanCalls(h).length, 1, JSON.stringify(scenario));
  }
});



const HOLD = "General file cleanup is temporarily unavailable while its safety update is validated. Scans and previews remain available.";

test("the shipped hold rejects every file mutation IPC and forged action without reaching the bridge", async () => {
  const h = harness();
  for (const cmd of ["clean_trash", "recycle_files", "bring_back", "delete_moved", "apply_tidy", "undo_tidy"]) {
    await assert.rejects(h.invoke(cmd, {}), { message: HOLD });
  }
  for (const act of ["clean-spot", "files-recycle", "bring-back", "delete-moved", "apply-room", "undo-room"]) {
    await h.action(act, { spot: ID, from: folder("delete") });
    assert.equal(h.messages.at(-1).text, HOLD);
  }
  await h.cleanSpot(ID);
  assert.equal(h.messages.at(-1).text, HOLD);
  assert.equal(h.calls.length, 0);
  assert.equal(h.state.cleanConfirm, null);
  assert.equal(h.state.filesWorking, false);
});

test("the release renders mutation controls disabled while preserving scans, plan selection and copy chores", async () => {
  const h = harness();
  h.state.archive = { dir: "D:\\Archive", reachable: true, free: 1000, moved: [{
    spot: ID, name: "Example", from: folder("delete"), to: "D:\\Archive\\Example", reachable: true, linked: true, bytes: 100, movedAt: 1,
  }] };
  const html = h.html();
  assert.ok(html.includes(HOLD));
  for (const act of ["clean-spot", "bring-back", "delete-moved"]) assert.ok(allDisabled(html, act), act);
  assert.equal(allDisabled(html, "refresh-spot"), false);
  await pick(h, "unticked", false);
  assert.deepEqual(h.selected(), [folder("delete")]);
  const room = "C:\\Example";
  h.state.rooms[room] = { name: "Example", grade: "B", line: "Scan results", findings: [], actions: [{ id: "example", title: "Move example", reason: "Synthetic", bytes: 10 }] };
  h.state.roomSel[room] = new Set(["example"]);
  const roomHtml = h.renderRoom(room);
  assert.ok(roomHtml.includes(HOLD));
  for (const act of ["apply-room", "undo-room"]) assert.ok(allDisabled(roomHtml, act), act);
  for (const act of ["toggle-action", "room-pick-all", "copy-chores", "rescan-room"]) {
    assert.ok(hasAct(roomHtml, act), act);
    assert.equal(allDisabled(roomHtml, act), false, act);
  }
  h.state.roomOpen = room;
  h.backend(async (cmd) => {
    assert.equal(cmd, "chore_prompt");
    return "Synthetic preview only";
  });
  await h.action("copy-chores");
  assert.equal(h.messages.some(m => m.clipboard === "Synthetic preview only"), true);
});

test("Files hold leaves scan controls and explicit duplicate selections available", async () => {
  const h = harness();
  h.state.filesPlaces = [{ path: "C:\\Files", name: "Files", kind: "added", on: true }];
  h.applyFiles({ busy: false, report: {
    measuredAt: 1, scannedFiles: 2, complete: true, accessTimes: false, wasted: 10, big: [],
    groups: [{ hash: "same", size: 10, kind: "documents", files: [
      { path: "C:\\Files\\one.txt", folder: "Files", keep: true, kept: false, modified: 1 },
      { path: "C:\\Files\\two.txt", folder: "Files", keep: false, kept: false, modified: 1 },
    ] }],
  } });
  await h.action("files-pick-all");
  const html = h.renderFiles();
  assert.ok(html.includes(HOLD));
  assert.ok(allDisabled(html, "files-recycle"));
  for (const act of ["files-scan", "files-place", "files-add-place", "files-pick", "files-keep"]) {
    assert.ok(hasAct(html, act), act);
    assert.equal(allDisabled(html, act), false, act);
  }
  assert.match(html, /data-act="files-pick"[^>]*checked/);
  h.backend(async cmd => {
    if (cmd === "scan_files") return null;
    if (cmd === "files_state") return { report: null, busy: true };
    throw new Error(cmd);
  });
  await h.action("files-scan");
  assert.ok(h.calls.some(c => c.cmd === "scan_files"));
  assert.equal(h.calls.some(c => c.cmd === "recycle_files"), false);
});

test("shader vendor grouping is collapsible even for a single vendor and makes no ownership claim", () => {
  const s = { ...spot(["one", "two"], { extras: false }), id: "shader-cache", label: "Graphics shader caches", action: "delete" };
  s.items.forEach((item, i) => { item.name = "NVIDIA · DXCache " + i; });
  const h = harness(s);
  assert.match(h.html(), /data-act="toggle-group"/);
  assert.match(h.html(), /NVIDIA/);
  assert.doesNotMatch(h.html(), /owner unknown|belongs to|game ownership/);
});

test("optional archive selections describe moving with links, not permanent deletion", () => {
  const h = harness();
  const p = h.pickOf(ID);
  p.goes = false;
  p.ticked.add(folder("recent"));
  const text = h.effectNow(h.current(), h.picked(h.current()));
  assert.match(text, /Moves.*archive drive and leaves links behind/);
  assert.doesNotMatch(text, /permanently|Deletes/);
});


test("shader cards group distinct owners and preserve complete Steam game titles", async () => {
  const s = { ...spot(["one", "two", "three", "four", "five"], { extras: false }), id: "shader-cache", action: "delete" };
  const names = ["NVIDIA · game.exe · DXCache a.bin", "NVIDIA · game.exe · DXCache b.bin", "NVIDIA · App unknown · DXCache c.bin", "Steam · Counter-Strike: Source · Deluxe · Shader cache", "Steam · Steam app 999999 · Shader cache"];
  s.items.forEach((item, i) => { item.name = names[i]; item.goes = false; item.optional = true; });
  const h = harness(s);
  h.state.trash.shaderCleanup = { token: "grouping-token", files: s.items.map(i => ({ path: i.path, bytes: i.bytes })) };
  h.html();
  const html = h.html();
  for (const label of ["NVIDIA · game.exe", "NVIDIA · App unknown", "Steam · Counter-Strike: Source · Deluxe", "Steam · Steam app 999999"]) {
    assert.ok(html.includes('class="lgroup-name">' + label + '</span>'), label);
  }
  assert.equal((html.match(/class="lgroup-name"/g) || []).length, 4);
  const group = "shader-cache:stays:Steam · Counter-Strike: Source · Deluxe";
  await h.action("pick-group", { group });
  assert.deepEqual(h.selected(), [folder("four")], "selection uses the exact displayed title, including colon and separator");
  await h.action("toggle-group", { group, expanded: "true" });
  assert.equal(h.state.groupOpen[group], false);
  assert.ok(h.html().includes('data-group="' + group + '" role="button" tabindex="0" aria-expanded="false"'));
  await h.action("pick-group", { group });
  assert.deepEqual(h.selected(), []);
  await h.action("pick-group", { group: "shader-cache:stays:NVIDIA · game.exe" });
  assert.deepEqual(h.selected(), [folder("one"), folder("two")]);
  assert.equal(h.calls.length, 0, "grouping and selection are previews only");
});

test("shader group selection honors the visible filter and unknown owners stay separate", async () => {
  const s = { ...spot(["one", "two", "three"], { extras: false }), id: "shader-cache", action: "delete" };
  s.items.forEach((item, i) => { item.name = ["NVIDIA · game.exe · a.bin", "NVIDIA · game.exe · b.bin", "NVIDIA · App unknown · c.bin"][i]; item.goes = false; item.optional = true; });
  const h = harness(s);
  h.state.trash.shaderCleanup = { token: "grouping-token", files: s.items.map(i => ({ path: i.path, bytes: i.bytes })) };
  h.html();
  h.state.listView.q = "a.bin";
  await h.action("pick-group", { group: "shader-cache:stays:NVIDIA · game.exe" });
  assert.deepEqual(h.selected(), [folder("one")]);
  h.state.listView.q = "";
  await h.action("pick-group", { group: "shader-cache:stays:NVIDIA · App unknown" });
  assert.deepEqual(h.selected(), [folder("one"), folder("three")]);
});


function shaderHarness() {
  const s = { ...spot(["nvidia", "steam", "kept", "amd", "unticked"], { extras: false }), id: "shader-cache", action: "delete" };
  s.items.forEach((i, n) => { i.name = ["NVIDIA · App unknown · a.nvph", "Steam · Portal 2 · Shader cache", "NVIDIA · game.exe · kept.nvph", "AMD · App unknown · cache", "NVIDIA · game.exe · b.nvph"][n]; });
  s.items[1].goes = false;
  s.items[1].optional = true;
  s.items[2].kept = true;
  const h = harness(s);
  h.state.trash.shaderCleanup = { token: "scan-a", files: [
    { path: folder("nvidia"), bytes: 11 },
    { path: folder("steam") + "\\a.bin", bytes: 22 },
    { path: folder("steam") + "\\b.bin", bytes: 33 },
    { path: folder("kept"), bytes: 44 },
    { path: folder("unticked"), bytes: 55 },
    { path: folder("steam-sibling") + "\\outside.bin", bytes: 999 },
  ], skipped: 1 };
  h.html(); // process the frozen scan before a user can select rows
  return h;
}
const shaderCalls = h => h.calls.filter(c => c.cmd === "clean_shader_cache");
const selectShader = (h, name) => h.action("pick", { path: folder(name) }, true);

test("shader cleanup resolves only explicit eligible leaves, excludes kept and unsupported rows, and ignores Recycle Bin preference", async () => {
  const h = shaderHarness();
  h.state.recycle = true;
  assert.equal(h.picked(h.current()).count, 0, "no shader cache starts selected");
  await selectShader(h, "nvidia");
  await selectShader(h, "steam");
  await selectShader(h, "kept");
  await selectShader(h, "amd");
  const selected = h.picked(h.current());
  assert.equal(selected.count, 3);
  assert.equal(selected.bytesChosen, 66);
  assert.deepEqual(Array.from(selected.paths), [folder("nvidia"), folder("steam") + "\\a.bin", folder("steam") + "\\b.bin"]);
  const html = h.html();
  assert.match(html, /Delete 3 cache files \(66 B\)/);
  assert.match(html, /deleted permanently; folders stay/);
  assert.doesNotMatch(html, /data-act="bin-mode"/);
  assert.ok(tags(html, "pick").filter(t => t.includes('data-path="' + folder("kept") + '"') || t.includes('data-path="' + folder("amd") + '"')).every(t => /\bdisabled\b/.test(t)));
  h.backend(async cmd => {
    assert.equal(cmd, "clean_shader_cache");
    return { deleted: 2, freed: 33, skipped: 1, message: "Deleted 2 cache files; 1 changed file stayed." };
  });
  await clean(h);
  assert.equal(shaderCalls(h).length, 0);
  await clean(h);
  assert.equal(shaderCalls(h).length, 1);
  assert.deepEqual(shaderCalls(h)[0].args, { token: "scan-a", paths: Array.from(selected.paths) });
  assert.equal(cleanCalls(h).length, 0, "the legacy cleanup API is never used");
  assert.ok(h.messages.some(m => m.text === "Deleted 2 cache files; 1 changed file stayed."));
  assert.equal(h.current().items.length, 5, "no skipped or unconfirmed row is claimed removed");
  assert.equal(h.picked(h.current()).count, 0);
});

test("shader cleanup consumes selection before awaiting IPC and cannot repeat from the same snapshot", async () => {
  const h = shaderHarness();
  const pending = deferred();
  h.backend(cmd => { assert.equal(cmd, "clean_shader_cache"); return pending.promise; });
  await selectShader(h, "nvidia");
  await clean(h);
  const operation = clean(h);
  assert.equal(shaderCalls(h).length, 1);
  assert.equal(h.state.cleaning, "shader-cache");
  assert.equal(h.picked(h.current()).count, 0);
  assert.doesNotMatch(h.html(), /data-act="stop-clean"/);
  await selectShader(h, "unticked");
  await clean(h);
  assert.equal(shaderCalls(h).length, 1);
  pending.resolve({ deleted: 1, freed: 11, message: "Deleted 1 cache file." });
  await operation;
  await selectShader(h, "unticked");
  await clean(h);
  await clean(h);
  assert.equal(shaderCalls(h).length, 1);
  assert.match(h.html(), /Check again before selecting files/);
});

test("a failed shader attempt also requires a new scan and explicit selection", async () => {
  const h = shaderHarness();
  h.backend(async cmd => { assert.equal(cmd, "clean_shader_cache"); throw new Error("Scan expired"); });
  await selectShader(h, "nvidia");
  await clean(h); await clean(h);
  assert.equal(h.state.cleaning, null);
  assert.equal(h.picked(h.current()).count, 0);
  await selectShader(h, "unticked");
  await clean(h); await clean(h);
  assert.equal(shaderCalls(h).length, 1);
  assert.ok(h.messages.some(m => m.error && /Scan expired/.test(m.text)));
});

test("a refreshed or busy scan invalidates shader confirmation and does not select newly eligible files", async () => {
  const h = shaderHarness();
  await selectShader(h, "nvidia"); await clean(h);
  assert.equal(h.state.cleanConfirm, "shader-cache");
  h.state.trash.shaderCleanup = { token: "scan-b", files: [{ path: folder("nvidia"), bytes: 12 }, { path: folder("unticked"), bytes: 55 }], skipped: 0 };
  await clean(h);
  assert.equal(h.state.cleanConfirm, null);
  assert.equal(h.picked(h.current()).count, 0);
  assert.equal(shaderCalls(h).length, 0);
  await selectShader(h, "unticked"); await clean(h);
  h.state.trash.busy = true;
  await clean(h);
  assert.equal(h.state.cleanConfirm, null);
  assert.equal(shaderCalls(h).length, 0);
  h.state.trash.busy = false;
  await selectShader(h, "nvidia");
  h.backend(async cmd => { assert.equal(cmd, "clean_shader_cache"); return { deleted: 1, message: "Deleted." }; });
  await clean(h);
  assert.equal(shaderCalls(h).length, 0);
  await clean(h);
  assert.deepEqual(shaderCalls(h)[0].args, { token: "scan-b", paths: [folder("nvidia")] });
});

test("a kept descendant excludes its files even when the Steam parent is selected", async () => {
  const h = shaderHarness();
  const parent = folder("steam"), kept = parent + "\\kept";
  h.current().items.push({ ...h.current().items[2], path: kept });
  h.state.trash.shaderCleanup = { token: "kept-child", files: [
    { path: kept + "\\stay.bin", bytes: 50 }, { path: parent + "\\kept-neighbor\\allowed.bin", bytes: 70 },
  ], skipped: 0 };
  await selectShader(h, "steam");
  assert.equal(h.picked(h.current()).bytesChosen, 70);
  assert.deepEqual(Array.from(h.picked(h.current()).paths), [parent + "\\kept-neighbor\\allowed.bin"]);
});

test("shader tick-all selects only displayed eligible rows and never hidden snapshot files", async () => {
  const h = shaderHarness();
  await h.action("pick-all", { list: "goes" });
  assert.equal(h.picked(h.current()).count, 2);
  assert.equal(h.picked(h.current()).bytesChosen, 66);
  await h.action("pick-all", { list: "new" });
  assert.equal(h.picked(h.current()).count, 4);
  assert.equal(h.picked(h.current()).bytesChosen, 121);
  await h.action("pick-all", { list: "goes" });
  assert.equal(h.picked(h.current()).count, 2);
  assert.equal(h.picked(h.current()).bytesChosen, 55);
});


test("shader selection never merges case-distinct filesystem paths", async () => {
  const h = shaderHarness();
  const displayed = folder("nvidia"), different = folder("NVIDIA");
  h.current().items.push({ ...h.current().items[0], path: different, name: "NVIDIA · other.exe · distinct.nvph" });
  h.state.trash.shaderCleanup = { token: "case-sensitive", files: [{ path: displayed, bytes: 11 }, { path: different, bytes: 99 }], skipped: 0 };
  await selectShader(h, "nvidia");
  assert.deepEqual(Array.from(h.picked(h.current()).paths), [displayed]);
  assert.equal(h.picked(h.current()).bytesChosen, 11);
});

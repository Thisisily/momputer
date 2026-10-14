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

function harness(initial = spot()) {
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
  vm.runInContext(source.slice(0, clickEnd) + `
    renderSheet = () => { if (state.sheet) renders.push(renderSpot(state.sheet.id)); };
    render = () => renderSheet();
    react = () => {};
    toast = (text, error) => messages.push({ text, error: !!error });
    globalThis.ui = { state, spotNow, picked, cleanSpot, renderSpot, effectNow, exactChoices, pickOf };
  })();`, context, { filename: "app/ui/app.js" });
  const ui = context.ui;
  ui.state.tab = "upkeep";
  ui.state.sheet = { kind: "spot", id };
  ui.state.trash = { spots: [copy(initial)], busy: false };
  const current = () => ui.spotNow(ui.state.trash.spots.find((s) => s.id === id));
  return {
    ...ui, calls, messages, renders, timers, current,
    backend: (fn) => { backend = fn; },
    html: () => ui.renderSpot(id),
    selected: () => Array.from(ui.exactChoices(current(), ui.picked(current()))),
    reopen: () => { ui.state.sheet = { kind: "spot", id }; },
    action: (act, data = {}, checked = false) => {
      const el = { dataset: { act, ...data }, checked, closest: () => null };
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

test("three app-data actions are visible; delete and recycle do not need an archive directory", async () => {
  const h = harness();
  assert.equal(h.state.forgottenMode, "move");
  assert.equal(tags(h.html(), "forgotten-mode").length, 3);
  for (const label of ["Archive", "Recycle Bin", "Delete for good"]) assert.ok(h.html().includes(label));
  assert.equal(hasAct(h.html(), "choose-archive"), true);
  for (const action of ["delete", "recycle"]) {
    await mode(h, action);
    assert.equal(h.current().action, "delete");
    assert.equal(hasAct(h.html(), "choose-archive"), false);
    assert.equal(hasAct(h.html(), "clean-spot"), true);
    assert.equal(allDisabled(h.html(), "clean-spot"), false);
    const effect = h.effectNow(h.current(), h.picked(h.current()));
    assert.match(effect, action === "delete" ? /Permanently deletes.*saved settings and history.*cannot be undone/ : /Recycle Bin.*saved settings and history.*restore/);
  }
  assert.equal(h.state.trash.spots[0].action, "move", "view mode does not mutate the archive definition");
  assert.equal(h.calls.length, 0);
});

test("disposal sends only checked listed folders, excluding hidden aggregates, kept and stale paths", async () => {
  const h = harness(spot(undefined, { hidden: true }));
  await mode(h, "delete");
  await pick(h, "unticked", false);
  await h.action("pick-all", { list: "new" });
  h.pickOf(ID).ticked.add(folder("stale-not-in-report"));
  const p = h.picked(h.current());
  assert.equal(p.count, 2);
  assert.equal(p.bytesChosen, 300);
  assert.match(h.html(), /not included; show them to select them/);
  successfulBackend(h);
  await clean(h);
  assert.equal(cleanCalls(h).length, 0);
  await clean(h);
  assert.deepEqual(cleanCalls(h)[0].args, { id: ID, pick: {
    goes: false, new: false, ticked: [folder("delete"), folder("recent")], unticked: [],
    discard: true, recycle: false, keep: [folder("protected")],
  } });
});

test("mode and selection changes require a fresh first press and matching second confirmation", async () => {
  const h = harness();
  successfulBackend(h);
  await mode(h, "delete");
  await clean(h);
  await mode(h, "recycle");
  assert.equal(h.state.cleanConfirm, null);
  await clean(h);
  assert.equal(cleanCalls(h).length, 0);
  await pick(h, "unticked", false);
  assert.equal(h.state.cleanConfirm, null);
  await clean(h);
  assert.equal(cleanCalls(h).length, 0);
  await clean(h);
  assert.equal(cleanCalls(h).length, 1);
  assert.equal(cleanCalls(h)[0].args.pick.recycle, true);
  assert.deepEqual(cleanCalls(h)[0].args.pick.ticked, [folder("delete")]);
});

test("a refreshed selection and an expired confirmation cannot reuse an earlier approval", async () => {
  const h = harness();
  successfulBackend(h);
  await mode(h, "delete");
  await clean(h);
  h.state.trash = trash(spot(["delete", "unticked", "newly-listed"]));
  await clean(h);
  assert.equal(cleanCalls(h).length, 0, "new displayed paths require their own confirmation");
  h.expireConfirmation();
  await clean(h);
  assert.equal(cleanCalls(h).length, 0, "expired approval requires a new first press");
});

test("unticked survivors stay unticked after disposal and repeated presses cannot select them", async () => {
  const h = harness();
  successfulBackend(h, spot(["unticked"]));
  await mode(h, "delete");
  await pick(h, "unticked", false);
  await clean(h);
  await clean(h);
  h.reopen();
  assert.deepEqual(h.selected(), []);
  assert.equal(h.picked(h.current()).count, 0);
  assert.match(h.html(), /Nothing ticked/);
  await clean(h);
  await clean(h);
  assert.equal(cleanCalls(h).length, 1);
  await pick(h, "unticked", true);
  await clean(h);
  assert.equal(cleanCalls(h).length, 1, "explicitly choosing a survivor still needs two presses");
});

test("mode and pick controls stay disabled and ignore repeated actions through delete and refresh", async () => {
  const initial = spot(["delete", "unticked", "other"]);
  initial.items[2].name = "Different app · other";
  const h = harness(initial);
  const deleting = deferred(), refreshing = deferred();
  h.backend((cmd) => {
    if (cmd === "clean_trash") return deleting.promise;
    if (cmd === "get_trash") return refreshing.promise;
    throw new Error(`Unexpected fake IPC command: ${cmd}`);
  });
  await mode(h, "delete");
  await pick(h, "unticked", false);
  await clean(h);
  const pending = clean(h);
  for (const act of ["forgotten-mode", "pick", "pick-all", "pick-group", "keep-item", "refresh-spot"]) {
    assert.equal(allDisabled(h.html(), act), true, `${act} is disabled while deleting`);
  }
  const before = h.selected();
  await mode(h, "recycle");
  await pick(h, "unticked", true);
  await h.action("pick-all", { list: "goes" });
  await h.action("pick-group", { group: `${ID}:goes:Example app` });
  await h.action("keep-item", { spot: ID, path: folder("delete") });
  await clean(h);
  assert.equal(h.state.forgottenMode, "delete");
  assert.deepEqual(h.selected(), before, "disabled selection handlers cannot change pending choices");
  assert.equal(cleanCalls(h).length, 1);
  deleting.resolve(done());
  await settle();
  assert.equal(h.state.cleaning, ID, "refresh keeps the cleanup locked");
  assert.deepEqual(h.selected(), []);
  for (const act of ["forgotten-mode", "pick", "pick-all", "pick-group"]) assert.equal(allDisabled(h.html(), act), true);
  await mode(h, "move");
  await pick(h, "unticked", true);
  await clean(h);
  assert.equal(h.state.forgottenMode, "delete");
  assert.deepEqual(h.selected(), []);
  assert.equal(cleanCalls(h).length, 1);
  refreshing.resolve(trash(spot(["unticked"])));
  await pending;
  assert.equal(h.state.cleaning, null);
  assert.deepEqual(h.selected(), []);
});

test("Recycle Bin disposal sets the backend flag without modifying the saved archive", async () => {
  const h = harness(spot(["delete"], { extras: false }));
  const archive = { dir: "F:\\FakeArchive", reachable: true, free: 10000, moved: [{ from: folder("already-archived"), spot: ID }] };
  h.state.archive = copy(archive);
  successfulBackend(h);
  await mode(h, "recycle");
  await clean(h);
  await clean(h);
  assert.equal(cleanCalls(h)[0].args.pick.discard, true);
  assert.equal(cleanCalls(h)[0].args.pick.recycle, true);
  assert.deepEqual(copy(h.state.archive), archive);
  assert.equal(h.calls.some((call) => ["choose_archive_dir", "archive_info", "delete_moved", "bring_back"].includes(call.cmd)), false);
});

test("returning to Archive preserves the existing move request and refreshes archive information", async () => {
  const h = harness(spot(["delete"], { extras: false }));
  const archive = { dir: "F:\\FakeArchive", reachable: true, free: 10000, moved: [] };
  h.state.archive = copy(archive);
  successfulBackend(h, spot([], { extras: false }), archive);
  await mode(h, "delete");
  await mode(h, "move");
  assert.equal(h.current().action, "move");
  assert.match(h.effectNow(h.current(), h.picked(h.current())), /archive drive.*links/);
  await clean(h);
  await clean(h);
  await settle();
  assert.deepEqual(cleanCalls(h)[0].args, { id: ID, pick: null });
  assert.equal(h.calls.filter((call) => call.cmd === "archive_info").length, 1);
  assert.equal(h.calls.some((call) => call.cmd === "get_trash"), false);
});

test("a failed disposal and refresh release the lock without reselecting app-data folders", async () => {
  const h = harness();
  h.backend(async () => { throw new Error("Fake IPC failure"); });
  await mode(h, "delete");
  await pick(h, "unticked", false);
  await clean(h);
  await clean(h);
  assert.equal(h.state.cleaning, null);
  assert.equal(h.state.cleanConfirm, null);
  assert.deepEqual(h.selected(), []);
  assert.equal(h.state.trash.spots[0].items.length, 4);
  assert.equal(h.messages.some((m) => m.error), true);
  await clean(h);
  await clean(h);
  assert.equal(cleanCalls(h).length, 1);
});

test("ordinary cleanup never reselects unticked survivors or new entries after a partial delete", async () => {
  const ordinary = (names, hidden = false) => ({
    ...spot(names, { hidden, extras: false }),
    id: "old-temp", label: "Old temp files", action: "delete", safety: "safe",
  });
  const h = harness(ordinary(["delete", "unticked"]));
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
    const h = harness(initial);
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

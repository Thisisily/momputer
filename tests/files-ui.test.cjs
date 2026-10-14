// Run the real Files UI and its delegated handlers with fake IPC. These tests never recycle
// files, start momputer, or read/write user settings; no browser or extra dependency is needed.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../app/ui/app.js"), "utf8");
const clickEnd = source.indexOf('  document.addEventListener("keydown",');
const eventsStart = source.indexOf('    bridge.listen("files-progress",');
const eventsEnd = source.indexOf('    bridge.listen("trash",', eventsStart);
assert.ok(clickEnd > 0 && eventsStart > clickEnd && eventsEnd > eventsStart, "Files UI handlers must be included in the harness");

const copy = (value) => JSON.parse(JSON.stringify(value));
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

function report(names = ["original", "delete", "unticked"], big = []) {
  return {
    measuredAt: 1, scannedFiles: names.length + big.length, complete: true, accessTimes: false,
    wasted: Math.max(0, names.length - 1) * 100,
    groups: names.length > 1 ? [{
      hash: "same-content", size: 100, kind: "documents",
      files: names.map((name, i) => ({ path: `C:\\Files\\${name}.txt`, folder: "Files", modified: 1, keep: i === 0, kept: false })),
    }] : [],
    big: big.map((name) => ({ path: `C:\\Files\\${name}.zip`, size: 1000, folder: "Files", modified: 1, accessed: 1, recency: 1, kept: false })),
  };
}

function harness(initial = report()) {
  const calls = [], messages = [], renders = [];
  const events = new Map(), timers = new Map();
  let nextTimer = 0, click;
  let backend = async (cmd) => {
    if (cmd === "files_state") return { report: copy(initial), busy: false };
    throw new Error(`Unexpected command: ${cmd}`);
  };
  const context = vm.createContext({
    console, messages, renders,
    window: { momMock: {
      invoke: (cmd, args) => { calls.push({ cmd, args: args && copy(args) }); return backend(cmd, args); },
      listen: (name, fn) => events.set(name, fn),
    } },
    localStorage: { getItem: () => null },
    document: { addEventListener: (name, fn) => { if (name === "click") click = fn; } },
    setTimeout: (fn) => { timers.set(++nextTimer, fn); return nextTimer; },
    clearTimeout: (id) => timers.delete(id),
  });
  // Keep all real definitions and the complete click handler. Replace only drawing outside
  // Files and startup (which would inspect the PC); register the actual Files event handlers.
  vm.runInContext(source.slice(0, clickEnd) + `
    render = () => renders.push(renderFiles());
    react = () => {};
    toast = (text, error) => messages.push({ text, error: !!error });
    ${source.slice(eventsStart, eventsEnd)}
    globalThis.ui = { state, applyFiles, loadFiles, filesChosen, renderFiles, keptPath };
  })();`, context, { filename: "app/ui/app.js" });
  const ui = context.ui;
  ui.state.tab = "upkeep";
  ui.state.upkeepTab = "files";
  ui.state.filesPlaces = [{ path: "C:\\Files", name: "Files", on: true, kind: "added" }];
  ui.applyFiles({ report: copy(initial), busy: false });
  return {
    ...ui, calls, messages, renders, timers,
    backend: (fn) => { backend = fn; },
    emit: (name, payload) => events.get(name)({ payload: copy(payload) }),
    chosen: () => Array.from(ui.filesChosen(ui.state.files.report).paths),
    action: (act, data = {}, checked = false) => {
      const el = { dataset: { act, ...data }, checked, closest: () => null };
      return click({ target: { closest: () => el }, preventDefault: () => {} });
    },
    expireConfirmation: () => { for (const fn of timers.values()) fn(); timers.clear(); },
  };
}

const file = (name) => `C:\\Files\\${name}.txt`;
const moved = (...names) => ({ recycled: names.map(file), skipped: [], freed: names.length * 100, message: "Moved." });
const recycleCalls = (h) => h.calls.filter((c) => c.cmd === "recycle_files");
const hasDisabled = (html, act) => new RegExp(`<[^>]+data-act="${act}"[^>]*\\bdisabled\\b`).test(html);

test("same-folder and cross-folder duplicates all start unticked and explain reference uncertainty", async () => {
  const r = report();
  const other = copy(report(["saved", "saved (2)"]).groups[0]);
  other.hash = "cross-folder-content";
  other.files[0].path = "C:\\Documents\\saved.pdf";
  other.files[1].path = "D:\\Archive\\saved (2).pdf";
  r.groups.push(other);
  const h = harness(r);
  assert.deepEqual(h.chosen(), []);
  assert.doesNotMatch(h.renderFiles(), /data-act="files-pick"[^>]*\bchecked\b/);
  assert.match(h.renderFiles(), /Nothing ticked/);
  assert.match(h.renderFiles(), /A higher number in a name does not prove a newer version/);
  assert.match(h.renderFiles(), /still needs a particular name or location/);
  assert.doesNotMatch(h.renderFiles(), /never sends the last copy/);
  await h.action("files-recycle");
  await h.action("files-recycle");
  assert.deepEqual(recycleCalls(h), []);
});

test("tick-all changes only shown groups, and previously chosen hidden copies remain explicit", async () => {
  const r = report();
  r.groups = Array.from({ length: 22 }, (_, i) => ({
    hash: `content-${i}`, size: 100, kind: "documents",
    files: ["keep", "extra"].map((name, j) => ({ path: `C:\\Files\\${i}-${name}.txt`, folder: "Files", modified: 1, keep: j === 0, kept: false })),
  }));
  const h = harness(r);
  assert.match(h.renderFiles(), /Tick all shown/);
  assert.match(h.renderFiles(), /Only changes extra copies in the 20 groups shown/);
  await h.action("files-pick-all");
  assert.equal(h.chosen().length, 20);
  assert.equal(h.chosen().includes("C:\\Files\\20-extra.txt"), false);
  await h.action("files-pick", { path: "C:\\Files\\20-extra.txt" }, true);
  assert.equal(h.chosen().length, 20, "a stale or hidden row cannot be picked");
  await h.action("toggle-copies");
  await h.action("files-pick-all");
  assert.equal(h.chosen().length, 22);
  await h.action("toggle-copies");
  assert.match(h.renderFiles(), /2 previously selected copies in hidden groups are still ticked/);
  await h.action("files-pick-all");
  assert.deepEqual(h.chosen(), ["C:\\Files\\20-extra.txt", "C:\\Files\\21-extra.txt"]);
  await h.action("files-pick-all");
  assert.equal(h.chosen().length, 22, "ticking visible copies preserves earlier explicit hidden choices");
});

test("keeper swaps never select the former keeper and a changed content group needs a new pick", async () => {
  const h = harness(report(["original", "delete", "keeper"]));
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-keep", { hash: "same-content", path: file("delete") });
  assert.deepEqual(h.chosen(), []);
  await h.action("files-keep", { hash: "same-content", path: file("keeper") });
  assert.deepEqual(h.chosen(), [], "neither former keeper is silently selected");
  await h.action("files-pick", { path: file("original") }, true);
  const refreshed = report(["original", "delete", "keeper", "new"]);
  h.emit("files", { report: refreshed, busy: false });
  assert.deepEqual(h.chosen(), [file("original")]);
  refreshed.groups[0].hash = "different-content-at-the-same-paths";
  refreshed.groups[0].files[0].keep = false;
  refreshed.groups[0].files[2].keep = true;
  h.emit("files", { report: refreshed, busy: false });
  assert.deepEqual(h.chosen(), [], "the old path selection does not authorize different contents");
  h.emit("files", { report: report(["original", "delete", "keeper", "new"]), busy: false });
  assert.deepEqual(h.chosen(), [], "returning to the earlier hash does not revive its old selection");
});

test("missing copies and big files lose their ticks; surviving choices persist and returning files stay unticked", async () => {
  const initial = report(["original", "missing", "remaining"], ["missing-big", "remaining-big"]);
  const h = harness(initial);
  for (const name of ["missing", "remaining"]) await h.action("files-pick", { path: file(name) }, true);
  for (const name of ["missing-big", "remaining-big"]) await h.action("big-pick", { path: `C:\\Files\\${name}.zip` }, true);
  assert.equal(h.chosen().length, 4);
  h.emit("files", { report: report(["original", "remaining"], ["remaining-big"]), busy: false });
  assert.deepEqual(h.chosen(), [file("remaining"), "C:\\Files\\remaining-big.zip"]);
  assert.equal(h.state.filesTicked.has(file("missing")), false);
  assert.equal(h.state.bigTicked.has("C:\\Files\\missing-big.zip"), false);
  h.emit("files", { report: initial, busy: false });
  assert.deepEqual(h.chosen(), [file("remaining"), "C:\\Files\\remaining-big.zip"], "returned paths require a fresh explicit choice");
});

test("an unavailable report clears duplicate and big-file selections before they can return", async () => {
  const initial = report(undefined, ["big"]);
  const h = harness(initial);
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("big-pick", { path: "C:\\Files\\big.zip" }, true);
  h.emit("files", { report: null, busy: false });
  assert.equal(h.state.filesTicked.size, 0);
  assert.equal(h.state.bigTicked.size, 0);
  h.emit("files", { report: initial, busy: false });
  assert.deepEqual(h.chosen(), []);
});

test("a refresh that protects a selected file clears its tick instead of reviving it later", async () => {
  const h = harness();
  await h.action("files-pick", { path: file("delete") }, true);
  const protectedReport = report();
  protectedReport.groups[0].files[1].kept = true;
  h.emit("files", { report: protectedReport, busy: false });
  assert.deepEqual(h.chosen(), []);
  h.emit("files", { report: report(), busy: false });
  assert.deepEqual(h.chosen(), [], "removing a protection does not become an automatic delete choice");
});

test("recycling selected copies preserves explicit unticks and requires a new choice before another delete", async () => {
  const h = harness();
  h.backend(async (cmd) => cmd === "recycle_files" ? moved("delete") : { report: report(["original", "unticked"]), busy: false });
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-pick", { path: file("unticked") }, false);
  await h.action("files-recycle");
  await h.action("files-recycle");
  assert.deepEqual(recycleCalls(h)[0].args.paths, [file("delete")]);
  assert.deepEqual(h.chosen(), []);
  assert.equal(h.state.filesTicked.has(file("unticked")), false);
  assert.equal(h.state.filesConfirm, null);
  assert.match(h.renderFiles(), /Nothing ticked/);
  await h.action("files-recycle");
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 1, "repeated presses cannot take a previously unticked survivor");
  await h.action("files-pick", { path: file("unticked") }, true);
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 1, "a new selection needs its own confirmation");
});

test("delete and refresh both lock controls, reject repeated submissions, and remove confirmed successes immediately", async () => {
  const h = harness(report(undefined, ["big"]));
  const recycling = deferred(), refreshing = deferred();
  h.backend((cmd) => cmd === "recycle_files" ? recycling.promise : refreshing.promise);
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-pick", { path: file("unticked") }, false);
  await h.action("files-recycle");
  const pending = h.action("files-recycle");
  for (const act of ["files-pick", "files-keep", "big-pick", "files-pick-all", "big-pick-all", "files-scan", "files-place", "files-add-place", "files-recycle"]) {
    assert.equal(hasDisabled(h.renderFiles(), act), true, `${act} is disabled while recycling`);
  }
  await h.action("files-pick", { path: file("unticked") }, true);
  await h.action("files-pick-all");
  await h.action("files-keep", { hash: "same-content", path: file("unticked") });
  await h.action("big-pick", { path: "C:\\Files\\big.zip" }, true);
  await h.action("files-scan");
  await h.action("files-recycle");
  assert.equal(h.state.filesTicked.has(file("unticked")), false);
  assert.equal(h.state.filesKeep["same-content"], undefined);
  assert.equal(h.state.bigTicked.size, 0);
  assert.equal(h.calls.some((c) => c.cmd === "scan_files"), false);
  recycling.resolve(moved("delete"));
  await settle();
  assert.equal(h.state.filesWorking, true, "still locked while files_state is pending");
  assert.equal(h.state.files.report.groups[0].files.some((f) => f.path === file("delete")), false);
  assert.deepEqual(h.chosen(), []);
  assert.equal(hasDisabled(h.renderFiles(), "files-pick"), true);
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 1);
  refreshing.resolve({ report: report(["original", "unticked"], ["big"]), busy: false });
  await pending;
  assert.equal(h.state.filesWorking, false);
  assert.deepEqual(h.chosen(), []);
});

test("partial success keeps the chosen keeper and unticks; skipped selected files remain available", async () => {
  const h = harness(report(["original", "delete", "unticked", "keeper"], ["big"]));
  await h.action("files-pick", { path: file("unticked") }, false);
  await h.action("files-keep", { hash: "same-content", path: file("keeper") });
  await h.action("files-pick", { path: file("original") }, true);
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("big-pick", { path: "C:\\Files\\big.zip" }, true);
  h.backend(async (cmd) => cmd === "recycle_files"
    ? { ...moved("delete"), skipped: [{ path: file("original"), why: "Open" }, { path: "C:\\Files\\big.zip", why: "Open" }] }
    : { report: report(["original", "unticked", "keeper"], ["big"]), busy: false });
  await h.action("files-recycle");
  await h.action("files-recycle");
  assert.equal(h.keptPath(h.state.files.report.groups[0]), file("keeper"));
  assert.deepEqual(h.chosen(), [file("original"), "C:\\Files\\big.zip"]);
  assert.equal(h.state.filesTicked.has(file("unticked")), false);
});

test("a rejected recycle and a failed refresh preserve the current choices and release the lock", async () => {
  const h = harness();
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-pick", { path: file("unticked") }, false);
  h.backend(async () => { throw new Error("simulated IPC failure"); });
  await h.action("files-recycle");
  await h.action("files-recycle");
  assert.equal(h.state.filesWorking, false);
  assert.equal(h.state.filesConfirm, null);
  assert.deepEqual(h.chosen(), [file("delete")]);
  assert.equal(h.state.files.report.groups[0].files.length, 3);
  assert.equal(h.messages.filter((m) => m.error).length, 2);
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 1, "retry first asks for confirmation again");
});

test("an older files_state response cannot restore deleted rows or re-arm a confirmation", async () => {
  const h = harness();
  const oldRead = deferred();
  let reads = 0;
  h.backend((cmd) => {
    if (cmd === "recycle_files") return Promise.resolve(moved("delete"));
    return ++reads === 1 ? oldRead.promise : Promise.resolve({ report: report(["original", "unticked"]), busy: false });
  });
  const old = h.loadFiles();
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-pick", { path: file("unticked") }, false);
  await h.action("files-recycle");
  await h.action("files-recycle");
  oldRead.resolve({ report: report(), busy: false });
  await old;
  assert.equal(h.state.files.report.groups[0].files.length, 2);
  assert.deepEqual(h.chosen(), []);
  assert.equal(h.state.filesConfirm, null);
});

test("fresh scans preserve explicit choices, invalidate confirmation and leave new files unticked", async () => {
  const h = harness();
  h.backend(async (cmd) => { assert.equal(cmd, "scan_files"); });
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-pick", { path: file("unticked") }, false);
  await h.action("files-recycle");
  await h.action("files-scan");
  assert.equal(h.state.filesConfirm, null);
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 0);
  h.emit("files-progress", { stage: "looking", files: 4, bytes: 400 });
  h.emit("files", { report: report(["original", "delete", "unticked", "new"]), busy: false });
  assert.deepEqual(h.chosen(), [file("delete")]);
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 0, "a fresh report needs a new first press");
  h.emit("files", { report: report(["original", "delete", "unticked"]), busy: false });
  assert.equal(h.state.filesConfirm, null, "a report arriving while armed cancels it");
});

test("selection changes and the confirmation timeout each require a fresh first press", async () => {
  const h = harness();
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-pick", { path: file("unticked") }, true);
  await h.action("files-recycle");
  await h.action("files-pick", { path: file("unticked") }, false);
  assert.equal(h.state.filesConfirm, null);
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 0);
  h.expireConfirmation();
  assert.equal(h.state.filesConfirm, null);
  await h.action("files-recycle");
  assert.equal(recycleCalls(h).length, 0);
});

test("tick-all in a smaller report and changing keeper do not discard earlier explicit unticks", async () => {
  const h = harness(report(["original", "delete", "unticked", "keeper"]));
  await h.action("files-pick", { path: file("unticked") }, false);
  await h.action("files-keep", { hash: "same-content", path: file("unticked") });
  await h.action("files-keep", { hash: "same-content", path: file("keeper") });
  assert.equal(h.chosen().includes(file("unticked")), false);
  h.emit("files", { report: report(["original", "delete"]), busy: false });
  assert.equal(h.keptPath(h.state.files.report.groups[0]), file("original"), "a missing keeper falls back to a copy that exists");
  await h.action("files-pick-all");
  await h.action("files-pick-all");
  h.emit("files", { report: report(["original", "delete", "unticked", "keeper"]), busy: false });
  assert.equal(h.chosen().includes(file("unticked")), false);
  assert.equal(h.keptPath(h.state.files.report.groups[0]), file("keeper"));
});

test("files events cannot replace an in-flight recycle list", async () => {
  const h = harness();
  const recycling = deferred();
  h.backend((cmd) => cmd === "recycle_files" ? recycling.promise : Promise.resolve({ report: report(["original", "unticked"]), busy: false }));
  await h.action("files-pick", { path: file("delete") }, true);
  await h.action("files-pick", { path: file("unticked") }, false);
  await h.action("files-recycle");
  const pending = h.action("files-recycle");
  h.emit("files-progress", { stage: "looking", files: 5, bytes: 500 });
  h.emit("files", { report: report(["original", "delete", "unticked", "late"]), busy: false });
  assert.equal(h.state.files.report.groups[0].files.length, 3);
  assert.equal(h.state.files.busy, false);
  recycling.resolve(moved("delete"));
  await pending;
  assert.deepEqual(h.chosen(), []);
});

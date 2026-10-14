// Run real rendering/click handlers with fake IPC only. These tests never open Explorer,
// start momputer, or inspect/change any real Recycle Bin or user settings.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../app/ui/app.js"), "utf8");
const clickEnd = source.indexOf('  document.addEventListener("keydown",');
assert.ok(clickEnd > 0, "The real delegated click handlers must be included");

const binSpot = (root = "C:\\$Recycle.Bin") => ({
  id: "recycle-bin", label: "Recycle Bin", group: "windows", action: "empty-bin", safety: "review",
  what: "Files you deleted.", effect: "Empties the Recycle Bin.", exact: true, available: true,
  measuredAt: 1, path: root, reclaimableItems: 1, reclaimableBytes: 100, totalBytes: 100,
  keptItems: 0, keptBytes: 0, newItems: 0, newBytes: 0,
  items: [{ name: "original report.pdf", sub: "from C:\\Documents", path: "C:/$Recycle.Bin/S-1-5-21-fake/$R123.pdf", bytes: 100, files: 1, ageSecs: 86400, ageKnown: true, goes: true, optional: false, reason: "Deleted 1 day ago" }],
});

function harness(spot = binSpot()) {
  const calls = [];
  let click;
  const context = vm.createContext({
    console,
    window: { momMock: { invoke: async (cmd, args) => {
      assert.ok(["open_recycle_bin", "reveal"].includes(cmd), `Unexpected fake IPC: ${cmd}`);
      calls.push({ cmd, args: args && JSON.parse(JSON.stringify(args)) });
      return null;
    } } },
    localStorage: { getItem: () => null },
    document: { addEventListener: (name, fn) => { if (name === "click") click = fn; } },
    setTimeout: () => 1, clearTimeout() {},
  });
  vm.runInContext(source.slice(0, clickEnd) + `
    toast = () => {};
    globalThis.ui = { state, renderSpot, itemRow };
  })();`, context, { filename: "app/ui/app.js" });
  const ui = context.ui;
  ui.state.trash = { spots: [spot], busy: false };
  ui.state.sheet = { kind: "spot", id: spot.id };
  return {
    ...ui, calls,
    html: () => ui.renderSpot(spot.id),
    action: (act, data = {}) => {
      const el = { dataset: { act, ...data }, closest: () => null };
      return click({ target: { closest: () => el }, preventDefault() {} });
    },
  };
}

test("Bin spot and item open the shell view, with or without a stored root path", () => {
  for (const root of ["C:\\$Recycle.Bin", null]) {
    const h = harness(binSpot(root));
    const html = h.html();
    assert.equal((html.match(/data-act="open-recycle-bin"/g) || []).length, 2);
    assert.match(html, /Open Recycle Bin to find original report\.pdf/);
    assert.match(html, />Open Recycle Bin<|Open Recycle Bin<\/button>/);
    assert.doesNotMatch(html, /data-act="reveal-path"/);
    assert.match(html, /original report\.pdf/);
    assert.doesNotMatch(html, />[^<]*\$R123\.pdf/);
  }
});

test("Bin item can open the shell view even when no raw storage path is available", () => {
  const s = binSpot(null);
  s.items[0].path = null;
  const h = harness(s);
  const html = h.itemRow(s, s.items[0], false);
  assert.match(html, /data-act="open-recycle-bin"/);
  assert.doesNotMatch(html, /data-act="reveal-path"/);
});

test("Open Recycle Bin sends only its dedicated fake IPC command", async () => {
  const h = harness();
  await h.action("open-recycle-bin", { path: "C:\\$Recycle.Bin\\S-1-5-21-fake\\$R123.pdf" });
  assert.deepEqual(h.calls, [{ cmd: "open_recycle_bin", args: undefined }]);
});

test("normal files still reveal their original path", async () => {
  const s = { ...binSpot(), id: "temp", action: "delete" };
  s.items[0].path = "C:\\Documents\\original report.pdf";
  const h = harness(s);
  assert.match(h.itemRow(s, s.items[0], true), /data-act="reveal-path"/);
  assert.doesNotMatch(h.itemRow(s, s.items[0], true), /data-act="open-recycle-bin"/);
  await h.action("reveal-path", { path: s.items[0].path });
  assert.deepEqual(h.calls, [{ cmd: "reveal", args: { path: s.items[0].path } }]);
});

test("mock has realistic Bin storage paths and a harmless shell-view handler", async () => {
  const mockSource = fs.readFileSync(path.join(__dirname, "../app/ui/mock.js"), "utf8");
  const context = vm.createContext({ window: {}, location: { search: "", origin: "http://mock.invalid" }, setTimeout: () => 1, clearTimeout() {} });
  vm.runInContext(mockSource, context, { filename: "app/ui/mock.js" });
  const bin = (await context.window.momMock.invoke("get_trash")).spots.find((s) => s.id === "recycle-bin");
  assert.equal(bin.path, "C:\\$Recycle.Bin");
  assert.equal(bin.items.length, 3);
  assert.ok(bin.items.every((i) => /^[CD]:\\\$Recycle\.Bin\\.*\\\$R/.test(i.path)));
  assert.equal(await context.window.momMock.invoke("open_recycle_bin"), null);
});


test("the demo holds Room mutations while its other scan handlers remain available", async () => {
  const mockSource = fs.readFileSync(path.join(__dirname, "../app/ui/mock.js"), "utf8");
  const context = vm.createContext({ window: {}, location: { search: "", origin: "http://mock.invalid" }, setTimeout: () => 1, clearTimeout() {} });
  vm.runInContext(mockSource, context);
  for (const cmd of ["apply_tidy", "undo_tidy"]) await assert.rejects(context.window.momMock.invoke(cmd, {}), /Room changes and undo are temporarily unavailable/);
  assert.ok((await context.window.momMock.invoke("get_trash")).spots.length > 0);
  assert.match(await context.window.momMock.invoke("chore_prompt", { path: "C:\\Example" }), /Mom here/);
});

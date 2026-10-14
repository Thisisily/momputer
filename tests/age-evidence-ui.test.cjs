// Render the real UI with synthetic age evidence. IPC is blocked, startup is excluded,
// and no test launches momputer or reads browser profiles, settings, or user files.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../app/ui/app.js"), "utf8");
const clickEnd = source.indexOf('  document.addEventListener("keydown",');
assert.ok(clickEnd > 0, "UI definitions must be available before startup");
const NOW = Math.floor(Date.now() / 1000);
const OLD = NOW - 800 * 86400;

function harness() {
  const context = vm.createContext({
    console,
    window: { momMock: { invoke: () => { throw new Error("Age rendering must not invoke IPC"); } } },
    localStorage: { getItem: () => null },
    document: { addEventListener() {} },
  });
  vm.runInContext(source.slice(0, clickEnd) + `
    globalThis.ui = { state, renderApps, extRow, profileRow, itemRow, listBody, bigList, picked, pickOf };
  })();`, context, { filename: "app/ui/app.js" });
  return context.ui;
}

function app(extra = {}) {
  return { key: "synthetic", name: "Example", publisher: "Example", size: 1234, lastUsed: OLD, dir: null, from: null, ...extra };
}

function appsHtml(apps) {
  const ui = harness();
  ui.state.health = { unusedApps: apps };
  return ui.renderApps();
}

const browser = { id: "chrome", name: "Chrome" };
const extension = (extra = {}) => ({ id: "synthetic", name: "Example extension", can: [], on: true, lastSaved: null, added: null, size: 0, page: "chrome://extensions", ...extra });

test("Steam update evidence is labeled updated; only a play record is labeled played", () => {
  const updated = appsHtml([app({ from: "Steam", lastUsedSource: "updated" })]);
  assert.match(updated, /updated 2 years ago/);
  assert.doesNotMatch(updated, /played 2 years ago|haven.t played/);
  assert.match(updated, /Games with older activity records/);
  const played = appsHtml([app({ from: "Steam", lastUsedSource: "played" })]);
  assert.match(played, /played 2 years ago/);
  assert.doesNotMatch(played, /updated 2 years ago/);
});

test("app data activity and legacy cached records do not imply the app was opened or played", () => {
  for (const extra of [{ lastUsedSource: "activity" }, {}, { from: "Steam" }, { from: "Steam", lastUsedSource: "unrecognized" }]) {
    const html = appsHtml([app(extra)]);
    assert.match(html, /activity recorded 2 years ago/);
    assert.doesNotMatch(html, /(?:opened|played) 2 years ago|haven.t opened/);
  }
});

test("missing or invalid activity dates remain unknown instead of becoming ancient or current use", () => {
  for (const lastUsed of [null, undefined, 0, -1, NaN, NOW + 86400]) {
    const html = appsHtml([app({ lastUsed, lastUsedSource: "played", from: "Steam" })]);
    assert.match(html, /activity date unknown/);
    assert.doesNotMatch(html, /played .* ago|NaN|1970/);
  }
});

test("an empty older-app list acknowledges omitted unknown activity", () => {
  const html = appsHtml([]);
  assert.match(html, /No apps with known activity records over a year old/);
  assert.match(html, /Apps with unknown activity are not listed/);
  assert.doesNotMatch(html, /Everything installed gets used|haven.t opened|haven.t played/);
});

test("extension storage dates describe changes, and missing dates do not imply disuse", () => {
  const ui = harness();
  const unknown = ui.extRow(browser, extension());
  assert.match(unknown, /storage activity unknown/);
  assert.doesNotMatch(unknown, /never saved|unused|saved something/);
  const known = ui.extRow(browser, extension({ lastSaved: OLD }));
  assert.match(known, /storage changed /);
  assert.doesNotMatch(known, /saved something|last used/);
  const off = ui.extRow(browser, extension({ on: false }));
  assert.match(off, /switched off/);
});

test("browser profile activity is attributed to its record and unknown activity stays neutral", () => {
  const ui = harness();
  const profile = { dir: "Default", name: "Example profile", extensions: [], notes: [], size: 0, cache: 0, lastUsed: OLD };
  assert.match(ui.profileRow(browser, profile), /browser recorded activity /);
  const unknown = ui.profileRow(browser, { ...profile, lastUsed: null });
  assert.match(unknown, /profile activity unknown/);
  assert.match(unknown, /data-led="off"/);
  assert.doesNotMatch(unknown, /data-led="green"/);
});

test("unreliable and legacy cleanup ages stay unknown and sort after known ages", () => {
  const ui = harness();
  ui.state.listView = { sort: "age", group: false, q: "" };
  const item = (name, extra = {}) => ({ name, path: `C:\\Fake\\${name}`, bytes: 1, ageSecs: 100 * 86400, ageKnown: true, goes: false, optional: false, ...extra });
  const old = item("known-old");
  const fresh = item("known-fresh", { ageSecs: 0 });
  const unknown = item("unknown", { ageKnown: false, ageSecs: 9000 * 86400 });
  const legacy = item("legacy", { ageKnown: undefined, ageSecs: 9999 * 86400 });
  const spot = { id: "old-temp", action: "delete" };
  const html = ui.listBody(spot, [legacy, unknown, fresh, old], false, "stays");
  assert.ok(html.indexOf("known-old") < html.indexOf("known-fresh"));
  assert.ok(html.indexOf("known-fresh") < html.indexOf("unknown"));
  assert.ok(html.indexOf("known-fresh") < html.indexOf("legacy"));
  assert.match(ui.itemRow(spot, unknown, false), /Age unknown/);
  assert.doesNotMatch(ui.itemRow(spot, unknown, false), /9000|changed /);
  assert.match(ui.itemRow(spot, fresh, false), /changed a moment ago/);
});

test("unknown members cannot inflate their group's oldest age", () => {
  const ui = harness();
  ui.state.listView = { sort: "age", group: true, q: "" };
  const item = (name, ageSecs, ageKnown = true) => ({ name, path: `C:\\Fake\\${name}`, bytes: 1, ageSecs, ageKnown, goes: false, optional: false });
  const html = ui.listBody({ id: "old-temp", action: "delete" }, [
    item("Alpha · recent", 0), item("Alpha · unknown", 9000, false),
    item("Beta · oldest", 100), item("Beta · recent", 1),
  ], false, "stays");
  assert.ok(html.indexOf('class="lgroup-name">Beta') < html.indexOf('class="lgroup-name">Alpha'));
});

test("big file ages use creation or modification recency as well as available access time", () => {
  const ui = harness();
  const file = { path: "C:\\Fake\\old.bin", folder: "Fake", size: 1234, modified: OLD, accessed: OLD, recency: NOW - 3 * 86400 };
  for (const accessTimes of [false, true]) {
    const html = ui.bigList({ big: [file], accessTimes });
    assert.match(html, /file activity recorded 3 days ago/);
    assert.doesNotMatch(html, /800 days|opened /);
  }
  const recentAccess = ui.bigList({ big: [{ ...file, accessed: NOW - 2 * 86400 }], accessTimes: true });
  assert.match(recentAccess, /file activity recorded 2 days ago/);
  const legacy = ui.bigList({ big: [{ ...file, recency: undefined }], accessTimes: false });
  assert.match(legacy, /file activity date unknown/);
});

test("pending Windows update files cannot be ticked or counted through stale selections", () => {
  const ui = harness();
  const setup = {
    name: "Windows setup · C:\\$GetCurrent", path: "C:\\$GetCurrent", bytes: 4096, files: 2,
    ageSecs: 800 * 86400, ageKnown: true, goes: false, optional: false,
    reason: "Windows has an update pending; finish the update before clearing setup files",
  };
  const spot = { id: "windows-junk", action: "delete", items: [setup], reclaimableItems: 0, reclaimableBytes: 0, newItems: 0, newBytes: 0 };
  const pk = ui.pickOf(spot.id);
  pk.goes = true;
  pk.new = true;
  pk.ticked.add(setup.path); // Left over from a selection before the update became pending.
  const html = ui.itemRow(spot, setup, true);
  assert.match(html, /Windows has an update pending/);
  assert.doesNotMatch(html, /<input[^>]+data-act="pick"/);
  assert.equal(ui.picked(spot).count, 0);
  assert.equal(ui.picked(spot).bytesChosen, 0);
  // A future refreshed verdict can offer a safe item again; protection is not a UI latch.
  const clear = { ...setup, goes: true, reason: "Last changed 800 days ago" };
  assert.match(ui.itemRow(spot, clear, true), /<input[^>]+data-act="pick"/);
});

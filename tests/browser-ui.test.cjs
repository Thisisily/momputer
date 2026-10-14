// Real browser-profile markup and delegated actions, with fake IPC and no app startup.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../app/ui/app.js"), "utf8");
const clickEnd = source.indexOf('  document.addEventListener("keydown",');
assert.ok(clickEnd > 0);
const browser = { id: "chrome", name: "Chrome" };
const profile = {
  dir: "Profile 2", name: "Work", path: "C:\\FakeBrowser\\Profile 2",
  lastUsed: null, size: 1000, cache: 200, history: 100, cookies: 2, cookieSites: 1,
  notes: [], extensions: [
    { id: "one", name: "Example", on: false, can: [], page: "chrome://extensions/?id=one", size: 10 },
  ],
};
const key = `${browser.id}|${profile.dir}`;

function harness() {
  let click;
  const calls = [];
  const context = vm.createContext({
    console,
    window: { momMock: { invoke: async (cmd, args) => { calls.push({ cmd, args }); return null; } } },
    localStorage: { getItem: () => null },
    document: { addEventListener: (name, fn) => { if (name === "click") click = fn; } },
  });
  vm.runInContext(source.slice(0, clickEnd) + `
    render = () => {};
    toast = () => {};
    globalThis.ui = { state, profileRow };
  })();`, context, { filename: "app/ui/app.js" });
  const ui = context.ui;
  return {
    ...ui, calls,
    html: () => ui.profileRow(browser, profile),
    action: (act, data = {}) => {
      const control = { dataset: { act, ...data }, closest: () => null };
      return click({ target: { closest: (selector) => selector === "[data-act]" ? control : null }, preventDefault() {} });
    },
  };
}

test("the profile name, metadata, size and chevron share one accessible native header button", () => {
  const h = harness();
  const html = h.html();
  const toggle = html.match(/<button\b[^>]*class="profile-toggle"[^>]*>([\s\S]*?)<\/button>/);
  assert.ok(toggle, "one full-header native button supports pointer, Enter and Space activation");
  assert.match(toggle[0], /type="button"/);
  assert.match(toggle[0], /data-act="profile-open"/);
  assert.match(toggle[0], /aria-expanded="false"/);
  for (const area of ["row-name", "row-meta", "row-nums num", "row-nums2 num", "row-chev"]) {
    assert.ok(toggle[1].includes(`class="${area}"`), `${area} is inside the same button`);
  }
  const panel = toggle[0].match(/aria-controls="([^"]+)"/)[1];
  const label = toggle[0].match(/aria-labelledby="([^"]+)"/)[1];
  assert.ok(html.includes(`id="${panel}" role="region" aria-labelledby="${label}"`));
  assert.ok(toggle[1].includes(`id="${label}">Work</span>`));
  assert.equal((html.match(/data-act="profile-open"/g) || []).length, 1);
  assert.doesNotMatch(toggle[1], /<button\b|<a\b/, "there are no nested interactive controls");
});

test("the shared header action expands and collapses details, including chevron activation", async () => {
  const h = harness();
  await h.action("profile-open", { key });
  assert.equal(h.state.browserOpen, key);
  assert.match(h.html(), /aria-expanded="true"/);
  assert.match(h.html(), /Everything/);
  assert.match(h.html(), /Show folder/);
  await h.action("profile-open", { key });
  assert.equal(h.state.browserOpen, null);
  assert.match(h.html(), /aria-expanded="false"/);
  assert.doesNotMatch(h.html(), /Show folder/);
  assert.equal(h.calls.length, 0, "opening a card only changes its details");
});

test("detail controls stay outside the toggle and do not collapse their profile", async () => {
  const h = harness();
  await h.action("profile-open", { key });
  const html = h.html();
  const headerEnd = html.indexOf("</button>");
  for (const action of ["ext-off", "reveal-path"]) assert.ok(html.indexOf(`data-act="${action}"`) > headerEnd);
  await h.action("ext-off", { key });
  assert.equal(h.state.browserOpen, key);
  assert.equal(h.state.extOff.has(key), true);
  assert.ok(h.html().indexOf('data-act="copy-fix"') > h.html().indexOf("</button>"));
  await h.action("reveal-path", { path: profile.path });
  assert.equal(h.state.browserOpen, key);
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0].cmd, "reveal");
  assert.equal(h.calls[0].args.path, profile.path);
});

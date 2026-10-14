// Setup weight in an agent's sheet, a project's folder button, and "Blank them" for keys in a
// shell history: real markup and delegated actions, with fake IPC and no app startup.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../app/ui/app.js"), "utf8");
const clickEnd = source.indexOf('  document.addEventListener("keydown",');
assert.ok(clickEnd > 0);

function harness() {
  let click;
  const calls = [];
  const context = vm.createContext({
    console,
    setTimeout: () => 0,
    window: { momMock: { invoke: async (cmd, args) => { calls.push({ cmd, args }); return cmd === "blank_secrets" ? "Took 1 key out of your PowerShell history." : null; } } },
    localStorage: { getItem: () => null },
    document: { addEventListener: (name, fn) => { if (name === "click") click = fn; } },
  });
  vm.runInContext(source.slice(0, clickEnd) + `
    render = () => {};
    renderSheet = () => {};
    toast = () => {};
    react = () => {};
    globalThis.ui = { state, renderAgentSheet, fixButton, tokensAbout };
  })();`, context, { filename: "app/ui/app.js" });
  const ui = context.ui;
  return {
    ...ui, calls,
    action: (act, data = {}) => {
      const control = { dataset: { act, ...data }, closest: () => null };
      return click({ target: { closest: (selector) => selector === "[data-act]" ? control : null }, preventDefault() {} });
    },
  };
}

const agent = (weight) => ({
  id: "claude", name: "Claude Code", problems: 0, checked: null, canCheck: false,
  settings: [], servers: [], hooks: [], skills: [], docs: [], weight,
});

test("setup weight says the whole in one line, offers a prompt for a big part, and lists the parts", () => {
  const h = harness();
  const parts = Array.from({ length: 8 }, (_, i) => ({ name: `part ${i}`, tokens: 900 - i * 100, scope: i ? "every chat" : "in storefront", doc: i ? null : "d1" }));
  h.state.agents = [agent({
    everywhere: 6640, most: 5030, mostIn: "storefront", servers: 3, parts,
    tips: [{ title: "storefront: memory index is about 4.5k tokens", detail: "Read at the start of every chat in storefront.", level: "note", doc: "d1", fix: { label: "Copy for your agent", text: "Help me shorten `MEMORY.md`." } }],
  })];
  const html = h.renderAgentSheet("claude");
  assert.match(html, /Setup weight/);
  assert.ok(html.includes("Every chat starts with about 6.6k tokens, about 5.0k more in storefront, plus what 3 MCP servers add."), html);
  assert.ok(html.includes("/context in Claude Code shows the exact total."));
  assert.ok(html.includes('data-act="copy-fix"') && html.includes("Help me shorten `MEMORY.md`."), "the tip carries its prompt");
  assert.ok(html.includes("about 900 tokens · in storefront"));
  assert.ok(html.includes("Show all 8"), "more than six parts fold away");
  assert.equal([h.tokensAbout(40), h.tokensAbout(340), h.tokensAbout(2140), h.tokensAbout(14400)].join("|"), "under 100|about 300|about 2.1k|about 14k");
  // An agent with nothing weighed shows no section.
  h.state.agents = [agent(null)];
  assert.ok(!h.renderAgentSheet("claude").includes("Setup weight"));
});

test("a project's folder button shows the folder, and keys are blanked only after Sure?", async () => {
  const h = harness();
  const folder = h.fixButton({ kind: "folder", label: "Open folder", value: "C:\\Users\\me\\Workspace\\atlas" });
  assert.match(folder, /data-act="reveal-path"/);
  assert.match(folder, /data-path="C:\\Users\\me\\Workspace\\atlas"/);
  const fix = { kind: "secrets", label: "Blank them", value: "psreadline:ConsoleHost_history.txt", hint: "Mom takes each key out and keeps no copy." };
  assert.match(h.fixButton(fix), />Blank them</);
  await h.action("secrets-blank", { id: fix.value });
  assert.equal(h.calls.filter((c) => c.cmd === "blank_secrets").length, 0, "the first press only asks");
  assert.match(h.fixButton(fix), /Sure\? Blank them/);
  assert.match(h.fixButton(fix), /keeps no copy/);
  await h.action("secrets-blank", { id: fix.value });
  const blank = h.calls.filter((c) => c.cmd === "blank_secrets");
  assert.equal(blank.length, 1);
  assert.equal(blank[0].args.id, "psreadline:ConsoleHost_history.txt");
  assert.ok(h.calls.some((c) => c.cmd === "get_system"), "the checkup is asked again afterwards");
});

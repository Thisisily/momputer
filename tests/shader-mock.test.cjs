// The browser demo uses in-memory fake IPC; this test never executes the native app.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

test("the shader demo consumes one scan, refreshes after rescan and reports version 0.3.2", async () => {
  const context = vm.createContext({
    window: {}, location: { search: "", origin: "http://localhost" }, console,
    setTimeout: (fn) => { fn(); return 1; }, clearTimeout() {}, setInterval() { return 1; }, clearInterval() {},
  });
  vm.runInContext(fs.readFileSync(path.join(__dirname, "../app/ui/mock.js"), "utf8"), context);
  const mock = context.window.momMock;
  let state;
  await mock.listen("trash", event => { state = event.payload; });
  await mock.invoke("measure_trash");
  assert.ok(state.shaderCleanup.files.length);
  assert.equal(state.shaderCleanup.skipped, 0);
  const snapshot = state.shaderCleanup, chosen = snapshot.files[0];
  const result = await mock.invoke("clean_shader_cache", { token: snapshot.token, paths: [chosen.path] });
  assert.equal(result.deleted, 1);
  assert.equal(result.freed, chosen.bytes);
  assert.equal(state.shaderCleanup, null);
  await assert.rejects(mock.invoke("clean_shader_cache", { token: snapshot.token, paths: [chosen.path] }));
  await mock.invoke("measure_spot", { id: "shader-cache" });
  assert.notEqual(state.shaderCleanup.token, snapshot.token);
  assert.equal(state.shaderCleanup.files.some(f => f.path === chosen.path), false);
  assert.equal(await mock.invoke("app_version"), "0.3.2");
  assert.equal((await mock.invoke("get_update")).latest, "0.3.2");
});

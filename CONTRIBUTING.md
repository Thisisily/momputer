# Contributing to momputer

- **Teach mom a new agent:** add its markers to `mark_from_env` and its process to `agent_process` in [`crates/mom-core/src/agents.rs`](crates/mom-core/src/agents.rs).
- **Teach mom a new runtime or naming trick:** [`classify.rs`](crates/mom-core/src/classify.rs) has tables and real-world test cases.
- **Find more trash:** each clean-up is a spot in [`trash.rs`](crates/mom-core/src/trash.rs) with what it is, what cleaning does and the items it found. Another agent tool's leftovers and conversations go in `agents()` in [`trash/harness.rs`](crates/mom-core/src/trash/harness.rs): where it lives, which folders hold leftovers, and which hold one conversation each. PC checkups live in [`health.rs`](crates/mom-core/src/health.rs).
- **Give mom new lines:** [`voice.rs`](crates/mom-core/src/voice.rs).
- **Work on the widget:** it's plain HTML, CSS and JS in [`app/ui`](app/ui), with no build step. Every list with tick boxes gets a Tick all / Untick all (`tickAllButton` in `app.js`), and anything that deletes or moves asks "Sure?" on the first press. Serve the folder (`python -m http.server -d app/ui`) and it runs in any browser with realistic mock data. Its libraries and fonts are pinned in `package.json` and copied into `app/ui/vendor` by `npm install && npm run vendor`. The copies are committed, so building the app never needs npm.
- **Give mom a new face:** she's drawn in SVG in [`app/ui/mom.js`](app/ui/mom.js), with eyes, brows, mouths and extras combined per mood, and animated with GSAP. [`app/art/mom-reference.webp`](app/art/mom-reference.webp) is the reference sheet she's drawn from. `node scripts/icons.js` redraws the app and tray icons from the same SVG.

`cargo test` runs the core tests, including one that rebuilds a real cluttered folder and checks mom grades it F, tidies it, and puts everything back.

Writing a skill: [skills/AUTHORING.md](skills/AUTHORING.md).

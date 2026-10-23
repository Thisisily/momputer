# Roadmap

What's next for momputer, and how each piece would work. None of this is built yet.

## A Mac version

momputer on macOS first, then Linux.

- The core (`crates/mom-core`) is mostly portable. The Windows-only parts already sit behind `cfg(windows)`, and each needs a macOS twin:
  - **The process list:** one `NtQuerySystemInformation` call today. On macOS that's `libproc` for the list, and `sysctl(KERN_PROCARGS2)` for each process's command line and environment, which is where the agent markers (`CLAUDE_CODE_SESSION_ID`, `CODEX_THREAD_ID`) live. It only works for your own processes, which is all mom needs.
  - **Trash:** the Recycle Bin becomes the Trash, and the Windows-only clean-ups get macOS ones: `~/Library/Caches`, crash reports in `~/Library/Logs/DiagnosticReports`, Xcode's DerivedData and old simulators, and Homebrew's cache.
  - **Administrator clean-ups:** UAC becomes macOS's own password prompt (Authorization Services).
  - **Checkup:** the drive, memory, sound and settings checks read macOS's own tools (`diskutil`, `system_profiler`, `defaults`).
- Tauri builds the `.dmg`. An app that opens without warnings needs an Apple Developer ID and notarization.

## Out of date: apps and global packages

Which apps and global packages have a newer version, and one button to update them.

### How it was done by hand (23 October 2026)

A PowerShell update notice (7.6.6 to 7.6.7) turned into updating the whole PC with an agent. That's the recipe:

1. **What's installed, and how.** Look up each tool on the PATH (`node`, `npm`, `pnpm`, `yarn`, `bun`, `deno`, `python`/`py`, `pip`, `uv`, `rustup`, `cargo`, `go`, `java`, `git`, `gh`, `docker`, `nvm`, `fnm`, `volta`, `scoop`, `choco`). How it was installed decides how it updates. Node was nvm's, because `C:\Program Files\nodejs` was a link into nvm's folder, so winget didn't know it. `py -0p` lists every Python.
2. **Each manager's own "outdated" list.** These are read-only, and each tool asks its own source:
   - winget: `winget upgrade --include-unknown --accept-source-agreements`, once with `--scope user` and once with `--scope machine`, plus its pins.
   - npm globals: `npm outdated -g --json`.
   - scoop: `scoop status`. Chocolatey: `choco outdated --limit-output`.
   - Rust: `rustup check`. uv: `uv self update --dry-run`.
   - Node's release lines: `https://nodejs.org/dist/index.json` (newest per major, LTS name, and a `security` flag).
3. **What projects need.** Read `.nvmrc`, `.node-version` and the `engines` field of `package.json` in the code folders. Never move Node to a major version a project can't run. The projects there pinned Node 22, and OpenClaw's newest release needed Node 24, so OpenClaw stayed on the version that runs on 22.
4. **What's running.** An app that's open can't be updated, or its installer waits for it, so open apps go last.
5. **Updating.** One administrator batch for machine-wide apps and one plain batch for per-user apps, each with a summary of every app's exit code, then measure again.

Lessons from doing it by hand:

- **Run batches hidden.** Never use a console window: selecting text in one pauses it until someone presses Esc or Enter.
- **Start batches on their own.** Started that way, they keep going if the agent or the widget stops.
- **PowerShell goes last.** It can't replace itself while it's running.
- **Scripts that Windows PowerShell 5.1 reads** need a byte-order mark, or plain ASCII only.
- **Explorer can't start a file whose path is over 260 characters.**
- **nvm-windows sometimes prints nothing at all.** Pointing the `nodejs` link at the new version's folder worked instead.
- **After a major npm update,** some globals' native modules stop loading and need reinstalling (`re2` in firebase-tools, `keytar` in mint). Load each one to check.

### In momputer

- A Versions tab in Upkeep: one row per app or package, showing what's installed, what's newest, and which tool updates it.
- It only reads lists, with each tool's own command. Those tools ask the internet, so the tab stays off until you switch it on, and it says which tool asks where.
- Updating uses the same tools. Mom never downloads an installer herself.

## Known holes: a vulnerability checker

Versions with known security problems, the ones attackers already use first.

### How it was done by hand (same session)

- **Node:** `npx is-my-node-vulnerable` compares the Node that runs against Node's security releases. Moving from 22.22.0 to 22.23.4 fixed about 30 CVEs.
- **Python:** `uvx pip-audit --path <Python>\Lib\site-packages --format json` for each Python install gives each problem and the version that fixes it (tornado, ujson and others).
- **Apps:** two were being exploited in the wild: WinRAR 6.02 (CVE-2025-8088, CVE-2023-38831) and MongoDB 7.0.9 ("MongoBleed", CVE-2025-14847). Both were found by checking versions by hand and fixed first.

### In momputer

- Match installed versions (winget's list, npm globals, each Python's packages, `cargo install --list`) against [OSV.dev](https://osv.dev): one open database for npm, PyPI, crates.io, Go and more.
- Check apps against CISA's [Known Exploited Vulnerabilities](https://www.cisa.gov/known-exploited-vulnerabilities-catalog) list, so what's attacked right now comes first.
- Each finding says what's affected and which version fixes it, with an update button (see Out of date).
- Off until you switch it on. OSV and CISA both offer their whole lists for download, so mom can download the lists and compare on your PC. Then nothing about your PC is sent anywhere.

## Also next

- **winget:** install momputer with one command.
- **An MCP server:** so agents can ask mom directly.
- **More mom:** bedtime (a nightly tidy-up), screen time for agents, and a nudge when a disk is nearly full.

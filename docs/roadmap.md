# Roadmap

What's next for momputer, and how each piece would work. None of this is built yet.

## A Mac version

The first part is built: `./scripts/build.sh --app` makes `momputer.app`, and the tests run on a Mac as they do on Windows.

How it works there:

- **The process list:** `libproc` for each process's folder and program, and `sysctl(KERN_PROCARGS2)` for its command line and environment, which is where the agent markers (`CLAUDE_CODE_SESSION_ID`, `CODEX_THREAD_ID`) live. A program that renames itself (npm does) writes over its own arguments, so mom reads where they end herself. It only works for your own processes, which is all she needs.
- **Orphans:** a Mac hands a process whose parent ended to launchd, where Windows leaves it its dead parent's number. Mom asks launchd for its own jobs (`launchctl list`); anything else of yours under it was left behind.
- **Ports and stopping:** each of your processes is asked for its sockets (`proc_pidfdinfo`). Stopping asks first (SIGTERM), then ends what's still there.
- **Storage:** every clean-up knows where a Mac keeps things (`Paths::layout`), and the Mac has its own: Xcode's DerivedData, simulators Xcode can't start, Homebrew's and CocoaPods' caches, each app's folder in `~/Library/Caches`, crash reports. What goes to the bin goes to the Trash.
- **Checkup:** `system_profiler`, `fdesetup`, `spctl`, `csrutil`, `tmutil` and `pmset`, none of which needs an administrator.
- **Programs:** an app is its bundle with every helper inside it, the apps that are open are the ones `lsappinfo` lists with a place in the Dock, and Quit asks the way ⌘Q does.
- **Startup:** the launch agents and daemons in the three LaunchAgents and LaunchDaemons folders, switched with `launchctl disable` and `enable`, which touches no file.
- **Speed:** crashes from the reports in DiagnosticReports, heat from `pmset -g therm`, and memory parked on the disk from `sysctl vm.swapusage`.
- **Apps:** an app's last use is the newest sign it left: Spotlight's record of it being opened (rarely there: it doesn't outlive an update), or the dates of its settings and data in the Library. Caches don't count, since a cleaner changes those. Inside another app's container mom asks only for dates, never looks around. A Steam game's last play comes from the account's own file, because Steam on a Mac doesn't write it in the game's record.

Still to do:

- **Login items.** Apps set to open at login are kept where only System Settings (or an administrator) can read them.
- **The Trash.** macOS shows what's in it only to apps given Full Disk Access.
- **Administrator clean-ups:** UAC becomes macOS's own password prompt (Authorization Services).
- **A download.** An app that opens without warnings needs an Apple Developer ID and notarization; then the release builds a `.dmg` beside the Windows installer.
- **Linux**, after that.

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

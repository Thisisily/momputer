# disk-space-report

An Agent Skill that answers "where did my disk space go?" on a Windows PC, the careful way. The agent surveys every local drive without changing anything: space used and free, the biggest folders and files, and the usual suspects by name and size (node_modules, virtual environments and build folders, Docker and WSL disk images, package and AI-model caches, browser caches, Windows' Temp and update leftovers, the hibernation and page files, the Recycle Bin, games and shader caches, old installers, big videos, big things untouched for a year). It builds one page with per-drive bars, a chart of what fills each drive that you can click into, and a plan. The user ticks what they want (every list has Tick all and Untick all) and copies an approval line back.

The skill never deletes a file itself. A ticked item is one of three things: a fixed cleanup command of the tool that owns the data (`npm cache clean --force`, `pip cache purge`, `docker builder prune --force`), one of Windows' own tools opened for the user (Storage settings, Disk Cleanup, the Recycle Bin, Installed apps), or a list for the user to go through or hand to the `tidy-folders` skill.

## Needs

An agent that runs commands on the Windows PC itself (Claude Code, Codex, or similar) and Python 3.8+ with nothing else installed. A cloud sandbox can't see the user's drives, so the skill does nothing useful there.

## Install

In Claude Code, one line:

```
/plugin install disk-space-report --marketplace Thisisily/momputer
```

Or copy the `disk-space-report` folder to:

- Claude Code: `~/.claude/skills/disk-space-report/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/disk-space-report/` (or `.agents/skills/` in a repo)

Then ask: "where did my disk space go?", "why is C: full?", "what can I clear to get space back?"

## What it will and won't do

- Survey: read-only. It writes one JSON file to a scratch folder. It opens no file: names, sizes and dates come from folder listings. It never follows a junction or symlink, never lists another account's folders, and skips what needs administrator rights.
- Never deletes, moves or empties anything itself. No file or folder name is ever put into a command: every command is a constant line in a table in `run_plan.py`, started without a shell.
- Runs only the ticked ids of the exact plan the user approved. A plan file changed after approval is refused, and an item that points at anything outside the table is refused.
- Code projects are listed, never cleaned: someone's `node_modules` may be needed offline.
- Windows itself is not walked. Its size is worked out from what is left over and labelled that way.
- Local fixed disks only. Cloud and removable drives are left out unless named; network drives always.
- Works within a time budget (7 minutes by default) and says so when it ran out: what it couldn't reach is labelled as partly counted, never guessed.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `scripts/survey.py` | Read-only survey, writes `survey.json` |
| `scripts/summarize.py` | Short digest of a survey for the agent to review |
| `scripts/build_page.py` | Builds the page and `diskspace-plan.json` |
| `scripts/run_plan.py` | The table of fixed commands; dry run and run of approved ids |
| `assets/disk-space-template.html` | The self-contained page (no network, light and dark) |
| `references/review-guide.md` | Survey fields, `notes.json`, the command table, judgement calls |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)

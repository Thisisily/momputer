# desktop-icons

An Agent Skill that tidies a cluttered Windows desktop the careful way. The agent surveys the user's own Desktop folder (wherever it really is, OneDrive included) and the Public Desktop without changing anything. It reads each shortcut file, never runs one, and finds shortcuts whose target is gone, several shortcuts to the same program, shortcuts to things already on the taskbar or in the Start menu, shortcuts that run a command, loose documents, screenshots, installers and folders and how long they have sat there, and how many icons there are against what fits on one screen. It builds one interactive page with charts. The user ticks what they want (every list has Tick all and Untick all), copies an approval line back, and only those items change. Shortcuts go to the Recycle Bin, loose files move into folders, every step is logged, and a fixed `revert.ps1` next to the log moves everything back.

## Needs

An agent that runs commands on the Windows PC itself (Claude Code, Codex, or similar) and Python 3.8+ with nothing else installed. A cloud sandbox can't see the user's desktop, so the skill does nothing useful there.

## Install

In Claude Code, one line:

```
/plugin install desktop-icons --marketplace Thisisily/momputer
```

Or copy the `desktop-icons` folder to:

- Claude Code: `~/.claude/skills/desktop-icons/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/desktop-icons/` (or `.agents/skills/` in a repo)

Then ask: "clean up my desktop", "which of my desktop shortcuts are broken?", "my desktop is full of icons".

## What it will and won't do

- Survey: read-only. It writes one JSON file to a scratch folder. The only file contents it reads are shortcut files (`.lnk` and `.url`, at most 256 KB each). It never runs a shortcut and never asks Windows to resolve one.
- A shortcut counts as dead only when the full path it recorded is on a fixed disk in this PC and nothing is there. Network shares are never contacted. Anything it can't read or check is "unknown" and is never ticked for removal.
- Changes only the user's own Desktop. Files it moves land in the user's Documents, Pictures, Videos, Music or Downloads, or in a few folders on the Desktop if the user prefers. The Public Desktop is never changed.
- Leaves working program shortcuts where they are unless the user ticks them. Leaves code projects, a program's own folder, hidden files and links alone.
- Doesn't move icons around on the screen. Windows keeps icon positions itself; the skill tells the user the two clicks for Auto arrange and Sort by.
- Changes only the ids the user approved for that exact plan, never overwrites, and keeps an undo log in `Documents\Tidy-up logs\`. A plan file changed after approval is refused.
- Treats file names and shortcut targets as data. None is ever written into a command or script.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `scripts/survey.py` | Read-only survey, writes `survey.json` |
| `scripts/summarize.py` | Short digest of a survey for the agent to review |
| `scripts/build_page.py` | Builds the page and `desktop-plan.json` |
| `scripts/apply_plan.py` | Dry run, apply, undo: the command the agent runs. Limits the run to the Desktop |
| `scripts/safe_apply.py` | The file-moving engine and all its guard rails, shared unchanged by every skill here that moves files |
| `scripts/revert.ps1` | Fixed revert script, copied next to each undo log |
| `assets/desktop-plan-template.html` | The self-contained page (no network, light and dark) |
| `references/review-guide.md` | Survey fields, how a shortcut is judged, `notes.json` format, judgement calls |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)

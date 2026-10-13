# tidy-folders

An Agent Skill that tidies a Windows PC's folders the careful way. The agent surveys Desktop, Documents, Downloads, Pictures, Videos, Music, the profile folder and the top of every drive without changing anything. It finds contracts, IDs, tax papers and invoices lying loose, games outside a Games folder, duplicate copies, old installers and empty folders, and builds one interactive page with charts. The user ticks what they want (every list has Tick all and Untick all), copies an approval line back, and only those items move. Deletions go to the Recycle Bin, every step is logged, and a fixed `revert.ps1` next to the log moves everything back.

## Needs

An agent that runs commands on the Windows PC itself (Claude Code, Codex, or similar) and Python 3.8+ with nothing else installed. A cloud sandbox can't see the user's folders, so the skill does nothing useful there.

## Install

Copy the `tidy-folders` folder to:

- Claude Code: `~/.claude/skills/tidy-folders/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/tidy-folders/` (or `.agents/skills/` in a repo)

Then ask: "tidy up my Downloads", "where are my tax papers?", "are all my games in the Games folder?"

## What it will and won't do

- Survey: read-only. It writes one JSON file to a scratch folder. It peeks into at most 300 small loose text and Word files for keywords, and hashes same-size files to confirm duplicates (`--no-peek` and `--no-hash` turn both off). It never reads OneDrive online-only files.
- Never moves installed programs or launcher games. It explains the launcher's own Move instead.
- Never touches Windows, Program Files, ProgramData, AppData, backup folders or code projects.
- Moves only the ids the user approved for that exact plan, never overwrites, and keeps an undo log in `Documents\Tidy-up logs\`. A plan file changed after approval is refused, and so is any row of an undo log that points into a protected folder.
- Works on local disks only. Network shares and mapped network drives are refused.
- Treats file names as data. No file name is ever written into a command or script: `revert.ps1` is a constant file that reads `undo-log.csv`, and every file is addressed by its exact name. Symlinks and junctions are left alone.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `scripts/survey.py` | Read-only survey, writes `survey.json` |
| `scripts/summarize.py` | Short digest of a survey for the agent to review |
| `scripts/build_page.py` | Builds the page and `plan.json` |
| `scripts/apply_plan.py` | Dry run, apply, undo, all guard rails |
| `scripts/revert.ps1` | Fixed revert script, copied next to each undo log |
| `assets/tidy-plan-template.html` | The self-contained page (no network, light and dark) |
| `references/review-guide.md` | Survey fields, `notes.json` format, judgement calls |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)

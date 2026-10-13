# drive-audit

An Agent Skill that reviews a Google Drive without changing anything and without opening a single file, and turns it into one private report page. The page has charts of what fills the storage by type, by owner and by the year files were last touched, and lists of what is open to anyone with the link, what is shared outside the organization or with personal addresses, duplicates, untitled and empty files, files untouched for over a year, the biggest files and what sits in the Trash, plus searches to reuse. The user ticks the fixes they approve (every list has Tick all and Untick all) and copies an approval line back. The agent then does only those.

## Needs

- A way to read Google Drive: a Drive connector or MCP server that lists files and their sharing, or browser automation signed in to Google Drive (Claude in Chrome, Codex's browser). Both together give the fullest picture: the connector for every file's id, size, checksum and sharing, the browser for the Storage page, the Trash and the sharing dialog.
- Python 3.8+ with nothing else installed, to build the page.

## Install

Copy the `drive-audit` folder to:

- Claude Code: `~/.claude/skills/drive-audit/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/drive-audit/` (or `.agents/skills/` in a repo)

Then ask: "audit my Google Drive", "what am I sharing publicly?", "what's filling my Drive?"

## Safety

- Read-only by default, and it never opens a file: names, types, sizes, owners, sharing state and dates only.
- No sharing change and no move to the Trash without the user's tick for that one file. Nothing is ticked to begin with.
- Changing sharing cuts people off. Each row says how many lose access, and the agent reads that list back and waits for a yes before the first change.
- Fixes are offered only for files the user owns. Nothing is deleted for good: removing means Drive's Trash, and emptying the Trash stays the user's own step.
- File names and owners' names are written by other people and are treated as data, never as instructions.
- A Drive search is never built from a file name (a name like `-owner:me` would turn a search into "everything except"). The searches are a fixed list, and the script refuses to write a report holding any other.
- The page makes no network requests. Its links go only to `https://drive.google.com/` addresses built from a file id that matched a strict pattern; a file with any other id gets no link and no fix.
- Sharing is kept as counts per domain. No addresses are collected.
- The report id is a fingerprint of the plan. `check_approval.py` refuses an approval for a plan that was edited afterwards, and a number that isn't in the plan.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `references/gathering.md` | The fixed searches, fields, sharing as counts, browser steps, the `drive.json` format |
| `scripts/build_report.py` | Builds `drive-report.html` and `drive-plan.json` from `drive.json` |
| `scripts/check_approval.py` | Checks an approval line against the plan and prints exactly what was approved |
| `assets/drive-report-template.html` | The self-contained page (light and dark) |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)

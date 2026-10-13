# Skills

Agent Skills that tidy one room of your digital life each. They all work the same way: look without changing anything, show one page of what they found, and act only on what you tick.

| Skill | What it does |
| --- | --- |
| [`mom`](mom) | Teaches an agent to clean up after itself: stop what it started, take out its own trash, put reports where they belong. |
| [`tidy-folders`](tidy-folders) | Surveys a Windows PC's folders, finds important papers lying loose, games outside the Games folder, duplicates and old installers, and moves only what you approve, with an undo. |
| [`inbox-cleanup`](inbox-cleanup) | Reviews two months of Gmail and builds a private report: who fills your inbox, what you never open, what to unsubscribe from, likely phishing, and a declutter plan. |
| [`calendar-fixer`](calendar-fixer) | Reviews your Google Calendar: double bookings, no breaks, stale recurring meetings, unanswered invitations. Fixes only what you tick. |
| [`drive-audit`](drive-audit) | Reviews Google Drive: what's shared with anyone who has the link, what fills your storage, duplicates, files nobody touched in a year. |
| [`disk-space-report`](disk-space-report) | Shows where a Windows PC's disk space went, and the safe way to get each part of it back. It deletes nothing itself. |
| [`backup-checkup`](backup-checkup) | Answers "if this disk died today, what would I lose?": which folders and code have a second copy, and which don't. |

## Install

Copy a skill's folder to:

- Claude Code: `~/.claude/skills/<name>/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/<name>/` (or `.agents/skills/` in a repo)

Then ask for what you want in plain words: "tidy up my Downloads", "clean up my Gmail".

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included. (The momputer app in this repository has its own, different license; the skills don't need the app.)

Writing one: [AUTHORING.md](AUTHORING.md).

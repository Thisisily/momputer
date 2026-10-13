# backup-checkup

An Agent Skill that answers "if this disk died today, what would I lose?" on a Windows PC. The agent checks, without changing anything, whether each of your main folders and the other places holding your own work has a second copy somewhere: OneDrive and its folder backup, Dropbox, Google Drive, iCloud Drive, Syncthing, Nextcloud, File History, Windows Backup, or a second drive. For developers it also finds the code that exists nowhere else: Git repositories with no remote, with commits that were never pushed, or with stashes. It builds one page with a headline ("3 of your 7 folders have no second copy"), each folder with its size and what protects it, a "you would lose" list with the irreplaceable things first, and a plan. The user ticks what they want (every list has Tick all and Untick all) and copies an approval line back.

The skill never copies, uploads or syncs anything and never changes a backup setting. A ticked item is one of Windows' own pages opened for the user (the backup settings, File History), or something the user does themselves with the agent explaining it.

## Needs

An agent that runs commands on the Windows PC itself (Claude Code, Codex, or similar) and Python 3.8+ with nothing else installed. A cloud sandbox can't see the user's folders, so the skill does nothing useful there.

## Install

In Claude Code, one line:

```
/plugin install backup-checkup --marketplace Thisisily/momputer
```

Or copy the `backup-checkup` folder to:

- Claude Code: `~/.claude/skills/backup-checkup/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/backup-checkup/` (or `.agents/skills/` in a repo)

Then ask: "are my files backed up?", "what would I lose if this disk died?", "is OneDrive actually backing up my Desktop?", "which of my repositories aren't pushed?"

## What it will and won't do

- Check: read-only. It writes one JSON file to a scratch folder. Your files are never opened: it reads names, sizes and dates, the registry, and the settings files that backup and sync programs keep about themselves.
- Never runs Git inside a repository it finds. A repository's own settings can make any Git command start a program, so the check reads the `.git` folder's files directly. That is why it says honestly that it can't see uncommitted changes.
- Never uploads, copies, syncs or changes a backup setting. It can open two Windows pages, and only when ticked.
- Account names and e-mail addresses from sync settings are masked. A token inside a Git remote address is dropped.
- Folder, repository and branch names are treated as data and never put into a command.
- A second drive only counts as a second copy if it is a different physical disk, and the page says which drives are.
- Says what it can't know: whether a sync program is paused or failing, whether a backup can really be restored, what a backup program covers when its settings can't be read from outside.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `scripts/survey.py` | The read-only check, writes `survey.json` |
| `scripts/summarize.py` | Short digest of a survey for the agent to review |
| `scripts/build_page.py` | Builds the page and `backup-plan.json` |
| `scripts/run_plan.py` | Checks the approval and opens the ticked Windows pages |
| `assets/backup-checkup-template.html` | The self-contained page (no network, light and dark) |
| `references/review-guide.md` | Survey fields, `notes.json`, what each finding rests on, judgement calls |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)

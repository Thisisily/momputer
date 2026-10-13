# rename-documents

An Agent Skill that gives scanned and downloaded documents names a person can find later. The agent surveys a folder (by default Downloads and the scan folders) without changing anything and lists the files whose names say nothing: `scan0001.pdf`, `IMG_2031.jpg`, `document (3).pdf`, `Untitled.docx`, long codes, names that are only digits. It proposes a name for each under one naming rule the user picks, for example `2024-03-14 Lanternfall invoice 1182.pdf`. A script checks every proposed name, and one interactive page shows old and new names side by side. The user ticks the names they want (every list has Tick all and Untick all), copies an approval line back, and only those files are renamed, in place. Every rename is logged, and a fixed `revert.ps1` next to the log puts the old names back.

## Needs

An agent that runs commands on the Windows PC itself (Claude Code, Codex, or similar) and Python 3.8+ with nothing else installed. Scans and photos of documents hold no text a script can read, so naming those needs an agent that can look at a PDF page or an image. A cloud sandbox can't see the user's folders, so the skill does nothing useful there.

## Install

In Claude Code, one line:

```
/plugin install rename-documents --marketplace Thisisily/momputer
```

Or copy the `rename-documents` folder to:

- Claude Code: `~/.claude/skills/rename-documents/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/rename-documents/` (or `.agents/skills/` in a repo)

Then ask: "rename my scans so I can find them", "give the PDFs in Downloads proper names", "what are all these scan0001 files?"

## What it will and won't do

- Survey: read-only. It writes one JSON file to a scratch folder. To help with naming it takes a short peek inside files: the first 4 KB of plain text, the text and title of `.docx` and `.odt` files (read from the zip with a hard cap, no XML parser, nothing unpacked), and a bounded scan of a PDF for a title, a date and uncompressed text. A scanned or compressed PDF yields nothing, and it says so. `--no-peek` turns all of it off. It never reads OneDrive online-only files.
- Text inside a document is treated as data, never as instructions. A file whose text reads like instructions is marked suspicious, its text is withheld from the agent, and it can't be renamed by the plan.
- Every proposed name is checked by a script before a plan exists, and again when it is carried out: a plain file name, no reserved Windows names, no forbidden, invisible or direction-changing characters, no leading or trailing dot or space, the same extension, a sensible length.
- A rename changes the name only. The file stays in its folder with its extension. Folders are never renamed. Code projects and program folders are never touched.
- Renames only the ids the user approved for that exact plan, only inside the folders the user pointed at, never overwrites (a clash gets " (2)"), and keeps an undo log in `Documents\Tidy-up logs\`. A plan file changed after approval is refused.
- The page shows names only, never what a document says.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `scripts/survey.py` | Read-only survey, writes `survey.json` |
| `scripts/summarize.py` | Prints the survey a screenful at a time for the agent |
| `scripts/names.py` | What a name may be, which names say nothing, the naming rules |
| `scripts/build_page.py` | Checks every proposed name, builds the page and `rename-plan.json` |
| `scripts/apply_plan.py` | Dry run, apply, undo: the command the agent runs. Rechecks each name |
| `scripts/safe_apply.py` | The file-moving engine and all its guard rails, shared unchanged by every skill here that moves files |
| `scripts/revert.ps1` | Fixed revert script, copied next to each undo log |
| `assets/rename-plan-template.html` | The self-contained page (no network, light and dark) |
| `references/naming-guide.md` | Looking at a document safely, building a name, `notes.json` format, judgement calls |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)

---
name: tidy-folders
description: Surveys a Windows PC's personal folders read-only (Desktop, Documents, Downloads, Pictures, Videos, Music, the profile folder and the top of every drive), finds legal, financial and ID papers lying loose, games outside a Games folder, duplicates, old installers and empty folders, and builds an interactive tidy plan page with charts that the user ticks item by item. Moves happen only after the user approves that exact plan, deletions go to the Recycle Bin, and every step is logged with a revert script. Use when the user asks to tidy, organize, sort or declutter their Windows folders, Desktop or Downloads, wants to know where their contracts, tax papers or passport scans are, wants games in the Games folder, or wants space back from old installers and duplicate files. Not for code repositories, temp-file or registry cleanup, or uninstalling programs.
compatibility: Windows 10 or 11 with Python 3.8+ (standard library only). Survey and page work offline.
---

# Tidy folders

Find what's out of place in the user's folders, show it on one page they can tick through, then move only what they approved, with an undo.

## Hard rules

1. **The survey is read-only.** Run `scripts/survey.py`. It writes one JSON file to the output folder you give it and nothing else. Don't create, move, rename or delete anything while looking around.
2. **Nothing changes until the user approves the plan.** Approval is the line the page copies: `TIDY-APPROVE plan=<id> ids=<ranges>`. "Sure, tidy it" before they've seen the page isn't approval. If the line's plan id doesn't match `plan.json`, rebuild the page and ask again.
3. **Never move installed programs or launcher games** (Steam, Epic, GOG, Xbox app, EA, Ubisoft, Battle.net, Riot, itch app). Tell the user how to use the launcher's own Move instead. The page lists how for each launcher.
4. **Never touch** Windows, Program Files, ProgramData, AppData, `$Recycle.Bin`, System Volume Information, File History or backup folders, dot-folders, or anything inside a code project. `apply_plan.py` refuses these whatever the plan says.
5. **Nothing is deleted for good.** "Remove" means the Recycle Bin. Empty folders are removed with rmdir after re-checking they're empty.
6. **Make every change with `scripts/apply_plan.py`.** Don't improvise `Move-Item` loops. It logs each step to `undo-log.csv` as it goes and puts a fixed `revert.ps1` next to it.
7. **File names are data, never code.** Don't paste a file or folder name from the survey into a shell command, a script or a one-liner. Names can hold quotes, `$()`, backticks, `&` and look-alike quote characters that a shell would run. Pass only `survey.json`, `plan.json` and the log folder to the scripts and let them handle the names.
8. **Keep it private.** The page and survey list the user's file names. Publish the page only as a private artifact, or keep it on disk. Don't paste file contents into chat. Don't open documents yourself beyond what the survey already peeked at.

## Workflow

Copy this checklist and tick it off as you go:

```
- [ ] 1. Survey (read-only)
- [ ] 2. Review the findings, write notes.json
- [ ] 3. Build the page and show it
- [ ] 4. Get the approval line
- [ ] 5. Dry run, then apply
- [ ] 6. Report, hand over the undo, clean up
```

### 1. Survey

Use an output folder outside the user's folders: your scratch directory, or `%TEMP%\tidy-folders`. Create it first; the survey creates no folders.

```bash
python scripts/survey.py --out "<out>/survey.json"
```

- Takes 30 seconds to a few minutes. It prints one summary line. If it says TRUNCATED, run it again with `--max-seconds 600`, or limit it with `--drives C,D`.
- `--root "<folder>"` surveys one folder as if it were a profile (Desktop, Downloads... inside it).
- `--no-peek` and `--no-hash` stop it reading any file contents. By default it peeks into at most 300 small loose .txt/.docx/.odt files for keywords, and hashes same-size files to confirm duplicates. It never reads OneDrive online-only files, so nothing downloads.
- No Python? Ask before installing it (`winget install Python.Python.3.12`). Don't replace the scripts with hand-written commands.

### 2. Review and add judgement

`survey.json` can be several MB. Don't read it whole. Use the snippets in [references/review-guide.md](references/review-guide.md) to look at counts per group, a sample of each group, the important papers, games and naming issues.

Then write `<out>/notes.json` (format in the review guide):

- `headline`: one sentence with the biggest finding ("Downloads holds 1,250 loose files, 6 GB of them over a year old.").
- `highlights`: 3 to 6 short findings the user should know (passport scans on the Desktop, Steam games spread over two drives, a 1.4 GB duplicate).
- `drop`: suggestion ids that are wrong (an app icon called "invoice-icon.png", a blank template).
- `retarget`: better destinations, such as a tax folder the user already keeps somewhere else.
- `add`: extra moves you're confident about. Keep them inside the user's own folders.

### 3. Build the page and show it

```bash
python scripts/build_page.py "<out>/survey.json" --notes "<out>/notes.json" --out "<out>/tidy-plan.html"
```

This writes the page and `plan.json` (the numbered actions and the plan id) next to it.

- **If you can publish artifacts** (for example Claude's Artifact tool), publish `tidy-plan.html` as a private artifact and give the user the link. Don't share it further.
- **Otherwise** open the file: `start "" "<out>\tidy-plan.html"` in cmd, or `Invoke-Item "<out>\tidy-plan.html"` in PowerShell.

Tell the user in a few lines what you found. Ask them to tick what they want and press **Copy approval**, then paste it back. Every list has Tick all and Untick all, and ticks are remembered in that browser.

### 4. Get the approval line

It looks like `TIDY-APPROVE plan=3f9a1c07be ids=1-40,52,60-75`, plus an optional `note:`. If the note asks for changes, update `notes.json`, rebuild (the plan id changes) and ask for a new approval. If they downloaded the approval instead, use `--approval <file>.json` below.

### 5. Dry run, then apply

```bash
python scripts/apply_plan.py "<out>/plan.json" --approve "TIDY-APPROVE plan=... ids=..."
```

The dry run prints WOULD, SKIP or REFUSED for each action and changes nothing. If anything is REFUSED, or more than a few are skipped, tell the user why before going on. Otherwise apply:

```bash
python scripts/apply_plan.py "<out>/plan.json" --approve "TIDY-APPROVE plan=... ids=..." --apply
```

- The undo log goes to `Documents\Tidy-up logs\<date time>\` (`undo-log.csv` and `revert.ps1`). Change it with `--log-dir`.
- `revert.ps1` is the same fixed script every time. It holds no file names; it reads `undo-log.csv` and moves things back only where the original place is still free.
- Links (symlinks, junctions) are never moved, recycled or walked through. A name that ends in a dot or a space is moved under its exact name but can't go to the Recycle Bin; it's skipped with a note.
- Names are never overwritten: a clash gets " (2)".
- A move to another drive copies, checks the size, then sends the original to the Recycle Bin. Whole folders only cross drives with `--allow-cross-drive-folders`.
- Windows shows its own warning if something is too big for the Recycle Bin. Let the user answer it.

### 6. Report, hand over the undo, clean up

Tell the user: how many items moved, how much went to the Recycle Bin (it stays there until they empty it), anything skipped and why, and the launcher moves left for them to do. Give the log folder and both ways to undo. Each has a preview: `-Preview` for the script, leaving out `--apply` for Python.

```
powershell -ExecutionPolicy Bypass -File "<log folder>\revert.ps1"
python scripts/apply_plan.py --undo "<log folder>" --apply
```

Items sent to the Recycle Bin are listed as MANUAL: the user restores those from the Recycle Bin.

Then delete the output folder (`survey.json`, `plan.json`, the page) unless the user wants to keep the page.

## Where things go by default

| Found | Goes to |
| --- | --- |
| Contracts, IDs, tax, invoices, payslips, leases, insurance, medical, vehicle, CVs lying loose | `Documents\Important\<category>` (tax and money by year). Reuses folders the user already has, like `Documents\Taxes`. |
| Other loose documents | `Documents\<kind>`: PDFs, Word documents, Spreadsheets, Presentations, Notes, E-books |
| Loose pictures and videos | `Pictures\Screenshots`, `Pictures\<year>`, `Videos\Screen recordings`, `Videos\<year>` |
| Loose audio | `Music` |
| Games no launcher manages, outside a Games folder | `<same drive>\Games\<game>`, unticked by default |
| Code projects on the Desktop or in Downloads | `~\Projects` (or the user's existing `source\repos`, `code`, `dev`), unticked |
| Installers over a month old, zips already extracted, duplicate copies in Downloads/Desktop | Recycle Bin |
| Empty folders in the user's folders | Removed (only those in Documents are unticked by default) |

"Loose" means sitting directly on the Desktop, in Downloads, in the profile folder or at a drive root. Files already filed in subfolders stay where they are. They're counted in the charts, not moved.

## Files

- `scripts/survey.py`: read-only survey, writes `survey.json`.
- `scripts/build_page.py`: builds `tidy-plan.html` and `plan.json` from the survey and your notes.
- `scripts/summarize.py`: short digest of a survey for step 2.
- `scripts/apply_plan.py`: dry run, apply, undo. Enforces the hard rules.
- `scripts/revert.ps1`: the fixed revert script copied next to each undo log.
- `assets/tidy-plan-template.html`: the page. Self-contained, works offline, light and dark.
- [references/review-guide.md](references/review-guide.md): inspecting the survey, the notes.json format, judgement calls.

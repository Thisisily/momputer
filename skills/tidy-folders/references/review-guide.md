# Reviewing a survey

## Contents
- Look at the survey without reading it whole
- What the survey contains
- notes.json format
- Judgement calls

## Look at the survey without reading it whole

```bash
python scripts/summarize.py "<out>/survey.json"                       # overview, 8 samples per group
python scripts/summarize.py "<out>/survey.json" --group important --limit 80
python scripts/summarize.py "<out>/survey.json" --section games        # games | important | naming | areas | big
```

Paths under the user's profile print as `~\...`. Suggestion ids (`#123`) are what `notes.json` refers to.

## What the survey contains

| Key | What it is |
| --- | --- |
| `budget` | entries looked at, seconds, `truncated` (time or entry limit hit), `hash_truncated`, content peeks, errors |
| `known_folders` | real paths of Desktop, Documents, Downloads, Pictures, Videos, Music, Profile, OneDrive (they may be redirected into OneDrive) |
| `targets` | destination roots the plan uses (Important, Projects...) |
| `areas` | one entry per surveyed folder: size, file count, loose files, size and count by type, age buckets, biggest subfolders, `truncated`, `program_like` (left alone) |
| `games.launchers` | each launcher: libraries, games (size where the launcher records it), how to move |
| `games.loose` | game folders found by engine files; `managed_by` set for itch app installs; `in_games_folder` |
| `important_docs` | every likely important paper, filed or loose: category, matched by `name` or `content` |
| `duplicates` | same-content groups; `method` is `sha256` (whole file compared) or `sampled` (over 256 MB: same size and 16 samples) |
| `installers`, `big_files`, `projects`, `naming_issues` | as named |
| `suggestions` | the draft plan: `id`, `group`, `op` (move, recycle, remove-empty-dir), `from`, `to`, `why`, `default` (ticked at first), `risk` (low or medium) |

## notes.json format

```json
{
  "headline": "Downloads holds 1,250 loose files; 6 GB of them haven't changed in over a year.",
  "highlights": [
    "2 passport scans and a signed lease are sitting on the Desktop.",
    "Steam games are spread over two drives; Steam can move them into one library.",
    "A 1.4 GB disk image sits at the top of D:\\ and again in D:\\Installers."
  ],
  "naming_rules": ["Optional. Replaces the page's default house style list."],
  "drop": [12, 40, 41],
  "retarget": {"15": "C:\\Users\\me\\OneDrive\\Documents\\Taxes\\2024\\p60.pdf"},
  "add": [
    {"op": "move", "from": "C:\\Users\\me\\Desktop\\Lease 2024", "to": "C:\\Users\\me\\Documents\\Important\\Housing\\Lease 2024",
     "why": "Folder of lease papers on the Desktop", "risk": "low", "default": true}
  ]
}
```

- JSON needs doubled backslashes in paths.
- `retarget` gives the full new path including the file name.
- `add` entries get new ids after the existing ones and land in the "Other suggestions" group. `op` is `move`, `recycle` or `remove-empty-dir`.
- Rebuilding after any change gives a new plan id, so an old approval can't apply to a changed plan.

## Judgement calls

- **Important papers are matched by name.** Drop obvious misses: app icons (`invoice-icon@2x.png`), program output with a keyword in its name (`certificate-check.pdf`), blank templates. Papers that belong to someone else (a CV a friend sent) are still worth filing, but say so in a highlight so the user can untick them.
- **Respect folders the user already keeps.** If they have `Documents\Taxes` or `OneDrive\Admin`, retarget there rather than creating `Documents\Important\Tax`. The survey already reuses exact-name matches inside Documents.
- **Don't add moves into or out of** backup folders, File History, synced app folders (Creative Cloud Files, Dropbox, Google Drive) or anything the areas list marks as a program folder.
- **OneDrive online-only files.** Moving them within OneDrive is fine. Moving them out of OneDrive downloads them first. Mention it if the plan moves many.
- **Games.** Launcher games are never in the plan. If a launcher has libraries on several drives, that's a highlight ("use Steam > Settings > Storage > Move"), not a move. Loose games are suggested into `<drive>\Games` on the same drive so the move is instant. If the user keeps games on another drive (for example `D:\Games`), say so and offer to retarget, but cross-drive folder moves need `--allow-cross-drive-folders` and take time.
- **Large groups.** Thousands of loose pictures from Downloads are normal. The page pages through them. Mention the count and total size in the headline rather than listing them.
- **Duplicates.** Only copies in messy places (Downloads, Desktop, profile root, drive roots) are suggested. `sampled` matches start unticked. Never suggest recycling a copy inside a backup.
- **Truncated survey.** If `budget.truncated` is true, say which areas were only partly counted (the page does too) and offer a longer rerun.

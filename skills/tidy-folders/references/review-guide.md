# Reviewing a survey

## Contents
- Look at the survey without reading it whole
- What the survey contains
- notes.json format
- Writing mom's take
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
| `areas` | one entry per surveyed folder: size, file count, loose files, size and count by type, age buckets, biggest subfolders, `truncated`, `program_like` (left alone), `years` (files and bytes by type for each year they last changed, counted from `years_from`; `older` is everything before), `loose_by_type` and `loose_age` (the loose files by type and by the age buckets) |
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
  "insights": [
    {"title": "Downloads is where things go to stay",
     "text": "1,250 files sit at the top of Downloads and 900 haven't changed since 2023. Filing the papers and letting old installers go would leave it nearly empty."},
    {"title": "2 passport scans are on the Desktop",
     "text": "With a signed lease next to them. The plan files all 3 under Documents\\Important, where a backup will find them."},
    {"title": "Steam games live on 2 drives",
     "text": "C: holds 4 and D: holds 9. Steam > Settings > Storage can gather them into one library; nothing here moves them."}
  ],
  "places": [
    {"path": "C:\\Users\\me\\OneDrive\\Documents\\Taxes", "note": "Your own tax folder: new tax papers are filed here."}
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
- `highlights` (a list of plain sentences) still works: each shows as an insight without a title.
- A `places` note shows beside the folder with that exact path in the page's charts: the main folders, the profile's other folders, the top of each drive.
- `retarget` gives the full new path including the file name.
- `add` entries get new ids after the existing ones and land in the "Other suggestions" group. `op` is `move`, `recycle` or `remove-empty-dir`.
- Rebuilding after any change gives a new plan id, so an old approval can't apply to a changed plan.

## Writing mom's take

The page already counts and draws everything. Mom's take is what only someone who looked can say: 3 to 6 insights, each a title (under 10 words) and a text (under 40).

- **What a folder is really for.** "Your Desktop is a screenshot shelf: 34 of its 41 files are screenshots." "D:\Old laptop is a 2019 backup, not clutter."
- **Habits over time.** "Downloads filled up after March 2022 and hasn't been emptied since." `summarize.py --section areas` prints files by year and the loose files' ages.
- **What's unusual.** Papers that belong to someone else, the same installer downloaded 3 times, a 4 GB video on the Desktop.
- **What would help.** End with the next step: what the plan does about it, or what's left for them (a launcher's own Move).
- **Kind, never scolding.** It's their computer, and a pile means they were busy. Say "easy to find" and "ready for a backup", never "messy" or "neglected".
- **Digits, not words**, and the folder's own name. Naming a kind (papers, pictures, videos, music, installers, code) lights it on the page.
- Never quote what's inside a file. Names and counts are enough.

## Judgement calls

- **Important papers are matched by name.** Drop obvious misses: app icons (`invoice-icon@2x.png`), program output with a keyword in its name (`certificate-check.pdf`), blank templates. Papers that belong to someone else (a CV a friend sent) are still worth filing, but say so in an insight so the user can untick them.
- **Respect folders the user already keeps.** If they have `Documents\Taxes` or `OneDrive\Admin`, retarget there rather than creating `Documents\Important\Tax`. The survey already reuses exact-name matches inside Documents.
- **Don't add moves into or out of** backup folders, File History, synced app folders (Creative Cloud Files, Dropbox, Google Drive) or anything the areas list marks as a program folder.
- **OneDrive online-only files.** Moving them within OneDrive is fine. Moving them out of OneDrive downloads them first. Mention it if the plan moves many.
- **Games.** Launcher games are never in the plan. If a launcher has libraries on several drives, that's an insight ("use Steam > Settings > Storage > Move"), not a move. Loose games are suggested into `<drive>\Games` on the same drive so the move is instant. If the user keeps games on another drive (for example `D:\Games`), say so and offer to retarget, but cross-drive folder moves need `--allow-cross-drive-folders` and take time.
- **Large groups.** Thousands of loose pictures from Downloads are normal. The page pages through them. Mention the count and total size in the headline rather than listing them.
- **Duplicates.** Only copies in messy places (Downloads, Desktop, profile root, drive roots) are suggested. `sampled` matches start unticked. Never suggest recycling a copy inside a backup.
- **Truncated survey.** If `budget.truncated` is true, say which areas were only partly counted (the page does too) and offer a longer rerun.

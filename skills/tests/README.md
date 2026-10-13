# Tests for the skills

These check the careful parts: the shared file-moving engine, each skill's survey, page and plan, and the mail-search rule. They are for people changing a skill. A skill doesn't need them to work, and nobody installing a skill needs this folder.

## What they touch

**Only a scratch folder you name on the command line.** Each test makes its own work folder inside it, with a marker file, works there, and removes it at the end. A test refuses a work folder it didn't make. Nothing else on the PC is created, moved, renamed or deleted:

- Every apply and undo runs on a made-up profile inside the work folder, using the scripts' `--root` test mode.
- Nothing goes to the real Recycle Bin. Where a plan sends things there, the test swaps the engine's Recycle Bin call for one that moves the file to a folder inside the work folder.
- A few checks only *look* outside it, and say so: the short names of `Program Files` and `AppData`, whether `C:\Windows\win.ini` is still there, this PC's all-users Start menu shortcuts (read to check the shortcut reader; no names kept), and a path on a second drive that is asked about, never created.
- One check makes a folder unlistable with `icacls` and puts the permission back afterwards, whatever happens.
- The page check starts its own headless Edge or Chrome with a throwaway profile folder inside the scratch folder, stops that browser (only that one) and deletes the profile.

The scratch folder must already exist and, for `test_safe_apply.py`, be under `%TEMP%`: those checks rely on Temp being a protected place.

## Running them

Windows, Python 3.8 or later, standard library only. Run with `-I` so nothing from the current folder or your environment is picked up.

```
mkdir %TEMP%\skill-tests
python -I skills\tests\run_all.py %TEMP%\skill-tests
```

That runs everything below against every skill and prints one line each. `--no-browser` skips the page checks; `--keep` leaves the work folders for a look.

| Test | What it checks |
| --- | --- |
| `check_shared.py [skills folder]` | Every skill's `scripts/safe_apply.py` and `scripts/revert.ps1` is byte-for-byte the copy in `tidy-folders`. Reads only |
| `test_safe_apply.py <scratch> <skill folder> [--entry apply_plan.py]` | One skill's copy of the engine and `revert.ps1`: hostile plan actions refused (network and device paths, short names, slashes, `..`, junctions, protected places), a plan or an undo log edited afterwards, the plan fingerprint and approval line, `--root`, `--within`, a skill's own extra rules, anything that can't be examined, apply and both undo routes with awkward names |
| `test_tidy_folders.py <scratch> [skill folder]` | tidy-folders end to end: survey, page, dry run, apply, `revert.ps1`, apply, `--undo`, the tree restored exactly, the page in a browser |
| `test_desktop_icons.py <scratch> [skill folder]` | desktop-icons end to end, with shortcut files written by hand (working, dead, cut short, hostile), shortcuts made by Windows, notes that must stop the build, and what `apply_plan.py` refuses |
| `test_rename_documents.py <scratch> [skill folder]` | rename-documents end to end: which names say nothing, the peeks, a PDF whose text is instructions, every bad name refused before a plan exists and again by `apply_plan.py` |
| `test_inbox_queries.py <scratch> [skill folder]` | inbox-cleanup never puts an address a mail search would misread (`-x@...`) into a query |
| `check_page.py <page.html> <scratch> --prefix TIDY-APPROVE [--plan plan.json] [--shots folder]` | A built page in a headless browser: no script errors, no network requests, no sideways scroll at 390 and 1280 px, Tick all and Untick all on every list, the approval line, light and dark. The three end-to-end tests call it |

Each prints `ok:` or `FAIL:` per check, then `ALL PASSED: N checks` or what failed, and exits 0 or 1. A line starting `not run:` names something that could not be checked on this PC (for example, a disk that makes no short names for new folders, or no second drive).

## What they don't check

- A real send to the Recycle Bin (the Windows call itself is never made).
- A move to another drive for real: there is nothing to move to. The checks that matter there (a folder holding a link is never copied) run with the "same drive?" answer swapped.
- Short names for folders the test makes, on a disk that doesn't create them.
- OneDrive online-only files.

## When you change the engine

`tidy-folders/scripts/safe_apply.py` and `revert.ps1` are the one implementation. Change them there, run `test_safe_apply.py` on tidy-folders, copy both files unchanged into every other skill that has them, and run `check_shared.py`. Never edit a copy.

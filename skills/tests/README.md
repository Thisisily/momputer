# Tests for the skills

These check the careful parts: the shared file-moving engine, each skill's survey, page and plan, the mail-search rule, the statement reader and the dead-link checker. They are for people changing a skill. A skill doesn't need them to work, and nobody installing a skill needs this folder.

## What they touch

**Only a scratch folder you name on the command line.** Each test makes its own work folder inside it, with a marker file, works there, and removes it at the end. A test refuses a work folder it didn't make. Nothing else on the PC is created, moved, renamed or deleted:

- Every apply and undo runs on a made-up profile inside the work folder, using the scripts' `--root` test mode.
- Plans and undo logs that name a real place (Windows' own folders, the real AppData, a drive's top) only ever go through a dry run or a preview, to check they are refused. The engine test stops before its first real run if any check has failed by then, and refuses to run a plan or log that names anything outside its work folder.
- Nothing goes to the real Recycle Bin. Where a plan sends things there, the test swaps the engine's Recycle Bin call for one that moves the file to a folder inside the work folder.
- A few checks only *look* outside it, and say so: the short names of `Program Files` and `AppData`, whether `C:\Windows\win.ini` is still there, this PC's all-users Start menu shortcuts (read to check the shortcut reader; no names kept), and a path on a second drive that is asked about, never created.
- One check makes a folder unlistable with `icacls` and puts the permission back afterwards, whatever happens.
- The page checks start their own headless Edge or Chrome with a throwaway profile folder inside the scratch folder, stop that browser (only that one) and delete the profile. The browser is driven over a pipe, so no debugging port is opened on the PC. Copy approval never reaches the real clipboard: the page's clipboard call is swapped for one that keeps the text.
- No mailbox, bank statement or browser profile of yours is read. The statements, mail finds and bookmarks are made up by `_gen_subscriptions.py` and `_gen_bookmarks.py`.
- bookmark-cleanup's scripts are run with `LOCALAPPDATA` and the home folder pointing into the work folder, so they never look at a real browser's folders.
- The link checker's test contacts no site. The checker meets a pretend network, a socket pair inside the test, and a small server on `127.0.0.1` that must stay unvisited. That server listens on a port the system picks, and on port 80 too unless something already answers there (it looks first). The only request it gets is the test's own, at the end. The real checker is run only on addresses it has first said it would not ask. Names that must count as public are `example.net` and `example.org`.

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
| `test_subscription_audit.py <scratch> [skill folder]` | subscription-audit end to end on made-up statements and mail finds: the fixed mail searches, what the statement reader keeps and what it leaves out (card numbers, IBANs, balances), what the report finds, the approval line accepted and refused, the page in a browser |
| `test_subscription_audit_reader.py <scratch> [skill folder]` | Its statement reader on many layouts: it asks instead of guessing a sign, a column, a date order or a decimal mark, prints no figures when it asks, scrubs card and account numbers, and writes only where it may |
| `test_subscription_audit_detect.py <scratch> [skill folder]` | Its report builder: what is a subscription and what is shopping, price rises, two plans, stopped plans, one payment counted once, odd input, which files it may write over, an approval that covers every field of the plan |
| `test_bookmark_cleanup.py <scratch> [skill folder]` | bookmark-cleanup end to end on a made-up browser profile and an exported file: read a copy, report, a new file from an approval line with nothing lost or invented, hostile titles and addresses kept as text, changed plans and lines refused, the page and the new files in a browser |
| `test_bookmark_cleanup_links.py <scratch> [skill folder]` | Its dead-link checker, without contacting a site: which addresses may be asked at all, redirects and errors on a pretend network, what a request holds, a local server that must stay unvisited |
| `test_calendar_fixer.py <scratch> [skill folder]` | calendar-fixer end to end on a made-up calendar (`_gen_calendar.py`): what it counts, what it flags, and that a new time is never offered on top of anything that makes you busy (an all-day event, Out of office, focus time), the page in a browser |
| `test_drive_audit.py <scratch> [skill folder]` | drive-audit end to end on a made-up Drive listing (`_gen_drive.py`): duplicates, stale and empty files, the copy that stays never offered for the Trash, an approval that would trash a kept copy refused, the page in a browser |
| `test_disk_space_report.py <scratch> [skill folder]` | disk-space-report end to end on a made-up profile with sparse files: the survey only reads, no name reaches a command, notes stay text, the dry run starts nothing and refuses a wrong id, a changed plan and a malformed line, the page in a browser. `run_plan.py` never gets `--run` |
| `test_backup_checkup.py <scratch> [skill folder]` | backup-checkup end to end on a made-up profile: the survey only reads, no name reaches a command, notes stay text, the chart numbers add up, the dry run starts nothing and refuses what it should, the page in a browser. `run_plan.py` never gets `--run` |
| `test_inbox_cleanup.py <scratch> [skill folder]` | inbox-cleanup end to end on a made-up mailbox (`_gen_inbox.py`): hostile senders and subjects stay text and never reach a query, the dashboard numbers add up, the approval line accepted and refused, the page in a browser |
| `../_kit/sync.py --check` | Every page built on the brand kit holds its current copy. Reads only |
| `check_page.py <page.html> <scratch> --prefix TIDY-APPROVE [--plan plan.json] [--shots folder]` | A built page in a headless browser: no script errors, no network requests, no sideways scroll at 390 and 1280 px, Tick all and Untick all on every list, the approval line, light and dark. The three end-to-end tests of the file-moving skills call it. The subscription-audit and bookmark-cleanup pages have another approval line, so their tests check them themselves, with the same browser code (`_report_page.py`) |

Each prints `ok:` or `FAIL:` per check, then `ALL PASSED: N checks` or what failed, and exits 0 or 1. A line starting `not run:` names something that could not be checked on this PC (for example, a disk that makes no short names for new folders, no second drive, or port 80 in use).

The subscription-audit statements are dated 2024 to 2026, and the reader compares a statement's dates with today's. Those tests hold until 2036.

## What they don't check

- A real send to the Recycle Bin (the Windows call itself is never made).
- A move to another drive for real: there is nothing to move to. The checks that matter there (a folder holding a link is never copied) run with the "same drive?" answer swapped.
- Short names for folders the test makes, on a disk that doesn't create them.
- OneDrive online-only files.
- A real mailbox, a real bank's export or a real browser's bookmarks: all three are made up.
- A real request by the link checker: it never gets past the pretend network and the local server.
- Importing the new bookmarks file into a browser.

## When you change the engine

`tidy-folders/scripts/safe_apply.py` and `revert.ps1` are the one implementation. Change them there, run `test_safe_apply.py` on tidy-folders, copy both files unchanged into every other skill that has them, and run `check_shared.py`. Never edit a copy.

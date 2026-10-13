#!/usr/bin/env python3
"""Check an approval against a backup plan and open only the ticked Windows pages.

    python run_plan.py backup-plan.json --approve "BACKUP-APPROVE plan=<id> ids=1,4-6"          # dry run
    python run_plan.py backup-plan.json --approve "BACKUP-APPROVE plan=<id> ids=1,4-6" --run

This script copies, uploads, syncs and changes nothing. For each approved id it does one of:
  * opens one of Windows' own pages (the constant argument lists in TABLE) for the person to use,
  * prints "for you to do".
Nothing from the plan file is ever run or put into a command: an item only names a TABLE entry.
A plan changed after the page was built no longer matches its id and is refused.

Python 3.8+, standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

PLAN_SCHEMA = "backup-checkup/plan@1"
APPROVE_RX = re.compile(r"^\s*BACKUP-APPROVE plan=([0-9a-f]{10}) ids=(none|\d{1,6}(?:-\d{1,6})?(?:,\d{1,6}(?:-\d{1,6})?)*)\s*$")

# The only programs this script can start: Windows' own pages, found in the Windows folder.
TABLE = {
    "open-windows-backup": {
        "argv": ["explorer.exe", "ms-settings:backup"], "where": "windows", "title": "Open Windows' backup settings",
        "does": "Opens Settings on the backup page (Windows 11: Accounts > Windows backup; Windows 10: Update & "
                "Security > Backup). OneDrive folder backup for Desktop, Documents and Pictures is switched on "
                "there. The clicks are yours."},
    "open-file-history": {
        "argv": ["control.exe", "/name", "Microsoft.FileHistory"], "where": "system32", "title": "Open File History",
        "does": "Opens File History in Control Panel. There you pick the drive, turn it on, or press Run now. The "
                "clicks are yours."},
}
# What a "for you" item is, in this script's own words (titles in the plan file hold folder names, which are data).
GUIDES = {
    "copy-folder": "Copy a folder to a second drive",
    "get-a-drive": "Get somewhere to keep a second copy",
    "push-repo": "Push a repository, or give it a remote",
    "start-client": "Start a sync program that isn't running",
    "check-client": "Check what a backup or sync program covers",
    "connect-drive": "Connect the backup drive",
}


def plan_id(actions):
    """Fingerprint of the whole action list. Any edit to the plan changes it."""
    blob = json.dumps(actions, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(blob.encode("utf-8")).hexdigest()[:10]


def shown(entry):
    return " ".join(entry["argv"])


def expand_ids(text):
    ids = []
    if text == "none":
        return ids
    for part in text.split(","):
        a, _, b = part.partition("-")
        lo, hi = int(a), int(b or a)
        if hi < lo or hi - lo > 100000:
            raise ValueError("bad id range: %s" % part)
        ids.extend(range(lo, hi + 1))
    return sorted(set(ids))


def windows_dir():
    if os.name != "nt":
        return None
    import ctypes
    buf = ctypes.create_unicode_buffer(260)
    n = ctypes.windll.kernel32.GetSystemWindowsDirectoryW(buf, 260)
    return buf.value if 0 < n < 260 else None


def find_tool(entry):
    """Full path of the Windows program a TABLE entry starts, or None. PATH and the current folder are never searched."""
    win = windows_dir()
    if not win:
        return None
    p = os.path.join(win, entry["argv"][0]) if entry["where"] == "windows" else os.path.join(win, "System32", entry["argv"][0])
    return p if os.path.isfile(p) else None


def load_plan(path):
    with open(path, encoding="utf-8") as f:
        plan = json.load(f)
    if not isinstance(plan, dict) or plan.get("schema") != PLAN_SCHEMA or not isinstance(plan.get("actions"), list):
        sys.exit("REFUSED: %s is not a backup-checkup plan." % path)
    ids = [a.get("id") if isinstance(a, dict) else None for a in plan["actions"]]
    if any(not isinstance(i, int) or isinstance(i, bool) for i in ids) or len(set(ids)) != len(ids):
        sys.exit("REFUSED: the plan's items are not numbered properly.")
    real = plan_id(plan["actions"])
    if plan.get("plan_id") != real:
        sys.exit("REFUSED: the plan file was changed after the page was built (its id no longer matches its "
                 "contents). Build the page again and ask for a new approval.")
    return plan, real


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Open the approved Windows pages of a backup plan; print the rest.")
    ap.add_argument("plan", help="backup-plan.json written by build_page.py")
    ap.add_argument("--approve", required=True, help='the line the page copied: "BACKUP-APPROVE plan=<id> ids=..."')
    ap.add_argument("--run", action="store_true", help="really open the pages (default: dry run, prints only)")
    args = ap.parse_args(argv)

    plan, real = load_plan(args.plan)
    m = APPROVE_RX.match((args.approve.strip().splitlines() or [""])[0])
    if not m:
        sys.exit('REFUSED: the approval must look like "BACKUP-APPROVE plan=<id> ids=1,4-6".')
    if m.group(1) != real:
        sys.exit("REFUSED: this approval is for plan %s, but the plan file is %s. Show the current page again and "
                 "ask for a new approval." % (m.group(1), real))
    try:
        wanted = expand_ids(m.group(2))
    except ValueError as ex:
        sys.exit("REFUSED: %s" % ex)
    by_id = {a["id"]: a for a in plan["actions"]}
    print("%s plan %s: %d item%s approved" % ("RUN" if args.run else "DRY RUN (nothing is opened)", real, len(wanted),
                                             "" if len(wanted) == 1 else "s"))
    failed = 0
    for i in wanted:
        a = by_id.get(i)
        if a is None:
            print("SKIP #%d: no such item in the plan" % i)
            continue
        kind = a.get("kind")
        if kind == "guide":
            label = GUIDES.get(a.get("guide_id")) if isinstance(a.get("guide_id"), str) else None
            print("FOR YOU #%d: %s. The page says which one and how; this script does nothing for it." % (
                i, label or "Something for you to do"))
            continue
        cid = a.get("command_id")
        entry = TABLE.get(cid) if kind == "open" and isinstance(cid, str) else None
        if entry is None:
            print("REFUSED #%d: not one of this script's fixed Windows pages. Nothing was opened for it." % i)
            failed += 1
            continue
        exe = find_tool(entry)
        if exe is None:
            print("SKIP #%d: %s was not found in the Windows folder" % (i, entry["argv"][0]))
            continue
        if not args.run:
            print("WOULD OPEN #%d: %s\n      %s" % (i, shown(entry), entry["does"]))
            continue
        try:
            subprocess.Popen([exe] + entry["argv"][1:], cwd=os.path.expanduser("~"), stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, shell=False)
            print("OPENED #%d: %s (for you to use; nothing is changed until you change it there)" % (i, entry["title"]))
        except OSError as ex:
            print("FAILED #%d: %s could not be opened (%s)" % (i, entry["title"], type(ex).__name__))
            failed += 1
    if not args.run:
        print("Dry run only. Add --run to open the pages.")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

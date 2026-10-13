#!/usr/bin/env python3
"""Do the ticked items of a disk-space plan. Only the fixed commands in TABLE can ever run.

    python run_plan.py diskspace-plan.json --approve "DISKSPACE-APPROVE plan=<id> ids=1-3,7"          # dry run
    python run_plan.py diskspace-plan.json --approve "DISKSPACE-APPROVE plan=<id> ids=1-3,7" --run

This script deletes, moves and empties nothing itself. For each approved id it does one of:
  * runs one cleanup command of the tool that owns the data (npm, pip, docker ...),
  * opens one of Windows' own tools for the person to use,
  * prints "for you to do".
The commands are the constant argument lists in TABLE, started without a shell. Nothing
from the plan file is ever run or put into a command: an item only names a TABLE entry.
A plan changed after the page was built no longer matches its id and is refused.

Python 3.8+, standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from datetime import datetime

PLAN_SCHEMA = "disk-space-report/plan@1"
APPROVE_RX = re.compile(r"^\s*DISKSPACE-APPROVE plan=([0-9a-f]{10}) ids=(none|\d{1,6}(?:-\d{1,6})?(?:,\d{1,6}(?:-\d{1,6})?)*)\s*$")
# A .cmd or .bat tool is started through cmd.exe by Windows, which reads some characters in the tool's
# own path as commands. Such a tool is only started from a plainly named folder (see plain_enough).
PLAIN_PATH_RX = re.compile(r"^[A-Za-z]:(\\[\w .\-~+@#()]+)+$")

# id -> a constant command. "covers" names the survey findings the command cleans (see survey.py RULES).
# kind "command": the owning tool's own cleanup, found on PATH. kind "open": a Windows tool, for the person.
TABLE = {
    "npm-cache-clean": {
        "kind": "command", "argv": ["npm", "cache", "clean", "--force"], "covers": ["npm-cache"], "risk": "low",
        "title": "Clear npm's download cache",
        "removes": "Every package npm has downloaded. npm fetches them again when a project needs them, so the next "
                   "installs are slower and need the internet. Projects' own node_modules folders are not touched."},
    "pnpm-store-prune": {
        "kind": "command", "argv": ["pnpm", "store", "prune"], "covers": ["pnpm-store"], "risk": "low",
        "title": "Prune pnpm's store",
        "removes": "Packages in pnpm's store that no project on this PC uses any more. Packages still in use stay."},
    "yarn-cache-clean": {
        "kind": "command", "argv": ["yarn", "cache", "clean"], "covers": ["yarn-cache"], "risk": "low",
        "title": "Clear Yarn's cache",
        "removes": "Every package Yarn has downloaded. Yarn fetches them again when needed (needs the internet)."},
    "pip-cache-purge": {
        "kind": "command", "argv": ["pip", "cache", "purge"], "covers": ["pip-cache"], "risk": "low",
        "title": "Clear pip's cache",
        "removes": "Downloaded and locally built Python packages. Installed packages stay; pip downloads again when needed."},
    "uv-cache-clean": {
        "kind": "command", "argv": ["uv", "cache", "clean"], "covers": ["uv-cache"], "risk": "low",
        "title": "Clear uv's cache",
        "removes": "Everything in uv's cache. Existing virtual environments keep working; uv downloads again when needed."},
    "nuget-locals-clear": {
        "kind": "command", "argv": ["dotnet", "nuget", "locals", "all", "--clear"], "covers": ["nuget-packages"],
        "risk": "low", "title": "Clear NuGet's packages and caches",
        "removes": "Every restored .NET package and NuGet's caches. The next build restores what it needs (needs the internet)."},
    "go-clean-modcache": {
        "kind": "command", "argv": ["go", "clean", "-modcache"], "covers": ["go-modcache"], "risk": "low",
        "title": "Clear Go's module cache",
        "removes": "Every downloaded Go module. Go downloads them again on the next build (needs the internet)."},
    "go-clean-cache": {
        "kind": "command", "argv": ["go", "clean", "-cache"], "covers": ["go-build-cache"], "risk": "low",
        "title": "Clear Go's build cache",
        "removes": "Compiled pieces Go kept. The next builds are slower once."},
    "conda-clean": {
        "kind": "command", "argv": ["conda", "clean", "--all", "--yes"], "covers": ["conda-pkgs"], "risk": "low",
        "title": "Clear conda's package cache",
        "removes": "Downloaded package files and unused cached packages. Existing environments keep working."},
    "docker-builder-prune": {
        "kind": "command", "argv": ["docker", "builder", "prune", "--force"], "covers": ["docker-disk"], "risk": "low",
        "frees": "unknown", "title": "Clear Docker's build cache",
        "removes": "Docker's build cache only. Images, containers and volumes stay. Docker Desktop must be running. "
                   "The disk image file itself gets smaller only after it is compacted."},
    "docker-system-prune": {
        "kind": "command", "argv": ["docker", "system", "prune", "--force"], "covers": ["docker-disk"], "risk": "medium",
        "frees": "unknown", "title": "Prune Docker (stopped containers, unused networks, unnamed images, build cache)",
        "removes": "Every stopped container, with anything saved only inside it, plus networks nothing uses, images "
                   "with no name and the build cache. Named images and volumes stay. Docker Desktop must be running. "
                   "The disk image file itself gets smaller only after it is compacted."},
    "wsl-shutdown": {
        "kind": "command", "argv": ["wsl", "--shutdown"], "covers": ["wsl-disk", "docker-disk"], "risk": "medium",
        "frees": "none", "where": "system32", "title": "Shut down WSL (the step before compacting a disk image)",
        "removes": "Nothing. It stops every running Linux distribution and Docker Desktop's engine, so unsaved work "
                   "inside them is lost. Compacting the disk image afterwards is for you to do; the page says how."},
    "open-storage-settings": {
        "kind": "open", "argv": ["explorer.exe", "ms-settings:storagesense"], "where": "windows", "risk": "low",
        "covers": ["user-temp", "windows-temp", "windows-update-cache", "windows-old", "delivery-optimization",
                   "windows-caches", "crash-dumps"],
        "title": "Open Storage settings",
        "removes": "Nothing by itself. Windows shows temporary files, update leftovers and a previous Windows "
                   "installation there, and you choose what it removes."},
    "open-disk-cleanup": {
        "kind": "open", "argv": ["cleanmgr.exe"], "where": "system32", "risk": "low",
        "covers": ["windows-update-cache", "windows-old", "memory-dumps", "gpu-shader-cache", "delivery-optimization",
                   "windows-caches", "windows-logs"],
        "title": "Open Disk Cleanup",
        "removes": "Nothing by itself. It lists update leftovers, memory dumps, the DirectX shader cache, Delivery "
                   "Optimization files and thumbnails for you to tick. Its 'Clean up system files' button needs an "
                   "administrator: that click is yours."},
    "open-recycle-bin": {
        "kind": "open", "argv": ["explorer.exe", "shell:RecycleBinFolder"], "where": "windows", "risk": "low",
        "covers": ["recycle-bin"], "title": "Open the Recycle Bin",
        "removes": "Nothing by itself. Look through it, then use Empty Recycle Bin yourself."},
    "open-installed-apps": {
        "kind": "open", "argv": ["explorer.exe", "ms-settings:appsfeatures"], "where": "windows", "risk": "low",
        "covers": ["game", "steam-downloads"], "title": "Open Apps > Installed apps",
        "removes": "Nothing by itself. Sort by size and uninstall what you no longer use."},
}


def plan_id(actions):
    """Fingerprint of the whole action list. Any edit to the plan changes it."""
    blob = json.dumps(actions, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(blob.encode("utf-8")).hexdigest()[:10]


def shown(entry):
    """The command as the person sees it: the constant words from TABLE."""
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
    """Full path of the program a TABLE entry starts, or None. Never looks in the current folder."""
    name = entry["argv"][0]
    where = entry.get("where", "path")
    if where in ("windows", "system32"):
        win = windows_dir()
        if not win:
            return None
        exe = name if name.lower().endswith(".exe") else name + ".exe"
        # Sysnative: where a 32-bit Python finds the 64-bit System32 (wsl.exe and cleanmgr.exe live only there)
        for p in ([os.path.join(win, exe)] if where == "windows" else
                  [os.path.join(win, "System32", exe), os.path.join(win, "Sysnative", exe)]):
            if os.path.isfile(p):
                return p
        return None
    for d in os.environ.get("PATH", "").split(os.pathsep):
        d = d.strip().strip('"')
        if not d or not os.path.splitdrive(d)[0] or not os.path.isabs(d):
            continue                              # relative entries (and "." itself) are never searched
        for ext in (".exe", ".cmd", ".bat"):
            p = os.path.join(os.path.normpath(d), name + ext)
            if os.path.isfile(p):
                return p
    return None


def plain_enough(path):
    """True if cmd.exe will read this .cmd/.bat path as one plain name. Brackets are only safe inside the
    quotes that a path with a space gets ("Program Files (x86)")."""
    if not PLAIN_PATH_RX.match(path):
        return False
    return " " in path or not ("(" in path or ")" in path)


def clean(text, limit=160):
    """Plan text for printing: one line, no control characters."""
    return re.sub(r"[\x00-\x1f\x7f  ]+", " ", str(text))[:limit]


def human(n):
    if not isinstance(n, int) or isinstance(n, bool) or n < 0:
        return "size unknown"
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%d %s" % (n, unit)) if unit == "B" else ("%.1f %s" % (n, unit))
        n /= 1024.0


def load_plan(path):
    with open(path, encoding="utf-8") as f:
        plan = json.load(f)
    if not isinstance(plan, dict) or plan.get("schema") != PLAN_SCHEMA or not isinstance(plan.get("actions"), list):
        sys.exit("REFUSED: %s is not a disk-space-report plan." % path)
    actions = plan["actions"]
    ids = [a.get("id") if isinstance(a, dict) else None for a in actions]
    if any(not isinstance(i, int) or isinstance(i, bool) for i in ids) or len(set(ids)) != len(ids):
        sys.exit("REFUSED: the plan's items are not numbered properly.")
    real = plan_id(actions)
    if plan.get("plan_id") != real:
        sys.exit("REFUSED: the plan file was changed after the page was built (its id no longer matches its "
                 "contents). Build the page again and ask for a new approval.")
    return plan, real


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Run the approved items of a disk-space plan (fixed commands only).")
    ap.add_argument("plan", help="diskspace-plan.json written by build_page.py")
    ap.add_argument("--approve", required=True, help='the line the page copied: "DISKSPACE-APPROVE plan=<id> ids=..."')
    ap.add_argument("--run", action="store_true", help="really run the commands (default: dry run, prints only)")
    ap.add_argument("--timeout", type=int, default=1800, help="seconds allowed per command (default 1800)")
    ap.add_argument("--log", help="where to write what was done (default: diskspace-run-log.txt next to the plan)")
    args = ap.parse_args(argv)

    plan, real = load_plan(args.plan)
    first = (args.approve.strip().splitlines() or [""])[0]
    m = APPROVE_RX.match(first)
    if not m:
        sys.exit('REFUSED: the approval must look like "DISKSPACE-APPROVE plan=<id> ids=1-3,7".')
    if m.group(1) != real:
        sys.exit("REFUSED: this approval is for plan %s, but the plan file is %s. Show the current page again and "
                 "ask for a new approval." % (m.group(1), real))
    try:
        wanted = expand_ids(m.group(2))
    except ValueError as ex:
        sys.exit("REFUSED: %s" % ex)
    by_id = {a["id"]: a for a in plan["actions"]}
    home = os.path.expanduser("~")
    env = dict(os.environ, NoDefaultCurrentDirectoryInExePath="1")   # cmd.exe must not pick a program up from the current folder
    log_path = args.log or os.path.join(os.path.dirname(os.path.abspath(args.plan)), "diskspace-run-log.txt")
    lines = []

    def say(text):
        print(text)
        lines.append(text)

    def free():
        try:
            return shutil.disk_usage(home).free
        except OSError:
            return None

    before = free()
    say("%s plan %s: %d item%s approved" % ("RUN" if args.run else "DRY RUN (nothing is started)", real, len(wanted),
                                           "" if len(wanted) == 1 else "s"))
    ran = failed = 0
    for i in wanted:
        a = by_id.get(i)
        if a is None:
            say("SKIP #%d: no such item in the plan" % i)
            continue
        kind = a.get("kind")
        if kind == "list":
            say("FOR YOU #%d: %s (%s item%s, %s). The page lists them; this script never touches them." % (
                i, clean(a.get("title", "a list")), a.get("count") if isinstance(a.get("count"), int) else "?",
                "" if a.get("count") == 1 else "s", human(a.get("bytes"))))
            continue
        cid = a.get("command_id")
        entry = TABLE.get(cid) if isinstance(cid, str) else None
        if entry is None or entry["kind"] != kind:
            say("REFUSED #%d: not one of this script's fixed commands. Nothing was run for it." % i)
            failed += 1
            continue
        exe = find_tool(entry)
        verb = "open" if kind == "open" else "run"
        if exe is None:
            say("SKIP #%d: %s is not installed or not on PATH (would %s: %s)" % (i, entry["argv"][0], verb, shown(entry)))
            continue
        if exe.lower().endswith((".cmd", ".bat")) and not plain_enough(exe):
            say("SKIP #%d: %s sits in a folder whose name this script won't hand to cmd.exe. Run it yourself: %s" % (
                i, entry["argv"][0], shown(entry)))
            continue
        if not args.run:
            say("WOULD %s #%d: %s\n      %s" % (verb.upper(), i, shown(entry), entry["removes"]))
            continue
        cmd = [exe] + entry["argv"][1:]
        try:
            if kind == "open":
                subprocess.Popen(cmd, cwd=home, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL, shell=False)
                say("OPENED #%d: %s (for you to use; nothing is removed until you choose it there)" % (i, entry["title"]))
                ran += 1
                continue
            r = subprocess.run(cmd, cwd=home, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, timeout=args.timeout, shell=False)
            tail = [clean(x, 200) for x in r.stdout.decode("utf-8", "replace").splitlines() if x.strip()][-6:]
            say("RAN #%d: %s -> exit code %d" % (i, shown(entry), r.returncode))
            for t in tail:
                say("      | " + t)
            ran += 1
            failed += 1 if r.returncode else 0
        except subprocess.TimeoutExpired:
            say("STOPPED #%d: %s took longer than %d s" % (i, shown(entry), args.timeout))
            failed += 1
        except OSError as ex:
            say("FAILED #%d: %s could not be started (%s)" % (i, shown(entry), type(ex).__name__))
            failed += 1
    after = free()
    if args.run and before is not None and after is not None:
        say("Free space on the drive your profile is on: %s before, %s after." % (human(before), human(after)))
    if args.run:
        try:
            with open(log_path, "a", encoding="utf-8") as f:
                f.write("\n".join(["--- " + datetime.now().isoformat(timespec="seconds")] + lines) + "\n")
            print("log: %s" % log_path)
        except OSError:
            print("log could not be written to %s" % log_path)
    else:
        print("Dry run only. Add --run to do it.")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

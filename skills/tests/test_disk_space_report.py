"""Checks the disk-space-report skill end to end on a made-up PC folder.

    python -I test_disk_space_report.py <scratch folder> [<skill folder>] [--keep] [--no-browser]

It makes a small made-up profile inside a work folder in <scratch folder> (a package cache, a
code project, a game, old installers and a long-untouched video, some with hostile names; the
big files are sparse, so they take almost no room), surveys it with survey.py --root, builds
the page with notes, and checks: the survey only read the folder and wrote one file; no folder
or file name reaches a command; what notes.json says stays text; the plan id is a fingerprint
run_plan.py checks; the dry run starts nothing and refuses a wrong id, a changed plan and a
malformed line. Then uv's lock: with a made-up uv first on PATH (it only writes files in the
work folder), run_plan.py --run starts its "cache clean" when the cache's lock is free or absent,
and starts nothing but prints BUSY while a helper process holds that lock. Then the page in a
headless browser. No real tool is ever given --run.

--keep leaves the work folder (fixture, survey, page, plan, screenshots) for a look.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import ctypes
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, X, Checks, remove_work_folder, run, set_times, tree, work_folder, write  # noqa: E402

MARK = "made-by-test_disk_space_report.txt"
MB = 1024 ** 2
YEARS_AGO = time.time() - 2 * 365 * 86400
# Names a shell would run, and a look-alike quote and angle brackets Windows allows in a name
HOSTILE_EXE = "Lanternfall $(Stop-Computer) & del ~ `whoami` 'x\u2019 ; %TEMP%.exe"
HOSTILE_GAME = "Copperline \uff1cscript\uff1e & echo hi"
HOSTILE_NOTE = "<script>alert(1)</script> & <img src=x onerror=alert(2)>"

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "disk-space-report"))
ap.add_argument("--keep", action="store_true")
ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser check of the page")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
sys.stdout.reconfigure(encoding="ascii", errors="backslashreplace")  # check names hold made-up file names: print them spelled out
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
SURVEY, BUILD, RUN = (os.path.join(S, n) for n in ("survey.py", "build_page.py", "run_plan.py"))
if not os.path.isfile(SURVEY):
    sys.exit("Not found: %s" % SURVEY)
sys.path.insert(0, S)
import run_plan  # noqa: E402  (only its table and fingerprint are used)

c = Checks()
check, note = c.check, c.note
T = work_folder(opts.scratch, "disk-space-report-test", MARK)
PC, OUT = os.path.join(T, "pc"), os.path.join(T, "out")


def py(*args):
    return run([PY, "-I"] + list(args))


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def save(obj, path):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
    return path


def sparse(path, size, when=None):
    """A file of `size` bytes that takes almost no room: marked sparse, then lengthened. Made-up data only."""
    write(path, b"")
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    k.CreateFileW.restype = ctypes.c_void_p
    k.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    k.DeviceIoControl.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32,
                                  ctypes.POINTER(ctypes.c_uint32), ctypes.c_void_p]
    k.CloseHandle.argtypes = [ctypes.c_void_p]
    h = k.CreateFileW(X + path, 0xC0000000, 0, None, 3, 0, None)  # read + write, OPEN_EXISTING
    if h in (None, ctypes.c_void_p(-1).value):
        raise OSError("can't open %s" % path)
    got = ctypes.c_uint32()
    k.DeviceIoControl(h, 0x000900C4, None, 0, None, 0, ctypes.byref(got), None)  # FSCTL_SET_SPARSE
    k.CloseHandle(h)
    with open(X + path, "r+b") as f:
        f.truncate(size)
    if when:
        set_times(path, when)
    return path


def stamps(root):
    """Every name, size and modified time under root: the survey must change none of them."""
    out = []
    for rel, kind in tree(root):
        out.append((rel, kind, os.lstat(X + root + rel).st_mtime_ns))
    return out


def actions_by(plan):
    return {a.get("command_id") or a.get("list_id"): a for a in plan["actions"]}


# A made-up uv: notes each call, answers "cache dir" with the folder the test names, and for anything
# else writes a marker file. It touches nothing outside the work folder.
FAKE_UV = "\r\n".join([
    "@echo off",
    '>>"%FAKE_UV_CALLS%" echo(%*',
    'if /i not "%~1 %~2"=="cache dir" goto clean',
    "echo(%FAKE_UV_CACHE%",
    "exit /b 0",
    ":clean",
    '>"%FAKE_UV_MARK%" echo cleaned',
    "exit /b 0", ""])
# Holds a lock on one byte of the file it is given, the way a running uv holds its cache's .lock,
# until its stdin is closed.
HOLD_LOCK = ("import msvcrt, os, sys\n"
             "fd = os.open(sys.argv[1], os.O_RDWR | os.O_BINARY)\n"
             "msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)\n"
             "print('held', flush=True)\n"
             "sys.stdin.read()\n"
             "msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)\n"
             "os.close(fd)\n")


def every_text(obj):
    if isinstance(obj, dict):
        for v in obj.values():
            for x in every_text(v):
                yield x
    elif isinstance(obj, list):
        for v in obj:
            for x in every_text(v):
                yield x
    elif isinstance(obj, str):
        yield obj


try:
    # ---------------------------------------------------------------- 1. a made-up PC
    print("\n-- 1. a made-up profile: caches, a project, a game, old installers, an old video")
    sparse(os.path.join(PC, "AppData", "Local", "npm-cache", "_cacache", "content-v2", "sha512", "aa", "blob"), 80 * MB)
    sparse(os.path.join(PC, "AppData", "Local", "pip", "Cache", "http", "blob"), 70 * MB)
    write(os.path.join(PC, "Workspace", "Nimbus", "package.json"), '{"name": "nimbus"}')
    sparse(os.path.join(PC, "Workspace", "Nimbus", "node_modules", "left-pad", "index.js"), 40 * MB)
    sparse(os.path.join(PC, "Games", HOSTILE_GAME, "data.pak"), 300 * MB)
    sparse(os.path.join(PC, "Downloads", HOSTILE_EXE), 60 * MB, YEARS_AGO)
    sparse(os.path.join(PC, "Downloads", "Atlas-Setup-2.1.msi"), 45 * MB, YEARS_AGO)
    sparse(os.path.join(PC, "Videos", "Storefront trip.mp4"), 1200 * MB, YEARS_AGO)
    os.makedirs(OUT)
    before = stamps(PC)

    # ---------------------------------------------------------------- 2. the survey
    print("\n-- 2. the survey only reads")
    survey_path = os.path.join(OUT, "survey.json")
    r = py(SURVEY, "--out", survey_path, "--root", PC, "--apparent-size", "--old-gb", "1", "--max-seconds", "120")
    check("survey.py finishes", r.returncode == 0, r.stdout[-300:] + r.stderr[-600:])
    survey = load(survey_path) if os.path.isfile(survey_path) else {}
    check("it wrote a disk-space-report survey", str(survey.get("schema", "")).startswith("disk-space-report/survey@"), survey.get("schema"))
    check("it wrote nothing but survey.json", sorted(os.listdir(OUT)) == ["survey.json"], os.listdir(OUT))
    check("it changed no name, size or date in the folder it read", stamps(PC) == before)
    rules = survey.get("rules") or {}
    check("it found the npm cache, the pip cache and the project's node_modules",
          all((rules.get(k) or {}).get("bytes", 0) >= 30 * MB for k in ("npm-cache", "pip-cache", "node-modules")),
          {k: (rules.get(k) or {}).get("bytes") for k in ("npm-cache", "pip-cache", "node-modules")})
    lists = survey.get("lists") or {}
    check("the old installers, the big video and what's untouched for a year are listed",
          len(lists.get("old_installers") or []) == 2 and lists.get("big_videos") and lists.get("old_large"),
          {k: len(v) for k, v in lists.items()})
    ages = survey.get("ages") or {}
    two_years = str(time.localtime(YEARS_AGO).tm_year)
    check("it counts space by the year files last changed, per drive (the old video's year holds its 1.2 GB)",
          len(ages) == 1 and list(ages.values())[0].get(two_years, 0) >= 1200 * MB, ages)
    tree_dates = [k.get("m") for d in survey.get("drives") or [] for k in (d.get("tree") or {}).get("c", [])]
    check("each folder in the chart says when anything in it last changed", tree_dates and all(tree_dates), tree_dates)
    r = py(os.path.join(S, "summarize.py"), survey_path)
    check("summarize.py reads it", r.returncode == 0 and r.stdout.strip(), r.stderr[-300:])
    r = py(os.path.join(S, "summarize.py"), survey_path, "--section", "ages")
    check("summarize.py shows the last-changed tally", r.returncode == 0 and two_years in r.stdout, r.stdout[-300:] + r.stderr[-300:])

    # ---------------------------------------------------------------- 3. the page and the plan
    print("\n-- 3. the page and the plan")
    notes = save({"headline": "C: has room. " + HOSTILE_NOTE, "highlights": ["C:\\ holds the made-up npm cache.", HOSTILE_NOTE],
                  "insights": [{"title": "Videos is the biggest thing here", "text": "1.2 GB from 2 years ago. " + HOSTILE_NOTE},
                               {"title": HOSTILE_NOTE, "text": "x" * 2000}, "a plain sentence works too", 42],
                  "drives": {"c:": "The made-up drive. " + HOSTILE_NOTE, "Q": "not a drive here"},
                  "places": [{"path": os.path.join(PC, "Videos"), "note": "Holiday films. " + HOSTILE_NOTE}, {"path": "", "note": "no path"}],
                  "drop": ["pip-cache-purge"], "focus": "c:"}, os.path.join(OUT, "notes.json"))
    page, plan_path = os.path.join(OUT, "disk-space.html"), os.path.join(OUT, "diskspace-plan.json")
    r = py(BUILD, survey_path, "--notes", notes, "--out", page)
    check("build_page.py writes the page and diskspace-plan.json next to it", r.returncode == 0 and os.path.isfile(page) and os.path.isfile(plan_path),
          r.stderr[-600:])
    plan = load(plan_path)
    by = actions_by(plan)
    check("the plan has the npm cleanup and the lists, numbered 1..n",
          "npm-cache-clean" in by and "project-leftovers" in by and "old-installers" in by
          and [a["id"] for a in plan["actions"]] == list(range(1, len(plan["actions"]) + 1)), sorted(by))
    check("what notes.json drops (pip-cache-purge) is not in the plan", "pip-cache-purge" not in by)
    check("the plan id is the fingerprint run_plan.py checks", plan["plan_id"] == run_plan.plan_id(plan["actions"]))
    cmds = [a for a in plan["actions"] if a["kind"] in ("command", "open")]
    check("every command and window is one of run_plan.py's, shown exactly as its table has it",
          cmds and all(a["command_id"] in run_plan.TABLE and a["shows"] == run_plan.shown(run_plan.TABLE[a["command_id"]]) for a in cmds),
          [(a.get("command_id"), a.get("shows")) for a in cmds])
    names = ("Lanternfall", "Stop-Computer", "Copperline", "whoami", "Atlas", "Storefront", "Nimbus", "left-pad")
    check("no folder or file name reaches a command", not any(n in a["shows"] for a in cmds for n in names))
    check("the hostile names are kept as they are, as data in the lists",
          any(HOSTILE_EXE in t for t in every_text(by["old-installers"]))
          and any(HOSTILE_GAME in t for t in every_text(by.get("games") or {})), sorted(by))
    check("every item says how much of it is on which drive", all(isinstance(a.get("by_drive"), dict) for a in plan["actions"]))
    with open(page, encoding="utf-8") as f:
        html = f.read()
    check("what notes.json says goes in as text: no tag from it is in the page", "<script>alert" not in html and "<img src=x" not in html)
    check("the page has one script (the kit and the page) and its data", html.count("<script") == 2, html.count("<script"))
    check("the brand kit is inlined: Geist and Doto as data, and mom",
          html.count("data:font/woff2;base64,") >= 3 and 'id="mom"' in html and "/*@kit-css*/" in html and "/*@kit-js*/" in html)
    check("nothing on the page loads from outside", "https://" not in html.replace("https://www.w3.org", "") and "http://" not in html.replace("http://www.w3.org", ""))
    data = json.loads(html.split('<script id="report-data" type="application/json">', 1)[1].split("</script>", 1)[0])
    check("the page opens on the drive notes.json names", data["notes"]["focus"] == "C", data["notes"].get("focus"))
    ins = data["notes"]["insights"]
    check("mom's take keeps the insights as text, cut to size, then the old highlights",
          len(ins) == 5 and ins[0]["title"] == "Videos is the biggest thing here" and len(ins[1]["text"]) <= 600
          and ins[2] == {"title": "", "text": "a plain sentence works too"} and any(HOSTILE_NOTE in i["text"] for i in ins), ins)
    check("a line for the drive it has, none for a drive it doesn't", set(data["notes"]["drives"]) <= {"C", "T"} and data["notes"]["drives"],
          data["notes"]["drives"])
    check("notes on places keep their path and note; one with no path is dropped",
          [p["note"] for p in data["notes"]["places"]] == ["Holiday films. " + HOSTILE_NOTE], data["notes"]["places"])
    check("the last-changed tally reaches the page", data["survey"].get("ages") == ages)
    r = py(BUILD, survey_path, "--notes", save({"drop": ["rm-rf"]}, os.path.join(OUT, "bad-notes.json")), "--out", os.path.join(OUT, "bad.html"))
    check("an id notes.json can't drop is refused, and nothing is written", r.returncode != 0 and not os.path.exists(os.path.join(OUT, "bad.html")), r.stderr[-300:])

    # ---------------------------------------------------------------- 4. the dry run (run_plan.py never gets --run here)
    print("\n-- 4. the dry run")
    every = "1-%d" % len(plan["actions"])
    r = py(RUN, plan_path, "--approve", "DISKSPACE-APPROVE plan=%s ids=%s" % (plan["plan_id"], every))
    said = r.stdout
    check("the dry run names every item and starts nothing", r.returncode == 0 and "DRY RUN (nothing is started)" in said
          and "RAN #" not in said and "OPENED #" not in said and said.count("FOR YOU #") == sum(a["kind"] == "list" for a in plan["actions"]),
          said[-600:] + r.stderr[-300:])
    check("a command is either explained (WOULD RUN) or skipped as not installed",
          all(("WOULD RUN #%d:" % a["id"]) in said or ("SKIP #%d:" % a["id"]) in said for a in plan["actions"] if a["kind"] == "command"), said[-600:])
    r = py(RUN, plan_path, "--approve", "DISKSPACE-APPROVE plan=0000000000 ids=1")
    check("an approval for another plan is refused", r.returncode != 0 and "REFUSED" in r.stdout + r.stderr, r.stdout + r.stderr)
    for bad in ("DISKSPACE-APPROVE ids=1", "DISKSPACE-APPROVE plan=%s ids=1;calc" % plan["plan_id"], "DESKTOP-APPROVE plan=%s ids=1" % plan["plan_id"]):
        r = py(RUN, plan_path, "--approve", bad)
        check("a malformed approval is refused: %s" % bad, r.returncode != 0 and "REFUSED" in r.stdout + r.stderr, r.stdout + r.stderr)
    changed = json.loads(json.dumps(plan))
    first = next(a for a in changed["actions"] if a["kind"] == "command")
    first["shows"] = "npm cache clean --force & calc"
    r = py(RUN, save(changed, os.path.join(OUT, "changed-plan.json")), "--approve", "DISKSPACE-APPROVE plan=%s ids=%d" % (plan["plan_id"], first["id"]))
    check("a plan changed after the page was built is refused", r.returncode != 0 and "REFUSED" in r.stdout + r.stderr, r.stdout + r.stderr)
    changed["plan_id"] = run_plan.plan_id(changed["actions"])
    first["command_id"] = "rm-everything"
    changed["plan_id"] = run_plan.plan_id(changed["actions"])
    r = py(RUN, save(changed, os.path.join(OUT, "changed-plan.json")), "--approve", "DISKSPACE-APPROVE plan=%s ids=%d" % (changed["plan_id"], first["id"]))
    check("a command that isn't in the table is refused even with a matching id", "REFUSED #%d" % first["id"] in r.stdout and r.returncode != 0,
          r.stdout + r.stderr)

    # ---------------------------------------------------------------- 4b. uv's lock, with a made-up uv
    print("\n-- 4b. uv cache clean waits for nobody: a held lock is BUSY and starts nothing")
    fake_dir, cache = os.path.join(T, "fake-uv"), os.path.join(T, "uv-cache")
    fake_exe = write(os.path.join(fake_dir, "uv.cmd"), FAKE_UV)
    os.makedirs(cache)
    calls, mark, lock = os.path.join(T, "uv-calls.txt"), os.path.join(T, "uv-cleaned.txt"), os.path.join(cache, ".lock")
    uv_actions = [{"id": 1, "kind": "command", "command_id": "uv-cache-clean"}]
    uv_plan = save({"schema": run_plan.PLAN_SCHEMA, "actions": uv_actions, "plan_id": run_plan.plan_id(uv_actions)},
                   os.path.join(OUT, "uv-plan.json"))
    uv_approve = "DISKSPACE-APPROVE plan=%s ids=1" % run_plan.plan_id(uv_actions)
    uv_env = dict(os.environ, PATH=os.pathsep.join([fake_dir, os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32")]),
                  FAKE_UV_CACHE=cache, FAKE_UV_CALLS=calls, FAKE_UV_MARK=mark)

    def uv_run(*more):
        for p in (calls, mark):
            if os.path.exists(p):
                os.remove(p)
        r = run([PY, "-I", RUN, uv_plan, "--approve", uv_approve, "--log", os.path.join(OUT, "uv-run-log.txt"), "--timeout", "60"]
                + list(more), env=uv_env)
        asked = []
        if os.path.exists(calls):
            with open(calls, encoding="utf-8", errors="replace") as f:
                asked = [l.strip() for l in f if l.strip()]
        return r, asked, os.path.exists(mark)

    if not run_plan.plain_enough(fake_exe):
        note("uv's lock was not checked: the scratch folder's name is not one run_plan.py hands to cmd.exe (%s)" % fake_exe)
    else:
        r, asked, cleaned = uv_run("--run")
        check("no .lock file: the answer is unknown and uv cache clean runs as before",
              r.returncode == 0 and "RAN #1: uv cache clean -> exit code 0" in r.stdout and cleaned and asked == ["cache dir", "cache clean"],
              (r.stdout + r.stderr)[-600:] + " calls=%s" % asked)
        write(lock, b"")
        r, asked, cleaned = uv_run("--run")
        check("a .lock nobody holds: uv is free and uv cache clean runs",
              r.returncode == 0 and "RAN #1: uv cache clean" in r.stdout and cleaned and asked == ["cache dir", "cache clean"],
              (r.stdout + r.stderr)[-600:] + " calls=%s" % asked)
        holder = subprocess.Popen([PY, "-I", "-c", HOLD_LOCK, lock], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL)
        try:
            held = holder.stdout.readline().strip() == b"held"
            check("a helper process holds the cache's lock", held)
            r, asked, cleaned = uv_run()
            check("the dry run doesn't ask uv anything, even while its lock is held",
                  r.returncode == 0 and "WOULD RUN #1: uv cache clean" in r.stdout and not asked and not cleaned,
                  (r.stdout + r.stderr)[-600:] + " calls=%s" % asked)
            r, asked, cleaned = uv_run("--run")
            busy = [l for l in r.stdout.splitlines() if l.startswith("BUSY #1:")]
            check("while the lock is held: one BUSY line, uv is only asked where its cache is, and nothing is cleaned",
                  held and busy and asked == ["cache dir"] and not cleaned and "RAN #" not in r.stdout,
                  (r.stdout + r.stderr)[-600:] + " calls=%s" % asked)
            check("BUSY says what to do and names the command", busy and "Close it, then approve this item again: uv cache clean" in busy[0], busy)
            check("BUSY is a skip, not a failure: the exit code stays 0", r.returncode == 0, r.returncode)
            with open(os.path.join(OUT, "uv-run-log.txt"), encoding="utf-8") as f:
                check("the BUSY line is in the run log", busy and busy[0] in f.read())
        finally:
            try:
                holder.stdin.close()
                holder.wait(timeout=10)
            except (OSError, subprocess.TimeoutExpired):
                holder.kill()
                holder.wait(timeout=10)
            holder.stdout.close()

    # ---------------------------------------------------------------- 5. the page in a browser
    print("\n-- 5. the page in a headless browser")
    if opts.no_browser:
        note("the page was not opened in a browser (--no-browser)")
    else:
        shots = os.path.join(OUT, "screenshots")
        os.makedirs(shots)
        r = run([PY, "-I", os.path.join(HERE, "check_page.py"), page, T, "--prefix", "DISKSPACE-APPROVE", "--plan", plan_path, "--shots", shots])
        print("\n".join("    " + l for l in r.stdout.splitlines() if l.strip()))
        if r.returncode == 2:
            note("no browser to open the page in: " + r.stdout.strip()[-120:])
        else:
            check("the page passes every browser check", r.returncode == 0, r.stdout[-600:] + r.stderr[-300:])
    check("no __pycache__ in the skill folder", not os.path.isdir(os.path.join(S, "__pycache__")))
finally:
    if not opts.keep:
        remove_work_folder(T, MARK)
    else:
        print("\nkept for a look: %s" % T)
c.finish("disk-space-report")

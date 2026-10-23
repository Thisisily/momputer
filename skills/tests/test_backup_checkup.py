"""Checks the backup-checkup skill end to end on a made-up PC folder.

    python -I test_backup_checkup.py <scratch folder> [<skill folder>] [--keep] [--no-browser]

It makes a small made-up profile inside a work folder in <scratch folder>: main folders (one inside
a stand-in OneDrive, one copied by a stand-in File History 200 days ago), a Dropbox that isn't
running, Git repositories written by hand (in step, no remote, unpushed, a stash, a remote that is
only a folder), and two folders standing in for a second disk and a second letter on the same disk.
Some names are hostile. The big files are sparse, so they take almost no room. Then it surveys it
with survey.py --root, builds the page with notes, and checks: the survey only read and wrote one
file; e-mail addresses are masked and remote tokens dropped; no folder, repository or branch name
reaches a command; what notes.json says stays text; the plan id is a fingerprint run_plan.py
checks; the dry run opens nothing and refuses a wrong id, a changed plan, a page not in its table
and a malformed line. Then the page in a headless browser. run_plan.py is never given --run.

--keep leaves the work folder (fixture, survey, page, plan, screenshots) for a look.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import ctypes
import hashlib
import html
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, X, Checks, remove_work_folder, run, set_times, tree, work_folder, write  # noqa: E402

MARK = "made-by-test_backup_checkup.txt"
MB = 1024 ** 2
DAY = 86400
NOW = time.time()
# Names a shell would run, look-alike quotes and angle brackets Windows allows in a name
HOSTILE_DIR = "Copperline \uff1cimg onerror\uff1e $(Stop-Computer) & echo `whoami` 'x\u2019 ;"
HOSTILE_REPO = "Lanternfall $(calc) `whoami` & del ~ 'x\u2019 ; %TEMP%"
HOSTILE_FILE = "Lanternfall $(Stop-Computer) & del ~ `whoami` 'x\u2019 ; %TEMP%.txt"
HOSTILE_BRANCH = "feature/$(calc)&whoami"
HOSTILE_NOTE = "<script>alert(1)</script> & <img src=x onerror=alert(2)>"
EMAIL = "lanternfall@example.com"
TOKEN = "ghp_FAKE0TOKEN0FOR0TESTS"
NAMES = ("Lanternfall", "Copperline", "Nimbus", "Atlas", "storefront", "Stop-Computer", "whoami", "calc", "feature/")

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "backup-checkup"))
ap.add_argument("--keep", action="store_true")
ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser check of the page")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
sys.stdout.reconfigure(encoding="ascii", errors="backslashreplace")  # check names hold made-up names: print them spelled out
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
SURVEY, BUILD, RUN = (os.path.join(S, n) for n in ("survey.py", "build_page.py", "run_plan.py"))
if not os.path.isfile(SURVEY):
    sys.exit("Not found: %s" % SURVEY)
sys.path.insert(0, S)
import run_plan  # noqa: E402  (only its table, its guide words and its fingerprint are used)

c = Checks()
check, note = c.check, c.note
T = work_folder(opts.scratch, "backup-checkup-test", MARK)
PC, OUT = os.path.join(T, "pc"), os.path.join(T, "out")
DISK2 = os.path.join(T, "drives", "Nimbus backup")       # stands in for a second disk
PART = os.path.join(T, "drives", "Atlas data")           # stands in for another letter on the same disk


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


def small(path, data="made-up", when=None):
    write(path, data)
    if when:
        set_times(path, when)
    return path


def sha(seed):
    return hashlib.sha1(seed.encode("utf-8")).hexdigest()


ZERO = "0" * 40


def repo(path, config, heads, remotes=None, logs=None, stash=0, files=(("src", "main.py"),), when=None):
    """A Git repository written by hand: only the files the survey reads. Git is never run, here or there."""
    g = os.path.join(path, ".git")
    small(os.path.join(g, "HEAD"), "ref: refs/heads/main\n")
    small(os.path.join(g, "config"), config)
    for name, tip in heads.items():
        small(os.path.join(g, "refs", "heads", *name.split("/")), tip + "\n")
    for name, tip in (remotes or {}).items():
        small(os.path.join(g, "refs", "remotes", *name.split("/")), tip + "\n")
    for ref, lines in (logs or {}).items():
        small(os.path.join(g, "logs", *ref.split("/")),
              "".join("%s %s Lanternfall <dev@example.com> %d +0000\t%s\n" % (old, new, int(ts), msg) for old, new, ts, msg in lines))
    if stash:
        small(os.path.join(g, "refs", "stash"), sha(path + "stash") + "\n")
        small(os.path.join(g, "logs", "refs", "stash"),
              "".join("%s %s Lanternfall <dev@example.com> %d +0000\tWIP on main\n" % (ZERO, sha(path + "s%d" % i), int(NOW - DAY)) for i in range(stash)))
    for parts in files:
        small(os.path.join(path, *parts), "print('made up')\n", when)


def stamps(*roots):
    """Every name, size and modified time under the roots: the survey must change none of them."""
    out = []
    for root in roots:
        for rel, kind in tree(root):
            out.append((root, rel, kind, os.lstat(X + root + rel).st_mtime_ns))
    return out


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


def filetime(ts):
    return int((ts + 11644473600) * 10000000)


try:
    # ---------------------------------------------------------------- 1. a made-up PC
    print("\n-- 1. a made-up profile: main folders, OneDrive, File History, Dropbox, repositories, a second disk")
    FH_DAYS = 200
    fh_when = NOW - FH_DAYS * DAY
    small(os.path.join(PC, "Desktop", "Atlas plan.docx"), "made-up plan", NOW - 2 * DAY)
    small(os.path.join(PC, "Desktop", HOSTILE_FILE), "made-up", NOW - 5 * DAY)
    sparse(os.path.join(PC, "Desktop", "Storefront pitch.pptx"), 18 * MB, NOW - 40 * DAY)
    for i in range(14):                                       # a photo a month for 14 months
        sparse(os.path.join(PC, "Pictures", "2026", "Lanternfall-%02d.jpg" % i), 60 * MB, NOW - (i * 30 + 3) * DAY)
    sparse(os.path.join(PC, "Videos", "Storefront trip.mp4"), 1200 * MB, NOW - 700 * DAY)
    sparse(os.path.join(PC, "Downloads", "Atlas-Setup-2.1.msi"), 45 * MB, NOW - 600 * DAY)
    small(os.path.join(PC, "OneDrive", "Nimbus notes.md"), "made-up", NOW - 9 * DAY)
    sparse(os.path.join(PC, "OneDrive", "Documents", "Copperline budget.xlsx"), 4 * MB, NOW - 20 * DAY)
    sparse(os.path.join(PC, "OneDrive", "Documents", "Atlas letters.docx"), 2 * MB, NOW - 75 * DAY)
    small(os.path.join(PC, "Dropbox", "Copperline invoice.pdf"), "made-up", NOW - 12 * DAY)
    sparse(os.path.join(PC, "Dropbox", "Nimbus scans.pdf"), 9 * MB, NOW - 100 * DAY)
    for i in range(120):                                      # a hostile folder name, holding photos: can't be replaced
        small(os.path.join(PC, HOSTILE_DIR, "scan-%03d.jpg" % i), "x" * 1024, NOW - (40 + i) * DAY)
    sparse(os.path.join(PC, HOSTILE_DIR, "panorama.tif"), 30 * MB, NOW - 150 * DAY)
    # Git repositories, written by hand
    W = os.path.join(PC, "Workspace")
    a = sha("nimbus-a")
    repo(os.path.join(W, "Nimbus"),
         '[remote "origin"]\n\turl = https://lanternfall:%s@git.example/lanternfall/nimbus.git\n[branch "main"]\n\tremote = origin\n\tmerge = refs/heads/main\n' % TOKEN,
         {"main": a}, {"origin/main": a},
         {"HEAD": [(ZERO, a, NOW - 10 * DAY, "commit (initial): start")], "refs/remotes/origin/main": [(ZERO, a, NOW - 10 * DAY, "update by push")]},
         when=NOW - 10 * DAY - 600)
    b = sha("copperline-b")
    repo(os.path.join(W, "Copperline"), "[core]\n\tbare = false\n", {"main": b},
         logs={"HEAD": [(ZERO, b, NOW - 3 * DAY, "commit (initial): start")]}, when=NOW - DAY)
    c0, c1, c2 = sha("storefront-0"), sha("storefront-1"), sha("storefront-2")
    branch_tip = sha("storefront-branch")
    repo(os.path.join(W, "storefront"),
         '[remote "origin"]\n\turl = git@git.example:lanternfall/storefront.git\n[branch "main"]\n\tremote = origin\n\tmerge = refs/heads/main\n',
         {"main": c2, HOSTILE_BRANCH: branch_tip}, {"origin/main": c0},
         {"HEAD": [(ZERO, c0, NOW - 40 * DAY, "commit (initial): start"), (c0, c1, NOW - 5 * DAY, "commit: menu"), (c1, c2, NOW - DAY, "commit: prices")],
          "refs/heads/main": [(ZERO, c0, NOW - 40 * DAY, "commit (initial): start"), (c0, c1, NOW - 5 * DAY, "commit: menu"), (c1, c2, NOW - DAY, "commit: prices")],
          "refs/remotes/origin/main": [(ZERO, c0, NOW - 40 * DAY, "update by push")]},
         files=[("src", "page-%02d.js" % i) for i in range(40)], when=NOW - DAY - 600)
    for i in range(40):                                       # enough code for Workspace to count as a folder of your own work
        small(os.path.join(W, "storefront", "src", "page-%02d.js" % i), "// made up\n" * 400, NOW - (i * 9 + 1) * DAY)
    f = sha("hostile-f")
    repo(os.path.join(W, HOSTILE_REPO), '[remote "origin"]\n\turl = https://git.example/lanternfall/odd.git\n',
         {"main": f}, {"origin/main": f}, {"HEAD": [(ZERO, f, NOW - 20 * DAY, "commit (initial): start")],
                                            "refs/remotes/origin/main": [(ZERO, f, NOW - 20 * DAY, "update by push")]}, stash=2, when=NOW - 20 * DAY - 600)
    h = sha("atlas-h")
    repo(os.path.join(W, "Atlas"), '[remote "origin"]\n\turl = ../atlas-bare.git\n', {"main": h}, {"origin/main": h},
         {"HEAD": [(ZERO, h, NOW - 60 * DAY, "commit (initial): start")]}, when=NOW - 60 * DAY - 600)
    # the programs' own settings files, made up: Dropbox's info.json, pCloud's folder, File History's configuration
    os.makedirs(os.path.join(PC, "AppData", "Roaming", "Dropbox"))
    save({"personal": {"path": os.path.join(PC, "Dropbox"), "host": 1}}, os.path.join(PC, "AppData", "Roaming", "Dropbox", "info.json"))
    os.makedirs(os.path.join(PC, "AppData", "Local", "pCloud"))
    small(os.path.join(PC, "AppData", "Local", "Microsoft", "Windows", "FileHistory", "Configuration", "Config1.xml"),
          "<DataProtectionUserConfig><Target><TargetUrl>%s</TargetUrl><TargetName>Nimbus backup</TargetName></Target>"
          "<Library>%s</Library></DataProtectionUserConfig>" % (html.escape(DISK2), html.escape(os.path.join(
              PC, "AppData", "Roaming", "Microsoft", "Windows", "Libraries", "Pictures.library-ms"))))
    # the second disk: File History's store, an old folder named like a backup, and footage that is only there
    small(os.path.join(DISK2, "FileHistory", "Lanternfall", "PC", "Configuration", "Config1.xml"), "<made-up/>", fh_when)
    small(os.path.join(DISK2, "Backup 2023", "Pictures", "old.jpg"), "made-up", NOW - 1000 * DAY)
    sparse(os.path.join(DISK2, "Atlas footage", "clip.mp4"), 2048 * MB, NOW - 500 * DAY)
    # the other letter on the same disk: scans that are only there, and a game library (left out)
    for i in range(110):
        small(os.path.join(PART, "Lanternfall scans", "scan-%03d.pdf" % i), "x" * 2048, NOW - (200 + i) * DAY)
    sparse(os.path.join(PART, "SteamLibrary", "steamapps", "game.pak"), 300 * MB, NOW - 90 * DAY)
    reg = save({"registry": {
        "HKCU\\Software\\Microsoft\\OneDrive\\Accounts\\Personal": {"UserFolder": "%ROOT%\\OneDrive", "UserEmail": EMAIL},
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders": {"Personal": "%USERPROFILE%\\OneDrive\\Documents"},
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\FileHistory": {"ProtectedUpToTime": filetime(fh_when)},
        "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\SystemRestore": {"RPSessionInterval": 1}},
        "processes": ["explorer.exe", "onedrive.exe"]}, os.path.join(T, "registry.json"))
    os.makedirs(OUT)
    before = stamps(PC, os.path.join(T, "drives"))

    # ---------------------------------------------------------------- 2. the survey
    print("\n-- 2. the survey only reads")
    survey_path = os.path.join(OUT, "survey.json")
    r = py(SURVEY, "--out", survey_path, "--root", PC, "--fake-registry", reg, "--extra-drive", DISK2, "--extra-partition", PART,
           "--max-seconds", "120")
    check("survey.py finishes", r.returncode == 0, r.stdout[-300:] + r.stderr[-600:])
    survey = load(survey_path) if os.path.isfile(survey_path) else {}
    check("it wrote a backup-checkup survey", str(survey.get("schema", "")).startswith("backup-checkup/survey@"), survey.get("schema"))
    check("it wrote nothing but survey.json", sorted(os.listdir(OUT)) == ["survey.json"], os.listdir(OUT))
    check("it changed no name, size or date in the folders it read", stamps(PC, os.path.join(T, "drives")) == before)
    with open(survey_path, encoding="utf-8") as fh:
        raw = fh.read()
    check("the e-mail address is masked", EMAIL not in raw and "l\u2022\u2022\u2022@example.com" in raw)
    check("the token in a remote address is dropped", TOKEN not in raw)
    places = {p["label"]: p for p in survey.get("places") or []}
    verdicts = {k: places[k]["verdict"] for k in places}
    check("Documents, inside the running OneDrive, has a second copy", verdicts.get("Documents") == "ok", verdicts)
    check("Pictures has only File History's 200-day-old copy", verdicts.get("Pictures") == "old", verdicts)
    check("Desktop, Videos and the hostile folder have no second copy",
          verdicts.get("Desktop") == "none" and verdicts.get("Videos") == "none" and verdicts.get(HOSTILE_DIR) == "none", verdicts)
    check("the Dropbox folder needs a look: Dropbox isn't running", verdicts.get("Dropbox folder") == "check", verdicts)
    states = {r["name"]: r["state"] for r in survey.get("repos") or []}
    check("the repositories read right without running Git", states == {"Nimbus": "ok", "Copperline": "no-remote", "storefront": "unpushed",
                                                                       HOSTILE_REPO: "stash", "Atlas": "local-remote"}, states)
    pics = places.get("Pictures") or {}
    since = pics.get("changed_since_copy") or {}
    check("Pictures counts what changed since its last copy (the 7 newest photos)",
          since.get("files") == 7 and since.get("bytes") == 7 * 60 * MB, since)
    months = pics.get("months") or {}
    check("Pictures is tallied by the month each photo last changed", sum(v["files"] for v in months.values()) == 14 and len(months) >= 13,
          sorted(months))
    r = py(os.path.join(S, "summarize.py"), survey_path)
    check("summarize.py reads it", r.returncode == 0 and "Pictures" in r.stdout, r.stderr[-300:])

    # ---------------------------------------------------------------- 3. the page and the plan
    print("\n-- 3. the page and the plan")
    notes = save({"headline": "Your photos have only an old copy. " + HOSTILE_NOTE,
                  "insights": [{"title": "Your photos stopped being copied", "text": "Nimbus backup last got Pictures 200 days ago."},
                               {"title": HOSTILE_NOTE, "text": HOSTILE_NOTE}],
                  "highlights": ["Copperline has no remote."],
                  "places": [{"path": os.path.join(PC, "Pictures"), "note": "A photo a month. " + HOSTILE_NOTE},
                             {"path": "C:\\Nowhere\\Atlas", "note": "not a surveyed folder"}],
                  "drives": {"T1": "The made-up second disk. " + HOSTILE_NOTE, "Q": "no such drive"},
                  "drop": ["checks"]}, os.path.join(OUT, "notes.json"))
    page, plan_path = os.path.join(OUT, "backup-checkup.html"), os.path.join(OUT, "backup-plan.json")
    r = py(BUILD, survey_path, "--notes", notes, "--out", page)
    check("build_page.py writes the page and backup-plan.json next to it", r.returncode == 0 and os.path.isfile(page) and os.path.isfile(plan_path),
          r.stderr[-600:])
    plan = load(plan_path)
    acts = plan["actions"]
    kinds = {a.get("command_id") or a.get("guide_id") for a in acts}
    check("the plan has both Windows pages, copies, a drive to get and the pushes, numbered 1..n",
          {"open-windows-backup", "open-file-history", "copy-folder", "get-a-drive", "push-repo"} <= kinds
          and [a["id"] for a in acts] == list(range(1, len(acts) + 1)), sorted(kinds))
    check("what notes.json drops (checks) is not in the plan", not kinds & {"start-client", "check-client", "connect-drive"}, sorted(kinds))
    check("the plan id is the fingerprint run_plan.py checks", plan["plan_id"] == run_plan.plan_id(acts))
    opens = [a for a in acts if a["kind"] == "open"]
    check("every Windows page is one of run_plan.py's, shown exactly as its table has it",
          opens and all(a["command_id"] in run_plan.TABLE and a["shows"] == run_plan.shown(run_plan.TABLE[a["command_id"]]) for a in opens),
          [(a.get("command_id"), a.get("shows")) for a in opens])
    check("every other item is a guide run_plan.py knows by its own words",
          all(a["kind"] == "guide" and a["guide_id"] in run_plan.GUIDES for a in acts if a["kind"] != "open"))
    check("no folder, repository or branch name reaches a command", not any(n in a["shows"] for a in opens for n in NAMES + (HOSTILE_DIR,)))
    check("nothing is ticked for the person", not any(a.get("default") for a in acts))
    check("the hostile names are kept as they are, as data in the titles and paths",
          any(HOSTILE_DIR in t for t in every_text(acts)) and any(HOSTILE_REPO in t for t in every_text(acts)), [a["title"] for a in acts])
    with open(page, encoding="utf-8") as fh:
        doc = fh.read()
    check("what notes.json says goes in as text: no tag from it is in the page", "<script>alert" not in doc and "<img src=x" not in doc)
    check("the page has one script (the kit and the page) and its data", doc.count("<script") == 2, doc.count("<script"))
    check("the brand kit is inlined: Geist and Doto as data, and mom",
          doc.count("data:font/woff2;base64,") >= 3 and 'id="mom"' in doc and "/*@kit-css*/" in doc and "/*@kit-js*/" in doc)
    check("nothing on the page loads from outside", "https://" not in doc.replace("https://www.w3.org", "") and "http://" not in doc.replace("http://www.w3.org", ""))
    data = json.loads(doc.split('<script id="report-data" type="application/json">', 1)[1].split("</script>", 1)[0])
    n = data["notes"]
    check("mom's take holds the agent's insights, then its highlights", [i["title"] for i in n["insights"]][:2] == ["Your photos stopped being copied", HOSTILE_NOTE]
          and n["insights"][-1]["text"] == "Copperline has no remote.", n["insights"])
    check("a note on a folder lands on that folder only; one on a folder not surveyed is dropped",
          list(n["places"].values()) == ["A photo a month. " + HOSTILE_NOTE] and str(pics.get("id")) in n["places"], n["places"])
    check("a note on a drive lands on that drive only", n["drives"] == {"T1": "The made-up second disk. " + HOSTILE_NOTE}, n["drives"])
    dash = data.get("dash") or {}
    cov = dash.get("coverage") or {}
    check("the builder works out the coverage: every counted byte in one of four shares",
          sum(cov.values()) == sum(p["own_bytes"] for p in survey["places"] if p["counts"]) and cov.get("ok") and cov.get("old") and cov.get("none"), cov)
    m = dash.get("months") or {}
    # Pictures' copy is from 200 days ago: the photos of the chart's months before that month are in
    # it, the rest only here. How many that is depends on today's date (4 or 5 of them).
    month = lambda ts: time.strftime("%Y-%m", time.localtime(ts))
    shown = [month(NOW - (i * 30 + 3) * DAY) for i in range(14) if month(NOW - (i * 30 + 3) * DAY) in (m.get("keys") or [])]
    copied = sum(1 for k in shown if k < month(fh_when))
    check("the months chart has 12 months; photos since Pictures' copy are only on this PC, older ones are in it",
          len(m.get("keys") or []) == 12 and copied >= 4 and sum(m.get("only_here") or []) >= (len(shown) - copied) * 60 * MB
          and copied * 60 * MB <= sum(m.get("copied") or []) < (copied + 1) * 60 * MB, (copied, m))
    fresh = {x["name"]: x for x in dash.get("fresh") or []}
    check("how fresh each copy is: File History 200 days, the folder named like a backup, Git's newest push",
          (fresh.get("File History") or {}).get("days") == FH_DAYS and "Backup 2023" in fresh and (fresh.get("Git remotes") or {}).get("days") == 10,
          dash.get("fresh"))
    check("every copy-a-folder item knows which drive it points at", all(str(a["id"]) in dash.get("copy_to", {}) for a in acts if a.get("guide_id") == "copy-folder"),
          dash.get("copy_to"))
    r = py(BUILD, survey_path, "--notes", save({"drop": ["robocopy"]}, os.path.join(OUT, "bad-notes.json")), "--out", os.path.join(OUT, "bad.html"))
    check("an id notes.json can't drop is refused, and nothing is written", r.returncode != 0 and not os.path.exists(os.path.join(OUT, "bad.html")), r.stderr[-300:])

    # ---------------------------------------------------------------- 4. the dry run (run_plan.py never gets --run here)
    print("\n-- 4. the dry run")
    every = "1-%d" % len(acts)
    r = py(RUN, plan_path, "--approve", "BACKUP-APPROVE plan=%s ids=%s" % (plan["plan_id"], every))
    said = r.stdout
    check("the dry run names every item and opens nothing", r.returncode == 0 and "DRY RUN (nothing is opened)" in said and "OPENED #" not in said
          and said.count("FOR YOU #") == sum(a["kind"] == "guide" for a in acts), said[-600:] + r.stderr[-300:])
    check("a Windows page is either explained (WOULD OPEN) or skipped as not found",
          all(("WOULD OPEN #%d:" % a["id"]) in said or ("SKIP #%d:" % a["id"]) in said for a in opens), said[-600:])
    check("no folder, repository or branch name is printed for a command to pick up", not any(x in said for x in NAMES + (HOSTILE_DIR, HOSTILE_REPO)), said)
    r = py(RUN, plan_path, "--approve", "BACKUP-APPROVE plan=%s ids=none" % plan["plan_id"])
    check("ids=none approves nothing", r.returncode == 0 and "0 items approved" in r.stdout, r.stdout + r.stderr)
    r = py(RUN, plan_path, "--approve", "BACKUP-APPROVE plan=0000000000 ids=1")
    check("an approval for another plan is refused", r.returncode != 0 and "REFUSED" in r.stdout + r.stderr, r.stdout + r.stderr)
    for bad in ("BACKUP-APPROVE ids=1", "BACKUP-APPROVE plan=%s ids=1;calc" % plan["plan_id"], "BACKUP-APPROVE plan=%s ids=1 & calc" % plan["plan_id"],
                "DISKSPACE-APPROVE plan=%s ids=1" % plan["plan_id"], "BACKUP-APPROVE plan=%s ids=-1" % plan["plan_id"]):
        r = py(RUN, plan_path, "--approve", bad)
        check("a malformed approval is refused: %s" % bad, r.returncode != 0 and "REFUSED" in r.stdout + r.stderr, r.stdout + r.stderr)
    changed = json.loads(json.dumps(plan))
    first = next(a for a in changed["actions"] if a["kind"] == "open")
    first["shows"] = "explorer.exe ms-settings:backup & calc"
    r = py(RUN, save(changed, os.path.join(OUT, "changed-plan.json")), "--approve", "BACKUP-APPROVE plan=%s ids=%d" % (plan["plan_id"], first["id"]))
    check("a plan changed after the page was built is refused", r.returncode != 0 and "REFUSED" in r.stdout + r.stderr, r.stdout + r.stderr)
    first["command_id"] = "robocopy-everything"
    changed["plan_id"] = run_plan.plan_id(changed["actions"])
    r = py(RUN, save(changed, os.path.join(OUT, "changed-plan.json")), "--approve", "BACKUP-APPROVE plan=%s ids=%d" % (changed["plan_id"], first["id"]))
    check("a Windows page that isn't in the table is refused even with a matching id", "REFUSED #%d" % first["id"] in r.stdout and r.returncode != 0,
          r.stdout + r.stderr)
    guide = next(a for a in changed["actions"] if a["kind"] == "guide")
    guide["guide_id"] = "$(calc)"
    changed["plan_id"] = run_plan.plan_id(changed["actions"])
    r = py(RUN, save(changed, os.path.join(OUT, "changed-plan.json")), "--approve", "BACKUP-APPROVE plan=%s ids=%d" % (changed["plan_id"], guide["id"]))
    check("a guide item with a made-up id prints only run_plan.py's own words", r.returncode == 0 and "$(calc)" not in r.stdout
          and "FOR YOU #%d: Something for you to do" % guide["id"] in r.stdout, r.stdout + r.stderr)

    # ---------------------------------------------------------------- 5. the page in a browser
    print("\n-- 5. the page in a headless browser")
    if opts.no_browser:
        note("the page was not opened in a browser (--no-browser)")
    else:
        shots = os.path.join(OUT, "screenshots")
        os.makedirs(shots)
        r = run([PY, "-I", os.path.join(HERE, "check_page.py"), page, T, "--prefix", "BACKUP-APPROVE", "--plan", plan_path, "--shots", shots])
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
c.finish("backup-checkup")

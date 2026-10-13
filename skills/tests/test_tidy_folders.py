"""Checks the tidy-folders skill end to end on a made-up profile.

    python -I test_tidy_folders.py <scratch folder> [<skill folder>] [--keep] [--no-browser]

It makes a pretend profile (Desktop, Documents, Downloads...) in a work folder inside <scratch
folder>, with loose papers, an old installer, a duplicate, an empty folder and awkward names.
Then: survey -> page -> dry run -> apply -> revert.ps1 -> apply -> --undo, checking the tree
comes back exactly. Nothing outside the work folder is touched, and nothing is sent to the real
Recycle Bin: the engine's Recycle Bin call is swapped for one that moves the file to a folder
inside the work folder.

--keep leaves the work folder (survey, page, plan, logs, screenshots) for a look.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import contextlib
import io
import json
import os
import shutil
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, X, Checks, remove_work_folder, run, tree, work_folder, write  # noqa: E402

MARK = "made-by-test_tidy_folders.txt"
SECRET = "FILE-CONTENT-MARKER-4471"

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "tidy-folders"))
ap.add_argument("--keep", action="store_true")
ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser check of the page")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
c = Checks()
check, note = c.check, c.note
pycache_before = os.path.isdir(os.path.join(S, "__pycache__"))
T = work_folder(opts.scratch, "tidy-folders-test", MARK)

try:
    # ------------------------------------------------------------ a launcher's file can't send the survey to another PC
    # Steam's own folder can be written by every account on a PC. A library it names on a share must
    # never be asked about: asking would sign in to that share as the person running the survey.
    sys.path.insert(0, S)
    import survey as tidy_survey  # noqa: E402  (the skill's own script)
    steam = os.path.join(T, "launchers", "Steam")
    here_lib = os.path.join(T, "launchers", "Library here")
    write(os.path.join(here_lib, "steamapps", "appmanifest_1.acf"), '"name" "Lanternfall Quest"\n"installdir" "Lanternfall Quest"\n"SizeOnDisk" "123"\n')
    write(os.path.join(here_lib, "steamapps", "appmanifest_2.acf"), '"name" "Climber"\n"installdir" "..\\..\\..\\Windows"\n')
    write(os.path.join(steam, "steamapps", "libraryfolders.vdf"),
          '"libraryfolders"\n{\n "0" { "path" "%s" }\n "1" { "path" "\\\\\\\\files.example\\\\share\\\\SteamLibrary" }\n "2" { "path" "relative\\\\lib" }\n}\n'
          % here_lib.replace("\\", "\\\\"))
    # A manifest that is a link, and a library reached through one: neither is read. (Making a
    # symlink needs a right most accounts don't have; without it these two are not run.)
    elsewhere = os.path.join(T, "launchers", "elsewhere")
    write(os.path.join(elsewhere, "steamapps", "appmanifest_9.acf"), '"name" "Reached Through A Link"\n"installdir" "Reached"\n')
    write(os.path.join(elsewhere, "linked.acf"), '"name" "Read Through A Link"\n"installdir" "Linked"\n')
    symlinks = []
    try:
        os.symlink(os.path.join(elsewhere, "linked.acf"), os.path.join(here_lib, "steamapps", "appmanifest_3.acf"))
        symlinks.append(os.path.join(here_lib, "steamapps", "appmanifest_3.acf"))
        os.symlink(elsewhere, os.path.join(T, "launchers", "Library link"), target_is_directory=True)
        symlinks.append(os.path.join(T, "launchers", "Library link"))
        with open(os.path.join(steam, "steamapps", "libraryfolders.vdf"), "a", encoding="utf-8") as f:
            f.write('"3" { "path" "%s" }\n' % os.path.join(T, "launchers", "Library link").replace("\\", "\\\\"))
    except OSError:
        note("this account can't make symlinks: the two link checks on launcher files were not run")
    asked = []
    real_opened, real_isdir, real_values, real_subkeys = tidy_survey._opened, os.path.isdir, tidy_survey.reg_values, tidy_survey.reg_subkeys

    def opened_noting(p, access, share):
        asked.append(str(p))
        return None if str(p).startswith("\\\\") else real_opened(p, access, share)  # noted, never passed on: this test contacts nothing

    def isdir_noting(p):
        asked.append(str(p))
        return False if str(p).startswith("\\\\") else real_isdir(p)

    program_data = os.environ.get("ProgramData")
    try:
        os.path.isdir, tidy_survey._opened = isdir_noting, opened_noting
        tidy_survey.reg_values = lambda hive, key, *a: {"SteamPath": steam} if key.endswith("Valve\\Steam") else {}
        tidy_survey.reg_subkeys = lambda hive, key: []
        os.environ["ProgramData"] = os.path.join(T, "launchers", "ProgramData")  # so no real launcher's files are read
        found = tidy_survey.launchers([])
    finally:
        if program_data is None:
            os.environ.pop("ProgramData", None)
        else:
            os.environ["ProgramData"] = program_data
        os.path.isdir, tidy_survey._opened, tidy_survey.reg_values, tidy_survey.reg_subkeys = real_isdir, real_opened, real_values, real_subkeys
    steam_found = next((f for f in found if f["name"] == "Steam"), {"libraries": [], "games": []})
    check("a game library on a share, or not a full path, is never asked about; one on this PC is read",
          not any(p.startswith("\\\\") or "files.example" in p or p.startswith("relative") for p in asked)
          and any(p.endswith("Library here\\steamapps") for p in asked)
          and sorted(os.path.basename(l) for l in steam_found["libraries"]) == ["Library here", "Steam"]
          and [g["name"] for g in steam_found["games"]] == ["Lanternfall Quest"], (asked, steam_found))
    if len(symlinks) == 2:
        check("a launcher file that is a link, and a library reached through a link, are not read",
              not any("Link" in g["name"] for g in steam_found["games"]) and not any("Library link" in l for l in steam_found["libraries"])
              and not tidy_survey.a_folder_here(os.path.join(T, "launchers", "Library link", "steamapps"))
              and tidy_survey.listed(os.path.join(T, "launchers", "Library link")) is None
              and tidy_survey.read_text(os.path.join(here_lib, "steamapps", "appmanifest_3.acf")) == ""
              and "Lanternfall Quest" in tidy_survey.read_text(os.path.join(here_lib, "steamapps", "appmanifest_1.acf")), steam_found)
    for link in symlinks:
        (os.rmdir if os.path.isdir(link) else os.remove)(link)  # the link itself, never what it leads to
    check("on_this_pc: only a full path on a disk in this PC",
          tidy_survey.on_this_pc(T) and not any(tidy_survey.on_this_pc(p) for p in (
              "\\\\files.example\\share\\x", "\\\\?\\UNC\\files.example\\share", "relative\\x", "C:", "", None, 7, "Q:relative",
              T + "\\..\\x", T + "\\.\\x", T + "\\\\x", T + "/x", T + "\\", T + "\\x.", T + "\\x ", T + "\\x:y", T + "\\x?", T + "\\x\ny",
              T.replace("\\", "/"))))
    # While a folder is being looked into, it and everything above it are held: nobody can swap one for a link meanwhile.
    inner = os.path.join(T, "launchers", "held", "inner")
    os.makedirs(inner)
    write(os.path.join(inner, "a file.txt"), "x")
    held = tidy_survey.hold(inner)
    swapped = []
    for name in ("held", os.path.join("held", "inner")):  # a folder above it, and the folder itself
        try:
            os.rename(os.path.join(T, "launchers", name), os.path.join(T, "launchers", name + " moved"))
            swapped.append(name)
        except OSError:
            pass
    tidy_survey.release(held)
    check("a folder that is being looked into can't be moved or replaced meanwhile, and can afterwards",
          held is not None and not swapped and tidy_survey.a_folder_here(inner))
    os.rename(os.path.join(T, "launchers", "held"), os.path.join(T, "launchers", "held moved"))
    moved = os.path.join(T, "launchers", "held moved", "inner")
    check("what isn't there, is a file, or isn't spelled as a full path on this PC is never 'a folder here'",
          tidy_survey.a_folder_here(moved) and not any(tidy_survey.a_folder_here(p) for p in (
              inner, os.path.join(moved, "missing"), os.path.join(moved, "a file.txt"), os.path.join(moved, "a file.txt", "below"),
              "\\\\files.example\\share", "moved\\inner", ""))
          and tidy_survey.listed(os.path.join(moved, "missing")) is None and tidy_survey.read_text(os.path.join(moved, "missing.txt")) == ""
          and tidy_survey.listed(moved) == ["a file.txt"])
    sys.path.remove(S)
    shutil.rmtree(os.path.join(T, "launchers"))  # this test's own fixture, inside its own work folder

    root = os.path.join(T, "profile")
    for d in ("Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"):
        os.makedirs(os.path.join(root, d))
    old = time.time() - 90 * 86400
    files = {
        "Desktop\\Lanternfall contract signed.pdf": b"%PDF-1.4 " + SECRET.encode(),
        "Desktop\\passport scan ’ copy.jpg": b"jpg" * 20,
        "Desktop\\notes; echo pwn & b.txt": SECRET.encode(),
        "Downloads\\Copperline invoice 1182.pdf": b"%PDF-1.4 y",
        "Downloads\\setup-old.exe": b"MZ" * 40000,
        "Downloads\\photo $(calc) `x`.png": b"png" * 10,
        "Downloads\\it's 'quoted'.mp4": b"mp4" * 10,
        "Downloads\\song.mp3": b"mp3" * 10,
        "Downloads\\bundle.zip": b"PK" * 30,
        "Downloads\\bundle\\readme.txt": b"r",
        "Documents\\kept\\report.docx": b"docx",
        "Downloads\\big copy (1).pdf": b"D" * 40000,
        "Documents\\kept\\big copy.pdf": b"D" * 40000,
    }
    for rel, data in files.items():
        os.utime(X + write(os.path.join(root, rel), data), (old, old))
    os.makedirs(os.path.join(root, "Pictures", "empty one", "deeper"))
    out = os.path.join(T, "out")
    os.makedirs(out)
    before = tree(root)

    print("\n-- 1. survey and page")
    survey_json, page, plan_json = os.path.join(out, "survey.json"), os.path.join(out, "tidy-plan.html"), os.path.join(out, "plan.json")
    r = run([PY, "-I", os.path.join(S, "survey.py"), "--out", survey_json, "--root", root])
    check("the survey runs", r.returncode == 0, r.stderr or r.stdout)
    check("the survey changed nothing and wrote one file", tree(root) == before and os.listdir(out) == ["survey.json"], os.listdir(out))
    evil = "</script><img src=x onerror=\"window.__pwned=1\"> ' ’ $(x) `y` ; &  "
    notes_json = os.path.join(out, "notes.json")
    json.dump({"headline": "Headline " + evil, "highlights": ["One " + evil]}, open(notes_json, "w", encoding="utf-8"))
    r = run([PY, "-I", os.path.join(S, "build_page.py"), survey_json, "--notes", notes_json, "--out", page])
    check("the page builds", r.returncode == 0 and os.path.isfile(page) and os.path.isfile(plan_json), r.stderr or r.stdout)
    plan = json.load(open(plan_json, encoding="utf-8"))
    sys.path.insert(0, S)
    import safe_apply as sa  # noqa: E402  (the skill's own copy)
    import build_page  # noqa: E402
    groups = sorted({a["group"] for a in plan["actions"]})
    check("the plan is tidy-folders/plan@1, its id is the fingerprint of its actions, and build_page uses the engine's definition",
          plan["schema"] == "tidy-folders/plan@1" and plan["plan_id"] == sa.plan_fingerprint(plan["actions"]) and build_page.plan_id is sa.plan_fingerprint)
    check("the survey found each kind of thing: %s" % ", ".join(groups),
          groups == ["archives", "documents", "duplicates", "empty", "important", "installers", "music", "pictures", "videos"], groups)
    html = open(page, encoding="utf-8").read()
    check("hostile text is escaped in the page", html.count("</script>") == 2 and "<img" not in html and " " not in html)
    check("no file contents in the survey, the plan or the page", not any(SECRET in open(p, encoding="utf-8").read() for p in (survey_json, plan_json, page)))

    print("\n-- 2. dry run, apply, both ways back")
    apply_py = [PY, "-I", os.path.join(S, "apply_plan.py")]
    ids = [a["id"] for a in plan["actions"]]
    approve = "TIDY-APPROVE plan=%s ids=1-%d" % (plan["plan_id"], max(ids))
    r = run(apply_py + [plan_json, "--approve", approve, "--root", root, "--log-dir", os.path.join(out, "log-1")])
    check("dry run: all %d actions would happen, nothing is refused, nothing changes" % len(ids),
          r.returncode == 0 and sum(1 for l in r.stdout.splitlines() if l.startswith("WOULD")) == len(ids) and "REFUSED" not in r.stdout and tree(root) == before,
          r.stdout[-1000:] + r.stderr)
    import apply_plan  # noqa: E402  (run in this process so the Recycle Bin call can be swapped)
    fake_bin, binned = os.path.join(T, "pretend-recycle-bin"), []

    def pretend_recycle(path):
        os.makedirs(fake_bin, exist_ok=True)
        dest = os.path.join(fake_bin, "%03d %s" % (len(binned), os.path.basename(path)))
        shutil.move(X + path, X + dest)
        binned.append((dest, path))
        return None

    def restore_bin():
        for dest, path in reversed(binned):
            shutil.move(X + dest, X + path)
        del binned[:]
        if os.path.isdir(fake_bin):
            os.rmdir(fake_bin)

    def apply_here(argv):
        buf, real_recycle = io.StringIO(), sa.recycle
        sa.recycle = pretend_recycle
        try:
            with contextlib.redirect_stdout(buf):
                apply_plan.main(argv)
        except SystemExit as ex:
            buf.write("\nEXIT %r" % (ex.code,))
        finally:
            sa.recycle = real_recycle
        return buf.getvalue()

    n_recycle = sum(1 for a in plan["actions"] if a["op"] == "recycle")
    for round_no, way_back in ((1, "revert.ps1"), (2, "--undo")):
        log = os.path.join(out, "log-%d" % round_no)
        text = apply_here([plan_json, "--approve", approve, "--root", root, "--log-dir", log, "--apply"])
        now = tree(root)
        check("apply (%d): everything done, nothing failed or refused; %d went to the (pretend) Recycle Bin" % (round_no, n_recycle),
              "EXIT" not in text and "FAILED" not in text and "REFUSED" not in text and len(binned) == n_recycle and now != before, text[-1200:])
        check("apply (%d): Desktop and Downloads hold no loose files any more; papers are under Documents\\Important" % round_no,
              os.listdir(os.path.join(root, "Desktop")) == [] and sorted(os.listdir(os.path.join(root, "Downloads"))) == ["bundle"]
              and os.path.isdir(os.path.join(root, "Documents", "Important")) and not os.path.exists(os.path.join(root, "Pictures", "empty one")),
              (os.listdir(os.path.join(root, "Desktop")), os.listdir(os.path.join(root, "Downloads"))))
        if way_back == "revert.ps1":
            r = run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", os.path.join(log, "revert.ps1"), "-Root", root])
        else:
            r = run(apply_py + ["--undo", log, "--root", root, "--apply"])
        check("%s: runs clean and lists the %d Recycle Bin items as MANUAL" % (way_back, n_recycle), r.returncode == 0 and r.stdout.count("MANUAL") == n_recycle,
              r.stdout[-800:] + r.stderr)
        restore_bin()
        check("%s (and the pretend Recycle Bin emptied back): the tree is exactly as it was, empty folders too" % way_back, tree(root) == before,
              [e for e in tree(root) if e not in before][:6] + [e for e in before if e not in tree(root)][:6])
    check("nothing in a name ran (no stray files)", sorted(os.listdir(T)) == sorted([MARK, "out", "profile"]), os.listdir(T))

    print("\n-- 3. the page in a headless browser")
    if opts.no_browser:
        note("the page was not opened in a browser (--no-browser)")
    else:
        shots = os.path.join(out, "screenshots")
        os.makedirs(shots)
        r = run([PY, "-I", os.path.join(HERE, "check_page.py"), page, T, "--prefix", "TIDY-APPROVE", "--plan", plan_json, "--shots", shots])
        print("\n".join("    " + l for l in r.stdout.splitlines() if l.strip()))
        if r.returncode == 2:
            note("no browser to open the page in: " + r.stdout.strip()[-120:])
        else:
            check("the page passes every browser check", r.returncode == 0, r.stdout[-600:] + r.stderr[-300:])
    check("no __pycache__ left in the skill folder", pycache_before or not os.path.isdir(os.path.join(S, "__pycache__")))
finally:
    if not opts.keep:
        remove_work_folder(T, MARK)
    else:
        print("\nkept for a look: %s" % T)
c.finish("tidy-folders")

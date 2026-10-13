"""Checks one skill's copy of the shared file-moving engine: scripts/safe_apply.py and scripts/revert.ps1.

    python -I test_safe_apply.py <scratch folder> <skill folder> [--entry apply_plan.py] [--keep]

Everything happens in a work folder this test makes inside <scratch folder> (which must already
exist and be under %TEMP%, because the checks rely on Temp being a protected place). Nothing
outside that work folder is created, moved, renamed or deleted. The work folder is removed at
the end unless --keep is given.

--entry names the script in the skill's scripts/ folder that is run as the command line: the
engine itself (the default), or a wrapper around it that adds no rules in --root mode, such as
tidy-folders' apply_plan.py.

Windows only. Python 3.8+, standard library only.
"""
import argparse
import contextlib
import csv
import ctypes
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
X = "\\\\?\\"
MARK = "made-by-test_safe_apply.txt"
PY = sys.executable

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch", help="an existing folder under %%TEMP%%; the test works in a subfolder it makes there")
ap.add_argument("skill", help="the skill folder to check, e.g. skills/tidy-folders")
ap.add_argument("--entry", default="safe_apply.py", help="script in the skill's scripts/ folder to run (default safe_apply.py)")
ap.add_argument("--keep", action="store_true", help="leave the work folder in place for a look")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
SCRATCH, SKILL = os.path.abspath(opts.scratch), os.path.abspath(opts.skill)
SCRIPTS = os.path.join(SKILL, "scripts")
ENTRY, REVERT = os.path.join(SCRIPTS, opts.entry), os.path.join(SCRIPTS, "revert.ps1")
for need in (os.path.join(SCRIPTS, "safe_apply.py"), REVERT, ENTRY):
    if not os.path.isfile(need):
        sys.exit("Not found: %s" % need)
if not os.path.isdir(SCRATCH):
    sys.exit("The scratch folder must already exist: %s" % SCRATCH)
TEMP = os.path.normcase(os.path.realpath(tempfile.gettempdir()))
if not os.path.normcase(os.path.realpath(SCRATCH)).startswith(TEMP + os.sep):
    sys.exit("The scratch folder must be under %s (the checks rely on Temp being a protected place)" % tempfile.gettempdir())
T = os.path.join(SCRATCH, "sa-" + os.path.basename(SKILL) + ("" if opts.entry == "safe_apply.py" else "-wrapped"))
if os.path.lexists(T) and not os.path.isfile(os.path.join(T, MARK)):
    sys.exit("%s exists and this test didn't make it. Remove it yourself or give another scratch folder." % T)

fails, notes, count = [], [], [0]


def check(name, ok, detail=""):
    count[0] += 1
    print(("ok:   " if ok else "FAIL: ") + name + ((" (" + str(detail)[:500] + ")") if detail and not ok else ""))
    if not ok:
        fails.append(name)


def note(text):
    notes.append(text)
    print("note: " + text)


def run(args, **kw):
    return subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace", **kw)


def entry(*args):
    return run([PY, "-I", ENTRY] + list(args))


def revert(folder, *args):
    return run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", os.path.join(folder, "revert.ps1")] + list(args))


def tree(root):
    out = []
    for d, dirs, files in os.walk(X + root):
        for f in files:
            out.append(os.path.join(d, f)[len(X + root):])
    return sorted(out)


def short(path):
    buf = ctypes.create_unicode_buffer(32768)
    n = ctypes.windll.kernel32.GetShortPathNameW(X + path, buf, len(buf))
    s = buf.value[4:] if n and buf.value.startswith(X) else buf.value
    return s if n else path


def by_id(stdout):
    """{action id: [(status, operation, the whole line)]} from an apply run's output."""
    out = {}
    for line in stdout.splitlines():
        m = re.match(r"^(WOULD|REFUSED|SKIP|DONE|FAILED|COPIED)\s+(\S+)\s+#(\d+)\s", line)
        if m:
            out.setdefault(int(m.group(3)), []).append((m.group(1), m.group(2), line))
    return out


def only(lines, aid, status, word=""):
    """True if action `aid` has exactly one line, with that status and that word in it."""
    hit = [x for x in lines.get(aid, []) if x[1] != "mkdir"]
    return len(hit) == 1 and hit[0][0] == status and word in hit[0][2]


def junction(link, target):
    made = subprocess.run(["cmd", "/c", "mklink", "/J", link, target], capture_output=True, text=True)
    if made.returncode == 0:
        links.append(link)
    return made.returncode == 0


def write_log(folder, rows):
    os.makedirs(folder)
    shutil.copyfile(REVERT, os.path.join(folder, "revert.ps1"))
    with open(os.path.join(folder, "undo-log.csv"), "w", newline="", encoding="utf-8-sig") as f:
        w = csv.DictWriter(f, ["time", "id", "op", "from", "to", "status", "note"], quoting=csv.QUOTE_ALL)
        w.writeheader()
        for i, row in enumerate(rows):
            w.writerow({"time": "2026-01-01T00:00:00", "id": i + 1, "op": row["op"], "from": row["from"],
                        "to": row.get("to", ""), "status": "DONE", "note": ""})


@contextlib.contextmanager
def patched(obj, name, value):
    old = getattr(obj, name)
    setattr(obj, name, value)
    try:
        yield
    finally:
        setattr(obj, name, old)


def in_process(argv, **hooks):
    """Run the engine inside this process (so it can be patched). Returns (output, exit code or None)."""
    buf, code = io.StringIO(), None
    with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(buf):
        try:
            sa.run(sa.parse_args(argv), **hooks)
        except SystemExit as ex:
            code = ex.code
            if isinstance(code, str):
                print(code)
    return buf.getvalue(), code


links = []          # every junction made here, removed (the link itself, never its target) at the end
locked = []         # folders made unlistable, to be opened up again whatever happens
pycache_before = os.path.isdir(os.path.join(SCRIPTS, "__pycache__"))
sys.path.insert(0, SCRIPTS)
import safe_apply as sa  # noqa: E402  (the skill's own copy)

try:
    # ------------------------------------------------------------ the fixture
    if os.path.isdir(X + T):
        shutil.rmtree(X + T)
    os.makedirs(T)
    open(os.path.join(T, MARK), "w").write("Made by test_safe_apply.py. Safe to delete.\n")
    root = os.path.join(T, "profile")
    for d in ("Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"):
        os.makedirs(os.path.join(root, d))
    outside = os.path.join(T, "outside")
    os.makedirs(outside)
    desk, doc, dl = os.path.join(root, "Desktop"), os.path.join(root, "Documents"), os.path.join(root, "Downloads")
    names = ["contract \u2019 x.pdf", "a; echo pwn & b.txt", "tax $(calc) `x`.docx", "plain.txt", "ends in a dot."]
    for n in names:
        open(X + os.path.join(desk, n), "w").write("x")
    folder_name = "folder \u2019 $(x) & 'y'"
    os.makedirs(os.path.join(desk, folder_name, "sub"))
    open(os.path.join(desk, folder_name, "sub", "in it.txt"), "w").write("f")
    open(os.path.join(outside, "secret.txt"), "w").write("s")
    victim = os.path.join(dl, "victim.txt")
    open(victim, "w").write("v")
    longdir = os.path.join(dl, "A Long Folder Name Here")
    os.makedirs(longdir)
    open(os.path.join(longdir, "inside.txt"), "w").write("i")
    alias = short(longdir)
    has_short = alias.lower() != longdir.lower()
    box = os.path.join(doc, "Box")
    os.makedirs(os.path.join(box, "Keep", "Inner"))
    open(os.path.join(box, "in.txt"), "w").write("b")
    # A junction inside the folder that leads outside it, and one that stays inside it
    out_link = os.path.join(doc, "shortcut-out")
    has_junction = junction(out_link, outside)
    jump = os.path.join(desk, "jump")
    has_jump = junction(jump, dl)

    appdata = os.environ["APPDATA"]
    startup = os.path.join(appdata, "Microsoft", "Windows", "Start Menu", "Programs", "Startup", "mom-sec-test-should-never-exist.txt")
    good = [{"op": "move", "from": os.path.join(desk, n), "to": os.path.join(doc, "Important", n)} for n in names]
    good.append({"op": "move", "from": os.path.join(desk, folder_name), "to": os.path.join(doc, "Important", folder_name)})
    bad = [
        ("network share as source", {"op": "move", "from": "\\\\localhost\\C$\\Windows\\notepad.exe", "to": os.path.join(doc, "n.exe")}),
        ("network share as destination", {"op": "move", "from": victim, "to": "\\\\localhost\\C$\\Users\\Public\\mom-sec-test.txt"}),
        ("device path", {"op": "recycle", "from": "\\\\.\\C:\\Windows\\notepad.exe"}),
        ("C:\\Windows", {"op": "move", "from": "C:\\Windows\\notepad.exe", "to": os.path.join(doc, "n2.exe")}),
        ("Startup folder as destination", {"op": "move", "from": victim, "to": startup}),
        ("Temp outside the chosen folder as source", {"op": "move", "from": os.path.join(outside, "secret.txt"), "to": os.path.join(doc, "secret.txt")}),
        ("Temp outside the chosen folder as destination", {"op": "move", "from": victim, "to": os.path.join(outside, "victim.txt")}),
        ("alternate data stream", {"op": "move", "from": victim + ":stream", "to": os.path.join(doc, "s.txt")}),
        ("short name of Program Files", {"op": "move", "from": short(os.environ["ProgramFiles"]), "to": os.path.join(doc, "pf")}),
        ("short name of AppData as the last part", {"op": "recycle", "from": short(os.path.dirname(appdata))}),
        ("short-name alias of a folder", {"op": "move", "from": alias, "to": os.path.join(doc, "aliased")}),
        ("short-name alias on the way to a file", {"op": "move", "from": os.path.join(alias, "inside.txt"), "to": os.path.join(doc, "inside.txt")}),
        ("forward slashes", {"op": "move", "from": victim.replace("\\", "/"), "to": os.path.join(doc, "fs.txt")}),
        ("a doubled backslash", {"op": "move", "from": victim.replace("\\Downloads\\", "\\Downloads\\\\"), "to": os.path.join(doc, "db.txt")}),
        ("a '.' folder", {"op": "move", "from": victim.replace("\\Downloads\\", "\\Downloads\\.\\"), "to": os.path.join(doc, "dot.txt")}),
        ("'..' in the destination", {"op": "move", "from": victim, "to": os.path.join(doc, "..", "..", "outside", "up.txt")}),
        ("a trailing backslash", {"op": "recycle", "from": longdir + "\\"}),
        ("a wildcard", {"op": "recycle", "from": os.path.join(dl, "*.txt")}),
        ("a control character in a name", {"op": "move", "from": victim, "to": os.path.join(doc, "new\nline.txt")}),
        ("a file reached through a junction that leads out", {"op": "move", "from": os.path.join(out_link, "secret.txt"), "to": os.path.join(doc, "got.txt")}),
        ("a junction itself", {"op": "recycle", "from": out_link}),
        ("a junction itself, even one that stays inside", {"op": "move", "from": jump, "to": os.path.join(doc, "jump")}),
        ("a destination through a junction that leads out", {"op": "move", "from": victim, "to": os.path.join(out_link, "dropped.txt")}),
        ("a main folder itself", {"op": "recycle", "from": desk}),
        ("a path through a file", {"op": "move", "from": victim, "to": os.path.join(longdir, "inside.txt", "under-a-file.txt")}),
        ("something that is not a path", {"op": "recycle", "from": 7}),
    ]
    if not has_short:
        bad = [b for b in bad if "alias" not in b[0]]
        note("this disk doesn't create short names for new folders: the two short-name alias actions were not run "
             "(the short names of Program Files and AppData were)")

    def make_plan(file_name, actions, schema=None):
        actions = [dict(a, id=i + 1) for i, a in enumerate(actions)]
        plan = {"schema": schema or os.path.basename(SKILL) + "/plan@1", "plan_id": sa.plan_fingerprint(actions), "actions": actions}
        path = os.path.join(T, file_name)
        json.dump(plan, open(path, "w", encoding="utf-8"), ensure_ascii=False)
        return path, plan, "TIDY-APPROVE plan=%s ids=1-%d" % (plan["plan_id"], len(actions))

    plan_path, plan, approve = make_plan("plan.json", good + [b[1] for b in bad])
    log1 = os.path.join(T, "log1")
    before = tree(root)

    # ------------------------------------------------------------ 1. dry run
    print("\n-- 1. dry run: good actions would happen, hostile ones are refused")
    r = entry(plan_path, "--approve", approve, "--root", root, "--log-dir", log1)
    lines = by_id(r.stdout)
    check("dry run exits clean", r.returncode == 0, r.stderr or r.stdout)
    # The alias rule, tested directly: a path whose real name differs from how it's written is refused.
    g = sa.Guard(root)
    check("guard accepts a plain path in the folder", g.why_not(victim) is None, g.why_not(victim))
    with patched(sa, "long_name", lambda p: os.path.join(os.path.dirname(p), "Some Other Real Name")):
        check("guard refuses a path that isn't the file's real name", "real name" in (g.why_not(victim) or ""), g.why_not(victim))
    with patched(sa, "long_name", lambda p: None):
        check("guard refuses a path Windows can't resolve", "resolve" in (g.why_not(victim) or ""), g.why_not(victim))
    pf = os.environ["ProgramFiles"]
    check("long_name spells Program Files' short alias out in full", sa.long_name(short(pf)).lower() == pf.lower(), (short(pf), sa.long_name(short(pf))))
    check("long_name keeps a name ending in a dot exact", sa.long_name(os.path.join(desk, "ends in a dot.")).endswith("ends in a dot."))
    check("long_name keeps parts that don't exist yet", sa.long_name(os.path.join(doc, "New", "x.txt")) == os.path.join(doc, "New", "x.txt"))
    check("dry run: the %d good moves would happen (a name ending in a dot and a folder too)" % len(good),
          all(only(lines, i + 1, "WOULD") for i in range(len(good))), r.stdout)
    for k, (why, _) in enumerate(bad):
        i = len(good) + k + 1
        check("refused: " + why, only(lines, i, "REFUSED"), lines.get(i) or r.stdout[-400:])
    check("dry run changed nothing", tree(root) == before and not os.path.exists(log1))

    # ------------------------------------------------------------ 2. the plan id and the approval
    print("\n-- 2. the plan id is a fingerprint; the approval must name it")
    tampered = json.loads(json.dumps(plan))
    tampered["actions"][0]["to"] = os.path.join(doc, "elsewhere.pdf")
    p2 = os.path.join(T, "plan-tampered.json")
    json.dump(tampered, open(p2, "w", encoding="utf-8"), ensure_ascii=False)
    r = entry(p2, "--approve", approve, "--root", root, "--log-dir", log1, "--apply")
    check("a plan edited after approval is refused", r.returncode != 0 and "changed after" in (r.stderr + r.stdout) and tree(root) == before, r.stderr or r.stdout)
    fixed = [{"id": 1, "op": "move", "from": "C:\\a\\b \u2019 c.txt", "to": "C:\\d\\e.txt"}, {"id": 2, "op": "recycle", "from": "C:\\f.iso", "bytes": 5},
             {"id": 7, "op": "remove-empty-dir", "from": "C:\\g", "why": "not part of the id"}]
    core = [[a["id"], a["op"], a["from"], a.get("to")] for a in fixed]
    check("the fingerprint is the first 10 hex digits of SHA-256 over [id, op, from, to]",
          sa.plan_fingerprint(fixed) == hashlib.sha256(json.dumps(core, ensure_ascii=False).encode("utf-8")).hexdigest()[:10])
    check("the fingerprint of a known plan hasn't changed", sa.plan_fingerprint(fixed) == "ffec2ca3f9", sa.plan_fingerprint(fixed))
    one_path, one, _ = make_plan("plan-one.json", [good[3]], schema="desktop-icons/plan@1")
    pid = one["plan_id"]
    for word in ("DESKTOP-APPROVE", "RENAME-APPROVE", "TIDY-APPROVE"):
        r = entry(one_path, "--approve", "%s plan=%s ids=1" % (word, pid), "--root", root, "--log-dir", log1)
        check("an approval line starting %s is read" % word, r.returncode == 0 and only(by_id(r.stdout), 1, "WOULD"), r.stderr or r.stdout)
    r = entry(one_path, "--approve", "DESKTOP-APPROVE plan=%s ids=1\nmoves: 1\nnote: please" % pid, "--root", root, "--log-dir", log1)
    check("the whole copied approval (summary and note lines too) is read", r.returncode == 0 and only(by_id(r.stdout), 1, "WOULD"), r.stderr or r.stdout)
    r = entry(one_path, "--approve", "DESKTOP-APPROVE ids=1", "--root", root, "--log-dir", log1, "--apply")
    check("an approval without the plan id is refused", r.returncode != 0 and tree(root) == before, r.stderr or r.stdout)
    r = entry(one_path, "--approve", "DESKTOP-APPROVE plan=%s ids=1" % ("0" * 10), "--root", root, "--log-dir", log1, "--apply")
    check("an approval for another plan id is refused", r.returncode != 0 and tree(root) == before, r.stderr or r.stdout)
    r = entry(one_path, "--approve", "DESKTOP-APPROVE plan=%s ids=1-2" % pid, "--root", root, "--log-dir", log1, "--apply")
    check("an approval naming an id the plan doesn't have is refused", r.returncode != 0 and tree(root) == before, r.stderr or r.stdout)
    r = entry(one_path, "--root", root, "--log-dir", log1, "--apply")
    check("no approval at all is refused", r.returncode != 0 and tree(root) == before, r.stderr or r.stdout)
    r = entry(one_path, "--approve", "DESKTOP-APPROVE plan=%s ids=none" % pid, "--root", root, "--log-dir", log1)
    check("ids=none approves nothing", r.returncode == 0 and not by_id(r.stdout), r.stderr or r.stdout)
    for schema in ("tidy-folders/survey@1", "plan@1", "x/plan@2", ""):
        wrong = dict(one, schema=schema)
        pw = os.path.join(T, "plan-wrong-schema.json")
        json.dump(wrong, open(pw, "w", encoding="utf-8"), ensure_ascii=False)
        r = entry(pw, "--approve", "DESKTOP-APPROVE plan=%s ids=1" % pid, "--root", root, "--log-dir", log1, "--apply")
        check("a file whose schema is %r is not taken for a plan" % schema, r.returncode != 0 and tree(root) == before, r.stderr or r.stdout)
    os.remove(os.path.join(T, "plan-wrong-schema.json"))

    # ------------------------------------------------------------ 3. --root
    print("\n-- 3. --root may not be a protected folder or hold one")
    r = entry(plan_path, "--approve", approve, "--root", os.environ["LOCALAPPDATA"], "--log-dir", log1)
    check("--root on AppData\\Local itself is refused", r.returncode != 0 and "--root" in (r.stderr + r.stdout), r.stderr or r.stdout)
    r = entry(plan_path, "--approve", approve, "--root", "C:\\", "--log-dir", log1)
    check("--root on a drive is refused", r.returncode != 0 and "--root" in (r.stderr + r.stdout), r.stderr or r.stdout)

    # ------------------------------------------------------------ 4. apply, then both undo routes
    print("\n-- 4. apply for real (work folder only), then both ways back")
    r = entry(plan_path, "--approve", approve, "--root", root, "--log-dir", log1, "--apply")
    after = tree(root)
    moved = ["\\Documents\\Important\\" + n for n in names] + ["\\Documents\\Important\\" + folder_name + "\\sub\\in it.txt"]
    check("apply moved the good files and the folder", r.returncode == 0 and all(m in after for m in moved), r.stdout[-800:] + r.stderr)
    check("apply left everything else alone", os.path.exists(victim) and os.path.exists(os.path.join(outside, "secret.txt"))
          and os.path.exists(os.path.join(longdir, "inside.txt")) and not os.path.exists(startup) and os.path.isdir(jump))
    with open(os.path.join(log1, "revert.ps1"), "rb") as a, open(REVERT, "rb") as b:
        check("the revert script next to the log is the skill's fixed file, byte for byte", a.read() == b.read())
    r = revert(log1, "-Root", root, "-Preview")
    check("revert.ps1 -Preview changes nothing", r.returncode == 0 and tree(root) == after, r.stdout + r.stderr)
    r = revert(log1, "-Root", root)
    check("revert.ps1 puts everything back, odd names and the folder included", r.returncode == 0 and tree(root) == before, r.stdout + r.stderr)
    check("revert.ps1 removed the folder the apply made", not os.path.exists(os.path.join(doc, "Important")))
    check("nothing in a file name ran (no stray files)", sorted(os.listdir(T)) == sorted(
        [MARK, "log1", "outside", "plan-one.json", "plan-tampered.json", "plan.json", "profile"]), os.listdir(T))
    log2 = os.path.join(T, "log2")
    r = entry(plan_path, "--approve", approve, "--root", root, "--log-dir", log2, "--apply")
    r = entry("--undo", log2, "--root", root)
    check("the undo's dry run changes nothing", r.returncode == 0 and tree(root) == after and "WOULD" in r.stdout, r.stdout[-600:] + r.stderr)
    r = entry("--undo", log2, "--root", root, "--apply")
    check("the undo puts everything back", r.returncode == 0 and tree(root) == before and not os.path.exists(os.path.join(doc, "Important")),
          r.stdout[-600:] + r.stderr)
    # A name taken again is never overwritten, by either route
    r = entry(plan_path, "--approve", approve, "--root", root, "--log-dir", os.path.join(T, "log2b"), "--apply")
    open(os.path.join(desk, "plain.txt"), "w").write("new")
    r1 = revert(os.path.join(T, "log2b"), "-Root", root)
    r2 = entry("--undo", os.path.join(T, "log2b"), "--root", root, "--apply")
    check("a name that is taken again is not overwritten (both routes skip it)",
          open(os.path.join(desk, "plain.txt")).read() == "new" and os.path.exists(os.path.join(doc, "Important", "plain.txt"))
          and "taken again" in r1.stdout and "exists again" in r2.stdout, r1.stdout + r2.stdout)
    os.remove(os.path.join(desk, "plain.txt"))
    r = entry("--undo", os.path.join(T, "log2b"), "--root", root, "--apply")
    check("...and comes back once the name is free", tree(root) == before, r.stdout + r.stderr)
    # moves never overwrite on the way out either
    os.makedirs(os.path.join(doc, "Important"))
    open(os.path.join(doc, "Important", "plain.txt"), "w").write("already here")
    r = entry(one_path, "--approve", "T-APPROVE plan=%s ids=1" % pid, "--root", root, "--log-dir", os.path.join(T, "log2c"), "--apply")
    check("a move onto a taken name gets \" (2)\" instead of overwriting", open(os.path.join(doc, "Important", "plain.txt")).read() == "already here"
          and os.path.exists(os.path.join(doc, "Important", "plain (2).txt")), r.stdout + r.stderr)
    r = revert(os.path.join(T, "log2c"), "-Root", root)
    os.remove(os.path.join(doc, "Important", "plain.txt"))
    os.rmdir(os.path.join(doc, "Important"))
    check("...and revert.ps1 brings it back under its own name", tree(root) == before, r.stdout + r.stderr)

    # ------------------------------------------------------------ 5. an edited undo log
    print("\n-- 5. an edited undo log can't send a file into Startup, Windows, a share, or anywhere by another name")
    log3 = os.path.join(T, "log3")
    rows = [
        {"op": "move", "from": startup, "to": victim},
        {"op": "move", "from": "\\\\localhost\\C$\\Users\\Public\\mom-sec-test.txt", "to": victim},
        {"op": "move", "from": os.path.join(outside, "moved.txt"), "to": victim},
        {"op": "move", "from": os.path.join(doc, "back.txt"), "to": "C:\\Windows\\win.ini"},
        {"op": "move", "from": os.path.join(doc, "..", "..", "up.txt"), "to": victim},
        {"op": "remove-empty-dir", "from": os.path.join(appdata, "mom-sec-test-dir")},
        {"op": "move", "from": os.path.join(doc, "from-pf"), "to": short(os.environ["ProgramFiles"])},
        {"op": "move", "from": os.path.join(doc, "back2.txt"), "to": victim.replace("\\", "/")},
        {"op": "move", "from": os.path.join(doc, "sub.", "back3.txt"), "to": victim},
        {"op": "move", "from": os.path.join(out_link, "landed.txt"), "to": victim},
        {"op": "move", "from": os.path.join(doc, "the-link"), "to": out_link},
        {"op": "move", "from": os.path.join(doc, "took.txt"), "to": os.path.join(out_link, "secret.txt")},
        {"op": "move", "from": os.path.join(longdir, "inside.txt", "under-a-file.txt"), "to": victim},
        {"op": "mkdir", "from": os.path.join(out_link)},
        {"op": "move", "from": "", "to": victim},
    ]
    write_log(log3, rows)

    def untouched():
        return (os.path.exists(victim) and not os.path.exists(startup) and os.path.exists("C:\\Windows\\win.ini")
                and not os.path.exists(os.path.join(outside, "moved.txt")) and not os.path.exists(os.path.join(appdata, "mom-sec-test-dir"))
                and not os.path.exists(os.path.join(T, "up.txt")) and not os.path.exists(os.path.join(doc, "back.txt"))
                and os.path.isdir(os.environ["ProgramFiles"]) and tree(root) == before
                and os.path.isdir(out_link) and sorted(os.listdir(outside)) == ["secret.txt"])

    r = entry("--undo", log3, "--root", root, "--apply")
    check("the Python undo refuses every row of an edited log", r.stdout.count("REFUSED") == len(rows) and untouched(), r.stdout + r.stderr)
    r = revert(log3, "-Root", root)
    check("revert.ps1 refuses every row of an edited log", r.stdout.count("REFUSED") == len(rows) and r.returncode == 1 and untouched(), r.stdout + r.stderr)
    r = revert(log3, "-Root", os.environ["LOCALAPPDATA"])
    check("revert.ps1 -Root on AppData\\Local exempts nothing", r.stdout.count("REFUSED") == len(rows) and untouched(), r.stdout + r.stderr)
    # A log with no usable columns, and an empty one: nothing moves, no crash into a move
    log3b = os.path.join(T, "log3b")
    write_log(log3b, [])
    with open(os.path.join(log3b, "undo-log.csv"), "w", encoding="utf-8-sig", newline="") as f:
        f.write('"when","what"\r\n"now","%s"\r\n' % victim)
    r = revert(log3b, "-Root", root)
    check("revert.ps1: a log without the expected columns moves nothing", r.returncode == 1 and "FAILED" in r.stdout and untouched(), r.stdout + r.stderr)
    r = entry("--undo", log3b, "--root", root, "--apply")
    check("the Python undo: a log without the expected columns moves nothing", untouched(), r.stdout + r.stderr)
    open(os.path.join(log3b, "undo-log.csv"), "w").close()
    r = revert(log3b, "-Root", root)
    check("revert.ps1: an empty log is nothing to do", r.returncode == 0 and untouched(), r.stdout + r.stderr)
    text = open(REVERT, encoding="utf-8").read()
    banned = [w for w in ("Move-Item", "Remove-Item", "Copy-Item", "Get-Item", "Invoke-Expression", "iex ", "Import-Csv", "Get-ChildItem",
                          "Test-Path", "Join-Path", "Resolve-Path", "Get-Content", "Rename-Item", "New-Item") if w.lower() in text.lower()]
    check("revert.ps1 uses no PowerShell file cmdlet and no Invoke-Expression", not banned, banned)
    check("revert.ps1 is plain ASCII with no file names of its own", all(ord(c) < 128 for c in text) and "Users\\" not in text)

    # ------------------------------------------------------------ 6. --within
    print("\n-- 6. --within: every source and destination must be inside one of the named folders")
    plain_txt = os.path.join(desk, "plain.txt")
    w_actions = [
        {"op": "move", "from": plain_txt, "to": os.path.join(doc, "Sorted", "plain.txt")},                      # 1 inside -> inside
        {"op": "move", "from": victim, "to": os.path.join(doc, "v.txt")},                                       # 2 source outside
        {"op": "move", "from": plain_txt, "to": os.path.join(dl, "p.txt")},                                     # 3 destination outside
        {"op": "move", "from": os.path.join(jump, "victim.txt"), "to": os.path.join(doc, "j.txt")},              # 4 source: inside by spelling, outside through a junction
        {"op": "move", "from": plain_txt, "to": os.path.join(jump, "new.txt")},                                 # 5 destination: the same
        {"op": "recycle", "from": victim.upper()},                                                              # 6 outside, spelled in capitals
        {"op": "move", "from": plain_txt.upper(), "to": os.path.join(doc, "Sorted", "PLAIN.TXT")},              # 7 inside, spelled in capitals
        {"op": "recycle", "from": os.path.join(outside, "secret.txt")},                                         # 8 outside and protected
        {"op": "move", "from": os.path.join(alias, "inside.txt"), "to": os.path.join(doc, "i.txt")},             # 9 short-name alias (if this disk has them)
    ]
    wp, wplan, wapprove = make_plan("plan-within.json", w_actions)
    within = ["--within", desk, "--within", doc]
    r = entry(wp, "--approve", wapprove, "--root", root, "--log-dir", log1, *within)
    L = by_id(r.stdout)
    check("--within: a move from one named folder to another would happen", r.returncode == 0 and only(L, 1, "WOULD"), r.stdout + r.stderr)
    check("--within: a source outside is refused", only(L, 2, "REFUSED", "outside the folders"), L.get(2))
    check("--within: a destination outside is refused", only(L, 3, "REFUSED", "destination is outside the folders"), L.get(3))
    check("--within: a source inside by spelling but outside through a junction is refused", only(L, 4, "REFUSED", "outside the folders"), L.get(4))
    check("--within: a destination inside by spelling but outside through a junction is refused", only(L, 5, "REFUSED", "destination is outside the folders"), L.get(5))
    check("--within: capitals don't move a path that is outside to the inside", only(L, 6, "REFUSED", "outside the folders"), L.get(6))
    check("--within: capitals don't move a path that is inside to the outside (same file, same answer)", only(L, 7, "WOULD"), L.get(7))
    check("--within: a protected place stays refused for its own reason", only(L, 8, "REFUSED", "AppData"), L.get(8))
    if has_short:
        check("--within: a short-name spelling is refused", only(L, 9, "REFUSED", "real name"), L.get(9))
    r0 = entry(wp, "--approve", wapprove, "--root", root, "--log-dir", log1)
    L0 = by_id(r0.stdout)
    check("without --within the same actions would happen: it is the limit that refuses them (a junction that resolves "
          "to an allowed place is followed)", all(only(L0, i, "WOULD") for i in (1, 2, 3, 4, 5, 7)) and only(L0, 6, "WOULD"), r0.stdout)
    r = entry(wp, "--approve", wapprove, "--root", root, "--log-dir", log1, "--within", desk.upper(), "--within", doc.upper())
    L = by_id(r.stdout)
    check("--within spelled in capitals is the same folder: same answers", r.returncode == 0 and only(L, 1, "WOULD") and only(L, 2, "REFUSED")
          and only(L, 3, "REFUSED") and only(L, 4, "REFUSED") and only(L, 5, "REFUSED"), r.stdout + r.stderr)
    r = entry(wp, "--approve", wapprove, "--root", root, "--log-dir", log1, "--within", jump, "--within", doc)
    L = by_id(r.stdout)
    check("--within a junction: only paths written through it and really inside it pass", r.returncode == 0 and only(L, 4, "WOULD")
          and only(L, 2, "REFUSED", "outside the folders") and only(L, 1, "REFUSED", "outside the folders"), r.stdout + r.stderr)
    # the folder itself is not "inside" it
    b_actions = [{"op": "recycle", "from": box}, {"op": "move", "from": os.path.join(box, "in.txt"), "to": os.path.join(box, "Sub", "in.txt")},
                 {"op": "move", "from": os.path.join(box, "in.txt"), "to": os.path.join(doc, "out.txt")},
                 {"op": "move", "from": box, "to": os.path.join(doc, "Box 2")}]
    bp, _, bapprove = make_plan("plan-box.json", b_actions)
    r = entry(bp, "--approve", bapprove, "--root", root, "--log-dir", log1, "--within", box)
    L = by_id(r.stdout)
    check("--within: the named folder itself is not inside it (it can't be removed or moved)", only(L, 1, "REFUSED", "outside the folders")
          and only(L, 4, "REFUSED", "outside the folders"), r.stdout)
    check("--within: moves inside the named folder would happen, moves out of it are refused", only(L, 2, "WOULD") and only(L, 3, "REFUSED", "destination is outside"), r.stdout)
    for why, value, word in (("ending in a backslash", desk + "\\", "plain path"), ("not a full path", "Desktop", "full path"),
                             ("written with forward slashes", desk.replace("\\", "/"), "full path"),
                             ("with '..' in it", os.path.join(desk, "..", "Desktop"), "plain path"),
                             ("given by a short name", short(os.environ["ProgramFiles"]), "real name"),
                             ("that doesn't exist", os.path.join(root, "Nowhere"), "not an existing folder"),
                             ("that is a file", plain_txt, "not an existing folder"),
                             ("on a share", "\\\\localhost\\C$\\Users", "full path")):
        r = entry(wp, "--approve", wapprove, "--root", root, "--log-dir", log1, "--apply", "--within", value)
        check("--within %s stops the run" % why, r.returncode != 0 and "--within" in (r.stderr + r.stdout) and word in (r.stderr + r.stdout)
              and tree(root) == before, r.stderr or r.stdout)
    # for real, and back, with the limit on both
    log4 = os.path.join(T, "log4")
    r = entry(wp, "--approve", wapprove, "--root", root, "--log-dir", log4, "--apply", *within)
    now = tree(root)
    check("--within: apply moves what is inside the limit and nothing else",
          r.returncode == 0 and "\\Documents\\Sorted\\plain.txt" in now and "\\Desktop\\plain.txt" not in now and os.path.exists(victim)
          and len(now) == len(before) and not os.path.exists(os.path.join(dl, "p.txt")) and not os.path.exists(os.path.join(dl, "new.txt")),
          r.stdout + r.stderr)
    w_rows = [{"op": "move", "from": os.path.join(dl, "pulled-out.txt"), "to": os.path.join(doc, "Sorted", "plain.txt")},
              {"op": "move", "from": os.path.join(jump, "pulled-out2.txt"), "to": os.path.join(doc, "Sorted", "plain.txt")}]
    log5 = os.path.join(T, "log5")
    write_log(log5, w_rows)
    r = entry("--undo", log5, "--root", root, "--apply", *within)
    check("--within: an undo log row that leads outside the limit is refused", r.stdout.count("REFUSED") == 2 and tree(root) == now, r.stdout + r.stderr)
    r = entry("--undo", log5, "--root", root)
    check("...and the same rows would be allowed with no limit (dry run only)", r.stdout.count("WOULD") == 2 and tree(root) == now, r.stdout + r.stderr)
    r = entry("--undo", log4, "--root", root, "--apply", *within)
    check("--within: the undo, with the same limit, puts it back", r.returncode == 0 and tree(root) == before, r.stdout + r.stderr)

    # ------------------------------------------------------------ 7. a skill's own rules
    print("\n-- 7. a skill can add places and reasons (never remove one)")
    inner = os.path.join(box, "Keep", "Inner")
    seen = []

    def my_places(given_root):
        return [([inner], "inside a place the skill added", "holds a place the skill added")]

    def my_check(action, guard):
        seen.append(action["id"])
        return "the skill said no" if action["id"] == 4 else None

    h_actions = [{"op": "move", "from": plain_txt, "to": os.path.join(inner, "p.txt")},
                 {"op": "move", "from": os.path.join(box, "Keep"), "to": os.path.join(doc, "Keep")},
                 {"op": "move", "from": "C:\\Windows\\notepad.exe", "to": os.path.join(doc, "n.exe")},
                 {"op": "move", "from": plain_txt, "to": os.path.join(doc, "four.txt")},
                 {"op": "move", "from": plain_txt, "to": os.path.join(doc, "five.txt")},
                 {"op": "move", "from": victim, "to": 5}]
    hp, _, happrove = make_plan("plan-hooks.json", h_actions)
    out, code = in_process([hp, "--approve", happrove, "--root", root, "--log-dir", log1], places=my_places, check=my_check)
    L = by_id(out)
    check("a place the skill added: nothing goes into it", only(L, 1, "REFUSED", "destination is inside a place the skill added"), out)
    check("a place the skill added: what holds it is not moved", only(L, 2, "REFUSED", "holds a place the skill added"), out)
    check("the skill's own check can refuse an action", only(L, 4, "REFUSED", "the skill said no") and only(L, 5, "WOULD"), out)
    check("the skill's check is asked only about actions the engine's own rules allow", seen == [4, 5] and only(L, 3, "REFUSED", "Windows")
          and only(L, 6, "REFUSED"), (seen, out))

    # ------------------------------------------------------------ 8. what can't be examined is refused
    print("\n-- 8. anything that can't be examined is refused, never treated as fine")
    real_lstat, real_realpath, real_scandir, real_listdir = os.lstat, os.path.realpath, os.scandir, os.listdir

    def lstat_failing(target, error):
        def fake(p, *a, **kw):
            if os.path.normcase(str(p)) == os.path.normcase(X + target):
                raise error
            return real_lstat(p, *a, **kw)
        return fake

    denied = PermissionError(13, "Access is denied")
    with patched(os, "lstat", lstat_failing(victim, denied)):
        check("is_link: an error other than not-found counts as a link", sa.is_link(victim) is True)
        check("state: an error other than not-found is 'unknown', and the name counts as taken", sa.state(victim) == "unknown" and sa.exists(victim) is True)
        check("free_name: a name that can't be examined gives no answer", sa.free_name(victim) is None)
        check("guard: a path that can't be looked up is refused", sa.Guard(root).why_not(victim) is not None, sa.Guard(root).why_not(victim))
    with patched(os, "lstat", lstat_failing(victim, FileNotFoundError(2, "No such file"))):
        check("is_link: not-found is simply not a link, and the name is free", sa.is_link(victim) is False and sa.state(victim) == "absent")
    check("is_link: a junction is a link; a plain file and a plain folder are not", sa.is_link(jump) and not sa.is_link(victim) and not sa.is_link(dl))
    resolved = sa.real(victim)
    with patched(sa, "real", lambda p: resolved), patched(os, "lstat", lstat_failing(dl, denied)):
        why = g.why_not(victim)
        check("guard: a folder on the way that can't be examined refuses the path", why == sa.CANT_EXAMINE, why)

    def realpath_failing(p, *a, **kw):
        raise OSError(1920, "The file cannot be accessed by the system")

    with patched(os.path, "realpath", realpath_failing):
        check("real: a path that can't be resolved gives None, not the unresolved path", sa.real(victim) is None and sa.real_folder(dl) is None)
        why = sa.Guard.why_not(g, victim)
        check("guard: a path that can't be resolved is refused", why == sa.CANT_RESOLVE, why)
        try:
            sa.Guard(root)
            stopped = False
        except SystemExit as ex:
            stopped = "--root" in str(ex.code)
        check("--root that can't be resolved stops the run", stopped)
        try:
            g2 = sa.Guard()
            ok = all(sa.norm(v) in g2.untouchable for v in g2.kf.values() if v)
        except BaseException as ex:  # noqa: B902
            ok = repr(ex)
        check("main folders that can't be resolved are still protected as written", ok is True, ok)
    with patched(sa, "real", lambda p: p):   # as if realpath had quietly given up
        why = g.why_not(os.path.join(jump, "victim.txt"))
        check("guard: a link left in the 'resolved' path is refused", why == "reached through a link that could not be followed safely", why)
    check("...and the same path is fine when the junction was really followed", g.why_not(os.path.join(jump, "victim.txt")) is None)

    def scandir_failing(p=".", *a, **kw):
        if os.path.normcase(str(p)) == os.path.normcase(X + os.path.join(box, "Keep")):
            raise denied
        return real_scandir(p, *a, **kw)

    with patched(os, "scandir", scandir_failing):
        info = sa.tree_info(box)
        check("tree_info: a folder that can't be read counts as not empty and as holding a link", info[0] >= 1 and info[3] is True, info)
        why = g.why_not(os.path.join(box, "Keep", "new.txt"), is_source=False)
        check("guard: nothing new goes into a folder that can't be listed", why == sa.CANT_RESOLVE, why)

    def listdir_failing(p=".", *a, **kw):
        raise denied

    with patched(os, "listdir", listdir_failing):
        check("looks_installed: a folder that can't be listed is left alone", sa.looks_installed(box) is True)
    # a copy never goes through a link: forward (existing rule) and back (undo), as if on another drive
    holder = os.path.join(doc, "Holder")
    os.makedirs(os.path.join(holder, "deep"))
    open(os.path.join(holder, "deep", "f.txt"), "w").write("h")
    has_inner = junction(os.path.join(holder, "deep", "way-out"), dl)
    x_actions = [{"op": "move", "from": holder, "to": os.path.join(root, "Videos", "Holder")}, {"op": "recycle", "from": holder},
                 {"op": "remove-empty-dir", "from": holder}]
    xpath, _, xapprove = make_plan("plan-cross.json", x_actions)
    held = tree(root)
    with patched(sa, "same_volume", lambda a, b: False):
        out, code = in_process([xpath, "--approve", xapprove, "--root", root, "--log-dir", os.path.join(T, "logx"), "--apply", "--allow-cross-drive-folders"])
        L = by_id(out)
        check("a folder that holds a link is not copied to another drive", only(L, 1, "REFUSED", "holds a link") and tree(root) == held, out)
        logx = os.path.join(T, "log6")
        write_log(logx, [{"op": "move", "from": os.path.join(root, "Videos", "Was here"), "to": holder}])
        out, code = in_process(["--undo", logx, "--root", root, "--apply"])
        check("the undo doesn't copy a folder that holds a link back from another drive", "REFUSED" in out and "holds a link" in out and tree(root) == held, out)
    check("a folder that holds a link doesn't go to the Recycle Bin whole, and isn't called empty", only(L, 2, "REFUSED", "holds a link") and only(L, 3, "SKIP"), out)
    out, code = in_process(["--undo", logx, "--root", root])
    check("...on the same drive it is a plain rename and would go back", "WOULD" in out, out)

    # A folder that can be passed through but not listed
    lock = os.path.join(root, "Music", "locked")
    os.makedirs(os.path.join(lock, "sub"))
    open(os.path.join(lock, "known.txt"), "w").write("k")
    open(os.path.join(lock, "sub", "deep.txt"), "w").write("d")
    has_lock_link = junction(os.path.join(lock, "j"), os.path.join(root, "Videos"))
    user = os.environ.get("USERNAME", "")
    made = run(["icacls", lock, "/deny", user + ":(RD)"])
    try:
        os.listdir(lock)
        is_locked = False
    except OSError:
        is_locked = made.returncode == 0
        locked.append(lock)
    if not is_locked:
        note("could not make an unlistable folder with icacls (%s): those checks were not run" % (made.stderr or made.stdout).strip()[:120])
    else:
        l_actions = [{"op": "move", "from": os.path.join(lock, "known.txt"), "to": os.path.join(doc, "k.txt")},
                     {"op": "recycle", "from": os.path.join(lock, "known.txt")},
                     {"op": "move", "from": os.path.join(lock, "sub", "deep.txt"), "to": os.path.join(doc, "d.txt")},
                     {"op": "move", "from": victim, "to": os.path.join(lock, "j", "dropped.txt")},
                     {"op": "move", "from": victim, "to": os.path.join(lock, "new.txt")},
                     {"op": "move", "from": os.path.join(lock, "j", "anything.txt"), "to": os.path.join(doc, "a.txt")},
                     {"op": "remove-empty-dir", "from": os.path.join(lock, "sub")}]
        lp, _, lapprove = make_plan("plan-locked.json", l_actions)
        r = entry(lp, "--approve", lapprove, "--root", root, "--log-dir", os.path.join(T, "log7"), "--apply")
        L = by_id(r.stdout)
        check("a plan action through a folder that can't be listed is refused (7 of 7)", all(only(L, i, "REFUSED") for i in range(1, 8)), r.stdout + r.stderr)
        l_rows = [{"op": "move", "from": os.path.join(lock, "j", "dropped.txt"), "to": victim},
                  {"op": "move", "from": os.path.join(lock, "known.txt"), "to": victim},
                  {"op": "move", "from": os.path.join(lock, "new.txt"), "to": victim},
                  {"op": "move", "from": os.path.join(doc, "k.txt"), "to": os.path.join(lock, "known.txt")},
                  {"op": "move", "from": os.path.join(doc, "d.txt"), "to": os.path.join(lock, "sub", "deep.txt")},
                  {"op": "mkdir", "from": os.path.join(lock, "sub")},
                  {"op": "remove-empty-dir", "from": os.path.join(lock, "j", "made")}]
        log8 = os.path.join(T, "log8")
        write_log(log8, l_rows)

        def lock_untouched():
            return (os.path.exists(victim) and not os.listdir(os.path.join(root, "Videos")) and not os.path.exists(os.path.join(doc, "k.txt"))
                    and not os.path.exists(os.path.join(doc, "d.txt")) and os.path.lexists(X + os.path.join(lock, "known.txt"))
                    and os.path.lexists(X + os.path.join(lock, "sub", "deep.txt")) and not os.path.lexists(X + os.path.join(lock, "new.txt")))

        r = entry("--undo", log8, "--root", root, "--apply")
        check("the Python undo refuses every row through a folder that can't be listed", r.stdout.count("REFUSED") == len(l_rows) and lock_untouched(), r.stdout + r.stderr)
        r = revert(log8, "-Root", root)
        check("revert.ps1 refuses every row through a folder that can't be listed", r.stdout.count("REFUSED") == len(l_rows) and r.returncode == 1
              and "can't be examined" in r.stdout and lock_untouched(), r.stdout + r.stderr)
    for d in list(locked):
        if run(["icacls", d, "/remove:d", user]).returncode == 0:
            locked.remove(d)
    check("the unlistable folder is listable again", not locked and sorted(os.listdir(lock)) == ["j", "known.txt", "sub"], locked)

    # A folder on another drive: revert.ps1 leaves it for the Python undo. Read-only on that drive: preview and dry run only.
    here = os.path.splitdrive(T)[0].upper()
    others = [chr(65 + i) + ":" for i in range(26) if ctypes.windll.kernel32.GetLogicalDrives() & (1 << i)
              and ctypes.windll.kernel32.GetDriveTypeW(chr(65 + i) + ":\\") == 3 and chr(65 + i) + ":" != here]
    others = [d for d in others if sa.long_name(os.path.join(d + "\\", "mom-skill-test-not-a-real-folder", "x")) is not None]
    if not others:
        note("no second local drive here: revert.ps1's 'a folder on another drive' answer was not run")
    else:
        far = os.path.join(others[0] + "\\", "mom-skill-test-not-a-real-folder", "Was here")
        plain_dir = os.path.join(doc, "Box")
        log9 = os.path.join(T, "log9")
        write_log(log9, [{"op": "move", "from": far, "to": plain_dir}, {"op": "move", "from": far + " 2", "to": holder},
                         {"op": "move", "from": far + ".txt", "to": victim}])
        state_now = tree(root)
        r = revert(log9, "-Root", root, "-Preview")
        check("revert.ps1 (preview): a folder that came from another drive is MANUAL, a file would go back",
              r.stdout.count("MANUAL") == 2 and "apply_plan.py --undo" in r.stdout and r.stdout.count("BACK") == 1 and tree(root) == state_now
              and not os.path.exists(os.path.dirname(far)), r.stdout + r.stderr)
        r = entry("--undo", log9, "--root", root)
        check("the Python undo (dry run): a plain folder would go back to the other drive, one that holds a link is refused",
              r.stdout.count("WOULD") == 2 and r.stdout.count("REFUSED") == 1 and "holds a link" in r.stdout and tree(root) == state_now
              and not os.path.exists(os.path.dirname(far)), r.stdout + r.stderr)

    # ------------------------------------------------------------ 9. odds and ends
    print("\n-- 9. the rest")
    survey_py = os.path.join(SCRIPTS, "survey.py")
    if os.path.isfile(survey_py) and re.search(r"^def known_folders\(", open(survey_py, encoding="utf-8").read(), re.M):
        import survey  # noqa: E402  (the skill's own; read-only helpers)
        check("the skill's survey and the engine agree on where the main folders are",
              survey.known_folders(root) == sa.known_folders(root) and survey.known_folders() == sa.known_folders(),
              (survey.known_folders(), sa.known_folders()))
    else:
        check("the engine finds the fixture's main folders", sa.known_folders(root)["Desktop"] == desk and sa.known_folders(root)["Profile"] == root)
    src = open(os.path.join(SCRIPTS, "safe_apply.py"), encoding="utf-8").read()
    check("the engine imports nothing from the skill around it (standard library only)",
          not re.search(r"^\s*(import|from)\s+(survey|build_page|apply_plan|summarize)\b", src, re.M), "")
    check("the engine never builds a command from a path (no subprocess, os.system, eval or exec)",
          not re.search(r"\b(subprocess|os\.system|os\.popen|eval\(|exec\()", src), "")
    check("no __pycache__ left in the skill folder", pycache_before or not os.path.isdir(os.path.join(SCRIPTS, "__pycache__")))
    check("the junction cases really ran", has_junction and has_jump and has_inner and has_lock_link)
    check("the folder outside is untouched to the end", sorted(os.listdir(outside)) == ["secret.txt"], os.listdir(outside))
    check("Downloads is untouched to the end", sorted(os.listdir(dl)) == ["A Long Folder Name Here", "victim.txt"], os.listdir(dl))
finally:
    user = os.environ.get("USERNAME", "")
    for d in locked:
        run(["icacls", d, "/remove:d", user])
    for link in links:
        try:
            os.rmdir(link)   # the link itself, never what it points at
        except OSError:
            pass
    if not opts.keep and os.path.isfile(os.path.join(T, MARK)) and not any(os.path.lexists(link) for link in links):
        shutil.rmtree(X + T, ignore_errors=True)

print("\n%s: %d checks on %s (run through %s)%s" % ("ALL PASSED" if not fails else "%d FAILED" % len(fails), count[0],
                                                     os.path.basename(SKILL), opts.entry, "" if not fails else ": %s" % fails))
for n in notes:
    print("not run: " + n)
sys.exit(1 if fails else 0)

#!/usr/bin/env python3
"""Carry out the approved part of a tidy plan, with an undo log. Dry run unless --apply.

    python apply_plan.py plan.json --approve "TIDY-APPROVE plan=3f9a1c07be ids=1-40,52"
    python apply_plan.py plan.json --approval tidy-approval-3f9a1c07be.json --apply
    python apply_plan.py --undo <log folder>             # dry run of the revert
    python apply_plan.py --undo <log folder> --apply

Rules it enforces, whatever the plan says:
  * only ids from this plan, and only if the approval names this plan_id
  * never touches Windows, Program Files, ProgramData, AppData, registered program
    folders, launcher game libraries, drive roots or the user's top-level folders;
    checked on the real location too, so 8.3 short names and junctions can't dodge it
  * never acts on a symlink or junction, and never walks through one
  * file names are data: they are never put into a command or a script. The revert
    script (revert.ps1) is a fixed file that reads undo-log.csv
  * every file is addressed by its exact name (extended-length paths), so a name
    ending in a dot or space can't be mistaken for its neighbour
  * moves never overwrite: an existing name gets " (2)"
  * "recycle" goes to the Recycle Bin (Windows asks before anything would be
    deleted for good); files are skipped if they changed since the survey
  * "remove-empty-dir" re-checks that nothing is inside, then uses rmdir only
  * a move to another drive copies, checks the size, then recycles the original
  * every step is written to undo-log.csv as it happens

Python 3.8+, standard library only. Windows only for --apply.
"""
import argparse
import csv
import json
import os
import re
import shutil
import sys
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
import survey as sv  # noqa: E402  (shared helpers: known folders, launchers, installed programs)

OPS = {"move", "recycle", "remove-empty-dir"}
LOG_FIELDS = ["time", "id", "op", "from", "to", "status", "note"]
REVERT_PS1 = os.path.join(HERE, "revert.ps1")   # fixed script, copied as-is next to each undo log
LINK_TAGS = (0xA0000003, 0xA000000C)             # junction, symlink
MAX_SHELL_PATH = 259                             # the Recycle Bin API can't take longer paths


# ---------------------------------------------------------------- exact paths
def xp(path):
    """Extended-length form of an absolute Windows path: the exact name, no trimming of dots or spaces."""
    if not sv.IS_WIN or path.startswith("\\\\?\\"):
        return path
    if re.match(r"^[A-Za-z]:\\", path):
        return "\\\\?\\" + path
    if path.startswith("\\\\"):
        return "\\\\?\\UNC\\" + path[2:]
    return path


def plain(path):
    if path.startswith("\\\\?\\UNC\\"):
        return "\\\\" + path[8:]
    return path[4:] if path.startswith("\\\\?\\") else path


def exists(path):
    return os.path.lexists(xp(path))


def is_link(path):
    """A symlink or junction (OneDrive cloud placeholders are reparse points too, but not links)."""
    try:
        st = os.lstat(xp(path))
    except OSError:
        return False
    if getattr(st, "st_file_attributes", 0) & 0x400 and getattr(st, "st_reparse_tag", 0) in LINK_TAGS:
        return True
    return os.path.islink(xp(path))


def odd_name(path):
    """True if a path component ends in a dot or space: the Windows shell can't address those."""
    return any(part != part.rstrip(". ") for part in re.split(r"[\\/]", path)[1:])


def real(path):
    """Where the path really is: parent resolved through junctions and 8.3 short names."""
    parent, name = os.path.split(path.rstrip("\\/"))
    try:
        parent = plain(os.path.realpath(parent))
    except OSError:
        pass
    return os.path.join(parent, name)


def tree_info(path):
    """(files, bytes, folders deepest-first, has_link) for a folder, never walking through a link."""
    files = total = 0
    dirs, has_link, stack = [], False, [xp(path)]
    while stack:
        d = stack.pop()
        try:
            with os.scandir(d) as it:
                entries = list(it)
        except OSError:
            return files + 1, total, dirs, has_link  # unreadable: treat as not empty
        for e in entries:
            if is_link(e.path):
                has_link = True
            elif e.is_dir(follow_symlinks=False):
                dirs.append(e.path)
                stack.append(e.path)
            else:
                files += 1
                try:
                    total += e.stat(follow_symlinks=False).st_size
                except OSError:
                    pass
    dirs.sort(key=len, reverse=True)
    return files, total, dirs, has_link


def size_of(path):
    if os.path.isdir(xp(path)):
        f, b, _, _ = tree_info(path)
        return b, f
    return os.path.getsize(xp(path)), 1


def free_name(path):
    if not exists(path):
        return path
    base, ext = (path, "") if os.path.isdir(xp(path)) else os.path.splitext(path)
    i = 2
    while exists("%s (%d)%s" % (base, i, ext)):
        i += 1
    return "%s (%d)%s" % (base, i, ext)


def same_volume(a, b):
    return os.path.splitdrive(a)[0].lower() == os.path.splitdrive(b)[0].lower()


# ---------------------------------------------------------------- Recycle Bin
def recycle(path):
    """Send a file or folder to the Recycle Bin. Returns None on success, else an error string."""
    if not sv.IS_WIN:
        return "the Recycle Bin is Windows-only"
    import ctypes
    from ctypes import wintypes

    if not os.path.isabs(path) or odd_name(path) or len(path) > MAX_SHELL_PATH:
        return "Windows can't recycle this name safely (ends in a dot or space, or is too long); do it by hand"
    drive = os.path.splitdrive(path)[0] + "\\"
    if ctypes.windll.kernel32.GetDriveTypeW(drive) != 3:  # DRIVE_FIXED: removable/network drives have no bin
        return "drive %s has no Recycle Bin; delete it by hand if you are sure" % drive

    class SHFILEOPSTRUCTW(ctypes.Structure):
        _fields_ = [("hwnd", wintypes.HWND), ("wFunc", wintypes.UINT), ("pFrom", ctypes.c_void_p),
                    ("pTo", wintypes.LPCWSTR), ("fFlags", ctypes.c_ushort), ("fAnyOperationsAborted", wintypes.BOOL),
                    ("hNameMappings", ctypes.c_void_p), ("lpszProgressTitle", wintypes.LPCWSTR)]

    FO_DELETE, FOF_SILENT, FOF_NOCONFIRMATION, FOF_ALLOWUNDO, FOF_NOERRORUI, FOF_WANTNUKEWARNING = \
        3, 0x4, 0x10, 0x40, 0x400, 0x4000
    buf = ctypes.create_unicode_buffer(path + "\0")  # pFrom must end in two nulls
    op = SHFILEOPSTRUCTW(None, FO_DELETE, ctypes.cast(buf, ctypes.c_void_p), None,
                         FOF_SILENT | FOF_NOCONFIRMATION | FOF_ALLOWUNDO | FOF_NOERRORUI | FOF_WANTNUKEWARNING,
                         False, None, None)
    rc = ctypes.windll.shell32.SHFileOperationW(ctypes.byref(op))
    if rc != 0:
        return "Windows refused (code 0x%X)" % rc
    if op.fAnyOperationsAborted:
        return "cancelled"
    return None if not exists(path) else "still there after recycling"


# ---------------------------------------------------------------- guard rails
class Guard:
    def __init__(self, root=None):
        self.kf = sv.known_folders(root)
        env = os.environ
        prot = [env.get("SystemRoot", r"C:\Windows"), env.get("ProgramFiles", r"C:\Program Files"),
                env.get("ProgramFiles(x86)", r"C:\Program Files (x86)"), env.get("ProgramData", r"C:\ProgramData"),
                env.get("LOCALAPPDATA"), env.get("APPDATA"), os.path.join(self.kf["Profile"], "AppData")]
        if root:  # single-folder mode (as in survey.py --root): the chosen folder may sit under %TEMP%
            prot = [p for p in prot if p and not sv.under(sv.norm(root), [sv.norm(p)])]
        progs = [] if root else sv.installed_programs(
            [self.kf.get(k) for k in ("Profile", "Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music",
                                      "OneDrive")])
        games = []
        for lch in ([] if root else sv.launchers(sv.drive_list())):
            games += [g["path"] for g in lch["games"]]
            if lch["name"] == "Steam":
                games += [os.path.join(lib, "steamapps") for lib in lch["libraries"]]
            elif lch["name"] in ("Xbox app", "Riot", "EA app"):
                games += lch["libraries"]
        keep_out = [sv.norm(self.kf[k]) for k in ("Profile", "Desktop", "Documents", "Downloads") if self.kf.get(k)]
        self.protected = [sv.norm(p) for p in prot if p]
        self.programs = [sv.norm(p["path"]) for p in progs]
        self.games = [sv.norm(p) for p in games if os.path.dirname(sv.norm(p)) != sv.norm(p)
                      and not any(sv.under(k, [sv.norm(p)]) for k in keep_out)]
        mains = [v for v in self.kf.values() if v]
        self.untouchable = {sv.norm(v) for v in mains} | {sv.norm(real(v)) for v in mains}  # the folders themselves

    def _why_not(self, n, is_source):
        if os.path.dirname(n) == n:
            return "a drive root"
        if n in self.untouchable:
            return "one of your main folders"
        if re.search(r"[\\/](\$recycle\.bin|system volume information)([\\/]|$)", n):
            return "a Windows system folder"
        if sv.under(n, self.protected):
            return "inside Windows, Program Files, ProgramData or AppData"
        if sv.under(n, self.programs):
            return "inside an installed program's folder"
        if sv.under(n, self.games):
            return "inside a launcher's game library (use the launcher's own Move)"
        if is_source and any(sv.under(p, [n]) for p in self.programs + self.games + self.protected):
            return "contains an installed program or game"
        return None

    def why_not(self, path, is_source=True):
        """None if `path` may be touched, else the reason it may not. Checks the path as written and where it really is."""
        if not path or not re.match(r"^[A-Za-z]:\\|^\\\\[^?\\]", path):
            return "not an absolute path"
        if ".." in re.split(r"[\\/]", path):
            return "a path with '..' in it"
        return self._why_not(sv.norm(path), is_source) or self._why_not(sv.norm(real(path)), is_source)


def looks_installed(folder):
    try:
        names = [n.lower() for n in os.listdir(xp(folder))]
    except OSError:
        return False
    return any(n.startswith("unins") and n.endswith(".exe") for n in names) or "steamapps" in names


# ---------------------------------------------------------------- approvals
def parse_ids(text):
    ids = set()
    for part in re.split(r"[,\s]+", text.strip()):
        if not part:
            continue
        m = re.fullmatch(r"(\d+)(?:-(\d+))?", part)
        if not m:
            sys.exit("Can't read id '%s' (use e.g. 1-40,52)" % part)
        a, b = int(m.group(1)), int(m.group(2) or m.group(1))
        ids.update(range(min(a, b), max(a, b) + 1))
    return ids


def read_approval(args, plan):
    if args.approval:
        with open(args.approval, encoding="utf-8") as f:
            ap = json.load(f)
        pid, ids = ap.get("plan_id"), set(int(i) for i in ap.get("ids", []))
    elif args.approve:
        m = re.search(r"plan=([0-9a-f]+)", args.approve)
        mi = re.search(r"ids=([0-9,\- ]+|none)", args.approve)
        if not m or not mi:
            sys.exit("--approve needs the text copied from the page: TIDY-APPROVE plan=... ids=...")
        pid, ids = m.group(1), (set() if mi.group(1) == "none" else parse_ids(mi.group(1)))
    else:
        sys.exit("Say what was approved: --approve \"TIDY-APPROVE plan=... ids=...\" or --approval file.json")
    if pid != plan["plan_id"]:
        sys.exit("This approval is for plan %s but the plan file is plan %s. Rebuild the page or use the matching plan.json."
                 % (pid, plan["plan_id"]))
    known = {a["id"] for a in plan["actions"]}
    unknown = sorted(ids - known)
    if unknown:
        sys.exit("Approval names ids that aren't in this plan: %s" % unknown[:20])
    return ids


# ---------------------------------------------------------------- apply
class Log:
    def __init__(self, folder, dry):
        self.dry, self.rows = dry, []
        self.path = os.path.join(folder, "undo-log.csv") if folder else None
        if not dry:
            os.makedirs(folder, exist_ok=True)
            new = not os.path.exists(self.path)
            self.f = open(self.path, "a", newline="", encoding="utf-8-sig")
            self.w = csv.DictWriter(self.f, LOG_FIELDS, quoting=csv.QUOTE_ALL)  # every field quoted: names kept exact
            if new:
                self.w.writeheader()
            shutil.copyfile(REVERT_PS1, os.path.join(folder, "revert.ps1"))  # fixed script, no names in it

    def add(self, aid, op, src, dst, status, note=""):
        row = {"time": datetime.now().isoformat(timespec="seconds"), "id": aid, "op": op, "from": src,
               "to": dst or "", "status": status, "note": note}
        self.rows.append(row)
        if not self.dry:
            self.w.writerow(row)
            self.f.flush()
        print("%-8s %-17s #%-5s %s%s%s" % (status, op, aid, src, " -> " + dst if dst else "",
                                          "  (" + note + ")" if note else ""))


def run_apply(args):
    with open(args.plan, encoding="utf-8") as f:
        plan = json.load(f)
    if plan.get("schema") != "tidy-folders/plan@1":
        sys.exit("%s is not a tidy-folders plan" % args.plan)
    ids = read_approval(args, plan)
    actions = [a for a in plan["actions"] if a["id"] in ids]
    dry = not args.apply
    guard = Guard(os.path.abspath(args.root) if args.root else None)
    docs = guard.kf.get("Documents") or os.path.join(guard.kf["Profile"], "Documents")
    log_dir = os.path.abspath(args.log_dir) if args.log_dir else os.path.join(
        docs, "Tidy-up logs", datetime.now().strftime("%Y-%m-%d %H%M"))
    log = Log(log_dir, dry)
    print("%s %d approved actions from plan %s\n" % ("DRY RUN:" if dry else "APPLYING", len(actions), plan["plan_id"]))
    # Empty-folder removals last, so moves that empty a folder don't race them
    order = sorted(actions, key=lambda a: (a["op"] == "remove-empty-dir", a["id"]))
    for a in order:
        aid, op, src, dst = a["id"], a.get("op"), a.get("from"), a.get("to")
        if op not in OPS:
            log.add(aid, str(op), str(src), dst, "SKIP", "unknown operation")
            continue
        reason = guard.why_not(src)
        if reason:
            log.add(aid, op, str(src), dst, "REFUSED", reason)
            continue
        if not exists(src):
            log.add(aid, op, src, dst, "SKIP", "no longer there")
            continue
        if is_link(src):
            log.add(aid, op, src, dst, "REFUSED", "a link to another place; left alone")
            continue
        is_dir = os.path.isdir(xp(src))
        if is_dir and op != "remove-empty-dir" and looks_installed(src):
            log.add(aid, op, src, dst, "REFUSED", "looks like an installed program (has an uninstaller)")
            continue
        if op == "move":
            reason = guard.why_not(dst, is_source=False)
            if reason:
                log.add(aid, op, src, dst, "REFUSED", "destination is " + reason)
                continue
            if sv.under(sv.norm(dst), [sv.norm(src)]):
                log.add(aid, op, src, dst, "REFUSED", "can't move a folder into itself")
                continue
            final = free_name(dst)
            cross = not same_volume(src, final)
            if cross and is_dir and not args.allow_cross_drive_folders:
                log.add(aid, op, src, final, "SKIP", "folder move to another drive: rerun with --allow-cross-drive-folders")
                continue
            if cross and is_dir and tree_info(src)[3]:
                log.add(aid, op, src, final, "REFUSED", "holds a link to another folder; can't be copied safely")
                continue
            note = "renamed to avoid overwriting" if final != dst else ""
            if dry:
                log.add(aid, op, src, final, "WOULD", "; ".join(x for x in (
                    note, "copy to other drive, then recycle original" if cross else "") if x))
                continue
            parent = os.path.dirname(final)
            made, p = [], parent
            while not exists(p):
                made.append(p)
                p = os.path.dirname(p)
            try:
                os.makedirs(xp(parent), exist_ok=True)
                for m in reversed(made):
                    log.add(aid, "mkdir", m, "", "DONE", "created for this move")
                if not cross:
                    os.rename(xp(src), xp(final))
                else:
                    want = size_of(src)
                    if is_dir:
                        shutil.copytree(xp(src), xp(final), symlinks=True)
                    else:
                        shutil.copy2(xp(src), xp(final))
                    if size_of(final) != want:
                        log.add(aid, op, src, final, "FAILED", "copy didn't match; original kept, copy left at destination")
                        continue
                    err = recycle(src)
                    if err:
                        log.add(aid, op, src, final, "COPIED", "original kept: " + err)
                        continue
                log.add(aid, op, src, final, "DONE", note)
            except OSError as ex:
                log.add(aid, op, src, final, "FAILED", str(ex))
        elif op == "recycle":
            if not is_dir and a.get("bytes") and os.path.getsize(xp(src)) != a["bytes"]:
                log.add(aid, op, src, "", "SKIP", "changed since the survey")
                continue
            if is_dir and tree_info(src)[3]:
                log.add(aid, op, src, "", "REFUSED", "holds a link to another folder; delete it by hand")
                continue
            if odd_name(src) or len(src) > MAX_SHELL_PATH:
                log.add(aid, op, src, "", "SKIP", "name ends in a dot or space, or is too long, for the Recycle Bin")
                continue
            if dry:
                log.add(aid, op, src, "", "WOULD", "to the Recycle Bin")
                continue
            err = recycle(src)
            log.add(aid, op, src, "", "FAILED" if err else "DONE", err or "in the Recycle Bin")
        elif op == "remove-empty-dir":
            files, _, subdirs, has_link = tree_info(src) if is_dir else (1, 0, [], False)
            if not is_dir or files or has_link:
                log.add(aid, op, src, "", "SKIP", "not empty any more")
                continue
            if dry:
                log.add(aid, op, src, "", "WOULD", "remove (empty)")
                continue
            try:
                for d in subdirs:  # deepest first; rmdir refuses anything that isn't empty
                    os.rmdir(d)
                    log.add(aid, op, plain(d), "", "DONE", "empty")
                os.rmdir(xp(src))
                log.add(aid, op, src, "", "DONE", "empty")
            except OSError as ex:
                log.add(aid, op, src, "", "FAILED", str(ex))
    summarize(log.rows, dry)
    if not dry:
        log.f.close()
        print("\nUndo log folder:\n  %s" % log_dir)
        print("It holds undo-log.csv and revert.ps1. To revert, either:\n"
              "  - in that folder, right-click revert.ps1 > Run with PowerShell, or\n"
              "  - run apply_plan.py with --undo and that folder (add --apply after checking the dry run)")


def summarize(rows, dry):
    c = {}
    for r in rows:
        c[r["status"]] = c.get(r["status"], 0) + 1
    print("\n%s: %s" % ("Dry run" if dry else "Done", ", ".join("%d %s" % (v, k.lower()) for k, v in sorted(c.items()))))
    if dry:
        print("Nothing was changed. Add --apply to do it.")


# ---------------------------------------------------------------- undo
def read_log(path):
    with open(path, encoding="utf-8-sig", newline="") as f:
        return list(csv.DictReader(f))


def run_undo(args):
    path = os.path.join(args.undo, "undo-log.csv") if os.path.isdir(args.undo) else args.undo
    rows = read_log(path)
    dry = not args.apply
    print("%s revert of %s\n" % ("DRY RUN:" if dry else "REVERTING", path))
    for r in reversed(rows):
        if r["status"] != "DONE":
            continue
        op, src, dst = r["op"], r["from"], r["to"]
        if not re.match(r"^[A-Za-z]:\\|^\\\\", src):
            continue
        if op == "move":
            if not exists(dst):
                print("SKIP     %s is gone from %s" % (src, dst))
            elif exists(src):
                print("SKIP     %s exists again; leaving %s where it is" % (src, dst))
            elif dry:
                print("WOULD    move back %s -> %s" % (dst, src))
            else:
                os.makedirs(xp(os.path.dirname(src)), exist_ok=True)
                if same_volume(src, dst):
                    os.rename(xp(dst), xp(src))
                else:
                    shutil.move(xp(dst), xp(src))
                print("DONE     moved back %s" % src)
        elif op == "mkdir":
            if os.path.isdir(xp(src)) and not os.listdir(xp(src)):
                if not dry:
                    os.rmdir(xp(src))
                print("%s remove folder created by the tidy-up: %s" % ("WOULD   " if dry else "DONE    ", src))
        elif op == "remove-empty-dir":
            if not dry:
                os.makedirs(xp(src), exist_ok=True)
            print("%s recreate empty folder %s" % ("WOULD   " if dry else "DONE    ", src))
        elif op == "recycle":
            print("MANUAL   restore from the Recycle Bin: %s" % src)
    if dry:
        print("\nNothing was changed. Add --apply to revert.")


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Apply an approved tidy plan (dry run unless --apply), or undo one.")
    ap.add_argument("plan", nargs="?", help="plan.json written by build_page.py")
    ap.add_argument("--approve", help='the text copied from the page: "TIDY-APPROVE plan=... ids=..."')
    ap.add_argument("--approval", help="the approval .json downloaded from the page")
    ap.add_argument("--apply", action="store_true", help="really do it (default is a dry run)")
    ap.add_argument("--log-dir", help="where the undo log goes (default: Documents\\Tidy-up logs\\<date time>)")
    ap.add_argument("--allow-cross-drive-folders", action="store_true",
                    help="allow moving whole folders to another drive (copy, verify, recycle original)")
    ap.add_argument("--undo", help="a log folder (or its undo-log.csv) to revert")
    ap.add_argument("--root", help="the folder given to survey.py --root, if the survey used it")
    args = ap.parse_args(argv)
    if args.apply and not sv.IS_WIN:
        sys.exit("--apply is Windows-only")
    if args.undo:
        run_undo(args)
    elif args.plan:
        run_apply(args)
    else:
        ap.error("give a plan.json (with --approve/--approval) or --undo <log folder>")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Carry out the approved part of a plan, with an undo log. Dry run unless --apply.

This is the one file-moving engine the skills in this collection share. A skill that moves,
renames or removes files ships this file and revert.ps1 unchanged in its scripts/ folder
(skills/tests/check_shared.py fails if a copy has drifted) and writes its plan in the form
below. It imports nothing from the skill around it.

    python safe_apply.py plan.json --approve "TIDY-APPROVE plan=3f9a1c07be ids=1-40,52"
    python safe_apply.py plan.json --approval approval-3f9a1c07be.json --apply
    python safe_apply.py plan.json --approve "..." --within "C:\\Users\\me\\Desktop" --apply
    python safe_apply.py --undo <log folder>             # dry run of the revert
    python safe_apply.py --undo <log folder> --apply

The plan file:

    {"schema": "<skill-name>/plan@1", "plan_id": plan_fingerprint(actions), "actions": [
      {"id": 1, "op": "move", "from": "<full path>", "to": "<full path>"},
      {"id": 2, "op": "recycle", "from": "<full path>", "bytes": 1234},
      {"id": 3, "op": "remove-empty-dir", "from": "<full path>"}]}

Rules it enforces, whatever the plan says:
  * only ids from this plan, and only if the approval names this plan_id; the id is a
    hash of the plan's actions, so a plan edited after approval is refused
  * only full paths on a local disk: network shares, mapped network drives and device
    paths are refused (they can be another name for a protected folder)
  * the path that is checked is the path that is used: anything not written as the
    plain, real name ('/', '..', an 8.3 short name such as APPDAT~1) is refused
  * never touches Windows, Program Files, ProgramData, AppData, drive roots or the
    user's top-level folders; checked on the real location too, so 8.3 short names and
    junctions can't dodge it
  * --within <folder> (repeatable) limits a run: every source and every destination
    must be inside one of those folders, as written and where it really is
  * anything that can't be examined is refused, never treated as fine
  * what was checked can't be swapped before it is used: the folder a thing is in and the
    folder it goes to are held open so they can't be renamed or replaced, each is asked
    where it really is, and every check is run again before anything on disk changes
  * --undo checks every row of the log the same way before it moves anything, and holds it
    to the skill's own rules too
  * never acts on a symlink or junction, and never walks through one it couldn't follow
  * file names are data: they are never put into a command or a script. The revert
    script (revert.ps1) is a fixed file that reads undo-log.csv
  * every file is addressed by its exact name (extended-length paths), so a name
    ending in a dot or space can't be mistaken for its neighbour
  * moves never overwrite: an existing name gets " (2)"
  * "recycle" goes to the Recycle Bin (Windows asks before anything would be
    deleted for good); files are skipped if they changed since the survey; only what the
    plan names goes, never the folder Windows pairs with a saved web page
  * "remove-empty-dir" re-checks that nothing is inside, then uses rmdir only
  * a move to another drive copies, checks the size, then recycles the original
  * every step is written to undo-log.csv as it happens, in a folder made new for the run:
    a log that is already there is never added to

A skill can add rules, never take one away. Its own apply_plan.py calls

    args = safe_apply.parse_args(argv)
    safe_apply.run(args, places=..., check=...)

  places(root) -> [(folders, why_inside, why_holding), ...]
      more places nothing may be touched in; nothing that holds one may be moved
  check(action, guard) -> a reason to refuse this action, or None
      called after the engine's own checks have passed for that action, and again just
      before it is carried out. --undo asks it about every row as well, as the action it
      was ("op", "from" and "to" as logged) with "undo": True, which says the thing is at
      "to" now. A "mkdir" row is the engine's own: it is judged by the move it was made for

Python 3.8+, standard library only. Windows only for --apply.
"""
import argparse
import csv
import hashlib
import json
import os
import re
import shutil
import stat
import sys
import tempfile
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
IS_WIN = os.name == "nt"

OPS = {"move", "recycle", "remove-empty-dir"}
LOG_FIELDS = ["time", "id", "op", "from", "to", "status", "note"]
REVERT_PS1 = os.path.join(HERE, "revert.ps1")   # fixed script, copied as-is next to each undo log
LINK_TAGS = (0xA0000003, 0xA000000C)             # junction, symlink
MAX_SHELL_PATH = 259                             # the Recycle Bin API can't take longer paths
CANT_RESOLVE = "a path Windows can't resolve"
CANT_EXAMINE = "a path that can't be examined"
SWAPPED = "not where it was when it was checked: a folder on the way to it was swapped or turned into a link"


# ---------------------------------------------------------------- small helpers
def norm(p):
    return os.path.normcase(os.path.normpath(p))


def under(path_n, roots_n):
    """True if normalized path equals or sits below any normalized root."""
    for r in roots_n:
        if path_n == r or path_n.startswith(r.rstrip("\\/") + os.sep):
            return True
    return False


def plan_fingerprint(actions):
    """The plan id: a hash of what the plan would do. Edit an action and the id no longer matches."""
    core = [[a["id"], a["op"], a["from"], a.get("to")] for a in actions]
    return hashlib.sha256(json.dumps(core, ensure_ascii=False).encode("utf-8")).hexdigest()[:10]


def _shell_folders():
    """Where Windows says Desktop, Documents... are (they may be redirected, e.g. into OneDrive)."""
    out = {}
    if not IS_WIN:
        return out
    import winreg
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                            r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders") as k:
            i = 0
            while True:
                try:
                    n, v, _ = winreg.EnumValue(k, i)
                except OSError:
                    break
                out.setdefault(n, v)
                i += 1
    except OSError:
        pass
    return out


def known_folders(root=None):
    """The person's main folders. With `root` (test or single-folder mode), that folder is the profile."""
    if root:
        out = {k: os.path.join(root, k) if os.path.isdir(os.path.join(root, k)) else None
               for k in ("Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music")}
        out["Profile"] = root
        return out
    home = os.path.expanduser("~")
    usf = _shell_folders()
    keys = {"Desktop": "Desktop", "Documents": "Personal",
            "Downloads": "{374DE290-123F-4565-9164-39C4925E467B}", "Pictures": "My Pictures",
            "Videos": "My Video", "Music": "My Music"}
    out = {}
    for label, reg in keys.items():
        v = usf.get(reg)
        p = os.path.normpath(os.path.expandvars(v)) if isinstance(v, str) and v.strip() else os.path.join(home, label)
        out[label] = p if os.path.isdir(p) else None
    out["Profile"] = home
    for env in ("OneDrive", "OneDriveConsumer", "OneDriveCommercial"):
        if os.environ.get(env) and os.path.isdir(os.environ[env]):
            out["OneDrive"] = os.environ[env]
            break
    return out


# ---------------------------------------------------------------- exact paths
def xp(path):
    """Extended-length form of an absolute Windows path: the exact name, no trimming of dots or spaces."""
    if not IS_WIN or path.startswith("\\\\?\\"):
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


def state(path):
    """'there', 'absent' or 'unknown'. Unknown means it couldn't be examined: never the same as absent."""
    try:
        os.lstat(xp(path))
        return "there"
    except FileNotFoundError:
        return "absent"
    except (OSError, ValueError):
        return "unknown"


def exists(path):
    """False only if nothing has that name. A name that can't be examined counts as taken."""
    return state(path) != "absent"


def _is_link_stat(st):
    if getattr(st, "st_file_attributes", 0) & 0x400 and getattr(st, "st_reparse_tag", 0) in LINK_TAGS:
        return True
    return stat.S_ISLNK(st.st_mode)


def is_link(path):
    """A symlink or junction (OneDrive cloud placeholders are reparse points too, but not links).
    Something that can't be examined counts as a link: it is left alone."""
    try:
        st = os.lstat(xp(path))
    except FileNotFoundError:
        return False
    except (OSError, ValueError):
        return True
    return _is_link_stat(st)


def odd_name(path):
    """True if a path component ends in a dot or space: the Windows shell can't address those."""
    return any(part != part.rstrip(". ") for part in re.split(r"[\\/]", path)[1:])


def real(path):
    """Where the path really is: the parent resolved through junctions, read by its exact name
    (the same way every file operation here addresses it, so both see the same folder).
    None if Windows can't say."""
    parent, name = os.path.split(path.rstrip("\\/"))
    try:
        parent = plain(os.path.realpath(xp(parent)))
    except (OSError, ValueError):
        return None
    return os.path.join(parent, name)


def real_folder(path):
    """Where a folder really is, the folder itself resolved too. None if Windows can't say."""
    try:
        return plain(os.path.realpath(xp(path)))
    except (OSError, ValueError):
        return None


def unfollowed(folder):
    """None if `folder` (a resolved path) and every folder on the way to it is a plain folder, or
    not there yet; else why not. os.path.realpath gives up quietly on some errors and leaves the
    rest of a path unresolved, so a link it did not follow can still be in its answer. A junction
    it did follow is no longer part of the path, and is fine."""
    if not re.match(r"^[A-Za-z]:\\", folder):
        return CANT_EXAMINE
    cur = folder[:3]
    for part in folder[3:].split("\\"):
        if not part:
            continue
        cur = os.path.join(cur, part)
        try:
            st = os.lstat(xp(cur))
        except FileNotFoundError:
            return None  # nothing by that name: it, and the rest, would be new
        except (OSError, ValueError):
            return CANT_EXAMINE
        if _is_link_stat(st):
            return "reached through a link that could not be followed safely"
    return None


def long_name(path):
    """`path` with every part that exists spelled by its real name (8.3 short names expanded), or
    None if Windows can't say. Links are not followed; parts that don't exist yet stay as written."""
    if not IS_WIN:
        return path
    import ctypes
    tail, cur = [], path
    while True:
        s = state(cur)
        if s == "there":
            break
        if s == "unknown":
            return None
        parent, name = os.path.split(cur)
        if not name or parent == cur:
            return None
        tail.insert(0, name)
        cur = parent
    if tail:
        # `cur` is where the new parts would be made. It has to be a folder that can be listed:
        # in one that can't, nothing put there could be examined, or brought back, afterwards.
        try:
            with os.scandir(xp(cur)):
                pass
        except OSError:
            return None
    buf = ctypes.create_unicode_buffer(32768)
    n = ctypes.windll.kernel32.GetLongPathNameW(xp(cur), buf, len(buf))
    if not n or n >= len(buf):
        return None
    return os.path.join(plain(buf.value), *tail)


def on_local_drive(path):
    """True for X:\\... on a fixed or removable drive. Network shares (\\\\server\\share, also as a mapped
    letter) and device paths can be another name for a protected folder, so they are never touched."""
    if not re.match(r"^[A-Za-z]:\\", path or ""):
        return False
    if not IS_WIN:
        return True
    import ctypes
    return ctypes.windll.kernel32.GetDriveTypeW(path[:3]) in (2, 3)  # DRIVE_REMOVABLE, DRIVE_FIXED


def not_plain(path):
    """None if `path` is written as the plain, real, full name of a place on a drive letter, else
    why not. The path that is checked must be the path that is used. Files are opened by their
    exact name, so anything Windows would quietly rewrite or read as another name is refused here:
    "/", "..", ".", doubled or trailing backslashes, and 8.3 short names (APPDAT~1 is AppData)."""
    if not isinstance(path, str) or not re.match(r"^[A-Za-z]:\\", path):
        return "not a full path on a local drive (network and device paths are refused)"
    if "/" in path or os.path.normpath(path) != path:
        return "not written as a plain path ('/', '..', '.', or a doubled or trailing backslash)"
    if ":" in path[2:] or re.search(r'[<>"|?*\x00-\x1f]', path[2:]):
        return "a path with characters Windows doesn't allow in names"
    actual = long_name(path)
    if actual is None:
        return CANT_RESOLVE
    if norm(actual) != norm(path):
        return "not the real name of that file or folder (a short 8.3 name or another alias)"
    # A folder whose name ends in a dot or space is one other programs read as a different
    # folder ("sub." as "sub"): never go through one, and never create one.
    folders = path[3:].split("\\")[:-1]
    if any(part != part.rstrip(". ") for part in folders):
        return "inside a folder whose name ends in a dot or space"
    return None


def limits(folders, flag="--within"):
    """[(as written, where it really is)] for the folders a run is limited to, ready for inside().
    Exits unless each is an existing folder written by its plain, real, full name: a limit must
    not be vaguer than the paths it limits."""
    out = []
    for f in folders or ():
        reason = not_plain(f)
        if not reason and not os.path.isdir(xp(f)):
            reason = "not an existing folder"
        there = None if reason else real_folder(f)
        if not reason:
            reason = CANT_RESOLVE if there is None else unfollowed(there)
        if reason:
            sys.exit("%s %s: %s" % (flag, f, reason))
        out.append((norm(f), norm(there)))
    return out


def inside(path, lims):
    """True if `path` is strictly inside one of the folders from limits(), as written and where
    it really is (so a junction inside a folder can't lead out of it)."""
    there = real(path) if isinstance(path, str) else None
    if there is None:
        return False
    n, t = norm(path), norm(there)
    return any(n != w and under(n, [w]) and t != r and under(t, [r]) for w, r in lims)


def tree_info(path):
    """(files, bytes, folders deepest-first, has_link) for a folder, never walking through a link.
    A folder or entry that can't be examined counts as a file and as a link: not empty, not safe to copy."""
    files = total = 0
    dirs, has_link, stack = [], False, [xp(path)]
    while stack:
        d = stack.pop()
        try:
            with os.scandir(d) as it:
                entries = list(it)
        except OSError:
            return files + 1, total, dirs, True  # unreadable: treat as not empty, and not safe to copy
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
    """`path` if nothing has that name, else the first free "name (2)", "name (3)"...
    None if that can't be worked out."""
    s = state(path)
    if s == "absent":
        return path
    if s == "unknown":
        return None
    base, ext = (path, "") if os.path.isdir(xp(path)) else os.path.splitext(path)
    for i in range(2, 100000):
        cand = "%s (%d)%s" % (base, i, ext)
        s = state(cand)
        if s == "absent":
            return cand
        if s == "unknown":
            return None
    return None


def same_volume(a, b):
    return os.path.splitdrive(a)[0].lower() == os.path.splitdrive(b)[0].lower()


# ---------------------------------------------------------------- Recycle Bin
# FOF_SILENT, FOF_NOCONFIRMATION, FOF_ALLOWUNDO (the Recycle Bin, not a delete), FOF_NOERRORUI,
# FOF_WANTNUKEWARNING (ask before anything would be deleted for good) and
# FOF_NO_CONNECTED_ELEMENTS: without it Windows takes "page_files" along with "page.htm" (and the
# other way round), a folder or file that was never in the plan, the approval or the undo log.
RECYCLE_FLAGS = 0x4 | 0x10 | 0x40 | 0x400 | 0x4000 | 0x2000


def shell_delete(path, flags):
    """SHFileOperationW's delete on one path. Returns (Windows' code, True if it was cancelled)."""
    import ctypes
    from ctypes import wintypes

    class SHFILEOPSTRUCTW(ctypes.Structure):
        _fields_ = [("hwnd", wintypes.HWND), ("wFunc", wintypes.UINT), ("pFrom", ctypes.c_void_p),
                    ("pTo", wintypes.LPCWSTR), ("fFlags", ctypes.c_ushort), ("fAnyOperationsAborted", wintypes.BOOL),
                    ("hNameMappings", ctypes.c_void_p), ("lpszProgressTitle", wintypes.LPCWSTR)]

    buf = ctypes.create_unicode_buffer(path + "\0")  # pFrom must end in two nulls
    op = SHFILEOPSTRUCTW(None, 3, ctypes.cast(buf, ctypes.c_void_p), None, flags, False, None, None)  # 3: FO_DELETE
    rc = ctypes.windll.shell32.SHFileOperationW(ctypes.byref(op))
    return rc, bool(op.fAnyOperationsAborted)


def recycle(path):
    """Send a file or folder to the Recycle Bin. Returns None on success, else an error string."""
    if not IS_WIN:
        return "the Recycle Bin is Windows-only"
    import ctypes

    if not os.path.isabs(path) or odd_name(path) or len(path) > MAX_SHELL_PATH:
        return "Windows can't recycle this name safely (ends in a dot or space, or is too long); do it by hand"
    drive = os.path.splitdrive(path)[0] + "\\"
    if ctypes.windll.kernel32.GetDriveTypeW(drive) != 3:  # DRIVE_FIXED: removable/network drives have no bin
        return "drive %s has no Recycle Bin; delete it by hand if you are sure" % drive
    rc, cancelled = shell_delete(path, RECYCLE_FLAGS)
    if rc != 0:
        return "Windows refused (code 0x%X)" % rc
    if cancelled:
        return "cancelled"
    return None if not exists(path) else "still there after recycling"


# ---------------------------------------------------------------- pins
# A check by name followed by a move by name leaves a gap: in between, a folder on the way could
# be swapped for a link to somewhere else. So before anything on disk changes, the folder the
# thing is in and the folder it is going to are opened and kept open ("pinned") in a way that
# lets them be used but not renamed or deleted; Windows then refuses to rename any folder above
# them as well. Each one is asked where it really is, every check is run again, and what passes
# then is what is used. The thing being moved is not held open itself: it couldn't be moved.
_K32 = []


def _kernel32():
    """kernel32 with the argument types spelled out (a handle is wider than ctypes' default int)."""
    if not _K32:
        import ctypes
        from ctypes import wintypes
        k = ctypes.WinDLL("kernel32", use_last_error=True)
        k.CreateFileW.restype = wintypes.HANDLE
        k.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                                  wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        k.GetFinalPathNameByHandleW.restype = wintypes.DWORD
        k.GetFinalPathNameByHandleW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR, wintypes.DWORD, wintypes.DWORD]
        k.GetFileInformationByHandle.restype = wintypes.BOOL
        k.GetFileInformationByHandle.argtypes = [wintypes.HANDLE, ctypes.c_void_p]
        k.GetFileInformationByHandleEx.restype = wintypes.BOOL
        k.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        k.CloseHandle.restype = wintypes.BOOL
        k.CloseHandle.argtypes = [wintypes.HANDLE]
        _K32.append(k)
    return _K32[0]


def open_folder(path):
    """A handle on the folder itself (never on what a link by that name leads to), open for listing
    and shared for reading and writing but not for deleting: until it is closed, the folder can't
    be renamed, deleted or replaced. None if it can't be opened that way."""
    import ctypes
    # FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES (Windows applies the sharing rules only to a handle
    # that asks to read, write or delete: one opened for attributes alone would stop nothing);
    # FILE_SHARE_READ | FILE_SHARE_WRITE; OPEN_EXISTING;
    # FILE_FLAG_BACKUP_SEMANTICS (it is a folder) | FILE_FLAG_OPEN_REPARSE_POINT (not through a link)
    h = _kernel32().CreateFileW(xp(path), 0x1 | 0x80, 0x1 | 0x2, None, 3, 0x02000000 | 0x00200000, None)
    return None if h is None or h == ctypes.c_void_p(-1).value else h


def close_folder(handle):
    _kernel32().CloseHandle(handle)


def final_name(handle, how):
    """GetFinalPathNameByHandleW's answer for an open folder, or None. `how`: 0 for "X:\\...",
    4 for the path from the top of its volume ("\\...")."""
    import ctypes
    buf = ctypes.create_unicode_buffer(32768)
    n = _kernel32().GetFinalPathNameByHandleW(handle, buf, len(buf), how)
    return plain(buf.value) if 0 < n < len(buf) else None


def final_path(handle):
    """Where an open folder really is, as Windows spells it: "X:\\...". On a volume Windows has no
    drive name for (some RAM disks; os.path.realpath leaves paths there as written too), the path
    from the top of the volume: "\\...". None if Windows won't say."""
    return final_name(handle, 0) or final_name(handle, 4)


def same_place(where, folder):
    """True if `where`, an answer from final_path(), is the folder `folder` ("X:\\...")."""
    return norm(where) == norm(folder if re.match(r"^[A-Za-z]:\\", where) else folder[2:])


def handle_kind(handle):
    """"folder" for an open plain folder, "link" for a junction or symlink, "other" for anything
    else. None if Windows won't say."""
    import ctypes
    k = _kernel32()
    info = (ctypes.c_uint32 * 13)()  # BY_HANDLE_FILE_INFORMATION: the attributes come first
    if not k.GetFileInformationByHandle(handle, info):
        return None
    if info[0] & 0x400:  # a reparse point: its tag says whether it is a link (a cloud folder is not)
        tag = (ctypes.c_uint32 * 2)()  # FILE_ATTRIBUTE_TAG_INFO: the attributes, then the reparse tag
        if not k.GetFileInformationByHandleEx(handle, 9, tag, 8):  # 9: FileAttributeTagInfo
            return None
        if tag[1] in LINK_TAGS:
            return "link"
    return "folder" if info[0] & 0x10 else "other"


class Pins:
    """The folders held open while something in them is changed. `with Pins() as pins:` lets go of
    them all at the end, whatever happens."""

    def __init__(self):
        self.handles, self.places = [], []

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def close(self):
        while self.handles:
            close_folder(self.handles.pop()[0])

    def hold(self, written, there):
        """Pin the folder that `written` is in. `there` is where the guard found that path to be
        (real(written), worked out when it was checked). None, or why it can't be relied on."""
        if not IS_WIN:
            return None
        if there is None:
            return CANT_RESOLVE
        self.places.append((written, there))
        folder = os.path.dirname(there)
        h = open_folder(folder)
        if h is None:
            return CANT_EXAMINE
        self.handles.append((h, folder))
        return self.moved()

    def moved(self):
        """None if every pinned folder is still a plain folder in the place that was approved, and
        every path still leads there; else why not. A folder that won't say counts as moved."""
        for h, folder in self.handles:
            where, kind = final_path(h), handle_kind(h)
            if where is None or kind is None:
                return CANT_EXAMINE
            if kind != "folder" or not same_place(where, folder):
                return SWAPPED
        for written, there in self.places:
            now = real(written)
            if now is None or norm(now) != norm(there):
                return SWAPPED
        return None


# ---------------------------------------------------------------- guard rails
class Guard:
    """Says why a path may not be touched. `within` and `places` only ever add reasons."""

    def __init__(self, root=None, within=(), places=()):
        self.kf = known_folders(root)
        env = os.environ
        prot = [env.get("SystemRoot", r"C:\Windows"), env.get("ProgramFiles", r"C:\Program Files"),
                env.get("ProgramFiles(x86)", r"C:\Program Files (x86)"), env.get("ProgramData", r"C:\ProgramData"),
                env.get("LOCALAPPDATA"), env.get("APPDATA"), os.path.join(os.path.expanduser("~"), "AppData")]
        self.protected = [norm(p) for p in prot if p]
        # The surveyed profile's own AppData (the same folder, unless --root): protected with no exemption
        self.profile_appdata = [norm(os.path.join(self.kf["Profile"], "AppData"))]
        # Single-folder mode (--root). The chosen folder may sit under %TEMP% (a test folder);
        # then that folder alone is exempt, never the protected place around it.
        self.roots = []
        if root:
            there = real(root)
            if there is None:
                sys.exit("--root %s: %s" % (root, CANT_RESOLVE))
            self.roots = sorted({norm(root), norm(there)})
            temp_dir = tempfile.gettempdir()
            temp = [norm(temp_dir), norm(real(temp_dir) or temp_dir)]
            for r in self.roots:
                if (os.path.dirname(r) == r or any(under(p, [r]) for p in self.protected)
                        or (under(r, self.protected) and not under(r, temp))):
                    sys.exit("--root can't be a drive, a protected folder, a folder that holds one, "
                             "or a folder inside one (other than a test folder under Temp)")
        # What the calling skill adds: [(folders, why a path inside one is refused, why a source holding one is)]
        self.places = [([norm(p) for p in folders if p], why_inside, why_holding)
                       for folders, why_inside, why_holding in (places or ())]
        mains = [v for v in self.kf.values() if v]
        # The folders themselves, as written and wherever they really are (one may be a junction to another drive)
        self.untouchable = ({norm(v) for v in mains} | {norm(real(v) or v) for v in mains}
                            | {norm(real_folder(v) or v) for v in mains})
        self.within = limits(within)

    def _why_not(self, n, is_source):
        if os.path.dirname(n) == n:
            return "a drive root"
        if n in self.untouchable:
            return "one of your main folders"
        if re.search(r"[\\/](\$recycle\.bin|system volume information)([\\/]|$)", n):
            return "a Windows system folder"
        if under(n, self.profile_appdata) or (under(n, self.protected) and not under(n, self.roots)):
            return "inside Windows, Program Files, ProgramData or AppData"
        for folders, why_inside, _ in self.places:
            if under(n, folders):
                return why_inside
        if is_source:
            for folders, _, why_holding in self.places:
                if any(under(p, [n]) for p in folders):
                    return why_holding
            if any(under(p, [n]) for p in self.protected + self.profile_appdata):
                return "contains Windows, Program Files, ProgramData or AppData"
        return None

    def why_not(self, path, is_source=True):
        """None if `path` may be touched, else the reason it may not. Checks the path as written and where it really is."""
        reason = not_plain(path)
        if reason:
            return reason
        there = real(path)
        if there is None:
            return CANT_RESOLVE
        if not on_local_drive(path) or not on_local_drive(there):
            return "on a network drive or share, not a local disk"
        reason = (unfollowed(os.path.dirname(there)) or self._why_not(norm(path), is_source)
                  or self._why_not(norm(there), is_source))
        if reason:
            return reason
        if self.within and not inside(path, self.within):
            return "outside the folders this run is limited to (--within)"
        return None


def looks_installed(folder):
    """Has an uninstaller or a Steam library in it. A folder that can't be listed counts: it is left alone."""
    try:
        names = [n.lower() for n in os.listdir(xp(folder))]
    except OSError:
        return True
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
    """The approved ids. The approval is the line the page copies, "<NAME>-APPROVE plan=<id> ids=<ranges>"
    (TIDY-APPROVE, DESKTOP-APPROVE...), or the approval file it downloads. Either must name this plan's id."""
    if args.approval:
        with open(args.approval, encoding="utf-8") as f:
            ap = json.load(f)
        pid, ids = ap.get("plan_id"), set(int(i) for i in ap.get("ids", []))
    elif args.approve:
        m = re.search(r"plan=([0-9a-f]+)", args.approve)
        mi = re.search(r"ids=([0-9,\- ]+|none)", args.approve)
        if not m or not mi:
            sys.exit("--approve needs the text copied from the page: <NAME>-APPROVE plan=... ids=...")
        pid, ids = m.group(1), (set() if mi.group(1) == "none" else parse_ids(mi.group(1)))
    else:
        sys.exit("Say what was approved: --approve \"<NAME>-APPROVE plan=... ids=...\" or --approval file.json")
    if pid != plan["plan_id"]:
        sys.exit("This approval is for plan %s but the plan file is plan %s. Rebuild the page or use the matching plan file."
                 % (pid, plan["plan_id"]))
    known = {a["id"] for a in plan["actions"]}
    unknown = sorted(ids - known)
    if unknown:
        sys.exit("Approval names ids that aren't in this plan: %s" % unknown[:20])
    return ids


# ---------------------------------------------------------------- apply
def new_folder(path):
    """Make `path` as a new folder and return its name: `path`, or "path (2)", "path (3)"... if
    that name is taken. Never a folder that was already there: what is in it isn't this run's."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    for i in range(1, 10000):
        cand = path if i == 1 else "%s (%d)" % (path, i)
        try:
            os.mkdir(cand)  # fails if anything has that name, and nothing can slip in between
            return cand
        except FileExistsError:
            pass
    sys.exit("Can't make a new folder for the undo log at %s" % path)


class Log:
    def __init__(self, folder, dry):
        self.dry, self.rows = dry, []
        self.folder = folder
        self.path = os.path.join(folder, "undo-log.csv") if folder else None
        if not dry:
            # A log folder is always new, and so are the two files in it ("x" refuses a name that
            # is there): rows somebody left in a folder of that name are never added to or replayed
            self.folder = new_folder(folder)
            self.path = os.path.join(self.folder, "undo-log.csv")
            self.f = open(self.path, "x", newline="", encoding="utf-8-sig")
            self.w = csv.DictWriter(self.f, LOG_FIELDS, quoting=csv.QUOTE_ALL)  # every field quoted: names kept exact
            self.w.writeheader()
            with open(REVERT_PS1, "rb") as script, open(os.path.join(self.folder, "revert.ps1"), "xb") as copy:
                copy.write(script.read())  # fixed script, no names in it

    def add(self, aid, op, src, dst, status, note=""):
        src, dst = str(src), ("" if dst is None else str(dst))  # a plan can hold anything where a path should be
        row = {"time": datetime.now().isoformat(timespec="seconds"), "id": aid, "op": op, "from": src,
               "to": dst, "status": status, "note": note}
        self.rows.append(row)
        if not self.dry:
            self.w.writerow(row)
            self.f.flush()
        print("%-8s %-17s #%-5s %s%s%s" % (status, op, aid, src, " -> " + dst if dst else "",
                                          "  (" + note + ")" if note else ""))


def refusal(a, guard, check, at=None):
    """(status, note) if action `a` may not be carried out as things stand now, else None.
    `at`, if given, is filled in with where the guard found "from" and "to" to be as it passed
    them: the places that are pinned, and compared again, before anything is done."""
    op, src, dst = a.get("op"), a.get("from"), a.get("to")
    reason = guard.why_not(src)
    if reason:
        return "REFUSED", reason
    if at is not None:
        at["from"] = real(src)
    there = state(src)
    if there == "absent":
        return "SKIP", "no longer there"
    if there == "unknown":
        return "REFUSED", "can't be examined; left alone"
    if is_link(src):
        return "REFUSED", "a link to another place; left alone"
    if os.path.isdir(xp(src)) and op != "remove-empty-dir" and looks_installed(src):
        return "REFUSED", "looks like an installed program (has an uninstaller), or can't be looked into"
    if op == "move":
        reason = guard.why_not(dst, is_source=False)
        if reason:
            return "REFUSED", "destination is " + reason
        if at is not None:
            at["to"] = real(dst)
        if under(norm(dst), [norm(src)]):
            return "REFUSED", "can't move a folder into itself"
    reason = check(a, guard) if check else None
    return ("REFUSED", reason) if reason else None


def run_apply(args, places=None, check=None):
    with open(args.plan, encoding="utf-8") as f:
        plan = json.load(f)
    if not isinstance(plan, dict) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*/plan@1", str(plan.get("schema"))):
        sys.exit("%s is not a plan this script can carry out (its schema should be <skill-name>/plan@1)" % args.plan)
    # The approval names the plan by its id, and the id is a hash of the actions: a plan
    # changed after the page was built (and approved) no longer matches its own id.
    try:
        intact = plan_fingerprint(plan["actions"]) == plan["plan_id"]
    except (KeyError, TypeError, ValueError):
        intact = False
    if not intact:
        sys.exit("%s was changed after its page was built, so the approval doesn't cover it. "
                 "Rebuild the page and ask for a new approval." % args.plan)
    ids = read_approval(args, plan)
    actions = [a for a in plan["actions"] if a["id"] in ids]
    dry = not args.apply
    root = os.path.abspath(args.root) if args.root else None
    guard = Guard(root, args.within, places(root) if places else ())
    docs = guard.kf.get("Documents") or os.path.join(guard.kf["Profile"], "Documents")
    log_dir = os.path.abspath(args.log_dir) if args.log_dir else os.path.join(
        docs, "Tidy-up logs", datetime.now().strftime("%Y-%m-%d %H%M"))
    log = Log(log_dir, dry)
    print("%s %d approved actions from plan %s\n" % ("DRY RUN:" if dry else "APPLYING", len(actions), plan["plan_id"]))

    def pinned(pins, a, *places):
        """Pin the folder each of `places` (a path, and where the guard found it) is in, then check
        action `a` again. None if it can go ahead: until the pins are let go, what was just checked
        can't be swapped for something else. Otherwise (status, note)."""
        for written, there in places:
            why = pins.hold(written, there)
            if why:
                return "REFUSED", why
        no = refusal(a, guard, check)
        if no:
            return no
        why = pins.moved()
        return ("REFUSED", why) if why else None

    def finish(pins, aid, op, src, dst, note):
        """Log what was just done, asking the pinned folders once more. Holding a folder open stops
        it being renamed or deleted, but an emptied one can still be turned into a link where it
        stands; if that happened in the instant it was used, there is no telling where the thing
        went. True if so: the run stops there."""
        late = pins.moved()
        log.add(aid, op, src, dst, "FAILED" if late else "DONE", "%s; look for it by hand" % late if late else note)
        return bool(late)

    halted = False
    # Empty-folder removals last, so moves that empty a folder don't race them
    order = sorted(actions, key=lambda a: (a["op"] == "remove-empty-dir", a["id"]))
    for a in order:
        aid, op, src, dst = a["id"], a.get("op"), a.get("from"), a.get("to")
        if op not in OPS:
            log.add(aid, str(op), str(src), dst, "SKIP", "unknown operation")
            continue
        at = {}
        no = refusal(a, guard, check, at)
        if no:
            log.add(aid, op, src, dst, *no)
            continue
        is_dir = os.path.isdir(xp(src))
        src_at = at["from"]  # where the guard found it: the folder that is in gets pinned, and this is the path used
        if op == "move":
            final = free_name(dst)
            if final is None:
                log.add(aid, op, src, dst, "REFUSED", "destination is " + CANT_EXAMINE)
                continue
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
            # the same folder the guard found for the destination, under the name that is free there
            final_at = os.path.join(os.path.dirname(at["to"]), os.path.basename(final)) if at["to"] else None
            parent = os.path.dirname(final)
            made, p = [], parent
            while not exists(p):
                made.append(p)
                p = os.path.dirname(p)
            try:
                os.makedirs(xp(parent), exist_ok=True)
                for m in reversed(made):
                    log.add(aid, "mkdir", m, "", "DONE", "created for this move")
                with Pins() as pins:
                    no = pinned(pins, a, (src, src_at), (final, final_at))
                    if no:
                        log.add(aid, op, src, final, *no)
                        continue
                    if not cross:
                        os.rename(xp(src_at), xp(final_at))
                    else:
                        want = size_of(src_at)
                        if is_dir:
                            shutil.copytree(xp(src_at), xp(final_at), symlinks=True)
                        else:
                            shutil.copy2(xp(src_at), xp(final_at))
                        if size_of(final_at) != want:
                            log.add(aid, op, src, final, "FAILED", "copy didn't match; original kept, copy left at destination")
                            continue
                        err = recycle(src_at)
                        if err:
                            log.add(aid, op, src, final, "COPIED", "original kept: " + err)
                            continue
                    halted = finish(pins, aid, op, src, final, note)
            except OSError as ex:
                log.add(aid, op, src, final, "FAILED", str(ex))
        elif op == "recycle":
            if not is_dir and a.get("bytes") and os.path.getsize(xp(src)) != a["bytes"]:
                log.add(aid, op, src, "", "SKIP", "changed since the survey")
                continue
            if is_dir and tree_info(src)[3]:
                log.add(aid, op, src, "", "REFUSED", "holds a link to another folder, or something that can't be "
                                                     "examined; delete it by hand")
                continue
            if odd_name(src) or len(src) > MAX_SHELL_PATH:
                log.add(aid, op, src, "", "SKIP", "name ends in a dot or space, or is too long, for the Recycle Bin")
                continue
            if dry:
                log.add(aid, op, src, "", "WOULD", "to the Recycle Bin")
                continue
            with Pins() as pins:
                no = pinned(pins, a, (src, src_at))
                if no:
                    log.add(aid, op, src, "", *no)
                    continue
                err = recycle(src_at)
                if err:
                    log.add(aid, op, src, "", "FAILED", err)
                else:
                    halted = finish(pins, aid, op, src, "", "in the Recycle Bin")
        elif op == "remove-empty-dir":
            files, _, subdirs, has_link = tree_info(src) if is_dir else (1, 0, [], False)
            if not is_dir or files or has_link:
                log.add(aid, op, src, "", "SKIP", "not empty any more")
                continue
            if dry:
                log.add(aid, op, src, "", "WOULD", "remove (empty)")
                continue
            try:
                with Pins() as pins:
                    no = pinned(pins, a, (src, src_at))
                    if not no:  # looked into again, now that it can't be swapped for another folder
                        files, _, subdirs, has_link = tree_info(src_at)
                        no = ("SKIP", "not empty any more") if files or has_link else None
                    if no:
                        log.add(aid, op, src, "", *no)
                        continue
                    for d in subdirs:  # deepest first; rmdir refuses anything that isn't empty
                        os.rmdir(d)
                        log.add(aid, op, src + plain(d)[len(src_at):], "", "DONE", "empty")
                    os.rmdir(xp(src_at))
                    halted = finish(pins, aid, op, src, "", "empty")
            except OSError as ex:
                log.add(aid, op, src, "", "FAILED", str(ex))
        if halted:
            break
    summarize(log.rows, dry)
    if not dry:
        log.f.close()
        print("\nUndo log folder:\n  %s" % log.folder)
        if log.folder != log_dir:
            print("  (%s was already there. A run always makes a new folder: it never adds to an older log.)" % log_dir)
        print("It holds undo-log.csv and a copy of revert.ps1. To revert, either:\n"
              "  - run this skill's own scripts\\revert.ps1 with -Log and that folder (add -Preview to look first), or\n"
              "  - run %s with --undo and that folder (add --apply after checking the dry run)"
              % os.path.basename(sys.argv[0] or "safe_apply.py"))
    if halted:
        sys.exit("Stopped: a folder was turned into a link while something in it was being moved. Nothing more was done.")


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


def run_undo(args, places=None, check=None):
    path = os.path.join(args.undo, "undo-log.csv") if os.path.isdir(args.undo) else args.undo
    rows = read_log(path)
    dry = not args.apply
    print("%s revert of %s\n" % ("DRY RUN:" if dry else "REVERTING", path))
    # The log is a file anyone could have edited: every row passes the same guard rails as a plan,
    # and the skill's own rules too. A row the skill's apply would never have written is refused.
    root = os.path.abspath(args.root) if args.root else None
    guard = Guard(root, args.within, places(root) if places else ())

    def why_not(r, at=None):
        """Why the row `r` may not be undone, or None. `at`, if given, is filled in with where the
        guard found "from" and "to" to be as it passed them (as in refusal(), for the apply)."""
        op, src, dst = r.get("op"), r.get("from") or "", r.get("to") or ""
        reason = guard.why_not(src, is_source=False)
        if not reason and at is not None:
            at["from"] = real(src)
        reason = reason or ("a link to another place" if is_link(src) else None)
        if not reason and op == "move":
            reason = guard.why_not(dst)
            if not reason and at is not None:
                at["to"] = real(dst)
            reason = reason or ("a link to another place" if is_link(dst) else None)
        if reason:
            return reason
        if op == "mkdir":
            # Written by the engine itself, under a move's id, for a folder that move needed: the
            # move went into it, from outside it (what it moved was there before the folder was).
            # A "mkdir" row with no such move in this log, or one the checks refuse, isn't one.
            for m in rows:
                to, was = norm(m.get("to") or ""), norm(m.get("from") or "")
                if (m.get("op") == "move" and m.get("id") == r.get("id") and to != norm(src) and under(to, [norm(src)])
                        and not under(was, [norm(src)]) and why_not(m) is None):
                    return None
            return "not a folder that was made for a move in this log"
        # The skill's own check, asked about the action as the apply saw it: this went from `from`
        # to `to`. "undo" tells it that the thing is at `to` now.
        return check({"id": r.get("id"), "op": op, "from": src, "to": dst or None, "undo": True}, guard) if check else None

    def pinned(pins, r, *places):
        """Pin the folder each of `places` is in, then check the row again. None, or why not."""
        for written, there in places:
            why = pins.hold(written, there)
            if why:
                return why
        return why_not(r) or pins.moved()

    failed = 0
    for r in reversed(rows):
        if r.get("status") != "DONE":
            continue
        op, src, dst = r.get("op"), r.get("from") or "", r.get("to") or ""
        if op not in ("move", "mkdir", "remove-empty-dir", "recycle"):
            continue
        at = {}
        reason = why_not(r, at)
        if reason:
            print("REFUSED  %s%s: %s" % (src, " <- " + dst if op == "move" else "", reason))
            continue
        src_at, dst_at = at["from"], at.get("to")  # where the guard found them: pinned, and the paths used
        try:
            if op == "move":
                if not exists(dst):
                    print("SKIP     %s is gone from %s" % (src, dst))
                elif exists(src):
                    print("SKIP     %s exists again; leaving %s where it is" % (src, dst))
                elif not same_volume(src, dst) and os.path.isdir(xp(dst)) and tree_info(dst)[3]:
                    # Back to another drive means a copy, and a copy must never go through a link
                    print("REFUSED  %s <- %s: holds a link to another folder; can't be copied safely" % (src, dst))
                elif dry:
                    print("WOULD    move back %s -> %s" % (dst, src))
                else:
                    os.makedirs(xp(os.path.dirname(src)), exist_ok=True)
                    with Pins() as pins:
                        reason = pinned(pins, r, (dst, dst_at), (src, src_at))
                        if reason:
                            print("REFUSED  %s <- %s: %s" % (src, dst, reason))
                            continue
                        if same_volume(src, dst):
                            os.rename(xp(dst_at), xp(src_at))
                        else:
                            shutil.move(xp(dst_at), xp(src_at))
                        late = pins.moved()
                    if late:  # see finish() in run_apply: a pinned folder was turned into a link as it was used
                        print("FAILED   %s <- %s: %s; look for it by hand" % (src, dst, late))
                        sys.exit("Stopped: a folder was turned into a link while something in it was being moved. Nothing more was done.")
                    print("DONE     moved back %s" % src)
            elif op == "mkdir":
                if os.path.isdir(xp(src)) and not os.listdir(xp(src)):
                    if not dry:
                        with Pins() as pins:
                            reason = pinned(pins, r, (src, src_at))
                            if reason:
                                print("REFUSED  %s: %s" % (src, reason))
                                continue
                            os.rmdir(xp(src_at))
                    print("%s remove folder created by the tidy-up: %s" % ("WOULD   " if dry else "DONE    ", src))
            elif op == "remove-empty-dir":
                if not dry:
                    os.makedirs(xp(src), exist_ok=True)
                print("%s recreate empty folder %s" % ("WOULD   " if dry else "DONE    ", src))
            elif op == "recycle":
                print("MANUAL   restore from the Recycle Bin: %s" % src)
        except OSError as ex:
            failed += 1
            print("FAILED   %s: %s" % (src, ex))
    if dry:
        print("\nNothing was changed. Add --apply to revert.")
    if failed:
        sys.exit(1)


# ---------------------------------------------------------------- command line
def parse_args(argv=None, description=None):
    ap = argparse.ArgumentParser(description=description or "Apply an approved plan (dry run unless --apply), or undo one.")
    ap.add_argument("plan", nargs="?", help="the plan file written by the skill's page builder")
    ap.add_argument("--approve", help='the text copied from the page: "<NAME>-APPROVE plan=... ids=..."')
    ap.add_argument("--approval", help="the approval .json downloaded from the page")
    ap.add_argument("--apply", action="store_true", help="really do it (default is a dry run)")
    ap.add_argument("--log-dir", help="where the undo log goes (default: Documents\\Tidy-up logs\\<date time>)")
    ap.add_argument("--allow-cross-drive-folders", action="store_true",
                    help="allow moving whole folders to another drive (copy, verify, recycle original)")
    ap.add_argument("--undo", help="a log folder (or its undo-log.csv) to revert")
    ap.add_argument("--root", help="the folder given to the survey's --root, if the survey used it")
    ap.add_argument("--within", action="append", default=[], metavar="FOLDER",
                    help="limit the run: every source and destination must be inside this folder (repeatable)")
    args = ap.parse_args(argv)
    if not args.undo and not args.plan:
        ap.error("give a plan file (with --approve/--approval) or --undo <log folder>")
    return args


def run(args, places=None, check=None):
    """Do what `args` (from parse_args) says. `places` and `check` are a skill's own extra rules; see the top."""
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    if args.apply and not IS_WIN:
        sys.exit("--apply is Windows-only")
    if args.undo:
        run_undo(args, places, check)
    else:
        run_apply(args, places, check)


def main(argv=None):
    run(parse_args(argv))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Read-only check of what would be lost if this PC's disk died today (backup-checkup skill).

Writes exactly one file: the JSON report named by --out. It never creates, copies,
uploads, syncs, moves or deletes anything, and it never changes a backup setting.

What it reads, and nothing else:
  * folder listings (names, sizes, dates) of your own folders. Your files are never opened.
  * the registry, and the settings files that backup and sync programs keep about
    themselves (OneDrive, Dropbox, Google Drive, Syncthing, Nextcloud, File History)
  * inside each Git repository's .git folder: config, HEAD, refs, packed-refs and the
    ref logs. Git itself is NEVER run: a repository's own config can make any git
    command start a program (core.fsmonitor, hooks, aliases). Reading the files can't.

Junctions and symlinks are never followed, online-only cloud files are never opened,
and account names and e-mail addresses are masked before they are written down.

Python 3.8+, standard library only.

    python survey.py --out survey.json
    python survey.py --out survey.json --root "<folder>" [--fake-registry reg.json] [--extra-drive "<folder>"]
"""
import argparse
import html
import json
import os
import re
import shutil
import sqlite3
import stat
import sys
import time
from collections import deque
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime

IS_WIN = os.name == "nt"
if IS_WIN:
    import ctypes
    import winreg
    from ctypes import wintypes

SCHEMA = "backup-checkup/survey@1"
LONG = "\\\\?\\"
MB, GB, DAY = 1024 ** 2, 1024 ** 3, 86400.0
A_DIRECTORY, A_REPARSE = 0x10, 0x400
A_OFFLINE, A_RECALL_OPEN, A_RECALL_DATA = 0x1000, 0x40000, 0x400000
CLOUD_ONLY = A_OFFLINE | A_RECALL_OPEN | A_RECALL_DATA
TAG_CLOUD = 0x9000001A          # cloud-files placeholder folder (OneDrive). Any other reparse folder is a link
OLD_COPY_DAYS = 30              # a copy older than this no longer counts as a second copy of today's files
WINDOW = 24                     # folder listings read at once
REPO_MAX = 2000
HEX_RX = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")
REFLOG_RX = re.compile(r"^([0-9a-f]{40,64}) ([0-9a-f]{40,64}) [^\t]*> (\d{1,12}) [+-]\d{4}(?:\t(.*))?$")

CLASSES = {
    "photos": "jpg jpeg png heic heif raw cr2 cr3 nef arw dng orf rw2 tif tiff gif bmp webp psd xcf",
    "documents": "pdf doc docx odt rtf txt md pages xls xlsx xlsm ods csv numbers ppt pptx odp key epub tex one kdbx",
    "videos": "mp4 mov mkv avi wmv webm m4v mts m2ts 3gp mpg mpeg",
    "audio": "mp3 wav flac m4a aac ogg wma opus aiff aif mid",
    "code": "py js ts tsx jsx rs go java kt c cc cpp h hpp cs html css scss json yml yaml toml sh ps1 bat ipynb sql "
            "rb php swift vue svelte lua r m",
    "archives": "zip rar 7z tar gz tgz bz2 xz zst cab",
    "installers": "exe msi msix msixbundle appx iso img dmg pkg apk",
    "programs": "dll sys pak pck vpk uasset bundle assets bnk wem rpa unity3d pdb lib obj",
}
EXT_CLASS = {e: c for c, exts in CLASSES.items() for e in exts.split()}
CLASS_WORDS = [("photos", "photo", "photos"), ("documents", "document", "documents"), ("videos", "video", "videos"),
               ("audio", "audio file", "audio files"), ("code", "code file", "code files"),
               ("archives", "archive", "archives"), ("installers", "installer", "installers"),
               ("programs", "program file", "program files"), ("other", "other file", "other files")]
LEVEL_WORDS = {3: "can't be replaced", 2: "hard to replace", 1: "can be downloaded or installed again"}
KNOWN_LEVEL = {"Pictures": 3, "Documents": 3, "Desktop": 3, "Videos": 2, "Music": 2, "Downloads": 1}
RECREATE = {"node_modules", "__pycache__", ".venv", ".tox", ".nox", ".next", ".nuxt", ".gradle", ".dart_tool",
            ".pytest_cache", ".mypy_cache", ".ruff_cache", ".turbo", ".parcel-cache", ".terraform"}
BUILD_DIRS = {"target", "build", "dist", "out", "bin", "obj", "venv"}
PROJECT_MARKERS = {"package.json", "cargo.toml", "pyproject.toml", "setup.py", "go.mod", "pom.xml", "build.gradle",
                   "build.gradle.kts", "cmakelists.txt", "composer.json", "pubspec.yaml", ".git"}
PROFILE_SKIP = {"appdata", "application data", "searches", "links", "contacts", "favorites", "3d objects", "ansel",
                "tracing", "scoop", "go", "miniconda3", "anaconda3", "pipx", "intelgraphicsprofiles",
                "microsoftedgebackups", "cookies", "local settings", "nethood", "printhood", "recent", "sendto",
                "start menu", "templates", "my documents"}
DRIVE_ROOT_SKIP = {"windows", "program files", "program files (x86)", "programdata", "users", "$recycle.bin",
                   "system volume information", "recovery", "perflogs", "config.msi", "windows.old", "msocache",
                   "$windows.~bt", "$windows.~ws", "$winreagent", "$sysreset", "onedrivetemp", "inetpub", "esd", "boot",
                   "documents and settings", "drivers", "swsetup", "intel", "amd", "nvidia", "dell", "hp", "lenovo",
                   "msi", "windowsapps", "wpsystem", "wudownloadcache", "windows10upgrade", "$getcurrent"}
GAME_DIRS = {"steamlibrary", "steam", "xboxgames", "epic games", "riot games", "ea games", "origin games", "gog games",
             "games", "gog galaxy", "battle.net", "ubisoft"}
BACKUP_NAME_RX = re.compile(r"(?i)(?:^|[^a-z])(backups?|back-up|bkp|sicherung|sauvegarde|respaldo|copia de seguridad|"
                            r"time ?machine)(?:[^a-z]|$)")
BACKUP_STORES = {"filehistory": "File History's store", "windowsimagebackup": "a Windows system image"}
BACKUP_FILE_EXTS = {"mrimg": "Macrium Reflect", "mrbak": "Macrium Reflect", "tib": "Acronis", "tibx": "Acronis",
                    "vbk": "Veeam", "vib": "Veeam", "bkf": "Windows Backup", "spf": "ShadowProtect", "gho": "Ghost"}
# Programs that keep copies but whose own settings this check can't read: (name, base, path below it, kind)
OTHER_CLIENTS = [
    ("pCloud", "local", "pCloud", "sync"), ("MEGA", "local", r"Mega Limited\MEGAsync", "sync"),
    ("Proton Drive", "local", r"Proton\Proton Drive", "sync"), ("Tresorit", "local", "Tresorit", "sync"),
    ("Synology Drive", "local", "SynologyDrive", "sync"), ("Resilio Sync", "roaming", "Resilio Sync", "sync"),
    ("Backblaze", "programdata", r"Backblaze\bzdata", "backup"), ("CrashPlan", "programdata", "CrashPlan", "backup"),
    ("Carbonite", "programdata", "Carbonite", "backup"), ("IDrive", "programdata", "IDrive", "backup"),
    ("Acronis", "programdata", r"Acronis\TrueImageHome", "backup"), ("Macrium Reflect", "programdata", r"Macrium\Reflect", "backup"),
    ("Veeam Agent", "programdata", r"Veeam\Endpoint", "backup"), ("Duplicati", "local", "Duplicati", "backup"),
    ("Arq", "programdata", "ArqAgent", "backup"), ("Kopia", "roaming", "kopia", "backup"),
]
PROCESS_OF = {"onedrive": "onedrive.exe", "dropbox": "dropbox.exe", "googledrive": "googledrivefs.exe",
              "syncthing": "syncthing.exe", "nextcloud": "nextcloud.exe", "owncloud": "owncloud.exe"}


def norm(p):
    return os.path.normcase(os.path.normpath(p))


def under(path_n, root_n):
    return path_n == root_n or path_n.startswith(root_n.rstrip("\\/") + os.sep)


def iso(ts):
    try:
        return datetime.fromtimestamp(ts).strftime("%Y-%m-%d") if ts and ts > 0 else None
    except (OverflowError, OSError, ValueError):
        return None


def ext_of(name):
    i = name.rfind(".")
    return name[i + 1:].lower() if i > 0 else ""


def mask(account):
    """j•••@example.com: enough to recognise your own account, not enough to copy it."""
    if not isinstance(account, str) or not account.strip():
        return None
    account = account.strip()
    if "@" in account:
        user, dom = account.split("@", 1)
        return user[:1] + "\u2022\u2022\u2022@" + dom[:80]
    return account[:1] + "\u2022\u2022\u2022"


def clean(text, limit=200):
    """One line of text from a settings file, with control characters removed."""
    return re.sub(r"[\x00-\x1f\x7f\u2028\u2029]+", " ", str(text)).strip()[:limit]


def filetime(v):
    """A Windows FILETIME (number or 8 raw bytes) as Unix seconds, or None."""
    if isinstance(v, (bytes, bytearray)) and len(v) == 8:
        v = int.from_bytes(v, "little")
    if not isinstance(v, int) or isinstance(v, bool) or v <= 0:
        return None
    ts = v / 10000000.0 - 11644473600
    return ts if 946684800 < ts < 4102444800 else None


def read_small(path, limit=1024 * 1024, tail=False):
    """Bytes of a small settings file. Never through a link, never an online-only file, never a big file."""
    try:
        st = os.lstat(path)
    except OSError:
        return None
    attrs = getattr(st, "st_file_attributes", 0)
    if not stat.S_ISREG(st.st_mode) or attrs & (A_REPARSE | CLOUD_ONLY):
        return None
    if st.st_size > limit and not tail:
        return None
    try:
        with open(path, "rb") as f:
            if st.st_size > limit:
                f.seek(st.st_size - limit)
            return f.read(limit)
    except OSError:
        return None


def read_text(path, limit=1024 * 1024, tail=False):
    data = read_small(path, limit, tail)
    if data is None:
        return None
    if data[:2] in (b"\xff\xfe", b"\xfe\xff"):
        return data.decode("utf-16", "replace")
    return data.decode("utf-8", "replace")


def listing(path):
    """One folder's entries as (name, attributes, size, modified, reparse tag, is a symlink), or the error.
    Everything comes from the folder's own listing; no entry is opened. Safe in a worker thread."""
    out = []
    try:
        with os.scandir(LONG + path if IS_WIN else path) as it:
            for e in it:
                try:
                    st = e.stat(follow_symlinks=False)
                except OSError:
                    continue
                attrs = getattr(st, "st_file_attributes", None)
                if attrs is None:
                    attrs = A_DIRECTORY if e.is_dir(follow_symlinks=False) else 0
                out.append((e.name, attrs, st.st_size, st.st_mtime, getattr(st, "st_reparse_tag", 0), e.is_symlink()))
    except OSError as ex:
        return ex
    return out


NETWORK_LETTERS = set()         # drive letters that are network shares (filled in by real_drives)
NETWORK_FOLDERS = []            # main folders that live on a network place


def is_network(p):
    """A \\\\server\\share address, or a path on a drive letter that is mapped to one."""
    if not isinstance(p, str):
        return False
    if p.startswith(("\\\\", "//")) and not p.startswith(LONG):
        return True
    return p[1:2] == ":" and p[:1].upper() in NETWORK_LETTERS


def is_local_dir(p):
    """True for a folder on this PC. A network place is never contacted to find out."""
    return isinstance(p, str) and bool(p.strip()) and not is_network(p) and os.path.isdir(p)


def is_link(attrs, tag, link):
    return link or bool(attrs & A_REPARSE and (tag & 0xFFFF0FFF) != TAG_CLOUD)


# ---------------------------------------------------------------- registry, processes, drives (all read-only)
class Registry(object):
    """The real registry, or a JSON stand-in for tests ({"registry": {"HKCU\\\\Software\\\\...": {...}}, "processes": [...]})."""

    def __init__(self, fake=None, root=None):
        self.fake = None
        self.processes = None
        if fake is not None:
            with open(fake, encoding="utf-8") as f:
                data = json.load(f)

            def fix(v):
                return v.replace("%ROOT%", root) if isinstance(v, str) else v

            self.fake = {k.lower(): {n: fix(v) for n, v in vals.items()} for k, vals in (data.get("registry") or {}).items()}
            self.processes = {str(p).lower() for p in data.get("processes") or []}
        elif root is not None:
            self.fake = {}                      # test folder with no stand-in: an empty registry, programs unknown

    def values(self, hive, path):
        if self.fake is not None:
            return dict(self.fake.get((hive + "\\" + path).lower(), {}))
        out = {}
        root = winreg.HKEY_CURRENT_USER if hive == "HKCU" else winreg.HKEY_LOCAL_MACHINE
        for flag in ((0,) if hive == "HKCU" else (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY)):
            try:
                with winreg.OpenKey(root, path, 0, winreg.KEY_READ | flag) as k:
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

    def subkeys(self, hive, path):
        if self.fake is not None:
            prefix = (hive + "\\" + path).lower() + "\\"
            return sorted({k[len(prefix):].split("\\")[0] for k in self.fake if k.startswith(prefix)})
        names = set()
        root = winreg.HKEY_CURRENT_USER if hive == "HKCU" else winreg.HKEY_LOCAL_MACHINE
        try:
            with winreg.OpenKey(root, path, 0, winreg.KEY_READ) as k:
                i = 0
                while True:
                    try:
                        names.add(winreg.EnumKey(k, i))
                    except OSError:
                        break
                    i += 1
        except OSError:
            pass
        return sorted(names)

    def running(self):
        """Lower-cased names of running programs (a snapshot from Windows; nothing is started or stopped)."""
        if self.processes is not None or self.fake is not None:
            return self.processes
        names = set()
        try:
            k32 = ctypes.WinDLL("kernel32", use_last_error=True)

            class PROCESSENTRY32W(ctypes.Structure):
                _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD), ("th32ProcessID", wintypes.DWORD),
                            ("th32DefaultHeapID", ctypes.c_size_t), ("th32ModuleID", wintypes.DWORD),
                            ("cntThreads", wintypes.DWORD), ("th32ParentProcessID", wintypes.DWORD),
                            ("pcPriClassBase", ctypes.c_long), ("dwFlags", wintypes.DWORD),
                            ("szExeFile", ctypes.c_wchar * 260)]

            k32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
            k32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
            k32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
            k32.CloseHandle.argtypes = [wintypes.HANDLE]
            snap = k32.CreateToolhelp32Snapshot(0x2, 0)
            if snap in (None, wintypes.HANDLE(-1).value):
                return None
            entry = PROCESSENTRY32W()
            entry.dwSize = ctypes.sizeof(entry)
            ok = k32.Process32FirstW(snap, ctypes.byref(entry))
            while ok:
                names.add(entry.szExeFile.lower())
                ok = k32.Process32NextW(snap, ctypes.byref(entry))
            k32.CloseHandle(snap)
        except (OSError, AttributeError, ValueError):
            return None
        self.processes = names
        return names


def disk_number(letter):
    """Which physical disk a drive letter is on, or None. Asks Windows about the volume; reads nothing from it."""
    try:
        k32 = ctypes.WinDLL("kernel32", use_last_error=True)

        class DEVICE_NUMBER(ctypes.Structure):
            _fields_ = [("DeviceType", wintypes.DWORD), ("DeviceNumber", wintypes.DWORD), ("PartitionNumber", wintypes.DWORD)]

        k32.CreateFileW.restype = wintypes.HANDLE
        k32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD,
                                    wintypes.DWORD, wintypes.HANDLE]
        k32.DeviceIoControl.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD, wintypes.LPVOID,
                                        wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
        k32.CloseHandle.argtypes = [wintypes.HANDLE]
        h = k32.CreateFileW("\\\\.\\%s:" % letter, 0, 3, None, 3, 0, None)     # no access asked for: a query only
        if h in (None, wintypes.HANDLE(-1).value):
            return None
        try:
            out, n = DEVICE_NUMBER(), wintypes.DWORD(0)
            ok = k32.DeviceIoControl(h, 0x2D1080, None, 0, ctypes.byref(out), ctypes.sizeof(out), ctypes.byref(n), None)
            return int(out.DeviceNumber) if ok else None
        finally:
            k32.CloseHandle(h)
    except (OSError, AttributeError, ValueError):
        return None


def real_drives(profile):
    k32 = ctypes.windll.kernel32
    mask_bits = k32.GetLogicalDrives()
    kinds = {2: "removable", 3: "fixed", 4: "network", 5: "disc", 6: "ramdisk"}
    home = os.path.splitdrive(profile)[0].upper()
    out = []
    for i in range(26):
        if not mask_bits & (1 << i):
            continue
        letter = chr(65 + i)
        root = letter + ":\\"
        kind = kinds.get(k32.GetDriveTypeW(root), "unknown")
        d = {"root": root, "type": kind, "label": "", "system": letter + ":" == home}
        if kind == "network":
            NETWORK_LETTERS.add(letter)
        if kind in ("fixed", "removable"):
            buf = ctypes.create_unicode_buffer(1024)
            target = buf.value if k32.QueryDosDeviceW(letter + ":", buf, 1024) else ""
            if not target.lower().startswith("\\device\\harddiskvolume"):
                d["type"] = "virtual"            # a cloud drive or other stand-in, not a disk
            else:
                label = ctypes.create_unicode_buffer(261)
                if k32.GetVolumeInformationW(root, label, 261, None, None, None, None, 0):
                    d["label"] = label.value
                try:
                    u = shutil.disk_usage(root)
                    d.update(total=u.total, free=u.free)
                except OSError:
                    d["type"] = "locked"
                d["disk"] = disk_number(letter)
        out.append(d)
    sysdisk = next((d.get("disk") for d in out if d["system"]), None)
    for d in out:
        if d["type"] in ("fixed", "removable") and not d["system"]:
            d["same_disk"] = (d.get("disk") == sysdisk) if d.get("disk") is not None and sysdisk is not None else None
    return out


def known_folders(profile, reg, test):
    names = {"Desktop": "Desktop", "Documents": "Personal", "Pictures": "My Pictures", "Videos": "My Video",
             "Music": "My Music", "Downloads": "{374DE290-123F-4565-9164-39C4925E467B}"}
    usf = reg.values("HKCU", r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders")
    out = {}
    for label, key in names.items():
        v = usf.get(key)
        p = None
        if isinstance(v, str) and v.strip():
            p = os.path.normpath(v.replace("%USERPROFILE%", profile) if test else os.path.expandvars(v))
        p = p or os.path.join(profile, label)
        out[label] = p if is_local_dir(p) else None
        if is_network(p):
            NETWORK_FOLDERS.append(label)
    return out


# ---------------------------------------------------------------- Git, by reading files only
def git_config(text):
    """Remotes and branch upstreams from a .git/config. A plain line reader: nothing in the file is acted on."""
    remotes, branches, section, sub = {}, {}, None, None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line[0] in "#;":
            continue
        m = re.match(r'^\[\s*([A-Za-z0-9.\-]+)(?:\s+"((?:[^"\\]|\\.)*)")?\s*\]', line)
        if m:
            section, sub = m.group(1).lower(), m.group(2)
            if sub is None and "." in section:                     # old style: [remote.origin]
                section, sub = section.split(".", 1)
            elif sub is not None:
                sub = re.sub(r"\\(.)", r"\1", sub)
            continue
        if "=" not in line or sub is None:
            continue
        k, v = line.split("=", 1)
        k, v = k.strip().lower(), v.strip().strip('"')
        if section == "remote" and k == "url":
            remotes.setdefault(sub, []).append(v)
        elif section == "branch" and k in ("remote", "merge"):
            branches.setdefault(sub, {})[k] = v
    return remotes, branches


def remote_shown(url):
    """A remote address for the page: no user name, password or token, and where it points."""
    url = clean(url, 400)
    m = re.match(r"^[A-Za-z][A-Za-z0-9+.\-]*://(?:[^/@]*@)?([^/:?#]+)(?::\d+)?(/[^?#]*)?", url)
    if m and not url.lower().startswith("file:"):
        host, path = m.group(1), (m.group(2) or "")
    else:
        m = re.match(r"^(?:[^@/\\:]+@)?([A-Za-z0-9.\-]+\.[A-Za-z]{2,}):([^\\]+)$", url)     # git@host:owner/repo
        if not m:
            return {"shown": "another folder, not a server", "local": True}
        host, path = m.group(1), "/" + m.group(2)
    parts = [p for p in path.split("/") if p]
    tail = "/".join(parts[-2:])
    if tail.endswith(".git"):
        tail = tail[:-4]
    return {"shown": clean(host + ("/" + tail if tail else ""), 120), "local": False}


def git_refs(gitdir):
    """{ref name: commit id} from packed-refs and the loose ref files."""
    refs = {}
    packed = read_text(os.path.join(gitdir, "packed-refs"), 8 * MB)
    for line in (packed or "").splitlines():
        if not line or line[0] in "#^":
            continue
        sha, _, name = line.partition(" ")
        name = name.strip()
        if HEX_RX.match(sha) and name.startswith("refs/"):
            refs[name] = sha
    seen = 0
    stack = [("refs/heads", 0), ("refs/remotes", 0)]
    while stack and seen < 5000:
        rel, depth = stack.pop()
        entries = listing(os.path.join(gitdir, rel.replace("/", os.sep)))
        if isinstance(entries, OSError):
            continue
        for name, attrs, size, m, tag, link in entries:
            if is_link(attrs, tag, link):
                continue
            child = rel + "/" + name
            if attrs & A_DIRECTORY:
                if depth < 8:
                    stack.append((child, depth + 1))
                continue
            seen += 1
            text = read_text(os.path.join(gitdir, child.replace("/", os.sep)), 256)
            if text and HEX_RX.match(text.strip()):
                refs[child] = text.strip()
    stash = read_text(os.path.join(gitdir, "refs", "stash"), 256)
    if stash and HEX_RX.match(stash.strip()):
        refs["refs/stash"] = stash.strip()
    return refs


def reflog(gitdir, ref):
    """[(old, new, time, is a commit)] from a ref's log. Only the ids and the time are kept."""
    text = read_text(os.path.join(gitdir, "logs", ref.replace("/", os.sep)), 2 * MB, tail=True)
    out = []
    for line in (text or "").splitlines():
        m = REFLOG_RX.match(line)
        if m:
            out.append((m.group(1), m.group(2), int(m.group(3)), (m.group(4) or "").startswith("commit")))
    return out


def inspect_repo(path):
    """What a repository's own files say about work that exists nowhere else. Git is never run."""
    gitdir = os.path.join(path, ".git")
    r = {"path": path, "name": os.path.basename(path.rstrip("\\/")), "remotes": [], "branches": [], "stashes": 0,
         "head": None, "last_commit": None, "last_remote_update": None, "state": "ok", "notes": []}
    if os.path.isdir(os.path.join(gitdir, "reftable")):
        r["state"] = "unknown"
        r["notes"].append("uses a newer way of storing branches that this check can't read")
        return r, None
    remotes, upstreams = git_config(read_text(os.path.join(gitdir, "config"), MB) or "")
    for name, urls in list(remotes.items())[:10]:
        shown = remote_shown(urls[0]) if urls else {"shown": "", "local": True}
        r["remotes"].append({"name": clean(name, 60), "shown": shown["shown"], "local": shown["local"]})
    off_site = [x for x in r["remotes"] if not x["local"]]
    head = (read_text(os.path.join(gitdir, "HEAD"), 1024) or "").strip()
    refs = git_refs(gitdir)
    heads = {k[len("refs/heads/"):]: v for k, v in refs.items() if k.startswith("refs/heads/")}
    remote_tips = {k: v for k, v in refs.items() if k.startswith("refs/remotes/")}
    remote_ids = set(remote_tips.values())
    remote_history, last_remote = set(), 0
    for ref in list(remote_tips)[:200]:
        for old, new, ts, _ in reflog(gitdir, ref):
            remote_history.update((old, new))
            last_remote = max(last_remote, ts)
    if head.startswith("ref: refs/heads/"):
        r["head"] = clean(head[len("ref: refs/heads/"):], 120)
    elif HEX_RX.match(head):
        r["head"] = "(no branch)"
        if head not in set(heads.values()) and head not in remote_ids and head not in remote_history:
            r["notes"].append("the commit you are on belongs to no branch")
    head_log = reflog(gitdir, "HEAD")
    last_commit = max([ts for _, _, ts, _ in head_log] or [0])
    worst = 0                                   # 0 in step, 1 differs, 2 only here
    for name, sha in sorted(heads.items())[:200]:
        b = {"name": clean(name, 120), "state": "in-step", "ahead": None}
        up = upstreams.get(name) or {}
        up_ref = None
        if up.get("remote") and up.get("merge", "").startswith("refs/heads/"):
            up_ref = "refs/remotes/%s/%s" % (up["remote"], up["merge"][len("refs/heads/"):])
        if not remotes:
            b["state"] = "no-remote"
        elif sha in remote_ids:
            b["state"] = "in-step"              # its newest commit is the newest commit of a remote branch
        elif sha in remote_history:
            b["state"] = "behind"               # the remote copy had this commit and has moved on
        else:
            log = reflog(gitdir, "refs/heads/" + name)
            target = remote_tips.get(up_ref) if up_ref else None
            marks = [i for i, (old, new, _, _) in enumerate(log) if new in ((target,) if target else remote_ids)]
            if marks:
                b["state"] = "ahead"
                b["ahead"] = sum(1 for _, _, _, is_commit in log[marks[-1] + 1:] if is_commit) or None
            elif up_ref is None or up_ref not in remote_tips:
                b["state"] = "only-here"        # never pushed under this name, and its tip is on no remote branch
            else:
                b["state"] = "differs"          # can't tell which side is newer without running git
        worst = max(worst, {"in-step": 0, "behind": 0, "differs": 1}.get(b["state"], 2))
        r["branches"].append(b)
    stash_log = read_text(os.path.join(gitdir, "logs", "refs", "stash"), MB, tail=True)
    r["stashes"] = len([x for x in (stash_log or "").splitlines() if x.strip()]) or (1 if "refs/stash" in refs else 0)
    r["last_commit"] = iso(last_commit)
    r["last_remote_update"] = iso(last_remote)
    if not heads:
        r["state"] = "empty" if not remotes else "ok"
        r["notes"].append("no commits yet")
    elif not remotes:
        r["state"] = "no-remote"
    elif not off_site:
        r["state"] = "local-remote"
    elif worst == 2:
        r["state"] = "unpushed"
    elif r["stashes"]:
        r["state"] = "stash"
    elif worst == 1:
        r["state"] = "differs"
    return r, last_commit


# ---------------------------------------------------------------- the check
class Place(object):
    def __init__(self, label, path, kind):
        self.label, self.path, self.kind = label, path, kind
        self.bytes = self.files = self.recreate = self.git_bytes = self.cloud_bytes = self.cloud_files = 0
        self.newest = 0.0
        self.cls = {c: [0, 0] for c in list(CLASSES) + ["other"]}
        self.truncated = self.measured = False
        self.repos, self.linked_worktrees = [], 0
        self.loose_only = False


class Survey(object):
    def __init__(self, args):
        self.args = args
        self.now = time.time()
        self.t0 = time.time()
        self.test = bool(args.root)
        self.entries = self.errors = self.links = 0
        self.truncated = False
        self.repos, self.repo_stats = [], {}
        self.pool = ThreadPoolExecutor(max_workers=max(1, args.threads))
        if args.root:
            root = os.path.abspath(args.root)
            self.env = {"profile": root, "local": os.path.join(root, "AppData", "Local"),
                        "roaming": os.path.join(root, "AppData", "Roaming"), "programdata": os.path.join(root, "ProgramData")}
            self.reg = Registry(args.fake_registry, root)
            self.drives = [{"root": root + "\\", "type": "fixed", "label": "Test folder", "system": True, "test": True}]
            for kind, folders in (("disk", args.extra_drive or []), ("partition", args.extra_partition or [])):
                for f in folders:
                    f = os.path.abspath(f)
                    self.drives.append({"root": f + "\\", "type": "fixed", "label": os.path.basename(f), "system": False,
                                        "test": True, "same_disk": kind == "partition", "total": 500 * GB, "free": 200 * GB})
        else:
            env, profile = os.environ, os.path.expanduser("~")
            self.env = {"profile": profile, "local": env.get("LOCALAPPDATA") or os.path.join(profile, "AppData", "Local"),
                        "roaming": env.get("APPDATA") or os.path.join(profile, "AppData", "Roaming"),
                        "programdata": env.get("ProgramData") or r"C:\ProgramData"}
            self.reg = Registry()
            self.drives = real_drives(profile)
        self.kf = known_folders(self.env["profile"], self.reg, self.test)
        self.protectors = []
        self.cant_see = []

    # -------------------------------------------------- who keeps copies
    def add(self, pid, name, kind, state, facts=None, covers=None, account=None, coverage="known", last_copy=None):
        p = {"id": pid, "name": name, "kind": kind, "state": state, "facts": facts or [], "covers": covers or [],
             "coverage": coverage}
        if account:
            p["account"] = account
        if last_copy:
            p["last_copy"] = iso(last_copy)
        exe = PROCESS_OF.get(pid)
        if exe and state == "set-up":
            running = self.reg.running()
            p["running"] = None if running is None else exe in running
            if p["running"] is False:
                p["facts"].append("It is not running right now, so nothing is being copied.")
        self.protectors.append(p)
        return p

    def find_protectors(self):
        e, reg = self.env, self.reg
        # OneDrive: accounts in the registry, the folder each one syncs
        accounts, roots = [], []
        base = r"Software\Microsoft\OneDrive\Accounts"
        for sub in reg.subkeys("HKCU", base):
            v = reg.values("HKCU", base + "\\" + sub)
            folder = v.get("UserFolder")
            if is_local_dir(folder):
                roots.append(os.path.normpath(folder))
                accounts.append(mask(v.get("UserEmail")) or ("personal" if sub.lower() == "personal" else "work"))
        if not roots and not self.test:
            for var in ("OneDrive", "OneDriveConsumer", "OneDriveCommercial"):
                p = os.environ.get(var)
                if is_local_dir(p) and os.path.normpath(p) not in roots:
                    roots.append(os.path.normpath(p))
        if roots:
            inside = [k for k in ("Desktop", "Documents", "Pictures") if self.kf.get(k) and
                      any(under(norm(self.kf[k]), norm(r)) for r in roots)]
            outside = [k for k in ("Desktop", "Documents", "Pictures") if self.kf.get(k) and k not in inside]
            facts = ["Folder backup is on for: " + ", ".join(inside) + "." if inside else
                     "Folder backup is off: Desktop, Documents and Pictures are not inside the OneDrive folder."]
            if inside and outside:
                facts.append("Not backed up by it: " + ", ".join(outside) + ".")
            self.add("onedrive", "OneDrive", "sync", "set-up", facts,
                     [{"path": r, "how": "inside your OneDrive folder"} for r in roots], ", ".join(a for a in accounts if a))
        else:
            self.add("onedrive", "OneDrive", "sync", "not-set-up", ["No signed-in OneDrive folder was found."])
        # Dropbox: info.json names its folder
        for base_dir in (e["roaming"], e["local"]):
            text = read_text(os.path.join(base_dir, "Dropbox", "info.json"), 256 * 1024)
            if not text:
                continue
            try:
                info = json.loads(text)
            except ValueError:
                continue
            covers = [{"path": os.path.normpath(v["path"]), "how": "inside your Dropbox folder"}
                      for v in (info.values() if isinstance(info, dict) else []) if isinstance(v, dict)
                      and is_local_dir(v.get("path"))]
            if covers:
                self.add("dropbox", "Dropbox", "sync", "set-up", [], covers)
                break
        # Google Drive for desktop
        drivefs = os.path.join(e["local"], "Google", "DriveFS")
        if os.path.isdir(drivefs):
            covers = self.google_drive_roots(drivefs)
            self.add("googledrive", "Google Drive", "sync", "set-up",
                     [] if covers else ["It is installed, but this check can't see which folders it backs up. Look at "
                                        "its Preferences > 'Folders from your computer'."],
                     covers, coverage="known" if covers else "unknown")
        # iCloud Drive and Box: a fixed folder
        for pid, name, folder, needs in (("icloud", "iCloud Drive", "iCloudDrive", None),
                                         ("box", "Box", "Box", os.path.join(e["local"], "Box", "Box"))):
            p = os.path.join(e["profile"], folder)
            if is_local_dir(p) and (needs is None or os.path.isdir(needs)):
                self.add(pid, name, "sync", "set-up", [], [{"path": p, "how": "inside your %s folder" % name}])
        # Syncthing: config.xml lists folders and the devices they are shared with
        text = read_text(os.path.join(e["local"], "Syncthing", "config.xml"), 4 * MB)
        if text and "<!DOCTYPE" not in text and "<!ENTITY" not in text:
            covers, alone = [], 0
            text = re.sub(r"<defaults>.*?</defaults>", "", text, flags=re.S)      # the template for new folders is not a folder
            for m in re.finditer(r"<folder\b([^>]*)>(.*?)</folder>", text, re.S):
                attrs = dict(re.findall(r'(\w+)="([^"]*)"', m.group(1)))
                path = html.unescape(attrs.get("path", ""))
                others = len(re.findall(r"<device\b", m.group(2))) - 1
                if path.startswith("~"):
                    path = e["profile"] + path[1:]
                if not is_local_dir(path):
                    continue
                if others >= 1 and attrs.get("paused") != "true":
                    covers.append({"path": os.path.normpath(path),
                                   "how": "Syncthing shares it with %d other device%s" % (others, "" if others == 1 else "s")})
                else:
                    alone += 1
            facts = ["%d folder%s shared with no other device or paused: no second copy there." % (alone, " is" if alone == 1 else "s are")] if alone else []
            if covers or alone:
                self.add("syncthing", "Syncthing", "sync", "set-up", facts, covers)
        # Nextcloud and ownCloud: the .cfg lists local folders
        for pid, name in (("nextcloud", "Nextcloud"), ("owncloud", "ownCloud")):
            text = read_text(os.path.join(e["roaming"], name, name.lower() + ".cfg"), MB)
            if text:
                paths = [os.path.normpath(p.strip().replace("/", "\\")) for p in re.findall(r"(?mi)^\d+\\Folders\w*\\\d+\\localPath=(.+)$", text)]
                covers = [{"path": p, "how": "inside your %s folder" % name} for p in paths if is_local_dir(p)]
                if covers:
                    self.add(pid, name, "sync", "set-up", [], covers)
        # Programs whose settings this check can't read
        for name, base_key, rel, kind in OTHER_CLIENTS:
            if os.path.isdir(os.path.join(e[base_key], rel)):
                self.add(name.lower().replace(" ", ""), name, kind, "installed",
                         ["It is installed, but this check can't see what it covers. Open it and look."], coverage="unknown")
        self.file_history()
        self.windows_backup()
        v = reg.values("HKLM", r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore")
        on = v.get("RPSessionInterval")
        self.add("systemrestore", "System Restore", "windows", "note",
                 [("It is on. " if on == 1 else "It looks off. " if on == 0 else "") +
                  "It is not a backup of your files: it keeps Windows' own settings and programs, on the same disk."],
                 coverage="none")

    def google_drive_roots(self, drivefs):
        """Folders Google Drive says it mirrors or backs up, from its own settings database (opened read-only, as-is)."""
        db = os.path.join(drivefs, "root_preference_sqlite.db")
        covers = []
        try:
            st = os.lstat(db)
            if not stat.S_ISREG(st.st_mode) or st.st_size > 64 * MB:
                return covers
            con = sqlite3.connect("file:" + db.replace("\\", "/").replace("%", "%25").replace("?", "%3f").replace("#", "%23")
                                  + "?mode=ro&immutable=1", uri=True, timeout=1)
            try:
                cols = [row[1] for row in con.execute("PRAGMA table_info(roots)")]
                wanted = [c for c in ("root_path", "last_seen_absolute_path") if c in cols]
                seen = set()
                for c in wanted:
                    for (value,) in con.execute('SELECT "%s" FROM roots LIMIT 200' % c):
                        if isinstance(value, str) and value.strip():
                            p = os.path.normpath(value.replace("\\\\?\\", ""))
                            if norm(p) not in seen and is_local_dir(p):
                                seen.add(norm(p))
                                covers.append({"path": p, "how": "listed in Google Drive's own settings as a folder it backs up"})
            finally:
                con.close()
        except (sqlite3.Error, OSError, ValueError):
            pass
        return covers

    def file_history(self):
        e, reg = self.env, self.reg
        text = None
        for name in ("Config1.xml", "Config2.xml"):
            text = text or read_text(os.path.join(e["local"], "Microsoft", "Windows", "FileHistory", "Configuration", name), 4 * MB)

        def tags(tag):
            return [html.unescape(x).strip() for x in re.findall(r"<%s>(.*?)</%s>" % (tag, tag), text or "", re.S)]

        target = (tags("TargetUrl") or [None])[0]
        if not text or not target:
            self.add("filehistory", "File History", "windows", "not-set-up", ["File History is not set up."])
            return
        name = clean((tags("TargetName") or [""])[0], 60)
        network = target.startswith("\\\\")
        present = None if network else os.path.isdir(target)       # a network address is never probed
        last = filetime(reg.values("HKCU", r"Software\Microsoft\Windows\CurrentVersion\FileHistory").get("ProtectedUpToTime")) or 0
        if present:                                                # the store's own dates say when it last wrote
            store = os.path.join(target, "FileHistory")
            stack, seen = [(store, 0)], 0
            while stack and seen < 400:
                p, depth = stack.pop()
                entries = listing(p)
                if isinstance(entries, OSError):
                    continue
                for n, attrs, size, m, tag, link in entries:
                    seen += 1
                    if is_link(attrs, tag, link):
                        continue
                    if attrs & A_DIRECTORY:
                        if depth < 3 and n.lower() != "data":
                            stack.append((os.path.join(p, n), depth + 1))
                    elif "configuration" in p.lower():
                        last = max(last, m)
        covers, excluded = [], [norm(x) for x in tags("FolderExclude") if x]
        folders = [os.path.normpath(x) for x in tags("UserFolder") if x]
        for lib in tags("Library"):                                # ...\Libraries\Pictures.library-ms means your Pictures
            m = re.search(r"([A-Za-z]+)\.library-ms\s*$", lib)
            if m and self.kf.get(m.group(1).capitalize()):
                folders.append(self.kf[m.group(1).capitalize()])
        age = (self.now - last) / DAY if last else None
        for f in folders:
            if is_local_dir(f) and not any(under(norm(f), x) for x in excluded) and norm(f) not in [norm(c["path"]) for c in covers]:
                covers.append({"path": f, "how": "File History copies it to " + (name or "its drive"), "last_copy": iso(last),
                               "age_days": None if age is None else int(age), "age_unknown": age is None})
        where = (name or "a drive") + (" (a network place)" if network else "")
        facts = ["It copies to %s." % where]
        if present is False:
            facts.append("That drive is not connected right now.")
        facts.append("Last copy: %s (%d day%s ago)." % (iso(last), int(age), "" if int(age) == 1 else "s") if last
                     else "This check couldn't read when it last ran.")
        self.add("filehistory", "File History", "windows", "set-up", facts, covers, last_copy=last or None)

    def windows_backup(self):
        base = r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsBackup"
        v = self.reg.values("HKLM", base)
        v.update(self.reg.values("HKLM", base + r"\Status"))
        last = filetime(v.get("LastSuccess"))
        if not v.get("ValidConfig") and not last:
            return
        age = (self.now - last) / DAY if last else None
        self.add("windowsbackup", "Backup and Restore (Windows 7)", "windows", "set-up",
                 [("Last finished: %s (%d days ago)." % (iso(last), int(age))) if last else "This check couldn't read when it last ran.",
                  "This check can't see which folders it includes. Open it in Control Panel and look."],
                 coverage="unknown", last_copy=last)

    # -------------------------------------------------- other drives
    def look_at_drives(self):
        for d in self.drives:
            d["backups"] = []
            if d["system"] or d["type"] not in ("fixed", "removable"):
                continue
            entries = listing(d["root"].rstrip("\\") if len(d["root"]) > 3 else d["root"])
            if isinstance(entries, OSError):
                continue
            for name, attrs, size, m, tag, link in entries:
                if is_link(attrs, tag, link):
                    continue
                lname = name.lower()
                path = os.path.join(d["root"], name)
                if attrs & A_DIRECTORY:
                    kind = BACKUP_STORES.get(lname) or ("a folder named like a backup" if BACKUP_NAME_RX.search(name) else None)
                    if kind:
                        newest, names = self.glance(path)
                        d["backups"].append({"name": name, "path": path, "kind": kind, "newest": iso(newest), "_names": names})
                elif ext_of(name) in BACKUP_FILE_EXTS:
                    d["backups"].append({"name": name, "path": path, "kind": "a %s backup file" % BACKUP_FILE_EXTS[ext_of(name)],
                                         "newest": iso(m), "bytes": size})

    def glance(self, path, limit=3000):
        """Newest date in a folder that looks like a backup, and the names at its top. A shallow look."""
        newest, seen, top = 0.0, 0, []
        stack = [(path, 0)]
        while stack and seen < limit:
            p, depth = stack.pop()
            entries = listing(p)
            if isinstance(entries, OSError):
                continue
            for name, attrs, size, m, tag, link in entries:
                seen += 1
                if is_link(attrs, tag, link):
                    continue
                if depth == 0 and len(top) < 40:
                    top.append(name)
                if attrs & A_DIRECTORY:
                    if depth < 3:
                        stack.append((os.path.join(p, name), depth + 1))
                elif m > newest:
                    newest = m
        return newest, top

    # -------------------------------------------------- your folders
    def plan_places(self):
        e, places, seen = self.env, [], set()

        def add(label, path, kind):
            if is_local_dir(path) and norm(path) not in seen:
                seen.add(norm(path))
                places.append(Place(label, os.path.normpath(path), kind))

        for k in ("Desktop", "Documents", "Pictures", "Videos", "Music", "Downloads"):
            add(k, self.kf.get(k), "known")
        for p in self.protectors:
            if p["kind"] == "sync":
                for c in p["covers"]:
                    add(p["name"] + " folder", c["path"], "sync")
        entries = listing(e["profile"])
        for name, attrs, size, m, tag, link in ([] if isinstance(entries, OSError) else sorted(entries)):
            if attrs & A_DIRECTORY and not is_link(attrs, tag, link) and not name.startswith((".", "$")) \
                    and name.lower() not in PROFILE_SKIP and not (attrs & 0x2 and attrs & 0x4):
                add(name, os.path.join(e["profile"], name), "profile")
        loose = Place("Loose files in your profile folder", e["profile"], "loose")
        loose.loose_only = True
        places.append(loose)
        backups = {norm(b["path"]) for d in self.drives for b in d.get("backups", [])}
        self.left_out = []
        for d in self.drives:
            if d["type"] != "fixed" or (d.get("test") and d["system"]):
                continue
            entries = listing(d["root"])
            for name, attrs, size, m, tag, link in ([] if isinstance(entries, OSError) else sorted(entries)):
                path = os.path.join(d["root"], name)
                if not attrs & A_DIRECTORY or is_link(attrs, tag, link) or name.startswith(("$", ".")) \
                        or (attrs & 0x2 and attrs & 0x4) or norm(path) in backups:
                    continue
                if name.lower() in DRIVE_ROOT_SKIP:
                    continue
                if name.lower() in GAME_DIRS:
                    self.left_out.append(path)
                    continue
                add(path, path, "drive")
        self.places = places
        self.place_roots = {norm(p.path) for p in places if not p.loose_only}
        self.skip_roots = backups | {norm(os.path.join(e["profile"], "AppData"))}

    def walk(self, place, deadline):
        todo = [(place.path, None, 0)]          # (folder, the repository it is in, 0 yours / 1 can be made again / 2 inside .git)
        flying = deque()
        roots, skip = self.place_roots, self.skip_roots
        place.measured = True
        while todo or flying:
            if time.time() > deadline or self.entries > self.args.max_entries:
                place.truncated = self.truncated = True
                for fut, _, _, _ in flying:
                    fut.cancel()
                return
            while todo and len(flying) < WINDOW:
                path, repo, mode = todo.pop()
                flying.append((self.pool.submit(listing, path), path, repo, mode))
            fut, path, repo, mode = flying.popleft()
            entries = fut.result()
            if isinstance(entries, OSError):
                self.errors += 1
                continue
            self.entries += len(entries)
            is_project = False
            if mode == 0:
                lower = {n.lower(): a for n, a, _, _, _, _ in entries}
                is_project = bool(PROJECT_MARKERS & lower.keys())
                if ".git" in lower and not place.loose_only:
                    git = next(x for x in entries if x[0].lower() == ".git")
                    if git[1] & A_DIRECTORY and not is_link(git[1], git[4], git[5]):
                        if len(self.repos) < REPO_MAX:
                            repo = self.new_repo(path, place)
                    elif not git[1] & A_DIRECTORY:
                        place.linked_worktrees += 1
            base = path if path.endswith("\\") else path + "\\"
            for name, attrs, size, m, tag, link in entries:
                if attrs & A_DIRECTORY:
                    if place.loose_only:
                        continue
                    if is_link(attrs, tag, link) or attrs & (A_OFFLINE | A_RECALL_OPEN):
                        self.links += 1
                        continue
                    child = base + name
                    child_n = norm(child)
                    if child_n in roots or child_n in skip:
                        continue                # counted as its own folder, or a backup store
                    lname = name.lower()
                    if mode:
                        cmode = mode
                    elif lname == ".git" and repo is not None and repo["path"] == path:
                        cmode = 2
                    elif lname in RECREATE or (is_project and lname in BUILD_DIRS):
                        cmode = 1
                    else:
                        cmode = 0
                    todo.append((child, repo, cmode))
                    continue
                if link:
                    continue
                if place.loose_only and (name.lower().startswith(("ntuser", ".")) or attrs & 0x4):
                    continue
                place.bytes += size
                place.files += 1
                if attrs & CLOUD_ONLY:
                    place.cloud_bytes += size
                    place.cloud_files += 1
                if mode == 1:
                    place.recreate += size
                elif mode == 2:
                    place.git_bytes += size
                else:
                    c = place.cls[EXT_CLASS.get(ext_of(name), "other")]
                    c[0] += 1
                    c[1] += size
                    if m > place.newest:
                        place.newest = m
                    if repo is not None:
                        s = self.repo_stats[repo["path"]]
                        s[0] += size
                        s[1] += 1
                        if m > s[2]:
                            s[2] = m

    def new_repo(self, path, place):
        r, last_commit = inspect_repo(path)
        r["place"] = place.label
        self.repos.append(r)
        self.repo_stats[path] = [0, 0, 0.0, last_commit]
        place.repos.append(r)
        return r

    # -------------------------------------------------- verdicts
    def protection(self, place):
        out, pn = [], norm(place.path)
        for p in self.protectors:
            for c in p["covers"]:
                if under(pn, norm(c["path"])):
                    state = "ok"
                    if c.get("age_days") is not None and c["age_days"] > OLD_COPY_DAYS:
                        state = "old"
                    elif p.get("running") is False or c.get("age_unknown"):
                        state = "check"         # set up, but not running or no date to go by
                    out.append({"by": p["name"], "how": c["how"], "state": state, "last_copy": c.get("last_copy"),
                                "age_days": c.get("age_days")})
        return out

    def judge(self):
        # A backup program usually covers everything; a sync program only its own folders.
        unknown = [p["name"] for p in self.protectors if p["coverage"] == "unknown" and p["kind"] != "sync"]
        maybe = [p["name"] for p in self.protectors if p["coverage"] == "unknown" and p["kind"] == "sync"]
        second = [d for d in self.drives if not d["system"] and d["type"] in ("fixed", "removable")]
        rows = []
        for i, pl in enumerate([p for p in self.places if p.measured and p.files], 1):
            own = pl.bytes - pl.recreate
            own_files = sum(v[0] for v in pl.cls.values())
            for r in pl.repos:
                s = self.repo_stats[r["path"]]
                r["bytes"], r["files"], r["newest_file"] = s[0], s[1], iso(s[2])
                r["changed_since_commit"] = bool(s[3] and s[2] > s[3] + 120)
            prot = self.protection(pl)
            program_like = pl.kind in ("drive", "profile") and own > 0 and \
                (pl.cls["programs"][1] + pl.cls["installers"][1]) >= own * 0.5
            work = sum(pl.cls[c][0] for c in ("photos", "documents", "videos", "audio", "code"))
            # a folder counts if it is one of your main or synced folders, or holds a real amount of your own files
            tiny = pl.kind not in ("known", "sync") and own < 10 * MB and not (work >= 25 and own >= 100 * 1024)
            counts = not program_like and not tiny and own_files > 0
            risky = [r for r in pl.repos if r["state"] in ("no-remote", "local-remote", "unpushed", "stash", "empty", "unknown")]
            safe_bytes = sum(r["bytes"] for r in pl.repos if r["state"] in ("ok", "differs"))
            if any(x["state"] == "ok" for x in prot):
                verdict = "ok"
            elif any(x["state"] == "old" for x in prot):
                verdict = "old"
            elif any(x["state"] == "check" for x in prot):
                verdict = "check"
            elif pl.repos and own > 0 and safe_bytes >= own * 0.8:
                verdict = "partial"
                prot.append({"by": "Git remotes", "state": "partial", "last_copy": None, "age_days": None,
                             "how": "%d of %d repositories here are in step with a remote. Anything not committed, and "
                                    "the rest, is only on this PC" % (len(pl.repos) - len(risky), len(pl.repos))})
            elif unknown:
                verdict = "check"
                prot.append({"by": ", ".join(unknown), "state": "check", "last_copy": None, "age_days": None,
                             "how": "installed, but this check can't see whether it covers this folder"})
            else:
                verdict = "none"
            pd = [d for d in second if d.get("backups")]
            hints = []
            base = os.path.basename(pl.path).lower()
            for d in pd:
                for b in d["backups"]:
                    if base and base in [n.lower() for n in b.get("_names", [])]:
                        hints.append({"drive": d["root"], "backup": b["name"], "newest": b["newest"], "same_disk": d.get("same_disk")})
            if pl.kind == "known":
                level = KNOWN_LEVEL[pl.label]
            elif pl.cls["photos"][0] + pl.cls["documents"][0] >= max(100, own_files * 0.25) or risky:
                level = 3
            elif program_like or (pl.cls["installers"][1] + pl.cls["archives"][1] + pl.cls["programs"][1]) >= max(1, own) * 0.6:
                level = 1
            else:
                level = 2
            what = [(pl.cls[c][0], one, many) for c, one, many in CLASS_WORDS if pl.cls[c][0]]
            what.sort(key=lambda t: -t[0])
            rows.append({
                "id": i, "label": pl.label, "path": pl.path, "kind": pl.kind, "bytes": pl.bytes, "own_bytes": own,
                "files": pl.files, "recreate_bytes": pl.recreate, "online_only_bytes": pl.cloud_bytes,
                "newest": iso(pl.newest), "classes": {c: {"files": v[0], "bytes": v[1]} for c, v in pl.cls.items() if v[0]},
                "what": ", ".join("%s %s" % ("{:,}".format(n), one if n == 1 else many) for n, one, many in what[:3]),
                "level": level, "level_words": LEVEL_WORDS[level], "protection": prot, "verdict": verdict,
                "counts": counts, "program_like": program_like, "truncated": pl.truncated,
                "repos": len(pl.repos), "repos_at_risk": len(risky), "linked_worktrees": pl.linked_worktrees,
                "maybe_on": hints[:3], "maybe_by": maybe if verdict == "none" else []})
            for r in pl.repos:
                r["place_id"], r["place_verdict"] = i, verdict
        return rows

    def run(self):
        self.find_protectors()
        self.look_at_drives()
        self.plan_places()
        deadline = self.t0 + self.args.max_seconds
        for pl in self.places:
            if time.time() < deadline:
                self.walk(pl, deadline)
            else:
                self.truncated = True
        self.pool.shutdown(wait=True)
        rows = self.judge()
        counted = [r for r in rows if r["counts"]]
        n = {v: len([r for r in counted if r["verdict"] == v]) for v in ("none", "old", "check", "partial", "ok")}
        at_risk = sum(r["own_bytes"] for r in counted if r["verdict"] in ("none", "old"))
        total = len(counted)
        if not total:
            headline = "No folders with your own files were found."
        elif n["none"]:
            headline = "%d of your %d folders ha%s no second copy" % (n["none"], total, "s" if n["none"] == 1 else "ve")
        elif n["old"]:
            headline = "%d of your %d folders ha%s only an old copy" % (n["old"], total, "s" if n["old"] == 1 else "ve")
        elif n["check"] or n["partial"]:
            headline = "None of your %d folders is plainly unprotected, but %d need%s a look" % (
                total, n["check"] + n["partial"], "s" if n["check"] + n["partial"] == 1 else "")
        else:
            headline = "All %d of your folders have a second copy" % total
        lose = []
        for r in counted:
            if r["verdict"] in ("none", "old", "partial", "check"):
                lose.append({"kind": "folder", "place": r["id"], "label": r["label"], "path": r["path"], "bytes": r["own_bytes"],
                             "level": r["level"], "level_words": r["level_words"], "verdict": r["verdict"], "what": r["what"],
                             "age_days": max([x["age_days"] or 0 for x in r["protection"]] or [0]) or None})
        order = {"none": 0, "old": 1, "partial": 2, "check": 3}
        lose.sort(key=lambda x: (order[x["verdict"]] > 1, -x["level"], order[x["verdict"]], -x["bytes"]))
        states = {}
        for r in self.repos:
            states[r["state"]] = states.get(r["state"], 0) + 1
        self.repos.sort(key=lambda r: (["no-remote", "local-remote", "unpushed", "stash", "empty", "unknown", "differs", "ok"].index(r["state"]),
                                       -r.get("bytes", 0)))
        for i, r in enumerate(self.repos, 1):
            r["id"] = i
        self.cant_see = [
            "Changes you haven't committed. This check reads Git's own files and never runs Git, so it can't see them. "
            "A repository marked 'files changed since the last commit' may hold some.",
            "Whether a sync program is paused, out of space or showing errors. Look at its icon in the taskbar.",
            "Whether a copy can really be restored. The only proof is opening a file from the copy.",
            "Other accounts on this PC, phones, and anything that lives only in a website or an app.",
        ]
        if NETWORK_FOLDERS:
            self.cant_see.append("%s %s kept on a network place, not on this PC. This check didn't look there. Ask whoever runs it "
                                 "how it is backed up." % (", ".join(NETWORK_FOLDERS), "is" if len(NETWORK_FOLDERS) == 1 else "are"))
        return {
            "schema": SCHEMA,
            "generated": datetime.now().isoformat(timespec="seconds"),
            "options": {"max_seconds": self.args.max_seconds, "test_root": self.test},
            "budget": {"entries_seen": self.entries, "seconds": round(time.time() - self.t0, 1), "truncated": self.truncated,
                       "errors": self.errors, "links_left_alone": self.links},
            "profile": self.env["profile"],
            "summary": dict(n, counted=total, bytes_at_risk=at_risk, headline=headline,
                            repos=len(self.repos), repo_states=states),
            "places": rows,
            "lose": lose,
            "protectors": self.protectors,
            "drives": [dict({k: v for k, v in d.items() if k not in ("disk", "backups")},
                            backups=[{k: v for k, v in bk.items() if k != "_names"} for bk in d.get("backups", [])])
                       for d in self.drives],
            "repos": self.repos,
            "not_measured": [p.path for p in self.places if not p.measured][:50],
            "left_out": self.left_out[:50],
            "cant_see": self.cant_see,
        }


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Read-only check of what has a second copy (backup-checkup skill).")
    ap.add_argument("--out", required=True, help="where to write the JSON report (the only file this script writes)")
    ap.add_argument("--root", help="check only this folder as if it were your profile (no registry, no real drives); for testing")
    ap.add_argument("--fake-registry", help="with --root: a JSON stand-in for the registry and running programs (testing)")
    ap.add_argument("--extra-drive", action="append", help="with --root: treat this folder as a second disk (testing)")
    ap.add_argument("--extra-partition", action="append",
                    help="with --root: treat this folder as another drive letter on the same disk (testing)")
    ap.add_argument("--max-seconds", type=float, default=240, help="stop measuring after this long (default 240)")
    ap.add_argument("--max-entries", type=int, default=6_000_000, help="stop after this many entries (default 6M)")
    ap.add_argument("--threads", type=int, default=min(4, os.cpu_count() or 2),
                    help="folder listings read at once (default: up to 4)")
    args = ap.parse_args(argv)
    if not args.root and (args.fake_registry or args.extra_drive or args.extra_partition):
        sys.exit("--fake-registry, --extra-drive and --extra-partition are for --root test runs only")
    if not IS_WIN and not args.root:
        sys.exit("This check is for Windows. Use --root <folder> to try it on a test folder.")
    out = os.path.abspath(args.out)
    if not os.path.isdir(os.path.dirname(out)):
        sys.exit("Output folder does not exist: %s (this script creates no folders)" % os.path.dirname(out))
    if args.root and not os.path.isdir(args.root):
        sys.exit("--root is not a folder: %s" % args.root)
    report = Survey(args).run()
    with open(out, "w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=1)
    b, s = report["budget"], report["summary"]
    print("check: %d entries in %.0fs%s; %s; %d repositories %s -> %s" % (
        b["entries_seen"], b["seconds"], " (TRUNCATED: raise --max-seconds)" if b["truncated"] else "", s["headline"],
        s["repos"], json.dumps(s["repo_states"]), out))


if __name__ == "__main__":
    main()

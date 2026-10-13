#!/usr/bin/env python3
"""Read-only survey of a Windows user's folders, for the tidy-folders skill.

Writes exactly one file: the JSON report named by --out. It never creates,
moves, renames or deletes anything else. File contents are read only for:
  * light keyword peeks into small .txt/.md/.rtf/.csv/.docx/.odt files that
    sit loose on the Desktop, in Downloads, etc. (turn off with --no-peek)
  * hashing same-size files to confirm duplicates (turn off with --no-hash)
Cloud-only OneDrive/Files On-Demand files are never read, so nothing downloads.

Python 3.8+, standard library only.

    python survey.py --out survey.json
    python survey.py --out survey.json --drives C,D --max-seconds 300
"""
import argparse
import hashlib
import heapq
import json
import os
import re
import socket
import sys
import time
import zipfile
from collections import deque
from datetime import datetime

IS_WIN = os.name == "nt"
if IS_WIN:
    import ctypes
    import winreg

SCHEMA = "tidy-folders/survey@1"

# Windows file attribute bits (from winnt.h)
A_HIDDEN, A_SYSTEM, A_REPARSE = 0x2, 0x4, 0x400
A_OFFLINE, A_RECALL_OPEN, A_RECALL_DATA = 0x1000, 0x40000, 0x400000
CLOUD_ONLY = A_OFFLINE | A_RECALL_OPEN | A_RECALL_DATA
TAG_MOUNT_POINT, TAG_SYMLINK = 0xA0000003, 0xA000000C  # junctions, symlinks

DAY = 86400.0
MIN_DUP_BYTES = 32 * 1024        # smaller duplicates aren't worth a line in the plan
PARTIAL_CHUNK = 64 * 1024        # size of one sample when comparing big files
FULL_HASH_MAX = 256 * 1024 ** 2  # bigger files are compared by 16 samples spread across the file
SAMPLES = 16
DUP_TYPES = {"documents", "images", "videos", "audio", "archives", "installers"}  # user content only
INSTALLER_MIN_AGE_DAYS = 30      # a month-old installer has done its job
GIANT_BYTES = 500 * 1024 ** 2
BIG_FILE_BYTES = 1024 ** 3
PEEK_MAX_FILES = 300
EXAMPLES = 8                     # examples kept per naming issue

TYPES = {
    "documents": "pdf doc docx odt rtf txt md pages xls xlsx xlsm ods csv numbers ppt pptx odp key epub mobi tex",
    "images": "jpg jpeg png gif bmp tif tiff webp heic heif raw cr2 cr3 nef arw dng svg psd",
    "videos": "mp4 mov mkv avi wmv webm m4v flv mts m2ts 3gp",
    "audio": "mp3 wav flac m4a aac ogg wma opus aiff aif mid",
    "archives": "zip rar 7z tar gz tgz bz2 xz zst cab",
    "installers": "exe msi msix msixbundle appx appxbundle iso img dmg pkg apk vhd vhdx",
    "code": "py js ts tsx jsx rs go java c cpp h hpp cs html css scss json yml yaml toml sh ps1 bat ipynb sql",
}
EXT_TYPE = {e: t for t, exts in TYPES.items() for e in exts.split()}
DOC_KIND_FOLDER = {
    "pdf": "PDFs", "doc": "Word documents", "docx": "Word documents", "odt": "Word documents",
    "rtf": "Word documents", "pages": "Word documents", "txt": "Notes", "md": "Notes",
    "xls": "Spreadsheets", "xlsx": "Spreadsheets", "xlsm": "Spreadsheets", "ods": "Spreadsheets",
    "csv": "Spreadsheets", "numbers": "Spreadsheets", "ppt": "Presentations", "pptx": "Presentations",
    "odp": "Presentations", "key": "Presentations", "epub": "E-books", "mobi": "E-books", "tex": "Notes",
}
DISK_IMAGE_EXTS = {"iso", "img", "vhd", "vhdx", "dmg"}

# Important papers: first matching category wins. Patterns run on the lower-cased
# file name with _ - . + replaced by spaces.
IMPORTANT = [
    ("Identity", r"passport|reisepass|personalausweis|id card|identity card|national id|driv\w* licen[cs]e|"
                 r"birth certificate|marriage certificate|divorce|\bvisa\b|residence (permit|card)|green card|"
                 r"social security|\bssn\b|naturali[sz]ation|citizenship"),
    ("Tax", r"\btax(es)?\b|tax return|\bw 2\b|\bw2\b|\b1099\b|\b1040\b|\bp60\b|\bp45\b|\bp11d\b|\bhmrc\b|\birs\b|"
            r"self assessment|vat return|steuer|imp[oô]t"),
    ("Housing", r"\blease\b|tenancy|rental agreement|\brent\b|mortgage|\bdeeds?\b|landlord|mietvertrag|council tax|utility bill"),
    ("Contracts", r"contract|agreement|\bnda\b|non ?disclosure|\bsigned\b|vertrag|power of attorney|letter of intent|"
                  r"\bmou\b|(stock|share) certificate|incorporation|bylaws|\bss ?4\b|trademark|patent|lawsuit|"
                  r"subpoena|court order"),
    ("Money", r"invoice|receipt|bank statement|account statement|card statement|payslip|pay slip|pay ?stub|salary|"
              r"\bbills?\b|rechnung|quittung|factura|facture|remittance|purchase order"),
    ("Insurance and health", r"insurance|versicherung|policy schedule|claim form|medical|prescription|vaccin|"
                             r"lab results|diagnosis|hospital|doctor"),
    ("Vehicle", r"\bv5c\b|car registration|vehicle registration|logbook"),
    ("Education and career", r"diploma|degree|transcript|certificate|\bcv\b|resume|résumé|cover letter|"
                             r"reference letter|offer letter"),
]
IMPORTANT_RX = [(c, re.compile(p)) for c, p in IMPORTANT]
IMPORTANT_EXTS = set("pdf doc docx odt rtf pages".split())  # .txt only counts through a content peek
SCAN_EXTS = set("jpg jpeg png heic tif tiff".split())         # photos/scans count for these categories only
SCAN_CATEGORIES = {"Identity", "Tax", "Money", "Insurance and health", "Vehicle"}
NOT_A_SCAN_RX = re.compile(r"@\dx|icon|logo|sprite|button|_(white|black|dark|light)|disabled|thumb", re.I)
CATEGORY_SYNONYMS = {
    "Identity": ["Identity", "ID", "IDs", "Passports", "Personal ID"],
    "Tax": ["Tax", "Taxes", "Tax returns"],
    "Housing": ["Housing", "Home", "House", "Rent", "Property"],
    "Contracts": ["Contracts", "Agreements", "Legal"],
    "Money": ["Money", "Finance", "Finances", "Invoices", "Receipts", "Bills", "Banking"],
    "Insurance and health": ["Insurance and health", "Insurance", "Health", "Medical"],
    "Vehicle": ["Vehicle", "Car", "Cars"],
    "Education and career": ["Education and career", "Career", "Education", "CV", "Work"],
}
# Content peeks: phrase -> (category, weight). A category needs a score of 2.
PEEK_PHRASES = {
    "this agreement is made": ("Contracts", 2), "in witness whereof": ("Contracts", 2),
    "the parties agree": ("Contracts", 1), "governing law": ("Contracts", 1), "hereinafter": ("Contracts", 1),
    "non-disclosure": ("Contracts", 1),
    "tenancy agreement": ("Housing", 2), "the landlord": ("Housing", 1), "the tenant": ("Housing", 1),
    "monthly rent": ("Housing", 1),
    "invoice number": ("Money", 2), "invoice no": ("Money", 2), "amount due": ("Money", 1),
    "total due": ("Money", 1), "payment terms": ("Money", 1), "net pay": ("Money", 2), "gross pay": ("Money", 1),
    "pay period": ("Money", 1), "receipt number": ("Money", 2),
    "tax year": ("Tax", 1), "taxable income": ("Tax", 2), "tax return": ("Tax", 2), "taxpayer": ("Tax", 1),
    "passport number": ("Identity", 2), "passport no": ("Identity", 2),
    "policy number": ("Insurance and health", 1), "policyholder": ("Insurance and health", 2),
    "patient name": ("Insurance and health", 2),
    "curriculum vitae": ("Education and career", 2), "work experience": ("Education and career", 1),
}
PEEK_TEXT_EXTS = {"txt", "md", "rtf", "csv"}
PEEK_ZIP_EXTS = {"docx": "word/document.xml", "odt": "content.xml"}

SCREENSHOT_RX = re.compile(r"screen ?shot|screen ?capture|^capture|snip|^scr[_ -]?\d", re.I)
RECORDING_RX = re.compile(r"recording|screen ?rec|^capture|clip", re.I)
YEAR_RX = re.compile(r"(?<!\d)(19[89]\d|20[0-4]\d)(?!\d)")
NAMING_RULES = [
    ("copy-number", re.compile(r"\(\d+\)(\.[^.]+)?$"), "ends in (1), (2)... (a second download or copy)"),
    ("copy-of", re.compile(r"^copy of |[ _-]copy( \(\d+\))?(\.[^.]+)?$", re.I), "'Copy of' / '- Copy'"),
    ("new-folder", re.compile(r"^new folder( \(\d+\))?$|^untitled|^new (text )?document", re.I), "'New folder' / 'Untitled'"),
    ("final", re.compile(r"final.*(final|v\d|\d)|\bfinal ?final\b", re.I), "'final final v3' style names"),
]

PROJECT_MARKERS = {".git", "package.json", "cargo.toml", "pyproject.toml", "go.mod", "pom.xml",
                   "build.gradle", "cmakelists.txt", "composer.json", "gemfile", "pubspec.yaml"}
SKIP_DIR_NAMES = {"node_modules", "__pycache__", ".venv", "venv", ".tox", "$recycle.bin",
                  "system volume information", "appdata"}
DRIVE_ROOT_SKIP = {"windows", "program files", "program files (x86)", "programdata", "users", "$recycle.bin",
                   "system volume information", "recovery", "perflogs", "config.msi", "windows.old",
                   "msocache", "$windows.~bt", "$windows.~ws", "$winreagent", "$sysreset", "onedrivetemp",
                   "inetpub", "esd", "boot", "documents and settings", "drivers", "swsetup", "intel", "amd",
                   "nvidia", "dell", "hp", "lenovo", "msi", "windowsapps", "wpsystem", "xboxgames",
                   "filehistory", "windowsimagebackup", "wudownloadcache", "windows10upgrade", "$getcurrent"}
GAME_FILE_MARKERS = {
    "unityplayer.dll": "Unity", "gameassembly.dll": "Unity", "steam_api.dll": "Steamworks",
    "steam_api64.dll": "Steamworks", "data.win": "GameMaker", "game.rgss3a": "RPG Maker",
    "game.rgss2a": "RPG Maker", "game.rgssad": "RPG Maker", "eossdk-win64-shipping.dll": "Epic Online Services",
    "galaxy.dll": "GOG Galaxy SDK", "galaxy64.dll": "GOG Galaxy SDK", "nw.dll": "NW.js (RPG Maker MV/MZ)",
}
ENGINE_SUBDIRS = {"binaries", "win64", "win32", "thirdparty", "steamworks", "x64", "x86", "x86_64", "bin",
                  "plugins", "engine", "managed", "redist", "_commonredist"}
GAMES_DIR_NAMES = {"games", "gaming", "spiele", "jeux", "juegos"}
# Profile folders that belong to Windows or to games' save data: never walked
PROFILE_SKIP = {"saved games", "searches", "links", "contacts", "favorites", "3d objects", "ansel", "tracing",
                "scoop", "npm-cache", "go", "miniconda3", "anaconda3", "pipx",
                "intelgraphicsprofiles", "microsoftedgebackups", "application data", "cookies", "local settings",
                "nethood", "printhood", "recent", "sendto", "start menu", "templates", "my documents"}
# Folders inside Documents (or the profile) that apps and games own: no empty-folder clean-up there
APP_OWNED_DIRS = {"my games", "windowspowershell", "powershell", "custom office templates", "outlook files",
                  "onenote notebooks", "fax", "scanned documents", "sound recordings", "iisexpress", "my web sites",
                  "sql server management studio", "zoom", "adobe", "rockstar games", "paradox interactive", "klei",
                  "electronic arts", "ubisoft", "bioware", "cd projekt red", "my received files", "wechat files",
                  "tencent files", "image-line", "native instruments", "ableton", "arduino", "github", "snagit",
                  "camtasia", "bandicam", "obs", "kingsoft", "savegames", "saves"}
PROJECTS_DIR_NAMES = ["Projects", "source\\repos", "code", "dev", "src", "repos"]

HOW_TO_MOVE = {
    "Steam": "Steam > Settings > Storage, pick the game, '...' > Move. Add a library on the new drive first.",
    "Xbox app": "Xbox app > the game > '...' > Manage > Files > Move.",
    "Epic Games": "Use the Epic launcher, not Explorer. If your launcher has no Move option, uninstall and "
                  "reinstall to the new folder.",
    "GOG Galaxy": "Use GOG Galaxy's own options for the game (Manage installation). If there is no Move option, "
                  "uninstall and reinstall to the new folder.",
    "Ubisoft Connect": "Use Ubisoft Connect, not Explorer. If there is no Move option, uninstall and reinstall.",
    "EA app": "Use the EA app, not Explorer. If there is no Move option, uninstall and reinstall.",
    "Riot": "Riot games can't be moved by hand; uninstall and reinstall to the new drive.",
    "Battle.net": "Use Battle.net's game settings, not Explorer. If there is no Move option, reinstall.",
}


def norm(p):
    return os.path.normcase(os.path.normpath(p))


def under(path_n, roots_n):
    """True if normalized path equals or sits below any normalized root."""
    for r in roots_n:
        if path_n == r or path_n.startswith(r.rstrip("\\/") + os.sep):
            return True
    return False


def iso(ts):
    try:
        return datetime.fromtimestamp(ts).strftime("%Y-%m-%d")
    except (OverflowError, OSError, ValueError):
        return None


def ext_of(name):
    i = name.rfind(".")
    return name[i + 1:].lower() if i > 0 else ""


def stem_tokens(name):
    i = name.rfind(".")
    stem = name[:i] if i > 0 else name
    return re.sub(r"[_\-.+]+", " ", stem.lower())


# ---------------------------------------------------------------- Windows bits
def reg_values(root, path, wow64=True):
    out = {}
    if not IS_WIN:
        return out
    for flag in ([winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY] if wow64 else [0]):
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


def reg_subkeys(root, path):
    names = set()
    if not IS_WIN:
        return []
    for flag in (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY):
        try:
            with winreg.OpenKey(root, path, 0, winreg.KEY_READ | flag) as k:
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


def expand(v):
    if not isinstance(v, str):
        return None
    return os.path.normpath(os.path.expandvars(v)) if v.strip() else None


def known_folders(root=None):
    if root:  # test / single-folder mode: treat `root` as the user profile
        out = {k: os.path.join(root, k) if os.path.isdir(os.path.join(root, k)) else None
               for k in ("Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music")}
        out["Profile"] = root
        return out
    home = os.path.expanduser("~")
    usf = reg_values(winreg.HKEY_CURRENT_USER,
                     r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders", False) if IS_WIN else {}
    keys = {"Desktop": "Desktop", "Documents": "Personal",
            "Downloads": "{374DE290-123F-4565-9164-39C4925E467B}", "Pictures": "My Pictures",
            "Videos": "My Video", "Music": "My Music"}
    out = {}
    for label, reg in keys.items():
        p = expand(usf.get(reg)) or os.path.join(home, label)
        out[label] = p if os.path.isdir(p) else None
    out["Profile"] = home
    for env in ("OneDrive", "OneDriveConsumer", "OneDriveCommercial"):
        if os.environ.get(env) and os.path.isdir(os.environ[env]):
            out["OneDrive"] = os.environ[env]
            break
    return out


def drive_list(only=None, include_removable=False):
    drives = []
    if not IS_WIN:
        return [{"root": "/", "type": "fixed"}]
    mask = ctypes.windll.kernel32.GetLogicalDrives()
    kinds = {2: "removable", 3: "fixed", 4: "network", 5: "cd", 6: "ramdisk"}
    for i in range(26):
        if not mask & (1 << i):
            continue
        letter = chr(65 + i)
        if only and letter not in only:
            continue
        root = letter + ":\\"
        kind = kinds.get(ctypes.windll.kernel32.GetDriveTypeW(root), "unknown")
        if kind == "fixed" or (include_removable and kind == "removable"):
            d = {"root": root, "type": kind}
            try:
                import shutil
                u = shutil.disk_usage(root)
                d.update(total=u.total, free=u.free)
            except OSError:
                pass
            drives.append(d)
    return drives


# ---------------------------------------------------------------- places other accounts can write to
# Steam's and Epic's own folders, the top of a drive, and wherever a registry value points can
# often be written by other accounts on the PC. A folder or file there can be swapped for a
# link, and a link can lead to another PC: listing or opening it would sign in to that PC as
# the person running the survey. So nothing in such a place is ever asked about by name alone.
# Every folder on the way is opened as itself (a link is opened as the link, never followed),
# checked to be a real folder, and kept open until the looking is done, which stops any of them
# being renamed, replaced or deleted meanwhile. Whatever can't be opened or examined, or isn't
# there, is "no".
_DRIVE_KINDS = {}
_WIN = {}


def on_this_pc(path):
    """True only for a full path, by its spelling, on a disk in this PC (built in, or plugged in)."""
    if not isinstance(path, str) or not re.match(r"^[A-Za-z]:\\", path) or not IS_WIN:
        return False
    root = path[:3].upper()
    if root not in _DRIVE_KINDS:
        import ctypes
        _DRIVE_KINDS[root] = ctypes.windll.kernel32.GetDriveTypeW(root)
    return _DRIVE_KINDS[root] in (2, 3)  # removable, fixed. Not a network drive, not unknown


def _win():
    if not _WIN:
        import ctypes
        from ctypes import wintypes

        class Tag(ctypes.Structure):  # FILE_ATTRIBUTE_TAG_INFO
            _fields_ = [("attributes", wintypes.DWORD), ("tag", wintypes.DWORD)]

        k = ctypes.WinDLL("kernel32", use_last_error=True)
        k.CreateFileW.restype = wintypes.HANDLE
        k.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.HANDLE]
        k.CloseHandle.argtypes = [wintypes.HANDLE]
        k.GetFileInformationByHandleEx.restype = wintypes.BOOL
        k.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        _WIN.update(k=k, Tag=Tag, byref=ctypes.byref, sizeof=ctypes.sizeof, none=wintypes.HANDLE(-1).value)
    return _WIN


def _opened(path, access, share):
    """(handle, is a folder, is a link) for `path` itself, or None if it can't be opened or examined.
    A link is opened as the link: nothing it leads to is touched."""
    w = _win()
    k = w["k"]
    # OPEN_EXISTING; FILE_FLAG_BACKUP_SEMANTICS (a folder can be opened) | FILE_FLAG_OPEN_REPARSE_POINT (a link is not followed)
    h = k.CreateFileW("\\\\?\\" + path, access, share, None, 3, 0x02000000 | 0x00200000, None)
    if h is None or h == w["none"]:
        return None
    info = w["Tag"]()
    if not k.GetFileInformationByHandleEx(h, 9, w["byref"](info), w["sizeof"](info)):  # 9: FileAttributeTagInfo
        k.CloseHandle(h)
        return None
    # A junction or a symlink is a reparse point whose tag says it stands for another name (bit 29)
    return h, bool(info.attributes & 0x10), bool(info.attributes & 0x400 and info.tag & 0x20000000)


def hold(path):
    """Handles that keep every folder from the drive down to `path` what and where it is, or None.
    None unless `path` is spelled as a full path on a disk in this PC and every part of it is a
    real folder: no part is a link, a file, missing, or impossible to open."""
    if not on_this_pc(path):
        return None
    handles, cur = [], path[:3]
    for part in [None] + [p for p in os.path.normpath(path)[3:].split("\\") if p]:
        if part:
            cur = os.path.join(cur, part)
        got = _opened(cur, 0x80, 0x1 | 0x2)  # FILE_READ_ATTRIBUTES; others may read and write in it, not rename or delete it
        if got is None or not got[1] or got[2]:
            release(handles + ([got[0]] if got else []))
            return None
        handles.append(got[0])
    return handles


def release(handles):
    for h in handles or []:
        _win()["k"].CloseHandle(h)


def a_folder_here(path):
    """True if `path` is a real folder on a disk in this PC, reached through no link."""
    held = hold(path)
    release(held)
    return held is not None


def listed(path):
    """The names in a folder, or None unless it is a real folder on a disk in this PC reached
    through no link. It and everything above it stay held while it is listed."""
    held = hold(path)
    if held is None:
        return None
    try:
        return os.listdir("\\\\?\\" + path)
    except OSError:
        return None
    finally:
        release(held)


def installed_programs(never=()):
    """InstallLocation of every registered program (read from the registry).

    `never` lists folders (profile, known folders) that a sloppy installer may
    have registered as its InstallLocation; those, their parents and drive roots
    are ignored so they can't hide the user's own files."""
    progs = []
    if not IS_WIN:
        return progs
    never_n = [norm(p) for p in never if p]
    base = r"Software\Microsoft\Windows\CurrentVersion\Uninstall"
    for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
        for sub in reg_subkeys(hive, base):
            v = reg_values(hive, base + "\\" + sub)
            loc = expand(v.get("InstallLocation"))
            if (loc and len(loc) > 3 and a_folder_here(loc) and os.path.dirname(norm(loc)) != norm(loc)
                    and not any(under(n, [norm(loc)]) for n in never_n)):
                progs.append({"name": v.get("DisplayName") or sub, "path": loc,
                              "publisher": v.get("Publisher") or "",
                              "bytes": int(v["EstimatedSize"]) * 1024 if isinstance(v.get("EstimatedSize"), int) else None})
    seen, out = set(), []
    for p in progs:
        if norm(p["path"]) not in seen:
            seen.add(norm(p["path"]))
            out.append(p)
    return out


def read_text(path, limit=2_000_000, folder_held=False):
    """The start of a small text file in a place other accounts may be able to write to, or "".
    Its folder is held as in hold() (or already is, by the caller: `folder_held`); the file is
    opened as itself and read only if it is a plain file. A file that is a link is never opened."""
    held = [] if folder_held else hold(os.path.dirname(path))
    if held is None:
        return ""
    try:
        got = _opened(path, 0x80000000, 0x1 | 0x2 | 0x4)  # GENERIC_READ; nobody else is kept out
        if got is None:
            return ""
        if got[1] or got[2]:
            release([got[0]])
            return ""
        import msvcrt
        with os.fdopen(msvcrt.open_osfhandle(got[0], os.O_RDONLY), "rb") as f:  # closing f closes the handle
            return f.read(limit).decode("utf-8", "replace")
    except OSError:
        return ""
    finally:
        release(held)


def launchers(drives):
    """Games installed through launchers. Read-only: registry, manifests, folder listings."""
    found = []

    # Steam: registry -> libraryfolders.vdf -> appmanifest_*.acf
    steam = None
    if IS_WIN:
        steam = expand((reg_values(winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam", False) or {}).get("SteamPath"))
    if steam and a_folder_here(steam):
        libs = {norm(steam): steam}
        for vdf in (os.path.join(steam, "steamapps", "libraryfolders.vdf"),
                    os.path.join(steam, "config", "libraryfolders.vdf")):
            for m in re.finditer(r'"path"\s+"([^"]+)"', read_text(vdf)):
                p = os.path.normpath(m.group(1).replace("\\\\", "\\"))
                if on_this_pc(p):  # a library it names on a share is never visited
                    libs.setdefault(norm(p), p)
        games, roots = [], []
        for lib in libs.values():
            sa = os.path.join(lib, "steamapps")
            held = hold(sa)  # None unless it is a real folder on this PC, reached through no link
            if held is None:
                continue
            try:  # held once: for the listing, and for every manifest read out of it
                names = os.listdir("\\\\?\\" + sa)
                roots.append(lib)
                for n in names:
                    if n.startswith("appmanifest_") and n.endswith(".acf"):
                        t = read_text(os.path.join(sa, n), 200_000, folder_held=True)
                        g = {k: (re.search(r'"%s"\s+"([^"]*)"' % k, t) or [None, None])[1]
                             for k in ("name", "installdir", "SizeOnDisk")}
                        if g["installdir"] and not re.search(r'[\\/:*?"<>|]', g["installdir"]):
                            games.append({"name": (g["name"] or g["installdir"])[:120],
                                          "path": os.path.join(sa, "common", g["installdir"]),
                                          "bytes": int(g["SizeOnDisk"]) if (g["SizeOnDisk"] or "").isdigit() else None})
            except OSError:
                pass
            finally:
                release(held)
        found.append({"name": "Steam", "libraries": roots, "games": games})

    # Epic: manifests in ProgramData
    epic_dir = os.path.join(os.environ.get("ProgramData", r"C:\ProgramData"), "Epic", "EpicGamesLauncher", "Data", "Manifests")
    held = hold(epic_dir)
    if held is not None:
        games, items = [], []
        try:  # held once: for the listing, and for every manifest read out of it
            for n in sorted(os.listdir("\\\\?\\" + epic_dir)):
                if n.endswith(".item"):
                    items.append((n, read_text(os.path.join(epic_dir, n), 2_000_000, folder_held=True)))
        except OSError:
            pass
        finally:
            release(held)
        for n, text in items:
            try:
                j = json.loads(text)
            except ValueError:
                continue
            if isinstance(j, dict) and on_this_pc(j.get("InstallLocation")):
                games.append({"name": str(j.get("DisplayName") or n)[:120], "path": os.path.normpath(j["InstallLocation"]),
                              "bytes": j.get("InstallSize") if isinstance(j.get("InstallSize"), int) else None})
        found.append({"name": "Epic Games", "libraries": sorted({os.path.dirname(g["path"]) for g in games}), "games": games})

    if IS_WIN:
        # GOG Galaxy and Ubisoft Connect register each game in HKLM
        for lname, key, namek, pathk in (("GOG Galaxy", r"SOFTWARE\GOG.com\Games", "gameName", "path"),
                                         ("Ubisoft Connect", r"SOFTWARE\Ubisoft\Launcher\Installs", None, "InstallDir")):
            games = []
            for sub in reg_subkeys(winreg.HKEY_LOCAL_MACHINE, key):
                v = reg_values(winreg.HKEY_LOCAL_MACHINE, key + "\\" + sub)
                p = expand(v.get(pathk))
                if p and a_folder_here(p):
                    games.append({"name": v.get(namek) if namek else os.path.basename(p.rstrip("\\/")),
                                  "path": p, "bytes": None})
            if games:
                found.append({"name": lname, "libraries": sorted({os.path.dirname(g["path"]) for g in games}), "games": games})

    # Folder-based launchers at drive roots
    for lname, folder in (("Xbox app", "XboxGames"), ("Riot", "Riot Games"), ("EA app", "EA Games")):
        games, libs = [], []
        for d in drives:
            base = os.path.join(d["root"], folder)
            names = listed(base)  # the top of a drive is a place anyone on the PC can make a folder, or a link, in
            if names is not None:
                libs.append(base)
                for n in sorted(names):
                    if n.lower() not in ("gamesave", "riot client") and a_folder_here(os.path.join(base, n)):
                        games.append({"name": n, "path": os.path.join(base, n), "bytes": None})
        if libs:
            found.append({"name": lname, "libraries": libs, "games": games})
    for f in found:
        f["how_to_move"] = HOW_TO_MOVE.get(f["name"], "Use the launcher's own move option; don't drag the folder.")
    return found


# ---------------------------------------------------------------- the survey
class Budget:
    def __init__(self, seconds, entries):
        self.t0, self.seconds, self.max_entries = time.time(), seconds, entries
        self.entries, self.truncated = 0, False

    def ok(self):
        if self.entries >= self.max_entries or time.time() - self.t0 > self.seconds:
            self.truncated = True
        return not self.truncated


class Survey:
    def __init__(self, args):
        self.args = args
        self.now = time.time()
        self.budget = Budget(args.max_seconds, args.max_entries)
        self.kf = known_folders(os.path.abspath(args.root) if args.root else None)
        if args.root:
            self.drives, self.programs, self.launchers = [], [], []
        else:
            self.drives = drive_list(set(args.drives.upper().replace(",", "")) if args.drives else None,
                                     args.include_removable)
            self.programs = installed_programs([self.kf.get(k) for k in
                                                ("Profile", "Desktop", "Documents", "Downloads", "Pictures", "Videos",
                                                 "Music", "OneDrive")])
            self.launchers = launchers(self.drives)
        # Protect each launcher game, Steam's steamapps folders and dedicated launcher folders, but
        # never a drive root or a folder that holds the user's own folders (an Epic game installed
        # straight into C:\SomeGame must not put all of C:\ off-limits).
        keep_out = [norm(self.kf[k]) for k in ("Profile", "Desktop", "Documents", "Downloads") if self.kf.get(k)]
        managed = [g["path"] for lch in self.launchers for g in lch["games"]]
        for lch in self.launchers:
            if lch["name"] == "Steam":
                managed += [os.path.join(lib, "steamapps") for lib in lch["libraries"]]
            elif lch["name"] in ("Xbox app", "Riot", "EA app"):
                managed += lch["libraries"]
        self.managed_games = [p for p in managed if os.path.dirname(norm(p)) != norm(p)
                              and not any(under(k, [norm(p)]) for k in keep_out)]
        env = os.environ
        self.protected = [p for p in {
            env.get("SystemRoot", r"C:\Windows"), env.get("ProgramFiles", r"C:\Program Files"),
            env.get("ProgramFiles(x86)", r"C:\Program Files (x86)"), env.get("ProgramData", r"C:\ProgramData"),
            env.get("LOCALAPPDATA"), env.get("APPDATA"), os.path.join(self.kf["Profile"], "AppData"),
        } if p]
        if args.root:  # an explicitly chosen folder may live under e.g. %TEMP%; keep only protections below it
            self.protected = [p for p in self.protected if not under(norm(args.root), [norm(p)])]
        self.protected_n = [norm(p) for p in self.protected]
        self.managed_n = [norm(p) for p in self.managed_games]
        self.programs_n = [norm(p["path"]) for p in self.programs]
        # Exact-match set: the walk meets every folder top-down, so an exact hit is enough there
        self.blocked_set = set(self.protected_n) | set(self.managed_n) | set(self.programs_n)
        self.areas, self.dirinfo = [], {}
        self.loose, self.important, self.installers, self.archives = [], [], [], []
        self.by_size, self.projects, self.game_roots = {}, [], {}
        self.big = []
        self.naming = {k: {"kind": k, "label": lbl, "count": 0, "examples": []} for k, _, lbl in NAMING_RULES}
        self.skipped, self.errors = [], 0

    # -------------------------------------------------- areas to walk
    def plan_areas(self):
        kf, seen = self.kf, set()

        def add(key, path, kind, loose=False):
            if path and os.path.isdir(path) and norm(path) not in seen:
                seen.add(norm(path))
                self.areas.append({"key": key, "path": path, "kind": kind, "loose": loose})

        for k in ("Desktop", "Downloads", "Documents", "Pictures", "Videos", "Music"):
            add(k, kf.get(k), "known", loose=k in ("Desktop", "Downloads", "Documents"))
        self.known_n = {norm(a["path"]) for a in self.areas}
        # Profile root: loose files only, then its other non-dot folders
        add("Profile", kf["Profile"], "profile-root", loose=True)
        try:
            with os.scandir(kf["Profile"]) as it:
                for e in it:
                    n = e.name
                    if (e.is_dir(follow_symlinks=False) and not n.startswith(".") and n.lower() not in SKIP_DIR_NAMES
                            and n.lower() not in PROFILE_SKIP and not self.reparse_skip(e) and not self.hidden_system(e)):
                        add(n, e.path, "profile")
        except OSError:
            self.errors += 1
        if kf.get("OneDrive"):
            add("OneDrive", kf["OneDrive"], "profile")
        for d in self.drives:
            add(d["root"], d["root"], "drive-root", loose=True)
            try:
                with os.scandir(d["root"]) as it:
                    for e in it:
                        if e.is_dir(follow_symlinks=False) and e.name.lower() not in DRIVE_ROOT_SKIP \
                                and not e.name.startswith(("$", ".")) and not self.reparse_skip(e) \
                                and not self.hidden_system(e):
                            add(e.path, e.path, "drive-top")
            except OSError:
                self.errors += 1
        # Areas never walk into each other, and none starts inside a protected folder
        self.areas = [a for a in self.areas if a["kind"] == "drive-root" or not self.blocked_dir(norm(a["path"]))]
        self.area_roots_n = {norm(a["path"]) for a in self.areas}

    # -------------------------------------------------- helpers
    @staticmethod
    def st(e):
        try:
            return e.stat(follow_symlinks=False)
        except OSError:
            return None

    def reparse_skip(self, e):
        s = self.st(e)
        if s is None:
            return True
        attrs = getattr(s, "st_file_attributes", 0)
        tag = getattr(s, "st_reparse_tag", 0)
        return bool(attrs & A_REPARSE) and tag in (TAG_MOUNT_POINT, TAG_SYMLINK) or e.is_symlink()

    def hidden_system(self, e):
        s = self.st(e)
        a = getattr(s, "st_file_attributes", 0) if s else 0
        return bool(a & A_HIDDEN and a & A_SYSTEM)

    def blocked_dir(self, path_n):
        return under(path_n, self.protected_n) or under(path_n, self.managed_n) or under(path_n, self.programs_n)

    # -------------------------------------------------- walk one area
    def walk(self, area):
        root = area["path"]
        stats = {"files": 0, "bytes": 0, "dirs": 0, "by_type": {}, "age": [0, 0, 0, 0, 0], "age_bytes": [0, 0, 0, 0, 0],
                 "loose_files": 0, "truncated": False, "children": {}}
        area["stats"] = stats
        only_files = area["kind"] in ("profile-root", "drive-root")
        # Breadth-first, so a folder cut short by the budget still shows its whole top level.
        # Big non-user folders (backups, datasets) get their own cap so they can't starve the rest.
        cap = None if area["kind"] == "known" else self.budget.entries + self.args.area_max_entries
        queue = deque([(root, 0, None)])
        while queue:
            if not self.budget.ok() or (cap and self.budget.entries > cap):
                stats["truncated"] = True
                break
            path, depth, top_child = queue.popleft()
            try:
                with os.scandir(path) as it:
                    entries = list(it)
            except OSError as ex:
                self.errors += 1
                if len(self.skipped) < 200:
                    self.skipped.append({"path": path, "reason": type(ex).__name__})
                continue
            self.budget.entries += len(entries)
            names = {e.name.lower() for e in entries}
            # dirinfo: [files here (-1 = never call it empty), bytes here, child folders, area key, depth]
            info = self.dirinfo.setdefault(path, [0, 0, [], area["key"], depth])
            if depth == 0 and area["kind"] in ("drive-top", "profile") and any(
                    n.endswith(".dll") or (n.startswith("unins") and n.endswith(".exe")) for n in names)                     and not self.detect_game(area, path, depth, names):
                stats["program_like"] = True  # an unregistered program or tool folder: leave it alone
                info[0] = -1
                continue
            is_game = self.detect_game(area, path, depth, names)
            # A project folder (git repo, node project...) is listed, not walked
            if not is_game and depth > 0 and (names & PROJECT_MARKERS or any(n.endswith(".sln") for n in names)):
                self.projects.append({"path": path, "area": area["key"], "depth": depth,
                                      "marker": sorted(names & PROJECT_MARKERS)[:1] or [".sln"]})
                info[0] = -1
                continue
            for e in entries:
                n = e.name
                s = self.st(e)
                if s is None:
                    self.errors += 1
                    continue
                attrs = getattr(s, "st_file_attributes", 0)
                if e.is_dir(follow_symlinks=False):
                    if only_files:
                        continue
                    child_n = norm(e.path)
                    if (n.lower() in SKIP_DIR_NAMES or n.startswith((".", "$")) or (attrs & A_HIDDEN and attrs & A_SYSTEM)
                            or self.reparse_skip(e) or child_n in self.area_roots_n or child_n in self.blocked_set):
                        info[0] = -1  # something we don't walk lives here: never call this folder empty
                        continue
                    info[2].append(e.path)
                    stats["dirs"] += 1
                    self.check_name(n, e.path)
                    queue.append((e.path, depth + 1, top_child or n))
                    continue
                size, mtime = s.st_size, s.st_mtime
                if info[0] >= 0:
                    info[0] += 1
                info[1] += size
                t = EXT_TYPE.get(ext_of(n), "other")
                bt = stats["by_type"].setdefault(t, [0, 0])
                bt[0] += 1
                bt[1] += size
                stats["files"] += 1
                stats["bytes"] += size
                age = (self.now - mtime) / DAY
                b = 0 if age < 30 else 1 if age < 180 else 2 if age < 365 else 3 if age < 1095 else 4
                stats["age"][b] += 1
                stats["age_bytes"][b] += size
                if top_child:
                    stats["children"][top_child] = stats["children"].get(top_child, 0) + size
                hidden = bool(attrs & (A_HIDDEN | A_SYSTEM))
                cloud = bool(attrs & CLOUD_ONLY)
                rec = {"path": e.path, "name": n, "bytes": size, "mtime": mtime, "type": t, "area": area["key"],
                       "kind": area["kind"], "depth": depth, "cloud": cloud}
                if not hidden:
                    self.check_name(n, e.path)
                    if depth == 0 and area["loose"]:
                        stats["loose_files"] += 1
                        self.loose.append(rec)
                    if t == "installers" and area["key"] == "Downloads" and depth == 0:
                        self.installers.append(rec)
                    if t == "archives" and area["key"] == "Downloads" and depth == 0:
                        rec["extracted"] = os.path.splitext(n)[0].lower() in names
                        self.archives.append(rec)
                    x = ext_of(n)
                    if x in IMPORTANT_EXTS or (x in SCAN_EXTS and not NOT_A_SCAN_RX.search(n)):
                        cat = self.classify_name(n)
                        if cat and (x in IMPORTANT_EXTS or cat in SCAN_CATEGORIES):
                            self.important.append(dict(rec, category=cat, matched="name"))
                    if size >= MIN_DUP_BYTES and not cloud and t in DUP_TYPES:
                        self.by_size.setdefault(size, []).append(rec)
                    if size >= BIG_FILE_BYTES:
                        heapq.heappush(self.big, (size, e.path, mtime))
                        if len(self.big) > 100:
                            heapq.heappop(self.big)

    def check_name(self, name, path):
        for kind, rx, _ in NAMING_RULES:
            if rx.search(name):
                d = self.naming[kind]
                d["count"] += 1
                if len(d["examples"]) < EXAMPLES:
                    d["examples"].append(path)
                break

    @staticmethod
    def classify_name(name):
        toks = stem_tokens(name)
        raw = name.lower()
        for cat, rx in IMPORTANT_RX:
            if rx.search(toks) or rx.search(raw):
                return cat
        return None

    def detect_game(self, area, path, depth, names):
        """Spot a game folder by its engine files. Returns True if `path` is inside a known game."""
        path_n = norm(path)
        if any(under(path_n, [g]) for g in self.game_roots):
            return True
        if ".itch" in names and path_n != norm(area["path"]):  # installed by the itch app
            self.game_roots[path_n] = {"path": path, "engine": "itch app", "area": area["key"], "managed_by": "itch app"}
            return True
        marker = next((GAME_FILE_MARKERS[n] for n in names if n in GAME_FILE_MARKERS), None)
        if not marker and any(n.startswith("goggame-") and n.endswith(".info") for n in names):
            marker = "GOG offline installer"
        if not marker and "engine" in names and any(n.endswith(".exe") for n in names):
            marker = "Unreal Engine"
        if not marker and {"renpy", "game"} <= names:
            marker = "Ren'Py"
        if not marker and any(n.endswith(".pck") and n[:-4] + ".exe" in names for n in names):
            marker = "Godot"
        if not marker:
            return False
        root, area_n = path, norm(area["path"])
        while norm(root) != area_n:
            base = os.path.basename(root).lower()
            if base in ENGINE_SUBDIRS or base.endswith("_data") or re.match(r"steamv\d+$", base):
                root = os.path.dirname(root)
            else:
                break
        root_n = norm(root)
        if root_n == area_n and area["kind"] != "drive-top":
            return False
        # Drop any earlier, deeper hit inside this root
        for g in [g for g in self.game_roots if under(g, [root_n])]:
            del self.game_roots[g]
        self.game_roots[root_n] = {"path": root, "engine": marker, "area": area["key"]}
        return True

    # -------------------------------------------------- post passes
    def subtree_bytes(self):
        sizes, visited = {}, set()
        for start in list(self.dirinfo):
            if start in visited:
                continue
            stack = [(start, False)]
            while stack:
                p, done = stack.pop()
                if done:
                    info = self.dirinfo[p]
                    sizes[p] = info[1] + sum(sizes.get(c, 0) for c in info[2])
                    continue
                if p in visited:
                    continue
                visited.add(p)
                stack.append((p, True))
                for c in self.dirinfo[p][2]:
                    if c not in visited and c in self.dirinfo:
                        stack.append((c, False))
        return sizes

    def empty_trees(self):
        """Top-most folders whose whole tree holds no files at all."""
        empty = {}
        for start in list(self.dirinfo):
            stack = [(start, False)]
            while stack:
                p, done = stack.pop()
                if p in empty:
                    continue
                info = self.dirinfo.get(p)
                if info is None:
                    empty[p] = False
                    continue
                if done:
                    empty[p] = info[0] == 0 and all(empty.get(c, False) for c in info[2])
                    continue
                stack.append((p, True))
                for c in info[2]:
                    if c not in empty:
                        stack.append((c, False))
        tops = []
        area_path = {a["key"]: a["path"] for a in self.areas}
        for p, is_empty in empty.items():
            if not is_empty or norm(p) in self.area_roots_n:
                continue
            parent = os.path.dirname(p)
            if empty.get(parent) and norm(parent) not in self.area_roots_n:
                continue
            info = self.dirinfo[p]
            rel = os.path.relpath(p, area_path[info[3]]).split(os.sep)
            first = rel[0].lower()
            if first in APP_OWNED_DIRS or "game" in first or "save" in first:
                continue  # apps and games keep their own (sometimes empty) folders here
            tops.append((p, info[3]))
        return sorted(tops)

    def peek(self, rec):
        e = ext_of(rec["name"])
        text = ""
        try:
            if e in PEEK_TEXT_EXTS and rec["bytes"] <= 2_000_000:
                with open(rec["path"], "r", encoding="utf-8", errors="replace") as f:
                    text = f.read(32_768)
            elif e in PEEK_ZIP_EXTS and rec["bytes"] <= 20_000_000:
                with zipfile.ZipFile(rec["path"]) as z:
                    with z.open(PEEK_ZIP_EXTS[e]) as f:
                        text = re.sub(r"<[^>]+>", " ", f.read(262_144).decode("utf-8", "replace"))
        except (OSError, zipfile.BadZipFile, KeyError, RuntimeError, ValueError):
            return None
        text = re.sub(r"\s+", " ", text.lower())
        score = {}
        for phrase, (cat, w) in PEEK_PHRASES.items():
            if phrase in text:
                score[cat] = score.get(cat, 0) + w
        best = max(score.items(), key=lambda kv: kv[1]) if score else None
        return best[0] if best and best[1] >= 2 else None

    def hash_dups(self):
        """Group same-size files by content. Files up to 256 MB are hashed whole; bigger ones by 16 samples."""
        budget = self.args.hash_budget_mb * 1024 * 1024
        deadline = time.time() + self.args.hash_seconds
        used, groups = 0, []
        # Most space to win first; a big file costs at most SAMPLES * PARTIAL_CHUNK to compare
        cands = sorted(((s, recs) for s, recs in self.by_size.items() if len(recs) > 1),
                       key=lambda kv: -kv[0] * (len(kv[1]) - 1))

        def cost(size, full):
            return size if full else min(size, SAMPLES * PARTIAL_CHUNK)

        def digest(path, size, full):
            h = hashlib.sha256()
            with open(path, "rb") as f:
                if full:
                    for chunk in iter(lambda: f.read(1 << 20), b""):
                        h.update(chunk)
                else:
                    step = max(1, (size - PARTIAL_CHUNK) // (SAMPLES - 1))
                    for i in range(SAMPLES):
                        f.seek(min(i * step, max(0, size - PARTIAL_CHUNK)))
                        h.update(f.read(PARTIAL_CHUNK))
            return h.hexdigest()

        for size, recs in cands:
            if time.time() > deadline or used >= budget:
                self.hash_truncated = True
                break
            recs = [r for r in recs if not self.excluded(r["path"])]
            if len(recs) < 2:
                continue
            full = size <= FULL_HASH_MAX
            by_hash = {}
            for r in recs:
                if used + cost(size, full) > budget:
                    self.hash_truncated = True
                    break
                try:
                    by_hash.setdefault(digest(r["path"], size, full), []).append(r)
                    used += cost(size, full)
                except OSError:
                    self.errors += 1
            for same in by_hash.values():
                if len(same) > 1:
                    groups.append({"bytes": size, "method": "sha256" if full else "sampled", "files": same})
        self.hash_bytes = used
        return groups

    def excluded(self, path):
        """Inside a game folder? (Projects and protected folders are never walked, so nothing in them is listed.)"""
        p = norm(path)
        return any(under(p, [g]) for g in self.game_roots)

    # -------------------------------------------------- suggestions
    def find_child(self, base, names):
        if not base or not os.path.isdir(base):
            return None
        try:
            existing = {n.lower(): n for n in os.listdir(base) if os.path.isdir(os.path.join(base, n))}
        except OSError:
            return None
        for n in names:
            if n.lower() in existing:
                return os.path.join(base, existing[n.lower()])
        return None

    def propose(self, dup_groups):
        docs = self.kf.get("Documents") or os.path.join(self.kf["Profile"], "Documents")
        pics = self.kf.get("Pictures") or os.path.join(self.kf["Profile"], "Pictures")
        vids = self.kf.get("Videos") or os.path.join(self.kf["Profile"], "Videos")
        music = self.kf.get("Music") or os.path.join(self.kf["Profile"], "Music")
        important_root = self.find_child(docs, ["Important", "Important documents", "Paperwork", "Papers",
                                                "Admin", "Personal"]) or os.path.join(docs, "Important")
        targets = {"Documents": docs, "Important": important_root, "Pictures": pics, "Videos": vids, "Music": music}
        out, claimed, planned = [], set(), set()
        nid = [0]

        def dest(folder, name):
            base, ext = os.path.splitext(name)
            cand, i = os.path.join(folder, name), 2
            while norm(cand) in planned or os.path.exists(cand):
                cand = os.path.join(folder, "%s (%d)%s" % (base, i, ext))
                i += 1
            planned.add(norm(cand))
            return cand

        def add(group, op, src, to=None, why="", default=True, risk="low", nbytes=0, extra=None):
            if norm(src) in claimed:
                return
            claimed.add(norm(src))
            nid[0] += 1
            s = {"id": nid[0], "group": group, "op": op, "from": src, "bytes": nbytes, "why": why,
                 "default": default, "risk": risk}
            if to:
                s["to"] = to
            if extra:
                s.update(extra)
            out.append(s)

        loose_zone = {"Desktop", "Downloads", "Profile"}

        def is_loose(r):
            return r["depth"] == 0 and (r["area"] in loose_zone or r["kind"] == "drive-root")

        # 1. Duplicates: only copies lying in messy places (Downloads, Desktop, profile or drive root) are
        #    offered to the Recycle Bin. Copies anywhere else (a backup, a library) stay and are just reported.
        rank = {"Documents": 0, "Pictures": 0, "Videos": 0, "Music": 0, "profile": 1, "drive-top": 1,
                "Desktop": 2, "Profile": 3, "drive-root": 3, "Downloads": 4}

        def messy(r):
            return r["area"] in ("Downloads", "Desktop", "Profile") or r["kind"] == "drive-root"

        for g in dup_groups:
            files = sorted(g["files"], key=lambda r: (rank.get(r["area"], rank.get(r["kind"], 1)), r["mtime"],
                                                      bool(NAMING_RULES[0][1].search(r["name"])), len(r["path"])))
            keep = files[0]
            sure = g["method"] == "sha256"
            for r in files[1:]:
                if not messy(r):
                    continue
                add("duplicates", "recycle", r["path"],
                    why="%s the copy kept in %s" % ("Same content as" if sure else "Same size and same 16 samples as",
                                                   os.path.dirname(keep["path"])),
                    default=sure, risk="low" if sure else "medium", nbytes=r["bytes"], extra={"keep": keep["path"]})

        # 2. Important papers lying loose
        peeked = 0
        imp_paths = {norm(r["path"]) for r in self.important}
        if self.args.peek:
            for r in self.loose:
                if peeked >= PEEK_MAX_FILES:
                    break
                if norm(r["path"]) in imp_paths or r["cloud"] or ext_of(r["name"]) not in (PEEK_TEXT_EXTS | set(PEEK_ZIP_EXTS)):
                    continue
                peeked += 1
                cat = self.peek(r)
                if cat:
                    self.important.append(dict(r, category=cat, matched="content"))
        self.peeked = peeked
        for r in self.important:
            if not (is_loose(r) or (r["area"] == "Documents" and r["depth"] == 0)):
                continue
            m = YEAR_RX.search(r["name"])
            year = m.group(1) if m else (iso(r["mtime"]) or "0000")[:4]
            syn = CATEGORY_SYNONYMS[r["category"]]  # reuse the user's own folder if they already have one
            folder = self.find_child(important_root, syn) or self.find_child(docs, syn) or \
                os.path.join(important_root, r["category"])
            if r["category"] in ("Tax", "Money"):
                folder = os.path.join(folder, year)
            add("important", "move", r["path"], dest(folder, r["name"]),
                "Looks like %s (%s)" % (r["category"].lower(), "file name" if r["matched"] == "name" else "content"),
                nbytes=r["bytes"], extra={"category": r["category"]})

        # 3. Old installers and already-extracted archives in Downloads
        for r in self.installers:
            age = (self.now - r["mtime"]) / DAY
            if age < INSTALLER_MIN_AGE_DAYS:
                continue
            e = ext_of(r["name"])
            giant = r["bytes"] >= GIANT_BYTES
            add("installers", "recycle", r["path"],
                why="%s, %d days old%s" % ("Disk image" if e in DISK_IMAGE_EXTS else "Installer", age,
                                           ", over 500 MB" if giant else ""),
                default=e not in DISK_IMAGE_EXTS, risk="medium" if e in DISK_IMAGE_EXTS else "low", nbytes=r["bytes"])
        for r in self.archives:
            if r.get("extracted") and (self.now - r["mtime"]) / DAY >= INSTALLER_MIN_AGE_DAYS:
                add("archives", "recycle", r["path"], why="Already extracted next to it", nbytes=r["bytes"])

        # 4. Loose files to their library
        for r in self.loose:
            if not is_loose(r) or r["name"].startswith("."):
                continue
            e, t = ext_of(r["name"]), r["type"]
            year = (iso(r["mtime"]) or "0000")[:4]
            src_label = r["area"] if r["kind"] != "drive-root" else "drive root"
            if t == "documents" and e in DOC_KIND_FOLDER:
                add("documents", "move", r["path"], dest(os.path.join(docs, DOC_KIND_FOLDER[e]), r["name"]),
                    "Document loose in " + src_label, nbytes=r["bytes"])
            elif t == "images":
                sub = "Screenshots" if SCREENSHOT_RX.search(r["name"]) else year
                add("pictures", "move", r["path"], dest(os.path.join(pics, sub), r["name"]),
                    "Picture loose in " + src_label, nbytes=r["bytes"])
            elif t == "videos":
                sub = "Screen recordings" if RECORDING_RX.search(r["name"]) else year
                add("videos", "move", r["path"], dest(os.path.join(vids, sub), r["name"]),
                    "Video loose in " + src_label, nbytes=r["bytes"])
            elif t == "audio":
                add("music", "move", r["path"], dest(music, r["name"]), "Audio loose in " + src_label, nbytes=r["bytes"])

        # 5. Loose games (not managed by a launcher) into a Games folder on the same drive
        sizes = self.subtree_bytes()
        for g in self.game_roots.values():
            p = g["path"]
            parts = [x.lower() for x in os.path.normpath(p).split(os.sep)]
            g["bytes"] = sizes.get(p)
            g["in_games_folder"] = any(x in GAMES_DIR_NAMES for x in parts[:-1])
            if g["in_games_folder"] or g.get("managed_by"):
                continue
            base = self.kf["Profile"] if self.args.root else os.path.splitdrive(p)[0] + os.sep  # same drive: instant move
            games_dir = self.find_child(base, ["Games", "Gaming"]) or os.path.join(base, "Games")
            add("games", "move", p, dest(games_dir, os.path.basename(p)),
                "%s game outside a Games folder. Shortcuts and some saves may need fixing after the move." % g["engine"],
                default=False, risk="medium", nbytes=g["bytes"] or 0, extra={"is_dir": True})

        # 6. Code projects lying on the Desktop / in Downloads / at the profile root
        proj_root = None
        for cand in PROJECTS_DIR_NAMES:
            c = os.path.join(self.kf["Profile"], cand)
            if os.path.isdir(c):
                proj_root = c
                break
        proj_root = proj_root or os.path.join(self.kf["Profile"], "Projects")
        for pr in self.projects:
            if pr["area"] in ("Desktop", "Downloads") and pr["depth"] == 1:
                add("projects", "move", pr["path"], dest(proj_root, os.path.basename(pr["path"])),
                    "Code project " + ("on the Desktop" if pr["area"] == "Desktop" else "in Downloads"), default=False, risk="medium", extra={"is_dir": True})

        # 7. Empty folders, only inside the user's own folders (tool and program folders keep theirs)
        user_areas = {"Desktop", "Downloads", "Pictures", "Videos", "Music", "Documents"}
        for p, area_key in self.empty_trees():
            if area_key in user_areas and not self.excluded(p):
                low = area_key != "Documents"
                add("empty", "remove-empty-dir", p, why="Empty folder (nothing inside, at any depth)",
                    default=low, risk="low" if low else "medium", extra={"is_dir": True})

        return out, targets, proj_root

    # -------------------------------------------------- run
    def run(self):
        self.plan_areas()
        for a in self.areas:
            if not self.budget.ok():
                a["stats"] = {"files": 0, "bytes": 0, "dirs": 0, "by_type": {}, "age": [0] * 5, "age_bytes": [0] * 5,
                              "loose_files": 0, "truncated": True, "children": {}}
                continue
            self.walk(a)
        # Things inside games/projects/protected aren't loose clutter
        self.loose = [r for r in self.loose if not self.excluded(r["path"])]
        self.important = [r for r in self.important if not self.excluded(r["path"])]
        self.installers = [r for r in self.installers if not self.excluded(r["path"])]
        dup_groups = self.hash_dups() if self.args.hash else []
        if not self.args.hash:
            self.hash_bytes = 0
        suggestions, targets, proj_root = self.propose(dup_groups)
        return self.report(suggestions, targets, proj_root, dup_groups)

    def report(self, suggestions, targets, proj_root, dup_groups):
        def slim(r, *extra):
            d = {"path": r["path"], "bytes": r["bytes"], "modified": iso(r["mtime"])}
            for k in extra:
                if k in r:
                    d[k] = r[k]
            return d

        areas = []
        for a in self.areas:
            s = a["stats"]
            kids = sorted(s["children"].items(), key=lambda kv: -kv[1])[:12]
            areas.append({"key": a["key"], "path": a["path"], "kind": a["kind"], "files": s["files"],
                          "bytes": s["bytes"], "dirs": s["dirs"], "loose_files": s["loose_files"],
                          "truncated": s["truncated"], "program_like": s.get("program_like", False),
                          "by_type": {t: {"files": v[0], "bytes": v[1]} for t, v in s["by_type"].items()},
                          "age": {"files": s["age"], "bytes": s["age_bytes"],
                                  "buckets": ["< 1 month", "1-6 months", "6-12 months", "1-3 years", "> 3 years"]},
                          "biggest_children": [{"name": k, "bytes": v} for k, v in kids]})
        return {
            "schema": SCHEMA,
            "generated": datetime.now().isoformat(timespec="seconds"),
            "host": {"computer": socket.gethostname(), "user": os.environ.get("USERNAME") or os.environ.get("USER"),
                     "profile": self.kf["Profile"], "python": sys.version.split()[0]},
            "options": {"max_seconds": self.args.max_seconds, "max_entries": self.args.max_entries,
                        "peek": self.args.peek, "hash": self.args.hash, "hash_budget_mb": self.args.hash_budget_mb},
            "budget": {"entries_seen": self.budget.entries, "seconds": round(time.time() - self.budget.t0, 1),
                       "truncated": self.budget.truncated, "hashed_bytes": self.hash_bytes,
                       "hash_truncated": getattr(self, "hash_truncated", False),
                       "content_peeks": getattr(self, "peeked", 0), "errors": self.errors},
            "known_folders": self.kf,
            "targets": dict(targets, Projects=proj_root),
            "drives": self.drives,
            "areas": areas,
            "games": {"launchers": self.launchers,
                      "loose": sorted(self.game_roots.values(), key=lambda g: -(g.get("bytes") or 0))},
            "installed_programs": {"count": len(self.programs),
                                   "outside_program_files": [p for p in self.programs
                                                             if not under(norm(p["path"]), self.protected_n)][:200]},
            "important_docs": [slim(r, "category", "matched") for r in self.important][:5000],
            "duplicates": [{"bytes": g["bytes"], "method": g["method"], "paths": [r["path"] for r in g["files"]]}
                           for g in sorted(dup_groups, key=lambda g: -g["bytes"] * (len(g["files"]) - 1))][:2000],
            "installers": [slim(r) for r in sorted(self.installers, key=lambda r: -r["bytes"])][:1000],
            "big_files": [{"path": p, "bytes": s, "modified": iso(m)} for s, p, m in sorted(self.big, reverse=True)],
            "projects": [{"path": p["path"], "area": p["area"], "marker": p["marker"][0]} for p in self.projects][:1000],
            "naming_issues": [v for v in self.naming.values() if v["count"]],
            "protected": self.protected + ["Launcher game libraries and every registered program's install folder"],
            "skipped": self.skipped,
            "suggestions": suggestions,
        }


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")  # file names can hold characters the console can't show
    ap = argparse.ArgumentParser(description="Read-only survey of a Windows user's folders (tidy-folders skill).")
    ap.add_argument("--out", required=True, help="where to write the JSON report (the only file this script writes)")
    ap.add_argument("--drives", help="drive letters to include, e.g. C,D (default: all fixed drives)")
    ap.add_argument("--include-removable", action="store_true", help="also survey USB/removable drives")
    ap.add_argument("--root", help="survey only this folder as if it were a user profile (no drives, registry or "
                                   "launchers); for testing or a single messy folder")
    ap.add_argument("--max-seconds", type=float, default=240, help="stop walking after this long (default 240)")
    ap.add_argument("--max-entries", type=int, default=2_000_000, help="stop after this many entries (default 2M)")
    ap.add_argument("--hash-budget-mb", type=int, default=2048, help="max MB read to confirm duplicates (default 2048)")
    ap.add_argument("--hash-seconds", type=float, default=60, help="max seconds spent confirming duplicates (default 60)")
    ap.add_argument("--area-max-entries", type=int, default=150_000,
                    help="cap per folder outside the user's own folders, e.g. a backup drive (default 150000)")
    ap.add_argument("--no-hash", dest="hash", action="store_false", help="don't read files to confirm duplicates")
    ap.add_argument("--no-peek", dest="peek", action="store_false", help="don't peek inside small documents")
    args = ap.parse_args(argv)
    out = os.path.abspath(args.out)
    if not os.path.isdir(os.path.dirname(out)):
        sys.exit("Output folder does not exist: %s (this script creates no folders)" % os.path.dirname(out))
    report = Survey(args).run()
    with open(out, "w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=1)
    b = report["budget"]
    groups = {}
    for s in report["suggestions"]:
        groups[s["group"]] = groups.get(s["group"], 0) + 1
    print("survey: %d entries in %.0fs%s, %d suggestions %s -> %s" % (
        b["entries_seen"], b["seconds"], " (TRUNCATED: raise --max-seconds)" if b["truncated"] else "",
        len(report["suggestions"]), json.dumps(groups), out))


if __name__ == "__main__":
    main()

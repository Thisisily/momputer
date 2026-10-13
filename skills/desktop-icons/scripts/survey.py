#!/usr/bin/env python3
"""Read-only survey of a Windows desktop, for the desktop-icons skill.

Writes exactly one file: the JSON report named by --out. It never creates, moves, renames or
deletes anything, and it never runs or resolves a shortcut. It looks at:
  * the person's own Desktop folder, wherever it really is (it may be inside OneDrive)
  * the Public Desktop, which it only reports on
  * taskbar pins and Start menu shortcuts, to tell which desktop shortcuts are also there
  * the desktop window itself: how many icons it shows and how far apart (two questions, no changes)
The only file contents it reads are shortcut files (.lnk and .url, at most 256 KB of each).
A shortcut's target is checked by asking whether that name exists, and only on a fixed disk in
this PC: network paths, removable drives and anything it can't parse are "unknown", never "dead".

Python 3.8+, standard library only.

    python survey.py --out survey.json
    python survey.py --out survey.json --root <folder>     # test mode: <folder> stands in for the profile
"""
import argparse
import hashlib
import json
import os
import re
import struct
import sys
import time
import unicodedata
from datetime import datetime
from urllib.parse import unquote, urlsplit

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
from safe_apply import known_folders, norm, under  # noqa: E402  (the same idea of "your folders" as the apply step)

IS_WIN = os.name == "nt"
SCHEMA = "desktop-icons/survey@1"
DAY = 86400.0
MAX_SHORTCUT_BYTES = 256 * 1024   # a .lnk is a few KB; nothing past this is read
MAX_ELSEWHERE = 3000              # Start menu and taskbar shortcuts looked at
FOLDER_ENTRY_CAP = 5000           # entries counted inside one desktop folder
STALE_DAYS = 30                   # untouched this long: ticked by default
A_HIDDEN, A_SYSTEM, A_REPARSE = 0x2, 0x4, 0x400
CLOUD_ONLY = 0x1000 | 0x40000 | 0x400000
LINK_TAGS = (0xA0000003, 0xA000000C)
LNK_CLSID = bytes.fromhex("0114020000000000c000000000000046")

TYPES = {
    "documents": "pdf doc docx odt rtf txt md pages xls xlsx xlsm ods csv numbers ppt pptx odp key epub mobi tex one",
    "pictures": "jpg jpeg png gif bmp tif tiff webp heic heif raw cr2 cr3 nef arw dng svg psd ico",
    "videos": "mp4 mov mkv avi wmv webm m4v flv mts m2ts 3gp",
    "audio": "mp3 wav flac m4a aac ogg wma opus aiff aif mid",
    "archives": "zip rar 7z tar gz tgz bz2 xz zst cab",
    "installers": "msi msix msixbundle appx appxbundle iso img dmg pkg apk vhd vhdx",
    "programs": "exe com bat cmd ps1 vbs js jar scr msc",
}
EXT_KIND = {e: k for k, exts in TYPES.items() for e in exts.split()}
KIND_LABEL = {"shortcut": "Shortcuts", "web-link": "Web and game links", "documents": "Documents", "pictures": "Pictures",
              "videos": "Videos", "audio": "Audio", "archives": "Archives", "installers": "Installers", "programs": "Program files",
              "folder": "Folders", "other": "Other files"}
SINGULAR = {"documents": "document", "pictures": "picture", "videos": "video", "audio": "audio file", "archives": "archive",
            "installers": "installer", "programs": "program file", "other": "file"}
SCREENSHOT_RX = re.compile(r"screen ?shot|screen ?capture|^capture|snip|^scr[_ -]?\d", re.I)
INSTALLER_NAME_RX = re.compile(r"setup|install|\bupdate|upgrade|x64|x86|win(32|64)|\d+\.\d+", re.I)
COPY_NAME_RX = re.compile(r"( - copy| - shortcut| \(\d+\)|^copy of )", re.I)
PROJECT_MARKERS = {".git", "package.json", "cargo.toml", "pyproject.toml", "go.mod", "pom.xml", "build.gradle",
                   "cmakelists.txt", "composer.json", "gemfile", "pubspec.yaml"}
# Programs that run whatever their arguments say. A shortcut to one, with arguments, is worth a look.
COMMAND_HOSTS = {"cmd.exe", "powershell.exe", "pwsh.exe", "wscript.exe", "cscript.exe", "mshta.exe", "rundll32.exe",
                 "regsvr32.exe", "certutil.exe", "bitsadmin.exe", "msiexec.exe", "forfiles.exe", "conhost.exe", "wmic.exe",
                 "curl.exe", "ftp.exe", "bash.exe", "wsl.exe", "schtasks.exe", "reg.exe"}
# Launchers many programs share: two shortcuts to "Update.exe" in different folders are two programs, not one installed twice
GENERIC_EXES = COMMAND_HOSTS | {"update.exe", "launcher.exe", "setup.exe", "start.exe", "run.exe", "javaw.exe", "java.exe",
                                "python.exe", "pythonw.exe", "explorer.exe", "uninstall.exe", "app.exe", "main.exe",
                                "game.exe", "electron.exe", "node.exe", "steam.exe"}
GAME_SCHEMES = {"steam": "Steam", "com.epicgames.launcher": "Epic Games", "uplay": "Ubisoft Connect", "origin": "EA app",
                "origin2": "EA app", "battlenet": "Battle.net", "goggalaxy": "GOG Galaxy", "ms-xbl": "Xbox", "itch": "itch"}
PRETEND_EXTS = set("pdf doc docx xls xlsx ppt pptx txt rtf jpg jpeg png gif mp3 mp4 zip csv".split())
LIBRARY_FOLDER = "From the Desktop"
DESKTOP_FOLDERS = {"documents": "Documents", "pictures": "Pictures", "videos": "Videos", "audio": "Music",
                   "archives": "Installers and archives", "installers": "Installers and archives",
                   "programs": "Other files", "other": "Other files", "folder": "Folders"}
AGE_BUCKETS = ["< 1 month", "1-6 months", "6-12 months", "1-3 years", "> 3 years"]


def iso(ts):
    try:
        return datetime.fromtimestamp(ts).strftime("%Y-%m-%d")
    except (OverflowError, OSError, ValueError):
        return None


def ext_of(name):
    i = name.rfind(".")
    return name[i + 1:].lower() if i > 0 else ""


def writable_text(name):
    """True if a name can go into the JSON report as it is (a lone surrogate can't be written as UTF-8)."""
    try:
        name.encode("utf-8")
        return True
    except UnicodeEncodeError:
        return False


def spelled_out(text):
    """`text` with anything invisible or direction-changing written as \\uXXXX, so a name such as
    "photo<right-to-left override>gpj.exe" can't pass itself off as "photoexe.jpg" on the page.
    (Zero-width joiners between letters stay: Persian and some Indic words are spelled with them.)"""
    out = []
    for i, ch in enumerate(text):
        if unicodedata.category(ch) in ("Cc", "Cf", "Cs", "Co", "Cn", "Zl", "Zp") and not (
                ch in "‌‍" and 0 < i < len(text) - 1 and text[i - 1].isalpha() and text[i + 1].isalpha()):
            out.append("\\u%04x" % ord(ch))
        else:
            out.append(ch)
    return "".join(out)


# ---------------------------------------------------------------- shortcut files
def parse_lnk(data, ansi=None):
    """What a Shell Link (.lnk; the format is Microsoft's MS-SHLLINK) says about its target, read
    straight from its bytes. Raises ValueError for anything that isn't a complete, readable link."""
    ansi = ansi or ("mbcs" if IS_WIN else "cp1252")
    try:
        if len(data) < 76 or struct.unpack_from("<I", data, 0)[0] != 0x4C or data[4:20] != LNK_CLSID:
            raise ValueError("not a shell link")
        flags = struct.unpack_from("<I", data, 20)[0]
        out = {"local": None, "network": False, "env": None, "relative": None, "workdir": None, "args": "",
               "installer": False, "id_list": bool(flags & 0x1)}
        pos = 76
        if flags & 0x1:  # LinkTargetIDList: shell items; only skipped over
            pos += 2 + struct.unpack_from("<H", data, pos)[0]
        if flags & 0x2:  # LinkInfo: where the target was when the link was made
            start = pos
            size, header, li_flags, _vol, base_off, net_off, suffix_off = struct.unpack_from("<7I", data, start)
            if size < 28 or header < 28 or start + size > len(data):
                raise ValueError("link info runs past the end of the file")
            base_u = suffix_u = 0
            if header >= 0x24:
                base_u, suffix_u = struct.unpack_from("<2I", data, start + 28)
            block = data[start:start + size]

            def text(off, wide):
                if not off or off >= len(block):
                    return ""
                if wide:
                    end = off
                    while end + 1 < len(block) and block[end:end + 2] != b"\0\0":
                        end += 2
                    return block[off:end].decode("utf-16-le", "replace")
                end = block.find(b"\0", off)
                return block[off:end if end >= 0 else len(block)].decode(ansi, "replace")

            suffix = text(suffix_u, True) if suffix_u else text(suffix_off, False)
            if li_flags & 0x1:  # VolumeIDAndLocalBasePath
                base = text(base_u, True) if base_u else text(base_off, False)
                if base:
                    out["local"] = base + ("\\" if suffix and not base.endswith("\\") else "") + suffix
            if li_flags & 0x2:  # CommonNetworkRelativeLink: a share. Noted, never contacted
                out["network"] = True
            pos = start + size
        for key, bit in (("name", 0x4), ("relative", 0x8), ("workdir", 0x10), ("args", 0x20), ("icon", 0x40)):
            if flags & bit:  # StringData: a character count, then the characters
                n = struct.unpack_from("<H", data, pos)[0]
                pos += 2
                width = 2 if flags & 0x80 else 1
                raw = data[pos:pos + n * width]
                if len(raw) != n * width:
                    raise ValueError("a string runs past the end of the file")
                out[key] = raw.decode("utf-16-le" if width == 2 else ansi, "replace")
                pos += n * width
        while pos + 8 <= len(data):  # ExtraData blocks
            size, sig = struct.unpack_from("<II", data, pos)
            if size < 4 or pos + size > len(data):
                break
            if sig == 0xA0000001 and size >= 0x314:  # EnvironmentVariableDataBlock: a path with %VARIABLES% in it
                wide = data[pos + 268:pos + 788].decode("utf-16-le", "replace").split("\0")[0]
                out["env"] = wide or data[pos + 8:pos + 268].split(b"\0")[0].decode(ansi, "replace")
            elif sig == 0xA0000006:  # DarwinDataBlock: Windows Installer works out the target when it is clicked
                out["installer"] = True
            pos += size
        return out
    except struct.error:
        raise ValueError("cut short")


def parse_url(data):
    """The address in an Internet Shortcut (.url), or None. Only its kind and host are kept."""
    for enc in ("utf-8-sig", "utf-16", "latin-1"):
        try:
            text = data.decode(enc)
            break
        except UnicodeDecodeError:
            continue
    section = ""
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1].strip().lower()
        elif section == "internetshortcut" and line.lower().startswith("url=") and len(line) < 4096:
            return line[4:].strip()
    return None


def read_head(path):
    with open("\\\\?\\" + path if IS_WIN and re.match(r"^[A-Za-z]:\\", path) else path, "rb") as f:
        return f.read(MAX_SHORTCUT_BYTES + 1)


class Prober:
    """Says whether a shortcut's target is there, by asking for that exact name and nothing more."""

    def __init__(self):
        self.windows = norm(os.environ.get("SystemRoot", r"C:\Windows"))
        self.drive_kind = {}

    def fixed(self, path):
        if not IS_WIN:
            return True
        d = path[:3].upper()
        if d not in self.drive_kind:
            import ctypes
            self.drive_kind[d] = ctypes.windll.kernel32.GetDriveTypeW(d)
        return self.drive_kind[d] == 3  # DRIVE_FIXED

    def look(self, path):
        """('ok' | 'dead' | 'unknown', why). Dead only when a fixed disk in this PC says the name isn't there."""
        if not isinstance(path, str) or not re.match(r"^[A-Za-z]:\\", path):
            return "unknown", "not a path on a disk in this PC"
        if ":" in path[2:] or re.search(r'[<>"|?*\x00-\x1f]', path[2:]) or "%" in path:
            return "unknown", "not a plain path"
        path = os.path.normpath(path)
        if not self.fixed(path):
            return "unknown", "on a drive that isn't a fixed disk in this PC (not checked)"
        try:
            os.lstat("\\\\?\\" + path if IS_WIN else path)
            return "ok", ""
        except FileNotFoundError:
            if under(norm(path), [self.windows]):
                return "unknown", "inside Windows, which hides some of its files from other programs"
            return "dead", "nothing is there any more"
        except (OSError, ValueError):
            return "unknown", "couldn't be looked at"


def read_shortcut(path, name, size, cloud, prober):
    """What one .lnk or .url leads to. Never raises: anything unreadable is 'unknown'.
    `target` is the path as the shortcut holds it; `to` and `args_preview` are for showing, spelled out."""
    sc = _read_shortcut(path, name, size, cloud, prober)
    sc["to"] = spelled_out(sc["to"])
    if "args_preview" in sc:
        sc["args_preview"] = spelled_out(sc["args_preview"])
    return sc


def _read_shortcut(path, name, size, cloud, prober):
    ext = ext_of(name)
    sc = {"status": "unknown", "reason": "", "target": "", "to": "", "leads_to": "unknown", "args_id": "", "has_args": False}
    if cloud:
        sc["reason"] = "online-only file, not downloaded to read it"
        return sc
    if size > MAX_SHORTCUT_BYTES:
        sc["reason"] = "too big to be a plain shortcut"
        return sc
    try:
        data = read_head(path)
    except OSError:
        sc["reason"] = "couldn't be read"
        return sc
    if ext == "url":
        url = parse_url(data)
        if not url:
            sc["reason"] = "no address inside"
            return sc
        try:
            parts = urlsplit(url)
            scheme, host = parts.scheme.lower(), (parts.hostname or "")
        except ValueError:
            sc["reason"] = "the address inside can't be read"
            return sc
        sc["args_id"] = hashlib.sha256(url.encode("utf-8", "replace")).hexdigest()[:10]
        if scheme in ("http", "https"):
            sc.update(status="ok", leads_to="web", to=host[:80], reason="web addresses aren't visited, so not checked")
        elif scheme in GAME_SCHEMES:
            sc.update(status="ok", leads_to="game", to=GAME_SCHEMES[scheme], reason="opens a game in its launcher; not checked")
        elif scheme == "file":
            if parts.netloc and parts.netloc.lower() != "localhost":
                sc.update(leads_to="network", to="a network share", reason="a network share; not contacted", flag="points at a network share")
            else:
                local = os.path.normpath(unquote(parts.path).lstrip("/").replace("/", "\\"))
                status, why = prober.look(local)
                sc.update(status=status, reason=why, target=local, to=local, leads_to="file")
        else:
            sc.update(leads_to="app link", to=(scheme or "?")[:40] + ":", reason="opens another program; not checked")
        return sc
    try:
        link = parse_lnk(data)
    except ValueError as ex:
        sc["reason"] = "not a readable shortcut (%s)" % ex
        return sc
    args = link.get("args") or ""
    sc["has_args"] = bool(args.strip())
    folder = os.path.dirname(path)
    # Where the link says its target is. A full path it recorded (or one with %VARIABLES%) can show
    # the target is gone. A relative path can only show it is there: it was relative to wherever
    # the link was first made, which may not be where the link is now.
    cands = []
    if link["local"]:
        cands.append((link["local"], True))
    if link["env"]:
        cands.append((os.path.expandvars(link["env"]), True))
    if link["relative"]:
        cands.append((os.path.normpath(os.path.join(folder, link["relative"])), False))
        if link["workdir"]:
            cands.append((os.path.join(link["workdir"], os.path.basename(link["relative"].replace("/", "\\"))), False))
    if not cands:
        if link["network"]:
            sc.update(leads_to="network", to="a network share", reason="a network share; not contacted")
        elif link["installer"]:
            sc["reason"] = "Windows Installer works out where this one leads"
        else:
            sc["reason"] = "a special Windows link with no file path in it"
        return sc
    looks = [(c,) + prober.look(c) + (sure,) for c, sure in cands]
    alive = [x for x in looks if x[1] == "ok"]
    sure = [x for x in looks if x[3]]
    if alive:
        target, status, why = alive[0][:3]
    elif not sure:
        sc["reason"] = "it only says where its target was from the place the shortcut was first made"
        return sc
    elif link["network"] or link["installer"] or any(x[1] == "unknown" for x in sure):
        target, status, why = next((x for x in sure if x[1] == "unknown"), sure[0])[:3]
        status, why = "unknown", why or ("Windows Installer works out where this one leads" if link["installer"] else "may be on a network share")
    else:
        target, status, why = sure[0][:3]
    target = os.path.normpath(target) if re.match(r"^[A-Za-z]:\\", target) else target
    sc.update(status=status, reason=why, target=target, to=target)
    sc["args_id"] = hashlib.sha256(args.encode("utf-8", "replace")).hexdigest()[:10] if args.strip() else ""
    base = os.path.basename(target).lower()
    if status == "ok" and os.path.isdir("\\\\?\\" + target if IS_WIN else target):
        sc["leads_to"] = "folder"
    elif ext_of(base) in ("exe", "com", "bat", "cmd", "msc", "scr", "ps1", "vbs", "js", "jar"):
        sc["leads_to"] = "program"
    else:
        sc["leads_to"] = "file"
    shown = name[:-4] if name.lower().endswith(".lnk") else name
    if base in COMMAND_HOSTS and sc["has_args"]:
        sc["flag"] = "runs a command: %s with instructions attached" % base
        sc["args_preview"] = args.strip()[:160]
    elif sc["leads_to"] == "program" and ext_of(shown) in PRETEND_EXTS:
        sc["flag"] = "named like a document but starts a program (%s)" % spelled_out(base)
    return sc


def shortcut_targets(folders, prober):
    """Targets of the shortcuts in the taskbar-pin and Start menu folders (never walking through a link)."""
    found, seen = set(), 0
    stack = [(f, 0) for f in folders if f and os.path.isdir(f)]
    while stack and seen < MAX_ELSEWHERE:
        folder, depth = stack.pop()
        try:
            with os.scandir(folder) as it:
                entries = list(it)
        except OSError:
            continue
        for e in entries:
            try:
                st = e.stat(follow_symlinks=False)
            except OSError:
                continue
            attrs = getattr(st, "st_file_attributes", 0)
            if attrs & A_REPARSE and getattr(st, "st_reparse_tag", 0) in LINK_TAGS or e.is_symlink():
                continue
            if e.is_dir(follow_symlinks=False):
                if depth < 4:
                    stack.append((e.path, depth + 1))
            elif e.name.lower().endswith(".lnk") and seen < MAX_ELSEWHERE:
                seen += 1
                sc = read_shortcut(e.path, e.name, st.st_size, bool(attrs & CLOUD_ONLY), prober)
                if sc["status"] == "ok" and sc["target"]:
                    found.add((norm(sc["target"]), sc["args_id"]))
    return found


# ---------------------------------------------------------------- the desktop window
def measure_desktop():
    """Ask the desktop window how many icons it shows and how far apart they sit. Two read-only
    questions to the window; nothing is selected, moved or changed. None if it can't be asked."""
    if not IS_WIN:
        return None
    try:
        import ctypes
        from ctypes import wintypes
        user32 = ctypes.windll.user32
        try:
            user32.SetProcessDPIAware()  # this process only: so its numbers are real pixels, like the desktop's
        except (AttributeError, OSError):
            pass
        user32.FindWindowW.restype = user32.FindWindowExW.restype = ctypes.c_void_p
        user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
        user32.FindWindowExW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, wintypes.LPCWSTR, wintypes.LPCWSTR]
        user32.SendMessageTimeoutW.argtypes = [ctypes.c_void_p, wintypes.UINT, ctypes.c_size_t, ctypes.c_ssize_t,
                                               wintypes.UINT, wintypes.UINT, ctypes.POINTER(ctypes.c_size_t)]
        user32.IsWindowVisible.argtypes = [ctypes.c_void_p]
        user32.GetWindowLongW.argtypes = [ctypes.c_void_p, ctypes.c_int]
        view = user32.FindWindowExW(user32.FindWindowW("Progman", None), None, "SHELLDLL_DefView", None)
        worker = None
        while not view:
            worker = user32.FindWindowExW(None, worker, "WorkerW", None)
            if not worker:
                break
            view = user32.FindWindowExW(worker, None, "SHELLDLL_DefView", None)
        icons = user32.FindWindowExW(view, None, "SysListView32", None) if view else None
        if not icons:
            return None

        def ask(msg, wparam=0):
            answer = ctypes.c_size_t(0)
            ok = user32.SendMessageTimeoutW(icons, msg, wparam, 0, 0x0002, 1000, ctypes.byref(answer))  # abort if hung
            return answer.value if ok else None

        count, spacing, ex_style = ask(0x1004), ask(0x1033), ask(0x1037)  # item count, item spacing, extended style
        if count is None or not spacing:
            return None
        cell_w, cell_h = spacing & 0xFFFF, (spacing >> 16) & 0xFFFF
        area = wintypes.RECT()
        if not user32.SystemParametersInfoW(0x0030, 0, ctypes.byref(area), 0) or cell_w < 16 or cell_h < 16:  # work area
            return None
        style = user32.GetWindowLongW(icons, -16)
        return {"measured": True, "width": area.right - area.left, "height": area.bottom - area.top,
                "cell_w": cell_w, "cell_h": cell_h, "icons_shown": int(count),
                "icons_hidden": not user32.IsWindowVisible(icons), "auto_arrange": bool(style & 0x0100),
                "align_to_grid": bool((ex_style or 0) & 0x00080000)}
    except Exception:  # noqa: BLE001  (a window that won't answer is not worth failing the survey for)
        return None


def screen_fit(screen):
    if screen and screen.get("cell_w") and screen.get("cell_h"):
        screen["columns"] = max(1, screen["width"] // screen["cell_w"])
        screen["rows"] = max(1, screen["height"] // screen["cell_h"])
        screen["fits"] = screen["columns"] * screen["rows"]
    return screen


# ---------------------------------------------------------------- the folders
def public_desktop(root):
    if root:
        p = os.path.join(root, "Public", "Desktop")
        return p if os.path.isdir(p) else None
    cands = []
    if IS_WIN:
        import winreg
        try:
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                                r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders") as k:
                cands.append(os.path.normpath(os.path.expandvars(winreg.QueryValueEx(k, "Common Desktop")[0])))
        except (OSError, TypeError):
            pass
    cands.append(os.path.join(os.environ.get("PUBLIC", r"C:\Users\Public"), "Desktop"))
    return next((c for c in cands if os.path.isdir(c)), None)


def elsewhere_folders(root):
    """(taskbar pin folders, Start menu folders): the same layout under --root as under a real profile."""
    appdata = os.path.join(root, "AppData", "Roaming") if root else os.environ.get("APPDATA")
    taskbar = [os.path.join(appdata, "Microsoft", "Internet Explorer", "Quick Launch", "User Pinned", "TaskBar")] if appdata else []
    start = [os.path.join(appdata, "Microsoft", "Windows", "Start Menu", "Programs")] if appdata else []
    if not root and os.environ.get("ProgramData"):
        start.append(os.path.join(os.environ["ProgramData"], "Microsoft", "Windows", "Start Menu", "Programs"))
    return taskbar, start


def folder_info(path):
    """A quick look inside one desktop folder: how much is in it, and whether it is somebody's project or program."""
    info = {"entries": 0, "bytes": 0, "truncated": False, "project": False, "program": False, "empty": False}
    stack, top = [path], True
    while stack:
        d = stack.pop()
        try:
            with os.scandir(d) as it:
                entries = list(it)
        except OSError:
            info["truncated"] = True
            continue
        if top:
            names = {e.name.lower() for e in entries}
            info["project"] = bool(names & PROJECT_MARKERS) or any(n.endswith(".sln") for n in names)
            info["program"] = any(n.startswith("unins") and n.endswith(".exe") for n in names) or "steamapps" in names \
                or any(n.endswith(".dll") for n in names)
            top = False
        for e in entries:
            info["entries"] += 1
            if info["entries"] > FOLDER_ENTRY_CAP:
                info["truncated"] = True
                return info
            try:
                st = e.stat(follow_symlinks=False)
            except OSError:
                info["truncated"] = True
                continue
            if getattr(st, "st_file_attributes", 0) & A_REPARSE and getattr(st, "st_reparse_tag", 0) in LINK_TAGS or e.is_symlink():
                info["truncated"] = True  # a link: not followed, so what is behind it isn't counted
            elif e.is_dir(follow_symlinks=False):
                stack.append(e.path)
            else:
                info["bytes"] += st.st_size
    info["empty"] = info["entries"] == 0 and not info["truncated"]
    return info


def list_desktop(path, where, prober, now, counter):
    """One record per thing sitting directly on a desktop folder."""
    items, left = [], {"hidden": 0, "links": 0, "unreadable": 0, "odd_names": 0}
    try:
        with os.scandir(path) as it:
            entries = sorted(it, key=lambda e: e.name.lower())
    except OSError:
        left["unreadable"] += 1
        return items, left
    for e in entries:
        try:
            st = e.stat(follow_symlinks=False)
        except OSError:
            left["unreadable"] += 1
            continue
        attrs = getattr(st, "st_file_attributes", 0)
        if attrs & A_REPARSE and getattr(st, "st_reparse_tag", 0) in LINK_TAGS or e.is_symlink():
            left["links"] += 1
            continue
        if attrs & (A_HIDDEN | A_SYSTEM) or e.name.lower() in ("desktop.ini", "thumbs.db"):
            left["hidden"] += 1
            continue
        if not writable_text(e.name):
            left["odd_names"] += 1
            continue
        is_dir = e.is_dir(follow_symlinks=False)
        ext = "" if is_dir else ext_of(e.name)
        kind = "folder" if is_dir else "shortcut" if ext == "lnk" else "web-link" if ext == "url" else EXT_KIND.get(ext, "other")
        created = getattr(st, "st_birthtime", None) or st.st_ctime
        touched = max(created, st.st_mtime)
        counter[0] += 1
        item = {"id": counter[0], "where": where, "name": e.name, "path": e.path, "kind": kind, "bytes": 0 if is_dir else st.st_size,
                "created": iso(created), "modified": iso(st.st_mtime), "idle_days": max(0, int((now - touched) / DAY)),
                "cloud": bool(attrs & CLOUD_ONLY)}
        if is_dir:
            item["folder"] = folder_info(e.path)
            item["bytes"] = item["folder"]["bytes"]
        elif kind in ("shortcut", "web-link"):
            item["shortcut"] = read_shortcut(e.path, e.name, st.st_size, item["cloud"], prober)
        if spelled_out(e.name) != e.name:
            item["shown"] = spelled_out(e.name)  # what the pages print instead of the name itself
            item["flag"] = "its name has invisible or direction-changing characters in it (spelled out here)"
        items.append(item)
    return items, left


def age_bucket(days):
    return 0 if days < 30 else 1 if days < 180 else 2 if days < 365 else 3 if days < 1095 else 4


def ago(days):
    if days < 1:
        return "today"
    if days < 60:
        return "%d days ago" % days
    if days < 730:
        return "%d months ago" % (days // 30)
    return "%d years ago" % (days // 365)


def clean_name(name):
    """A shortcut's name without the marks of a copy, for choosing which duplicate to keep."""
    stem = name.rsplit(".", 1)[0]
    return COPY_NAME_RX.sub("", stem).strip().lower()


def survey(args):
    now = time.time()
    root = os.path.abspath(args.root) if args.root else None
    kf = known_folders(root)
    desktop = kf.get("Desktop")
    if not desktop:
        sys.exit("No Desktop folder found%s." % (" in " + root if root else ""))
    public = public_desktop(root)
    prober = Prober()
    counter = [0]
    mine, left = list_desktop(desktop, "desktop", prober, now, counter)
    theirs, left_public = list_desktop(public, "public", prober, now, counter) if public and norm(public) != norm(desktop) else ([], {})
    items = mine + theirs

    taskbar_dirs, start_dirs = elsewhere_folders(root)
    pinned, in_start = shortcut_targets(taskbar_dirs, prober), shortcut_targets(start_dirs, prober)
    for it in items:
        sc = it.get("shortcut")
        if sc and sc["status"] == "ok" and sc["target"]:
            key = (norm(sc["target"]), sc["args_id"])
            sc["pinned"], sc["in_start"] = key in pinned, key in in_start

    # Several shortcuts to the same thing: keep the plainest-named one; one on the Public Desktop always stays
    groups = {}
    for it in items:
        sc = it.get("shortcut")
        if sc and sc["status"] == "ok" and (sc["target"] or it["kind"] == "web-link") and sc["leads_to"] != "network":
            key = (norm(sc["target"]) if sc["target"] else it["kind"] + ":" + sc["to"], sc["args_id"])
            groups.setdefault(key, []).append(it)
    duplicates, extra = [], {}
    for key, same in groups.items():
        if len(same) < 2:
            continue
        same.sort(key=lambda it: (it["where"] != "public", bool(COPY_NAME_RX.search(it["name"])), len(it["name"]), it["name"].lower()))
        keep = same[0]
        duplicates.append({"to": keep["shortcut"]["to"], "keep": keep["id"], "items": [it["id"] for it in same]})
        for it in same[1:]:
            extra[it["id"]] = keep

    # The same program file name in two different places: perhaps installed twice. Only reported.
    by_exe = {}
    for it in items:
        sc = it.get("shortcut")
        if sc and sc["status"] == "ok" and sc["leads_to"] == "program":
            exe = os.path.basename(sc["target"]).lower()
            if exe not in GENERIC_EXES:
                by_exe.setdefault(exe, {}).setdefault(norm(sc["target"]), []).append(it)
    twice = [{"program": spelled_out(exe), "places": sorted(spelled_out(os.path.dirname(v[0]["shortcut"]["target"])) for v in places.values()),
              "items": sorted(it["id"] for v in places.values() for it in v)}
             for exe, places in sorted(by_exe.items()) if len(places) > 1]

    # Where loose files would go: the matching library folder, in a "From the Desktop" folder
    docs, pics = kf.get("Documents"), kf.get("Pictures")
    home = {"documents": docs, "pictures": pics or docs, "videos": kf.get("Videos") or docs, "audio": kf.get("Music") or docs,
            "archives": kf.get("Downloads") or docs, "installers": kf.get("Downloads") or docs,
            "programs": kf.get("Downloads") or docs, "other": kf.get("Downloads") or docs, "folder": docs}

    def library_place(it):
        base = home.get(it["kind"])
        if not base:
            return os.path.join(desktop, DESKTOP_FOLDERS[it["kind"]], it["name"])  # no such library here: a folder on the Desktop
        if it["kind"] == "pictures" and SCREENSHOT_RX.search(it["name"]) and pics:
            return os.path.join(pics, "Screenshots", it["name"])
        return os.path.join(base, LIBRARY_FOLDER, it["name"])

    suggestions, for_you, alone = [], [], []

    def add(group, op, it, why, default, risk, to=None, **more):
        s = {"id": len(suggestions) + 1, "item": it["id"], "group": group, "op": op, "from": it["path"], "bytes": it["bytes"],
             "why": why, "default": default, "risk": risk}
        if to:
            s["to"] = to
        s.update(more)
        suggestions.append(s)

    for it in mine:
        sc, idle = it.get("shortcut"), it["idle_days"]
        if sc:
            leads = ("to " + sc["to"]) if sc["to"] else "somewhere this couldn't read"
            if sc.get("flag") or it.get("flag"):
                add("check", "recycle", it, "Worth a look: %s." % (sc.get("flag") or it["flag"]), False, "medium")
            elif sc["status"] == "dead":
                add("dead", "recycle", it, "Leads %s, and %s." % (leads, sc["reason"]), True, "low")
            elif it["id"] in extra:
                keep = extra[it["id"]]
                add("duplicates", "recycle", it, "Same place as \"%s\"%s, which stays." % (
                    keep.get("shown") or keep["name"], " on the Public Desktop" if keep["where"] == "public" else ""), True, "low")
            elif sc["status"] == "ok" and (sc.get("pinned") or sc.get("in_start")):
                add("elsewhere", "recycle", it, "Works. Also %s." % ("pinned to your taskbar" if sc.get("pinned") else "in your Start menu"),
                    False, "low")
            elif sc["status"] == "ok":
                add("shortcuts", "recycle", it, "Works: leads %s." % leads, False, "low")
            else:
                add("shortcuts", "recycle", it, "Couldn't tell where this leads (%s). Not counted as broken." % sc["reason"], False, "medium")
        elif it["kind"] == "folder":
            f = it["folder"]
            if f["project"] or f["program"]:
                alone.append({"path": it["path"], "why": "a code project" if f["project"] else "looks like a program's own folder"})
            elif f["empty"]:
                add("empty", "remove-empty-dir", it, "Empty folder.", True, "low", is_dir=True)
            else:
                add("folders", "move", it, "Folder with %s%d things inside, last changed %s. Shortcuts into it would need fixing." % (
                    "over " if f["truncated"] else "", f["entries"], ago(idle)), False, "medium", to=library_place(it), bucket="folder", is_dir=True)
        else:
            label = SINGULAR.get(it["kind"], "file")
            if it.get("flag"):
                add("check", "move", it, "Worth a look: %s. A name like this can make one kind of file look like another." % it["flag"],
                    False, "medium", to=library_place(it), bucket=it["kind"])
            elif it["kind"] == "programs" and not INSTALLER_NAME_RX.search(it["name"]):
                add("files", "move", it, "A program file kept on the Desktop, last changed %s. Maybe a tool you start from here." % ago(idle),
                    False, "medium", to=library_place(it), bucket=it["kind"])
            else:
                stale = idle >= STALE_DAYS
                add("files", "move", it, "Loose %s, last changed %s%s." % (label, ago(idle), "" if stale else ": maybe still in use"),
                    stale, "low", to=library_place(it), bucket=it["kind"])
    for it in theirs:
        sc = it.get("shortcut")
        shown = it.get("shown") or it["name"]
        if sc and sc["status"] == "dead":
            for_you.append({"path": it["path"], "name": shown,
                            "why": "On the Public Desktop and leads nowhere (%s is gone). Deleting it needs administrator rights: "
                                   "right-click it > Delete." % (sc["to"] or "its target")})
        elif (sc and sc.get("flag")) or it.get("flag"):
            for_you.append({"path": it["path"], "name": shown, "why": "On the Public Desktop, worth a look: %s." % ((sc or {}).get("flag") or it["flag"])})

    screen = None
    if args.screen and args.cell:
        (w, h), (cw, ch) = args.screen, args.cell
        screen = {"measured": False, "width": w, "height": h, "cell_w": cw, "cell_h": ch, "icons_shown": None}
    elif not root:
        screen = measure_desktop()
    screen = screen_fit(screen)

    by_kind, age = {}, [0] * 5
    for it in items:
        k = by_kind.setdefault(it["kind"], {"count": 0, "bytes": 0})
        k["count"] += 1
        k["bytes"] += it["bytes"]
        age[age_bucket(it["idle_days"])] += 1
    shortcuts = [it for it in items if it.get("shortcut")]
    counts = {"icons": len(items), "mine": len(mine), "public": len(theirs), "by_kind": by_kind,
              "shortcuts": len(shortcuts), "dead": sum(1 for it in shortcuts if it["shortcut"]["status"] == "dead"),
              "unknown": sum(1 for it in shortcuts if it["shortcut"]["status"] == "unknown"),
              "duplicate_extras": len(extra), "flagged": sum(1 for it in items if it.get("flag") or it.get("shortcut", {}).get("flag")),
              "loose_files": sum(1 for it in mine if not it.get("shortcut") and it["kind"] != "folder"),
              "loose_bytes": sum(it["bytes"] for it in mine if not it.get("shortcut") and it["kind"] != "folder"),
              "folders": sum(1 for it in mine if it["kind"] == "folder"),
              "age": {"buckets": AGE_BUCKETS, "counts": age}}
    return {
        "schema": SCHEMA,
        "generated": datetime.now().isoformat(timespec="seconds"),
        "test_root": root,
        "desktop": {"path": desktop, "in_onedrive": bool(kf.get("OneDrive") and under(norm(desktop), [norm(kf["OneDrive"])]))},
        "public_desktop": {"path": public},
        "known_folders": kf,
        "screen": screen,
        "counts": counts,
        "items": items,
        "duplicates": duplicates,
        "twice": twice,
        "left_alone": {"mine": left, "public": left_public, "folders": alone},
        "for_you": for_you,
        "desktop_folders": DESKTOP_FOLDERS,
        "library_folder": LIBRARY_FOLDER,
        "suggestions": suggestions,
    }


def size_pair(text):
    m = re.fullmatch(r"(\d{2,5})x(\d{2,5})", text)
    if not m:
        raise argparse.ArgumentTypeError("use WIDTHxHEIGHT, e.g. 1920x1040")
    return int(m.group(1)), int(m.group(2))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")  # file names can hold characters the console can't show
    ap = argparse.ArgumentParser(description="Read-only survey of a Windows desktop (desktop-icons skill).")
    ap.add_argument("--out", required=True, help="where to write the JSON report (the only file this script writes)")
    ap.add_argument("--root", help="test mode: treat this folder as the profile (Desktop, Public\\Desktop, Documents... "
                                   "inside it); the real desktop window, registry and Start menu are not looked at")
    ap.add_argument("--screen", type=size_pair, help="test mode: pretend the screen's work area is this big, e.g. 1920x1040")
    ap.add_argument("--cell", type=size_pair, help="test mode: pretend each icon takes this much room, e.g. 76x100")
    args = ap.parse_args(argv)
    out = os.path.abspath(args.out)
    if not os.path.isdir(os.path.dirname(out)):
        sys.exit("Output folder does not exist: %s (this script creates no folders)" % os.path.dirname(out))
    report = survey(args)
    with open(out, "w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=1)
    c, s = report["counts"], report["screen"]
    print("desktop: %d icons (%d yours, %d public): %d shortcuts (%d dead, %d extra copies, %d unknown), %d loose files, %d folders%s; "
          "%d suggestions -> %s" % (c["icons"], c["mine"], c["public"], c["shortcuts"], c["dead"], c["duplicate_extras"], c["unknown"],
                                    c["loose_files"], c["folders"], "; about %d fit on one screen" % s["fits"] if s and s.get("fits") else "",
                                    len(report["suggestions"]), out))


if __name__ == "__main__":
    main()

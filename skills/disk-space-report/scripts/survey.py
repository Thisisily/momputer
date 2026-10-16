#!/usr/bin/env python3
"""Read-only survey of where a Windows PC's disk space went (disk-space-report skill).

Writes exactly one file: the JSON report named by --out. It never creates, moves,
renames, deletes or empties anything else, and it never opens a file: it reads
names, sizes, dates and attributes from folder listings only. Junctions and
symlinks are never followed, other accounts' folders are never listed, and what
needs administrator rights is skipped and reported as "not measured".

Python 3.8+, standard library only.

    python survey.py --out survey.json
    python survey.py --out survey.json --drives C,D --max-seconds 1200
    python survey.py --out survey.json --root "<folder>" --apparent-size    # test mode
"""
import argparse
import bisect
import heapq
import json
import os
import shutil
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime

IS_WIN = os.name == "nt"
if IS_WIN:
    import ctypes
    import winreg
    from ctypes import wintypes

SCHEMA = "disk-space-report/survey@1"
LONG = "\\\\?\\"            # exact-name prefix: long paths work, and a name is never tidied up on the way
MB, GB, DAY = 1024 ** 2, 1024 ** 3, 86400.0
YEAR = 365 * DAY

# Windows file attribute bits (winnt.h)
A_DIRECTORY, A_SPARSE, A_REPARSE, A_COMPRESSED = 0x10, 0x200, 0x400, 0x800
A_OFFLINE, A_RECALL_OPEN, A_RECALL_DATA = 0x1000, 0x40000, 0x400000
CLOUD_ONLY = A_OFFLINE | A_RECALL_OPEN | A_RECALL_DATA   # online-only file: takes no space here
TAG_CLOUD = 0x9000001A      # cloud-files placeholder (OneDrive). Every other reparse folder is a link: left alone

BIG_FILE = 100 * MB         # files this big are remembered for the "biggest files" list
MEDIA_MIN = 500 * MB        # videos, disk images and model files from this size are listed
INSTALLER_AGE_DAYS = 30     # a month-old installer has done its job
YEARS_KEPT = 12             # the "last changed" chart counts years one by one this far back
KEEP_DEPTH = 6              # folder levels kept for the "what fills the drive" chart
READ_AHEAD = 24             # subfolder listings read in the background per folder being walked
TREE_FANOUT = 14
LIST_MAX = 300

CATEGORIES = [
    ("dev", "Developer leftovers",
     "Things code projects create and can create again: dependency folders, build output, virtual disks, SDKs."),
    ("caches", "Package caches",
     "Downloads that package managers keep so the next install is faster. They fill up again as you work."),
    ("browsers", "Browser caches", "Copies of pages and pictures your browsers keep to load sites faster."),
    ("apps", "Other apps' caches", "Cache folders inside apps' own settings folders."),
    ("windows", "Windows' own",
     "Temporary files, update leftovers, memory dumps, the page and hibernation files, the Recycle Bin."),
    ("games", "Games and shader caches", "Game libraries, and the shader caches games and graphics drivers build."),
    ("downloads", "Old installers and disk images in Downloads", "Installers over a month old. They've done their job."),
    ("videos", "Big videos", "Videos of 500 MB or more in your own folders."),
    ("images", "Disk images and virtual machines", "Virtual machine disks and disc images of 500 MB or more."),
    ("models", "AI model files", "Model files of 500 MB or more outside the known model caches."),
    ("old", "Big and untouched for a year", "Files and folders of a few GB where nothing has changed in over a year."),
]

# rule id -> (category, plain title, what it is)
RULES = {
    "node-modules": ("dev", "node_modules folders",
                     "Packages a JavaScript project downloaded. Its install command brings them back, but only while online."),
    "python-venv": ("dev", "Python virtual environments",
                    "A private copy of Python and its packages for one project. Can be made again from the project's requirements."),
    "rust-target": ("dev", "Rust build folders (target)", "Everything Cargo compiled for a project. The next build makes it again, slowly."),
    "build-output": ("dev", "Build output (build, dist, .next and similar)", "What a project's build wrote. Building again re-creates it."),
    "gradle-project": ("dev", ".gradle folders inside projects", "Gradle's working files for one project."),
    "python-bytecode": ("dev", "Python's __pycache__ and test caches",
                        "Small files Python and its test tools write next to code. Made again on the next run."),
    "docker-disk": ("dev", "Docker Desktop's disk image",
                    "One file holding every Docker image, container and volume. It grows as you use Docker and doesn't shrink by itself."),
    "wsl-disk": ("dev", "WSL (Linux) disk images",
                 "Each Linux distribution lives in one disk-image file. It grows and doesn't shrink by itself."),
    "android-sdk": ("dev", "Android SDK and emulator system images", "Platforms, build tools and emulator images from Android Studio."),
    "android-avd": ("dev", "Android emulator devices", "The virtual phones and tablets made in Android Studio's Device Manager."),
    "rustup-toolchains": ("dev", "Rust toolchains", "Compiler versions installed by rustup."),
    "npm-cache": ("caches", "npm's download cache", "Every package npm has downloaded."),
    "pnpm-store": ("caches", "pnpm's store", "pnpm keeps one copy of each package here and links projects to it."),
    "yarn-cache": ("caches", "Yarn's cache", "Every package Yarn has downloaded."),
    "pip-cache": ("caches", "pip's cache", "Downloaded and built Python packages."),
    "uv-cache": ("caches", "uv's cache", "Downloaded and built Python packages, and uv's own Python builds."),
    "cargo-registry": ("caches", "Cargo's registry and git checkouts", "Source code of every Rust crate downloaded."),
    "nuget-packages": ("caches", "NuGet packages and caches", "Every .NET package restored on this PC."),
    "gradle-cache": ("caches", "Gradle's caches", "Downloaded libraries, Gradle versions and build caches."),
    "maven-repo": ("caches", "Maven's local repository", "Every Java library Maven has downloaded."),
    "go-modcache": ("caches", "Go's module cache", "Source code of every Go module downloaded."),
    "go-build-cache": ("caches", "Go's build cache", "Compiled pieces Go keeps to make the next build faster."),
    "conda-pkgs": ("caches", "conda's package cache", "Downloaded conda packages."),
    "hf-cache": ("caches", "Hugging Face model cache", "AI models and datasets downloaded through Hugging Face tools."),
    "model-cache": ("caches", "Other AI model stores (Ollama, LM Studio, PyTorch, Whisper)", "Models downloaded by local AI tools."),
    "test-browsers": ("caches", "Browsers downloaded for testing (Playwright, Puppeteer, Cypress)",
                      "Whole browsers that test tools download. Old versions pile up."),
    "other-pkg-cache": ("caches", "Other package caches (Electron, Deno, Bun, Dart, Composer)", "Downloads kept by other tools."),
    "browser-cache": ("browsers", "Browser cache", "Cached pages, scripts and pictures."),
    "app-cache": ("apps", "App cache", "Cache folders an app keeps in its settings folder."),
    "user-temp": ("windows", "Your Temp folder", "Scratch files that programs leave behind."),
    "windows-temp": ("windows", "Windows' Temp folder", "Scratch files left by Windows and installers."),
    "windows-update-cache": ("windows", "Windows Update downloads", "Update files Windows has already installed or is waiting to install."),
    "windows-old": ("windows", "Previous Windows installation (Windows.old)",
                    "The old Windows kept after an upgrade, so you can go back. Windows removes it by itself after a while."),
    "delivery-optimization": ("windows", "Delivery Optimization cache", "Update files kept to share with other PCs."),
    "windows-installer": ("windows", "Windows' installer cache",
                          "Needed to repair and uninstall programs. Never delete it by hand."),
    "windows-logs": ("windows", "Windows' log files", "Logs from updates and servicing."),
    "windows-caches": ("windows", "Thumbnail, web and error-report caches", "Small caches Windows keeps for you."),
    "memory-dumps": ("windows", "Memory dumps from crashes", "A copy of memory saved when Windows crashed."),
    "crash-dumps": ("windows", "Crash dumps from programs", "A copy of a program's memory saved when it crashed."),
    "hibernation-file": ("windows", "Hibernation file (hiberfil.sys)", "Where Windows saves memory for Hibernate and Fast Startup."),
    "page-file": ("windows", "Page file (pagefile.sys)", "Extra memory on disk. Windows manages it."),
    "swap-file": ("windows", "Swap file (swapfile.sys)", "Extra memory on disk for Store apps. Windows manages it."),
    "recycle-bin": ("windows", "Recycle Bin", "Deleted files waiting for you to empty the bin."),
    "driver-leftovers": ("windows", "Graphics driver installer leftovers", "Unpacked driver installers. Safe to remove once the driver is installed."),
    "game": ("games", "Games", "Installed games."),
    "steam-shader-cache": ("games", "Steam's shader cache", "Shaders Steam prepared so games stutter less. Rebuilt when needed."),
    "steam-downloads": ("games", "Steam's unfinished downloads", "Downloads and patches Steam was in the middle of."),
    "gpu-shader-cache": ("games", "Graphics shader caches", "Shaders compiled by DirectX and your graphics driver. Rebuilt when needed."),
}
LEFTOVER_RULES = {"node-modules", "python-venv", "rust-target", "build-output", "gradle-project"}   # inside projects
AGGREGATED = {"python-bytecode", "browser-cache", "app-cache"}     # too many to list one by one

# Known places: (rule, base, path below the base). Bases are worked out in Survey.places().
KNOWN = [
    ("npm-cache", "local", "npm-cache"), ("npm-cache", "roaming", "npm-cache"),
    ("pnpm-store", "local", r"pnpm\store"), ("pnpm-store", "local", "pnpm-cache"),
    ("yarn-cache", "local", r"Yarn\Cache"), ("yarn-cache", "local", r"Yarn\Berry\cache"),
    ("pip-cache", "local", r"pip\Cache"), ("uv-cache", "local", r"uv\cache"),
    ("cargo-registry", "profile", r".cargo\registry"), ("cargo-registry", "profile", r".cargo\git"),
    ("nuget-packages", "profile", r".nuget\packages"), ("nuget-packages", "local", r"NuGet\v3-cache"),
    ("nuget-packages", "local", r"NuGet\plugins-cache"),
    ("gradle-cache", "profile", r".gradle\caches"), ("gradle-cache", "profile", r".gradle\wrapper\dists"),
    ("gradle-cache", "profile", r".gradle\daemon"),
    ("maven-repo", "profile", r".m2\repository"),
    ("go-modcache", "profile", r"go\pkg\mod"), ("go-build-cache", "local", "go-build"),
    ("conda-pkgs", "profile", r"miniconda3\pkgs"), ("conda-pkgs", "profile", r"anaconda3\pkgs"),
    ("conda-pkgs", "profile", r".conda\pkgs"),
    ("hf-cache", "profile", r".cache\huggingface"),
    ("model-cache", "profile", r".cache\torch"), ("model-cache", "profile", r".cache\whisper"),
    ("model-cache", "profile", r".ollama\models"), ("model-cache", "profile", r".lmstudio\models"),
    ("model-cache", "profile", r".cache\lm-studio\models"), ("model-cache", "local", r"nomic.ai\GPT4All"),
    ("test-browsers", "local", "ms-playwright"), ("test-browsers", "profile", r".cache\puppeteer"),
    ("test-browsers", "local", r"Cypress\Cache"),
    ("other-pkg-cache", "local", r"electron\Cache"), ("other-pkg-cache", "local", "deno"),
    ("other-pkg-cache", "profile", r".bun\install\cache"), ("other-pkg-cache", "local", r"Pub\Cache"),
    ("other-pkg-cache", "local", "Composer"),
    ("android-sdk", "local", r"Android\Sdk"), ("android-avd", "profile", r".android\avd"),
    ("rustup-toolchains", "profile", r".rustup\toolchains"),
    ("user-temp", "local", "Temp"), ("crash-dumps", "local", "CrashDumps"),
    ("windows-caches", "local", r"Microsoft\Windows\INetCache"), ("windows-caches", "local", r"Microsoft\Windows\Explorer"),
    ("windows-caches", "programdata", r"Microsoft\Windows\WER"),
    ("gpu-shader-cache", "local", "D3DSCache"), ("gpu-shader-cache", "local", r"NVIDIA\DXCache"),
    ("gpu-shader-cache", "local", r"NVIDIA\GLCache"), ("gpu-shader-cache", "local", r"AMD\DxCache"),
    ("gpu-shader-cache", "local", r"AMD\DxcCache"), ("gpu-shader-cache", "local", r"AMD\VkCache"),
    ("gpu-shader-cache", "local", r"AMD\GLCache"), ("gpu-shader-cache", "local", r"Intel\ShaderCache"),
    ("driver-leftovers", "programdata", r"NVIDIA Corporation\Downloader"),
]
BROWSERS = [
    ("Chrome", "local", r"Google\Chrome\User Data"), ("Edge", "local", r"Microsoft\Edge\User Data"),
    ("Brave", "local", r"BraveSoftware\Brave-Browser\User Data"), ("Vivaldi", "local", r"Vivaldi\User Data"),
    ("Opera", "local", "Opera Software"), ("Opera", "roaming", "Opera Software"),
    ("Firefox", "local", r"Mozilla\Firefox\Profiles"),
]
# Folders inside the running Windows folder that are measured. The rest of Windows is worked out, not walked.
WINDOWS_PROBES = [
    ("windows-temp", "Temp"), ("windows-update-cache", r"SoftwareDistribution\Download"),
    ("memory-dumps", "Minidump"), ("memory-dumps", "LiveKernelReports"),
    ("windows-installer", "Installer"), ("windows-logs", "Logs"),
    ("delivery-optimization", r"ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache"),
]
ROOT_FILES = {"pagefile.sys": "page-file", "hiberfil.sys": "hibernation-file", "swapfile.sys": "swap-file"}
CACHE_NAMES = {"cache", "code cache", "gpucache", "cache2", "cachestorage", "cacheddata", "cachedextensionvsixs",
               "dawncache", "dawngraphitecache", "dawnwebgpucache", "grshadercache", "graphitedawncache",
               "shadercache", "startupcache", "component_crx_cache", "cache_data", "localcache"}
PASS_THROUGH = {"microsoft", "google", "packages", "programs"}      # vendor folders: the app is one level down
BY_NAME = {"node_modules": "node-modules", ".next": "build-output", ".nuxt": "build-output", ".svelte-kit": "build-output",
           ".turbo": "build-output", ".parcel-cache": "build-output", ".angular": "build-output",
           ".dart_tool": "build-output", ".terraform": "build-output", "__pycache__": "python-bytecode",
           ".pytest_cache": "python-bytecode", ".mypy_cache": "python-bytecode", ".ruff_cache": "python-bytecode",
           ".tox": "python-venv", ".nox": "python-venv"}
PROJECT_MARKERS = {"package.json", "cargo.toml", "pyproject.toml", "setup.py", "requirements.txt", "go.mod", "pom.xml",
                   "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "gradlew",
                   "cmakelists.txt", "composer.json", "gemfile", "pubspec.yaml", "makefile", ".git"}
GRADLE_MARKERS = {"build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "gradlew"}
DOTNET_EXTS = (".sln", ".csproj", ".fsproj", ".vbproj")
DOTNET = frozenset(["dotnet"])
EMPTY = frozenset()
GAME_LIBS = {"xboxgames": "Xbox app", "epic games": "Epic Games", "riot games": "Riot", "ea games": "EA app",
             "origin games": "EA app", "gog games": "GOG", "games": "your Games folder"}
LAUNCHER_GAME_DIRS = {"gog galaxy": "GOG", "ubisoft game launcher": "Ubisoft Connect"}   # their "games" subfolder
NOT_GAMES = {"launcher", "directxredist", "epic online services", "gamesave", "riot client"}
SHARED_USERS = {"default", "default user", "all users"}
INSTALLER_EXTS = set("exe msi msix msixbundle appx appxbundle iso img dmg pkg apk vhd vhdx".split())
VIDEO_EXTS = set("mp4 mov mkv avi wmv webm m4v flv mts m2ts 3gp mpg mpeg ts".split())
IMAGE_EXTS = set("iso img vhd vhdx vmdk vdi qcow2 ova wim esd".split())
MODEL_EXTS = set("gguf safetensors ckpt pt pth onnx".split())


def key_of(path):
    """Lower-cased path with no trailing backslash (except a drive root): how folders are matched."""
    p = os.path.normpath(path).lower()
    return p if len(p) <= 3 else p.rstrip("\\")


def ext_of(name):
    i = name.rfind(".")
    return name[i + 1:].lower() if i > 0 else ""


def iso(ts):
    try:
        return datetime.fromtimestamp(ts).strftime("%Y-%m-%d") if ts and ts > 0 else None
    except (OverflowError, OSError, ValueError):
        return None


# ---------------------------------------------------------------- Windows bits (all read-only)
if IS_WIN:
    _k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _k32.GetCompressedFileSizeW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(wintypes.DWORD)]
    _k32.GetCompressedFileSizeW.restype = wintypes.DWORD

    class SHQUERYRBINFO(ctypes.Structure):
        if ctypes.sizeof(ctypes.c_void_p) == 4:
            _pack_ = 1
        _fields_ = [("cbSize", wintypes.DWORD), ("i64Size", ctypes.c_int64), ("i64NumItems", ctypes.c_int64)]


def on_disk_size(path, logical):
    """Space a sparse or compressed file really takes (asks the file system; the file is not opened)."""
    high = wintypes.DWORD(0)
    ctypes.set_last_error(0)
    low = _k32.GetCompressedFileSizeW(path, ctypes.byref(high))
    if low == 0xFFFFFFFF and ctypes.get_last_error():
        return logical
    return (high.value << 32) | low


def recycle_bin(root):
    """Size and item count of your own Recycle Bin on one drive, from Windows itself."""
    try:
        info = SHQUERYRBINFO()
        info.cbSize = ctypes.sizeof(info)
        if ctypes.windll.shell32.SHQueryRecycleBinW(root, ctypes.byref(info)) != 0:
            return None
        return {"bytes": int(info.i64Size), "items": int(info.i64NumItems)}
    except (OSError, AttributeError):
        return None


def volume_info(root):
    label, fs = ctypes.create_unicode_buffer(261), ctypes.create_unicode_buffer(261)
    ok = ctypes.windll.kernel32.GetVolumeInformationW(root, label, 261, None, None, None, fs, 261)
    return (label.value, fs.value) if ok else ("", "")


def fixed_drives(only):
    """Local fixed disks. Cloud and virtual drives (they also call themselves 'fixed') are left out."""
    k32 = ctypes.windll.kernel32
    mask = k32.GetLogicalDrives()
    why = {2: "removable drive", 4: "network drive", 5: "disc drive", 6: "RAM disk"}
    drives, others = [], []
    for i in range(26):
        if not mask & (1 << i):
            continue
        letter = chr(65 + i)
        root = letter + ":\\"
        kind = k32.GetDriveTypeW(root)
        if kind != 3 and not (kind == 2 and only and letter in only):     # a removable drive only when named
            others.append({"root": root, "why": why.get(kind, "not a local disk")})
            continue
        if only and letter not in only:
            others.append({"root": root, "why": "left out with --drives"})
            continue
        buf = ctypes.create_unicode_buffer(1024)
        target = buf.value if k32.QueryDosDeviceW(letter + ":", buf, 1024) else ""
        if not only and not target.lower().startswith("\\device\\harddiskvolume"):
            others.append({"root": root, "why": "cloud, virtual or mapped drive (name it with --drives to include it)"})
            continue
        try:
            u = shutil.disk_usage(root)
        except OSError:
            others.append({"root": root, "why": "couldn't be read (locked or empty)"})
            continue
        label, fs = volume_info(root)
        drives.append({"root": root, "label": label, "fs": fs, "total": u.total, "free": u.free})
    return drives, others


def downloads_folder(profile):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                            r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders") as k:
            v = winreg.QueryValueEx(k, "{374DE290-123F-4565-9164-39C4925E467B}")[0]
            if isinstance(v, str) and v.strip():
                return os.path.normpath(os.path.expandvars(v))
    except OSError:
        pass
    return os.path.join(profile, "Downloads")


# ---------------------------------------------------------------- the walk
def listing(path):
    """One folder's entries as (name, attributes, size, modified, reparse tag, is a symlink), or the error.

    Everything comes from the folder's own listing: no entry is opened. Safe to run in a worker thread."""
    out = []
    try:
        with os.scandir(LONG + path if IS_WIN else path) as it:
            for e in it:
                try:
                    st = e.stat(follow_symlinks=False)
                except OSError:
                    out.append((e.name, -1, 0, 0.0, 0, False))
                    continue
                attrs = getattr(st, "st_file_attributes", None)
                if attrs is None:                # not Windows (test mode only)
                    attrs = A_DIRECTORY if e.is_dir(follow_symlinks=False) else 0
                out.append((e.name, attrs, st.st_size, st.st_mtime, getattr(st, "st_reparse_tag", 0), e.is_symlink()))
    except OSError as ex:
        return ex
    return out


class Frame(object):
    """One folder being measured."""
    __slots__ = ("path", "key", "name", "lname", "depth", "bytes", "own", "files", "dirs", "cloud", "newest",
                 "newest_own", "subs", "kids", "rule", "group", "inside", "user", "system", "pf", "browser", "app",
                 "appbase", "dl", "markers", "pm", "gamelib", "partial", "denied", "stale", "left", "profile", "users",
                 "ctx", "fut", "sent")

    def __init__(self, path, key, name, depth):
        self.path, self.key, self.name, self.lname, self.depth = path, key, name, name.lower(), depth
        self.bytes = self.own = self.files = self.dirs = self.cloud = self.left = self.sent = 0
        self.newest = self.newest_own = 0.0
        self.subs = self.kids = self.rule = self.group = self.browser = self.app = self.gamelib = self.stale = None
        self.ctx = None                          # the usual suspect this folder sits inside, if any
        self.fut = None                          # this folder's listing, if it is being read ahead
        self.inside = self.user = self.system = self.pf = self.appbase = self.partial = self.denied = False
        self.profile = self.users = False
        self.dl = -1
        self.markers = self.pm = EMPTY


class Survey(object):
    def __init__(self, args):
        self.args = args
        self.now = time.time()
        self.t0 = time.time()
        self.apparent = args.apparent_size or not IS_WIN
        self.old_bytes = int(args.old_gb * GB)
        self.test = bool(args.root)
        self.entries = self.errors = self.links = self.cloud_dirs = self.other_users = 0
        self.truncated = self.stop = False
        self.findings, self.agg, self.projects, self.skipped, self.not_measured = [], {}, [], [], []
        self.big, self.old_installers, self.big_videos, self.disk_images = [], [], [], []
        self.model_files, self.old_files, self.stale_folders = [], [], []
        # space by the year files were last changed, per drive: one counter per file, from the listing
        this_year = datetime.now().year
        self.year0 = this_year - YEARS_KEPT + 1
        self.year_starts = [time.mktime((y, 1, 1, 0, 0, 0, 0, 0, -1)) for y in range(self.year0, this_year + 1)]
        self.ages = {}
        if args.root:
            root = os.path.abspath(args.root)
            self.drives, self.other_drives = [{"root": root, "label": "Test folder", "fs": "", "test": True}], []
            self.env = {"profile": root, "appdata": os.path.join(root, "AppData"), "windows": os.path.join(root, "Windows"),
                        "programdata": os.path.join(root, "ProgramData"), "downloads": os.path.join(root, "Downloads"),
                        "pf": [os.path.join(root, "Program Files"), os.path.join(root, "Program Files (x86)")]}
        else:
            only = set(args.drives.upper().replace(",", "").replace(":", "")) if args.drives else None
            self.drives, self.other_drives = fixed_drives(only)
            env, profile = os.environ, os.path.expanduser("~")
            self.env = {"profile": profile, "appdata": os.path.join(profile, "AppData"),
                        "windows": env.get("SystemRoot") or r"C:\Windows",
                        "programdata": env.get("ProgramData") or r"C:\ProgramData",
                        "downloads": downloads_folder(profile),
                        "pf": [p for p in {env.get("ProgramFiles"), env.get("ProgramFiles(x86)"),
                                           env.get("ProgramW6432")} if p]}
            if env.get("LOCALAPPDATA"):
                self.env["local"] = env["LOCALAPPDATA"]
            if env.get("APPDATA"):
                self.env["roaming"] = env["APPDATA"]
        e = self.env
        e.setdefault("local", os.path.join(e["appdata"], "Local"))
        e.setdefault("roaming", os.path.join(e["appdata"], "Roaming"))
        e["locallow"] = os.path.join(e["appdata"], "LocalLow")
        self.profile_key = key_of(e["profile"])
        self.local_key = key_of(e["local"]) + "\\"
        self.special = self.places()
        self.pool = ThreadPoolExecutor(max_workers=args.threads) if args.threads > 1 else None

    def places(self):
        """Folders that mean something, by lower-cased path."""
        sp, e = {}, self.env

        def put(path, what):
            sp.setdefault(key_of(path), what)

        put(e["windows"], ("skip",))                    # measured piece by piece, see windows_parts()
        for d in self.drives:
            r = d["root"]
            put(os.path.join(r, "$Recycle.Bin"), ("skip",) if not self.test else ("rule", "recycle-bin"))
            put(os.path.join(r, "System Volume Information"), ("skip",))
            put(os.path.join(r, "Users"), ("users",))
            put(os.path.join(r, "Users", "Public"), ("public",))
            put(os.path.join(r, "Windows.old"), ("rule", "windows-old"))
            put(os.path.join(r, "NVIDIA"), ("rule", "driver-leftovers"))
            put(os.path.join(r, "AMD"), ("rule", "driver-leftovers"))
        put(e["profile"], ("profile",))
        put(e["appdata"], ("appdata",))
        for b in ("local", "roaming", "locallow"):
            put(e[b], ("appbase",))
        put(e["downloads"], ("downloads",))
        for p in e["pf"]:
            put(p, ("pf",))
        put(e["programdata"], ("system",))
        for rule, base, rel in KNOWN:
            put(os.path.join(e[base], rel), ("rule", rule))
        for name, base, rel in BROWSERS:
            put(os.path.join(e[base], rel), ("browser", name))
        return sp

    # -------------------------------------------------- one folder
    def ahead(self, F):
        """Start reading the listings of the next few subfolders in the background (cold disks are slow)."""
        subs = F.subs
        if self.pool is None or not subs:
            return
        base = F.path if F.path.endswith("\\") else F.path + "\\"
        while F.sent < READ_AHEAD and F.sent < len(subs):
            s = subs[len(subs) - 1 - F.sent]     # subfolders are taken from the end of the list
            s[3] = self.pool.submit(listing, base + s[0])
            F.sent += 1

    def scan(self, F):
        path = F.path
        entries = F.fut.result() if F.fut is not None else listing(path)
        F.fut = None
        if isinstance(entries, OSError):
            F.denied = True
            self.errors += 1
            if len(self.skipped) < 300:
                self.skipped.append({"path": path, "why": "no access" if isinstance(entries, PermissionError)
                                     else type(entries).__name__})
            return
        self.entries += len(entries)
        base = path if path.endswith("\\") else path + "\\"
        kbase = F.key if F.key.endswith("\\") else F.key + "\\"
        if F.user and not F.inside and F.rule is None and F.gamelib is None:
            names = {e[0].lower() for e in entries}
            F.rule = self.classify(F, names)
            if F.rule is None:
                m = names & PROJECT_MARKERS
                if any(n.endswith(DOTNET_EXTS) for n in names):
                    m = m | DOTNET
                F.markers = m
        watch = F.depth == 0 or F.dl >= 0       # small files matter here: system files at a drive's top, installers
        special, apparent = self.special, self.apparent
        starts, ages = self.year_starts, self.ages.setdefault(F.key[:1].upper(), {})
        future = self.now + DAY                   # a date past tomorrow is a wrong clock, not a change
        own = files = cloud = 0
        newest = 0.0
        subs = []
        for name, attrs, size, m, tag, link in entries:
            if attrs < 0:
                self.errors += 1
                continue
            if attrs & A_DIRECTORY:
                if link or (attrs & A_REPARSE and (tag & 0xFFFF0FFF) != TAG_CLOUD):
                    self.links += 1              # junction, symlink or other redirect: never followed
                    continue
                if attrs & (A_OFFLINE | A_RECALL_OPEN):
                    self.cloud_dirs += 1         # online-only folder: listing it would fetch it
                    continue
                lname = name.lower()
                key = kbase + lname
                sp = special.get(key)
                if sp is not None and sp[0] == "skip":
                    continue
                if F.users and sp is None:       # someone else's account: not ours to look at
                    if lname not in SHARED_USERS:
                        self.other_users += 1
                    continue
                subs.append([name, lname, key, None])
                continue
            if link:
                self.links += 1
                continue
            if attrs & CLOUD_ONLY:
                cloud += size
                size = 0
            elif attrs & (A_SPARSE | A_COMPRESSED) and not apparent:
                size = on_disk_size(LONG + base + name, size)
            own += size
            files += 1
            when = 0 if m > future else m
            if size:
                y = bisect.bisect_right(starts, when) - 1   # -1: before the first year kept (or no date)
                ages[y] = ages.get(y, 0) + size
            if when > newest:
                newest = when
            if size >= BIG_FILE or watch:
                self.file_hit(F, name, size, m, base)
        F.own = F.bytes = own
        F.files, F.cloud = files, cloud
        F.newest = F.newest_own = newest
        if F.depth == 0 or F.users:              # your own profile first, in case time runs out
            pk = self.profile_key
            subs.sort(key=lambda s: 1 if pk == s[2] or pk.startswith(s[2] + "\\") else 0)
        F.subs = subs
        if F.depth < KEEP_DEPTH:
            F.kids = []
        self.ahead(F)

    @staticmethod
    def classify(F, names):
        """Is this folder something a code project can make again? Only asked in your own folders."""
        n = F.lname
        if n in BY_NAME:
            return BY_NAME[n]
        if "pyvenv.cfg" in names or "conda-meta" in names:
            return "python-venv"
        pm = F.pm
        if n == "target":
            if "cargo.toml" in pm or "cachedir.tag" in names or ".rustc_info.json" in names:
                return "rust-target"
            if "pom.xml" in pm:
                return "build-output"
        elif n in ("build", "dist", "out"):
            if pm:
                return "build-output"
        elif n in ("bin", "obj"):
            if "dotnet" in pm:
                return "build-output"
        elif n == ".gradle":
            if pm & GRADLE_MARKERS:
                return "gradle-project"
        return None

    def file_hit(self, F, name, size, mtime, base):
        path, lname = base + name, name.lower()
        if F.depth == 0 and lname in ROOT_FILES:
            self.findings.append({"rule": ROOT_FILES[lname], "path": path, "bytes": size, "files": 1, "modified": iso(mtime)})
            return
        ext = ext_of(name)
        age = self.now - mtime
        if size >= BIG_FILE:
            if ext in ("vhdx", "vhd"):
                k = F.key + "\\"
                rule = None
                if k.startswith(self.local_key + "docker\\"):
                    rule = "docker-disk"
                elif lname == "ext4.vhdx" or k.startswith(self.local_key + "packages\\") or k.startswith(self.local_key + "wsl\\"):
                    rule = "wsl-disk"
                if rule:
                    self.findings.append({"rule": rule, "path": path, "bytes": size, "files": 1, "modified": iso(mtime)})
            mine = F.user and not F.inside and F.rule is None
            heapq.heappush(self.big, (size, path, mtime, F.rule or F.ctx, mine))
            if len(self.big) > LIST_MAX:
                heapq.heappop(self.big)
        if F.dl >= 0 and ext in INSTALLER_EXTS and age >= INSTALLER_AGE_DAYS * DAY and not F.inside and F.rule is None:
            self.old_installers.append((size, path, mtime))
        if F.inside or F.rule is not None or F.system:
            return
        if size >= self.old_bytes and age > YEAR:
            self.old_files.append((size, path, mtime))
        if not F.user or size < MEDIA_MIN:
            return
        if ext in VIDEO_EXTS:
            self.big_videos.append((size, path, mtime))
        elif ext in IMAGE_EXTS and F.dl < 0:
            self.disk_images.append((size, path, mtime))
        elif ext in MODEL_EXTS:
            self.model_files.append((size, path, mtime))

    def child(self, F, sub):
        name, lname, key, fut = sub
        C = Frame(F.path + name if F.path.endswith("\\") else F.path + "\\" + name, key, name, F.depth + 1)
        C.fut = fut
        C.user, C.system, C.browser, C.app = F.user, F.system, F.browser, F.app
        C.inside = F.inside or F.rule is not None
        C.ctx = F.rule or F.ctx
        C.pm = F.markers
        if F.dl == 0:
            C.dl = 1
        sp = self.special.get(key)
        if sp is not None:
            kind = sp[0]
            if kind == "rule":
                if not C.inside:
                    C.rule = sp[1]
            elif kind == "profile":
                C.user, C.system, C.profile = True, False, True
            elif kind in ("appdata", "appbase"):
                C.user, C.appbase = False, kind == "appbase"
            elif kind == "downloads":
                C.dl = 0
            elif kind in ("pf", "system"):
                C.user, C.system, C.pf = False, True, kind == "pf"
            elif kind == "browser":
                C.browser = sp[1]
            elif kind == "users":
                C.user, C.users = False, True
            elif kind == "public":
                C.user = True
        elif F.profile and lname.startswith("."):
            C.user = False                       # tool folders such as .cargo or .vscode: not your projects
        elif F.appbase:
            if lname in PASS_THROUGH:
                C.appbase = True
            else:
                C.app = name
        if C.rule is None and not C.inside:
            if F.gamelib:
                if lname not in NOT_GAMES:
                    C.rule, C.group = "game", F.gamelib
            elif F.lname == "steamapps":
                if lname == "common":
                    C.gamelib = "Steam"
                elif lname == "shadercache":
                    C.rule = "steam-shader-cache"
                elif lname in ("downloading", "temp"):
                    C.rule = "steam-downloads"
                elif lname == "workshop":
                    C.rule, C.group = "game", "Steam Workshop"
            elif lname == "games" and F.lname in LAUNCHER_GAME_DIRS:
                C.gamelib = LAUNCHER_GAME_DIRS[F.lname]
            elif lname in GAME_LIBS and (F.depth == 0 or F.pf or (lname == "games" and F.profile)):
                C.gamelib = GAME_LIBS[lname]
            elif not C.user and (C.browser or C.app) and lname in CACHE_NAMES:
                C.rule, C.group = ("browser-cache", C.browser) if C.browser else ("app-cache", C.app)
        return C

    def fold(self, P, C):
        """A finished folder is added to its parent and remembered if it is one of the usual suspects."""
        P.bytes += C.bytes
        P.files += C.files
        P.dirs += C.dirs + 1
        P.cloud += C.cloud
        if C.partial:
            P.partial = True
        if C.newest > P.newest:
            P.newest = C.newest
        rule = C.rule
        if rule in LEFTOVER_RULES:
            P.left += C.bytes
        elif rule != "python-bytecode" and C.newest_own > P.newest_own:
            P.newest_own = C.newest_own          # a project's own last change, dependencies not counted
        self.finish(C, P)
        if C.depth <= KEEP_DEPTH and C.bytes >= self.keep_min and P.kids is not None:
            node = {"n": C.name, "b": C.bytes, "f": C.files}
            if C.newest:
                node["m"] = iso(C.newest)[:10]      # the newest change anywhere inside it
            if C.own:
                node["o"] = C.own
            if C.kids:
                node["c"] = C.kids
            if rule:
                node["r"] = rule
            if C.partial:
                node["p"] = 1
            P.kids.append(node)

    def finish(self, C, P=None):
        rule = C.rule
        if rule:
            if rule in AGGREGATED:
                a = self.agg.setdefault((rule, C.group), [0, 0, 0, 0.0])
                a[0] += C.bytes
                a[1] += C.files
                a[2] += 1
                a[3] = max(a[3], C.newest)
            elif not C.denied:
                f = {"rule": rule, "path": C.path, "bytes": C.bytes, "files": C.files, "modified": iso(C.newest)}
                if C.group:
                    f["group"] = C.group
                if rule in LEFTOVER_RULES and P is not None:
                    f["project"] = P.path
                if C.partial:
                    f["partial"] = True
                self.findings.append(f)
        if C.left:
            self.projects.append({"path": C.path, "bytes": C.bytes, "leftover_bytes": C.left,
                                  "last_changed": iso(C.newest_own)})
        stale = C.stale
        if (rule is None and not C.inside and not C.system and not C.profile and not C.users and not C.partial
                and 1 <= C.depth <= KEEP_DEPTH + 2 and C.files > 1 and C.bytes >= self.old_bytes
                and 0 < C.newest < self.now - YEAR):
            stale = [(C.bytes, C.path, C.newest, C.files)]   # stands in for anything stale inside it
        if stale:
            if P is None:
                self.stale_folders += stale
            else:
                P.stale = stale if P.stale is None else P.stale + stale

    def walk(self, top, deadline):
        self.stop = False
        stack = [top]
        self.scan(top)
        while stack:
            F = stack[-1]
            if F.subs and not self.stop:
                if time.time() > deadline or self.entries >= self.args.max_entries:
                    self.stop = self.truncated = True
                    continue
                C = self.child(F, F.subs.pop())
                if F.sent:
                    F.sent -= 1
                self.ahead(F)
                stack.append(C)
                self.scan(C)
            else:
                if F.subs:
                    F.partial = True             # time ran out before its subfolders were measured
                    for s in F.subs:
                        if s[3] is not None:
                            s[3].cancel()
                F.subs = None
                stack.pop()
                if stack:
                    self.fold(stack[-1], F)
        self.finish(top)
        return top

    # -------------------------------------------------- one drive
    def windows_parts(self, drive, deadline):
        """Measure the few folders inside Windows worth knowing about. The rest of Windows is never walked."""
        win = self.env["windows"]
        if key_of(win)[:len(key_of(drive["root"]))] != key_of(drive["root"]) or not os.path.isdir(win):
            return [], 0
        nodes, total = [], 0
        try:
            with os.scandir(LONG + win) as it:
                for e in it:
                    if e.name.lower() == "memory.dmp" and not e.is_dir(follow_symlinks=False) and not e.is_symlink():
                        st = e.stat(follow_symlinks=False)
                        path, size = os.path.join(win, e.name), st.st_size
                        if st.st_file_attributes & (A_SPARSE | A_COMPRESSED) and not self.apparent:
                            size = on_disk_size(LONG + path, size)
                        self.findings.append({"rule": "memory-dumps", "path": path, "bytes": size, "files": 1,
                                              "modified": iso(st.st_mtime)})
                        nodes.append({"n": e.name, "b": size, "f": 1, "r": "memory-dumps"})
                        total += size
        except OSError:
            self.errors += 1
        for rule, rel in WINDOWS_PROBES:
            path = os.path.join(win, rel)
            try:
                st = os.lstat(path)
            except FileNotFoundError:
                continue
            except OSError:
                self.not_measured.append({"what": RULES[rule][1], "why": "Needs administrator rights."})
                continue
            if not st.st_file_attributes & A_DIRECTORY or st.st_file_attributes & A_REPARSE:
                continue
            top = Frame(path, key_of(path), rel, 2)
            top.rule, top.system = rule, True
            before = len(self.skipped)
            self.walk(top, deadline)
            if top.denied:
                del self.skipped[before:]
                self.not_measured.append({"what": RULES[rule][1], "why": "Needs administrator rights."})
                continue
            nodes.append({"n": rel, "b": top.bytes, "f": top.files, "r": rule})
            total += top.bytes
        return nodes, total

    def survey_drive(self, drive, deadline):
        root = drive["root"]
        top = Frame(root, key_of(root), root, 0)
        top.user = True                          # folders you made at the top of a drive count as yours
        if self.test:
            top.profile = True
        self.keep_min = 4 * MB
        self.walk(top, deadline)
        seen = top.bytes
        kids = top.kids or []
        if not self.test:
            rb = recycle_bin(root)
            if rb is None:
                self.not_measured.append({"what": "Recycle Bin on " + root, "why": "Windows didn't report its size."})
            elif rb["bytes"] or rb["items"]:
                self.findings.append({"rule": "recycle-bin", "path": root + "$Recycle.Bin", "bytes": rb["bytes"],
                                      "files": rb["items"], "modified": None})
                kids.append({"n": "Recycle Bin", "b": rb["bytes"], "f": rb["items"], "r": "recycle-bin"})
                seen += rb["bytes"]
        win_nodes, win_bytes = self.windows_parts(drive, deadline)
        seen += win_bytes
        if drive.get("test"):                    # a made-up drive around the test folder, so the page has bars to draw
            drive["total"] = (int(seen * 1.25) // (64 * GB) + 1) * 64 * GB
            drive["free"] = drive["total"] - seen
        used = drive["total"] - drive["free"]
        rest = max(0, used - seen)
        is_system = bool(win_nodes) or key_of(self.env["windows"]).startswith(key_of(root))
        if is_system and not self.test:
            name = "Windows and system files (worked out, not measured)"
            win_nodes.append({"n": "Windows itself, System Restore, and places only an administrator can open",
                              "b": rest, "f": 0, "est": 1})
            kids.append({"n": name, "b": rest + win_bytes, "f": 0, "c": win_nodes, "est": 1})
        else:
            if win_nodes:
                kids.append({"n": "Windows (the measured parts)", "b": win_bytes, "f": 0, "c": win_nodes})
            if rest > max(used // 100, 64 * MB):
                kids.append({"n": "Not counted (system folders, other accounts, no access)", "b": rest, "f": 0, "est": 1})
        floor = max(int(used * 0.0005), 16 * MB)
        tree = {"n": root, "b": used, "f": top.files, "o": top.own, "c": kids}
        prune(tree, floor)
        drive.update(used=used, seen_bytes=seen, system=is_system, truncated=bool(top.partial), tree=tree,
                     files=top.files, folders=top.dirs, online_only_bytes=top.cloud)

    def run(self):
        left = len(self.drives)
        sysroot = key_of(self.env["windows"])
        self.drives.sort(key=lambda d: 0 if sysroot.startswith(key_of(d["root"])) else 1)
        try:
            for i, d in enumerate(self.drives):
                remaining = self.args.max_seconds - (time.time() - self.t0)
                share = remaining / left
                if i == 0 and left > 1:          # the drive with Windows and your profile gets most of the time
                    share = remaining * 0.7
                t = time.time()
                self.survey_drive(d, t + max(5.0, share))
                d["seconds"] = round(time.time() - t, 1)
                left -= 1
        finally:
            if self.pool is not None:
                self.pool.shutdown(wait=True)
        return self.report()

    # -------------------------------------------------- report
    def report(self):
        for (rule, group), (nbytes, files, n, newest) in self.agg.items():
            f = {"rule": rule, "path": None, "bytes": nbytes, "files": files, "count": n, "modified": iso(newest)}
            if group:
                f["group"] = group
            self.findings.append(f)
        if self.other_users:
            self.not_measured.append({"what": "%d other account%s on this PC" % (self.other_users, "" if self.other_users == 1 else "s"),
                                      "why": "This survey only looks at your own account's folders."})
        if not self.test:
            self.not_measured.append({"what": "System Restore points and shadow copies (System Volume Information)",
                                      "why": "Needs administrator rights. Their space is inside the worked-out Windows figure."})
        for d in self.other_drives:
            self.not_measured.append({"what": "Drive " + d["root"], "why": d["why"][:1].upper() + d["why"][1:] + "."})
        seen_what, unique = set(), []
        for m in self.not_measured:
            if m["what"] not in seen_what:
                seen_what.add(m["what"])
                unique.append(m)
        self.not_measured = unique
        totals, cats = {}, {}
        for f in self.findings:
            t = totals.setdefault(f["rule"], {"bytes": 0, "count": 0})
            t["bytes"] += f["bytes"]
            t["count"] += f.get("count", 1)
        for rule, t in totals.items():
            c = cats.setdefault(RULES[rule][0], {"bytes": 0, "count": 0})
            c["bytes"] += t["bytes"]
            c["count"] += t["count"]

        def rows(items, limit=LIST_MAX):
            return [{"path": p, "bytes": s, "modified": iso(m)} for s, p, m in sorted(items, reverse=True)[:limit]]

        stale = sorted(self.stale_folders, reverse=True)[:100]
        stale_keys = [key_of(p) + "\\" for _, p, _, _ in stale]
        old_files = [x for x in self.old_files if not any(key_of(x[1]).startswith(k) for k in stale_keys)]
        old = ([{"path": p, "bytes": s, "modified": iso(m), "folder": True, "files": n} for s, p, m, n in stale]
               + rows(old_files))
        old.sort(key=lambda r: -r["bytes"])
        big = sorted(self.big, reverse=True)
        same = {}
        for s, p, m, _, mine in big:
            if mine:
                same.setdefault(s, []).append(p)
        lists = {
            "old_installers": rows(self.old_installers), "big_videos": rows(self.big_videos),
            "disk_images": rows(self.disk_images), "model_files": rows(self.model_files), "old_large": old[:LIST_MAX],
            "same_size": [{"bytes": s, "paths": ps} for s, ps in sorted(same.items(), reverse=True) if len(ps) > 1][:50],
        }
        for cat, src in (("downloads", self.old_installers), ("videos", self.big_videos),
                         ("images", self.disk_images), ("models", self.model_files)):
            if src:
                cats[cat] = {"bytes": sum(x[0] for x in src), "count": len(src)}
        if old:
            cats["old"] = {"bytes": sum(r["bytes"] for r in old), "count": len(old)}
        per_rule, findings = {}, []
        for f in sorted(self.findings, key=lambda f: -f["bytes"]):   # keep the biggest of each kind; totals stay whole
            per_rule[f["rule"]] = per_rule.get(f["rule"], 0) + 1
            if per_rule[f["rule"]] <= LIST_MAX:
                findings.append(f)
        self.projects.sort(key=lambda p: -p["leftover_bytes"])
        return {
            "schema": SCHEMA,
            "generated": datetime.now().isoformat(timespec="seconds"),
            "options": {"max_seconds": self.args.max_seconds, "apparent_size": bool(self.args.apparent_size),
                        "old_gb": self.args.old_gb, "test_root": bool(self.args.root)},
            "budget": {"entries_seen": self.entries, "seconds": round(time.time() - self.t0, 1), "truncated": self.truncated,
                       "errors": self.errors, "links_left_alone": self.links, "online_only_folders": self.cloud_dirs},
            "profile": self.env["profile"],
            "drives": self.drives,
            "categories": [{"id": c, "title": t, "about": a, "bytes": cats.get(c, {}).get("bytes", 0),
                            "count": cats.get(c, {}).get("count", 0)} for c, t, a in CATEGORIES],
            "rules": {r: {"category": v[0], "title": v[1], "about": v[2], "bytes": totals.get(r, {}).get("bytes", 0),
                          "count": totals.get(r, {}).get("count", 0)} for r, v in RULES.items() if r in totals},
            "findings": findings,
            "projects": self.projects[:LIST_MAX],
            "project_count": len(self.projects),
            "lists": lists,
            "big_files": [{"path": p, "bytes": s, "modified": iso(m), "in": r} for s, p, m, r, _ in big[:150]],
            # bytes by the year files were last changed; "older" holds everything before the first year
            "ages": {d: {("older" if y < 0 else str(self.year0 + y)): b for y, b in sorted(h.items())}
                     for d, h in self.ages.items() if h},
            "not_measured": self.not_measured,
            "skipped": self.skipped[:100],
            "skipped_count": self.errors,
        }


def prune(node, floor):
    """Keep the biggest few subfolders of each folder, so the chart data stays small."""
    kids = node.get("c")
    if not kids:
        node.pop("c", None)
        return
    kids.sort(key=lambda k: -k["b"])
    limit = max(floor, node["b"] // 200)
    keep = [k for k in kids[:TREE_FANOUT] if k["b"] >= limit]
    for k in keep:
        prune(k, floor)
    if keep:
        node["c"] = keep
    else:
        node.pop("c")


def human(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%d %s" % (n, unit)) if unit == "B" else ("%.1f %s" % (n, unit))
        n /= 1024.0


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Read-only survey of where disk space went (disk-space-report skill).")
    ap.add_argument("--out", required=True, help="where to write the JSON report (the only file this script writes)")
    ap.add_argument("--drives", help="drive letters to survey, e.g. C,D (default: every local fixed disk)")
    ap.add_argument("--root", help="survey only this folder, as if it were both the drive and your profile "
                                   "(no real drives, no registry); for testing")
    ap.add_argument("--max-seconds", type=float, default=420, help="stop walking after this long (default 420)")
    ap.add_argument("--threads", type=int, default=min(4, os.cpu_count() or 2),
                    help="folder listings read at once (default: up to 4; 1 reads one at a time)")
    ap.add_argument("--max-entries", type=int, default=8_000_000, help="stop after this many entries (default 8M)")
    ap.add_argument("--old-gb", type=float, default=2.0,
                    help="'big and untouched for a year' starts at this many GB (default 2)")
    ap.add_argument("--apparent-size", action="store_true",
                    help="count sparse and compressed files at their full length instead of the space they take "
                         "(like du --apparent-size); for testing with sparse files")
    args = ap.parse_args(argv)
    if not IS_WIN and not args.root:
        sys.exit("This survey is for Windows. Use --root <folder> to try it on a test folder.")
    out = os.path.abspath(args.out)
    if not os.path.isdir(os.path.dirname(out)):
        sys.exit("Output folder does not exist: %s (this script creates no folders)" % os.path.dirname(out))
    if args.root and not os.path.isdir(args.root):
        sys.exit("--root is not a folder: %s" % args.root)
    report = Survey(args).run()
    with open(out, "w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=1)
    b = report["budget"]
    drives = ", ".join("%s %d%% full" % (d["root"] if not d.get("test") else "test folder",
                                         round(100.0 * d["used"] / d["total"]) if d["total"] else 0)
                       for d in report["drives"])
    top = sorted(report["categories"], key=lambda c: -c["bytes"])[:3]
    print("survey: %d entries in %.0fs%s; %s; biggest: %s -> %s" % (
        b["entries_seen"], b["seconds"], " (TRUNCATED: raise --max-seconds)" if b["truncated"] else "", drives,
        ", ".join("%s %s" % (c["title"], human(c["bytes"])) for c in top if c["bytes"]) or "nothing notable", out))


if __name__ == "__main__":
    main()

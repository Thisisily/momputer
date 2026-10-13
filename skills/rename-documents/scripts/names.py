#!/usr/bin/env python3
"""What a file name may be, for the rename-documents skill.

One definition, used twice: when the plan is built (build_page.py), and again when it is
carried out (apply_plan.py), so a name that didn't pass here can't reach the disk by either
door. Also: which names say nothing, the naming rules on offer, and where to look by default.

Python 3.8+, standard library only. Reads folder listings; changes nothing.
"""
import os
import re
import sys
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
import safe_apply  # noqa: E402  (where the person's folders are; what an installed program's folder looks like)

MAX_NAME = 120          # characters in a new name, extension included
MAX_PATH = 240          # characters in the folder plus the new name
FORBIDDEN = '<>:"|?*'   # Windows doesn't allow these in a name ("/" and "\\" are handled as path separators)
RESERVED = ({"con", "prn", "aux", "nul", "conin$", "conout$"} | {"com%s" % d for d in "0123456789¹²³"}
            | {"lpt%s" % d for d in "0123456789¹²³"})
# Unicode categories that have no place in a file name: control, format (zero-width and
# direction-changing characters), surrogates, private use, unassigned, line and paragraph separators
BAD_CATEGORIES = {"Cc", "Cf", "Cs", "Co", "Cn", "Zl", "Zp"}
JOINERS = "‌‍"  # zero-width non-joiner and joiner: part of how Persian and some Indic words are spelled

DOC_EXTS = set("pdf doc docx odt rtf txt md xls xlsx ods csv ppt pptx odp jpg jpeg png tif tiff heic webp bmp gif".split())
PROJECT_MARKERS = {".git", "package.json", "cargo.toml", "pyproject.toml", "go.mod", "pom.xml", "build.gradle",
                   "cmakelists.txt", "composer.json", "gemfile", "pubspec.yaml", "node_modules", ".venv"}

DATE = r"\d{4}(-\d{2}(-\d{2})?)?"
RULES = {
    "date-who-what": {"label": "Date first", "example": "2024-03-14 Lanternfall invoice 1182.pdf",
                      "how": "The date as YYYY-MM-DD, then who it is from, what it is, and its number if it has one. Sorts by time.",
                      "fits": r"^%s \S" % DATE},
    "who-what-date": {"label": "Who first", "example": "Lanternfall invoice 1182 2024-03-14.pdf",
                      "how": "Who it is from, what it is, its number, then the date. Sorts by sender.",
                      "fits": r"^\D.* %s$" % DATE},
    "what-who-date": {"label": "Kind first", "example": "Invoice Lanternfall 1182 2024-03-14.pdf",
                      "how": "What it is, who it is from, its number, then the date. Sorts by kind of paper.",
                      "fits": r"^\D.* %s$" % DATE},
}

COPY_MARK = re.compile(r"( \(\d+\)| - copy( \(\d+\))?|_copy\d*)$", re.I)
SAYS_NOTHING = [
    ("camera", "a camera or phone default", re.compile(
        r"^(img|dsc|dscn|dscf|pxl|mvimg|cimg|sam|vid|mov|aud|ptt|p)[ _\-]?\d{3,}([ _\-](wa)?\d+)*$")),
    ("scanner", "a scanner default", re.compile(
        r"^(scan|scanned|scn|skm|hpscan|epson|brn|fax|camscanner|adobe scan|genius scan|microsoft lens|office lens)"
        r"([ _\-]?(document|image|page))?[ _\-.\d]*$")),
    ("untitled", "\"Untitled\" or \"New document\"", re.compile(
        r"^(untitled|unnamed|noname|no name|new document|new microsoft word document|new text document|new rich text document|"
        r"new bitmap image|new microsoft excel worksheet|new microsoft powerpoint presentation|unbenannt|sans titre)[ _\-]?\d*$")),
    ("generic", "one everyday word", re.compile(
        r"^(document|doc|file|image|photo|picture|pic|page|download|attachment|export|output|print|temp|tmp|new|copy)[ _\-]?\d*$")),
    ("digits", "only digits", re.compile(r"^(?=.*\d)[\d\s_\-.()]+$")),
    ("code", "a long code", re.compile(r"^([0-9a-f]{16,}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$")),
]


def random_looking(stem):
    """A long run of letters and digits that keeps switching between the two: an id, not words.
    ("lanternfall-invoice-20240314" switches once; "a8f3k2m9x1q7w5z0b4n6" switches all the way.)"""
    token = stem.replace("-", "").replace("_", "")
    if len(token) < 20 or not re.fullmatch(r"[a-z0-9]+", token):
        return False
    return sum(1 for a, b in zip(token, token[1:]) if a.isdigit() != b.isdigit()) >= 6


def says_nothing(name):
    """(key, description) if a file's name says nothing about what it is, else None."""
    stem = os.path.splitext(name)[0].strip().lower()
    stem = COPY_MARK.sub("", stem).strip()
    for key, label, rx in SAYS_NOTHING:
        if rx.match(stem):
            return key, label
    if random_looking(stem):
        return "code", "a long code"
    return None


def unseen(text, i):
    """True if the character at `i` is invisible or changes the direction of the text around it."""
    ch = text[i]
    if unicodedata.category(ch) not in BAD_CATEGORIES:
        return False
    return not (ch in JOINERS and 0 < i < len(text) - 1 and text[i - 1].isalpha() and text[i + 1].isalpha())


def bad_character(text):
    """The first character in `text` that may not be in a file name, or None."""
    for i, ch in enumerate(text):
        if ch in "/\\" or ch in FORBIDDEN or unseen(text, i):
            return ch
    return None


def shown(text, limit=150):
    """`text` made safe to look at: anything invisible or direction-changing is spelled out as \\uXXXX,
    so a name with a right-to-left override in it can't pass itself off as another kind of file."""
    if not isinstance(text, str):
        text = repr(text)
    cut = text[:limit]
    return "".join("\\u%04x" % ord(ch) if unseen(cut, i) else ch for i, ch in enumerate(cut)) + ("..." if len(text) > limit else "")


def why_bad(old_name, new_name, folder=None):
    """None if `new_name` may replace `old_name` in the same folder, else why not."""
    if not isinstance(old_name, str) or not isinstance(new_name, str):
        return "not text"
    if not new_name.strip():
        return "empty"
    if "/" in new_name or "\\" in new_name:
        return "a path, not a name (it has a slash in it)"
    ch = bad_character(new_name)
    if ch is not None:
        if ch in FORBIDDEN:
            return "has a character Windows doesn't allow in names (%s)" % ch
        return "has an invisible or direction-changing character in it (U+%04X)" % ord(ch)
    if new_name != new_name.strip(" .") or new_name[0] in " .":
        return "starts or ends with a space or a dot"
    if not new_name[0].isalnum():
        return "doesn't start with a letter or a digit"
    if new_name.split(".")[0].rstrip(" ").lower() in RESERVED:
        return "a name Windows keeps for a device (%s)" % new_name.split(".")[0].rstrip(" ").upper()
    old_stem, old_ext = os.path.splitext(old_name)
    stem, ext = os.path.splitext(new_name)
    if ext.lower() != old_ext.lower():
        return "a different extension (%s instead of %s): a rename never changes what kind of file it is" % (ext or "none", old_ext or "none")
    if not stem or stem != stem.rstrip(" ."):
        return "has a space or a dot just before the extension"
    if len(new_name) > MAX_NAME:
        return "too long (%d characters; %d at most)" % (len(new_name), MAX_NAME)
    if folder and len(folder) + 1 + len(new_name) > MAX_PATH:
        return "too long for the folder it is in (the whole path would be over %d characters)" % MAX_PATH
    if new_name == old_name:
        return "the same as the name it already has"
    if new_name.lower() == old_name.lower():
        return "the name it already has with different capitals"
    return None


def follows(rule, name):
    """True if `name` has the shape the chosen naming rule asks for."""
    r = RULES.get(rule)
    return bool(r and re.search(r["fits"], os.path.splitext(name)[0]))


def default_folders(root=None):
    """Where scans and downloads usually land: Downloads, and the scan folders if they exist."""
    kf = safe_apply.known_folders(root)
    docs = kf.get("Documents") or os.path.join(kf["Profile"], "Documents")
    pics = kf.get("Pictures") or os.path.join(kf["Profile"], "Pictures")
    cands = [kf.get("Downloads"), os.path.join(docs, "Scans"), os.path.join(docs, "Scanned Documents"), os.path.join(pics, "Scans")]
    return [c for c in cands if c and os.path.isdir(c)]


def not_for_renaming(folder):
    """Why nothing in `folder` should be renamed (a code project, a program's own folder, unreadable), or None."""
    try:
        names = {n.lower() for n in os.listdir(safe_apply.xp(folder))}
    except OSError:
        return "a folder that can't be listed"
    if names & PROJECT_MARKERS or any(n.endswith((".sln", ".csproj", ".xcodeproj")) for n in names):
        return "a code project"
    if any(n.startswith("unins") and n.endswith(".exe") for n in names) or "steamapps" in names:
        return "a program's own folder"
    return None


def off_limits(folder, profile):
    """Why nothing in `folder` should be renamed because of where it sits: the folder itself and
    every folder above it are looked at, up to (not including) the profile folder or the drive."""
    stop, first = safe_apply.norm(profile), True
    for _ in range(64):
        if safe_apply.norm(folder) == stop or os.path.dirname(folder) == folder:
            return None
        why = not_for_renaming(folder)
        if why:
            return why if first else "inside " + why
        folder, first = os.path.dirname(folder), False
    return "too deep to check"

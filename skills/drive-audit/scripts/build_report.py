#!/usr/bin/env python3
"""Turn drive.json (what the agent gathered, read-only) into the Drive report page.

    python build_report.py drive.json --out drive-report.html

Writes the page and drive-plan.json next to it: the numbered fixes that an approval like
"DRIVE-APPROVE report=ab12cd34ef unshare=1-3 outside=2 duplicates=1-4 empty=none stale=5"
refers to, and the searches the person can reuse. The page holds file names, types, sizes,
owners' names, sharing state as counts, and dates. Never what is inside a file, and never
the addresses a file is shared with.

The drive.json format is in references/gathering.md. Python 3.8+, standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import sys
from datetime import date, datetime, timedelta
from urllib.parse import quote, unquote

HERE = os.path.dirname(os.path.abspath(__file__))
TEMPLATE = os.path.join(HERE, "..", "assets", "drive-report-template.html")

# Thresholds (all documented in SKILL.md so the agent can explain them)
STALE_DAYS = 365           # not changed and not opened for this long
BIG_ROWS = 25              # how many of the biggest files are listed
MAX_ROWS = 100             # per tickable list
MAX_TABLE = 40
MAX_FILES = 300000
TEXT_MAX = 90
YEARS = 10                 # columns in the "last touched" chart

# ---- safe forms. A file name or an owner's name is written by whoever made or shared the
# file. It is only ever shown as text. The things below are the only ones that reach a link,
# a search, or (later) a Drive tool call, and each must match its pattern in full.
SAFE_FILE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{18,99}")      # starts with a letter or digit: never "-..."
FILE_LINK = re.compile(r"https://drive\.google\.com/open\?id=[A-Za-z0-9][A-Za-z0-9_-]{18,99}")
SEARCH_LINK = re.compile(r"https://drive\.google\.com/drive/search\?q=[A-Za-z0-9%._-]{1,200}")
# Searches are never built from a name. They are these fixed texts, plus one with a date in it.
FIXED_SEARCHES = {"owner:me sharedwith:public", "owner:me sharedwith:external", "owner:me title:Untitled",
                  "owner:me type:video", "owner:me type:image", "owner:me type:pdf", "is:unorganized owner:me", "is:trashed"}
DATED_SEARCH = re.compile(r"owner:me before:\d{4}-\d{2}-\d{2}")
FIXED_PAGES = {"https://drive.google.com/drive/quota", "https://drive.google.com/drive/trash"}
SAFE_DAY = re.compile(r"\d{4}-\d{2}-\d{2}")
SAFE_CHECKSUM = re.compile(r"[0-9a-f]{32,64}")
_LABEL = r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?"
SAFE_DOMAIN = re.compile(_LABEL + r"(?:\." + _LABEL + r")+")
LIST_ACTIONS = {"unshare": "unshare", "outside": "remove_outside", "duplicates": "trash", "empty": "trash", "stale": "trash"}

FREEMAIL = {"gmail.com", "googlemail.com", "outlook.com", "hotmail.com", "live.com", "msn.com", "yahoo.com",
            "ymail.com", "icloud.com", "me.com", "aol.com", "proton.me", "protonmail.com", "gmx.com", "gmx.net",
            "mail.com", "yandex.com", "yandex.ru", "mail.ru", "zoho.com", "qq.com", "163.com"}
ROLES = {"reader": "view", "viewer": "view", "view": "view", "commenter": "comment", "comment": "comment",
         "writer": "edit", "editor": "edit", "edit": "edit", "fileorganizer": "edit", "organizer": "edit"}
ROLE_RANK = {"": 0, "view": 1, "comment": 2, "edit": 3}
CAN = {"view": "open it", "comment": "open it and comment", "edit": "edit it"}
TYPE_LABELS = {"folder": "Folders", "shortcut": "Shortcuts", "doc": "Google Docs", "sheet": "Google Sheets",
               "slides": "Google Slides", "form": "Google Forms", "gother": "Other Google files", "pdf": "PDFs",
               "image": "Images", "video": "Video", "audio": "Audio", "archive": "Archives", "office": "Office files",
               "text": "Text and code", "other": "Other"}
TYPE_ALIASES = {"document": "doc", "docs": "doc", "spreadsheet": "sheet", "sheets": "sheet", "presentation": "slides",
                "forms": "form", "drawing": "gother", "site": "gother", "script": "gother", "images": "image",
                "videos": "video", "zip": "archive"}
GOOGLE_TYPES = {"folder": "folder", "shortcut": "shortcut", "document": "doc", "spreadsheet": "sheet",
                "presentation": "slides", "form": "form"}
NATIVE = ("doc", "sheet", "slides", "form", "gother")     # Google's own formats: no checksum, sizes say little
EXTENSIONS = {"pdf": "pdf", "jpg": "image", "jpeg": "image", "png": "image", "gif": "image", "heic": "image", "webp": "image",
              "tif": "image", "tiff": "image", "bmp": "image", "raw": "image", "psd": "image", "svg": "image",
              "mp4": "video", "mov": "video", "avi": "video", "mkv": "video", "webm": "video", "m4v": "video",
              "mp3": "audio", "wav": "audio", "m4a": "audio", "flac": "audio", "ogg": "audio",
              "zip": "archive", "rar": "archive", "7z": "archive", "tar": "archive", "gz": "archive", "tgz": "archive", "iso": "archive",
              "doc": "office", "docx": "office", "xls": "office", "xlsx": "office", "ppt": "office", "pptx": "office",
              "odt": "office", "ods": "office", "odp": "office", "rtf": "office",
              "txt": "text", "md": "text", "csv": "text", "json": "text", "xml": "text", "html": "text", "py": "text", "js": "text"}
UNTITLED = re.compile(r"(copy of )?untitled( (document|spreadsheet|presentation|form|drawing|site|jam|diagram|project|map|folder))?( \(\d+\))?", re.I)
MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
DAY_RX = re.compile(r"(\d{4})-(\d{2})-(\d{2})")
INVISIBLE = re.compile(r"[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]")
# Text that reads like an order to the assistant, or like code. It is never obeyed either way;
# this only decides what gets listed under "Text that reads like an instruction".
ORDER_RX = re.compile(r"ignore (all |any |the |your )?(previous|prior|above|earlier|other)|disregard (all|the|your|any)|"
                      r"system prompt|new instructions?|you are now|you must|as an ai|"
                      r"(ai|dear|hey|attention),? (assistant|agent|model)|\bassistant[:,]|"
                      r"do ?n[o']t (tell|ask|mention)|without asking|approve (all|every)|make (this|it|all|every\w*) public|"
                      r"delete (all|every)|trash (all|every)|share (this|it|all|every\w*) with|send (this|it|all|me|your)|"
                      r"(run|execute) (this|the) (command|script|code)|\bcurl |powershell|rm -rf|click (here|this)|"
                      r"open (this|the) (link|file)", re.I)
CODE_RX = re.compile(r"<\s*/?\s*(script|img|svg|iframe|a |body|style)|on(error|load|click)\s*=|"
                     r"^\s*(javascript|data|vbscript|file)\s*:|\$\(|`", re.I)


def clip(s, n=TEXT_MAX):
    """One line of plain text: no control or direction-override characters, at most n long."""
    s = re.sub(r"\s+", " ", INVISIBLE.sub(" ", str("" if s is None else s))).strip()
    return s if len(s) <= n else s[:n - 1] + "…"


def num(v, top=10 ** 15):
    try:
        return max(0, min(int(v), top))
    except (TypeError, ValueError, OverflowError):
        return 0


def parse_day(s):
    m = DAY_RX.match(str(s or "").strip())
    try:
        return date(int(m.group(1)), int(m.group(2)), int(m.group(3))) if m else None
    except ValueError:
        return None


def iso(d):
    return d.isoformat() if d else ""


def size_label(n):
    units, i, n = ["B", "KB", "MB", "GB", "TB"], 0, float(n)
    while n >= 1024 and i < len(units) - 1:
        n, i = n / 1024, i + 1
    return ("%.1f %s" if i and n < 10 else "%.0f %s") % (n, units[i])


def people(n, word="person", many="people"):
    return "%d %s" % (n, word if n == 1 else many)


def mask(account):
    account = clip(account, 80)
    if "@" not in account:
        return account
    user, dom = account.split("@", 1)
    return user[:1] + "•••@" + dom


def file_link(fid):
    """The only file address the page links to, built from an id that matched SAFE_FILE_ID in full."""
    return "https://drive.google.com/open?id=" + fid if isinstance(fid, str) and SAFE_FILE_ID.fullmatch(fid) else ""


def search(title, why, query):
    """A Drive search the person can reuse. Only fixed texts get through; never anything from a file."""
    if query not in FIXED_SEARCHES and not DATED_SEARCH.fullmatch(query):
        sys.exit("refusing to build a search that isn't on the list: %r" % query)
    return {"title": title, "why": why, "query": query, "link": "https://drive.google.com/drive/search?q=" + quote(query, safe="")}


def role(v):
    return "view" if v is True else ROLES.get(str(v or "").strip().lower(), "")


def type_of(raw, name):
    if raw.get("folder") is True:
        return "folder"
    t = str(raw.get("type") or "").strip().lower()
    t = TYPE_ALIASES.get(t, t)
    if t in TYPE_LABELS:
        return t
    mime = str(raw.get("mime") or raw.get("mimeType") or "").strip().lower()
    if mime.startswith("application/vnd.google-apps."):
        return GOOGLE_TYPES.get(mime.rsplit(".", 1)[-1], "gother")
    if mime == "application/pdf":
        return "pdf"
    for prefix, kind in (("image/", "image"), ("video/", "video"), ("audio/", "audio"), ("text/", "text")):
        if mime.startswith(prefix):
            return kind
    if any(x in mime for x in ("zip", "compressed", "x-tar", "x-7z", "x-rar", "gzip")):
        return "archive"
    if any(x in mime for x in ("msword", "officedocument", "ms-excel", "ms-powerpoint", "opendocument")):
        return "office"
    return EXTENSIONS.get(name.rsplit(".", 1)[-1].lower() if "." in name else "", "other")


def norm_sharing(sh, own):
    """Sharing as counts. Addresses are never read here: at most the domain part, to tell inside from outside."""
    sh = sh if isinstance(sh, dict) else {}
    found = sh.get("anyone_can_find") is True
    anyone = role(sh.get("anyone")) or ("view" if found else "")
    count, outside, personal, names = num(sh.get("people"), 100000), 0, 0, []
    doms = sh.get("domains") if isinstance(sh.get("domains"), dict) else None
    if doms is not None:
        total = 0
        for d, c in list(doms.items())[:500]:
            d, c = str(d).strip().lower(), num(c, 100000)
            if not c:
                continue
            total += c
            if not (own and (d == own or d.endswith("." + own))):
                outside += c
                if SAFE_DOMAIN.fullmatch(d) and len(d) <= 60 and d not in names:
                    names.append(d)
            personal += c if d in FREEMAIL else 0
        count = max(count, total)
    else:
        outside, personal = num(sh.get("outside"), 100000), num(sh.get("personal"), 100000)
        count = max(count, outside)
    if not own:
        outside = count              # a personal account has no "inside": everyone else is someone else
    return {"anyone": anyone, "found": found, "domain_link": role(sh.get("domain")), "people": count,
            "outside": min(outside, count), "personal": min(personal, count), "domains": names[:3],
            "more_domains": max(0, len(names) - 3)}


def note_suspicious(found, where, value):
    if not isinstance(value, str) or len(found) >= 20:
        return
    why = "reads like an instruction" if ORDER_RX.search(value) else ("looks like code" if CODE_RX.search(value) else "")
    row = {"where": where, "text": clip(value, 120), "why": why}
    if why and row not in found:
        found.append(row)


def norm_file(raw, own, suspicious):
    fid = raw.get("id") if isinstance(raw.get("id"), str) else ""
    full = raw.get("name") if isinstance(raw.get("name"), str) else ""
    name = clip(full)
    kind = type_of(raw, full)
    mine = raw.get("mine") is True          # the JSON value true. Anything else means "not the person's"
    checksum = str(raw.get("checksum") or "").strip().lower()
    modified, opened = parse_day(raw.get("modified")), parse_day(raw.get("opened"))
    note_suspicious(suspicious, "file name", full)
    if not mine:
        note_suspicious(suspicious, "owner name", raw.get("owner"))
    return {"id": fid, "ok": bool(SAFE_FILE_ID.fullmatch(fid)), "name": name or "(no name)", "name_key": full.strip().casefold(),
            "type": kind, "size": num(raw.get("size")), "size_known": raw.get("size") is not None, "mine": mine,
            "owner": "you" if mine else (clip(raw.get("owner"), 50) or "someone else"),
            "created": parse_day(raw.get("created")), "modified": modified, "opened": opened,
            "last": max([d for d in (modified, opened) if d] or [None], key=lambda d: d or date.min),
            "trashed": raw.get("trashed") is True, "checksum": checksum if SAFE_CHECKSUM.fullmatch(checksum) else "",
            "empty": raw.get("empty") is True, "sharing": norm_sharing(raw.get("sharing"), own)}


def plan_id(plan):
    """The report id: a fingerprint of every numbered fix and every search. Change one and the id changes."""
    core = json.dumps([plan.get("lists"), plan.get("searches"), plan.get("pages")], sort_keys=True, ensure_ascii=False,
                      separators=(",", ":"))
    return hashlib.sha256(core.encode("utf-8")).hexdigest()[:10]


def check_plan(plan):
    """Problems that make a plan unsafe to act on. Empty when every id, search and link is in a safe form."""
    lists = plan.get("lists") if isinstance(plan, dict) else None
    if not isinstance(lists, dict) or set(lists) != set(LIST_ACTIONS):
        return ["the lists aren't the ones this skill writes"]
    bad = []
    for key, rows in lists.items():
        for i, r in enumerate(rows if isinstance(rows, list) else [None]):
            at = "%s %d" % (key, i + 1)
            if not isinstance(r, dict) or r.get("n") != i + 1 or isinstance(r.get("n"), bool):
                bad.append(at + ": not a numbered row")
                continue
            fid = r.get("file")
            if not (isinstance(fid, str) and SAFE_FILE_ID.fullmatch(fid)):
                bad.append(at + ": file id %r" % (fid,))
            elif r.get("link") != "https://drive.google.com/open?id=" + fid or not FILE_LINK.fullmatch(r["link"]):
                bad.append(at + ": link %r" % (r.get("link"),))
            if r.get("action") != LIST_ACTIONS[key]:
                bad.append(at + ": action %r" % (r.get("action"),))
            if r.get("mine") is not True:
                bad.append(at + ": not marked as the person's own file")
            keep = r.get("keep_file", "")
            if keep != "" and not (isinstance(keep, str) and SAFE_FILE_ID.fullmatch(keep)):
                bad.append(at + ": kept file id %r" % (keep,))
            lc = r.get("loses_count")
            if not (lc is None or (isinstance(lc, int) and not isinstance(lc, bool) and lc >= 0)) or not isinstance(r.get("cuts"), bool):
                bad.append(at + ": who loses access")
    for i, s in enumerate(plan.get("searches") if isinstance(plan.get("searches"), list) else [None]):
        q = s.get("query") if isinstance(s, dict) else None
        if not isinstance(q, str) or (q not in FIXED_SEARCHES and not DATED_SEARCH.fullmatch(q)):
            bad.append("search %d: %r isn't one of the fixed searches" % (i + 1, q))
        elif not (isinstance(s.get("link"), str) and SEARCH_LINK.fullmatch(s["link"]) and unquote(s["link"].split("?q=", 1)[1]) == q):
            bad.append("search %d: link %r" % (i + 1, s.get("link")))
    for i, p in enumerate(plan.get("pages") if isinstance(plan.get("pages"), list) else [None]):
        if not isinstance(p, dict) or p.get("link") not in FIXED_PAGES:
            bad.append("page %d: link %r" % (i + 1, p.get("link") if isinstance(p, dict) else p))
    return bad


def trash_clashes(rows):
    """Trash fixes that can't go together, as sentences. `rows` is [("stale=3", row), ...].

    A duplicate's row promises that one copy stays (`keep_file`). If that copy is itself among
    the rows, the promise is broken: of two copies, none would be left. The same file twice is
    a slip too. The builder checks every Trash row it writes this way, and check_approval.py
    checks the ones a person ticked, which covers a plan written before the kept copy was left
    out of the other lists."""
    going, bad = {}, []
    for at, r in rows:
        if r.get("action") != "trash":
            continue
        if r.get("file") in going:
            bad.append("%s and %s are the same file" % (going[r.get("file")], at))
        going.setdefault(r.get("file"), at)
    for at, r in rows:
        keep = r.get("keep_file") if r.get("action") == "trash" else ""
        if keep and keep in going:
            bad.append("%s would move to the Trash the copy that %s keeps" % (going[keep], at))
    return bad


def check_links(obj):
    """Every other link in the data (the tables) must be a file link this script built, or empty."""
    bad = []
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k == "link":
                if v != "" and not (isinstance(v, str) and FILE_LINK.fullmatch(v)):
                    bad.append("link %r" % (v,))
            else:
                bad += check_links(v)
    elif isinstance(obj, list):
        for v in obj:
            bad += check_links(v)
    return bad


def build(box):
    suspicious = []
    today = parse_day(box.get("today")) or date.today()
    account = box.get("account") if isinstance(box.get("account"), str) else ""
    own = str(box.get("domain") or (account.rsplit("@", 1)[-1] if "@" in account else "")).strip().lower()
    own = own if SAFE_DOMAIN.fullmatch(own) and own not in FREEMAIL and len(own) <= 60 else ""
    cutoff = today - timedelta(days=STALE_DAYS)

    files, seen, odd_ids, unreadable = [], set(), 0, 0
    for raw in (box.get("files") if isinstance(box.get("files"), list) else [])[:MAX_FILES]:
        if not isinstance(raw, dict):
            unreadable += 1
            continue
        f = norm_file(raw, own, suspicious)
        if f["ok"]:
            if f["id"] in seen:
                continue             # the same file listed twice (it showed up in two searches)
            seen.add(f["id"])
        else:
            odd_ids += 1
        files.append(f)
    live = [f for f in files if not f["trashed"]]
    mine = [f for f in live if f["mine"]]
    theirs = [f for f in live if not f["mine"]]
    stuff = [f for f in mine if f["type"] not in ("folder", "shortcut")]          # things that take up space
    binned = [f for f in files if f["trashed"] and f["mine"]]

    def fixable(f):
        return f["ok"] and f["mine"] and not f["trashed"]

    def shared(f):
        return bool(f["sharing"]["anyone"]) or f["sharing"]["people"] > 0

    def loses(f):
        """What moving this file to the Trash does to other people, as a count."""
        s = f["sharing"]
        if not shared(f):
            return {"loses": "Nobody else has access.", "loses_count": 0, "cuts": False}
        who = ([people(s["people"])] if s["people"] else []) + (["anyone with the link"] if s["anyone"] else [])
        return {"loses": "Shared with %s. Their access ends when the Trash empties after 30 days." % " and ".join(who),
                "loses_count": s["people"], "cuts": True}

    def row(f, action, why, **more):
        r = {"file": f["id"], "name": f["name"], "kind": TYPE_LABELS[f["type"]], "size": f["size"], "mine": True,
             "modified": iso(f["modified"]), "opened": iso(f["opened"]), "link": file_link(f["id"]), "action": action,
             "why": why, "people": f["sharing"]["people"], "public": bool(f["sharing"]["anyone"])}
        r.update(more)
        return r

    def untouched(f):
        if not f["last"] or f["last"] >= cutoff:
            return ""
        years = (today - f["last"]).days // 365
        return "Not opened or changed since %s (over %s)" % (iso(f["last"]), "a year" if years <= 1 else "%d years" % years)

    # ---- 1. anyone with the link
    unshare = []
    for f in sorted((f for f in mine if f["sharing"]["anyone"] and fixable(f)),
                    key=lambda f: (not f["sharing"]["found"], -ROLE_RANK[f["sharing"]["anyone"]], f["type"] != "folder",
                                   f["last"] or today, -f["size"], f["id"])):
        s = f["sharing"]
        why = ["Anyone with the link can %s%s" % (CAN[s["anyone"]], ", and it can turn up in web searches" if s["found"] else "")]
        if f["type"] == "folder":
            why.append("A folder: everything inside is open the same way")
        if untouched(f):
            why.append(untouched(f))
        why.append("%s added by name keep%s access" % (people(s["people"]), "s" if s["people"] == 1 else "") if s["people"]
                   else "Nobody was added by name, so only you keep access")
        unshare.append(row(f, "unshare", why, role=s["anyone"], keeps=s["people"], loses_count=None, cuts=True,
                           loses="Anyone who was using only the link loses access. Drive can't say how many that is."))

    # ---- 2. people outside the domain (or, for a personal account, anyone at all)
    outside = []
    for f in sorted((f for f in mine if f["sharing"]["outside"] > 0 and fixable(f)),
                    key=lambda f: (-f["sharing"]["personal"], -f["sharing"]["outside"], f["type"] != "folder", f["id"])):
        s = f["sharing"]
        if own:
            why = ["%s outside %s can open it" % (people(s["outside"]), own)]
            if s["personal"]:
                why.append("%d of them at a personal address" % s["personal"] if s["outside"] > 1 else "A personal address")
            if s["domains"]:
                why.append("At: " + ", ".join(s["domains"]) + (" and %d more" % s["more_domains"] if s["more_domains"] else ""))
        else:
            why = ["Shared with %s" % people(s["outside"])]
        if f["type"] == "folder":
            why.append("A folder: they can open everything inside")
        if untouched(f):
            why.append(untouched(f))
        outside.append(row(f, "remove_outside", why, loses_count=s["outside"], cuts=True, stays=s["people"] - s["outside"],
                           loses="%s lose%s access." % (people(s["outside"]).capitalize(), "s" if s["outside"] == 1 else "")))

    # ---- 3. duplicates: same checksum, or (for ordinary files) the same name and size
    groups = {}
    for f in stuff:
        if f["size"] <= 0:
            continue
        if f["checksum"]:
            groups.setdefault(("same contents", f["checksum"], f["size"]), []).append(f)
        elif f["type"] not in NATIVE and f["name_key"]:
            groups.setdefault(("same name and size", f["name_key"], f["size"]), []).append(f)
    duplicates, in_trash_list, stays, dup_groups, wasted = [], set(), set(), 0, 0
    for (how, _, size), g in sorted(groups.items(), key=lambda kv: (-kv[0][2] * (len(kv[1]) - 1), kv[0][1])):
        if len(g) < 2:
            continue
        # keep the copy other people use, then the one touched last, then the oldest
        g.sort(key=lambda f: (not shared(f), -(f["last"] or date.min).toordinal(), (f["created"] or date.max).toordinal(), f["id"]))
        keep, dup_groups = g[0], dup_groups + 1
        for f in g[1:]:
            wasted += f["size"]
            if not fixable(f):
                continue
            in_trash_list.add(id(f))
            # This row says the kept copy stays, so the kept copy is offered for the Trash nowhere
            # below, however old or empty it is: ticking both would leave no copy at all.
            stays.add(id(keep))
            duplicates.append(row(f, "trash", ["%s as \"%s\"%s, which stays" % (how.capitalize(), clip(keep["name"], 50),
                                                                                " (last touched %s)" % iso(keep["last"]) if keep["last"] else "")],
                                  keep_file=keep["id"] if keep["ok"] else "", **loses(f)))

    # ---- 4. untitled and empty
    empty = []
    for f in sorted(stuff, key=lambda f: (f["last"] or date.min, f["id"])):
        if id(f) in in_trash_list or id(f) in stays or not fixable(f):
            continue
        blank = f["empty"] or (f["size_known"] and f["size"] == 0 and f["type"] not in NATIVE)
        nameless = bool(UNTITLED.fullmatch(f["name"])) or f["name"] == "(no name)"
        if blank or nameless:
            in_trash_list.add(id(f))
            why = (["Never given a name"] if nameless else []) + (["Empty"] if blank else [])
            empty.append(row(f, "trash", why + ([untouched(f)] if untouched(f) else []), **loses(f)))

    # ---- 5. stale: not opened and not changed for over a year
    old = [f for f in stuff if untouched(f)]
    stale = [row(f, "trash", [untouched(f)], **loses(f))
             for f in sorted(old, key=lambda f: (-f["size"], f["last"], f["id"]))
             if fixable(f) and id(f) not in in_trash_list and id(f) not in stays]

    lists, more = {}, {}
    for key, rows in (("unshare", unshare), ("outside", outside), ("duplicates", duplicates), ("empty", empty), ("stale", stale)):
        more[key] = max(0, len(rows) - MAX_ROWS)
        lists[key] = [dict(r, n=i + 1) for i, r in enumerate(rows[:MAX_ROWS])]

    # ---- what fills the storage
    by_type, by_owner, by_year = {}, {}, {}
    for f in stuff:
        t = by_type.setdefault(TYPE_LABELS[f["type"]], [0, 0])
        t[0], t[1] = t[0] + f["size"], t[1] + 1
        if f["last"]:
            y = by_year.setdefault(max(f["last"].year, today.year - YEARS + 1), [0, 0])
            y[0], y[1] = y[0] + f["size"], y[1] + 1
    for f in theirs:
        if f["type"] not in ("folder", "shortcut"):
            o = by_owner.setdefault(f["owner"], [0, 0])
            o[0], o[1] = o[0] + f["size"], o[1] + 1
    types = sorted(([k, v[0], v[1]] for k, v in by_type.items()), key=lambda r: (-r[1], r[0]))
    owners = sorted(([k, v[0], v[1], False] for k, v in by_owner.items()), key=lambda r: (-r[1], r[0]))
    if len(owners) > 7:
        owners = owners[:7] + [["Everyone else", sum(o[1] for o in owners[7:]), sum(o[2] for o in owners[7:]), False]]
    owners = [["You", sum(f["size"] for f in stuff), len(stuff), True]] + owners
    # A very large Drive: the agent may hand over totals it counted instead of every file.
    counted = False
    if isinstance(box.get("by_type"), list) and box["by_type"]:
        counted = True
        types = sorted(([TYPE_LABELS.get(TYPE_ALIASES.get(str(r.get("type")).lower(), str(r.get("type")).lower()), clip(r.get("type"), 30)),
                         num(r.get("bytes")), num(r.get("count"))] for r in box["by_type"] if isinstance(r, dict)), key=lambda r: (-r[1], r[0]))[:15]
    if isinstance(box.get("by_owner"), list) and box["by_owner"]:
        counted = True
        owners = sorted((["You" if r.get("mine") is True else clip(r.get("owner"), 50) or "someone else", num(r.get("bytes")), num(r.get("count")),
                          r.get("mine") is True] for r in box["by_owner"] if isinstance(r, dict)), key=lambda r: (not r[3], -r[1], r[0]))[:9]
        for r in box["by_owner"]:
            if isinstance(r, dict) and r.get("mine") is not True:
                note_suspicious(suspicious, "owner name", r.get("owner"))
    years = [[("%d" % y) if y > today.year - YEARS + 1 else "≤%d" % y, by_year.get(y, [0, 0])[0], by_year.get(y, [0, 0])[1],
              1 if y < cutoff.year else 0] for y in range(today.year - YEARS + 1, today.year + 1)]
    overview = [["Anyone with the link", sum(1 for f in mine if f["sharing"]["anyone"]), True],
                ["People outside %s" % own if own else "Other people", sum(1 for f in mine if not f["sharing"]["anyone"] and f["sharing"]["outside"]), bool(own)],
                ["Only people at %s" % own if own else "", sum(1 for f in mine if own and not f["sharing"]["anyone"] and not f["sharing"]["outside"]
                                                            and (f["sharing"]["people"] or f["sharing"]["domain_link"])), False],
                ["Only you", sum(1 for f in mine if not f["sharing"]["anyone"] and not f["sharing"]["people"] and not f["sharing"]["domain_link"]), False]]
    overview = [r for r in overview if r[0]]

    def flag(f):
        s = f["sharing"]
        return "anyone with the link" if s["anyone"] else ("outside %s" % own if own and s["outside"] else ("shared" if s["people"] else ""))

    biggest = [{"name": f["name"], "kind": TYPE_LABELS[f["type"]], "size": f["size"], "last": iso(f["last"]), "shared": flag(f),
                "link": file_link(f["id"])} for f in sorted(stuff, key=lambda f: (-f["size"], f["id"]))[:BIG_ROWS] if f["size"] > 0]
    open_theirs = [{"name": f["name"], "owner": f["owner"], "kind": TYPE_LABELS[f["type"]], "role": f["sharing"]["anyone"], "link": file_link(f["id"])}
                   for f in sorted(theirs, key=lambda f: (-ROLE_RANK[f["sharing"]["anyone"]], f["id"])) if f["sharing"]["anyone"]]
    no_fix = [f for f in mine if not f["ok"] and (f["sharing"]["anyone"] or f["sharing"]["outside"])]

    counts = box.get("counts") if isinstance(box.get("counts"), dict) else {}
    storage = box.get("storage") if isinstance(box.get("storage"), dict) else {}
    my_bytes = sum(f["size"] for f in stuff)
    bin_bytes = num(counts.get("trashed_bytes")) or sum(f["size"] for f in binned)
    bin_count = num(counts.get("trashed")) or len(binned)

    # ---- searches the person can reuse. Fixed texts only: nothing here comes from a file.
    searches = [search("Your files that anyone with the link can open", "The place to look after sharing a link in a hurry.", "owner:me sharedwith:public")]
    if own:
        searches.append(search("Your files shared outside your organization", "Work and school accounts only.", "owner:me sharedwith:external"))
    searches.append(search("Your files not changed in over a year", "Sort by size to see what is worth clearing out.", "owner:me before:%s" % cutoff.isoformat()))
    searches.append(search("Files you never named", "Untitled documents, sheets and slides.", "owner:me title:Untitled"))
    for label, title, q in (("Video", "Your videos", "owner:me type:video"), ("Images", "Your images", "owner:me type:image"),
                            ("PDFs", "Your PDFs", "owner:me type:pdf")):
        if any(t[0] == label for t in types[:3]):
            searches.append(search(title, "One of the three kinds of file that take the most room.", q))
    searches.append(search("Files with no folder", "Files of yours left behind when someone deleted the folder they were in.", "is:unorganized owner:me"))
    searches.append(search("What is in the Trash", "It still counts against your storage until it is emptied.", "is:trashed"))
    pages = [{"title": "Storage, biggest first", "why": "Drive's own list of what uses your storage.", "link": "https://drive.google.com/drive/quota"},
             {"title": "Trash", "why": "Emptying it deletes for good, so that step is yours.", "link": "https://drive.google.com/drive/trash"}]

    notes = []
    if odd_ids:
        notes.append("%d files had no id in a plain form, so they have no link and no fix is offered for them%s." % (
            odd_ids, " (%d of them are shared in a way that needs a look; do those by hand)" % len(no_fix) if no_fix else ""))
    if unreadable:
        notes.append("%d rows weren't file records and were skipped." % unreadable)
    plan = {"lists": lists, "searches": searches, "pages": pages}
    report = {
        "account": mask(account), "domain": own, "today": today.isoformat(), "cutoff": cutoff.isoformat(),
        "sources": [clip(s, 40) for s in (box.get("sources") if isinstance(box.get("sources"), list) else [])][:6],
        "complete": bool(box.get("complete", True)) and not counted,
        "agent_notes": clip(box.get("notes"), 400), "headline": clip(box.get("headline"), 240),
        "highlights": [clip(h, 240) for h in (box.get("highlights") if isinstance(box.get("highlights"), list) else [])][:6],
        "storage": {k: num(storage.get(k)) for k in ("used_bytes", "total_bytes", "drive_bytes", "gmail_bytes", "photos_bytes") if storage.get(k) is not None},
        "totals": {"files": num(counts.get("files")) or len(live), "listed": len(live), "mine": len(mine), "mine_bytes": my_bytes,
                   "theirs": len(theirs), "public": len(unshare), "public_edit": sum(1 for r in unshare if r["role"] == "edit"),
                   "outside": len(outside), "outside_people": sum(r["loses_count"] for r in outside),
                   "dup_groups": dup_groups, "dup_bytes": wasted, "empty": len(empty), "stale": len(old),
                   "stale_bytes": sum(f["size"] for f in old), "trash": bin_count, "trash_bytes": bin_bytes,
                   "public_theirs": len(open_theirs)},
        "by_type": types, "by_owner": owners, "by_year": years, "overview": overview,
        "lists": lists, "more": more, "searches": searches, "pages": pages,
        "tables": {"biggest": {"rows": biggest, "more": 0},
                   "public_theirs": {"rows": open_theirs[:MAX_TABLE], "more": max(0, len(open_theirs) - MAX_TABLE)}},
        "suspicious": suspicious, "script_notes": notes,
        "thresholds": {"stale_days": STALE_DAYS, "big_rows": BIG_ROWS},
        "generated": datetime.now().isoformat(timespec="seconds"),
    }
    # Last check before anything is written: every file id, every search and every link is in a
    # form this script built or matched in full. A file name can never get this far as anything but text.
    bad = check_plan(plan) + check_links(report["tables"])
    if bad:
        sys.exit("refusing to write the report: something isn't in a safe form: %s" % "; ".join(bad[:3]))
    # And no two Trash fixes on the page undo each other's promise, even with everything ticked.
    clash = trash_clashes([("%s=%d" % (key, r["n"]), r) for key in ("duplicates", "empty", "stale") for r in lists[key]])
    if clash:
        sys.exit("refusing to write the report: %s." % "; ".join(clash[:3]))
    report["report_id"] = plan_id(plan)
    return report


def embed(obj):
    return (json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
            .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Build the Drive report page from drive.json.")
    ap.add_argument("drive")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write drive-plan.json (default: next to --out)")
    a = ap.parse_args(argv)
    with open(a.drive, encoding="utf-8") as f:
        box = json.load(f)
    if not isinstance(box, dict) or not box.get("files"):
        sys.exit("drive.json has no 'files'. See references/gathering.md.")
    report = build(box)
    with open(TEMPLATE, encoding="utf-8") as f:
        html = f.read()
    if "__DRIVE_DATA__" not in html:
        sys.exit("template is missing the __DRIVE_DATA__ placeholder")
    plan_path = a.plan or os.path.join(os.path.dirname(os.path.abspath(a.out)), "drive-plan.json")
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump({"schema": "drive-audit/plan@1", "report_id": report["report_id"], "today": report["today"], "domain": report["domain"],
                   "lists": report["lists"], "searches": report["searches"], "pages": report["pages"]}, f, ensure_ascii=False, indent=1)
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(html.replace("__DRIVE_DATA__", embed(report)))
    t, n = report["totals"], {k: len(v) for k, v in report["lists"].items()}
    print("page: %s (%d KB)\nplan: %s  report_id=%s\n%d files (%d yours, %s), %d in the Trash (%s)\nfixes offered: %d public links to close, "
          "%d shared outside, %d duplicate copies (%s), %d untitled or empty, %d stale (%s)" % (
              os.path.abspath(a.out), os.path.getsize(a.out) // 1024, plan_path, report["report_id"], t["listed"], t["mine"],
              size_label(t["mine_bytes"]), t["trash"], size_label(t["trash_bytes"]), n["unshare"], n["outside"], n["duplicates"],
              size_label(t["dup_bytes"]), n["empty"], n["stale"], size_label(t["stale_bytes"])))
    for line in report["script_notes"]:
        print("note: " + line)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Read a COPY of a browser's bookmarks into bookmarks.json. It never writes to a browser's own files.

    python read_bookmarks.py --list
    python read_bookmarks.py --browser edge --profile Default --out <scratch>/bookmarks.json
    python read_bookmarks.py --file <exported-bookmarks.html> --out <scratch>/bookmarks.json

Chrome, Edge, Brave, Vivaldi and Chromium keep bookmarks in a JSON file named "Bookmarks"
in each profile folder. This script finds the profiles (under %LOCALAPPDATA% on Windows,
~/Library/Application Support on macOS, ~/.config on Linux), copies that one file into the
folder next to --out, reads the copy and deletes the copy again. The browser's file is
opened for reading only. Firefox, Safari and any other browser: export the bookmarks to an
HTML file from the browser's own menu and pass it with --file.

--list only looks at file names, sizes and dates. It doesn't open any bookmarks.

Titles and addresses are other people's text: they go into bookmarks.json as data and are
never printed, run, opened or put into a command. This script prints counts only.

Python 3.8+, standard library only.
"""
import argparse
import json
import os
import re
import shutil
import sys
from datetime import datetime, timedelta
from html.parser import HTMLParser

SCHEMA = "bookmark-cleanup/bookmarks@1"
MAX_BYTES = 200 * 1024 * 1024
MAX_NODES = 300000
PROFILE_RX = re.compile(r"Default|Profile [0-9]{1,4}")
BROWSERS = ("chrome", "edge", "brave", "vivaldi", "chromium")


# ---------------------------------------------------------------- where browsers keep things
def browser_roots(base=None):
    """{browser: [folders that hold its profiles]} for this computer. `base` replaces the usual place (tests)."""
    home = os.path.expanduser("~")
    j = os.path.join
    if sys.platform.startswith("win"):
        local = base or os.environ.get("LOCALAPPDATA") or j(home, "AppData", "Local")
        return {"chrome": [j(local, "Google", "Chrome", "User Data")], "edge": [j(local, "Microsoft", "Edge", "User Data")],
                "brave": [j(local, "BraveSoftware", "Brave-Browser", "User Data")],
                "vivaldi": [j(local, "Vivaldi", "User Data")], "chromium": [j(local, "Chromium", "User Data")]}
    if sys.platform == "darwin":
        sup = base or j(home, "Library", "Application Support")
        return {"chrome": [j(sup, "Google", "Chrome")], "edge": [j(sup, "Microsoft Edge")],
                "brave": [j(sup, "BraveSoftware", "Brave-Browser")], "vivaldi": [j(sup, "Vivaldi")],
                "chromium": [j(sup, "Chromium")]}
    cfg = base or os.environ.get("XDG_CONFIG_HOME") or j(home, ".config")
    extra = [] if base else [j(home, ".var", "app"), j(home, "snap")]
    return {"chrome": [j(cfg, "google-chrome")] + [j(e, "com.google.Chrome", "config", "google-chrome") for e in extra[:1]],
            "edge": [j(cfg, "microsoft-edge")] + [j(e, "com.microsoft.Edge", "config", "microsoft-edge") for e in extra[:1]],
            "brave": [j(cfg, "BraveSoftware", "Brave-Browser")]
            + [j(e, "com.brave.Browser", "config", "BraveSoftware", "Brave-Browser") for e in extra[:1]],
            "vivaldi": [j(cfg, "vivaldi")],
            "chromium": [j(cfg, "chromium")] + [j(e, "chromium", "common", "chromium") for e in extra[1:]]}


def profiles(base=None):
    """Every profile that has a Bookmarks file: [{browser, profile, path}]. File names only; nothing is opened."""
    found = []
    for browser, roots in browser_roots(base).items():
        for root in roots:
            try:
                names = sorted(os.listdir(root))
            except OSError:
                continue
            for name in names:
                path = os.path.join(root, name, "Bookmarks")
                if PROFILE_RX.fullmatch(name) and os.path.isfile(path) and not os.path.islink(path) \
                        and not os.path.islink(os.path.join(root, name)):
                    found.append({"browser": browser, "profile": name, "path": path})
    return found


def inside_browser_data(path, base=None):
    """True when a path lies inside a folder where a browser keeps its own files. Nothing is ever written there."""
    full = os.path.normcase(os.path.realpath(os.path.abspath(path)))
    spots = [r for roots in browser_roots(base).values() for r in roots]
    home = os.path.expanduser("~")
    spots += [os.path.join(home, "AppData", "Roaming", "Mozilla"), os.path.join(home, ".mozilla"),
              os.path.join(home, "Library", "Application Support", "Firefox"), os.path.join(home, "Library", "Safari"),
              os.path.join(home, "AppData", "Roaming", "Opera Software")]
    for spot in spots:
        s = os.path.normcase(os.path.realpath(spot))
        if full == s or full.startswith(s + os.sep):
            return True
    return os.path.basename(full).lower() in ("bookmarks", "bookmarks.bak") or bool(
        re.search(r"[\\/]user data[\\/]", full + os.sep, re.I))


def plain_name(path):
    """False for a file name that isn't simply a name: 'x:stream' writes into another file on Windows."""
    name = os.path.basename(os.path.abspath(path))
    return bool(name) and ":" not in name and name == name.rstrip(". ")


def may_write(path, ending, mark, base=None):
    """The full path, or stop: this skill writes only plain files of its own, outside every browser's folder.

    `mark` is a piece of text an earlier file of the same kind starts with; any other existing
    file is left alone.
    """
    full = os.path.abspath(path)
    if inside_browser_data(full, base) or not plain_name(full) or not full.lower().endswith(ending):
        sys.exit("%s: this has to be a plain %s file outside any browser's own folder (a scratch folder, the Desktop, "
                 "Documents). This skill never writes anywhere else." % (os.path.basename(full), ending))
    if os.path.exists(full):
        try:
            with open(full, "rb") as f:
                head = f.read(8192).decode("utf-8", "replace")
        except OSError:
            head = ""
        if mark not in head:
            sys.exit("%s already exists and isn't an earlier file of this kind: choose another name (nothing was written)"
                     % os.path.basename(full))
    return full


# ---------------------------------------------------------------- dates
def from_chrome_time(v):
    """Chromium counts microseconds from 1601-01-01. Returns (ISO day, unix seconds) or (None, None)."""
    try:
        us = int(v)
    except (TypeError, ValueError):
        return None, None
    if us <= 0:
        return None, None
    try:
        dt = datetime(1601, 1, 1) + timedelta(microseconds=us)
    except OverflowError:
        return None, None
    if not 1990 <= dt.year <= 2100:
        return None, None
    return dt.date().isoformat(), int((dt - datetime(1970, 1, 1)).total_seconds())


def from_unix(v):
    try:
        ts = int(str(v).strip())
    except (TypeError, ValueError):
        return None, None
    if ts > 10 ** 14:      # microseconds
        ts //= 10 ** 6
    elif ts > 10 ** 11:    # milliseconds
        ts //= 1000
    if not 631152000 <= ts <= 4102444800:  # 1990 to 2100
        return None, None
    return (datetime(1970, 1, 1) + timedelta(seconds=ts)).date().isoformat(), ts


# ---------------------------------------------------------------- the two formats
class Tree:
    def __init__(self):
        self.nodes = []

    def add(self, kind, parent, title, **more):
        if len(self.nodes) >= MAX_NODES:
            sys.exit("more than %d bookmarks and folders: that isn't a bookmarks file this script should read" % MAX_NODES)
        node = {"id": len(self.nodes) + 1, "type": kind, "parent": parent, "title": title if isinstance(title, str) else ""}
        node.update({k: v for k, v in more.items() if v not in (None, "")})
        self.nodes.append(node)
        return node["id"]


def from_chromium(doc):
    """The Bookmarks JSON of a Chromium-family browser as a flat list of nodes."""
    roots = doc.get("roots") if isinstance(doc, dict) else None
    if not isinstance(roots, dict):
        sys.exit("this isn't a Chromium Bookmarks file (no 'roots')")
    tree, has_used = Tree(), False
    todo = []  # (children, parent id), walked without recursion: a file can nest as deep as it likes
    bar = roots.get("bookmark_bar")
    if isinstance(bar, dict):
        todo.append((bar.get("children"), tree.add("folder", 0, bar.get("name") or "Bookmarks bar", root="bar")))
    other = roots.get("other")
    if isinstance(other, dict):
        todo.append((other.get("children"), 0))  # "Other bookmarks" is the top level, as in an exported file
    mobile = roots.get("synced")
    i = 0
    while i < len(todo) or mobile:
        if i == len(todo):  # everything else is in: the phone's bookmarks go last, in a folder of their own
            if isinstance(mobile, dict) and mobile.get("children"):
                todo.append((mobile.get("children"), tree.add("folder", 0, mobile.get("name") or "Mobile bookmarks")))
            mobile = None
            continue
        children, parent = todo[i]
        i += 1
        for c in children if isinstance(children, list) else []:
            if not isinstance(c, dict):
                continue
            added, ts = from_chrome_time(c.get("date_added"))
            if c.get("type") == "folder":
                todo.append((c.get("children"), tree.add("folder", parent, c.get("name"), added=added, added_ts=ts)))
            elif c.get("type") == "url" and isinstance(c.get("url"), str):
                has_used |= "date_last_used" in c
                used, _ = from_chrome_time(c.get("date_last_used"))
                tree.add("url", parent, c.get("name"), url=c["url"], added=added, added_ts=ts, last_used=used)
    return order(tree.nodes), has_used


def order(nodes):
    """Children listed under their parents in file order (a walk without recursion lists them level by level)."""
    kids = {}
    for n in nodes:
        kids.setdefault(n["parent"], []).append(n)
    out, stack = [], list(reversed(kids.get(0, [])))
    while stack:
        n = stack.pop()
        out.append(n)
        stack.extend(reversed(kids.get(n["id"], [])))
    renumber = {n["id"]: i for i, n in enumerate(out, 1)}
    renumber[0] = 0
    for n in out:
        n["id"], n["parent"] = renumber[n["id"]], renumber[n["parent"]]
    return out


class Netscape(HTMLParser):
    """The bookmarks HTML file every browser can export and import (the Netscape bookmark format)."""

    def __init__(self):
        HTMLParser.__init__(self, convert_charrefs=True)
        self.tree, self.stack, self.waiting, self.cur, self.text, self.has_used = Tree(), [0], None, None, [], False

    def handle_starttag(self, tag, attrs):
        a = {k.lower(): (v or "") for k, v in attrs}
        if tag in ("h3", "a"):
            self.finish()
            self.waiting = None  # the folder before this one had no list of its own: it is empty
            self.cur, self.text = (tag, a), []
        elif tag == "dl":
            self.finish()
            self.stack.append(self.waiting if self.waiting is not None else self.stack[-1])
            self.waiting = None

    def handle_endtag(self, tag):
        if tag in ("h3", "a"):
            self.finish()
        elif tag == "dl":
            self.finish()
            self.waiting = None
            if len(self.stack) > 1:
                self.stack.pop()

    def handle_data(self, data):
        if self.cur:
            self.text.append(data)

    def finish(self):
        if not self.cur:
            return
        (tag, a), title = self.cur, "".join(self.text).strip()
        self.cur = None
        added, ts = from_unix(a.get("add_date"))
        if tag == "h3":
            root = "bar" if a.get("personal_toolbar_folder", "").lower() == "true" else (
                "unfiled" if a.get("unfiled_bookmarks_folder", "").lower() == "true" else None)
            self.waiting = self.tree.add("folder", self.stack[-1], title, added=added, added_ts=ts, root=root)
        elif "href" in a:
            self.has_used |= "last_visit" in a
            used, _ = from_unix(a.get("last_visit"))
            self.tree.add("url", self.stack[-1], title, url=a["href"], added=added, added_ts=ts, last_used=used,
                          tags=a.get("tags"))


def from_html(text):
    p = Netscape()
    p.feed(text)
    p.close()
    p.finish()
    return p.tree.nodes, p.has_used


def read_any(path):
    """(nodes, has_last_used, kind) from a Bookmarks JSON file or an exported bookmarks HTML file."""
    if os.path.getsize(path) > MAX_BYTES:
        sys.exit("that file is larger than %d MB: it isn't a bookmarks file" % (MAX_BYTES // (1024 * 1024)))
    with open(path, "rb") as f:
        raw = f.read()
    text = raw.decode("utf-8-sig", "replace")
    if text.lstrip()[:1] == "{":
        try:
            doc = json.loads(text)
        except (ValueError, RecursionError):
            sys.exit("that file starts like JSON but can't be read as JSON")
        nodes, used = from_chromium(doc)
        return nodes, used, "chromium"
    if "<a " not in text.lower() and "<dl" not in text.lower():
        sys.exit("that file is neither a Chromium Bookmarks file nor an exported bookmarks HTML file")
    nodes, used = from_html(text)
    return nodes, used, "html"


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description="Read a copy of a browser's bookmarks into bookmarks.json.")
    ap.add_argument("--list", action="store_true", help="show which browser profiles have bookmarks (names, sizes, dates only)")
    ap.add_argument("--browser", choices=BROWSERS)
    ap.add_argument("--profile", help="the profile folder's name as --list shows it, e.g. Default")
    ap.add_argument("--file", help="an exported bookmarks HTML file (any browser)")
    ap.add_argument("--out", help="bookmarks.json to write, in a scratch folder")
    ap.add_argument("--root", help=argparse.SUPPRESS)  # look for browsers under this folder instead (tests)
    a = ap.parse_args(argv)
    found = profiles(a.root)
    if a.list:
        rows = []
        for p in found:
            st = os.stat(p["path"])
            rows.append({"browser": p["browser"], "profile": p["profile"], "size_kb": st.st_size // 1024,
                         "changed": datetime.fromtimestamp(st.st_mtime).date().isoformat()})
        json.dump({"profiles": rows, "note": "Pick one with --browser and --profile. For Firefox, Safari or anything "
                                             "not listed, export the bookmarks to an HTML file and use --file."},
                  sys.stdout, indent=1)
        sys.stdout.write("\n")
        return
    if not a.out or bool(a.file) == bool(a.browser):
        sys.exit("give --out, and either --browser with --profile or --file (or use --list first)")
    out = may_write(a.out, ".json", "bookmark-cleanup/bookmarks@", a.root)
    scratch = os.path.dirname(out)
    if not os.path.isdir(scratch):
        sys.exit("the folder for --out doesn't exist: create a scratch folder first")
    if a.browser:
        match = [p for p in found if p["browser"] == a.browser and p["profile"] == (a.profile or "Default")]
        if not match:
            sys.exit("no such profile. Run --list and use the names exactly as shown.")
        source, label = match[0]["path"], {"browser": a.browser, "profile": match[0]["profile"]}
    else:
        source, label = os.path.abspath(a.file), {"browser": "", "profile": ""}
        if not os.path.isfile(source) or os.path.islink(source):
            sys.exit("--file isn't a file")
    if os.path.getsize(source) > MAX_BYTES:
        sys.exit("that file is larger than %d MB: it isn't a bookmarks file" % (MAX_BYTES // (1024 * 1024)))
    copy = os.path.join(scratch, "bookmarks-source-copy.tmp")
    try:
        # The browser's file is opened for reading only; all the work is on this copy. "xb" makes a
        # new file or fails: it never writes through something that is already there.
        with open(source, "rb") as src, open(copy, "xb") as dst:
            shutil.copyfileobj(src, dst)
    except FileExistsError:
        sys.exit("a leftover bookmarks-source-copy.tmp is in the scratch folder: delete it and run again")
    try:
        nodes, has_used, kind = read_any(copy)
    finally:
        os.remove(copy)
    urls = sum(1 for n in nodes if n["type"] == "url")
    doc = {"schema": SCHEMA, "source": dict(label, kind=kind, has_last_used=has_used,
                                            read=datetime.now().isoformat(timespec="seconds")), "nodes": nodes}
    with open(out, "w", encoding="ascii") as f:
        json.dump(doc, f)  # plain ASCII with escapes: every title comes back exactly, whatever characters it holds
    json.dump({"written": out, "bookmarks": urls, "folders": len(nodes) - urls, "format": kind,
               "records_last_opened": has_used}, sys.stdout, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()

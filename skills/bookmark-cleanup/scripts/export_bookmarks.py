#!/usr/bin/env python3
"""Write a NEW bookmarks file with the approved changes applied. It never touches a browser's own files.

    python export_bookmarks.py bookmarks-plan.json --approve "BOOKMARKS-APPROVE report=ab12cd34ef dupes=1-40 folders=1-3 stale=none bar=none big=none dead=1-4 look=none" --out <folder>/bookmarks-tidy.html --dry-run
    python export_bookmarks.py bookmarks-plan.json --approve "..." --out <folder>/bookmarks-tidy.html

Pass only the first line the page copied. The result is a bookmarks HTML file in the
standard format every browser imports, plus bookmarks-removed.html next to it holding
everything that was left out, so nothing is lost. The person imports the new file
themselves; this script changes nothing in any browser.

It refuses when:
  * the plan file no longer matches its own id (it was changed after the page was built),
  * the approval is for a different report, or holds anything but the fixed words and numbers,
  * a number isn't in the plan,
  * --out already exists, isn't an .html file, or lies inside a browser's own folder.

Every title and address is escaped on the way out. Bookmarklets (javascript: addresses) are
written back exactly as they were unless their removal was ticked; they are never run.
After writing, the file is read back and compared with what was meant to be in it.

Python 3.8+, standard library only.
"""
import argparse
import html
import json
import os
import re
import sys
from datetime import date

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
from build_report import ARCHIVE, FROM_BAR, HOST_NAME, LISTS, PLAN_SCHEMA, look_at, plan_id  # noqa: E402  (the id is a hash of the plan)
from read_bookmarks import from_html, inside_browser_data, plain_name  # noqa: E402

_IDS = r"(?:none|[0-9]{1,6}(?:-[0-9]{1,6})?(?:,[0-9]{1,6}(?:-[0-9]{1,6})?)*)"  # plain digits only: no look-alikes
LINE = re.compile(r"BOOKMARKS-APPROVE report=([0-9a-f]{10})((?: (?:%s)=%s)*)" % ("|".join(LISTS), _IDS))
HEADER = ("<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<!-- This is an automatically generated file.\n"
          "     It will be read and overwritten.\n     DO NOT EDIT! -->\n"
          '<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">\n<TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n')
_CTRL = re.compile(r"[\x00-\x1f\x7f-\x9f\u2028\u2029]")


_HALF = re.compile("[%s-%s]" % (chr(0xD800), chr(0xDFFF)))


def whole(s):
    """Text that can be written to a file: half a character (a lone surrogate) becomes the 'unknown' mark."""
    return _HALF.sub(chr(0xFFFD), str(s or ""))


def one_line(s):
    """A title as one line of text."""
    return _CTRL.sub(" ", whole(s)).strip()


def address(s):
    """An address without line breaks or control characters (browsers drop them too)."""
    return _CTRL.sub("", whole(s)).strip()


def parse_ids(text):
    ids = set()
    if text == "none":
        return ids
    for part in text.split(","):
        a, _, b = part.partition("-")
        lo, hi = int(a), int(b or a)
        if lo > hi or hi - lo > 500000:
            sys.exit("Can't read the range '%s' in the approval." % part)
        ids.update(range(lo, hi + 1))
    return ids


def read_approval(line, plan):
    m = LINE.fullmatch(line.strip(" \r\n"))
    if not m:
        sys.exit("That isn't an approval line. It looks like: BOOKMARKS-APPROVE report=<id> dupes=1-40 folders=1-3 "
                 "stale=none bar=none big=none dead=1-4 look=none  (pass only that first line, nothing else)")
    if m.group(1) != plan["report_id"]:
        sys.exit("This approval is for report %s but the plan file is report %s. Show the person the current page "
                 "and ask again." % (m.group(1), plan["report_id"]))
    picked = {name: set() for name in LISTS}
    seen = set()
    for word in m.group(2).split():
        name, _, ids = word.partition("=")
        if name in seen:
            sys.exit("The approval names '%s' twice." % name)
        seen.add(name)
        picked[name] = parse_ids(ids)
    problems = []
    for name in LISTS:
        known = {x["n"] for x in plan["lists"][name]}
        problems += ["%s=%d: there is no number %d in that list" % (name, n, n) for n in sorted(picked[name] - known)][:10]
    if problems:
        sys.exit("The approval doesn't fit the plan:\n  " + "\n  ".join(problems[:20]))
    return picked


def site_of(url):
    i = look_at(url)
    if not i["web"] or i["secret"]:
        return ""
    return i["host"][4:] if i["host"].startswith("www.") else i["host"]


def apply(plan, picked):
    """The tree after the approved changes: (nodes, children per folder, what was removed, counts)."""
    nodes = {n["id"]: dict(n) for n in plan["nodes"]}
    kids = {}
    for n in plan["nodes"]:
        kids.setdefault(n["parent"], []).append(n["id"])
    by_n = {name: {x["n"]: x for x in plan["lists"][name]} for name in LISTS}
    removed, counts, skipped = {}, {"removed": 0, "moved": 0, "sorted_into_site_folders": 0, "folders_merged": 0,
                                    "folders_removed": 0}, []
    next_id = [max(nodes) + 1]

    def detach(i):
        kids[nodes[i]["parent"]].remove(i)

    def attach(i, folder):
        nodes[i]["parent"] = folder
        kids.setdefault(folder, []).append(i)

    def folder_named(parent, title):
        for c in kids.get(parent, []):
            if nodes[c]["type"] == "folder" and nodes[c].get("title") == title and not nodes[c].get("root"):
                return c
        i = next_id[0]
        next_id[0] += 1
        nodes[i] = {"id": i, "type": "folder", "parent": parent, "title": title}
        kids.setdefault(parent, []).append(i)
        return i

    def holds_bookmarks(fid):
        todo = [fid]
        while todo:
            for c in kids.get(todo.pop(), []):
                if nodes[c]["type"] == "url":
                    return True
                todo.append(c)
        return False

    for name in ("dupes", "dead", "look"):                      # 1. leave these out
        for n in sorted(picked[name]):
            i = by_n[name][n]["id"]
            if i not in removed:
                detach(i)
                removed[i] = name
                counts["removed"] += 1
    gone = set()
    for n in sorted(picked["folders"]):                         # 2. two folders with one name become one
        x = by_n["folders"][n]
        if x["op"] == "merge_folder" and x["into"] not in gone:
            for c in list(kids.get(x["id"], [])):
                detach(c)
                attach(c, x["into"])
            detach(x["id"])
            gone.add(x["id"])
            counts["folders_merged"] += 1
    for name, title in (("stale", ARCHIVE), ("bar", FROM_BAR)):  # fixed names: nothing in the plan file chooses them
        target = None
        for n in sorted(picked[name]):                          # 3. moves into one new folder at the end
            i = by_n[name][n]["id"]
            if i in removed:
                continue
            target = target or folder_named(0, title)
            detach(i)
            attach(i, target)
            counts["moved"] += 1
    for n in sorted(picked["big"]):                             # 4. a pile sorted into folders named after sites
        x = by_n["big"][n]
        if x["id"] in gone:
            skipped.append("big=%d: that folder was merged into another one" % n)
            continue
        for host, _ in x["sites"]:
            if not HOST_NAME.fullmatch(str(host)):
                continue
            members = [c for c in kids.get(x["id"], []) if nodes[c]["type"] == "url" and site_of(nodes[c]["url"]) == host]
            if len(members) < 2:
                continue
            sub = folder_named(x["id"], host)
            for c in members:
                detach(c)
                attach(c, sub)
                counts["sorted_into_site_folders"] += 1
    for n in sorted(picked["folders"]):                         # 5. empty folders, checked again now
        x = by_n["folders"][n]
        if x["op"] != "remove_folder":
            continue
        if holds_bookmarks(x["id"]):
            skipped.append("folders=%d: no longer empty after the other changes, so it stays" % n)
            continue
        detach(x["id"])
        counts["folders_removed"] += 1
    return nodes, kids, removed, counts, skipped


def walk(nodes, kids):
    """Every bookmark and folder still in the tree, in order: (kind, path of folder names, title, address)."""
    out, stack = [], [(i, ()) for i in reversed(kids.get(0, []))]
    while stack:
        i, path = stack.pop()
        n = nodes[i]
        title = one_line(n.get("title"))
        if n["type"] == "folder":
            out.append(("folder", path, title, ""))
            stack.extend((c, path + (title,)) for c in reversed(kids.get(i, [])))
        else:
            out.append(("url", path, title, address(n.get("url"))))
    return out


def render(nodes, kids):
    """The bookmarks as a Netscape bookmark file. Every title and address goes through html.escape."""
    lines = [HEADER + "<DL><p>"]
    stack = [("open", i, 1) for i in reversed(kids.get(0, []))]
    while stack:
        what, i, depth = stack.pop()
        pad = "    " * depth
        if what == "close":
            lines.append(pad + "</DL><p>")
            continue
        n = nodes[i]
        added = ' ADD_DATE="%d"' % n["added_ts"] if isinstance(n.get("added_ts"), int) else ""
        title = html.escape(one_line(n.get("title")), quote=True)
        if n["type"] == "folder":
            mark = {"bar": ' PERSONAL_TOOLBAR_FOLDER="true"', "unfiled": ' UNFILED_BOOKMARKS_FOLDER="true"'}.get(n.get("root"), "")
            lines.append("%s<DT><H3%s%s>%s</H3>" % (pad, added, mark, title))
            lines.append(pad + "<DL><p>")
            stack.append(("close", i, depth))
            stack.extend(("open", c, depth + 1) for c in reversed(kids.get(i, [])))
        else:
            tags = ' TAGS="%s"' % html.escape(one_line(n["tags"]), quote=True) if n.get("tags") else ""
            lines.append('%s<DT><A HREF="%s"%s%s>%s</A>' % (pad, html.escape(address(n.get("url")), quote=True), added, tags, title))
    lines.append("</DL><p>\n")
    return "\n".join(lines)


def reread(text):
    """What a browser would find in the file: parsed with the same reader used for exported files."""
    found, _ = from_html(text)
    nodes = {n["id"]: n for n in found}
    kids = {}
    for n in found:
        kids.setdefault(n["parent"], []).append(n["id"])
    return walk(nodes, kids)


def write_checked(path, nodes, kids):
    text, want = render(nodes, kids), walk(nodes, kids)
    if reread(text) != want:
        sys.exit("refusing to write %s: read back, the file would not hold exactly the bookmarks it should" % os.path.basename(path))
    with open(path, "x", encoding="utf-8", newline="\n") as f:  # "x": never over an existing file
        f.write(text)
    with open(path, encoding="utf-8") as f:
        if reread(f.read()) != want:
            os.remove(path)
            sys.exit("the file read back differently from what was written; it was removed again")
    return sum(1 for w in want if w[0] == "url")


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description="Write a new bookmarks HTML file with the approved changes applied.")
    ap.add_argument("plan", help="bookmarks-plan.json, as written by build_report.py")
    ap.add_argument("--approve", required=True, help="the first line the page copied")
    ap.add_argument("--out", required=True, help="the new bookmarks file to write, e.g. Desktop/bookmarks-tidy.html")
    ap.add_argument("--dry-run", action="store_true", help="say what would be written, write nothing")
    ap.add_argument("--root", help=argparse.SUPPRESS)  # where browsers live, for tests
    a = ap.parse_args(argv)
    with open(a.plan, encoding="utf-8") as f:
        plan = json.load(f)
    if not isinstance(plan, dict) or plan.get("schema") != PLAN_SCHEMA:
        sys.exit("%s is not a bookmarks plan" % a.plan)
    # The approval names the report by its id, and the id is a hash of the bookmarks and the
    # suggestions: a plan changed after the page was built (and approved) no longer matches.
    try:
        intact = plan_id(plan["nodes"], plan["lists"]) == plan["report_id"] and all(
            [x["n"] for x in plan["lists"][name]] == list(range(1, len(plan["lists"][name]) + 1)) for name in LISTS) and all(
            n["id"] == k and n["type"] in ("folder", "url") and n["parent"] < k for k, n in enumerate(plan["nodes"], 1))
    except (KeyError, TypeError):
        intact = False
    if not intact:
        sys.exit("%s was changed after its page was built, so the approval doesn't cover it. "
                 "Rebuild the report with build_report.py and ask for a new approval." % a.plan)
    picked = read_approval(a.approve, plan)
    out = os.path.abspath(a.out)
    side = os.path.join(os.path.dirname(out), "bookmarks-removed.html")
    if not out.lower().endswith(".html") or not plain_name(out):
        sys.exit("--out must be a new .html file with a plain name")
    if inside_browser_data(out, a.root):
        sys.exit("--out is inside a browser's own folder. This skill never writes there: choose the Desktop or Documents.")
    if not os.path.isdir(os.path.dirname(out)):
        sys.exit("the folder for --out doesn't exist")
    nodes, kids, removed, counts, skipped = apply(plan, picked)
    summary = {"report_id": plan["report_id"], "bookmarks_before": sum(1 for n in plan["nodes"] if n["type"] == "url"),
               "bookmarks_in_new_file": sum(1 for w in walk(nodes, kids) if w[0] == "url"), "changes": counts,
               "left_as_it_was": skipped}
    if a.dry_run:
        json.dump(dict(summary, dry_run=True, would_write=out), sys.stdout, ensure_ascii=False, indent=1)
        sys.stdout.write("\n")
        return
    if os.path.exists(out) or (removed and os.path.exists(side)):
        sys.exit("%s or bookmarks-removed.html already exists there. Choose a new name or folder: nothing is overwritten." % os.path.basename(out))
    write_checked(out, nodes, kids)
    summary["written"] = out
    if removed:
        label = {"dupes": "Extra copies", "dead": "Dead links", "look": "Other"}
        rn = {1: {"id": 1, "type": "folder", "parent": 0, "title": "Removed by bookmark-cleanup on " + date.today().isoformat()}}
        rk = {0: [1], 1: []}
        for name in ("dupes", "dead", "look"):
            ids = [i for i, why in removed.items() if why == name]
            if not ids:
                continue
            fid = len(rn) + 1
            rn[fid] = {"id": fid, "type": "folder", "parent": 1, "title": label[name]}
            rk[1].append(fid)
            for i in ids:
                k = len(rn) + 1
                rn[k] = dict(plan["nodes"][i - 1], id=k, parent=fid)
                rk.setdefault(fid, []).append(k)
        write_checked(side, rn, rk)
        summary["removed_bookmarks_kept_in"] = side
    summary["next"] = ("Nothing in any browser was changed. The person imports the new file themselves "
                       "(see references/formats.md for the steps in each browser).")
    json.dump(summary, sys.stdout, ensure_ascii=False, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()

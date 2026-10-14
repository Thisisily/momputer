"""Checks the bookmark-cleanup skill end to end on made-up bookmarks.

    python -I test_bookmark_cleanup.py <scratch folder> [<skill folder>] [--keep] [--no-browser]

It writes a pretend browser tree (an Edge profile, a Chrome profile, a guest profile) and an
exported bookmarks file into a work folder inside <scratch folder>. Then: list the profiles ->
read a copy -> report -> report again with link results from a pretend network -> export a new
bookmarks file from an approval line -> read that file back. It checks what the report found,
that hostile titles and addresses stay text, that nothing is lost or invented by the export,
that every changed approval line or plan is refused, and (what an outside review found) very
deep folders, half a character in a title, files that may not be written over, and output
names that aren't plain names. Then the page in a headless browser, and the exported files
opened the way a double-click would open them.

The skill's scripts are run with LOCALAPPDATA and the home folder pointing into the work
folder, so they never look at a real browser's folders. No site is contacted: every address in
the fixture is a .example name, a local address or not a web page, and the one run of the real
link checker happens only after it has said it would ask nobody. Nothing outside the work
folder is written.

--keep leaves the work folder (bookmarks, reports, plans, new files, screenshots) for a look.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import collections
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, Checks, hashes, hidden_streams, ranges, remove_work_folder, run, work_folder  # noqa: E402
from _gen_bookmarks import write_bookmarks  # noqa: E402
from _report_page import SIZES, NoBrowser, Pages, basics  # noqa: E402

MARK = "made-by-test_bookmark_cleanup.txt"
LQ, RQ, LS, RS, DOT = chr(0x201c), chr(0x201d), chr(0x2018), chr(0x2019), chr(0x2022)
PAGE_LISTS = ["dupes", "folders", "stale", "bar", "big", "dead", "look"]  # the order the page writes them in its approval line

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "bookmark-cleanup"))
ap.add_argument("--keep", action="store_true")
ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser checks of the page and the exported files")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
sys.stdout.reconfigure(encoding="ascii", errors="backslashreplace")  # check names hold hostile titles: print them spelled out
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
if not os.path.isfile(os.path.join(S, "read_bookmarks.py")):
    sys.exit("Not found: %s" % os.path.join(S, "read_bookmarks.py"))
c = Checks()
check, note = c.check, c.note
T = work_folder(opts.scratch, "bookmark-cleanup-test", MARK)
IN, WORK, OUT = os.path.join(T, "in"), os.path.join(T, "work"), os.path.join(T, "out")
LOCAL = os.path.join(IN, "local")  # stands in for %LOCALAPPDATA%
EDGE = os.path.join(LOCAL, "Microsoft", "Edge", "User Data", "Default", "Bookmarks")
# The skill asks where browsers keep their files. Here the answer is always the pretend tree, never this PC's own.
ENV = dict(os.environ, LOCALAPPDATA=LOCAL, APPDATA=os.path.join(T, "no-home-here", "AppData", "Roaming"),
           USERPROFILE=os.path.join(T, "no-home-here"), HOME=os.path.join(T, "no-home-here"))


def py(*args):
    return run([PY, "-I"] + list(args), env=ENV)


def data_of(page):
    html = open(page, encoding="utf-8").read()
    tag = '<script id="bookmarks-data" type="application/json">'
    a = html.index(tag) + len(tag)
    blob = html[a:html.index("</script>", a)]
    return html, blob, json.loads(blob)


def line(rid, **picks):
    return "BOOKMARKS-APPROVE report=%s" % rid + "".join(" %s=%s" % (k, ranges(picks.get(k, []))) for k in br.LISTS)


def further_checks(W, plan_path):
    """What an outside review found, kept as tests. `plan_path` is the plan of the main report."""
    os.makedirs(W)

    # ---- folders nested thousands deep: no crash, no traceback, no hang
    deep = ('{"type": "folder", "name": "level", "children": [' * 2500 +
            '{"type": "url", "name": "at the bottom", "url": "https://deep.example/", "date_added": "13300000000000000"}' + "]}" * 2500)
    with open(os.path.join(W, "deep-bookmarks"), "w", encoding="utf-8") as f:
        f.write('{"roots": {"bookmark_bar": {"type": "folder", "name": "Bar", "children": [' + deep + ']}}}')
    r = py(os.path.join(S, "read_bookmarks.py"), "--file", os.path.join(W, "deep-bookmarks"), "--out", os.path.join(W, "deep.json"))
    check("a Bookmarks file nested 2,500 deep: read, or refused in a sentence, never a traceback", "Traceback" not in r.stderr and
          (r.returncode == 0 or "can't be read" in r.stderr), r.stderr[-300:])
    check("...and the scratch copy is gone either way", "bookmarks-source-copy.tmp" not in os.listdir(W), os.listdir(W))
    rows = ["<!DOCTYPE NETSCAPE-Bookmark-file-1>", "<DL><p>"]
    for i in range(3000):
        rows += ["<DT><H3>level %d with a fairly long folder name</H3>" % i, "<DL><p>"]
    rows += ['<DT><A HREF="https://deep.example/">at the bottom</A>', '<DT><A HREF="https://deep.example">at the bottom again</A>'] + ["</DL><p>"] * 3001
    with open(os.path.join(W, "deep.html"), "w", encoding="utf-8") as f:
        f.write("\n".join(rows))
    r = py(os.path.join(S, "read_bookmarks.py"), "--file", os.path.join(W, "deep.html"), "--out", os.path.join(W, "deep-html.json"))
    check("an exported file nested 3,000 deep is read", r.returncode == 0 and '"folders": 3000' in r.stdout, r.stderr[-300:])
    r = py(os.path.join(S, "build_report.py"), os.path.join(W, "deep-html.json"), "--out", os.path.join(W, "deep.html.report.html"), "--today", "2026-10-13")
    plan = json.load(open(os.path.join(W, "bookmarks-plan.json"), encoding="utf-8")) if r.returncode == 0 else {"lists": {"dupes": []}}
    check("...and its report builds; the place of a bookmark is shown by its nearest folders", r.returncode == 0 and "Traceback" not in r.stderr and
          len(plan["lists"]["dupes"]) == 1 and len(plan["lists"]["dupes"][0]["where"]) <= 90 and
          plan["lists"]["dupes"][0]["where"] == chr(0x2026) + " / level 2998 with a fairly long folder na" + chr(0x2026) + " / level 2999 with a fairly long folder na" + chr(0x2026),
          (r.stderr[-300:], plan["lists"]["dupes"][:1]))
    approve = "BOOKMARKS-APPROVE report=%s dupes=1" % plan.get("report_id", "")
    r = py(os.path.join(S, "export_bookmarks.py"), os.path.join(W, "bookmarks-plan.json"), "--approve", approve, "--out", os.path.join(W, "deep-tidy.html"))
    check("...and it exports, one bookmark fewer", r.returncode == 0 and '"bookmarks_in_new_file": 1' in r.stdout, r.stderr[-300:])

    # ---- a title with half a character in it (a lone surrogate, which no file can hold as text)
    half = ('{"roots": {"bookmark_bar": {"type": "folder", "name": "Bar", "children": ['
            '{"type": "url", "name": "broken \\ud83d title", "url": "https://half.example/"},'
            '{"type": "url", "name": "broken \\ud83d title", "url": "https://half.example"}]}}}')
    with open(os.path.join(W, "half-bookmarks"), "w", encoding="ascii") as f:
        f.write(half)
    hw = os.path.join(W, "half")
    os.makedirs(hw)
    r = py(os.path.join(S, "read_bookmarks.py"), "--file", os.path.join(W, "half-bookmarks"), "--out", os.path.join(hw, "bookmarks.json"))
    r2 = py(os.path.join(S, "build_report.py"), os.path.join(hw, "bookmarks.json"), "--out", os.path.join(hw, "report.html"), "--today", "2026-10-13")
    hplan = json.load(open(os.path.join(hw, "bookmarks-plan.json"), encoding="utf-8")) if r2.returncode == 0 else {}
    r3 = py(os.path.join(S, "export_bookmarks.py"), os.path.join(hw, "bookmarks-plan.json"), "--approve",
            "BOOKMARKS-APPROVE report=%s dupes=1" % hplan.get("report_id", ""), "--out", os.path.join(hw, "tidy.html"))
    check("half a character in a title: read, reported and exported without a traceback or an empty file",
          r.returncode == 0 and r2.returncode == 0 and r3.returncode == 0 and "Traceback" not in r.stderr + r2.stderr + r3.stderr
          and os.path.getsize(os.path.join(hw, "report.html")) > 10000 and ">broken " + chr(0xfffd) + " title</A>" in open(os.path.join(hw, "tidy.html"), encoding="utf-8").read(),
          (r.stderr[-200:], r2.stderr[-200:], r3.stderr[-200:]))

    # ---- nothing but its own earlier output is ever written over, and never inside a browser's folder
    mine = os.path.join(W, "notes.json")
    with open(mine, "w") as f:
        f.write('{"my": "notes"}')
    foreign = os.path.join(W, "page.html")
    with open(foreign, "w") as f:
        f.write("<html><title>Mine</title></html>")
    for script, args in (("read_bookmarks.py", ["--file", os.path.join(W, "deep.html"), "--out", mine]),
                         ("read_bookmarks.py", ["--file", os.path.join(W, "deep.html"), "--out", os.path.join(W, "bookmarks.txt")]),
                         ("check_links.py", [os.path.join(hw, "bookmarks.json"), "--out", mine, "--person-agreed"]),
                         ("build_report.py", [os.path.join(hw, "bookmarks.json"), "--out", foreign]),
                         ("build_report.py", [os.path.join(hw, "bookmarks.json"), "--out", os.path.join(hw, "r2.html"), "--plan", mine]),
                         ("build_report.py", [os.path.join(hw, "bookmarks.json"), "--out", os.path.join(hw, "bookmarks.json")])):
        r = py(os.path.join(S, script), *args)
        check("%s refuses to write over %s" % (script, os.path.basename(args[-1] if args[-1] != "--person-agreed" else args[-2])),
              r.returncode != 0 and "Traceback" not in r.stderr, r.stderr[-200:])
    check("the two files that were in the way are untouched", open(mine).read() == '{"my": "notes"}' and open(foreign).read() == "<html><title>Mine</title></html>")
    r = py(os.path.join(S, "build_report.py"), os.path.join(hw, "bookmarks.json"), "--out", os.path.join(hw, "report.html"), "--today", "2026-10-13")
    check("building again over its own earlier report is fine", r.returncode == 0, r.stderr[-200:])

    # ---- the names of the folders that bookmarks are moved into are fixed: nothing in the plan file chooses them
    plan = json.load(open(plan_path, encoding="utf-8"))
    check("the plan file carries no folder names to tamper with", "new_folders" not in plan)
    plan["new_folders"] = {"archive": "ZZZ <script>alert(1)</script>", "from_bar": "ZZZ"}
    edited = os.path.join(W, "plan-with-names.json")
    json.dump(plan, open(edited, "w", encoding="utf-8"))
    approve = "BOOKMARKS-APPROVE report=%s stale=1-3 bar=1-2" % plan["report_id"]
    out = os.path.join(W, "names.html")
    r = py(os.path.join(S, "export_bookmarks.py"), edited, "--approve", approve, "--out", out)
    text = open(out, encoding="utf-8").read() if os.path.exists(out) else ""
    check("folder names smuggled into the plan file are ignored", r.returncode == 0 and "ZZZ" not in text and ">Old bookmarks</H3>" in text
          and ">From the bookmarks bar</H3>" in text, r.stderr[-300:])

    # ---- an output name that isn't a plain name (on Windows "x:y" writes into another file's hidden stream)
    approve = "BOOKMARKS-APPROVE report=%s dupes=1" % plan["report_id"]
    for bad in ("Bookmarks:t4.html", "tidy.html:hidden.html", "names.html:x.html"):
        before = sorted(os.listdir(W))
        r = py(os.path.join(S, "export_bookmarks.py"), plan_path, "--approve", approve, "--out", os.path.join(W, bad))
        check("export refuses --out %s and writes nothing" % bad, r.returncode != 0 and "plain name" in r.stderr and sorted(os.listdir(W)) == before, r.stderr[-200:])
    r = py(os.path.join(S, "read_bookmarks.py"), "--file", os.path.join(W, "deep.html"), "--out", os.path.join(W, "deep.html:stream.json"))
    check("the reader refuses such an --out too", r.returncode != 0 and "plain .json file" in r.stderr, r.stderr[-200:])
    check("no hidden streams were made in the work folder", not hidden_streams(W), hidden_streams(W)[:3])


def page_checks(b, page, plan, shots):
    """The report in a headless browser: every size, light and dark, then ticking and the approval line."""
    lists = plan["lists"]
    everything = {k: [x["n"] for x in lists[k]] for k in PAGE_LISTS}
    suggested = {k: [x["n"] for x in lists[k] if x["default"]] for k in PAGE_LISTS}
    more = "Array.from(document.querySelectorAll('button.more')).filter(b => !b.classList.contains('hidden')).forEach(b => b.click())"

    def approval(picks):
        return "BOOKMARKS-APPROVE report=%s" % plan["report_id"] + "".join(" %s=%s" % (k, ranges(picks.get(k, []))) for k in PAGE_LISTS)

    for width, height, scheme in SIZES:
        tag = "%d-%s" % (width, scheme)
        b.open(page, width, height, scheme)
        b.js("try { localStorage.clear() } catch (e) {}")
        b.open(page, width, height, scheme)
        b.js(more)
        b.settle(0.5)
        basics(check, b, tag, scheme)
        n = b.js("({tiles: document.querySelectorAll('.tile').length, cols: document.querySelectorAll('svg.cols rect.col').length,"
                 " bars: document.querySelectorAll('svg.hbar').length, cards: document.querySelectorAll('[data-list]').length,"
                 " rows: Object.fromEntries(Array.from(document.querySelectorAll('[data-list]')).map(c => [c.dataset.list, c.querySelectorAll('.item').length])),"
                 " scripts: document.scripts.length, imgs: document.images.length, frames: document.querySelectorAll('iframe,object,embed').length,"
                 " handlers: Array.from(document.querySelectorAll('main *')).filter(e => Array.from(e.attributes).some(a => a.name.startsWith('on'))).length})")
        check(tag + ": everything drew, every row of every list", n["tiles"] == 6 and n["cols"] >= 6 and n["bars"] >= 12 and n["cards"] == 7
              and n["rows"] == {k: len(lists[k]) for k in PAGE_LISTS}, n)
        check(tag + ": hostile titles and addresses made no elements or handlers", n["scripts"] == 2 and n["imgs"] == 0 and n["frames"] == 0 and n["handlers"] == 0, n)
        links = b.js("Array.from(document.querySelectorAll('a[href]')).map(a => { const u = new URL(a.href); "
                     "return [u.protocol, u.username + u.password, a.rel, a.getAttribute('href').slice(0, 12)]; })")
        check(tag + ": every link is plain http(s), no password, rel=noopener noreferrer", len(links) > 150 and all(
            x[0] in ("http:", "https:") and x[1] == "" and x[2] == "noopener noreferrer" for x in links), [x for x in links if x[0] not in ("http:", "https:")][:5])
        text = b.js("document.body.innerText")
        for needle in ("<img src=x onerror=alert(1)>", "</script><script>alert(2)</script>", "$(calc) `whoami` ; rm -rf & echo",
                       "</A><DT><A HREF=\"javascript:alert(3)\">injected</A>", "javascript:(function(){document.body.style", "JaVaScRiPt:alert(4)",
                       "data:text/html,<script>alert(6)</script>", "file:///C:/Users/someone/secret.txt", "https://quote.example/\"onmouseover=\"alert(7)",
                       "https://tag.example/<script>alert(8)</script>", LQ + "curly" + RQ + " " + LS + "quotes" + RS, "https://" + DOT * 3 + "@vault.example/login"):
            check(tag + ": shown as text: " + needle[:34], needle in text)
        check(tag + ": the password is nowhere on the page", "hunter2" not in text and "hunter2" not in b.js("document.documentElement.outerHTML"))
        notlinks = b.js("Array.from(document.querySelectorAll('[data-list=look] .item')).filter(li => li.querySelector('a')).length")
        check(tag + ": nothing in 'not web pages' is a link", notlinks == 0, notlinks)
        if shots:
            b.shot(os.path.join(shots, "bookmarks-%s.png" % tag))

    # ---- ticking and the approval line (phone width, light)
    b.open(page, 390, 844, "light")
    first = b.copy()
    check("first load ticks only the suggestions", first == approval(suggested), (first, approval(suggested)))
    hidden = b.js("document.querySelectorAll('[data-list=stale] .item').length")
    check("a long list starts with its first 150 rows", hidden == 150 and len(lists["stale"]) > 150, hidden)
    b.js("document.getElementById('untick-all').click()")
    check("Untick all clears every tick", b.js("document.querySelectorAll('.item input:checked').length") == 0 and b.copy() == approval({}))
    b.js("document.getElementById('tick-all').click()")
    ticks = b.js("({on: document.querySelectorAll('.item input:checked').length, all: document.querySelectorAll('.item input').length})")
    got = b.copy()
    check("Tick all ticks every box, including rows not drawn yet", ticks["on"] == ticks["all"] and got == approval(everything), (ticks, got))
    for k in PAGE_LISTS:
        b.js("document.querySelector('[data-list=%s] button[aria-label^=\"Untick all\"]').click()" % k)
        got = b.copy()
        check("Untick all for '%s' clears only that list" % k, got == approval(dict(everything, **{k: []})), got)
        b.js("document.querySelector('[data-list=%s] button[aria-label^=\"Tick all\"]').click()" % k)
        check("Tick all for '%s' brings it back" % k, b.copy() == approval(everything))
    b.js("document.getElementById('untick-all').click()")
    b.js(more)
    shown = b.js("document.querySelectorAll('[data-list=stale] .item').length")
    check("Show more draws the rest", shown == len(lists["stale"]), shown)
    b.js("""(() => { const box = (k, n) => document.querySelectorAll('[data-list=' + k + '] .item input')[n - 1];
        box('dupes', 2).click(); box('dupes', 3).click(); box('dupes', 5).click(); box('folders', 1).click(); box('stale', %d).click();
        box('bar', 12).click(); box('big', 1).click(); box('dead', 4).click(); box('look', 1).click(); box('look', 2).click(); })()""" % len(lists["stale"]))
    b.js("document.getElementById('toggle-notes').click(); document.getElementById('notes').value = 'keep  both\\nRecipes <b>folders</b>';")
    got = b.copy()
    want = approval({"dupes": [2, 3, 5], "folders": [1], "stale": [len(lists["stale"])], "bar": [12], "big": [1], "dead": [4], "look": [1, 2]}) + \
        "\nnote: keep both Recipes <b>folders</b>"
    check("approval names exactly the boxes clicked, with the note", got == want, (got, want))
    toast = b.js("document.getElementById('toast').textContent")
    check("copy says what happened", "Approval copied" in toast or "Copy the selected text" in toast, toast)
    b.open(page, 390, 844, "light")
    check("ticks are remembered after a reload", b.copy().split("\n")[0] == want.split("\n")[0])
    b.js("document.getElementById('tick-suggested').click()")
    check("Tick suggested only goes back to the suggestions", b.copy() == first)
    p = b.problems()
    check("no script errors while clicking", not p["exceptions"] and not p["console_errors"], p)


def exported_file_checks(b, files):
    """Open the exported bookmark files as someone double-clicking them would, and check that nothing runs."""
    for path in files:
        name = os.path.basename(path)
        b.open(path, 1280, 900, "light", settle=1.5)
        p = b.problems()
        check(name + ": opening it runs nothing (no alert, no script error)", not p["dialogs"] and not p["exceptions"], p)
        check(name + ": opening it requests nothing", len(p["requests"]) == 1 and p["requests"][0].startswith("file:///"), p["requests"])
        n = b.js("({scripts: document.scripts.length, imgs: document.images.length, frames: document.querySelectorAll('iframe,object,embed,link,style,form,input').length,"
                 " handlers: Array.from(document.querySelectorAll('*')).filter(e => Array.from(e.attributes).some(a => a.name.toLowerCase().startsWith('on'))).length,"
                 " links: document.querySelectorAll('a[href]').length, folders: document.querySelectorAll('h3').length,"
                 " tags: Array.from(new Set(Array.from(document.body.querySelectorAll('*')).map(e => e.tagName))).sort()})")
        check(name + ": only bookmark-file tags came out of it", n["scripts"] == 0 and n["imgs"] == 0 and n["frames"] == 0 and n["handlers"] == 0
              and set(n["tags"]) <= {"A", "DL", "DT", "H1", "H3", "P"}, n)
        print("      %s: %d links, %d folders" % (name, n["links"], n["folders"]))


try:
    os.makedirs(WORK)
    check("fixtures written", write_bookmarks(IN) == LOCAL and os.path.isfile(EDGE), os.listdir(IN))
    before_in, before_skill = hashes(IN), hashes(SKILL)
    sys.path.insert(0, S)
    import build_report as br  # noqa: E402  (the skill's own)
    import check_links as cl  # noqa: E402
    import export_bookmarks as ex  # noqa: E402
    import read_bookmarks as rb  # noqa: E402

    # ---------------------------------------------------------------- 1. reading a copy
    print("\n-- 1. reading a copy of the bookmarks")
    r = py(os.path.join(S, "read_bookmarks.py"), "--list", "--root", LOCAL)
    found = json.loads(r.stdout)["profiles"]
    check("--list finds the two profiles and not the guest one", [(p["browser"], p["profile"]) for p in found] == [("chrome", "Profile 1"), ("edge", "Default")], found)
    check("--list prints no titles or addresses", "example" not in r.stdout and "Bar site" not in r.stdout)
    bm = os.path.join(WORK, "bookmarks.json")
    r = py(os.path.join(S, "read_bookmarks.py"), "--root", LOCAL, "--browser", "edge", "--profile", "Default", "--out", bm)
    got = json.loads(r.stdout) if r.returncode == 0 else {}
    doc = json.load(open(bm, encoding="utf-8"))
    nodes = doc["nodes"]
    check("the Edge profile was read from a copy", r.returncode == 0 and got["bookmarks"] == sum(1 for n in nodes if n["type"] == "url") == 246 and got["records_last_opened"], r.stderr[:300] + r.stdout[:300])
    check("the browser's files are untouched and nothing was added beside them", hashes(IN) == before_in)
    check("the scratch copy was deleted again", sorted(os.listdir(WORK)) == ["bookmarks.json"], os.listdir(WORK))
    check("the output prints no titles or addresses", "example" not in r.stdout and "hunter2" not in r.stdout)
    check("the bar is marked, the phone's bookmarks come last in their own folder",
          [n["title"] for n in nodes if n.get("root") == "bar"] == ["Favorites bar"] and [n["title"] for n in nodes if n["parent"] == 0][-1] == "Mobile favorites")
    check("ids run in file order with parents first", all(n["id"] == i and n["parent"] < i for i, n in enumerate(nodes, 1)))
    first = next(n for n in nodes if n.get("title") == "Bar site 1")
    check("dates: added and last opened", first["added"] == "2021-02-01" and first["last_used"] == "2026-09-01" and first["added_ts"] == 1612180800, first)
    check("a bookmark never opened has no last_used", "last_used" not in next(n for n in nodes if n.get("title") == "Bar site 9"))
    for bad_out, why in ((os.path.join(LOCAL, "Microsoft", "Edge", "User Data", "Default", "out.json"), "outside any browser's own folder"),
                         (os.path.join(LOCAL, "Google", "Chrome", "User Data", "bookmarks.json"), "outside any browser's own folder")):
        r = py(os.path.join(S, "read_bookmarks.py"), "--root", LOCAL, "--browser", "edge", "--out", bad_out)
        check("refuses to write " + os.path.relpath(bad_out, LOCAL), r.returncode != 0 and why in r.stderr and not os.path.exists(bad_out), r.stderr[:200])
    for args in (("--browser", "edge", "--profile", "Profile 9"), ("--browser", "edge", "--profile", "..\\..\\..\\Google\\Chrome\\User Data\\Profile 1"),
                 ("--browser", "brave")):
        r = py(os.path.join(S, "read_bookmarks.py"), "--root", LOCAL, "--out", os.path.join(WORK, "x.json"), *args)
        check("refuses a profile that isn't in the list: %s" % (args,), r.returncode != 0 and not os.path.exists(os.path.join(WORK, "x.json")), r.stderr[:200])
    xj = os.path.join(WORK, "exported.json")
    r = py(os.path.join(S, "read_bookmarks.py"), "--file", os.path.join(IN, "exported.html"), "--out", xj)
    xnodes = json.load(open(xj, encoding="utf-8"))["nodes"]
    check("an exported HTML file is read: toolbar, unfiled folder, an empty folder, tags",
          r.returncode == 0 and [(n["title"], n.get("root")) for n in xnodes if n["type"] == "folder"] ==
          [("Bookmarks Toolbar", "bar"), ("Nothing in here", None), ("Nested", None), ("Other Bookmarks", "unfiled")]
          and next(n for n in xnodes if n.get("title") == "Menu one")["tags"] == "news,daily"
          and next(n for n in xnodes if n.get("title") == "Toolbar & friends")["url"] == "https://toolbar.example/a?b=1&c=2", [n for n in xnodes if n["type"] == "folder"])
    check("a tag smuggled into a title in that file is dropped, not kept", [n["title"] for n in xnodes if "raw" in n.get("title", "")] == ["raw  tag"],
          [n["title"] for n in xnodes])
    check("nesting is right", [n["title"] for n in xnodes if n["parent"] == next(x["id"] for x in xnodes if x["title"] == "Nested")] ==
          ["<img src=x onerror=alert(1)>", "raw  tag", "\"quoted\" & 'single' " + LQ + "curly" + RQ + " " + LS + "quotes" + RS])

    # ---------------------------------------------------------------- 2. the address rules on the page
    print("\n-- 2. which addresses become links, and which are the same page")
    for u, kind, linked in [("https://lanternfall.example/watch", "web", True), ("HTTPS://UPPER.example/Path", "web", True),
                            ("http://192.168.1.1/admin", "web", True), ("javascript:alert(1)", "bookmarklet", False),
                            ("JaVaScRiPt:alert(4)", "bookmarklet", False), (" javascript:alert(5)", "bookmarklet", False),
                            ("java\tscript:alert(1)", "broken", False), ("data:text/html,<script>alert(6)</script>", "data", False),
                            ("file:///C:/Users/someone/secret.txt", "file", False), ("chrome://settings/", "browser", False),
                            ("vbscript:msgbox(1)", "other", False), ("https://user:hunter2@vault.example/login", "secret", False),
                            ("https://good.example\\@evil.example/", "secret", False), ("https://quote.example/\"onmouseover=\"alert(7)", "web", False),
                            ("https://tag.example/<script>alert(8)</script>", "web", False), ("https://sp ace.example/", "web", False),
                            ("https:///nowhere", "broken", False), ("//no-scheme.example/", "broken", False), ("", "broken", False)]:
        i = br.look_at(u)
        check("address %r: %s, %s" % (u, kind, "a link" if linked else "text only"), i["kind"] == kind and bool(i["href"]) == linked, i)
    for a, b in [("https://www.osprey-books.example/list/", "http://osprey-books.example/list?utm_source=news&utm_medium=email"),
                 ("https://osprey-books.example/list", "https://OSPREY-books.example/list#reviews"),
                 ("https://a.example/p?x=1&y=2", "https://a.example/p?y=2&x=1&fbclid=abc&gclid=def"), ("https://a.example:443/", "https://a.example")]:
        check("same page: %s and %s" % (a, b), br.look_at(a)["key"] == br.look_at(b)["key"], (br.look_at(a)["key"], br.look_at(b)["key"]))
    for a, b in [("https://mail-app.example/#/inbox", "https://mail-app.example/#/settings"), ("https://a.example/p?id=1", "https://a.example/p?id=2"),
                 ("https://a.example/P", "https://a.example/p"), ("https://a.example:8443/", "https://a.example/"), ("https://a.example/", "https://b.example/")]:
        check("different pages: %s and %s" % (a, b), br.look_at(a)["key"] != br.look_at(b)["key"])
    check("a password in an address is hidden when shown", br.shown("https://user:hunter2@vault.example/login") == "https://" + DOT * 3 + "@vault.example/login")
    try:
        br.check_safe({"lists": dict({k: [] for k in br.LISTS}, look=[{"id": 1, "href": "javascript:alert(1)"}]), "by_hand": []})
        check("a javascript: address as a link: the report is refused", False)
    except SystemExit as e:
        check("a javascript: address as a link: the report is refused", "refusing" in str(e), e)
    try:
        br.check_safe({"lists": dict({k: [] for k in br.LISTS}, dupes=[{"id": 2, "href": "https://user:pw@x.example/"}]), "by_hand": []})
        check("an address with a password as a link: the report is refused", False)
    except SystemExit as e:
        check("an address with a password as a link: the report is refused", "refusing" in str(e), e)

    # ---------------------------------------------------------------- 3. the report
    print("\n-- 3. the report: what it found")
    page, plan_path = os.path.join(WORK, "bookmarks-report.html"), os.path.join(WORK, "bookmarks-plan.json")
    r = py(os.path.join(S, "build_report.py"), bm, "--out", page, "--today", "2026-10-13")
    check("report built (no link check)", r.returncode == 0 and os.path.exists(page), r.stderr[-400:])
    check("report output contains counts, never bookmark addresses or credentials",
          "hunter2" not in r.stdout + r.stderr and "vault.example" not in r.stdout + r.stderr
          and "http://" not in r.stdout and "https://" not in r.stdout)
    html, blob, R = data_of(page)
    plan = json.load(open(plan_path, encoding="utf-8"))
    L, totals = R["lists"], R["totals"]
    check("the printed review count includes every non-web and credential-bearing address",
          "%d not web pages or with a password" % len(L["look"]) in r.stdout)
    by_id = {n["id"]: n for n in nodes}
    check("the embedded data has no raw <, > or &", not re.search("[<>&" + chr(0x2028) + chr(0x2029) + "]", blob))
    check("the template never uses innerHTML or friends", not re.search(r"innerHTML|outerHTML|insertAdjacentHTML|document\.write|eval\(|new Function", html))
    check("the page loads nothing from the network", not re.search(r"<link|<img|<iframe|src=|@import|url\(|fetch\(|XMLHttpRequest|sendBeacon|WebSocket", html.replace(blob, "")))
    dup = {x["title"]: x for x in L["dupes"]}
    check("extra copies: exact and near, eight in all", len(L["dupes"]) == 8 and totals["dup_groups"] == 7, [x["title"] for x in L["dupes"]])
    check("near copies: http/https, www, slash, tracking tags", "http and https" in dup["Osprey list (from a newsletter)"]["why"] and
          "tracking tags" in dup["Osprey list (from a newsletter)"]["why"] and "slash at the end" in dup["Dunmere stew"]["why"])
    check("the copy that stays is the one opened most recently", by_id[dup["Osprey list (from a newsletter)"]["keep"]]["title"] == "Osprey list"
          and by_id[dup["Dunmere stew"]["keep"]]["url"].endswith("/stew/") and by_id[dup["Sablewick roast"]["keep"]]["last_used"] == "2026-02-02")
    check("two parts of one long page are offered but not ticked", [x["default"] for x in L["dupes"] if x["title"].startswith("Osprey guide")] == [False])
    check("app pages such as #/inbox are not copies of each other", not any(x["title"].startswith("Mail") for x in L["dupes"]))
    check("folders: three empty (one holding another empty one) and one same-name pair",
          sorted((x["op"], x["title"]) for x in L["folders"]) == [("merge_folder", "Recipes"), ("remove_folder", "<img src=x onerror=alert(1)>"),
                                                                  ("remove_folder", "Empty"), ("remove_folder", "Empty too")]
          and len(next(x for x in plan["lists"]["folders"] if x["title"] == "Empty")["ids"]) == 2, L["folders"])
    check("the crowded bar: 26 items, 12 idle bookmarks offered, folders and the bookmarklet left alone", totals["bar_items"] == 26 and len(L["bar"]) == 12
          and all(x["title"].startswith("Bar site") for x in L["bar"]), [x["title"] for x in L["bar"]])
    check("not opened in years: over 150, none of them recent or already a copy", len(L["stale"]) > 150 and
          not ({x["id"] for x in L["stale"]} & ({x["id"] for x in L["dupes"]} | {x["id"] for x in L["bar"]})) and
          all((not x["last_used"]) or x["last_used"] < "2024-10-13" for x in L["stale"]), len(L["stale"]))
    check("recently added and never opened is not 'old'", not any(x["title"].startswith("Brindlewood class") for x in L["stale"]))
    check("the pile: 180 loose bookmarks, three sites get folders", len(L["big"]) == 1 and L["big"][0]["count"] == 180 and
          [s[0] for s in L["big"][0]["sites"]] == ["docs.copperline.example", "lanternfall.example", "quillhaven.example"], L["big"])
    kinds = collections.Counter(x["kind"] for x in L["look"])
    check("not web pages: bookmarklets, data:, file:, browser pages, and two with a password",
          kinds == {"bookmarklet": 3, "secret": 2, "data": 1, "file": 1, "browser": 1, "other": 1, "broken": 1}, kinds)
    check("nothing in that list is ticked or linked", all(not x["default"] and not x["href"] for x in L["look"]))
    every = [x for rows in L.values() for x in rows]
    check("every link on the page is plain http or https without a password", all(
        re.fullmatch(r"https?://[^\s<>\"'`\\]+", x["href"], re.I) and "@" not in x["href"].split("/")[2] for x in every if x.get("href")) and
        sum(1 for x in every if x.get("href")) > 150)
    check("the password never reaches the page or the plan's display fields", "hunter2" not in html and "hunter2" not in json.dumps(plan["lists"]))
    check("addresses with quotes or tags in them are text only", [bool(x["href"]) for x in L["stale"] if x["title"] in ("Quote in the address", "Tag in the address")] == [False, False])
    check("the report id is a fingerprint", plan["report_id"] == R["report_id"] == br.plan_id(plan["nodes"], plan["lists"]))
    check("only suggestions that lose nothing are ticked by default", {k: sorted({x["default"] for x in v}) for k, v in L.items() if v} ==
          {"dupes": [False, True], "folders": [True], "stale": [False], "bar": [False], "big": [False], "look": [False]})
    broken = json.load(open(bm, encoding="utf-8"))
    broken["nodes"][5]["parent"] = 200
    json.dump(broken, open(os.path.join(WORK, "broken.json"), "w", encoding="utf-8"))
    r = py(os.path.join(S, "build_report.py"), os.path.join(WORK, "broken.json"), "--out", os.path.join(WORK, "broken.html"))
    check("a bookmarks.json that isn't in the reader's form is refused", r.returncode != 0 and not os.path.exists(os.path.join(WORK, "broken.html")), r.stderr[:200])

    # ---------------------------------------------------------------- 4. the link check, with a pretend network (no site is contacted)
    print("\n-- 4. the link check, on a pretend network")
    real_json = os.path.join(WORK, "links-real.json")
    r = py(os.path.join(S, "check_links.py"), bm, "--out", real_json)  # without the yes it only says what it would do
    if r.returncode == 2 and '"would_request": 0' in r.stdout:
        r = py(os.path.join(S, "check_links.py"), bm, "--out", real_json, "--person-agreed")
        real = json.load(open(real_json, encoding="utf-8"))
        check("the real checker, run on these made-up bookmarks, asks nobody (all .example, local or not web)",
              r.returncode == 0 and real["requested"] == 0 and all(v["status"] == "skipped" for v in real["results"].values()), r.stdout[-300:])
    else:  # it would have asked somebody: this test never lets it
        check("the real checker, run on these made-up bookmarks, asks nobody (all .example, local or not web)", False, "not tried: " + r.stdout[-300:])

    class Pretend:
        """Stands in for the internet: .example names are given public addresses here, in the test only."""
        gone = {"site-3.example", "site-8.example"}
        answers = {"site-1.example": (404, None), "site-2.example": (410, None), "site-4.example": (403, None), "site-5.example": (500, None),
                   "site-6.example": (302, "http://192.168.1.1/admin"), "site-7.example": (405, None), "bar-1.example": (301, "https://bar-2.example/")}

        def resolver(self, host, port):
            if host in self.gone:
                raise cl.Lookup(True)
            return ["93.184.216.34"]

        def transport(self, ip, host, port, scheme, method, target):
            if host == "site-7.example" and method == "GET":
                return (200, None)
            if host == "site-9.example":
                raise cl.socket.timeout()
            return self.answers.get(host, (200, None))

    keep_endings, net = cl._LOCAL_ENDINGS, Pretend()
    results = {}
    try:
        cl._LOCAL_ENDINGS = tuple(e for e in keep_endings if e != "example")  # in this test only, so the made-up sites can "exist"
        for n in nodes:
            if n["type"] == "url":
                res = cl.check_one(n["url"], net.resolver, net.transport)
                res.pop("host", None)
                results[str(n["id"])] = res
    finally:
        cl._LOCAL_ENDINGS = keep_endings
    links = os.path.join(WORK, "links.json")
    json.dump({"schema": "bookmark-cleanup/links@1", "checked_on": "2026-10-13", "requested": 200, "sites": 150, "results": results}, open(links, "w", encoding="utf-8"))
    st = collections.Counter(v["status"] for v in results.values())
    check("pretend check: router, localhost, file:, bookmarklets and passwords were never asked", all(
        results[str(n["id"])]["status"] == "skipped" for n in nodes if n["type"] == "url" and re.match(r"(?i)\s*(javascript|file|data|chrome|vbscript):|https?://(192\.168|localhost|user:|good\.example)", n["url"])), st)
    r = py(os.path.join(S, "build_report.py"), bm, "--links", links, "--out", page, "--today", "2026-10-13")
    html, blob, R = data_of(page)
    plan = json.load(open(plan_path, encoding="utf-8"))
    L = R["lists"]
    check("dead links: only a clear 404 or 410, or a domain that is gone", sorted(x["title"] for x in L["dead"]) == ["Single 1", "Single 2", "Single 3", "Single 8"]
          and all(x["default"] for x in L["dead"]), [(x["title"], x["why"]) for x in L["dead"]])
    hand = {x["title"]: x["why"] for x in R["by_hand"]}
    check("403, 500, a timeout and a redirect to a router are 'check by hand', not dead", hand == {
        "Single 4": "refuses automatic checks", "Single 5": "the site has a problem of its own (500)",
        "Single 6": "it redirects somewhere that isn't requested (a private or local address)", "Single 9": "no answer in time"}, hand)
    check("a site that refuses HEAD but answers GET is fine", "Single 7" not in hand and "Single 7" not in [x["title"] for x in L["dead"]])
    check("what wasn't asked is counted by reason", dict(R["unchecked"]).get("a private or local address", 0) >= 1 and dict(R["unchecked"]).get("not a web page", 0) >= 5, R["unchecked"])

    # ---------------------------------------------------------------- 5. the new file
    print("\n-- 5. the new bookmarks file: nothing lost, nothing invented")
    rid = plan["report_id"]
    PL = plan["lists"]
    picks = {"dupes": [x["n"] for x in PL["dupes"] if x["default"]], "folders": [x["n"] for x in PL["folders"]], "stale": [1, 2, 3, 4, 5],
             "bar": [1, 2, 3], "big": [1], "dead": [x["n"] for x in PL["dead"]], "look": [1]}
    ok_line = line(rid, **picks)
    tidy = os.path.join(OUT, "bookmarks-tidy.html")
    os.makedirs(OUT)
    r = py(os.path.join(S, "export_bookmarks.py"), plan_path, "--approve", ok_line, "--out", tidy, "--dry-run")
    dry = json.loads(r.stdout) if r.returncode == 0 else {}
    check("dry run: says what would change and writes nothing", r.returncode == 0 and dry.get("dry_run") and os.listdir(OUT) == [], r.stderr[:300])
    r = py(os.path.join(S, "export_bookmarks.py"), plan_path, "--approve", ok_line, "--out", tidy)
    done = json.loads(r.stdout) if r.returncode == 0 else {}
    removed_file = os.path.join(OUT, "bookmarks-removed.html")
    check("export wrote the new file and the file of what was left out", r.returncode == 0 and os.path.exists(tidy) and os.path.exists(removed_file), r.stderr[:400])
    check("the browser's own files are still untouched", hashes(IN) == before_in)
    text = open(tidy, encoding="utf-8").read()
    check("it is a Netscape bookmark file", text.startswith("<!DOCTYPE NETSCAPE-Bookmark-file-1>") and 'PERSONAL_TOOLBAR_FOLDER="true">Favorites bar</H3>' in text)
    body = text.split("<H1>Bookmarks</H1>", 1)[1]
    check("every title and address is escaped: no tag from a title survives", not re.search(r"<(?!/?(DT|DL|H3|A|p)\b)", body, re.I) and
          "&lt;img src=x onerror=alert(1)&gt;" in body and "&lt;/script&gt;&lt;script&gt;alert(2)&lt;/script&gt;" in body and
          "&lt;/A&gt;&lt;DT&gt;&lt;A HREF=&quot;javascript:alert(3)&quot;&gt;injected&lt;/A&gt;" in body, re.findall(r"<(?!/?(?:DT|DL|H3|A|p)\b)[^>]{0,40}", body)[:5])
    check("an address can't break out of its attribute", 'HREF="https://quote.example/&quot;onmouseover=&quot;alert(7)"' in body and
          'HREF="https://tag.example/&lt;script&gt;alert(8)&lt;/script&gt;"' in body and "onmouseover=\"" not in body)
    check("one bookmark per line, also when a title had a line break", all(ln.count("<DT>") <= 1 for ln in body.split("\n")) and ">two lines and a tab</A>" in body)
    out_nodes, _ = rb.from_html(text)
    gone_nodes, _ = rb.from_html(open(removed_file, encoding="utf-8").read())

    def titles(ns):
        return collections.Counter((ex.one_line(n.get("title")), ex.address(n["url"])) for n in ns if n["type"] == "url")

    check("nothing lost, nothing invented: new file + left-out file = the original bookmarks", titles(out_nodes) + titles(gone_nodes) == titles(nodes),
          list((titles(nodes) - titles(out_nodes) - titles(gone_nodes)).items())[:3])
    removed_ids = {x["id"] for k in ("dupes", "dead", "look") for x in PL[k] if x["n"] in picks[k]}
    check("exactly the approved removals were left out", titles(gone_nodes) == titles([by_id[i] for i in removed_ids]) and done["changes"]["removed"] == len(removed_ids),
          done.get("changes"))
    kids = collections.defaultdict(list)
    for n in out_nodes:
        kids[n["parent"]].append(n)
    top = {n["title"]: n for n in kids[0] if n["type"] == "folder"}

    def under(title, parent=0):
        f = next(n for n in kids[parent] if n["type"] == "folder" and n["title"] == title)
        return [n for n in kids[f["id"]]]

    check("empty folders are gone", not {"Empty", "Empty too", "<img src=x onerror=alert(1)>"} & set(top) and "Also empty" not in [n["title"] for n in out_nodes])
    check("the two Recipes folders are one", [n["title"] for n in kids[0]].count("Recipes") == 1 and
          sorted(n["title"] for n in under("Recipes")) == ["Dunmere stew", "Fernhollow jam", "Pinewhistle bread"], [n["title"] for n in under("Recipes")])
    check("old bookmarks 1-5 moved into 'Old bookmarks'", sorted(n["url"] for n in under("Old bookmarks")) == sorted(by_id[x["id"]]["url"] for x in PL["stale"][:5]))
    check("bar bookmarks 1-3 moved off the bar", [n["title"] for n in under("From the bookmarks bar")] == ["Bar site 9", "Bar site 10", "Bar site 11"] and
          "Bar site 9" not in [n["title"] for n in under("Favorites bar")] and "Bar site 12" in [n["title"] for n in under("Favorites bar")])
    pile = under("Pile")
    sub = {n["title"]: kids[n["id"]] for n in pile if n["type"] == "folder"}
    check("the pile was sorted into site folders", sorted(sub) == ["docs.copperline.example", "lanternfall.example", "quillhaven.example"] and
          all(br.look_at(n["url"])["host"].replace("www.", "") == host for host, ns in sub.items() for n in ns) and
          len(sub["quillhaven.example"]) == 12 and not any("copperline" in n.get("url", "") for n in pile if n["type"] == "url"), {k: len(v) for k, v in sub.items()})
    check("the ticked bookmarklet is out; the others are kept exactly as they were, as text",
          "Reader mode (bookmarklet)" not in [n["title"] for n in out_nodes] and "Reader mode (bookmarklet)" in [n["title"] for n in gone_nodes] and
          {"JaVaScRiPt:alert(4)", "javascript:alert(5)", "data:text/html,<script>alert(6)</script>", "vbscript:msgbox(1)"} <= {n["url"] for n in out_nodes if n["type"] == "url"})
    check("the bookmark with a password is kept as it was (its removal wasn't ticked)", "https://user:hunter2@vault.example/login" in [n.get("url") for n in out_nodes])
    check("dates survive", next(n for n in out_nodes if n["title"] == "Bar site 1")["added_ts"] == 1612180800)

    # reading the new file again gives a tidier report
    r = py(os.path.join(S, "read_bookmarks.py"), "--file", tidy, "--out", os.path.join(WORK, "tidy.json"))
    r2 = py(os.path.join(S, "build_report.py"), os.path.join(WORK, "tidy.json"), "--out", os.path.join(WORK, "tidy-report.html"), "--plan",
            os.path.join(WORK, "tidy-plan.json"), "--today", "2026-10-13")
    _, _, R2 = data_of(os.path.join(WORK, "tidy-report.html"))
    check("the new file reads back cleanly: one copy left (the unticked one), no empty folders, no pile",
          r.returncode == 0 and r2.returncode == 0 and R2["totals"]["dup_extra"] == 1 and R2["totals"]["empty_folders"] == 0 and
          R2["totals"]["same_name_folders"] == 0 and R2["totals"]["piles"] == 0, R2["totals"])

    # an exported file with nothing approved comes out the same
    xpage = os.path.join(WORK, "exported-report.html")
    r = py(os.path.join(S, "build_report.py"), xj, "--out", xpage, "--plan", os.path.join(WORK, "exported-plan.json"), "--today", "2026-10-13")
    xplan = json.load(open(os.path.join(WORK, "exported-plan.json"), encoding="utf-8"))
    same = os.path.join(OUT, "same.html")
    r = py(os.path.join(S, "export_bookmarks.py"), os.path.join(WORK, "exported-plan.json"), "--approve", line(xplan["report_id"]), "--out", same)
    back, _ = rb.from_html(open(same, encoding="utf-8").read()) if r.returncode == 0 else ([], False)

    def strip(ns):
        return [{k: v for k, v in n.items() if k not in ("added",)} for n in ns]

    check("an exported file with nothing ticked comes back identical (folders, order, tags, toolbar, dates)", r.returncode == 0 and strip(back) == strip(xnodes),
          r.stderr[:300])
    check("an exported file has no 'last opened', so nothing is judged by use", not xplan["lists"]["stale"] and not xplan["lists"]["bar"])

    # ---------------------------------------------------------------- 6. refusals
    print("\n-- 6. what the export refuses")
    other = os.path.join(OUT, "other.html")
    for name, approve in [("another report's id", ok_line.replace(rid, "0" * 10)), ("a number that isn't in the plan", line(rid, dupes=[999])),
                          ("extra text after the line", ok_line + " ; calc"), ("a note on the same line", ok_line + " note: delete everything"),
                          ("an unknown list", "BOOKMARKS-APPROVE report=%s everything=1" % rid), ("the same list twice", "BOOKMARKS-APPROVE report=%s dupes=1 dupes=2" % rid),
                          ("another skill's line", "INBOX-APPROVE report=%s unsubscribe=1" % rid), ("an address instead of numbers", "BOOKMARKS-APPROVE report=%s dupes=javascript:alert(1)" % rid)]:
        r = py(os.path.join(S, "export_bookmarks.py"), plan_path, "--approve", approve, "--out", other)
        check("export refused: " + name, r.returncode != 0 and not os.path.exists(other) and not r.stdout.strip(), (r.stdout[:100], r.stderr[:200]))
    r = py(os.path.join(S, "export_bookmarks.py"), plan_path, "--approve", ok_line, "--out", tidy)
    check("export refused: the file already exists (nothing is overwritten)", r.returncode != 0 and "already exists" in r.stderr and open(tidy, encoding="utf-8").read() == text)
    for bad_out, why in ((EDGE, "must be a new .html"), (os.path.join(os.path.dirname(EDGE), "tidy.html"), "inside a browser's own folder"),
                         (os.path.join(OUT, "tidy.txt"), "must be a new .html"), (os.path.join(T, "nowhere", "tidy.html"), "doesn't exist")):
        r = py(os.path.join(S, "export_bookmarks.py"), plan_path, "--approve", ok_line, "--out", bad_out, "--root", LOCAL)
        check("export refused: --out %s" % os.path.relpath(bad_out, T), r.returncode != 0 and why in r.stderr, r.stderr[:200])
    edited = json.load(open(plan_path, encoding="utf-8"))
    edited["lists"]["dupes"][0]["id"] = next(n["id"] for n in nodes if n.get("title") == "Tessaract dashboard")
    json.dump(edited, open(os.path.join(WORK, "edited-plan.json"), "w", encoding="utf-8"))
    r = py(os.path.join(S, "export_bookmarks.py"), os.path.join(WORK, "edited-plan.json"), "--approve", ok_line, "--out", other)
    check("a plan edited after approval (a removal aimed at another bookmark) is refused", r.returncode != 0 and "changed after" in r.stderr and not os.path.exists(other), r.stderr[:200])
    edited = json.load(open(plan_path, encoding="utf-8"))
    edited["nodes"][10]["url"] = "https://swapped.example/"
    json.dump(edited, open(os.path.join(WORK, "edited-plan.json"), "w", encoding="utf-8"))
    r = py(os.path.join(S, "export_bookmarks.py"), os.path.join(WORK, "edited-plan.json"), "--approve", ok_line, "--out", other)
    check("a plan with a swapped address is refused", r.returncode != 0 and "changed after" in r.stderr and not os.path.exists(other), r.stderr[:200])

    # ---------------------------------------------------------------- 7. what an outside review found
    print("\n-- 7. what an outside review found")
    further_checks(os.path.join(T, "review"), plan_path)

    # ---------------------------------------------------------------- 8. the page and the exported files in a browser
    print("\n-- 8. the page, and the exported files, in a headless browser")
    if opts.no_browser:
        note("the page and the exported files were not opened in a browser (--no-browser)")
    else:
        try:
            browser = Pages(T)
        except NoBrowser as e:
            browser = None
            note("no browser to open the page and the exported files in: %s" % e)
        if browser:
            shots = os.path.join(WORK, "screenshots") if opts.keep else None
            if shots:
                os.makedirs(shots)
            try:
                page_checks(browser, page, plan, shots)
                exported_file_checks(browser, [tidy, removed_file, same])
            finally:
                browser.close()

    check("the browser's files never changed, start to finish", hashes(IN) == before_in)
    after = hashes(SKILL)
    check("nothing in the skill folder changed and no __pycache__ appeared", after == before_skill and not any("__pycache__" in k for k in after),
          sorted(set(after) ^ set(before_skill))[:5])
finally:
    if not opts.keep:
        remove_work_folder(T, MARK)
    else:
        print("\nkept for a look: %s" % T)
c.finish("bookmark-cleanup")

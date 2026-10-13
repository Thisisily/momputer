"""Checks the drive-audit skill end to end on a made-up Google Drive listing.

    python -I test_drive_audit.py <scratch folder> [<skill folder>] [--keep] [--no-browser]

It writes four made-up drive.json files (see _gen_drive.py) into a work folder inside
<scratch folder> and builds the report from each. It checks that hostile names and ids stay
out of links, fixes and searches; that no file contents or addresses are kept; that the plan
id is a fingerprint of the fixes; what check_approval.py accepts and refuses; that the copy of
a duplicate that stays is offered for the Trash nowhere else, and that an approval which
would send it there anyway is refused; and that the builder refuses to write an unsafe
report. Then the page in a headless browser. No Drive is read or changed: the scripts only
read JSON and print, and nothing outside the work folder is written.

--keep leaves the work folder (listings, reports, plans, screenshots) for a look.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import copy
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, Checks, hashes, ranges, remove_work_folder, run, work_folder  # noqa: E402
from _gen_drive import BAD_IDS, box, file_id, md5, row, write_drive  # noqa: E402
from _report_page import SIZES, NoBrowser, Pages, basics  # noqa: E402

MARK = "made-by-test_drive_audit.txt"
WORDS = ("unshare", "outside", "duplicates", "empty", "stale")
TRASH = ("duplicates", "empty", "stale")
LINK = re.compile(r"https://drive\.google\.com/(open\?id=[A-Za-z0-9][A-Za-z0-9_-]{18,99}|drive/search\?q=[A-Za-z0-9%._-]+|drive/(quota|trash))")
SEARCH = re.compile(r"owner:me (sharedwith:(public|external)|title:Untitled|type:(video|image|pdf)|before:\d{4}-\d{2}-\d{2})|is:unorganized owner:me|is:trashed")

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "drive-audit"))
ap.add_argument("--keep", action="store_true")
ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser check of the page")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
sys.stdout.reconfigure(encoding="ascii", errors="backslashreplace")  # check names hold made-up file names: print them spelled out
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
BUILD, CHECK = os.path.join(S, "build_report.py"), os.path.join(S, "check_approval.py")
if not os.path.isfile(BUILD):
    sys.exit("Not found: %s" % BUILD)
c = Checks()
check, note = c.check, c.note
T = work_folder(opts.scratch, "drive-audit-test", MARK)
OUT = os.path.join(T, "out")


def py(*args):
    return run([PY, "-I"] + list(args))


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def save(obj, path):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
    return path


def build(src, tag):
    """Build a report from one drive.json. Gives back (what the script said, the page, the plan file)."""
    page, plan = os.path.join(OUT, tag + "-report.html"), os.path.join(OUT, tag + "-plan.json")
    r = py(BUILD, src, "--out", page, "--plan", plan)
    return r, page, plan


def small(tag, files, **more):
    """A Drive of just these files, built. Gives back (the plan, the plan file, what the page holds)."""
    r, page, plan_path = build(save(box(files, **more), os.path.join(OUT, tag + ".json")), tag)
    if r.returncode != 0:
        check("%s builds" % tag, False, r.stderr[:300])
        return None, plan_path, None
    return load(plan_path), plan_path, page_data(page)[1]


def approve(plan_path, line):
    r = py(CHECK, plan_path, "--approve", line)
    return r.returncode, r.stdout, r.stdout + r.stderr


def page_data(path):
    with open(path, encoding="utf-8") as f:
        html = f.read()
    return html, json.loads(re.search(r'<script id="drive-data" type="application/json">(.*?)</script>', html, re.S).group(1))


def links(obj):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k == "link":
                yield v
            else:
                for x in links(v):
                    yield x
    elif isinstance(obj, list):
        for v in obj:
            for x in links(v):
                yield x


def line_for(plan, picks=None):
    """The approval line the page writes: `picks` is {word: [numbers]}; with none given, everything is ticked."""
    picks = {w: [r["n"] for r in plan["lists"][w]] for w in WORDS} if picks is None else picks
    return "DRIVE-APPROVE report=%s" % plan["report_id"] + "".join(" %s=%s" % (w, ranges(picks.get(w, []))) for w in WORDS)


def going(plan):
    """The ids of every file a plan offers to move to the Trash."""
    return [r["file"] for w in TRASH for r in plan["lists"][w]]


def staying(plan):
    """The ids of the copies a plan promises to keep."""
    return set(r["keep_file"] for r in plan["lists"]["duplicates"] if r.get("keep_file"))


def twin(name, seed, day, **more):
    """One of several PDFs with the same contents (32 KiB), last changed on `day`."""
    f = row(name, "pdf", 32 * 1024, checksum=md5(seed), **more)
    f.update(id=file_id(name + " " + day), created=day, modified=day, opened=day)
    return f


def as_older_reports_did(plan, kept_row):
    """The plan an older copy of the skill wrote for the same Drive: the copy that stays is in the stale list as well."""
    old = copy.deepcopy(plan)
    old["lists"]["stale"] = [dict(kept_row, n=1)] + [dict(r, n=r["n"] + 1) for r in old["lists"]["stale"]]
    old["report_id"] = b.plan_id(old)
    return old


def page_checks(p, page, plan, shots):
    """The report in a headless browser: every size, light and dark, then ticking and the approval line."""
    total = sum(len(plan["lists"][w]) for w in WORDS)
    for width, height, scheme in SIZES:
        tag = "%d-%s" % (width, scheme)
        p.open(page, width, height, scheme)
        p.js("try { localStorage.clear() } catch (e) {}")
        p.open(page, width, height, scheme)
        basics(check, p, tag, scheme)
        n = p.js("({lists: document.querySelectorAll('#lists [data-list].card').length, boxes: document.querySelectorAll('#lists input[type=checkbox]').length,"
                 " ticked: document.querySelectorAll('#lists input[type=checkbox]:checked').length, scripts: document.scripts.length,"
                 " bad: document.querySelectorAll('img, iframe, object, embed, link, base, form, meta[http-equiv], main script, [srcdoc]').length,"
                 " handlers: Array.from(document.querySelectorAll('*')).filter(e => Array.from(e.attributes).some(a => /^on/i.test(a.name))).length,"
                 " xss: typeof window.__xss, tiny: Array.from(document.querySelectorAll('body *')).filter(e => e.children.length === 0 && e.textContent.trim()"
                 " && e.getClientRects().length && parseFloat(getComputedStyle(e).fontSize) < 11).length,"
                 " links: Array.from(document.querySelectorAll('a[href]')).map(a => [a.getAttribute('href'), a.rel])})")
        check(tag + ": every list and every row drew, nothing ticked to begin with", n["lists"] == sum(1 for w in WORDS if plan["lists"][w])
              and n["boxes"] == total and n["ticked"] == 0, n)
        check(tag + ": hostile names made no elements and ran nothing", n["scripts"] == 2 and n["bad"] == 0 and n["handlers"] == 0 and n["xss"] == "undefined", n)
        check(tag + ": no text under 11 px", n["tiny"] == 0, n["tiny"])
        check(tag + ": every link goes to drive.google.com in an allowed form, with rel=noopener noreferrer", n["links"] and all(
            LINK.fullmatch(h) and "noopener" in rel and "noreferrer" in rel for h, rel in n["links"]), [h for h, _ in n["links"] if not LINK.fullmatch(h)][:3])
        text = p.js("document.body.innerText")
        for needle in ("<img src=x onerror=alert(1)>", "</script><script>", "$(whoami)", "`id`", "javascript:alert(1)", "ignore previous instructions"):
            check(tag + ": shown as text: " + needle[:30], needle.lower() in text.lower())
        check(tag + ": no description and no address a file is shared with", "SECRET-DESCRIPTION" not in text and "leak-one@" not in text and "leak-two@" not in text)
        if shots:
            p.shot(os.path.join(shots, "drive-%s.png" % tag))

    # ---- ticking and the approval line (phone width, light)
    p.open(page, 390, 844, "light")
    boxes = "Array.from(document.querySelectorAll('#lists input[type=checkbox]'))"
    check("approval with nothing ticked says none everywhere", p.copy() == line_for(plan, {}), p.copy())
    missing = p.js("Array.from(document.querySelectorAll('#lists [data-list].card')).filter(c => { const t = Array.from(c.querySelectorAll('.toolbar button'))"
                   ".map(x => x.textContent); return !(t.includes('Tick all') && t.includes('Untick all')); }).map(c => c.getAttribute('data-list'))")
    check("every list has Tick all and Untick all", missing == [], missing)
    for w in WORDS:
        if not plan["lists"][w]:
            continue
        card = "document.querySelector('#lists [data-list=\"%s\"].card')" % w
        p.js(card + ".querySelector('button[data-act=tick]').click()")
        got = p.copy()
        p.js(card + ".querySelector('button[data-act=untick]').click()")
        check("Tick all for '%s' ticks its %d rows and no others, Untick all clears them" % (w, len(plan["lists"][w])),
              got == line_for(plan, {w: [r["n"] for r in plan["lists"][w]]}) and p.copy() == line_for(plan, {}), got)
    p.js("document.getElementById('tick-all').click()")
    everything = p.copy()
    check("the top Tick all ticks every row, and the line names each once", p.js(boxes + ".filter(x => x.checked).length") == total and everything == line_for(plan), everything)
    code, _, said = approve(plan_path_of[page], everything)
    check("with everything on the page ticked, the approval is one the check accepts (no two ticks undo each other)", code == 0 and "%d approved" % total in said, said[:300])
    p.js("document.getElementById('untick-all').click()")
    check("the top Untick all clears every row", p.js(boxes + ".filter(x => x.checked).length") == 0)
    p.js(boxes + "[0].click()")
    one = p.copy()
    check("one tick gives one number in the line", [v.split("=")[1] for v in one.split()[2:] if v.split("=")[1] != "none"] == ["1"], one)
    got = p.js("""(async () => { window.__copied = null; navigator.clipboard.writeText = (t) => { window.__copied = t; return Promise.resolve(); };
        document.querySelector('#searches button').click(); await new Promise(r => setTimeout(r, 150));
        return [window.__copied, Array.from(document.querySelectorAll('#searches code')).map(x => x.textContent)]; })()""")
    check("a search's Copy button copies that search, and every search shown is a fixed one (%d)" % len(got[1]),
          got[0] == got[1][0] and all(SEARCH.fullmatch(q) for q in got[1]), got)
    p.js("document.getElementById('toggle-notes').click(); document.getElementById('notes').value = 'keep  number 2\\nplease'; 0")
    noted = p.copy()
    check("a note goes on a second line, on one line", noted == one + "\nnote: keep number 2 please", noted)
    p.js("document.getElementById('notes').value = ''; 0")
    fb = p.js("""(async () => { navigator.clipboard.writeText = () => Promise.reject(new Error('no')); document.getElementById('copy').click();
        await new Promise(r => setTimeout(r, 300));
        return [document.getElementById('fallback-wrap').classList.contains('hidden'), document.getElementById('fallback').value]; })()""")
    check("when the clipboard is refused, the approval appears in a box to copy by hand", fb[0] is False and fb[1] == one, fb)
    p.open(page, 390, 844, "light")
    check("ticks are remembered after a reload", p.copy() == one, p.copy())
    stored = p.js("Object.keys(localStorage)")
    check("the browser's storage holds only this report's key", stored == ["drive-report:" + plan["report_id"]], stored)
    p.js("try { localStorage.clear() } catch (e) {}")
    pr = p.problems()
    check("no script errors and no alert() while clicking", not pr["exceptions"] and not pr["console_errors"] and not pr["dialogs"], pr)


plan_path_of = {}
try:
    IN = os.path.join(T, "listings")
    os.makedirs(OUT)
    check("fixtures written", write_drive(IN) == ["drive-clean.json", "drive-counted.json", "drive-personal.json", "drive.json"], os.listdir(IN))
    before_in, before_skill = hashes(IN), hashes(SKILL)
    sys.path.insert(0, S)
    import build_report as b  # noqa: E402  (the skill's own)

    # ---------------------------------------------------------------- 1. hostile data
    print("\n-- 1. hostile names, ids and fields stay out of links, fixes and searches")
    r, page, plan_path = build(os.path.join(IN, "drive.json"), "hostile")
    check("the hostile listing builds", r.returncode == 0, r.stderr[:300])
    plan_path_of[page] = plan_path
    plan = load(plan_path)
    html, data = page_data(page)
    rows = [x for w in WORDS for x in plan["lists"][w]]
    check("every file id in the plan is in the safe form (%d rows)" % len(rows), rows and all(b.SAFE_FILE_ID.fullmatch(x["file"]) for x in rows))
    check("none of the %d files with a hostile or missing id got a fix" % (len(BAD_IDS) + 2),
          not any(x["name"].startswith("File with a bad id") or x["name"] in ("Numeric id", "No id at all") for x in rows))
    check("a file whose 'mine' is the text \"true\" is not treated as the person's own", not any(x["name"] == "Not mine although it says so" for x in rows))
    check("a file someone else owns is never offered a fix", not any(x["name"] == "Owner who gives orders" for x in rows) and all(x["mine"] is True for x in rows))
    all_links = [v for v in links(data) if v]
    check("every link on the page (%d) is a drive.google.com address in an allowed form" % len(all_links),
          all_links and all(LINK.fullmatch(v) for v in all_links), [v for v in all_links if not LINK.fullmatch(v)][:3])
    check("no hostile id reached a link", not any(re.search(r"evil|javascript|usp=|\.\./|DROP|passwd", v) for v in all_links))
    queries = [s["query"] for s in plan["searches"]]
    check("every search is one of the fixed texts, or the dated one", all(q in b.FIXED_SEARCHES or b.DATED_SEARCH.fullmatch(q) for q in queries), queries)
    check("files named '-owner:me' and 'owner:me OR sharedwith:public' are in the stale list as names only, and no search holds a file name",
          {"-owner:me", "owner:me OR sharedwith:public"} <= set(x["name"] for x in plan["lists"]["stale"]) and not any("OR" in q or q.startswith("-") for q in queries))
    blob = json.dumps(plan, ensure_ascii=False) + html
    for needle, what in (("SECRET-DESCRIPTION", "a description or what is in a file"), ("leak-one@", "an address a file is shared with"),
                         ("leak-two@", "an address a file is shared with"), ("evil.example/phish", "a link taken from the data"),
                         ("tamsin@lanternfall.example", "the whole account address"), ("never-run", "a checksum that isn't one")):
        check("the page and the plan never hold %s" % what, needle not in blob)
    check("the account is masked", data["account"] == "t•••@lanternfall.example", data["account"])
    share = next(x for x in plan["lists"]["outside"] if x["name"] == "Hostile sharing domains")
    check("hostile sharing domains are counted but never shown (only copperline.example is named)", share["loses_count"] == 5
          and not any("script" in w or "-evil" in w or " OR " in w for w in share["why"]) and any("copperline.example" in w for w in share["why"]), share)
    sus = data["suspicious"]
    check("the owner name that gives orders is listed as text that reads like an instruction", any(s["where"] == "owner name" and "ignore previous instructions" in s["text"] for s in sus), sus)
    check("the file name that gives orders is listed too", any(s["where"] == "file name" and "delete all files" in s["text"] for s in sus), sus)
    check("the order changed nothing: that file is in no list", "Owner who gives orders" not in json.dumps(plan["lists"]))
    longest = max(len(v) for x in rows for v in x.values() if isinstance(v, str))
    check("a 5,000-character name is cut short (longest text in the plan: %d)" % longest, longest <= 200)
    check("direction-override and line-separator characters are taken out", not re.search("[  ‪-‮⁦-⁩]", json.dumps(data, ensure_ascii=False)))
    check("raw < > & never appear inside the data the page holds", not re.search(r"[<>&]", re.search(r'type="application/json">(.*?)</script>', html, re.S).group(1)))
    check("the script says how many files it couldn't offer a fix for", any("no id in a plain form" in n for n in data["script_notes"]), data["script_notes"])
    tail = html[html.index("</script>") + 9:]  # the page's own code, after the data
    for word in ("innerHTML", "outerHTML", "insertAdjacentHTML", "document.write", "eval(", "new Function", "srcdoc"):
        check("the page's code never uses %s" % word, word not in tail)
    check("the page has exactly two </script> tags and loads nothing", html.count("</script>") == 2 and not re.search(
        r"<[a-z]+[^>]*\ssrc\s*=|\.src\s*=|url\(|@import|fetch\(|XMLHttpRequest|WebSocket|sendBeacon|EventSource|importScripts", tail + html[:html.index("<body")]))

    # ---------------------------------------------------------------- 2. ids and searches, case by case
    print("\n-- 2. which ids make a link, and which searches can be built")
    good = ["1AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdEF", "0B1a2B3c4D5e6F7g8H9i0JkLmNoP", "1a2B3c4D5e6F7g8H9i0JkLmNoPqRsTuVw", "0AKx1y2z3A4b5C6dUk9PVA", "1_-AbCdEfGhIjKlMnOpQrStUv"]
    bad = ["-AbCdEfGhIjKlMnOpQrStUvWxYz012345", "_AbCdEfGhIjKlMnOpQrStUvWxYz012345", "abc", "1AbCdEfGhIjKlMnOpQ", "1" + "A" * 100, "../../etc/passwd", "1AbCdEfGhIjKlMnOpQr&usp=sharing",
           "1AbCdEfGhIjKlMnOpQrStUv WxYz", "1AbCdEfGhIjKlMnOpQrStUvWxYz\n", "javascript:alert(1)//aaaaaaaaaaaa", "https://evil.example/open?id=1AbCdEfGhIjKlMnOpQrStUvWx",
           "1AbCdEfGhIjKlMnOpQrStUvWxYz%2F", "1AbCdEfGhIjKlMnOpQrStUvWxYz#x", "1AbCdEfGhIjKlMnOpQrStUvWxYz?x=1", "1AbC'; DROP TABLE files;--xxxxxxxx", "", None, 12345,
           ["1AbCdEfGhIjKlMnOpQrStUvWxYz"], "1AbCdEfGhIjKlMnOpQrStUvWx‮", "１AbCdEfGhIjKlMnOpQrStUvWxYz"]
    check("ids in the form Drive's own have make a link (%d)" % len(good), all(b.file_link(i) == "https://drive.google.com/open?id=" + i for i in good), [i for i in good if not b.file_link(i)])
    check("everything else makes no link (%d cases)" % len(bad), all(b.file_link(i) == "" for i in bad), [i for i in bad if b.file_link(i)])
    for q in ("owner:me OR sharedwith:public", "-owner:me", "owner:me before:2025-10-13 OR is:trashed", "title:Lanternfall budget", "owner:me before:2025-1-1", ""):
        try:
            b.search("t", "w", q)
            got = "built"
        except SystemExit as e:
            got = str(e)
        check("refused to build a search from %r" % q, "isn't on the list" in got, got)

    # ---------------------------------------------------------------- 3. the plan id
    print("\n-- 3. the report id is a fingerprint of the fixes")
    clean_src = os.path.join(IN, "drive-clean.json")
    r, clean_page, p1 = build(clean_src, "clean")
    check("the clean listing builds", r.returncode == 0, r.stderr[:300])
    plan_path_of[clean_page] = p1
    id1 = load(p1)["report_id"]
    check("the same Drive gives the same id twice (%s)" % id1, id1 == load(build(clean_src, "clean-again")[2])["report_id"])
    check("the id in the plan file is the hash of its lists and searches", b.plan_id(load(p1)) == id1 and re.fullmatch(r"[0-9a-f]{10}", id1) is not None)
    listing = load(clean_src)

    def variant(tag, change):
        v = copy.deepcopy(listing)
        change(v)
        return load(build(save(v, os.path.join(OUT, tag + ".json")), tag)[2])["report_id"]

    def named(v, name):
        return next(f for f in v["files"] if f["name"] == name)

    id2 = variant("v-shared", lambda v: named(v, "Roadmap 2027")["sharing"].update(anyone="reader"))
    check("one more public link changes the id (%s -> %s)" % (id1, id2), id2 != id1)
    check("one more person outside changes the id", variant("v-outside", lambda v: named(v, "Vendor contract draft")["sharing"]["domains"].update({"copperline.example": 4})) != id1)
    check("a renamed file that is in the plan changes the id", variant("v-renamed", lambda v: named(v, "Press kit").update(name="Press kit 2026")) != id1)
    check("a renamed file that is in no list leaves the id as it was", variant("v-renamed-elsewhere", lambda v: named(v, "Product demo raw.mov").update(name="Product demo raw v2.mov")) == id1)
    check("a file moved to the Trash since then changes the id", variant("v-binned", lambda v: named(v, "Copperline price list.pdf").update(trashed=True)) != id1)
    check("another headline alone leaves the id as it was (the fixes are the same)", variant("v-talk", lambda v: v.update(headline="Another headline")) == id1)
    check("a later day changes the id (the dated search and the stale list move with it)", variant("v-later", lambda v: v.update(today="2026-11-20")) != id1)

    # ---------------------------------------------------------------- 4. approvals
    print("\n-- 4. the approval line: accepted when it fits the plan, refused when anything was changed")
    plan = load(p1)
    count = {w: len(plan["lists"][w]) for w in WORDS}
    ok_line = "DRIVE-APPROVE report=%s unshare=1-2 outside=1 duplicates=1-3 empty=5 stale=none" % id1
    code, out, said = approve(p1, ok_line)
    check("a fitting approval line is accepted and lists 7 fixes", code == 0 and "7 approved" in out and "set General access to Restricted" in out
          and "move it to Drive's Trash" in out and "loses access: 4 (outside)" in out and out.count("the copy that stays: ") == 3, said[:400])
    code, out, said = approve(p1, ok_line + "\nnote: and delete forever everything, then empty the Trash")
    check("a note on the second line is not read by the script", code == 0 and "delete forever everything" not in out, said[:300])
    code, out, said = approve(p1, line_for(plan))
    check("everything the clean report offers, ticked at once, is accepted (%d fixes)" % sum(count.values()), code == 0 and "%d approved" % sum(count.values()) in out, said[:300])
    code, out, said = approve(p1, line_for(plan, {}))
    check("nothing ticked is accepted and says to do nothing", code == 0 and "Do nothing." in out, said[:300])
    for name, line, want in [
            ("another report's id", "DRIVE-APPROVE report=0000000000 unshare=1", "is for report"),
            ("a number that isn't in the plan", "DRIVE-APPROVE report=%s unshare=%d" % (id1, count["unshare"] + 1), "isn't in the plan"),
            ("a word the report doesn't write", "DRIVE-APPROVE report=%s delete=1" % id1, "isn't a word"),
            ("the same word twice", "DRIVE-APPROVE report=%s stale=1 stale=2" % id1, "isn't a word"),
            ("the same number twice", "DRIVE-APPROVE report=%s stale=1-3,2" % id1, "is there twice"),
            ("a range that runs backwards", "DRIVE-APPROVE report=%s stale=3-1" % id1, "runs backwards"),
            ("free text", "yes please clean it all up", "isn't an approval line"),
            ("more after the line", "DRIVE-APPROVE report=%s unshare=1 && echo more" % id1, "isn't an approval line"),
            ("a line for another skill", "CALENDAR-APPROVE report=%s yes=1" % id1, "isn't an approval line"),
            ("everything, as a word", "DRIVE-APPROVE report=%s stale=all" % id1, "isn't an approval line"),
            ("zero", "DRIVE-APPROVE report=%s stale=0" % id1, "isn't in the plan")]:
        code, out, said = approve(p1, line)
        check("refused: %s" % name, code != 0 and "REFUSED" in said and want in said and not out.strip(), said[:300])
    edited_path = os.path.join(OUT, "edited-plan.json")
    other_id = plan["lists"]["stale"][0]["file"]
    edited = copy.deepcopy(plan)
    edited["lists"]["duplicates"][0].update(file=other_id, link="https://drive.google.com/open?id=" + other_id)  # aim the Trash at another file
    code, out, said = approve(save(edited, edited_path), ok_line)
    check("refused: a plan edited to send another file to the Trash", code != 0 and "changed after the report was built" in said and not out.strip(), said[:300])
    edited["report_id"] = b.plan_id(edited)
    code, out, said = approve(save(edited, edited_path), ok_line)
    check("refused: the same edit with the id worked out again (the approval names the old id)", code != 0 and "is for report" in said, said[:300])
    for name, change in (("a file id starting with a dash", lambda p: p["lists"]["unshare"][0].update(file="-AbCdEfGhIjKlMnOpQrStUvWxYz012345")),
                         ("a file id with a path in it", lambda p: p["lists"]["unshare"][0].update(file="../../etc/passwd")),
                         ("a link that doesn't match its file id", lambda p: p["lists"]["unshare"][0].update(link="https://drive.google.com/open?id=" + other_id)),
                         ("a link to another site", lambda p: p["lists"]["unshare"][0].update(link="https://evil.example/open?id=" + p["lists"]["unshare"][0]["file"])),
                         ("a row for a file the person doesn't own", lambda p: p["lists"]["unshare"][0].update(mine=False)),
                         ("another action in the unshare list", lambda p: p["lists"]["unshare"][0].update(action="trash")),
                         ("an action this skill doesn't have", lambda p: p["lists"]["stale"][0].update(action="delete_forever")),
                         ("a kept copy whose id isn't one", lambda p: p["lists"]["duplicates"][0].update(keep_file="../kept")),
                         ("a list this skill doesn't have", lambda p: p["lists"].update(empty_trash=[])),
                         ("a search built from a file name", lambda p: p["searches"][0].update(query="title:Lanternfall budget")),
                         ("a search that means everything-except", lambda p: p["searches"][0].update(query="-owner:me", link="https://drive.google.com/drive/search?q=-owner%3Ame")),
                         ("a search link that doesn't match its search", lambda p: p["searches"][0].update(link="https://drive.google.com/drive/search?q=is%3Atrashed")),
                         ("a page link to another site", lambda p: p["pages"][0].update(link="https://evil.example/drive/quota"))):
        edited = copy.deepcopy(plan)
        change(edited)
        edited["report_id"] = b.plan_id(edited)
        code, out, said = approve(save(edited, edited_path), "DRIVE-APPROVE report=%s unshare=1" % edited["report_id"])
        check("refused: a plan holding %s, even with a matching id" % name, code != 0 and "isn't in a safe form" in said and not out.strip(), said[:300])
    code, out, said = approve(clean_src, ok_line)
    check("refused: a file that isn't a plan", code != 0 and "isn't a drive-audit plan" in said, said[:300])

    # ---------------------------------------------------------------- 5. the copy that stays
    print("\n-- 5. the copy of a duplicate that stays is offered for the Trash nowhere, and no approval sends it there")
    for tag in ("hostile", "clean"):
        made = load(os.path.join(OUT, tag + "-plan.json"))
        check("%s report: no kept copy is in a Trash list, and no file is in two (%d kept, %d offered)" % (tag, len(staying(made)), len(going(made))),
              staying(made) and not staying(made) & set(going(made)) and len(going(made)) == len(set(going(made))))
        check("%s report: all its Trash fixes can go together" % tag, b.trash_clashes([("%s=%d" % (w, x["n"]), x) for w in TRASH for x in made["lists"][w]]) == [])
    scan = [x["name"] for w in TRASH for x in plan["lists"][w] if x["name"].startswith("Saltmarsh survey scan")]
    check("of two copies untouched for years, one is the duplicate and the other is in no list", scan == ["Saltmarsh survey scan (1).pdf"], scan)

    # two copies, both last changed on 1 January 2024: stale, and each other's duplicate
    pair = [twin("Lanternfall lease.pdf", "lease", "2024-01-01"), twin("Lanternfall lease (1).pdf", "lease", "2024-01-01")]
    plan2, plan2_path, data2 = small("pair", pair)
    dup = plan2["lists"]["duplicates"]
    check("two old copies: one row, in duplicates, naming the other as the copy that stays",
          [len(plan2["lists"][w]) for w in WORDS] == [0, 0, 1, 0, 0] and {dup[0]["file"], dup[0]["keep_file"]} == {f["id"] for f in pair}, plan2["lists"])
    check("the page still counts both as untouched for over a year", data2["totals"]["stale"] == 2 and data2["totals"]["dup_groups"] == 1, data2["totals"])
    code, out, said = approve(plan2_path, line_for(plan2))
    check("everything ticked: one file goes, and the check names the copy that stays", code == 0 and "1 approved" in out and "file id: %s" % dup[0]["file"] in out
          and "the copy that stays: %s" % dup[0]["keep_file"] in out and out.count("file id: ") == 1, said[:400])

    # what an older copy of the skill wrote for that Drive: the kept copy is in the stale list too
    kept = next(f for f in pair if f["id"] == dup[0]["keep_file"])
    alone, _, _ = small("pair-kept-alone", [kept])
    check("a file with no twin that is untouched for years is offered as stale", [x["file"] for x in alone["lists"]["stale"]] == [kept["id"]], alone["lists"])
    old = as_older_reports_did(plan2, alone["lists"]["stale"][0])
    old_path = save(old, os.path.join(OUT, "older-plan.json"))
    check("the older plan is in a safe form and has the id of its contents (only the clash is wrong with it)", b.check_plan(old) == [] and old["report_id"] != plan2["report_id"])
    clash = b.trash_clashes([("%s=%d" % (w, x["n"]), x) for w in TRASH for x in old["lists"][w]])
    check("its two Trash fixes are found to undo each other", clash == ["stale=1 would move to the Trash the copy that duplicates=1 keeps"], clash)
    for line in ("duplicates=1 stale=1", "stale=1 duplicates=1", "unshare=none outside=none duplicates=1 empty=none stale=1"):
        code, out, said = approve(old_path, "DRIVE-APPROVE report=%s %s" % (old["report_id"], line))
        check("older plan, both copies ticked (%s): refused, with which two and what to ask" % line, code != 0 and not out.strip()
              and "REFUSED: stale=1 would move to the Trash the copy that duplicates=1 keeps." in said and "which of the two to untick" in said, said[:300])
    for line, gone in (("duplicates=1", dup[0]["file"]), ("stale=1", dup[0]["keep_file"])):
        code, out, said = approve(old_path, "DRIVE-APPROVE report=%s %s" % (old["report_id"], line))
        check("older plan, only %s ticked: accepted, one file goes and its twin is left" % line, code == 0 and "1 approved" in out and out.count("file id: ") == 1
              and "file id: %s" % gone in out, said[:300])

    # three copies; other people use one of them, so that one stays
    trio = [twin("Copperline quote.pdf", "quote", "2023-05-02"), twin("Copperline quote (1).pdf", "quote", "2023-06-01"),
            twin("Copy of Copperline quote.pdf", "quote", "2022-11-20", sharing={"domains": {"lanternfall.example": 2}})]
    plan3, plan3_path, _ = small("trio", trio)
    dup = plan3["lists"]["duplicates"]
    check("three old copies: two rows, both keeping the shared one, which is in no list",
          [len(plan3["lists"][w]) for w in WORDS] == [0, 0, 2, 0, 0] and staying(plan3) == {trio[2]["id"]} and trio[2]["id"] not in going(plan3), plan3["lists"])
    code, out, said = approve(plan3_path, line_for(plan3))
    check("everything ticked: two go and the shared copy stays", code == 0 and "2 approved" in out and out.count("the copy that stays: %s" % trio[2]["id"]) == 2, said[:400])
    alone, _, _ = small("trio-kept-alone", [trio[2]])
    old = as_older_reports_did(plan3, alone["lists"]["stale"][0])
    old_path = save(old, os.path.join(OUT, "older-plan.json"))
    code, out, said = approve(old_path, "DRIVE-APPROVE report=%s duplicates=1 stale=1" % old["report_id"])
    check("older plan, three copies: one duplicate and the kept copy ticked is refused, even though a third copy would be left",
          code != 0 and not out.strip() and "the copy that duplicates=1 keeps" in said, said[:300])
    code, out, said = approve(old_path, "DRIVE-APPROVE report=%s duplicates=1-2 stale=1" % old["report_id"])
    check("older plan, three copies: all three ticked is refused", code != 0 and not out.strip() and "REFUSED" in said, said[:300])
    code, out, said = approve(old_path, "DRIVE-APPROVE report=%s duplicates=1-2" % old["report_id"])
    check("older plan, three copies: the two duplicates alone are accepted", code == 0 and "2 approved" in out, said[:300])

    # a kept copy that is also untitled and marked empty; and one matched by name and size only
    blank = [twin("Untitled document", "blank", "2026-09-01", empty=True), twin("Untitled document", "blank", "2026-08-01", empty=True)]
    plan4, _, _ = small("blank", blank)
    check("two untitled, empty copies: one is the duplicate, and the one that stays is not in the untitled-and-empty list",
          [len(plan4["lists"][w]) for w in WORDS] == [0, 0, 1, 0, 0] and not staying(plan4) & set(going(plan4)), plan4["lists"])
    same_name = [row("Kestrel handbook.docx", "docx", 123456, modified=700, id=file_id("handbook a")), row("Kestrel handbook.docx", "docx", 123456, modified=900, id=file_id("handbook b"))]
    plan5, _, _ = small("same-name", same_name)
    check("two old files with the same name and size (no checksum): one row, and the copy that stays is in no list",
          [len(plan5["lists"][w]) for w in WORDS] == [0, 0, 1, 0, 0] and plan5["lists"]["duplicates"][0]["keep_file"] == same_name[0]["id"], plan5["lists"])
    # a twin that can't be offered a fix promises nothing, so the other file is still offered on its own
    odd = [twin("Osprey brief.pdf", "brief", "2024-03-01"), dict(twin("Osprey brief (1).pdf", "brief", "2024-02-01"), id="abc")]
    plan6, _, _ = small("odd-twin", odd)
    check("a stale file whose only twin has no usable id: no duplicate row, so it is offered as stale", [len(plan6["lists"][w]) for w in WORDS] == [0, 0, 0, 0, 1]
          and plan6["lists"]["stale"][0]["file"] == odd[0]["id"], plan6["lists"])

    # the same file in two lists, as only a plan edited by hand could have it
    twice = copy.deepcopy(plan2)
    twice["lists"]["empty"] = [dict(alone_row, n=1) for alone_row in small("pair-kept-alone-again", [kept])[0]["lists"]["stale"]]
    twice["lists"]["stale"] = copy.deepcopy(twice["lists"]["empty"])
    twice["lists"]["duplicates"] = []
    twice["report_id"] = b.plan_id(twice)
    code, out, said = approve(save(twice, edited_path), "DRIVE-APPROVE report=%s empty=1 stale=1" % twice["report_id"])
    check("refused: one file ticked in two lists", code != 0 and not out.strip() and "empty=1 and stale=1 are the same file" in said, said[:300])

    # ---------------------------------------------------------------- 6. the builder's last check
    print("\n-- 6. the builder refuses to write a report that isn't safe to act on")
    target, target_plan = os.path.join(OUT, "refused-report.html"), os.path.join(OUT, "refused-plan.json")

    def refused_with(name, value):
        """Run the builder with one of its parts swapped, as if that part had a bug. Gives back what it said."""
        real = getattr(b, name)
        setattr(b, name, value)
        try:
            b.main([clean_src, "--out", target, "--plan", target_plan])
            return ""
        except SystemExit as e:
            return str(e)
        finally:
            setattr(b, name, real)

    said = refused_with("file_link", lambda fid: "https://drive.google.com.evil.example/open?id=%s" % fid)
    check("a link that isn't in a safe form: nothing is written", "refusing to write the report" in said and not os.path.exists(target) and not os.path.exists(target_plan), said[:300])
    said = refused_with("trash_clashes", lambda found: ["stale=1 would move to the Trash the copy that duplicates=1 keeps"])
    check("two Trash fixes that undo each other: nothing is written", "refusing to write the report: stale=1 would move" in said
          and not os.path.exists(target) and not os.path.exists(target_plan), said[:300])

    # ---------------------------------------------------------------- 7. the other shapes of drive.json
    print("\n-- 7. a personal account, a very large Drive, an empty file")
    r, ppage, pplan = build(os.path.join(IN, "drive-personal.json"), "personal")
    pdata = page_data(ppage)[1] if r.returncode == 0 else {}
    check("a personal account builds: no organization, every share is listed, no 'sharedwith:external' search", r.returncode == 0 and pdata["domain"] == ""
          and len(pdata["lists"]["outside"]) > count["outside"] and "owner:me sharedwith:external" not in [s["query"] for s in pdata["searches"]], r.stderr[:300])
    r, cpage, _ = build(os.path.join(IN, "drive-counted.json"), "counted")
    cdata = page_data(cpage)[1] if r.returncode == 0 else {}
    check("the counted form builds, with the agent's totals in the charts", r.returncode == 0 and cdata["complete"] is False and cdata["totals"]["files"] == 48210
          and cdata["by_type"][0][0] == "Video" and cdata["totals"]["trash"] == 912, r.stderr[:300])
    r, _, _ = build(save({"files": []}, os.path.join(OUT, "nothing.json")), "nothing")
    check("a drive.json with no files is refused with a pointer to the format", r.returncode != 0 and "gathering.md" in r.stderr, r.stderr[:300])

    # ---------------------------------------------------------------- 8. the page in a browser
    print("\n-- 8. the page in a headless browser")
    if opts.no_browser:
        note("the page was not opened in a browser (--no-browser)")
    else:
        try:
            browser = Pages(T)
        except NoBrowser as e:
            browser = None
            note("no browser to open the page in: %s" % e)
        if browser:
            shots = os.path.join(OUT, "screenshots") if opts.keep else None
            if shots:
                os.makedirs(shots)
            try:
                hostile_page = os.path.join(OUT, "hostile-report.html")
                page_checks(browser, hostile_page, load(plan_path_of[hostile_page]), shots)
            finally:
                browser.close()

    after = hashes(SKILL)
    check("nothing in the skill folder changed and no __pycache__ appeared", after == before_skill and not any("__pycache__" in k for k in after),
          sorted(set(after) ^ set(before_skill))[:5])
    check("the listings were only read", hashes(IN) == before_in)
finally:
    if not opts.keep:
        remove_work_folder(T, MARK)
    else:
        print("\nkept for a look: %s" % T)
c.finish("drive-audit")

"""Checks the inbox-cleanup skill end to end on a made-up mailbox.

    python -I test_inbox_cleanup.py <scratch folder> [<skill folder>] [--keep] [--no-browser]

It writes three made-up mailbox.json files (see _gen_inbox.py) into a work folder inside
<scratch folder> and builds the report from each. It checks that hostile names, subjects and
addresses stay text and never reach a Gmail search; that no message body and no full account
address is kept; the numbers the dashboard draws (mail by kind per day and week, weekday by
hour in the person's own time, unread by age, opened by kind, the people written back to);
that the report id is a fingerprint of the plan, so a plan changed after approval no longer
matches; the approval line's form; and that the builder refuses to write a report with an
unsafe search. Then the page in a headless browser at 390 and 1280 px, light and dark:
every chart drew, every link is a Gmail search, every list has Tick all and Untick all, and
Copy approval gives exactly the line for what was ticked. No mailbox is read or changed:
the scripts only read JSON, and nothing outside the work folder is written.

--keep leaves the work folder (mailboxes, reports, plans, screenshots) for a look.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import copy
import hashlib
import json
import os
import re
import sys
from datetime import date, datetime
from urllib.parse import unquote

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, Checks, hashes, ranges, remove_work_folder, run, work_folder  # noqa: E402
from _gen_inbox import (HOSTILE_NAME, ORDER_SUBJECT, SCRIPT_SUBJECT, SECRET_BODY, SENDERS, SHELL_SUBJECT,  # noqa: E402
                        box, write_mailbox)
from _report_page import SIZES, NoBrowser, Pages, basics  # noqa: E402

MARK = "made-by-test_inbox_cleanup.txt"
WORDS = ("unsubscribe", "filter", "spam", "steps")
NUMS = r"none|\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*"
LINE = re.compile(r"INBOX-APPROVE report=([0-9a-f]{10}) unsubscribe=(%s) filter=(%s) spam=(%s) steps=(%s)" % ((NUMS,) * 4))
LINK = re.compile(r"https://mail\.google\.com/mail/u/0/#search/[A-Za-z0-9%._\-()~!*']+")
HOSTILE_ADDRESSES = ("-everyone@evil.example", "a..b@odd.example", "$(calc)@evil.example")

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "inbox-cleanup"))
ap.add_argument("--keep", action="store_true")
ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser check of the page")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
sys.stdout.reconfigure(encoding="ascii", errors="backslashreplace")  # check names hold made-up hostile text: print it spelled out
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
BUILD = os.path.join(S, "build_report.py")
if not os.path.isfile(BUILD):
    sys.exit("Not found: %s" % BUILD)
c = Checks()
check, note = c.check, c.note
T = work_folder(opts.scratch, "inbox-cleanup-test", MARK)
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
    """Build a report from one mailbox.json. Gives back (what the script said, the page, the plan file)."""
    page, plan = os.path.join(OUT, tag + "-report.html"), os.path.join(OUT, tag + "-plan.json")
    return py(BUILD, src, "--out", page, "--plan", plan), page, plan


def page_data(path):
    with open(path, encoding="utf-8") as f:
        html = f.read()
    m = re.search(r'<script id="inbox-data" type="application/json">(.*?)</script>', html, re.S)
    return html, m.group(1), json.loads(m.group(1))


def fingerprint(plan):
    """How a plan's id is made: a hash of its numbered lists and its steps' searches."""
    core = json.dumps([plan["lists"], [s["queries"] for s in plan["steps"]]], sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(core.encode("utf-8")).hexdigest()[:10]


def line_for(plan, picks=None):
    """The approval line the page writes: `picks` is {word: [numbers]}; with none given, everything is ticked."""
    lists = {"unsubscribe": plan["lists"]["unsubscribe"], "filter": plan["lists"]["filter"], "spam": plan["lists"]["suspicious"], "steps": plan["steps"]}
    picks = {w: [x["n"] for x in lists[w]] for w in WORDS} if picks is None else picks
    return "INBOX-APPROVE report=%s" % plan["report_id"] + "".join(" %s=%s" % (w, ranges(picks.get(w, []))) for w in WORDS)


def numbers(text):
    out = []
    for part in ([] if text == "none" else text.split(",")):
        a, _, z = part.partition("-")
        out += list(range(int(a), int(z or a) + 1))
    return out


def read_line(line, plan):
    """What SKILL.md asks of the agent before acting: the line in exactly the page's form, this plan's id,
    and only numbers the plan has. Gives back {word: [numbers]}, or a reason it is refused."""
    m = LINE.fullmatch(line.split("\n")[0])
    if not m:
        return "not in the page's form"
    if m.group(1) != plan["report_id"] or fingerprint(plan) != plan["report_id"]:
        return "another report's id, or a plan changed since"
    have = {"unsubscribe": plan["lists"]["unsubscribe"], "filter": plan["lists"]["filter"], "spam": plan["lists"]["suspicious"], "steps": plan["steps"]}
    out = {}
    for i, w in enumerate(WORDS):
        got = numbers(m.group(i + 2))
        if not set(got) <= {x["n"] for x in have[w]}:
            return "a number that isn't in the plan"
        out[w] = got
    return out


def all_queries(plan, data=None):
    qs = [q for s in plan["steps"] for q in s["queries"]] + [v["query"] for vs in plan["lists"].values() for v in vs if v["query"]]
    if data:
        qs += [v["query"] for k in ("top_senders", "people", "newsletters", "never_opened") for v in data[k] if v["query"]]
    return qs


def page_checks(p, page, plan, data, shots):
    """The report in a headless browser: every size, light and dark, then ticking and the approval line."""
    n_kinds = len(data["kinds"])
    plan_rows = sum(len(plan["lists"][k]) for k in ("unsubscribe", "filter", "suspicious")) + len(plan["steps"])
    for width, height, scheme in SIZES:
        tag = "%d-%s" % (width, scheme)
        p.open(page, width, height, scheme)
        p.js("try { localStorage.clear() } catch (e) {}")
        p.open(page, width, height, scheme)
        basics(check, p, tag, scheme)
        n = p.js("""({cols: document.querySelectorAll('#volume .col').length, heat: document.querySelectorAll('#heat .heat i').length,
            waffle: document.querySelectorAll('#waffle .waffle i').length, opened: document.querySelectorAll('#opened .row').length,
            top: document.querySelectorAll('#top .row').length, people: document.querySelectorAll('#people .row').length,
            backlog: document.querySelectorAll('#backlog .col').length, facts: document.querySelectorAll('#facts .fact').length,
            take: document.querySelectorAll('#take .row').length, tables: document.querySelectorAll('.tbl').length,
            nl: document.querySelectorAll('#nl tbody tr, #nl .row').length, items: document.querySelectorAll('ul.items li.item').length,
            boxes: document.querySelectorAll('ul.items input[type=checkbox]').length, unit: (document.querySelector('#units [aria-pressed=true]') || {}).dataset,
            scripts: document.scripts.length, xss: typeof window.__xss,
            bad: document.querySelectorAll('img, iframe, object, embed, link, base, form, meta[http-equiv], main script, [srcdoc]').length,
            handlers: Array.from(document.querySelectorAll('*')).filter(e => Array.from(e.attributes).some(a => /^on/i.test(a.name))).length,
            tiny: Array.from(document.querySelectorAll('body *')).filter(e => e.children.length === 0 && e.textContent.trim()
                && e.getClientRects().length && parseFloat(getComputedStyle(e).fontSize) < 11).length,
            nav: Array.from(document.querySelectorAll('#secnav [data-target]')).map(b => b.dataset.target),
            links: Array.from(document.querySelectorAll('a[href]')).map(a => [a.getAttribute('href'), a.rel, a.target])})""")
        wide = width >= 720
        check(tag + ": every chart drew (%s by %s)" % (n["cols"], "day" if wide else "week"),
              n["cols"] == (60 if wide else 9) and n["heat"] == 7 * 24 and n["waffle"] == 100 and n["opened"] == n_kinds
              and n["top"] == len(data["top_senders"]) and n["people"] == len(data["people"]) and n["backlog"] == 9
              and n["facts"] == 5 and n["take"] == len(data["insights"]), n)
        check(tag + ": tables on a wide screen, ranked rows on a phone (%d newsletters)" % n["nl"],
              n["nl"] == len(data["newsletters"]) and (n["tables"] == 4 if wide else n["tables"] == 0), n)
        check(tag + ": every plan row drew with its tick (%d)" % plan_rows, n["items"] == plan_rows == n["boxes"], n)
        check(tag + ": the section bar names only sections that are there", n["nav"] == ["glance", "time", "mix", "senders", "newsletters", "unread", "look", "plan"], n["nav"])
        check(tag + ": hostile names made no elements and ran nothing", n["scripts"] == 2 and n["bad"] == 0 and n["handlers"] == 0 and n["xss"] == "undefined", n)
        check(tag + ": no text under 11 px", n["tiny"] == 0, n["tiny"])
        bad_links = [h for h, rel, tgt in n["links"] if not (LINK.fullmatch(h) and "noopener" in rel and "noreferrer" in rel and tgt == "_blank")]
        check(tag + ": every link (%d) is a Gmail search with rel=noopener noreferrer" % len(n["links"]), n["links"] and not bad_links, bad_links[:3])
        searched = [unquote(h.split("#search/", 1)[1]) for h, _, _ in n["links"]]
        check(tag + ": every linked search is one the builder made and checked", all(q in b.FIXED_QUERIES or b.SAFE_QUERY.fullmatch(q) for q in searched),
              [q for q in searched if q not in b.FIXED_QUERIES and not b.SAFE_QUERY.fullmatch(q)][:3])
        text = p.js("document.body.textContent")
        for needle in (HOSTILE_NAME, SCRIPT_SUBJECT, ORDER_SUBJECT, SHELL_SUBJECT, "<b>Bold</b> & \"quoted\"", "scan <b>bold</b> & co.pdf"):
            check(tag + ": shown as text: " + needle[:32], needle in text)
        check(tag + ": no message body and no full account address", SECRET_BODY not in text and "tamsin@" not in text)
        if shots:
            p.shot(os.path.join(shots, "inbox-%s.png" % tag))

    # ---- ticking and the approval line (phone width, light)
    p.open(page, 390, 844, "light")
    p.js("try { localStorage.clear() } catch (e) {}")
    p.open(page, 390, 844, "light")
    first = p.copy()
    default = line_for(plan, {"unsubscribe": [x["n"] for x in plan["lists"]["unsubscribe"]], "filter": [x["n"] for x in plan["lists"]["filter"]],
                              "spam": [x["n"] for x in plan["lists"]["suspicious"]]})
    check("first load ticks the senders, not the bulk steps", first == default, (first, default))
    check("that line is one the agent may act on", isinstance(read_line(first, plan), dict), read_line(first, plan))
    check("#copy and #fallback are there", p.js("!!document.getElementById('copy') && document.getElementById('fallback').tagName === 'TEXTAREA'"))
    missing = p.js("""Array.from(document.querySelectorAll('ul.items')).filter(u => { const t = Array.from(u.parentElement.querySelectorAll('button'))
        .map(x => x.textContent); return !(t.includes('Tick all') && t.includes('Untick all')); }).length""")
    check("every tickable list has Tick all and Untick all beside it", missing == 0 and p.js("document.querySelectorAll('ul.items').length") == 4, missing)
    p.js("document.getElementById('untick-all').click()")
    check("the top Untick all clears every list", p.copy() == line_for(plan, {}) and p.js("document.querySelectorAll('ul.items input:checked').length") == 0, p.copy())
    p.js("document.getElementById('tick-all').click()")
    everything = p.copy()
    check("the top Tick all ticks every row, and the line names each once", everything == line_for(plan), everything)
    check("with everything ticked the line is still one the agent may act on", isinstance(read_line(everything, plan), dict), read_line(everything, plan))
    for i, w in enumerate(WORDS):
        p.js("""(() => { const u = document.querySelectorAll('ul.items')[%d]; Array.from(u.parentElement.querySelectorAll('button'))
            .filter(x => x.textContent === 'Untick all')[0].click(); })()""" % i)
        got = p.copy()
        check("Untick all for '%s' clears only that list" % w, (" %s=none" % w) in got and got.count("=none") == 1, got)
        p.js("""(() => { const u = document.querySelectorAll('ul.items')[%d]; Array.from(u.parentElement.querySelectorAll('button'))
            .filter(x => x.textContent === 'Tick all')[0].click(); })()""" % i)
        check("Tick all for '%s' brings it back" % w, p.copy() == everything, p.copy())
    p.js("document.getElementById('untick-all').click()")
    p.js("""(() => { const lists = Array.from(document.querySelectorAll('ul.items')).map(u => u.querySelectorAll('input'));
        lists[0][0].click(); lists[0][2].click(); lists[0][3].click(); lists[2][1].click(); lists[3][2].click(); })()""")
    one = p.copy()
    check("the line names exactly the boxes clicked", one == line_for(plan, {"unsubscribe": [1, 3, 4], "spam": [2], "steps": [3]}), one)
    p.js("document.getElementById('toggle-notes').click(); document.getElementById('notes').value = 'keep  number 2\\n it is <b>mine</b>'; 0")
    noted = p.copy()
    check("a note goes on a second line, on one line", noted == one + "\nnote: keep number 2 it is <b>mine</b>", noted)
    check("a note never changes what is approved", read_line(noted, plan) == {"unsubscribe": [1, 3, 4], "filter": [], "spam": [2], "steps": [3]}, read_line(noted, plan))
    p.js("document.getElementById('notes').value = ''; 0")
    fb = p.js("""(async () => { navigator.clipboard.writeText = () => Promise.reject(new Error('no')); document.getElementById('copy').click();
        await new Promise(r => setTimeout(r, 300));
        return [document.getElementById('fallback-wrap').classList.contains('hidden'), document.getElementById('fallback').value]; })()""")
    check("when the clipboard is refused, the approval appears in a box to copy by hand", fb[0] is False and fb[1] == one, fb)
    p.open(page, 390, 844, "light")
    check("ticks are remembered after a reload", p.copy() == one, p.copy())
    stored = p.js("Object.keys(localStorage)")
    check("the browser's storage holds only this report's ticks", stored == ["inbox-report:" + plan["report_id"]], stored)
    # finding a sender
    p.open(page, 1280, 900, "light")
    found = p.js("""(async () => { const q = document.getElementById('q'); q.value = 'copperline'; q.dispatchEvent(new Event('input'));
        await new Promise(r => setTimeout(r, 400));
        const shown = Array.from(document.querySelectorAll('[data-q]')).filter(e => e.getClientRects().length);
        const out = [shown.length, shown.every(e => e.getAttribute('data-q').includes('copperline')), document.querySelectorAll('.nomatch').length];
        q.value = ''; q.dispatchEvent(new Event('input')); await new Promise(r => setTimeout(r, 400));
        out.push(document.querySelectorAll('.nomatch, .match').length); return out; })()""")
    check("search keeps only the rows that match, and clearing it brings the rest back", found[0] > 3 and found[1] and found[2] > 0 and found[3] == 0, found)
    sorted_ok = p.js("""(() => { const th = Array.from(document.querySelectorAll('#nl th button')).find(b => b.textContent.startsWith('Opened'));
        th.click(); const v = Array.from(document.querySelectorAll('#nl tbody tr:not(.hidden) td:nth-child(3)')).map(td => parseInt(td.textContent, 10));
        return v.length > 3 && v.every((x, i) => i === 0 || v[i - 1] >= x); })()""")
    check("the newsletters table sorts by a column", sorted_ok)
    theme = p.js("""(async () => { const before = document.documentElement.getAttribute('data-theme'); document.getElementById('theme').click();
        await new Promise(r => setTimeout(r, 600)); return [before, document.documentElement.getAttribute('data-theme')]; })()""")
    check("the theme switch works", theme[1] == "dark", theme)
    p.js("try { localStorage.clear() } catch (e) {}")
    pr = p.problems()
    check("no script errors and no alert() while clicking", not pr["exceptions"] and not pr["console_errors"] and not pr["dialogs"], pr)


try:
    IN = os.path.join(T, "mailboxes")
    os.makedirs(OUT)
    check("fixtures written", write_mailbox(IN) == ["mailbox-counted.json", "mailbox-plain.json", "mailbox.json"], os.listdir(IN))
    before_in, before_skill = hashes(IN), hashes(SKILL)
    sys.path.insert(0, S)
    import build_report as b  # noqa: E402  (the skill's own)

    # ---------------------------------------------------------------- 1. hostile data
    print("\n-- 1. hostile names, subjects and addresses stay text and out of every search")
    r, page, plan_path = build(os.path.join(IN, "mailbox.json"), "hostile")
    check("the hostile mailbox builds", r.returncode == 0, r.stderr[:400])
    plan = load(plan_path)
    html, blob, R = page_data(page)
    check("the page's data has no raw < > & or line separators", not re.search("[<>&  ]", blob))
    check("the page has exactly two </script> tags", html.count("</script>") == 2)
    code = html[html.index("</script>") + 9:]  # the page's own code, after the data
    for word in ("innerHTML", "outerHTML", "insertAdjacentHTML", "document.write", "eval(", "new Function", "srcdoc"):
        check("the page's code never uses %s" % word, word not in code)
    rest = html.replace(blob, "")
    check("the page loads nothing: no outside scripts, styles, images, frames or requests", not re.search(
        r"<link|<img|<iframe|\ssrc\s*=|\.src\s*=|@import|fetch\(|XMLHttpRequest|WebSocket|sendBeacon|EventSource|importScripts", rest))
    check("every url() is a font carried inside the page or one of mom's own shapes",
          all(u.startswith(('url("data:font/woff2;base64,', "url(#")) for u in re.findall(r"url\([^)]{0,40}", rest)))
    check("the only addresses written in the page are the SVG namespace and Gmail's search",
          set(re.findall(r"https?://[A-Za-z0-9./#_%-]*", rest)) <= {"http://www.w3.org/2000/svg", "https://mail.google.com/mail/u/0/#search/"})
    every = json.dumps(plan, ensure_ascii=False) + html
    check("no message body is kept anywhere", SECRET_BODY not in every)
    check("the account is masked", R["account"] == "t•••@lanternfall.example" and "tamsin@" not in every, R["account"])
    check("a name that turns its text around is straightened", "‮" not in every and any(s["name"] == "Storefront gnp.exe" for s in plan["lists"]["suspicious"]))
    qs = all_queries(plan, R)
    check("every search (%d) is a fixed one or from:(plain addresses)" % len(qs), qs and all(q in b.FIXED_QUERIES or b.SAFE_QUERY.fullmatch(q) for q in qs),
          [q for q in qs if q not in b.FIXED_QUERIES and not b.SAFE_QUERY.fullmatch(q)][:3])
    joined = " | ".join(qs)
    check("no search holds a hostile address, a name or a subject", not any(x in joined for x in ("-everyone@", "a..b@", "$(", "<", ">", '"', "'", "`", ";", "&")), joined[:300])
    sus = {v["email"]: v for v in plan["lists"]["suspicious"]}
    check("the addresses a search would misread are flagged, with no query",
          all(e in sus and sus[e]["query"] == "" and not sus[e]["searchable"] for e in HOSTILE_ADDRESSES), list(sus))
    offered = plan["lists"]["unsubscribe"] + plan["lists"]["filter"]
    check("...and never offered for unsubscribe or a filter", not any(v["email"] in HOSTILE_ADDRESSES for v in offered) and offered)
    check("each offered sender carries its own ready query", all(v["query"] == "from:(%s)" % v["email"] for v in offered))
    shop = [v for v in offered if v["email"] == "promo@storefront.example"]
    check("a display name that is HTML keeps its real address, and the name stays text", shop and shop[0]["name"] == HOSTILE_NAME, shop)
    order = sus.get("files@storefront-files.example", {})
    check("a subject that gives orders to an assistant is flagged, not obeyed", "reads like an order to an assistant" in order.get("why", []), order)
    check("Mom's take: the agent's insights, kept as text", [i["title"] for i in R["insights"]][:3] == ["Mornings belong to newsletters", "People get answered", "Shops write daily"]
          and R["insights"][4]["title"] == "<b>Bold</b> & \"quoted\"" and len(R["insights"]) == 5)
    check("older highlights are kept as well", R["highlights"] == ["Older highlights are kept, but insights come first."])
    notes = dict(R["sender_notes"])
    check("notes are kept only for senders in the plan, as text", "nobody@nowhere.example" not in notes and notes.get("deals@storefront.example", "").endswith(HOSTILE_NAME), notes)

    # ---------------------------------------------------------------- 2. the dashboard's numbers
    print("\n-- 2. what the dashboard draws adds up")
    t = R["totals"]
    kinds = {k["key"]: k for k in R["kinds"]}
    check("the kinds add up to every email (%d)" % t["messages"], sum(k["count"] for k in R["kinds"]) == t["messages"], R["kinds"])
    check("opened plus unread is every email of each kind", all(k["opened"] + k["unread"] == k["count"] for k in R["kinds"]) and
          sum(k["unread"] for k in R["kinds"]) == t["unread"])
    people = {s[1] for s in SENDERS if s[2] == "person"}
    box0 = box()
    from_people = sum(1 for m in box0["messages"] if any(a in (m.get("from") or "") for a in people))
    check("mail from people is the 4 people's mail and nothing else (%d)" % from_people, kinds["people"]["count"] == from_people, kinds["people"])
    check("phishing is counted as spam, never as people", kinds["spam"]["senders"] >= 4 and kinds["spam"]["count"] >= 20, kinds.get("spam"))
    days = R["volume"]["days"]
    weeks = R["volume"]["weeks"]
    dsum = sum(sum(v) for v in days["series"].values())
    check("mail per day: 60 days, every email once", len(days["labels"]) == 60 and dsum == t["messages"] and days["labels"][0] == "Sat 15 Aug", (len(days["labels"]), dsum))
    check("mail per week: 9 weeks counted back from the last day, the first one 4 days, the same emails",
          len(weeks["labels"]) == 9 and weeks["first_days"] == 4 and weeks["labels"][-1] == "7 Oct" and sum(sum(v) for v in weeks["series"].values()) == dsum, weeks["labels"])
    heat = R["heat"]
    check("weekday by hour: every email with a time (all but the one with only a day)", heat and heat["n"] == t["messages"] - 1 and
          len(heat["cells"]) == 7 and all(len(r) == 24 for r in heat["cells"]) and sum(map(sum, heat["cells"])) == heat["n"] and heat["tz"] == "UTC+03:00", heat and heat["n"])
    hours = [sum(row[h] for row in heat["cells"]) for h in range(24)]
    check("arrival hours are on the person's clock whatever offset the tool wrote: the busiest is 07:00 (mostly written as 04:00Z)",
          hours.index(max(hours)) == 7 and hours[7] > 2 * hours[4], hours)
    bl = R["backlog"]
    check("unread by age: 9 weeks, oldest first, every unread email", bl["labels"][0] == "8 weeks" and bl["labels"][-1] == "This week" and
          bl["n"] == t["unread"] and sum(sum(v) for v in bl["series"].values()) == t["unread"] and 0 < bl["older_than_2w"] < bl["n"], bl["labels"])
    names = [s["name"] for s in R["people"]]
    check("the people written back to come first, and no flagged sender is among them", names[:2] == ["Oskar Copperline", "Mira Lanternfall"] and
          set(names) == {"Oskar Copperline", "Mira Lanternfall", "Juno Atlas", "Pell Nimbus"} and R["people"][0]["replied"] > 0, names)
    check("the busiest senders are ranked, each with its kind and how much is opened", [s["name"] for s in R["top_senders"][:1]] == ["Storefront Deals"]
          and all(s["kind"] in b.KINDS and 0 <= s["opened"] <= 100 for s in R["top_senders"]))
    nl_names = {s["name"] for s in R["newsletters"]}
    phish = {s["email"] for s in plan["lists"]["suspicious"] if s["score"] >= 3}
    check("the newsletters table holds the newsletters and shops, and no sender flagged as phishing",
          {"Storefront Deals", "Copperline Digest", "Lanternfall Weekly", "Nimbus Notes", HOSTILE_NAME} <= nl_names
          and not any(s["email"] in phish for s in R["newsletters"]) and len(R["newsletters"]) <= t["newsletters"], sorted(nl_names))
    for when, tz, want in [("2026-10-01T05:12:00Z", 180, (date(2026, 10, 1), 8)), ("2026-10-01T08:12:00+03:00", 180, (date(2026, 10, 1), 8)),
                           ("2026-10-01T22:30:00-02:00", 180, (date(2026, 10, 2), 3)), ("Wed, 30 Sep 2026 23:30:00 -0100", 180, (date(2026, 10, 1), 3)),
                           (int((datetime(2026, 10, 1, 5, 0) - datetime(1970, 1, 1)).total_seconds() * 1000), 180, (date(2026, 10, 1), 8)),
                           ("2026-10-01T05:12:00Z", None, (date(2026, 10, 1), 5)), ("2026-10-01", 180, (date(2026, 10, 1), None)),
                           ("2026-10-01T25:00:00Z", 180, (date(2026, 10, 1), None)), ("not a date", 180, (None, None)), ("", None, (None, None)),
                           ({"x": 1}, 180, (None, None)), (True, 180, (None, None))]:
        check("arrival of %r at %r is %r" % (when, tz, want), b.when_of({"date": when}, tz) == want, b.when_of({"date": when}, tz))
    for text, want in [("+03:00", 180), ("-0530", -330), ("UTC+3", 180), ("Z", 0), ("+15:00", None), ("3", None), ("+03:00; drop", None)]:
        check("offset %r is %r" % (text, want), b.offset_minutes(text) == want, b.offset_minutes(text))
    for args, want in [(("spam", True, "a@x.example", set()), "spam"), (("promotions", False, "a@x.example", set()), "promotions"),
                       (("social", True, "a@x.example", set()), "notifications"), (("updates", True, "a@x.example", set()), "newsletters"),
                       (("primary", False, "a@x.example", {"a@x.example"}), "newsletters"), (("updates", False, "a@x.example", set()), "notifications"),
                       (("primary", False, "no-reply@x.example", set()), "notifications"), (("primary", False, "noreply.bank@x.example", set()), "notifications"),
                       (("primary", False, "nora@x.example", set()), "people"), (("primary", False, "info@x.example", set()), "notifications")]:
        check("kind of %r is %s" % (args[:3], want), b.kind_of(*args) == want, b.kind_of(*args))

    r2, page2, plan2_path = build(os.path.join(IN, "mailbox-plain.json"), "plain")
    _, _, R2 = page_data(page2) if r2.returncode == 0 else (None, None, {})
    check("an older mailbox.json (days without times, highlights, no offset) builds", r2.returncode == 0, r2.stderr[:300])
    check("...with no weekday-by-hour map, and its highlights as Mom's take", R2.get("heat") is None and
          [i["text"] for i in R2.get("insights", [])] == ["Storefront Deals writes every day.", "You open nearly all mail from people."])
    r3, page3, plan3_path = build(os.path.join(IN, "mailbox-counted.json"), "counted")
    _, _, R3 = page_data(page3) if r3.returncode == 0 else (None, None, {})
    counted = load(os.path.join(IN, "mailbox-counted.json"))
    check("a mailbox only counted per sender builds", r3.returncode == 0, r3.stderr[:300])
    check("...its mail over time is the daily counts in one series, with no hour map and no backlog",
          list(R3["volume"]["days"]["series"]) == ["other"] and sum(R3["volume"]["days"]["series"]["other"]) == sum(counted["daily"].values())
          and R3["heat"] is None and R3["backlog"] is None and R3["complete"] is False)
    check("...and its odd address is flagged, never offered", "-everyone@evil.example" in {v["email"] for v in load(plan3_path)["lists"]["suspicious"]}
          and "-everyone@evil.example" not in {v["email"] for v in load(plan3_path)["lists"]["unsubscribe"] + load(plan3_path)["lists"]["filter"]})

    # ---------------------------------------------------------------- 3. the report id and the approval
    print("\n-- 3. the report id is a fingerprint of the plan; the approval line fits only this plan")
    rid = plan["report_id"]
    check("the id in the plan file is the hash of its lists and searches (%s)" % rid, fingerprint(plan) == rid == R["report_id"] and re.fullmatch(r"[0-9a-f]{10}", rid))
    check("the same mailbox gives the same id twice", load(build(os.path.join(IN, "mailbox.json"), "again")[2])["report_id"] == rid)

    def variant(tag, change):
        v = box()
        change(v)
        r, _, pp = build(save(v, os.path.join(OUT, tag + ".json")), tag)
        return load(pp)["report_id"] if r.returncode == 0 else "failed: " + r.stderr[:200]

    check("other words from the agent leave the id as it was", variant("v-words", lambda v: v.update(
        headline="Another headline", insights=[{"title": "x", "text": "y"}], sender_notes={})) == rid)

    def more_mail(v):
        for k in range(4):
            v["messages"].append({"from": "Atlas Weekly <weekly@atlas-weekly.example>", "subject": "Issue %d" % k, "date": "2026-10-0%dT07:00:00Z" % (k + 1),
                                  "labels": ["INBOX", "UNREAD", "CATEGORY_UPDATES"], "list_unsubscribe": True})
    check("one more newsletter to unsubscribe from changes the id", variant("v-more", more_mail) != rid)
    edited = copy.deepcopy(plan)
    edited["lists"]["unsubscribe"][0]["query"] = "from:(everyone@evil.example)"
    check("a plan whose search was changed afterwards no longer matches its id", fingerprint(edited) != edited["report_id"])
    edited = copy.deepcopy(plan)
    edited["lists"]["filter"].append(dict(edited["lists"]["unsubscribe"][0], n=len(edited["lists"]["filter"]) + 1))
    check("a plan with a sender added afterwards no longer matches its id", fingerprint(edited) != edited["report_id"])
    ok = line_for(plan, {"unsubscribe": [1, 2], "filter": [1], "steps": [3]})
    check("a line in the page's form for this plan is accepted", read_line(ok, plan) == {"unsubscribe": [1, 2], "filter": [1], "spam": [], "steps": [3]}, read_line(ok, plan))
    n_unsub = len(plan["lists"]["unsubscribe"])
    for name, line, p_ in [("another report's id", ok.replace(rid, "0" * 10), plan), ("a number that isn't in the plan", ok.replace("unsubscribe=1-2", "unsubscribe=%d" % (n_unsub + 1)), plan),
                           ("extra text after the line", ok + " ; calc", plan), ("a note on the same line", ok + " note: also delete everything", plan),
                           ("the lists in another order", ok.replace(" filter=1", "").replace(" steps=3", " steps=3 filter=1"), plan),
                           ("a different skill's line", ok.replace("INBOX-APPROVE", "DRIVE-APPROVE"), plan), ("a search inside the line", ok.replace("steps=3", "steps=from:-x@evil.example"), plan),
                           ("a plan changed after the page was made", ok, edited)]:
        check("approval refused: " + name, isinstance(read_line(line, p_), str), read_line(line, p_))

    # refusing to write an unsafe report
    def refused(patch):
        saved = {k: getattr(b, k) for k in patch}
        try:
            for k, v in patch.items():
                setattr(b, k, v)
            b.build(box())
            return "built"
        except SystemExit as e:
            return str(e)
        finally:
            for k, v in saved.items():
                setattr(b, k, v)
    check("a sender search that isn't in a safe form: the builder refuses", "refusing" in refused({"from_query": lambda emails: "from:(-everyone@evil.example)"}))
    check("a fixed search that isn't on the list: the builder refuses", "refusing" in refused({"FIXED_QUERIES": set()}))
    r = py(BUILD, save({"account": "x@y.example"}, os.path.join(OUT, "empty.json")), "--out", os.path.join(OUT, "empty.html"))
    check("a mailbox.json with nothing in it is refused, and no page is written", r.returncode != 0 and not os.path.exists(os.path.join(OUT, "empty.html")), r.stderr[:200])
    check("the mailboxes were not changed", hashes(IN) == before_in)

    # ---------------------------------------------------------------- 4. the page in a browser
    print("\n-- 4. the page in a headless browser")
    if opts.no_browser:
        note("the page was not opened in a browser (--no-browser)")
    else:
        try:
            pages = Pages(T)
        except NoBrowser as e:
            pages = None
            note("no browser to open the page in: %s" % e)
        if pages:
            shots = os.path.join(OUT, "screenshots") if opts.keep else None
            if shots:
                os.makedirs(shots)
            try:
                page_checks(pages, page, plan, R, shots)
                for pg, tag in ((page2, "plain"), (page3, "counted")):
                    for width, height, scheme in ((1280, 900, "light"), (390, 844, "dark")):
                        pages.open(pg, width, height, scheme)
                        basics(check, pages, "%s %d-%s" % (tag, width, scheme), scheme)
                        seen = pages.js("""({heat: !document.getElementById('heat-block').classList.contains('hidden'),
                            unread: !document.getElementById('unread').classList.contains('hidden'),
                            nav: Array.from(document.querySelectorAll('#secnav [data-target]')).map(b => b.dataset.target),
                            cols: document.querySelectorAll('#volume .col').length})""")
                        want_unread = tag == "plain"
                        check("%s %d-%s: what there's no data for is left out, and the section bar with it" % (tag, width, scheme),
                              seen["heat"] is False and seen["unread"] is want_unread and ("unread" in seen["nav"]) is want_unread and seen["cols"] > 0, seen)
                        if shots:
                            pages.shot(os.path.join(shots, "inbox-%s-%d-%s.png" % (tag, width, scheme)))
            finally:
                pages.close()

    after = hashes(SKILL)
    check("nothing in the skill folder changed and no __pycache__ appeared", after == before_skill and not any("__pycache__" in k for k in after),
          sorted(set(after) ^ set(before_skill))[:5])
finally:
    if not opts.keep:
        remove_work_folder(T, MARK)
    else:
        print("\nkept for a look: %s" % T)
c.finish("inbox-cleanup")

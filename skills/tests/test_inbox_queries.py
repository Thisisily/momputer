"""Checks that inbox-cleanup never puts an address a mail search would misread into a query.

    python -I test_inbox_queries.py <scratch folder> [<inbox-cleanup skill folder>] [--keep]

Gmail reads `from:-x@evil.example` as "everything except". The report builder must flag such
senders, give them no query, and never offer them for unsubscribe or a filter.

Everything happens in a work folder this test makes inside <scratch folder> (which must already
exist). Nothing outside it is created or changed; no mailbox is touched. Python 3.8+, standard
library only.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
MARK = "made-by-test_inbox_queries.txt"
PY = sys.executable

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch", help="an existing folder; the test works in a subfolder it makes there")
ap.add_argument("skill", nargs="?", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "inbox-cleanup"))
ap.add_argument("--keep", action="store_true", help="leave the work folder in place for a look")
opts = ap.parse_args()
SCRATCH, SKILL = os.path.abspath(opts.scratch), os.path.abspath(opts.skill)
BUILD = os.path.join(SKILL, "scripts", "build_report.py")
if not os.path.isfile(BUILD):
    sys.exit("Not found: %s" % BUILD)
if not os.path.isdir(SCRATCH):
    sys.exit("The scratch folder must already exist: %s" % SCRATCH)
T = os.path.join(SCRATCH, "inbox-queries")
if os.path.lexists(T) and not os.path.isfile(os.path.join(T, MARK)):
    sys.exit("%s exists and this test didn't make it. Remove it yourself or give another scratch folder." % T)

fails, count = [], [0]


def check(name, ok, detail=""):
    count[0] += 1
    print(("ok:   " if ok else "FAIL: ") + name + ((" (" + str(detail)[:400] + ")") if detail and not ok else ""))
    if not ok:
        fails.append(name)


pycache_before = os.path.isdir(os.path.join(SKILL, "scripts", "__pycache__"))
try:
    if os.path.isdir(T):
        shutil.rmtree(T)
    os.makedirs(T)
    open(os.path.join(T, MARK), "w").write("Made by test_inbox_queries.py. Safe to delete.\n")
    box = {"account": "someone@example.com", "messages": []}
    senders = [("-everyone@evil.example", "Deals"), ("news@shop.example", "Shop"), ("a..b@odd.example", "Odd"),
               ("x@-dash.example", "Dash"), ("ok+tag@mail.example", "Tagged")]
    for email, name in senders:
        for k in range(8):
            box["messages"].append({"from_email": email, "from_name": name, "subject": "Sale %d" % k, "date": "2026-10-%02d" % (k + 1),
                                    "labels": ["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"], "list_unsubscribe": True, "size": 1000})
    mb = os.path.join(T, "mailbox.json")
    json.dump(box, open(mb, "w", encoding="utf-8"))
    out = os.path.join(T, "inbox-report.html")
    r = subprocess.run([PY, "-I", BUILD, mb, "--out", out], capture_output=True, text=True, encoding="utf-8", errors="replace")
    check("inbox report builds", r.returncode == 0, r.stderr or r.stdout)
    ip = json.load(open(os.path.join(T, "inbox-plan.json"), encoding="utf-8"))
    queries = [q for s in ip["steps"] for q in s["queries"]] + [v.get("query", "") for vs in ip["lists"].values() for v in vs]
    joined = " | ".join(queries)
    check("no query holds an address starting with '-', with '..' or a '-' label", all(b not in joined for b in ("-everyone@", "a..b@", "@-dash")), joined)
    check("plain addresses (and +tags) are still searchable", "news@shop.example" in joined and "ok+tag@mail.example" in joined, joined)
    sus = {v["email"]: v for v in ip["lists"]["suspicious"]}
    check("the odd addresses are flagged, with no query", all(e in sus and sus[e]["query"] == "" and not sus[e]["searchable"]
                                                              for e in ("-everyone@evil.example", "a..b@odd.example", "x@-dash.example")), list(sus))
    offered = ip["lists"]["unsubscribe"] + ip["lists"]["filter"]
    check("the odd addresses are never offered for unsubscribe or filter",
          not any(v["email"].startswith("-") or ".." in v["email"] or "@-" in v["email"] for v in offered), [v["email"] for v in offered])
    check("each offered sender carries its own ready query", all(v["query"] == "from:(%s)" % v["email"] for v in offered) and offered, offered[:2])
    check("no __pycache__ left in the skill folder", pycache_before or not os.path.isdir(os.path.join(SKILL, "scripts", "__pycache__")))
finally:
    if not opts.keep and os.path.isfile(os.path.join(T, MARK)):
        shutil.rmtree(T, ignore_errors=True)

print("\n%s: %d checks on %s" % ("ALL PASSED" if not fails else "%d FAILED: %s" % (len(fails), fails), count[0], os.path.basename(SKILL)))
sys.exit(1 if fails else 0)

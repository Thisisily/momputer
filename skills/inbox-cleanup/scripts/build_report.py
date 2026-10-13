#!/usr/bin/env python3
"""Turn mailbox.json (what the agent gathered, read-only) into the inbox report page.

    python build_report.py mailbox.json --out inbox-report.html

Writes the page and inbox-plan.json next to it (numbered senders and steps, so an
approval like "INBOX-APPROVE report=ab12cd34ef unsubscribe=1-6 filter=2 steps=3"
can be mapped back to exact senders and queries). The page holds sender names,
addresses, subject lines and counts only. Never message bodies.

The mailbox.json format is in references/gathering.md. Python 3.8+, standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import sys
from datetime import date, datetime, timedelta

HERE = os.path.dirname(os.path.abspath(__file__))
TEMPLATE = os.path.join(HERE, "..", "assets", "inbox-report-template.html")

# Thresholds (all documented in SKILL.md so the agent can explain them)
UNSUB_MAX_READ = 0.25      # a newsletter read a quarter of the time or less is an unsubscribe candidate
UNSUB_MIN_COUNT = 2        # one stray email isn't a pattern
FILTER_MAX_READ = 0.70     # read sometimes, but noisy: file it out of the inbox instead
FILTER_MIN_COUNT = 6
NEVER_OPENED_MIN = 3
BIG_BYTES = 5 * 1024 * 1024
SUBJECT_MAX = 90
QUERY_CHUNK = 15           # senders per from:(...) query, keeps Gmail filters well under their size limit

CATEGORIES = ["primary", "promotions", "social", "updates", "forums"]
FREEMAIL = {"gmail.com", "googlemail.com", "outlook.com", "hotmail.com", "live.com", "msn.com", "yahoo.com",
            "ymail.com", "icloud.com", "me.com", "aol.com", "proton.me", "protonmail.com", "gmx.com", "gmx.net",
            "mail.com", "yandex.com", "yandex.ru", "mail.ru", "zoho.com", "qq.com", "163.com"}
OFFICIAL_WORDS = re.compile(r"\b(support|security|account|billing|service|verification|admin|helpdesk|it team|"
                            r"payroll|hr|compliance|fraud|delivery|customs)\b", re.I)
URGENT_WORDS = re.compile(r"urgent|verify|suspend|locked|unusual (sign|activity)|password|confirm your|final notice|"
                          r"overdue|payment (failed|declined)|gift ?card|winner|prize|crypto|wallet|bitcoin|"
                          r"act now|within 24 hours|refund|invoice attached|reactivate", re.I)
ADDR_RX = re.compile(r"[\w.+'-]+@[\w-]+(\.[\w-]+)+")
# Sender addresses are chosen by whoever sent the mail, and they end up inside Gmail search
# queries. Only plain addresses are ever put into a query: letters, digits and . _ + - only,
# and every part starts with a letter or digit. No spaces, quotes or brackets, and above all
# no leading "-": Gmail reads "-word" as NOT, so "from:-x@evil.example" would match every
# email EXCEPT that sender's, and a filter built on it would sweep up the whole mailbox.
_LABEL = r"[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?"
SAFE_ADDR = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]*@" + _LABEL + r"(?:\." + _LABEL + r")+")
# What a finished query may look like: one of the fixed searches below, or from:(a OR b ...).
SAFE_QUERY = re.compile(r"from:\((?:%s)(?: OR (?:%s))*\)" % (SAFE_ADDR.pattern, SAFE_ADDR.pattern))
FIXED_QUERIES = {"category:promotions is:unread older_than:14d", "category:social older_than:30d",
                 "category:updates is:unread older_than:30d", "larger:10M", "has:attachment larger:5M older_than:1y",
                 "is:unread older_than:7d -in:chats", "in:inbox older_than:30d", "in:spam"}


def parse_from(m):
    name, email = m.get("from_name"), m.get("from_email")
    raw = m.get("from") or ""
    if not email:
        mm = re.search(r"<([^>]+)>", raw)
        email = mm.group(1) if mm else (ADDR_RX.search(raw).group(0) if ADDR_RX.search(raw) else raw)
        if name is None:
            name = raw.split("<")[0].strip().strip('"') if "<" in raw else ""
    email = clip((email or "").strip().lower(), 120)
    return clip((name or "").strip(), 80), email


def queryable(email):
    """True only for a plain address that Gmail's search can't read as anything but an address."""
    return isinstance(email, str) and len(email) <= 100 and ".." not in email and bool(SAFE_ADDR.fullmatch(email))


def domain_of(email):
    return email.rsplit("@", 1)[-1] if "@" in email else email


def category_of(m):
    c = (m.get("category") or "").lower()
    if c in CATEGORIES:
        return c
    for lab in m.get("labels", []):
        lab = lab.upper()
        if lab.startswith("CATEGORY_"):
            c = lab[9:].lower()
            return "primary" if c == "personal" else c
    return "primary" if "INBOX" in [x.upper() for x in m.get("labels", [])] else "other"


def is_unread(m):
    if "unread" in m:
        return bool(m["unread"])
    return "UNREAD" in [x.upper() for x in m.get("labels", [])]


def is_spam(m):
    return bool(m.get("spam")) or "SPAM" in [x.upper() for x in m.get("labels", [])]


def day_of(m):
    d = str(m.get("date") or "")[:10]
    try:
        return datetime.strptime(d, "%Y-%m-%d").date()
    except ValueError:
        return None


def clip(s, n=SUBJECT_MAX):
    s = re.sub(r"\s+", " ", str(s or "")).strip()
    return s if len(s) <= n else s[:n - 1] + "…"


def suspicion(m, name, email):
    score, why = 0, []
    dom = domain_of(email)
    if is_spam(m):
        score, why = score + 3, why + ["Gmail put it in Spam"]
    auth = str(m.get("auth") or "").lower()
    if auth in ("fail", "softfail"):
        score, why = score + 3, why + ["failed sender checks (SPF/DKIM/DMARC)"]
    if "xn--" in dom:
        score, why = score + 2, why + ["look-alike domain (punycode)"]
    if dom in FREEMAIL and OFFICIAL_WORDS.search(name or ""):
        score, why = score + 2, why + ["official-sounding name from a free email address"]
    shown = ADDR_RX.search(name or "")
    if shown and shown.group(0).lower() != email:
        score, why = score + 2, why + ["the name shows a different address"]
    rt = (m.get("reply_to") or "").lower()
    if rt and "@" in rt and domain_of(ADDR_RX.search(rt).group(0) if ADDR_RX.search(rt) else rt) != dom:
        score, why = score + 1, why + ["replies go to a different domain"]
    if URGENT_WORDS.search(m.get("subject") or ""):
        score, why = score + 1, why + ["pressure words in the subject"]
    if re.search(r"\d{4,}", dom) or re.search(r"[a-z0-9]{20,}", email.split("@")[0]):
        score, why = score + 1, why + ["random-looking address"]
    if not queryable(email):
        score, why = score + 2, why + ["malformed sender address"]
    return score, why


def chunks(seq, n):
    return [seq[i:i + n] for i in range(0, len(seq), n)]


def from_query(emails):
    safe = [e for e in emails if queryable(e)]
    return "from:(" + " OR ".join(safe) + ")" if safe else ""


def sender_queries(senders):
    """from:(...) queries for a list of senders, in chunks. Unsafe addresses are left out."""
    out = [from_query([s["email"] for s in c]) for c in chunks([s for s in senders if queryable(s["email"])], QUERY_CHUNK)]
    return [q for q in out if q]


def mask(account):
    if not account or "@" not in account:
        return account or ""
    user, dom = account.split("@", 1)
    return user[:1] + "•••@" + dom


def analyse(box):
    offered = {e.lower() for e in box.get("gmail_unsubscribe_offered", [])}
    senders, days, cats = {}, {}, {}
    suspicious_msgs, big = [], []
    msgs = box.get("messages") or []
    for m in msgs:
        name, email = parse_from(m)
        if not email:
            continue
        s = senders.setdefault(email, {"email": email, "name": name, "domain": domain_of(email), "count": 0,
                                       "unread": 0, "bytes": 0, "categories": {}, "list_unsubscribe": False,
                                       "one_click": False, "last": None, "subject": "", "spam": 0, "score": 0,
                                       "why": set(), "starred": 0, "replied": 0})
        if name and not s["name"]:
            s["name"] = name
        s["count"] += 1
        unread = is_unread(m)
        s["unread"] += 1 if unread else 0
        size = int(m.get("size") or 0)
        s["bytes"] += size
        cat = "spam" if is_spam(m) else category_of(m)
        s["categories"][cat] = s["categories"].get(cat, 0) + 1
        s["list_unsubscribe"] |= bool(m.get("list_unsubscribe"))
        s["one_click"] |= bool(m.get("one_click"))
        s["starred"] += 1 if m.get("starred") or "STARRED" in [x.upper() for x in m.get("labels", [])] else 0
        s["replied"] += 1 if m.get("replied") else 0
        d = day_of(m)
        if d and (s["last"] is None or d > s["last"]):
            s["last"], s["subject"] = d, clip(m.get("subject"))
        elif not s["subject"]:
            s["subject"] = clip(m.get("subject"))
        if d:
            days[d] = days.get(d, 0) + 1
        c = cats.setdefault(cat, [0, 0])
        c[0] += 1
        c[1] += 1 if unread else 0
        sc, why = suspicion(m, name, email)
        if is_spam(m):
            s["spam"] += 1
        if sc >= 2:
            s["score"] = max(s["score"], sc)
            s["why"].update(why)
            suspicious_msgs.append({"from": email, "name": name, "subject": clip(m.get("subject")), "date": str(d or ""),
                                    "score": sc, "why": why})
        att = sum(int(a.get("size") or 0) for a in m.get("attachments", []) or [])
        if max(size, att) >= BIG_BYTES:
            big.append({"from": email, "name": name, "subject": clip(m.get("subject")), "date": str(d or ""),
                        "bytes": max(size, att), "files": [clip(a.get("name"), 60) for a in m.get("attachments", []) or []][:5]})

    # Pre-aggregated sender rows (when the agent counted instead of listing every message)
    for r in box.get("senders") or []:
        email = clip((r.get("email") or "").strip().lower(), 120)
        if not email or email in senders:
            continue
        cat = (r.get("category") or "other").lower()
        last = None
        try:
            last = datetime.strptime(str(r.get("last_date"))[:10], "%Y-%m-%d").date()
        except ValueError:
            pass
        senders[email] = {"email": email, "name": r.get("name") or "", "domain": domain_of(email),
                          "count": int(r.get("count") or 0), "unread": int(r.get("unread") or 0),
                          "bytes": int(r.get("bytes") or 0), "categories": {cat: int(r.get("count") or 0)},
                          "list_unsubscribe": bool(r.get("list_unsubscribe")), "one_click": bool(r.get("one_click")),
                          "last": last, "subject": clip(r.get("sample_subject")), "spam": int(r.get("spam") or 0),
                          "score": 3 if r.get("spam") else (0 if queryable(email) else 2),
                          "why": ({"Gmail put it in Spam"} if r.get("spam") else set()) |
                                 (set() if queryable(email) else {"malformed sender address"}),
                          "starred": 0, "replied": 0}
        if not msgs:
            c = cats.setdefault(cat, [0, 0])
            c[0] += int(r.get("count") or 0)
            c[1] += int(r.get("unread") or 0)

    for d, n in (box.get("daily") or {}).items():
        try:
            dd = datetime.strptime(d[:10], "%Y-%m-%d").date()
        except ValueError:
            continue
        days[dd] = max(days.get(dd, 0), int(n))

    rows = []
    for s in senders.values():
        s["read_rate"] = (s["count"] - s["unread"]) / s["count"] if s["count"] else 0
        s["main_category"] = max(s["categories"].items(), key=lambda kv: kv[1])[0] if s["categories"] else "other"
        s["newsletter"] = (s["list_unsubscribe"] or s["email"] in offered or
                           (s["main_category"] in ("promotions", "social", "forums") and s["count"] >= 2))
        s["gmail_unsub"] = s["list_unsubscribe"] or s["email"] in offered
        rows.append(s)
    rows.sort(key=lambda s: (-s["count"], s["email"]))
    return rows, days, cats, suspicious_msgs, big


def build(box):
    rows, days, cats, sus_msgs, big = analyse(box)
    total = sum(s["count"] for s in rows) if rows else 0
    unread = sum(s["unread"] for s in rows)

    def sender_view(s, i=None):
        v = {"email": s["email"], "name": clip(s["name"], 60), "count": s["count"], "unread": s["unread"],
             "read_rate": round(s["read_rate"], 3), "category": s["main_category"], "last": str(s["last"] or ""),
             "subject": s["subject"], "bytes": s["bytes"], "gmail_unsub": s["gmail_unsub"], "one_click": s["one_click"],
             "searchable": queryable(s["email"]),
             # The search for this one sender, ready to use. Empty when the address isn't plain:
             # then nothing is searched or filtered by address at all.
             "query": from_query([s["email"]])}
        if i is not None:
            v["n"] = i
        return v

    suspicious = [s for s in rows if s["score"] >= 2]
    sus_set = {s["email"] for s in suspicious if s["score"] >= 3}
    unsub = [s for s in rows if s["newsletter"] and s["email"] not in sus_set and queryable(s["email"])
             and s["count"] >= UNSUB_MIN_COUNT
             and s["read_rate"] <= UNSUB_MAX_READ and not s["starred"] and not s["replied"]]
    unsub_set = {s["email"] for s in unsub}
    filt = [s for s in rows if s["newsletter"] and s["email"] not in sus_set and s["email"] not in unsub_set
            and queryable(s["email"])
            and s["count"] >= FILTER_MIN_COUNT and s["read_rate"] <= FILTER_MAX_READ]
    never = [s for s in rows if s["count"] >= NEVER_OPENED_MIN and s["unread"] == s["count"] and s["email"] not in sus_set]

    lists = {
        "unsubscribe": [sender_view(s, i + 1) for i, s in enumerate(unsub)],
        "filter": [sender_view(s, i + 1) for i, s in enumerate(filt)],
        "suspicious": [dict(sender_view(s, i + 1), score=s["score"], why=sorted(s["why"])) for i, s in enumerate(
            sorted(suspicious, key=lambda s: (-s["score"], -s["count"])))],
    }
    counts = box.get("counts") or {}

    def cnt(q):
        return counts.get(q)

    steps = []

    def step(title, why, queries, how, agent_can, count=None):
        steps.append({"n": len(steps) + 1, "title": title, "why": why, "queries": queries, "how": how,
                      "agent": agent_can, "count": count})

    if unsub:
        step("Unsubscribe from %d newsletters you don't read" % len(unsub),
             "%d emails in this window, %d%% of them never opened." % (
                 sum(s["count"] for s in unsub), round(100 * sum(s["unread"] for s in unsub) / max(1, sum(s["count"] for s in unsub)))),
             sender_queries(unsub),
             "Open one email from each sender and use Gmail's own Unsubscribe link next to the sender name, or Manage subscriptions. "
             "Where Gmail offers no Unsubscribe, make a filter that deletes or archives them instead.",
             "With your tick per sender (list below).", sum(s["count"] for s in unsub))
    if filt:
        step("Send %d noisy newsletters to a label" % len(filt),
             "You read some of these, so keep them, but out of the inbox.",
             sender_queries(filt),
             "Search the query, then Show search options > Create filter > Skip the Inbox + Apply the label 'Newsletters'. "
             "Tick 'Also apply filter to matching conversations' to clean up what's there now.",
             "With your tick per sender (list below).", sum(s["count"] for s in filt))
    step("Archive old unread promotions", "Promotions you didn't open in two weeks are rarely opened later.",
         ["category:promotions is:unread older_than:14d"],
         "Search the query, select all (including 'Select all conversations that match this search'), Archive. "
         "Archived mail stays searchable.", "Only after you confirm the count.",
         cnt("category:promotions is:unread older_than:14d"))
    step("Clear old social and update notifications", "Notifications lose their point after a month.",
         ["category:social older_than:30d", "category:updates is:unread older_than:30d"],
         "Search each query, select all, Archive.", "Only after you confirm the count.",
         cnt("category:social older_than:30d"))
    if suspicious:
        step("Report %d suspicious senders" % len(suspicious),
             "Likely spam or phishing. Never use their unsubscribe links: that confirms your address is live.",
             sender_queries(suspicious),
             "Open the message menu (three dots) > Report phishing, or Report spam, then Block sender. Don't click links or open attachments.",
             "With your tick per sender (list below).", sum(s["count"] for s in suspicious))
    step("Find big attachments", "Old large attachments are what fill Gmail storage.",
         ["larger:10M", "has:attachment larger:5M older_than:1y"],
         "Search, save anything you need to your computer or Drive, then delete the emails and empty Trash.",
         "No. You decide what to keep.", cnt("larger:10M"))
    step("Keep it tidy", "A weekly two-minute sweep keeps the inbox at zero.",
         ["is:unread older_than:7d -in:chats", "in:inbox older_than:30d"],
         "Once a week: search the first query, archive what you won't act on. Keep Gmail's inbox tabs on.",
         "No.", None)
    if "in:spam" in counts:
        step("Leave Spam alone", "Gmail deletes Spam after 30 days by itself.", ["in:spam"],
             "Nothing to do unless storage is tight: then 'Delete all spam messages now'.", "No.", counts.get("in:spam"))

    start = box.get("window", {}).get("start")
    end = box.get("window", {}).get("end")
    if days:
        d0 = min(days) if not start else datetime.strptime(start[:10], "%Y-%m-%d").date()
        d1 = max(days) if not end else datetime.strptime(end[:10], "%Y-%m-%d").date()
    else:
        d1 = date.today()
        d0 = d1 - timedelta(days=60)
    daily = []
    d = d0
    while d <= d1 and len(daily) < 400:
        daily.append([d.isoformat(), days.get(d, 0)])
        d += timedelta(days=1)

    top = [dict(sender_view(s)) for s in rows[:15]]
    report = {
        "account": mask(box.get("account")),
        "window": {"start": d0.isoformat(), "end": d1.isoformat()},
        "sources": box.get("sources", []),
        "complete": box.get("complete", True),
        "agent_notes": box.get("notes", ""),
        "headline": box.get("headline", ""),
        "highlights": box.get("highlights", []),
        "storage": box.get("storage"),
        "totals": {"messages": total, "unread": unread, "senders": len(rows),
                   "newsletters": sum(1 for s in rows if s["newsletter"]), "never_opened": len(never),
                   "suspicious": len(suspicious), "suspicious_messages": len(sus_msgs),
                   "big_messages": len(big), "big_bytes": sum(b["bytes"] for b in big),
                   "inbox_count": counts.get("in:inbox") or counts.get("inbox"), "spam_count": counts.get("in:spam")},
        "categories": sorted(([k, v[0], v[1]] for k, v in cats.items()), key=lambda kv: -kv[1]),
        "daily": daily,
        "top_senders": top,
        "never_opened": [sender_view(s) for s in never[:60]],
        "big": sorted(big, key=lambda b: -b["bytes"])[:20],
        "suspicious_examples": sorted(sus_msgs, key=lambda m: -m["score"])[:25],
        "lists": lists,
        "steps": steps,
        "thresholds": {"unsubscribe_max_read": UNSUB_MAX_READ, "unsubscribe_min_count": UNSUB_MIN_COUNT,
                       "filter_max_read": FILTER_MAX_READ, "filter_min_count": FILTER_MIN_COUNT,
                       "never_opened_min": NEVER_OPENED_MIN, "big_mb": BIG_BYTES // (1024 * 1024)},
        "generated": datetime.now().isoformat(timespec="seconds"),
    }
    # Last check before anything is written: every query is a fixed search or from:(plain addresses).
    emitted = [q for s in steps for q in s["queries"]] + [v["query"] for vs in lists.values() for v in vs if v["query"]]
    emitted += [v["query"] for v in report["top_senders"] + report["never_opened"] if v["query"]]
    bad = [q for q in emitted if q not in FIXED_QUERIES and not SAFE_QUERY.fullmatch(q)]
    if bad:
        sys.exit("refusing to write the report: a search query isn't in a safe form: %r" % bad[:3])
    core = json.dumps([lists, [s["queries"] for s in steps]], sort_keys=True, ensure_ascii=False)
    report["report_id"] = hashlib.sha256(core.encode("utf-8")).hexdigest()[:10]
    return report


def embed(obj):
    return (json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
            .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Build the inbox report page from mailbox.json.")
    ap.add_argument("mailbox")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write inbox-plan.json (default: next to --out)")
    a = ap.parse_args(argv)
    with open(a.mailbox, encoding="utf-8") as f:
        box = json.load(f)
    if not (box.get("messages") or box.get("senders")):
        sys.exit("mailbox.json has no 'messages' or 'senders'. See references/gathering.md.")
    report = build(box)
    plan_path = a.plan or os.path.join(os.path.dirname(os.path.abspath(a.out)), "inbox-plan.json")
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump({"schema": "inbox-cleanup/plan@1", "report_id": report["report_id"], "lists": report["lists"],
                   "steps": report["steps"]}, f, ensure_ascii=False, indent=1)
    with open(TEMPLATE, encoding="utf-8") as f:
        html = f.read()
    if "__INBOX_DATA__" not in html:
        sys.exit("template is missing the __INBOX_DATA__ placeholder")
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(html.replace("__INBOX_DATA__", embed(report)))
    t = report["totals"]
    print("page: %s (%d KB)\nplan: %s  report_id=%s\n%d emails, %d unread, %d senders, %d to unsubscribe, %d to filter, "
          "%d suspicious, %d big" % (os.path.abspath(a.out), os.path.getsize(a.out) // 1024, plan_path, report["report_id"],
                                     t["messages"], t["unread"], t["senders"], len(report["lists"]["unsubscribe"]),
                                     len(report["lists"]["filter"]), t["suspicious"], t["big_messages"]))


if __name__ == "__main__":
    main()

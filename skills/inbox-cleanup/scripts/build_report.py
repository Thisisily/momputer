#!/usr/bin/env python3
"""Turn mailbox.json (what the agent gathered, read-only) into the inbox report page.

    python build_report.py mailbox.json --out inbox-report.html

Writes the page and inbox-plan.json next to it (numbered senders and steps, so an
approval like "INBOX-APPROVE report=ab12cd34ef unsubscribe=1-6 filter=2 steps=3"
can be mapped back to exact senders and queries). The page holds sender names,
addresses, subject lines and counts only. Never message bodies. Every number the page's
charts draw (mail per week and day by kind, weekday by hour, unread by age, opened by kind,
the newsletters table) is worked out here; the page only draws them.

The mailbox.json format is in references/gathering.md. Python 3.8+, standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import sys
from datetime import date, datetime, timedelta
from email.utils import parsedate_to_datetime

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
# Mail that talks to an assistant instead of a person. It's listed as suspicious, never obeyed.
ORDERS = re.compile(r"ignore (all |any |the )?(previous|prior|above|earlier|your) (instructions|rules)|disregard (all |any |the )?"
                    r"(previous |prior |above )?instructions|system prompt|as an ai\b|you are (an? )?(ai|assistant|agent)\b|"
                    r"forward (all|every|each)( of)?( your| my| the)? (e-?mails?|mail|messages)", re.I)
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

# What a message is, for the charts (separate from the plan's own rules above). In this order:
# Spam, then Gmail's Promotions and Social tabs, then anything with an unsubscribe option, then
# Updates and Forums or a role address (no-reply@, alerts@ ...), and what's left is people.
KINDS = ["people", "newsletters", "promotions", "notifications", "spam"]
ROLE_ADDR = re.compile(r"^(no-?reply|do-?not-?reply|donotreply|notifications?|notify|alerts?|updates?|mailer-daemon|"
                       r"postmaster|bounces?|newsletters?|news|digest|info|hello|team|support|billing|receipts?|orders?|"
                       r"accounts?|security|marketing|service|admin|system|automated|robot|bot|auto-confirm|confirm)"
                       r"(?=$|[._+-])", re.I)
DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
# Characters that turn text around or hide in it: a name like "Shop <U+202E>gnp.exe" reads as "exe.png".
INVISIBLE = re.compile("[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\u200b-\u200f\u202a-\u202e\u2060-\u2064\u2066-\u2069\ufeff]")
TS = re.compile(r"(\d{4}-\d{2}-\d{2})(?:[T ](\d{1,2}):(\d{2})(?::\d{2}(?:\.\d+)?)?\s*(Z|[+-]\d{2}:?\d{2})?)?", re.I)
OFFSET = re.compile(r"(?:UTC|GMT)?\s*([+-])(\d{1,2})(?::?(\d{2}))?$", re.I)


def parse_from(m):
    name, email = m.get("from_name"), m.get("from_email")
    raw = str(m.get("from") or "")
    if not email:
        # the address is the last <...> that holds an @: a display name may itself hold "<" and ">"
        found = [x for x in re.finditer(r"<([^<>]+)>", raw) if "@" in x.group(1)] or list(re.finditer(r"<([^<>]+)>", raw))
        mm = found[-1] if found else None
        email = mm.group(1) if mm else (ADDR_RX.search(raw).group(0) if ADDR_RX.search(raw) else raw)
        if name is None:
            name = raw[:mm.start()].strip().strip('"') if mm else ""
    email = clip((email or "").strip().lower(), 120)
    return clip((name or "").strip(), 80), email


def queryable(email):
    """True only for a plain address that Gmail's search can't read as anything but an address."""
    return isinstance(email, str) and len(email) <= 100 and ".." not in email and bool(SAFE_ADDR.fullmatch(email))


def domain_of(email):
    return email.rsplit("@", 1)[-1] if "@" in email else email


def labels_of(m):
    labs = m.get("labels")
    return [x.upper() for x in labs if isinstance(x, str)] if isinstance(labs, list) else []


def category_of(m):
    c = str(m.get("category") or "").lower()
    if c in CATEGORIES:
        return c
    for lab in labels_of(m):
        if lab.startswith("CATEGORY_"):
            c = lab[9:].lower()
            return "primary" if c == "personal" else c
    return "primary" if "INBOX" in labels_of(m) else "other"


def is_unread(m):
    if "unread" in m:
        return bool(m["unread"])
    return "UNREAD" in labels_of(m)


def is_spam(m):
    return bool(m.get("spam")) or "SPAM" in labels_of(m)


def offset_minutes(text):
    """"+03:00", "-0530", "UTC+3", "Z" as minutes east of UTC. None when it isn't one."""
    t = str(text or "").strip()
    if t.upper() in ("Z", "UTC", "GMT"):
        return 0
    mm = OFFSET.fullmatch(t)
    if not mm or int(mm.group(2)) > 14 or int(mm.group(3) or 0) > 59:
        return None
    return (1 if mm.group(1) == "+" else -1) * (int(mm.group(2)) * 60 + int(mm.group(3) or 0))


def when_of(m, tz=None):
    """(day, hour) a message arrived. hour is None when only the day is known. tz: the person's own
    offset in minutes; a time written with its own offset (or Z) is moved to it. Takes ISO 8601
    ("2026-10-01T08:12:00Z"), an email Date header ("Wed, 01 Oct 2026 08:12:00 +0000") or Gmail's
    internalDate (milliseconds since 1970, UTC)."""
    raw = m.get("date")
    if isinstance(raw, (int, float)) and not isinstance(raw, bool) or (isinstance(raw, str) and re.fullmatch(r"\d{12,13}", raw.strip())):
        try:
            t = datetime(1970, 1, 1) + timedelta(milliseconds=int(raw))
        except (ValueError, OverflowError):
            return None, None
        if tz is not None:
            t += timedelta(minutes=tz)
        return t.date(), t.hour
    s = str(raw or "").strip()
    mm = TS.match(s)
    if not mm:
        try:
            t = parsedate_to_datetime(s)
        except (TypeError, ValueError, IndexError):
            return None, None
        if t is None:
            return None, None
        if tz is not None and t.utcoffset() is not None:
            t = t.replace(tzinfo=None) - t.utcoffset() + timedelta(minutes=tz)
        return t.date(), t.hour
    try:
        d = datetime.strptime(mm.group(1), "%Y-%m-%d")
    except ValueError:
        return None, None
    if mm.group(2) is None or int(mm.group(2)) > 23 or int(mm.group(3)) > 59:
        return d.date(), None
    t = d.replace(hour=int(mm.group(2)), minute=int(mm.group(3)))
    own = offset_minutes(mm.group(4)) if mm.group(4) else None
    if tz is not None and own is not None:
        t = t - timedelta(minutes=own) + timedelta(minutes=tz)
    return t.date(), t.hour


def day_of(m, tz=None):
    return when_of(m, tz)[0]


def kind_of(cat, list_unsub, email, offered):
    """What a message is, for the charts: people, newsletters, promotions, notifications or spam."""
    if cat == "spam":
        return "spam"
    if cat == "promotions":
        return "promotions"
    if cat == "social":
        return "notifications"
    if list_unsub or email in offered:
        return "newsletters"
    if cat in ("updates", "forums") or ROLE_ADDR.match(email.split("@")[0]):
        return "notifications"
    return "people"


def clip(s, n=SUBJECT_MAX):
    s = re.sub(r"\s+", " ", INVISIBLE.sub("", str(s or ""))).strip()
    return s if len(s) <= n else s[:n - 1] + "…"


def short_day(d):
    return "%d %s" % (d.day, MONTHS[d.month - 1])


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
    if URGENT_WORDS.search(str(m.get("subject") or "")):
        score, why = score + 1, why + ["pressure words in the subject"]
    if ORDERS.search(str(m.get("subject") or "") + " " + (name or "")):
        score, why = score + 2, why + ["reads like an order to an assistant"]
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


def number(v):
    """A count or size from the gathered JSON, or 0 when it isn't a plain number."""
    try:
        return max(0, int(v or 0))
    except (TypeError, ValueError):
        return 0


def analyse(box):
    offered = {e.lower() for e in box.get("gmail_unsubscribe_offered", []) if isinstance(e, str)}
    tz = offset_minutes(box.get("utc_offset")) if box.get("utc_offset") not in (None, "") else None
    senders, days, cats = {}, {}, {}
    suspicious_msgs, big = [], []
    # for the charts: each message's kind by day, weekday by hour, and the unread ones by day
    dash = {"kind_days": {}, "heat": [[0] * 24 for _ in DAYS], "heat_n": 0, "unread_days": {}, "tz": tz}
    msgs = [m for m in box.get("messages") or [] if isinstance(m, dict)]
    for m in msgs:
        name, email = parse_from(m)
        if not email:
            continue
        s = senders.setdefault(email, {"email": email, "name": name, "domain": domain_of(email), "count": 0,
                                       "unread": 0, "bytes": 0, "categories": {}, "list_unsubscribe": False,
                                       "one_click": False, "last": None, "subject": "", "spam": 0, "score": 0,
                                       "why": set(), "starred": 0, "replied": 0, "kinds": {}, "kinds_unread": {}})
        if name and not s["name"]:
            s["name"] = name
        s["count"] += 1
        unread = is_unread(m)
        s["unread"] += 1 if unread else 0
        size = number(m.get("size"))
        s["bytes"] += size
        cat = "spam" if is_spam(m) else category_of(m)
        s["categories"][cat] = s["categories"].get(cat, 0) + 1
        s["list_unsubscribe"] |= bool(m.get("list_unsubscribe"))
        s["one_click"] |= bool(m.get("one_click"))
        s["starred"] += 1 if m.get("starred") or "STARRED" in labels_of(m) else 0
        s["replied"] += 1 if m.get("replied") else 0
        sc, why = suspicion(m, name, email)
        kind = kind_of(cat, bool(m.get("list_unsubscribe")), email, offered)
        if sc >= 3 or (sc >= 2 and kind == "people"):   # phishing that looks like a person isn't one
            kind = "spam"
        s["kinds"][kind] = s["kinds"].get(kind, 0) + 1
        if unread:
            s["kinds_unread"][kind] = s["kinds_unread"].get(kind, 0) + 1
        d, hour = when_of(m, tz)
        if d and (s["last"] is None or d > s["last"]):
            s["last"], s["subject"] = d, clip(m.get("subject"))
        elif not s["subject"]:
            s["subject"] = clip(m.get("subject"))
        if d:
            days[d] = days.get(d, 0) + 1
            kd = dash["kind_days"].setdefault(d, {})
            kd[kind] = kd.get(kind, 0) + 1
            if unread:
                ud = dash["unread_days"].setdefault(d, {})
                ud[kind] = ud.get(kind, 0) + 1
            if hour is not None:
                dash["heat"][d.weekday()][hour] += 1
                dash["heat_n"] += 1
        c = cats.setdefault(cat, [0, 0])
        c[0] += 1
        c[1] += 1 if unread else 0
        if is_spam(m):
            s["spam"] += 1
        if sc >= 2:
            s["score"] = max(s["score"], sc)
            s["why"].update(why)
            suspicious_msgs.append({"from": email, "name": name, "subject": clip(m.get("subject")), "date": str(d or ""),
                                    "score": sc, "why": why})
        atts = [a for a in m.get("attachments") or [] if isinstance(a, dict)] if isinstance(m.get("attachments"), list) else []
        att = sum(number(a.get("size")) for a in atts)
        if max(size, att) >= BIG_BYTES:
            big.append({"from": email, "name": name, "subject": clip(m.get("subject")), "date": str(d or ""),
                        "bytes": max(size, att), "files": [clip(a.get("name"), 60) for a in atts][:5], "kind": kind})

    # Pre-aggregated sender rows (when the agent counted instead of listing every message)
    for r in box.get("senders") or []:
        if not isinstance(r, dict):
            continue
        email = clip(str(r.get("email") or "").strip().lower(), 120)
        if not email or email in senders:
            continue
        cat = "spam" if r.get("spam") and not r.get("category") else str(r.get("category") or "other").lower()
        last = None
        try:
            last = datetime.strptime(str(r.get("last_date"))[:10], "%Y-%m-%d").date()
        except ValueError:
            pass
        n = number(r.get("count"))
        kind = kind_of(cat, bool(r.get("list_unsubscribe")), email, offered)
        if r.get("spam") or (kind == "people" and not queryable(email)):
            kind = "spam"
        senders[email] = {"email": email, "name": clip(r.get("name"), 80), "domain": domain_of(email),
                          "count": n, "unread": min(n, number(r.get("unread"))),
                          "bytes": number(r.get("bytes")), "categories": {cat: n},
                          "list_unsubscribe": bool(r.get("list_unsubscribe")), "one_click": bool(r.get("one_click")),
                          "last": last, "subject": clip(r.get("sample_subject")), "spam": number(r.get("spam")),
                          "score": 3 if r.get("spam") else (0 if queryable(email) else 2),
                          "why": ({"Gmail put it in Spam"} if r.get("spam") else set()) |
                                 (set() if queryable(email) else {"malformed sender address"}),
                          "starred": number(r.get("starred")), "replied": number(r.get("replied")), "kinds": {kind: n},
                          "kinds_unread": {kind: min(n, number(r.get("unread")))}}
        if not msgs:
            c = cats.setdefault(cat, [0, 0])
            c[0] += n
            c[1] += min(n, number(r.get("unread")))

    daily_in = box.get("daily") if isinstance(box.get("daily"), dict) else {}
    for d, n in daily_in.items():
        try:
            dd = datetime.strptime(str(d)[:10], "%Y-%m-%d").date()
        except ValueError:
            continue
        days[dd] = max(days.get(dd, 0), number(n))

    rows = []
    for s in senders.values():
        s["read_rate"] = (s["count"] - s["unread"]) / s["count"] if s["count"] else 0
        s["main_category"] = max(s["categories"].items(), key=lambda kv: kv[1])[0] if s["categories"] else "other"
        s["newsletter"] = (s["list_unsubscribe"] or s["email"] in offered or
                           (s["main_category"] in ("promotions", "social", "forums") and s["count"] >= 2))
        s["gmail_unsub"] = s["list_unsubscribe"] or s["email"] in offered
        s["kind"] = max(KINDS, key=lambda k: (s["kinds"].get(k, 0), -KINDS.index(k))) if s["kinds"] else "people"
        rows.append(s)
    rows.sort(key=lambda s: (-s["count"], s["email"]))
    dash["listed"] = bool(msgs)
    return rows, days, cats, suspicious_msgs, big, dash


def agent_insights(box):
    """Mom's take: the agent's own findings, [{title, text}]. Older mailbox.json files give `highlights`
    (plain sentences) instead; those are taken as insights without a title."""
    out = []
    src = box.get("insights")
    for it in src if isinstance(src, list) else []:
        if isinstance(it, dict) and (it.get("text") or it.get("title")):
            out.append({"title": clip(it.get("title"), 90), "text": clip(it.get("text"), 420)})
        elif isinstance(it, str) and it.strip():
            out.append({"title": "", "text": clip(it, 420)})
    if not out:
        hl = box.get("highlights")
        for h in hl if isinstance(hl, list) else []:
            if isinstance(h, str) and h.strip():
                out.append({"title": "", "text": clip(h, 420)})
    return out[:6]


def parse_day(text):
    try:
        return datetime.strptime(str(text)[:10], "%Y-%m-%d").date()
    except ValueError:
        return None


def dashboard(rows, days, dash, d0, d1):
    """Everything the page's charts draw, worked out here so the page only draws."""
    n_days = (d1 - d0).days + 1
    listed = dash["listed"]
    kind_days = dash["kind_days"]

    def series_for(day_list, source, rest=None):
        out = {k: [sum(source.get(d, {}).get(k, 0) for d in group) for group in day_list] for k in KINDS}
        if rest:
            out["other"] = [sum(max(0, rest.get(d, 0) - sum(source.get(d, {}).values())) for d in group) for group in day_list]
        return {k: v for k, v in out.items() if any(v)}

    # mail per day and per week, by kind. Weeks are counted back from the last day, so the newest
    # column is a whole week and only the first one can be short.
    all_days = [d0 + timedelta(days=i) for i in range(min(n_days, 400))]
    weeks = []
    end = d1
    while end >= d0 and len(weeks) < 60:
        start = max(d0, end - timedelta(days=6))
        weeks.insert(0, [start + timedelta(days=i) for i in range((end - start).days + 1)])
        end = start - timedelta(days=1)
    if listed:
        by_day, by_week = series_for([[d] for d in all_days], kind_days, days), series_for(weeks, kind_days, days)
    else:  # only counted per sender: one series of all mail, from the daily counts if there are any
        by_day = {"other": [days.get(d, 0) for d in all_days]}
        by_week = {"other": [sum(days.get(d, 0) for d in w) for w in weeks]}
        by_day, by_week = ({k: v for k, v in x.items() if any(v)} for x in (by_day, by_week))
    volume = {"days": {"labels": ["%s %s" % (DAYS[d.weekday()], short_day(d)) for d in all_days], "series": by_day},
              "weeks": {"labels": [short_day(w[0]) for w in weeks], "series": by_week,
                        "first_days": len(weeks[0]) if weeks else 0}}

    # what's still unread, by how long ago it came: this week, last week, ... oldest first
    unread_days = dash["unread_days"]
    backlog = None
    if listed and unread_days:
        ages = list(reversed(weeks))   # ages[0] is this week
        labels = ["This week" if i == 0 else "1 week" if i == 1 else "%d weeks" % i for i in range(len(ages))]
        ser = series_for(list(reversed(ages)), unread_days)
        backlog = {"labels": list(reversed(labels)), "series": ser,
                   "older_than_2w": sum(sum(unread_days.get(d, {}).values()) for d in unread_days if (d1 - d).days >= 14),
                   "n": sum(sum(v.values()) for v in unread_days.values())}

    heat = None
    if dash["heat_n"]:
        tz = dash["tz"]
        heat = {"cells": dash["heat"], "n": dash["heat_n"],
                "tz": "" if tz is None else "UTC%s%02d:%02d" % ("+" if tz >= 0 else "-", abs(tz) // 60, abs(tz) % 60)}

    kinds = []
    for k in KINDS:
        n = sum(s["kinds"].get(k, 0) for s in rows)
        if not n:
            continue
        unread = sum(s["kinds_unread"].get(k, 0) for s in rows)
        kinds.append({"key": k, "count": n, "unread": unread, "opened": n - unread,
                      "senders": sum(1 for s in rows if s["kind"] == k)})
    return {"volume": volume, "backlog": backlog, "heat": heat, "kinds": kinds}


def build(box):
    rows, days, cats, sus_msgs, big, dash = analyse(box)
    total = sum(s["count"] for s in rows) if rows else 0
    unread = sum(s["unread"] for s in rows)
    notes_in = box.get("sender_notes") if isinstance(box.get("sender_notes"), dict) else {}
    notes = {str(k).strip().lower(): clip(v, 160) for k, v in notes_in.items() if isinstance(v, str) and v.strip()}
    seen = {s["email"] for s in rows}
    notes = {k: v for k, v in notes.items() if k in seen}

    def dash_view(s):
        """A sender as the dashboard shows it: the plan's fields plus what it is and how often it's opened."""
        v = sender_view(s)
        v.update(kind=s["kind"], opened=int(round(100 * s["read_rate"])), replied=s["replied"], starred=s["starred"],
                 note=notes.get(s["email"], ""))
        return v

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
    counts = box.get("counts") if isinstance(box.get("counts"), dict) else {}

    def cnt(q):
        v = counts.get(q)
        if isinstance(v, bool):
            return None
        if isinstance(v, (int, float)):
            return max(0, int(v))
        return int(v.strip()) if isinstance(v, str) and v.strip().isdigit() else None

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
             "Nothing to do unless storage is tight: then 'Delete all spam messages now'.", "No.", cnt("in:spam"))

    win = box.get("window") if isinstance(box.get("window"), dict) else {}
    start, end = parse_day(win.get("start")) if win.get("start") else None, parse_day(win.get("end")) if win.get("end") else None
    if days:
        d0 = start or min(days)
        d1 = end or max(days)
    else:
        d1 = end or date.today()
        d0 = start or d1 - timedelta(days=60)
    if d0 > d1:
        d0, d1 = d1, d0
    daily = []
    d = d0
    while d <= d1 and len(daily) < 400:
        daily.append([d.isoformat(), days.get(d, 0)])
        d += timedelta(days=1)

    top = [dash_view(s) for s in rows[:15]]
    # the people who write to you, the ones you answer first
    people = sorted([s for s in rows if s["kind"] == "people" and s["score"] < 2],
                    key=lambda s: (-(s["replied"] + s["starred"]), -s["count"], s["email"]))[:10]
    newsletters = [s for s in rows if s["newsletter"] and s["score"] < 3][:150]
    storage = box.get("storage") if isinstance(box.get("storage"), dict) else None
    if storage:
        storage = {k: float(storage[k]) for k in ("used_gb", "total_gb") if isinstance(storage.get(k), (int, float)) and not isinstance(storage.get(k), bool)}
        storage = storage if storage.get("used_gb") is not None else None
    sources = box.get("sources") if isinstance(box.get("sources"), list) else []
    hl = box.get("highlights") if isinstance(box.get("highlights"), list) else []
    shown = {v["email"] for vs in lists.values() for v in vs}
    report = {
        "account": mask(clip(box.get("account"), 120) if isinstance(box.get("account"), str) else ""),
        "window": {"start": d0.isoformat(), "end": d1.isoformat(), "days": (d1 - d0).days + 1},
        "sources": [clip(x, 60) for x in sources if isinstance(x, str)][:6],
        "complete": box.get("complete", True) is not False,
        "agent_notes": clip(box.get("notes"), 1200) if isinstance(box.get("notes"), str) else "",
        "headline": clip(box.get("headline"), 220) if isinstance(box.get("headline"), str) else "",
        "highlights": [clip(h, 420) for h in hl if isinstance(h, str)][:6],
        "insights": agent_insights(box),
        "storage": storage,
        "totals": {"messages": total, "unread": unread, "senders": len(rows),
                   "newsletters": sum(1 for s in rows if s["newsletter"]), "never_opened": len(never),
                   "suspicious": len(suspicious), "suspicious_messages": len(sus_msgs),
                   "big_messages": len(big), "big_bytes": sum(b["bytes"] for b in big),
                   "inbox_count": cnt("in:inbox") if "in:inbox" in counts else cnt("inbox"), "spam_count": cnt("in:spam"),
                   "people": sum(1 for s in rows if s["kind"] == "people"),
                   "one_click": sum(1 for s in newsletters if s["one_click"])},
        "categories": sorted(([k, v[0], v[1]] for k, v in cats.items()), key=lambda kv: -kv[1]),
        "daily": daily,
        "top_senders": top,
        "people": [dash_view(s) for s in people],
        "newsletters": [dash_view(s) for s in newsletters],
        "never_opened": [dash_view(s) for s in never[:60]],
        "big": sorted(big, key=lambda b: -b["bytes"])[:20],
        "suspicious_examples": sorted(sus_msgs, key=lambda m: -m["score"])[:25],
        # what each sender in the plan is, and the agent's note on it, as [address, text] pairs
        "sender_kinds": [[s["email"], s["kind"]] for s in rows if s["email"] in shown],
        "sender_notes": [[e, t] for e, t in sorted(notes.items()) if e in shown],
        "fixed_queries": sorted(FIXED_QUERIES),
        "lists": lists,
        "steps": steps,
        "thresholds": {"unsubscribe_max_read": UNSUB_MAX_READ, "unsubscribe_min_count": UNSUB_MIN_COUNT,
                       "filter_max_read": FILTER_MAX_READ, "filter_min_count": FILTER_MIN_COUNT,
                       "never_opened_min": NEVER_OPENED_MIN, "big_mb": BIG_BYTES // (1024 * 1024)},
        "generated": datetime.now().isoformat(timespec="seconds"),
    }
    report.update(dashboard(rows, days, dash, d0, d1))
    # Last check before anything is written: every query is a fixed search or from:(plain addresses).
    emitted = [q for s in steps for q in s["queries"]] + [v["query"] for vs in lists.values() for v in vs if v["query"]]
    emitted += [v["query"] for v in report["top_senders"] + report["never_opened"] + report["people"] + report["newsletters"] if v["query"]]
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
    if not isinstance(box, dict) or not (box.get("messages") or box.get("senders")):
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

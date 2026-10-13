#!/usr/bin/env python3
"""Turn calendar.json (what the agent gathered, read-only) into the calendar report page.

    python build_report.py calendar.json --out calendar-report.html

Writes the page and calendar-plan.json next to it: the numbered fixes that an approval
like "CALENDAR-APPROVE report=ab12cd34ef yes=1,3 no=2 move=1 decline=2 shorten=1-4 habits=1"
refers to. The page holds event titles, times, calendar names, guest counts and at most
the organizer's name. Never descriptions, and never the guest list.

The calendar.json format is in references/gathering.md. Python 3.8+, standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import sys
from datetime import date, datetime, timedelta

HERE = os.path.dirname(os.path.abspath(__file__))
TEMPLATE = os.path.join(HERE, "..", "assets", "calendar-report-template.html")

# Thresholds (all documented in SKILL.md so the agent can explain them)
PAST_DAYS = 14             # looked at for patterns only
FUTURE_DAYS = 28           # the part that can still be fixed
WORK_START, WORK_END = 9 * 60, 18 * 60   # used when calendar.json gives no working hours
WORK_DAYS = (0, 1, 2, 3, 4)              # Monday to Friday
GAP = 5                    # minutes: a gap this short between two meetings is no break
RUN_MEETINGS = 3           # this many meetings in a row, or...
RUN_MINUTES = 120          # ...this long without a break, is a back-to-back run
FREE_BLOCK = 120           # a working day needs one free block this long
BREAK = 15                 # the break offered after a run
ODD_FROM, ODD_UNTIL = 22, 6   # a meeting starting between these hours may be a time-zone slip
SEVERAL = 3                # "several people": this many, the person included
STALE_DAYS = 730           # a repeating meeting this old with no end date is worth a look
STALE_DECLINES = 2
MAX_PROPOSE = 200          # Calendar can't propose a new time above this many guests
TEXT_MAX = 90
MAX_ROWS = 60              # per tickable list
MAX_TABLE = 40
MAX_EVENTS = 6000

# ---- safe forms. Everything that ends up in a link, or that the agent later hands to a
# calendar tool, must match one of these in full. Titles, names and places never do: they
# are only ever shown as text.
SAFE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,199}")       # event and series ids
SAFE_KEY = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,39}")       # the agent's short calendar keys
SAFE_WHEN = re.compile(r"\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2})?")
SAFE_TZ = re.compile(r"[A-Za-z][A-Za-z0-9_+/-]{0,39}")
DAY_LINK = re.compile(r"https://calendar\.google\.com/calendar/u/0/r/day/\d{4}/\d{1,2}/\d{1,2}")
# A place or video address is written by whoever made the event. It becomes a link only when
# it is plain https on one of these meeting hosts. Anything else ("javascript:...", http, a
# look-alike host, a port, a user@host trick) stays text.
_HOST = r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+"
MEETING_URL = re.compile(r"https://(" + _HOST + r")(?:/[A-Za-z0-9._~%/?=&+-]*)?")
MEETING_HOSTS = ("meet.google.com", "teams.microsoft.com", "teams.live.com", "whereby.com")   # exactly these
MEETING_DOMAINS = ("zoom.us", "webex.com")                                                    # these and their subdomains
LIST_ACTIONS = {"answer": {"answer"}, "drop": {"decline_series", "end_series"}, "move": {"move", "propose"},
                "shorten": {"shorten"}, "breaks": {"add_break"}, "duplicates": {"delete_event"},
                "links": {"add_video"}, "habits": {"habit"}}
HABIT_KEYS = {"speedy", "working_hours", "focus", "answer", "review", "timezone", "auto_video"}

RESPONSES = {"accepted": "accepted", "yes": "accepted", "declined": "declined", "no": "declined",
             "tentative": "tentative", "maybe": "tentative", "needsaction": "needs_action",
             "needs_action": "needs_action", "unanswered": "needs_action", "invited": "needs_action"}
# "other" is a note on the calendar, never time that's taken: where the person works that day, a
# birthday, a task. A booking Calendar added from a mail (a flight, a table) is time like any event's.
KINDS = {"": "event", "event": "event", "default": "event", "focus": "focus", "focustime": "focus",
         "focus_time": "focus", "out_of_office": "out_of_office", "outofoffice": "out_of_office",
         "working_location": "other", "workinglocation": "other", "birthday": "other", "reminder": "other",
         "task": "other", "from_gmail": "from_gmail", "fromgmail": "from_gmail"}
DAY_KEYS = {"mon": 0, "monday": 0, "tue": 1, "tuesday": 1, "wed": 2, "wednesday": 2, "thu": 3, "thursday": 3,
            "fri": 4, "friday": 4, "sat": 5, "saturday": 5, "sun": 6, "sunday": 6}
DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
DAY_NAMES = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
WHEN_RX = re.compile(r"(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::\d{2}(?:\.\d+)?)?)?\s*(Z|[+-]\d{2}:?\d{2})?")
INVISIBLE = re.compile(r"[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]")
# Text that reads like an order to the assistant, or like code. It is never obeyed either way;
# this only decides what gets listed under "Text that reads like an instruction".
ORDER_RX = re.compile(r"ignore (all |any |the |your )?(previous|prior|above|earlier|other)|disregard (all|the|your|any)|"
                      r"system prompt|new instructions?|you are now|you must|as an ai|"
                      r"(ai|dear|hey|attention),? (assistant|agent|model)|\bassistant[:,]|"
                      r"do ?n[o']t (tell|ask|mention)|without asking|approve (all|every)|accept (all|every)|"
                      r"decline (all|every)|delete (all|every)|cancel (all|every)|forward (this|it|all|every)|"
                      r"share (this|it|all|every)|send (this|it|all|me|your)|(run|execute) (this|the) (command|script|code)|"
                      r"\bcurl |powershell|rm -rf|click (here|this)|open (this|the) link", re.I)
CODE_RX = re.compile(r"<\s*/?\s*(script|img|svg|iframe|a |body|style)|on(error|load|click)\s*=|"
                     r"^\s*(javascript|data|vbscript|file)\s*:|\$\(|`", re.I)


def clip(s, n=TEXT_MAX):
    """One line of plain text: no control or direction-override characters, at most n long."""
    s = re.sub(r"\s+", " ", INVISIBLE.sub(" ", str("" if s is None else s))).strip()
    return s if len(s) <= n else s[:n - 1] + "…"


def num(v, top=100000):
    try:
        return max(0, min(int(v), top))
    except (TypeError, ValueError, OverflowError):
        return 0


def parse_when(s):
    """(datetime, has a clock time, written as UTC). The clock time is read as written."""
    m = WHEN_RX.fullmatch(str(s or "").strip())
    if not m:
        return None, False, False
    try:
        if m.group(4) is None:
            return datetime(int(m.group(1)), int(m.group(2)), int(m.group(3))), False, False
        return (datetime(int(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4)), int(m.group(5))),
                True, m.group(6) == "Z")
    except ValueError:
        return None, False, False


def parse_day(s):
    dt = parse_when(str(s or "").strip()[:10])[0]
    return dt.date() if dt else None


def parse_clock(s, default):
    m = re.fullmatch(r"(\d{1,2}):(\d{2})", str(s or "").strip())
    if m and int(m.group(1)) < 24 and int(m.group(2)) < 60:
        return int(m.group(1)) * 60 + int(m.group(2))
    return default


def clock(minutes):
    return "%02d:%02d" % (minutes // 60, minutes % 60)


def hm(dt):
    return "%02d:%02d" % (dt.hour, dt.minute)


def day_label(d):
    return "%s %d %s" % (DAYS[d.weekday()], d.day, MONTHS[d.month - 1])


def length(minutes):
    h, m = divmod(int(minutes), 60)
    return ("%d h %d min" % (h, m)) if h and m else ("%d h" % h if h else "%d min" % m)


def minutes_of(e):
    return int((e["end"] - e["start"]).total_seconds() // 60)


def when_label(e):
    a, b = e["start"], e["end"]
    if e["all_day"]:
        last = (b - timedelta(days=1)).date()
        return day_label(a.date()) + (", all day" if last <= a.date() else " to %s, all day" % day_label(last))
    if a.date() == b.date():
        return "%s, %s–%s" % (day_label(a.date()), hm(a), hm(b))
    return "%s %s to %s %s" % (day_label(a.date()), hm(a), day_label(b.date()), hm(b))


def span(e):
    return "all day" if e["all_day"] else "%s–%s" % (hm(e["start"]), hm(e["end"]))


def stamp(dt, all_day=False):
    return dt.strftime("%Y-%m-%d") if all_day else dt.strftime("%Y-%m-%dT%H:%M")


def day_link(d):
    """The only Calendar address the page links to: one day in the person's own calendar."""
    return "https://calendar.google.com/calendar/u/0/r/day/%d/%d/%d" % (d.year, d.month, d.day)


def meeting_link(u):
    """The address itself if it is https on an allowed meeting host, otherwise ''."""
    if not isinstance(u, str):
        return ""
    u = u.strip()
    m = MEETING_URL.fullmatch(u) if 0 < len(u) <= 600 else None
    if not m:
        return ""
    host = m.group(1)
    if host in MEETING_HOSTS or any(host == d or host.endswith("." + d) for d in MEETING_DOMAINS):
        return u
    return ""


def mask(account):
    account = clip(account, 80)
    if "@" not in account:
        return account
    user, dom = account.split("@", 1)
    return user[:1] + "•••@" + dom


def merged(intervals):
    out = []
    for s, e in sorted(intervals):
        if out and s <= out[-1][1]:
            out[-1][1] = max(out[-1][1], e)
        else:
            out.append([s, e])
    return out


def longest_gap(intervals, lo, hi):
    """The longest stretch between lo and hi that no interval covers."""
    best, t = 0, lo
    for s, e in merged(intervals):
        if s > t:
            best = max(best, min(s, hi) - t)
        t = max(t, e)
        if t >= hi:
            break
    return max(best, hi - t, 0)


def guests(n):
    return "nobody" if n <= 0 else ("1 guest" if n == 1 else "%d guests" % n)


def said(title):
    return '"%s"' % clip(title, 50)


def working(cal):
    w = cal.get("working_hours") if isinstance(cal.get("working_hours"), dict) else {}
    ws, we = parse_clock(w.get("start"), WORK_START), parse_clock(w.get("end"), WORK_END)
    if we <= ws:
        ws, we = WORK_START, WORK_END
    days = set()
    for d in w.get("days") if isinstance(w.get("days"), list) else []:
        if isinstance(d, int) and not isinstance(d, bool) and 1 <= d <= 7:
            days.add(d - 1)
        elif isinstance(d, str) and d.strip().lower() in DAY_KEYS:
            days.add(DAY_KEYS[d.strip().lower()])
    return ws, we, (days or set(WORK_DAYS)), bool(w)


def marked_free(raw):
    """Is this event marked Free? "busy": false, or the way Google's own data says it
    ("transparency": "transparent"). With neither given it counts as busy."""
    busy = raw.get("busy")
    return ((busy is not None and str(busy).lower() in ("false", "free", "transparent", "0", "none"))
            or str(raw.get("transparency") or "").lower() == "transparent")


def norm_calendars(cal):
    out = {}
    for c in cal.get("calendars") if isinstance(cal.get("calendars"), list) else []:
        key = c.get("key") if isinstance(c, dict) else None
        if not isinstance(key, str) or not SAFE_KEY.fullmatch(key) or key in out:
            continue
        # "mine" must be the JSON value true. A string such as "false" never counts as yes.
        out[key] = {"key": key, "name": clip(c.get("name") or key, 40), "mine": c.get("mine") is True,
                    "primary": c.get("primary") is True, "raw_name": c.get("name")}
    if not out:                       # no list given: everything is on the person's one calendar
        out["primary"] = {"key": "primary", "name": "Your calendar", "mine": True, "primary": True, "raw_name": "", "only": True}
    return out


def norm_event(raw, cals, default_cal, suspicious):
    start, timed, zulu = parse_when(raw.get("start"))
    if start is None:
        return None
    end = parse_when(raw.get("end"))[0]
    all_day = bool(raw.get("all_day")) or not timed
    if all_day:
        start = datetime(start.year, start.month, start.day)
        end = datetime(end.year, end.month, end.day) if end else None
        if end is None or end <= start:
            end = start + timedelta(days=1)
    elif end is None or end <= start:
        end = start + timedelta(minutes=30)
    end = min(end, start + timedelta(days=60))

    key = raw.get("calendar")
    c = default_cal if key is None or (default_cal or {}).get("only") else (cals.get(key) if isinstance(key, str) else None)
    rid = raw.get("id")
    rid = rid if isinstance(rid, str) else ("" if rid is None else str(rid))
    sid = raw.get("series_id")
    sid = sid if isinstance(sid, str) and SAFE_ID.fullmatch(sid) else ""

    att, counts = raw.get("attendees"), {"total": 0, "accepted": 0, "declined": 0, "tentative": 0, "needs_action": 0}
    answers_known = False             # without the answers, "nobody accepted" can't be said
    if isinstance(att, dict):
        for k in counts:
            counts[k] = num(att.get(k))
        counts["needs_action"] = max(counts["needs_action"], num(att.get("needsAction")))
        answers_known = any(k in att for k in ("accepted", "declined", "tentative", "needs_action", "needsAction"))
    elif isinstance(att, list):       # a raw guest list: only the answers are counted, nothing else is kept
        for a in att:
            r = (a.get("response") or a.get("responseStatus")) if isinstance(a, dict) else ""
            counts[RESPONSES.get(str(r or "").lower().replace(" ", ""), "needs_action")] += 1
        counts["total"], answers_known = len(att), True
    elif isinstance(att, (int, float)) and not isinstance(att, bool):
        counts["total"] = num(att)
    total = max(counts["total"], counts["accepted"] + counts["declined"] + counts["tentative"] + counts["needs_action"])
    resp = RESPONSES.get(str(raw.get("my_response") or "").lower().replace(" ", ""), "")

    place, video = raw.get("location"), raw.get("video")
    link = meeting_link(video) or meeting_link(place)
    text = clip(place, 80) or (link.split("/")[2] if link else clip(video, 80))
    tz = raw.get("timezone")
    for label, value in (("event title", raw.get("title")), ("organizer name", raw.get("organizer")),
                         ("place", place), ("video address", video)):
        note_suspicious(suspicious, label, value, skip_code=bool(meeting_link(value)))
    return {
        "id": rid, "ok": bool(SAFE_ID.fullmatch(rid)) and c is not None, "cal": c["key"] if c else "",
        "cal_name": c["name"] if c else "another calendar", "cal_mine": bool(c and c["mine"]),
        "cal_primary": bool(c and c["primary"]), "title": clip(raw.get("title")) or "(no title)",
        "has_title": bool(clip(raw.get("title"))), "title_key": re.sub(r"\s+", " ", str(raw.get("title") or "")).strip().casefold(),
        "start": start, "end": end, "all_day": all_day, "zulu": zulu,
        "kind": KINDS.get(str(raw.get("kind") or "").lower(), "event"),
        "mine": raw.get("mine") is True and bool(c and c["mine"]), "organizer": clip(raw.get("organizer"), 60),
        "resp": resp, "total": total, "others": max(0, total - 1),
        "others_yes": max(0, counts["accepted"] - (1 if resp == "accepted" else 0)), "answers_known": answers_known,
        "recurring": bool(raw.get("recurring")) or bool(sid), "series": sid,
        "series_started": parse_day(raw.get("series_started")), "series_ends": raw.get("series_ends"),
        "where": {"text": text, "link": link},
        "has_where": bool(text or link or raw.get("has_video")),
        "busy": not marked_free(raw),
        "cancelled": str(raw.get("status") or "").lower() == "cancelled",
        "tz": tz if isinstance(tz, str) and SAFE_TZ.fullmatch(tz) else "",
    }


def note_suspicious(found, where, value, skip_code=False):
    if not isinstance(value, str) or len(found) >= 20:
        return
    why = "reads like an instruction" if ORDER_RX.search(value) else (
        "looks like code" if CODE_RX.search(value) and not skip_code else "")
    row = {"where": where, "text": clip(value, 120), "why": why}
    if why and row not in found:
        found.append(row)


def plan_id(lists):
    """The report id: a fingerprint of every numbered fix. Change one and the id changes."""
    core = json.dumps(lists, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(core.encode("utf-8")).hexdigest()[:10]


def check_plan(lists):
    """Problems that make a plan unsafe to act on. Empty when every id, time and link is in a safe form."""
    if not isinstance(lists, dict) or set(lists) != set(LIST_ACTIONS):
        return ["the lists aren't the ones this skill writes"]
    bad = []
    for key, rows in lists.items():
        for i, r in enumerate(rows if isinstance(rows, list) else [None]):
            at = "%s %d" % (key, i + 1)
            if not isinstance(r, dict) or r.get("n") != i + 1 or isinstance(r.get("n"), bool):
                bad.append(at + ": not a numbered row")
                continue
            if r.get("action") not in LIST_ACTIONS[key]:
                bad.append(at + ": action %r" % (r.get("action"),))
            if key == "habits":
                if r.get("key") not in HABIT_KEYS:
                    bad.append(at + ": habit %r" % (r.get("key"),))
                continue
            for f in ("event", "series"):
                v = r.get(f)
                if not (v in (None, "") and (f == "series" or key == "breaks")) and not (isinstance(v, str) and SAFE_ID.fullmatch(v)):
                    bad.append(at + ": %s id %r" % (f, v))
            if not (isinstance(r.get("calendar"), str) and SAFE_KEY.fullmatch(r["calendar"])):
                bad.append(at + ": calendar %r" % (r.get("calendar"),))
            slot = r.get("slot") if isinstance(r.get("slot"), dict) else {}
            for label, v in (("start", r.get("start")), ("end", r.get("end")), ("new end", r.get("new_end", "2000-01-01")),
                             ("end date", r.get("end_on", "2000-01-01")), ("slot start", slot.get("start", "2000-01-01")),
                             ("slot end", slot.get("end", "2000-01-01"))):
                if not (isinstance(v, str) and SAFE_WHEN.fullmatch(v)):
                    bad.append(at + ": %s %r" % (label, v))
            if r.get("slot") is not None and not isinstance(r.get("slot"), dict):
                bad.append(at + ": slot")
            if key == "answer" and r.get("suggest") not in ("yes", "maybe", "no"):
                bad.append(at + ": suggestion %r" % (r.get("suggest"),))
            if key == "move" and not (r.get("suggest") == r.get("action") or (r.get("suggest") == "decline" and r.get("can_decline") is True
                                                                              and r.get("action") == "propose")):
                bad.append(at + ": suggestion %r" % (r.get("suggest"),))
            if not (isinstance(r.get("tells_count"), int) and r["tells_count"] >= 0):
                bad.append(at + ": who is told")
    return bad + check_links(lists)


def check_links(obj, at="report"):
    """Every link anywhere in the data must be one this script builds: a Calendar day, or an allowed meeting host."""
    bad = []
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k == "day_link":
                if not (isinstance(v, str) and DAY_LINK.fullmatch(v)):
                    bad.append("%s: day link %r" % (at, v))
            elif k == "link":
                if v != "" and meeting_link(v) != v:
                    bad.append("%s: meeting link %r" % (at, v))
            else:
                bad += check_links(v, at)
    elif isinstance(obj, list):
        for v in obj:
            bad += check_links(v, at)
    return bad


def build(cal):
    suspicious = []
    cals = norm_calendars(cal)
    for c in cals.values():
        note_suspicious(suspicious, "calendar name", c.pop("raw_name"))
    own = [c for c in cals.values() if c["mine"]]
    primary = next((c for c in own if c["primary"]), own[0] if own else None)
    cal_tz = cal.get("timezone") if isinstance(cal.get("timezone"), str) and SAFE_TZ.fullmatch(cal["timezone"]) else ""
    ws, we, wdays, hours_given = working(cal)
    now, has_clock, _ = parse_when(cal.get("today"))       # "today" may carry the time: what already ended is past
    today = now.date() if now else date.today()
    win = cal.get("window") if isinstance(cal.get("window"), dict) else {}
    d0 = parse_day(win.get("start")) or today - timedelta(days=PAST_DAYS)
    d1 = parse_day(win.get("end")) or today + timedelta(days=FUTURE_DAYS - 1)
    if not d0 <= today <= d1 or (d1 - d0).days > 120:
        d0, d1 = today - timedelta(days=PAST_DAYS), today + timedelta(days=FUTURE_DAYS - 1)
    one = timedelta(days=1)

    # ---- events: read, drop repeats of the same id, leave other people's calendars out
    events, seen, unreadable, odd_ids, elsewhere, zulu = [], {}, 0, 0, 0, 0
    for raw in (cal.get("events") if isinstance(cal.get("events"), list) else [])[:MAX_EVENTS]:
        e = norm_event(raw, cals, primary, suspicious) if isinstance(raw, dict) else None
        if e is None:
            unreadable += 1
            continue
        if not e["cal_mine"]:
            elsewhere += 1          # a colleague's or a subscribed calendar: not the person's time
            continue
        if not e["ok"]:
            odd_ids += 1
        zulu += 1 if e["zulu"] else 0
        old = seen.get(e["id"]) if e["ok"] else None
        if old is not None:         # one event seen through two of the person's calendars
            if e["cal_primary"] and not old["cal_primary"]:
                events[events.index(old)] = seen[e["id"]] = e
            continue
        if e["ok"]:
            seen[e["id"]] = e
        events.append(e)
    events.sort(key=lambda e: (e["start"], e["end"], e["cal"], e["id"]))

    days_off = set(d for d in (parse_day(x) for x in (cal.get("days_off") if isinstance(cal.get("days_off"), list) else [])) if d)
    out_of_office = []
    for e in events:
        if e["kind"] != "out_of_office" or e["cancelled"]:
            continue
        if e["all_day"]:
            d = e["start"].date()
            while d < e["end"].date() and len(days_off) < 400:
                days_off.add(d)
                d += one
        else:
            out_of_office.append(e)

    def working_day(d):
        return d.weekday() in wdays and d not in days_off

    def counted(e):
        return not e["cancelled"] and e["resp"] != "declined" and e["kind"] in ("event", "focus")

    def meeting(e):
        return counted(e) and e["kind"] == "event" and not e["all_day"] and e["others"] >= 1

    def fixable(e):
        return e["ok"] and e["kind"] == "event" and not e["cancelled"]

    def takes_time(e):
        """Does this event hold the person's time? Anything marked busy that they haven't declined:
        a meeting, a solo block, focus time, an Out of office block, an all-day event not marked
        Free. This is about where something new may go. It is asked apart from counted() and
        meeting(), which are about the meeting numbers and leave most of these out."""
        return e["busy"] and not e["cancelled"] and e["resp"] != "declined" and e["kind"] != "other"

    # ---- duplicates: same title and time, different events, on the person's own calendars
    groups, copies, dup_groups = {}, set(), []
    for e in events:
        if e["kind"] == "event" and not e["cancelled"] and e["has_title"]:
            groups.setdefault((e["title_key"], e["start"], e["end"], e["all_day"]), []).append(e)
    for g in groups.values():
        if len(g) < 2:
            continue
        g.sort(key=lambda e: (-e["others"], not e["cal_primary"], e["cal"], e["id"]))
        copies.update(id(x) for x in g[1:])
        dup_groups.append((g[0], g[1:]))

    # ---- what each day holds, in minutes from midnight. Two maps, asked separately: the time
    # that's taken (where nothing new is suggested), and the meetings (what the numbers count).
    # An all-day event runs from midnight to midnight on every date it covers; its end is the
    # day after its last one.
    taken_day, meet_day = {}, {}
    for e in events:
        takes, meets = takes_time(e), meeting(e)
        if id(e) in copies or not (takes or meets):
            continue
        d = max(e["start"].date(), d0 - one)
        last = min((e["end"] - timedelta(minutes=1)).date(), d1 + timedelta(days=14))
        while d <= last:
            midnight = datetime(d.year, d.month, d.day)
            s = max(0, int((e["start"] - midnight).total_seconds() // 60))
            en = min(1440, int((e["end"] - midnight).total_seconds() // 60))
            if en > s:
                if takes:
                    taken_day.setdefault(d, []).append((s, en, e))
                if meets:
                    meet_day.setdefault(d, []).append((s, en, e))
            d += one

    given = {}                      # days the agent only counted (the form for very busy calendars)
    for r in cal.get("days") if isinstance(cal.get("days"), list) else []:
        d = parse_day(r.get("date")) if isinstance(r, dict) else None
        if d and d0 <= d <= d1:
            given[d] = r
    stats, d = {}, d0
    while d <= d1:
        iv = sorted(meet_day.get(d, []), key=lambda x: (x[0], x[1]))
        runs, cur = [], None
        for s, en, e in iv:
            if cur and s - cur["end"] <= GAP:
                cur["end"], cur["longest"] = max(cur["end"], en), max(cur["longest"], en - s)
                cur["events"].append((s, en, e))
            else:
                cur = {"date": d, "start": s, "end": en, "longest": en - s, "events": [(s, en, e)]}
                runs.append(cur)
        runs = [r for r in runs if len(r["events"]) >= 2 and r["end"] - r["start"] > r["longest"]
                and (len(r["events"]) >= RUN_MEETINGS or r["end"] - r["start"] >= RUN_MINUTES)]
        st = {"minutes": sum(b - a for a, b in merged([(s, en) for s, en, _ in iv])), "count": len(iv), "runs": runs,
              "free": longest_gap([(s, en) for s, en, _ in iv], ws, we) if working_day(d) else None, "counted_run": None}
        g = given.get(d)
        if g:
            st["minutes"], st["count"] = num(g.get("meeting_minutes"), 1440), num(g.get("meetings"), 500)
            st["free"] = num(g.get("longest_free_minutes"), 1440) if working_day(d) and "longest_free_minutes" in g else None
            rm, rn = num(g.get("longest_run_minutes"), 1440), num(g.get("longest_run_meetings"), 500)
            if not runs and rn >= 2 and (rn >= RUN_MEETINGS or rm >= RUN_MINUTES):
                st["counted_run"] = (rn, rm)
        st["no_free"] = bool(working_day(d) and st["free"] is not None and st["free"] < FREE_BLOCK and st["minutes"] > 0)
        stats[d] = st
        d += one
    all_days = sorted(stats)
    future = [e for e in events if today <= e["start"].date() <= d1 and not (has_clock and e["end"] <= now)]

    # ---- clashes
    act = [e for e in future if counted(e) and e["busy"] and not e["all_day"] and id(e) not in copies
           and minutes_of(e) < 1440]
    pairs, clash_with = [], {}
    for i, a in enumerate(act):
        for b in act[i + 1:]:
            if b["start"] >= a["end"] or len(pairs) >= 2000:
                break
            if not (meeting(a) or meeting(b)) or "focus" in (a["kind"], b["kind"]):
                continue
            mins = int((min(a["end"], b["end"]) - b["start"]).total_seconds() // 60)
            pairs.append((a, b, mins))
            clash_with.setdefault(id(a), []).append(b)
            clash_with.setdefault(id(b), []).append(a)

    def outside_why(e):
        if e["all_day"]:
            return []
        d, s = e["start"].date(), e["start"].hour * 60 + e["start"].minute
        if d in days_off:
            return ["On a day you're off"]
        if any(o["start"] < e["end"] and e["start"] < o["end"] for o in out_of_office):
            return ["While you're out of office"]
        if d.weekday() not in wdays:
            return ["On a %s, outside your working days" % DAY_NAMES[d.weekday()]]
        why = []
        if s < ws:
            why.append("Starts %s, before your day starts at %s" % (hm(e["start"]), clock(ws)))
        if s + minutes_of(e) > we:
            why.append("Ends %s, after your day ends at %s" % (hm(e["end"]), clock(we)))
        return why

    def slip_why(e):
        if e["all_day"]:
            return []
        mins, h = minutes_of(e), e["start"].hour
        if mins >= 1440 and mins % 1440 == 0 and (h or e["start"].minute):
            return ["Looks like an all-day event that a time zone shifted: it runs %s to %s" % (hm(e["start"]), hm(e["end"]))]
        differs = bool(e["tz"] and cal_tz and e["tz"] != cal_tz)
        if (h >= ODD_FROM or h < ODD_UNTIL) and (differs or e["others"] >= 1):
            return ["Starts at %s. It was set in %s; your calendar is in %s" % (hm(e["start"]), e["tz"], cal_tz) if differs
                    else "Starts at %s, an odd hour for a meeting" % hm(e["start"])]
        return []

    reserved = {}

    def find_slot(e):
        """The first stretch of the same length, inside working hours, that nothing takes: no busy
        event of any kind, no time already offered to another meeting. Only on a day this script
        saw in full: not past the last day gathered, and not one the agent only counted (its
        meetings aren't listed, so where it's free isn't known). None when there is no such stretch."""
        need = minutes_of(e)
        if need > we - ws:
            return None
        d = max(e["start"].date(), today + one)
        for _ in range(10):
            if d > d1:
                break
            if working_day(d) and d not in given:
                taken = [(s, en) for s, en, x in taken_day.get(d, []) if x is not e] + reserved.get(d, [])
                t = ws
                for s, en in merged(taken) + [[we, we]]:
                    if min(s, we) - t >= need:
                        reserved.setdefault(d, []).append((t, t + need))
                        base = datetime(d.year, d.month, d.day)
                        return {"start": stamp(base + timedelta(minutes=t)), "end": stamp(base + timedelta(minutes=t + need)),
                                "when": "%s, %s–%s" % (day_label(d), clock(t), clock(t + need))}
                    t = max(t, -(-en // 30) * 30)      # start on the hour or the half hour
            d += one
        return None

    def row(e, **more):
        r = {"event": e["id"], "calendar": e["cal"], "calendar_name": e["cal_name"], "title": e["title"],
             "start": stamp(e["start"], e["all_day"]), "end": stamp(e["end"], e["all_day"]), "when": when_label(e),
             "mine": e["mine"], "organizer": "" if e["mine"] else e["organizer"], "guests": e["others"],
             "repeats": e["recurring"], "where": e["where"], "day_link": day_link(e["start"].date())}
        r.update(more)
        return r

    # ---- 1. invitations never answered
    answer = []
    for e in future:
        if not (fixable(e) and e["resp"] == "needs_action" and not e["mine"] and id(e) not in copies):
            continue
        firm = ["Clashes with %s (%s)" % (said(o["title"]), span(o)) for o in clash_with.get(id(e), []) if o["resp"] != "needs_action"]
        soft = ["Clashes with another invitation you haven't answered: %s (%s)" % (said(o["title"]), span(o))
                for o in clash_with.get(id(e), []) if o["resp"] == "needs_action"]
        away = outside_why(e)
        why = (firm + soft + away)[:4] or ["No clash on your calendar" + ("" if e["all_day"] else ", inside your working hours")]
        answer.append(row(e, action="answer", suggest="no" if firm or away else ("maybe" if soft else "yes"), why=why,
                          tells="the organizer", tells_count=1))

    # ---- 2. meetings to move: clashes, outside working hours, time-zone slips
    movers, outside_rows, slip_rows = {}, [], []

    def want_move(e, why):
        if fixable(e) and not (e["resp"] == "needs_action" and not e["mine"]) and id(e) not in copies:
            movers.setdefault(id(e), (e, []))[1].append(why)

    for a, b, mins in pairs:
        mover = min((a, b), key=lambda e: ({"needs_action": 0, "tentative": 1}.get(e["resp"], 2), not e["mine"],
                                           e["others"], -e["start"].toordinal(), e["id"]))
        other = b if mover is a else a
        want_move(mover, "Clashes with %s (%s)" % (said(other["title"]), span(other)))
    for e in future:
        if id(e) in copies or not counted(e):
            continue
        away = outside_why(e) if meeting(e) else []
        slip = slip_why(e)
        for w in away + ([] if minutes_of(e) >= 1440 else slip):     # a shifted all-day event is fixed by hand
            want_move(e, w)
        if away:
            outside_rows.append({"title": e["title"], "when": when_label(e), "why": "; ".join(away),
                                 "organizer": "you" if e["mine"] else e["organizer"], "day_link": day_link(e["start"].date())})
        if slip:
            slip_rows.append({"title": e["title"], "when": when_label(e), "why": slip[0],
                              "organizer": "you" if e["mine"] else e["organizer"], "day_link": day_link(e["start"].date())})
    move = []
    for e, why in sorted(movers.values(), key=lambda x: (x[0]["start"], x[0]["id"])):
        if not e["mine"] and e["total"] > MAX_PROPOSE:
            continue
        if e["mine"]:
            move.append(row(e, action="move", can_decline=False, suggest="move", why=why[:4], slot=find_slot(e),
                            tells=guests(e["others"]), tells_count=e["others"]))
        else:                       # someone else's meeting: propose a time to the organizer, or decline this one
            away = any(w.startswith(("On a day you're off", "While you're out of office")) for w in why)
            move.append(row(e, action="propose", can_decline=True, suggest="decline" if away else "propose", why=why[:4],
                            slot=find_slot(e), tells="the organizer", tells_count=1))

    # ---- 3. back-to-back runs: shorten what the person owns, or hold a break after the run
    shorten, breaks, run_rows, done = [], [], [], set()
    for d in all_days:
        if d < today:
            continue
        if stats[d]["counted_run"]:
            rn, rm = stats[d]["counted_run"]
            run_rows.append({"day": day_label(d), "span": "", "meetings": rn, "minutes": rm, "day_link": day_link(d)})
        for r in stats[d]["runs"]:
            evs, base = r["events"], datetime(d.year, d.month, d.day)
            run_rows.append({"day": day_label(d), "span": "%s–%s" % (clock(r["start"]), clock(r["end"])),
                             "meetings": len(evs), "minutes": r["end"] - r["start"], "day_link": day_link(d)})
            for i, (s, en, e) in enumerate(evs[:-1]):
                dur = minutes_of(e)
                if (e["mine"] and fixable(e) and id(e) not in done and e["start"].date() == d == e["end"].date()
                        and 25 <= dur <= 180 and 0 <= evs[i + 1][0] - en <= GAP):
                    done.add(id(e))
                    cut = 5 if dur <= 30 else 10
                    new_end = e["end"] - timedelta(minutes=cut)
                    shorten.append(row(e, action="shorten", new_end=stamp(new_end), tells=guests(e["others"]),
                                       tells_count=e["others"],
                                       why=["%d to %d minutes: it ends %s instead of %s, which leaves a gap before %s"
                                            % (dur, dur - cut, hm(new_end), hm(e["end"]), said(evs[i + 1][2]["title"]))]))
            end = r["end"]
            if primary and end + BREAK <= we and all(not (s < end + BREAK and end < en) for s, en, _ in taken_day.get(d, [])):
                a, b = base + timedelta(minutes=end), base + timedelta(minutes=end + BREAK)
                breaks.append({"calendar": primary["key"], "calendar_name": primary["name"], "title": "Break",
                               "start": stamp(a), "end": stamp(b), "when": "%s, %s–%s" % (day_label(d), hm(a), hm(b)),
                               "action": "add_break", "tells": "nobody", "tells_count": 0, "day_link": day_link(d),
                               "why": ["After %d meetings in a row (%s) with no gap" % (len(evs), length(r["end"] - r["start"]))]})

    # ---- 4. repeating meetings that have gone stale
    series = {}

    def of(sid):
        return series.setdefault(sid, {"past": [0, 0, 0], "future": [], "started": None, "ends": None})

    for e in events:
        if not (e["series"] and e["kind"] == "event" and not e["cancelled"]):
            continue
        s = of(e["series"])
        if e["start"].date() < today:
            s["past"][0] += 1
            s["past"][1] += 1 if e["resp"] == "declined" else 0
            s["past"][2] += 1 if e["resp"] == "accepted" else 0
        elif e["start"].date() <= d1:
            s["future"].append(e)
        s["started"] = s["started"] or e["series_started"]
        s["ends"] = s["ends"] if s["ends"] is not None else e["series_ends"]
    for r in cal.get("series") if isinstance(cal.get("series"), list) else []:
        sid = r.get("id") if isinstance(r, dict) else None
        if not isinstance(sid, str) or not SAFE_ID.fullmatch(sid):
            continue
        s = of(sid)
        s["started"] = parse_day(r.get("started")) or s["started"]
        s["ends"] = r.get("ends") if r.get("ends") is not None else s["ends"]
        recent = r.get("recent") if isinstance(r.get("recent"), dict) else {}
        if num(recent.get("held")) > s["past"][0]:
            s["past"] = [num(recent.get("held")), num(recent.get("declined")), num(recent.get("accepted"))]
        nxt = r.get("next") if isinstance(r.get("next"), dict) else None
        if nxt and not s["future"]:        # the series was only counted: its next meeting stands in for it
            e = norm_event({"id": nxt.get("id"), "start": nxt.get("start"), "end": nxt.get("end"), "calendar": r.get("calendar"),
                            "title": r.get("title"), "organizer": r.get("organizer"), "mine": r.get("mine"),
                            "attendees": r.get("attendees"), "my_response": r.get("my_response"), "recurring": True,
                            "series_id": sid}, cals, primary, suspicious)
            if e and e["cal_mine"] and today <= e["start"].date():
                s["future"].append(e)
    drop = []
    for sid, s in series.items():
        open_ones = [e for e in s["future"] if e["resp"] != "declined"]
        if not open_ones:
            continue                # nothing ahead, or the person has already declined what's ahead
        e = min(open_ones, key=lambda x: (x["start"], x["id"]))
        if not fixable(e) or e["others"] < 1:
            continue
        held, declined, accepted = s["past"]
        never_ends = isinstance(s["ends"], str) and s["ends"].strip().lower() in ("never", "none", "no end")
        age = (today - s["started"]).days if s["started"] else None
        why = []
        if declined >= STALE_DECLINES and accepted == 0:
            why.append("You declined %d of the last %d and accepted none" % (declined, held))
        if e["answers_known"] and e["others_yes"] == 0 and (held >= 1 or (age is not None and age >= 30)):
            why.append("None of the %s has accepted" % ("other guest" if e["others"] == 1 else "%d other guests" % e["others"]))
        if never_ends and age is not None and age >= STALE_DAYS:
            why.append("Running since %s %d with no end date" % (MONTHS[s["started"].month - 1], s["started"].year))
        if not why:
            continue
        if e["mine"]:               # the person's own series: give it an end date, never "delete this and following"
            drop.append(row(e, series=sid, action="end_series", why=why, tells=guests(e["others"]), tells_count=e["others"],
                            end_on=stamp(datetime(today.year, today.month, today.day) - one, True)))
        else:
            drop.append(row(e, series=sid, action="decline_series", why=why, tells="the organizer", tells_count=1))
    drop.sort(key=lambda r: (-len(r["why"]), r["start"], r["event"]))

    # ---- 5. several people, but no video link and no place
    links, no_link_rows, seen_links = [], [], set()
    for e in future:
        if not (meeting(e) and e["total"] >= SEVERAL and not e["has_where"] and id(e) not in copies):
            continue
        k = e["series"] or e["id"]
        if k in seen_links:
            continue
        seen_links.add(k)
        if e["mine"] and fixable(e):
            links.append(row(e, series=e["series"], action="add_video", scope="series" if e["series"] else "event",
                             why=["%d people, no video link and no place" % e["total"]], tells=guests(e["others"]),
                             tells_count=e["others"]))
        else:
            no_link_rows.append({"title": e["title"], "when": when_label(e), "organizer": "you" if e["mine"] else e["organizer"],
                                 "people": e["total"], "day_link": day_link(e["start"].date())})

    # ---- 6. duplicates the person owns
    duplicates, dup_rows = [], []
    for keep, extras in dup_groups:
        for x in extras:
            if not today <= x["start"].date() <= d1:
                continue
            if x["mine"] and x["others"] == 0 and fixable(x):
                duplicates.append(row(x, action="delete_event", keep_calendar=keep["cal_name"], tells="nobody", tells_count=0,
                                      why=["Same title and time as the one on %s, which stays" % keep["cal_name"]]))
            else:
                dup_rows.append({"title": x["title"], "when": when_label(x), "calendars": "%s and %s" % (keep["cal_name"], x["cal_name"]),
                                 "why": "It has guests" if x["others"] else ("It isn't yours to delete" if not x["mine"] else "Its id isn't in a plain form"),
                                 "day_link": day_link(x["start"].date())})

    # ---- days with no free block, and the standing habits that follow from the findings
    no_free_rows = [{"day": day_label(d), "minutes": stats[d]["minutes"], "meetings": stats[d]["count"],
                     "free": stats[d]["free"], "day_link": day_link(d)} for d in all_days if d >= today and stats[d]["no_free"]]
    habits = []

    def habit(key, title, why, how, agent):
        habits.append({"key": key, "title": title, "why": why, "how": how, "agent": agent, "action": "habit",
                       "tells": "nobody", "tells_count": 0})

    if run_rows:
        habit("speedy", "Turn on speedy meetings",
              "%d back-to-back run%s in the next four weeks." % (len(run_rows), "" if len(run_rows) == 1 else "s"),
              "Calendar > Settings > Event settings > Speedy meetings. New 30-minute meetings then end 5 minutes early and longer ones 10 minutes early.",
              "With your tick, in the browser. It only changes events you create from now on.")
    if outside_rows:
        habit("working_hours", "Set your working hours in Calendar",
              "%d meeting%s outside %s–%s or on a day off." % (len(outside_rows), "" if len(outside_rows) == 1 else "s", clock(ws), clock(we)),
              "Calendar > Settings > General > Working hours. People who invite you outside them are warned. Work and school accounts only.",
              "With your tick, using the hours this report used.")
    if no_free_rows:
        habit("focus", "Hold a two-hour block on your fullest days",
              "%d working day%s with no free block of two hours." % (len(no_free_rows), "" if len(no_free_rows) == 1 else "s"),
              "Create a repeating private event (or Focus time) on the days that fill up first. Nobody is invited.",
              "With your tick, once you say which days and hours.")
    if answer:
        habit("answer", "Answer invitations the day they arrive",
              "%d invitation%s waiting for an answer." % (len(answer), "" if len(answer) == 1 else "s"),
              "Open Calendar once a day and answer what's new. Organizers plan around answers, not silence.",
              "No. This one is yours.")
    if slip_rows:
        habit("timezone", "Show a second time zone",
              "%d event%s at an odd hour." % (len(slip_rows), "" if len(slip_rows) == 1 else "s"),
              "Calendar > Settings > Time zone > Display secondary time zone, and keep 'Ask to update my primary time zone to current location' on.",
              "With your tick, once you say which zone.")
    if links or no_link_rows:
        habit("auto_video", "Add a video link to every meeting you create",
              "%d meeting%s with several people and no link or place." % (len(links) + len(no_link_rows), "" if len(links) + len(no_link_rows) == 1 else "s"),
              "Calendar > Settings > Event settings > Automatically add Google Meet video conferences to events I create.",
              "With your tick, in the browser.")
    habit("review", "Look over your repeating meetings every three months",
          "%d repeating meeting%s worth a look right now." % (len(drop), "" if len(drop) == 1 else "s") if drop
          else "Repeating meetings outlive their purpose quietly.",
          "For each one: still useful, still the right people? If not, decline the series or give it an end date.",
          "It can rebuild this report whenever you ask.")

    lists, more = {}, {}
    for key, rows in (("answer", answer), ("drop", drop), ("move", move), ("shorten", shorten), ("breaks", breaks),
                      ("duplicates", duplicates), ("links", links), ("habits", habits)):
        more[key] = max(0, len(rows) - MAX_ROWS)
        lists[key] = [dict(r, n=i + 1) for i, r in enumerate(rows[:MAX_ROWS])]

    def table(rows):
        return {"rows": rows[:MAX_TABLE], "more": max(0, len(rows) - MAX_TABLE)}

    clash_rows = [{"when": when_label(b), "a": a["title"], "b": b["title"], "minutes": mins, "day_link": day_link(b["start"].date()),
                   "kind": "double booking" if a["start"] == b["start"] or mins >= 0.8 * min(minutes_of(a), minutes_of(b)) else "overlap"}
                  for a, b, mins in pairs]
    weeks = {}
    for d in all_days:
        w = weeks.setdefault(d - timedelta(days=d.weekday()), {"minutes": 0, "capacity": 0, "meetings": 0, "first": d, "last": d})
        w["minutes"] += stats[d]["minutes"]
        w["meetings"] += stats[d]["count"]
        w["capacity"] += (we - ws) if working_day(d) else 0
        w["last"] = d
    ahead = [d for d in all_days if d >= today]
    ahead_minutes = sum(stats[d]["minutes"] for d in ahead)
    notes = []
    if zulu:
        notes.append("%d times were written in UTC (ending in Z) and read as written; if this calendar isn't on UTC, they are shifted." % zulu)
    if odd_ids:
        notes.append("%d events had an id that isn't in a plain form, so no fix is offered for them." % odd_ids)
    if elsewhere:
        notes.append("%d events on calendars that aren't this person's own were left out." % elsewhere)
    if unreadable:
        notes.append("%d events had no readable start time and were skipped." % unreadable)
    report = {
        "account": mask(cal.get("account")), "timezone": cal_tz,
        "window": {"start": d0.isoformat(), "end": d1.isoformat(), "today": today.isoformat()},
        "sources": [clip(s, 40) for s in (cal.get("sources") if isinstance(cal.get("sources"), list) else [])][:6],
        "complete": bool(cal.get("complete", True)) and not given,
        "agent_notes": clip(cal.get("notes"), 400), "headline": clip(cal.get("headline"), 240),
        "highlights": [clip(h, 240) for h in (cal.get("highlights") if isinstance(cal.get("highlights"), list) else [])][:6],
        "working": {"start": clock(ws), "end": clock(we), "days": [DAYS[i] for i in sorted(wdays)], "given": hours_given},
        "calendars": [c["name"] for c in own][:8],
        "totals": {"meetings_ahead": sum(stats[d]["count"] for d in ahead), "minutes_ahead": ahead_minutes,
                   "weeks_ahead": max(1, round(len(ahead) / 7)), "clashes": len(pairs), "unanswered": len(answer),
                   "runs": len(run_rows), "no_free_days": len(no_free_rows), "outside": len(outside_rows),
                   "stale": len(drop), "duplicates": len(duplicates) + len(dup_rows), "slips": len(slip_rows),
                   "no_link": len(links) + len(no_link_rows), "events": len(events)},
        "daily": [[d.isoformat(), stats[d]["minutes"], stats[d]["count"], 0 if working_day(d) else 1,
                   1 if stats[d]["no_free"] else 0] for d in all_days],
        "weekly": [{"label": ("%d %s" % (w["first"].day, MONTHS[w["first"].month - 1])) + ("" if w["first"] == w["last"] else
                              " – %d %s" % (w["last"].day, MONTHS[w["last"].month - 1])), "minutes": w["minutes"], "capacity": w["capacity"], "meetings": w["meetings"],
                    "now": w["first"] <= today <= w["last"]} for _, w in sorted(weeks.items())],
        "lists": lists, "more": more,
        "tables": {"clashes": table(clash_rows), "runs": table(run_rows), "no_free": table(no_free_rows),
                   "outside": table(outside_rows), "slips": table(slip_rows), "no_link": table(no_link_rows),
                   "duplicates": table(dup_rows)},
        "suspicious": suspicious, "script_notes": notes,
        "thresholds": {"gap": GAP, "run_meetings": RUN_MEETINGS, "run_minutes": RUN_MINUTES, "free_block": FREE_BLOCK,
                       "break": BREAK, "several": SEVERAL, "stale_years": STALE_DAYS // 365},
        "generated": datetime.now().isoformat(timespec="seconds"),
    }
    # Last check before anything is written: every id, time and link is in a form this script
    # built or matched in full. A title or a place can never get this far as anything but text.
    bad = check_plan(lists) + check_links(report["tables"])
    if bad:
        sys.exit("refusing to write the report: something isn't in a safe form: %s" % "; ".join(bad[:3]))
    report["report_id"] = plan_id(lists)
    return report


def embed(obj):
    return (json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
            .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Build the calendar report page from calendar.json.")
    ap.add_argument("calendar")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write calendar-plan.json (default: next to --out)")
    a = ap.parse_args(argv)
    with open(a.calendar, encoding="utf-8") as f:
        cal = json.load(f)
    if not isinstance(cal, dict) or not (cal.get("events") or cal.get("days")):
        sys.exit("calendar.json has no 'events' or 'days'. See references/gathering.md.")
    report = build(cal)
    with open(TEMPLATE, encoding="utf-8") as f:
        html = f.read()
    if "__CALENDAR_DATA__" not in html:
        sys.exit("template is missing the __CALENDAR_DATA__ placeholder")
    plan_path = a.plan or os.path.join(os.path.dirname(os.path.abspath(a.out)), "calendar-plan.json")
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump({"schema": "calendar-fixer/plan@1", "report_id": report["report_id"], "today": report["window"]["today"],
                   "timezone": report["timezone"], "lists": report["lists"]}, f, ensure_ascii=False, indent=1)
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(html.replace("__CALENDAR_DATA__", embed(report)))
    t, n = report["totals"], {k: len(v) for k, v in report["lists"].items()}
    print("page: %s (%d KB)\nplan: %s  report_id=%s\n%d meetings ahead (%s), %d clashes, %d unanswered, %d back-to-back runs, "
          "%d days with no free two hours\nfixes offered: %d answers, %d to drop, %d to move, %d to shorten, %d breaks, "
          "%d duplicates, %d links, %d habits" % (
              os.path.abspath(a.out), os.path.getsize(a.out) // 1024, plan_path, report["report_id"], t["meetings_ahead"],
              length(t["minutes_ahead"]), t["clashes"], t["unanswered"], t["runs"], t["no_free_days"], n["answer"], n["drop"],
              n["move"], n["shorten"], n["breaks"], n["duplicates"], n["links"], n["habits"]))
    for line in report["script_notes"]:
        print("note: " + line)


if __name__ == "__main__":
    main()

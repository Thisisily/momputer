#!/usr/bin/env python3
"""Check every proposed name, then build the rename page and the plan file.

    python build_page.py survey.json --notes notes.json --out rename-plan.html

Writes two files: the page (--out) and rename-plan.json next to it (or --plan). The plan
holds one numbered rename per proposal that passed; apply_plan.py only ever acts on ids from it.

notes.json is written by the agent. It holds names and nothing else that reaches the disk:

    {
      "rule": "date-who-what",                # the naming rule the person picked (see names.py)
      "headline": "41 scans and downloads have names that say nothing; 33 now have a proposed name.",
      "insights": [{"title": "Your scanner names everything scan0001", "text": "..."}],   # mom's take, 3 to 6
      "names": {
        "12": {"name": "2024-03-14 Lanternfall invoice 1182.pdf", "why": "Invoice, dated in its header", "sure": true,
               "kind": "invoice"}
      },
      "kinds": {"14": "photo"},                # what a file is, for files you can tell but can't name
      "suspicious": [7]                        # documents whose text reads like instructions
    }

"highlights" (a list of plain sentences) still works and shows as insights without a title.
The page also works out, from the survey: how many names say something, say nothing, look like
a second copy, or were left alone; where the empty names come from; how many of each kind; when
the files arrived; and the years in the new names. The page only draws them.

A file is named by its survey id; the path comes from the survey, never from the notes. Each
name must pass names.why_bad(): a plain file name (no path), no reserved Windows name, no
characters Windows forbids, no invisible or direction-changing characters, no leading or
trailing dot or space, the same extension, a sensible length. A name that fails is listed on
the page as refused and gets no line in the plan. A clash gets " (2)". The file stays in its
folder.

Python 3.8+, standard library only. Reads folder listings; changes nothing but its two outputs.
"""
import argparse
import json
import os
import re
import sys
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
import names  # noqa: E402  (the one definition of what a name may be)
from safe_apply import norm, plan_fingerprint, xp  # noqa: E402  (one definition of the plan id)

TEMPLATE = os.path.join(HERE, "..", "assets", "rename-plan-template.html")


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def free_name(name, taken):
    """`name`, or the first "name (2)", "name (3)"... that nothing in `taken` (lower-cased names) has."""
    if name.lower() not in taken:
        return name
    stem, ext = os.path.splitext(name)
    for i in range(2, 1000):
        cand = "%s (%d)%s" % (stem, i, ext)
        if cand.lower() not in taken:
            return cand
    return None


def build(survey, notes):
    items = {it["id"]: it for it in survey.get("items", [])}
    rule = notes.get("rule")
    if rule is not None and rule not in names.RULES:
        sys.exit("notes.json: rule must be one of %s" % ", ".join(names.RULES))
    proposals = notes.get("names") or {}
    if not isinstance(proposals, dict):
        sys.exit("notes.json: names must be an object of survey id -> {name, why}")
    wanted = {}
    for key, value in proposals.items():
        try:
            item_id = int(key)
        except (TypeError, ValueError):
            sys.exit("notes.json names: %r is not a survey id" % (key,))
        if item_id not in items:
            sys.exit("notes.json names: #%s is not a file the survey listed. A name can only be proposed for a listed file." % key)
        wanted[item_id] = value if isinstance(value, dict) else {"name": value}
    try:
        suspicious = {int(i) for i in notes.get("suspicious", [])}
    except (TypeError, ValueError):
        sys.exit("notes.json: suspicious must be a list of survey ids")
    suspicious = {i for i in suspicious if i in items} | {i for i, it in items.items() if it.get("flag")}
    taken = {}  # folder -> lower-cased names in it now, plus the new names already planned
    actions, refused = [], []
    for item_id in sorted(wanted):
        it, prop = items[item_id], wanted[item_id]
        new, why = prop.get("name"), names.shown(str(prop.get("why", ""))[:160])
        problem = names.why_bad(it["name"], new, it["folder"])
        if not problem and item_id in suspicious:
            problem = "this file is listed as suspicious (its text reads like instructions), so it is left as it is"
        if problem:
            refused.append({"id": item_id, "old": names.shown(it["name"]), "proposed": names.shown(new), "why": problem})
            continue
        key = norm(it["folder"])
        if key not in taken:
            try:
                taken[key] = {n.lower() for n in os.listdir(xp(it["folder"]))}
            except OSError:
                refused.append({"id": item_id, "old": names.shown(it["name"]), "proposed": names.shown(new), "why": "its folder can't be listed any more"})
                continue
        final = free_name(new, taken[key])
        if final is None or names.why_bad(it["name"], final, it["folder"]):
            refused.append({"id": item_id, "old": names.shown(it["name"]), "proposed": names.shown(new), "why": "that name is taken, and so are its numbered variants"})
            continue
        taken[key].add(final.lower())
        fits = names.follows(rule, final) if rule else True
        sure = bool(prop.get("sure")) and fits
        a = {"id": item_id, "group": it["folder"], "op": "move", "from": it["path"], "to": os.path.join(it["folder"], final),
             "old": names.shown(it["name"]), "new": final, "bytes": it.get("bytes", 0), "why": why, "default": sure, "risk": "low" if sure else "medium"}
        if final != new:
            a["note"] = "Another file already has that name, so this one gets a number."
        elif not fits:
            a["note"] = "Doesn't follow the naming rule (%s)." % names.RULES[rule]["label"].lower()
        actions.append(a)
    for a in actions:  # belt and braces: a rename stays in its folder
        if norm(os.path.dirname(a["from"])) != norm(os.path.dirname(a["to"])) or os.path.basename(a["to"]) != a["new"]:
            sys.exit("#%d would leave its folder. Nothing was written." % a["id"])
    named = {a["id"] for a in actions} | {r["id"] for r in refused}
    return actions, refused, sorted(suspicious), [i for i in sorted(items) if i not in named and i not in suspicious], rule


def text(value, limit):
    """A line the agent wrote, as plain text of a sane length: one space between words, anything
    invisible spelled out, and no long number (an account, card or ID number has no place here)."""
    plain = " ".join(str(value if value is not None else "").split())
    return names.shown(names.LONG_DIGITS_RX.sub("[a long number]", plain), limit)


# The kind words the survey can read off a page, said the way a person files them
KIND_SAME = {"pay slip": "payslip", "quotation": "quote", "resume": "cv"}


def kind_word(value):
    """A kind of paper ("invoice", "letter"), lower-cased and short, or None."""
    if not isinstance(value, str) or names.LONG_DIGITS_RX.search(value):
        return None
    k = text(value, 40).lower().strip(" .")
    return k if any(ch.isalpha() for ch in k) and "\\u" not in k else None


def agent_notes(notes, items):
    """What the agent wrote for the page: the headline, mom's take, and the kind of each file it
    could tell. Text only, cut to size. An id the survey doesn't have stops the build."""
    insights = []
    given = notes.get("insights") or []
    if not isinstance(given, list):
        sys.exit("notes.json: insights must be a list of {\"title\", \"text\"}")
    for it in given[:8]:
        if isinstance(it, dict) and text(it.get("text"), 500):
            insights.append({"title": text(it.get("title"), 90), "text": text(it.get("text"), 500)})
        elif isinstance(it, str) and text(it, 500):
            insights.append({"title": "", "text": text(it, 500)})
    for h in (notes.get("highlights") or [])[:8]:
        if len(insights) < 8 and text(h, 300):
            insights.append({"title": "", "text": text(h, 300)})
    kinds = {}
    proposals = notes.get("names") if isinstance(notes.get("names"), dict) else {}
    loose = notes.get("kinds") or {}
    if not isinstance(loose, dict):
        sys.exit("notes.json: kinds must be an object of survey id -> kind, such as {\"14\": \"photo\"}")
    for source in (loose, {k: v.get("kind") for k, v in proposals.items() if isinstance(v, dict) and v.get("kind")}):
        for key, value in source.items():
            try:
                item_id = int(key)
            except (TypeError, ValueError):
                sys.exit("notes.json kinds: %r is not a survey id" % (key,))
            if item_id not in items:
                sys.exit("notes.json kinds: #%s is not a file the survey listed." % key)
            if kind_word(value):
                kinds[item_id] = kind_word(value)
    # "invoices" next to "invoice" is the same kind
    words = set(kinds.values())
    kinds = {i: (k[:-1] if k.endswith("s") and k[:-1] in words else k) for i, k in kinds.items()}
    return insights, kinds


def default_headline(counts, actions):
    nothing, docs = counts.get("say_nothing", 0), counts.get("documents", 0)
    if not nothing:
        return "All %d documents already have names that say what they are." % docs if docs else "There were no documents to look at."
    text = "%d of %d documents %s that say nothing." % (nothing, docs, "has a name" if nothing == 1 else "have names")
    if actions:
        text += " %d %s a new name waiting." % (len(actions), "has" if len(actions) == 1 else "now have")
    return text


# Why a name says nothing (names.SAYS_NOTHING's keys), as the page says it for many and for one, and
# where such names come from
WHY = {"scanner": ("Scanner defaults", "your scanner", "Scanner default"),
       "camera": ("Camera and phone defaults", "a phone or camera", "Camera or phone default"),
       "untitled": ("Untitled and New document", "programs that save as Untitled", "Untitled or New document"),
       "generic": ("Everyday words", "files saved under a single everyday word, such as document or download", "An everyday word"),
       "digits": ("Only digits", "names made only of digits", "Only digits"),
       "code": ("Long codes", "websites that name a file with a code", "A long code")}


def year_of(name):
    """The year in a new name: from a date written YYYY-MM(-DD) if there is one, else a year on its own."""
    stem = os.path.splitext(name)[0]
    m = re.search(r"(?<!\d)((?:19|20)\d{2})-[01]\d(?!\d)", stem) or re.search(r"(?<![\d-])((?:19|20)\d{2})(?![\d-])", stem)
    return int(m.group(1)) if m else None


def timeline(keys, unit, most):
    """Labels from the first key to the last with no gaps, at most `most` of them: the earlier ones
    go into one "<first" label. Keys are "2024" (unit "year") or "2024-03" (unit "month")."""
    if not keys:
        return []
    first, last = min(keys), max(keys)
    if unit == "year":
        labels = [str(y) for y in range(int(first), int(last) + 1)]
    else:
        y, m, labels = int(first[:4]), int(first[5:7]), []
        while "%04d-%02d" % (y, m) <= last:
            labels.append("%04d-%02d" % (y, m))
            y, m = (y + 1, 1) if m == 12 else (y, m + 1)
    if len(labels) > most:
        labels = ["<" + labels[-most + 1]] + labels[-most + 1:]
    return labels


def slot(key, labels):
    """The label a key is counted under."""
    if labels and labels[0].startswith("<") and key < labels[0][1:]:
        return 0
    return labels.index(key) if key in labels else None


def stats(survey, flagged, actions, kinds):
    """The numbers the page draws. Worked out here, so the page only draws them."""
    counts = survey.get("counts") or {}
    items = survey.get("items", [])
    susp = set(flagged)
    nothing = [it for it in items if it.get("why_key") != "named" and it["id"] not in susp]
    docs = counts.get("documents", 0)
    copies = counts.get("copies", 0)
    parts = {"nothing": len(nothing), "left": len(susp), "copies": copies}
    parts["clear"] = max(0, docs - parts["nothing"] - parts["left"] - copies)

    why = {}
    for it in items:
        key = it.get("why_key")
        if key in WHY:
            w = why.setdefault(key, {"key": key, "label": WHY[key][0], "source": WHY[key][1], "n": 0, "examples": []})
            w["n"] += 1
            if len(w["examples"]) < 2 and it["id"] not in susp and not str(it.get("flag", "")).startswith("its name"):
                w["examples"].append(names.shown(it["name"], 40))
    why = sorted(why.values(), key=lambda w: (-w["n"], w["label"]))

    # Kinds: what the agent said, else the kind word the survey read off the page
    pool = [it for it in items if it["id"] not in susp]
    kind_of = {}
    for it in pool:
        k = kinds.get(it["id"])
        if not k:
            words = (it.get("hints") or {}).get("kind_words") or []
            k = KIND_SAME.get(words[0], words[0]) if words else None
        if k:
            kind_of[it["id"]] = k
    renamed = {a["id"] for a in actions}
    tally = {}
    for i, k in kind_of.items():
        t = tally.setdefault(k, {"kind": k, "n": 0, "renamed": 0})
        t["n"] += 1
        t["renamed"] += i in renamed
    kind_list = sorted(tally.values(), key=lambda t: (-t["n"], t["kind"]))
    top = [t["kind"] for t in kind_list[:6]]

    # When the files arrived: the day each was last changed, which for a scan or a download is the day it came
    dates = [it["modified"] for it in pool if it.get("modified")]
    arrived = None
    if dates:
        years = sorted({d[:4] for d in dates})
        unit = "year" if int(years[-1]) - int(years[0]) >= 2 else "month"
        cut = 4 if unit == "year" else 7
        labels = timeline({d[:cut] for d in dates}, unit, 12 if unit == "year" else 18)
        lines = [{"kind": k, "values": [0] * len(labels)} for k in top] + [{"kind": None, "values": [0] * len(labels)}]
        for it in pool:
            if it.get("modified"):
                at = slot(it["modified"][:cut], labels)
                k = kind_of.get(it["id"])
                if at is not None:
                    lines[top.index(k) if k in top else -1]["values"][at] += 1
        arrived = {"unit": unit, "labels": labels, "series": [s for s in lines if any(s["values"])]}

    # The years in the new names: the papers' own dates, as the agent read them
    paper_years = [y for y in (year_of(a["new"]) for a in actions) if y]
    paper = None
    if paper_years:
        labels = timeline({str(y) for y in paper_years}, "year", 15)
        values = [0] * len(labels)
        for y in paper_years:
            values[slot(str(y), labels)] += 1
        paper = {"labels": labels, "values": values, "dated": len(paper_years), "total": len(actions)}

    # Each folder that was looked in: its documents, the names that say nothing, the new names
    folders = []
    for f in survey.get("folders", []):
        if f.get("skipped"):
            continue
        inside = lambda p: norm(p) == norm(f["path"]) or norm(p).startswith(norm(f["path"]) + "\\")  # noqa: E731
        folders.append({"path": f["path"], "documents": f.get("documents", 0),
                        "nothing": sum(1 for it in nothing if inside(it["folder"])),
                        "renames": sum(1 for a in actions if inside(a["group"]))})
    return {"parts": parts, "why": why, "kinds": kind_list, "unknown": len(pool) - len(kind_of), "pool": len(pool),
            "arrived": arrived, "paper": paper, "folders": folders, "sure": sum(1 for a in actions if a.get("default"))}, kind_of


def page_view(survey, flagged, unnamed, actions, kinds):
    """What the page shows about the files: names, sizes, dates, and the kind of paper. Never
    anything else read from inside one."""
    items = {it["id"]: it for it in survey.get("items", [])}
    numbers, kind_of = stats(survey, flagged, actions, kinds)

    def slim(i):
        it = items[i]
        why = WHY[it["why_key"]][2] if it.get("why_key") in WHY else it["why"]
        return {"id": i, "name": names.shown(it["name"]), "folder": it["folder"], "why": why, "modified": it.get("modified"), "bytes": it.get("bytes", 0)}

    return {"generated": survey.get("generated"), "profile": survey.get("profile") or "", "counts": survey.get("counts"),
            "folders": [{"path": f["path"], "skipped": f.get("skipped"), "listed": f.get("listed", 0), "documents": f.get("documents", 0),
                         "left": f.get("left", [])[:50]} for f in survey.get("folders", [])],
            "suspicious": [slim(i) for i in flagged], "unnamed": [slim(i) for i in unnamed][:500], "unnamed_count": len(unnamed),
            "stats": numbers}, {str(i): k for i, k in kind_of.items()}


def embed(obj):
    # Safe inside <script type="application/json">: no "<", ">" or "&" survive as raw characters,
    # and nothing outside ASCII does either (so no line separator, and no direction-changing
    # character, sits raw in the page source). JSON.parse gives every character back exactly.
    return (json.dumps(obj, ensure_ascii=True, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Check every proposed name, then build the rename page and rename-plan.json.")
    ap.add_argument("survey")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write the plan (default: rename-plan.json next to --out)")
    ap.add_argument("--notes", help="notes.json with the naming rule and the proposed names")
    args = ap.parse_args(argv)

    survey = load(args.survey)
    if not str(survey.get("schema", "")).startswith("rename-documents/survey@"):
        sys.exit("%s is not a rename-documents survey" % args.survey)
    notes = load(args.notes) if args.notes else {}
    actions, refused, flagged, unnamed, rule = build(survey, notes)
    insights, kinds = agent_notes(notes, {it["id"] for it in survey.get("items", [])})
    pid = plan_fingerprint(actions)
    plan_path = args.plan or os.path.join(os.path.dirname(os.path.abspath(args.out)), "rename-plan.json")
    plan = {"schema": "rename-documents/plan@1", "plan_id": pid, "generated": datetime.now().isoformat(timespec="seconds"),
            "survey": os.path.abspath(args.survey), "rule": rule, "folders": [f["path"] for f in survey.get("folders", []) if not f.get("skipped")],
            "actions": actions}
    with open(TEMPLATE, encoding="utf-8") as f:
        html = f.read()
    if "__RENAME_DATA__" not in html:
        sys.exit("template is missing the __RENAME_DATA__ placeholder")
    rules = {k: {x: v[x] for x in ("label", "example", "how")} for k, v in names.RULES.items()}
    view, kind_of = page_view(survey, flagged, unnamed, actions, kinds)
    # a refused name is shown as proposed, but an account or card number in it is not
    shown_refused = [dict(r, proposed=names.LONG_DIGITS_RX.sub("[a long number]", r["proposed"])) for r in refused]
    data = {"plan_id": pid, "actions": actions, "refused": shown_refused, "rule": rule, "rules": rules,
            "notes": {"headline": text(notes.get("headline"), 400) or default_headline(survey.get("counts") or {}, actions),
                      "insights": insights},
            "kinds": kind_of, "survey": view}
    html = html.replace("__RENAME_DATA__", embed(data))
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump(plan, f, ensure_ascii=False, indent=1)
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(html)
    print("page: %s (%d KB)\nplan: %s  plan_id=%s  %d renames, %d names refused, %d suspicious, %d without a name" % (
        os.path.abspath(args.out), os.path.getsize(args.out) // 1024, plan_path, pid, len(actions), len(refused), len(flagged), len(unnamed)))
    for r in refused:
        print("REFUSED #%d %s -> %s: %s" % (r["id"], r["old"], r["proposed"], r["why"]))
    print("folders (give each to apply_plan.py with --within, unless they are the default ones): %s" % "; ".join(plan["folders"]))


if __name__ == "__main__":
    main()

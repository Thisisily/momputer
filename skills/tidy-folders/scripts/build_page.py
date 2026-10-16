#!/usr/bin/env python3
"""Turn a survey (from survey.py) into the interactive tidy-plan page and a plan file.

    python build_page.py survey.json --out tidy-plan.html [--notes notes.json]

Writes two files: the page (--out) and plan.json next to it (or --plan). The plan
holds the final numbered actions; apply_plan.py only ever acts on ids from it.

notes.json is optional and is how the agent adds judgement without editing the
big survey file:

    {
      "headline": "Downloads holds 4,000 loose files; most are PDFs and screenshots.",
      "insights": [{"title": "Downloads is where things go to stay", "text": "..."}],   # mom's take, 3 to 6
      "places": [{"path": "C:\\\\Users\\\\me\\\\Documents\\\\Taxes", "note": "Your own tax folder."}],
      "naming_rules": ["..."],                    # replaces the default house style list
      "drop": [12, 40],                           # suggestion ids to leave out
      "retarget": {"15": "C:\\\\Users\\\\me\\\\Documents\\\\Tax\\\\2024\\\\p60.pdf"},  # new destinations
      "add": [{"op": "move", "from": "...", "to": "...", "why": "...", "risk": "low"}]
    }
"highlights" (a list of plain sentences) still works and shows as insights without a title.

The page draws what this script works out (the "dash" part of its data): the folders and the
kinds of file that fill them, files by the year they last changed, what lies loose where,
important papers loose and filed, and the facts at the top. The page itself only draws.

A file added for the Recycle Bin is looked at here (its size, when it last changed): the apply
step lets it go only if it is still that file. Copies of another file are the survey's to find;
it names the copy that stays, and the apply step reads both before one goes.

Python 3.8+, standard library only.
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
from safe_apply import file_facts, plan_fingerprint as plan_id  # noqa: E402  (one definition: the apply step recomputes it)

TEMPLATE = os.path.join(HERE, "..", "assets", "tidy-plan-template.html")
OPS = {"move", "recycle", "remove-empty-dir"}

# The kinds of file the page shows side by side, each with a light of its own colour (in this
# order: --c1 to --c6, then grey). The survey's types fold into them.
KINDS = [("documents", ("documents",)), ("pictures", ("images",)), ("videos", ("videos",)), ("music", ("audio",)),
         ("installers", ("installers", "archives")), ("code", ("code",)), ("other", ("other",))]
TYPE_KIND = {t: k for k, types in KINDS for t in types}
KIND_IDS = [k for k, _ in KINDS]
# Words in mom's take that name a kind, so the page can light it beside the insight
KIND_WORDS = [
    ("documents", r"documents?|papers?|pdfs?|contracts?|invoices?|tax|taxes|passports?|leases?|spreadsheets?|receipts?|payslips?"),
    ("pictures", r"pictures?|photos?|screenshots?|images?|scans?"),
    ("videos", r"videos?|recordings?|clips?|films?|movies?"),
    ("music", r"music|songs?|audio|mp3s?|podcasts?"),
    ("installers", r"installers?|setups?|zips?|archives?|disk images?|isos?"),
    ("code", r"code|projects?|repos?|repositories"),
]
KIND_WORDS_RX = [(k, re.compile(r"(?<!\w)(%s)(?!\w)" % p, re.I)) for k, p in KIND_WORDS]
MESSY = ("Desktop", "Downloads")   # with the profile folder and each drive's top: where loose files pile up
YEAR_OLD = (3, 4)                  # the survey's age buckets that are a year old or more


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def apply_notes(actions, notes):
    drop = {int(i) for i in notes.get("drop", [])}
    retarget = {int(k): v for k, v in (notes.get("retarget") or {}).items()}
    out = []
    for a in actions:
        if a["id"] in drop:
            continue
        if a["id"] in retarget:
            a = dict(a, to=retarget[a["id"]], why=a.get("why", "") + " (destination set by your assistant)")
        out.append(a)
    next_id = max([a["id"] for a in actions] + [0]) + 1
    for extra in notes.get("add", []):
        if extra.get("op") not in OPS or not extra.get("from") or (extra["op"] == "move" and not extra.get("to")):
            sys.exit("notes.json 'add' entries need op (%s), from, and to for moves: %r" % ("/".join(sorted(OPS)), extra))
        a = {"id": next_id, "group": extra.get("group", "custom"), "op": extra["op"], "from": extra["from"],
             "bytes": extra.get("bytes", 0), "why": extra.get("why", ""), "default": bool(extra.get("default", False)),
             "risk": extra.get("risk", "medium")}
        if extra.get("to"):
            a["to"] = extra["to"]
        if extra.get("is_dir") or os.path.isdir(extra["from"]):
            a["is_dir"] = True
        elif a["op"] == "recycle":
            # What the file is now: the apply step lets it go only if that is still so. One that
            # can't be read here gets no such line, and the apply step leaves it alone.
            try:
                a.update(file_facts(extra["from"]))
            except (OSError, ValueError, TypeError):
                pass
        out.append(a)
        next_id += 1
    return out


def text(v, limit):
    """A note from the agent, as plain text of a sane length (the page shows it with textContent)."""
    return " ".join(str(v or "").split())[:limit]


def normpath(p):
    return str(p or "").replace("/", "\\").rstrip("\\").lower()


def human(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%d %s" % (n, unit)) if unit in ("B", "KB") else ("%.1f %s" % (n, unit))
        n /= 1024.0


def kinds_of(by_type):
    """The survey's {type: {files, bytes}} folded into the page's kinds: {kind: [files, bytes]}."""
    out = {}
    for t, v in (by_type or {}).items():
        cur = out.setdefault(TYPE_KIND.get(t, "other"), [0, 0])
        cur[0] += int(v.get("files") or 0)
        cur[1] += int(v.get("bytes") or 0)
    return out


def area_name(a):
    """What a surveyed folder is called on the page."""
    kind, key, path = a.get("kind"), a.get("key") or "", a.get("path") or ""
    if kind in ("known", "profile"):
        return key
    if kind == "profile-root":
        return "Your profile folder"
    if kind == "drive-root":
        return "Top of " + path[:2]
    return path


def messy(a):
    return a.get("kind") in ("profile-root", "drive-root") or (a.get("kind") == "known" and a.get("key") in MESSY)


def local_path(p):
    """Only a plain path on a lettered drive is looked at here: nothing on a share is asked about."""
    return isinstance(p, str) and re.match(r"^[A-Za-z]:\\", p) is not None and "\\\\" not in p[2:]


def dashboard(survey, actions, notes):
    """Everything the page's charts draw, worked out here so the page only draws."""
    areas = [a for a in survey.get("areas") or [] if not a.get("program_like")]
    place_notes = {}
    for it in (notes.get("places") or [])[:60]:
        if isinstance(it, dict) and text(it.get("path"), 400) and text(it.get("note"), 300):
            place_notes[normpath(it["path"])] = text(it["note"], 300)

    folders, loose, totals, year_tally, loose_age = [], [], {}, {}, [0, 0, 0, 0, 0]
    for a in areas:
        ks = kinds_of(a.get("by_type"))
        for k, v in ks.items():
            cur = totals.setdefault(k, [0, 0])
            cur[0] += v[0]
            cur[1] += v[1]
        for y, h in (a.get("years") or {}).items():
            row = year_tally.setdefault(y, {})
            for k, v in kinds_of(h).items():
                row[k] = row.get(k, 0) + v[0]
        if a.get("files"):
            folders.append({"name": area_name(a), "path": a.get("path"), "kind": a.get("kind"), "bytes": a.get("bytes", 0),
                            "files": a.get("files", 0), "loose": a.get("loose_files", 0), "kinds": ks,
                            "partly": bool(a.get("truncated")), "note": place_notes.get(normpath(a.get("path")), "")})
        if messy(a) and a.get("loose_files"):
            lk = kinds_of(a.get("loose_by_type"))
            ages = (list(a.get("loose_age") or []) + [0] * 5)[:5]
            for i, n in enumerate(ages):
                loose_age[i] += n
            loose.append({"name": area_name(a), "path": a.get("path"), "files": a.get("loose_files", 0),
                          "bytes": sum(v[1] for v in lk.values()), "kinds": {k: v[0] for k, v in lk.items()},
                          "old": sum(ages[i] for i in YEAR_OLD), "note": place_notes.get(normpath(a.get("path")), "")})
    folders.sort(key=lambda f: -f["bytes"])
    loose.sort(key=lambda f: -f["files"])

    years = sorted(int(y) for y in year_tally if y != "older")
    older = "older" in year_tally
    since = int(survey.get("years_from") or (years[0] if years else datetime.now().year - 11))
    first = since if older else (years[0] if years else since)   # "older" sits just before the first year counted
    labels = (["older"] if older else []) + ([str(y) for y in range(first, years[-1] + 1)] if years else [])
    series = {k: [year_tally.get(lab, {}).get(k, 0) for lab in labels] for k in KIND_IDS}

    found, in_plan = {}, {}
    for d in survey.get("important_docs") or []:
        c = d.get("category") or "Other"
        found[c] = found.get(c, 0) + 1
    for a in actions:
        if a.get("group") == "important":
            c = a.get("category") or "Other"
            in_plan[c] = in_plan.get(c, 0) + 1
    papers = sorted(({"category": c, "found": max(found.get(c, 0), in_plan.get(c, 0)), "loose": in_plan.get(c, 0)}
                     for c in set(found) | set(in_plan)), key=lambda p: (-p["found"], p["category"]))

    def group_sum(g):
        rows = [a for a in actions if a.get("group") == g]
        return len(rows), sum(int(a.get("bytes") or 0) for a in rows)

    copies, copies_bytes = group_sum("duplicates")
    installers, installers_bytes = group_sum("installers")
    # Destination folders that aren't there yet: the page says the move makes them
    new_dirs = sorted({normpath(os.path.dirname(a["to"])) for a in actions
                       if a.get("op") == "move" and local_path(a.get("to")) and not os.path.isdir(os.path.dirname(a["to"]))})
    budget, every = survey.get("budget") or {}, survey.get("areas") or []
    return {
        "kinds": [{"id": k, "files": totals.get(k, [0, 0])[0], "bytes": totals.get(k, [0, 0])[1]} for k in KIND_IDS],
        "folders": folders[:60],
        "years": {"labels": labels, "since": since, "series": {k: v for k, v in series.items() if any(v)}},
        "loose": loose[:30],
        "loose_age": loose_age,
        "papers": papers,
        "new_dirs": new_dirs[:400],
        "facts": {
            "files": sum(int(a.get("files") or 0) for a in areas),
            "loose": sum(f["files"] for f in loose),
            "papers_loose": sum(in_plan.values()),
            "copies": copies, "copies_bytes": copies_bytes,
            "installers": installers, "installers_bytes": installers_bytes,
            "free_bytes": sum(int(a.get("bytes") or 0) for a in actions if a.get("op") == "recycle"),
            "out_of_place": len(actions),
            "out_folders": sum(1 for a in actions if a.get("is_dir")),
        },
        "left": {
            "protected": list(survey.get("protected") or [])[:40],
            "programs": int((survey.get("installed_programs") or {}).get("count") or 0),
            "projects": len(survey.get("projects") or []),
            "program_folders": [a["path"] for a in every if a.get("program_like")][:40],
            "partly": [a["path"] for a in every if a.get("truncated")][:40],
            "hash_truncated": bool(budget.get("hash_truncated")),
            "errors": int(budget.get("errors") or 0),
        },
    }


def kinds_named(s):
    """The kinds a sentence of mom's take is about, in the page's order (at most 3)."""
    return [k for k, rx in KIND_WORDS_RX if rx.search(s)][:3]


def agent_notes(notes):
    """Mom's take: the agent's insights, then any plain highlights, as text of a sane length."""
    out = []
    for it in (notes.get("insights") or [])[:8]:
        if isinstance(it, dict) and text(it.get("text"), 600):
            out.append({"title": text(it.get("title"), 90), "text": text(it.get("text"), 600)})
        elif isinstance(it, str) and text(it, 600):
            out.append({"title": "", "text": text(it, 600)})
    for h in (notes.get("highlights") or [])[:8]:
        if len(out) < 8 and text(h, 600):
            out.append({"title": "", "text": text(h, 600)})
    for it in out:
        it["kinds"] = kinds_named(it["title"] + " " + it["text"])
    return out


def default_headline(dash):
    f = dash["facts"]
    if not f["out_of_place"]:
        return "Everything is where it belongs. Nothing needs moving."
    line = "{:,} things could go in their place".format(f["out_of_place"])
    if dash["loose"]:
        top = dash["loose"][0]["name"]
        line += ", most of them from " + ("the " + top if top in MESSY else top[0].lower() + top[1:])
    line += "."
    if f["free_bytes"]:
        line += " Old copies and installers make up %s of it." % human(f["free_bytes"])
    return line


def page_view(survey):
    """The parts of the survey the page shows as they are (it never needs the full file lists)."""
    return {k: survey.get(k) for k in ("generated", "budget", "known_folders", "games", "naming_issues")}


def embed(obj):
    # Safe inside <script type="application/json">: no "<", ">" or "&" survive as raw characters
    return (json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
            .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Build the tidy-plan page and plan.json from a survey.")
    ap.add_argument("survey")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write the plan (default: plan.json next to --out)")
    ap.add_argument("--notes", help="optional notes.json with headline, insights, places, drop, retarget, add")
    args = ap.parse_args(argv)

    survey = load(args.survey)
    if not str(survey.get("schema", "")).startswith("tidy-folders/survey@"):
        sys.exit("%s is not a tidy-folders survey" % args.survey)
    notes = load(args.notes) if args.notes else {}
    actions = apply_notes(survey.get("suggestions", []), notes)
    pid = plan_id(actions)
    plan_path = args.plan or os.path.join(os.path.dirname(os.path.abspath(args.out)), "plan.json")
    plan = {"schema": "tidy-folders/plan@1", "plan_id": pid, "generated": datetime.now().isoformat(timespec="seconds"),
            "survey": os.path.abspath(args.survey), "actions": actions}
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump(plan, f, ensure_ascii=False, indent=1)

    with open(TEMPLATE, encoding="utf-8") as f:
        html = f.read()
    dash = dashboard(survey, actions, notes)
    rules = [text(r, 300) for r in notes.get("naming_rules") or [] if text(r, 300)][:12]
    data = {"plan_id": pid, "actions": actions, "survey": page_view(survey), "dash": dash,
            "notes": {"headline": text(notes.get("headline") or default_headline(dash), 300),
                      "insights": agent_notes(notes), "naming_rules": rules}}
    if "__TIDY_DATA__" not in html:
        sys.exit("template is missing the __TIDY_DATA__ placeholder")
    html = html.replace("__TIDY_DATA__", embed(data))
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(html)
    groups = {}
    for a in actions:
        groups[a["group"]] = groups.get(a["group"], 0) + 1
    print("page: %s (%d KB)\nplan: %s  plan_id=%s  %d actions %s" % (
        os.path.abspath(args.out), os.path.getsize(args.out) // 1024, plan_path, pid, len(actions), json.dumps(groups)))


if __name__ == "__main__":
    main()

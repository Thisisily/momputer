#!/usr/bin/env python3
"""Turn a survey (from survey.py) into the interactive tidy-plan page and a plan file.

    python build_page.py survey.json --out tidy-plan.html [--notes notes.json]

Writes two files: the page (--out) and plan.json next to it (or --plan). The plan
holds the final numbered actions; apply_plan.py only ever acts on ids from it.

notes.json is optional and is how the agent adds judgement without editing the
big survey file:

    {
      "headline": "Downloads holds 4,000 loose files; most are PDFs and screenshots.",
      "highlights": ["3 passport scans are on the Desktop", "..."],
      "naming_rules": ["..."],                    # replaces the default house style list
      "drop": [12, 40],                           # suggestion ids to leave out
      "retarget": {"15": "C:\\\\Users\\\\me\\\\Documents\\\\Tax\\\\2024\\\\p60.pdf"},  # new destinations
      "add": [{"op": "move", "from": "...", "to": "...", "why": "...", "risk": "low"}]
    }

A file added for the Recycle Bin is looked at here (its size, when it last changed): the apply
step lets it go only if it is still that file. Copies of another file are the survey's to find;
it names the copy that stays, and the apply step reads both before one goes.

Python 3.8+, standard library only.
"""
import argparse
import json
import os
import sys
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
from safe_apply import file_facts, plan_fingerprint as plan_id  # noqa: E402  (one definition: the apply step recomputes it)

TEMPLATE = os.path.join(HERE, "..", "assets", "tidy-plan-template.html")
OPS = {"move", "recycle", "remove-empty-dir"}


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


def page_view(survey):
    """The parts of the survey the page shows (it never needs the full file lists)."""
    keep = ("generated", "host", "budget", "known_folders", "targets", "drives", "areas", "games",
            "naming_issues", "protected", "projects")
    view = {k: survey.get(k) for k in keep}
    view["important_docs"] = [{"category": d["category"]} for d in survey.get("important_docs", [])]
    view["installed_programs"] = {"count": (survey.get("installed_programs") or {}).get("count", 0)}
    return view


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
    ap.add_argument("--notes", help="optional notes.json with headline, highlights, drop, retarget, add")
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
    data = {"plan_id": pid, "actions": actions, "notes": {k: notes[k] for k in ("headline", "highlights", "naming_rules")
                                                         if k in notes}, "survey": page_view(survey)}
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

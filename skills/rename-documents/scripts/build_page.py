#!/usr/bin/env python3
"""Check every proposed name, then build the rename page and the plan file.

    python build_page.py survey.json --notes notes.json --out rename-plan.html

Writes two files: the page (--out) and rename-plan.json next to it (or --plan). The plan
holds one numbered rename per proposal that passed; apply_plan.py only ever acts on ids from it.

notes.json is written by the agent. It holds names and nothing else that reaches the disk:

    {
      "rule": "date-who-what",                # the naming rule the person picked (see names.py)
      "headline": "41 scans and downloads have names that say nothing; 33 now have a proposed name.",
      "highlights": ["...", "..."],
      "names": {
        "12": {"name": "2024-03-14 Lanternfall invoice 1182.pdf", "why": "Invoice, dated in its header", "sure": true}
      },
      "suspicious": [7]                        # documents whose text reads like instructions
    }

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


def page_view(survey, flagged, unnamed):
    """What the page shows about the files: names, sizes and dates. Never anything read from inside one."""
    items = {it["id"]: it for it in survey.get("items", [])}

    def slim(i):
        it = items[i]
        return {"id": i, "name": names.shown(it["name"]), "folder": it["folder"], "why": it["why"], "modified": it.get("modified"), "bytes": it.get("bytes", 0)}

    return {"generated": survey.get("generated"), "profile": survey.get("profile") or "", "counts": survey.get("counts"),
            "folders": [{"path": f["path"], "skipped": f.get("skipped"), "listed": f.get("listed", 0), "documents": f.get("documents", 0),
                         "left": f.get("left", [])[:50]} for f in survey.get("folders", [])],
            "suspicious": [slim(i) for i in flagged], "unnamed": [slim(i) for i in unnamed][:500], "unnamed_count": len(unnamed)}


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
    data = {"plan_id": pid, "actions": actions, "refused": refused, "rule": rule, "rules": rules,
            "notes": {k: names.shown(str(notes[k]), 400) if k == "headline" else [names.shown(str(h), 300) for h in notes[k]][:8]
                      for k in ("headline", "highlights") if k in notes},
            "survey": page_view(survey, flagged, unnamed)}
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

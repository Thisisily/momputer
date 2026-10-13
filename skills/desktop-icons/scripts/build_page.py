#!/usr/bin/env python3
"""Turn a desktop survey (from survey.py) into the interactive plan page and a plan file.

    python build_page.py survey.json --out desktop-plan.html [--notes notes.json]

Writes two files: the page (--out) and desktop-plan.json next to it (or --plan). The plan
holds the final numbered actions; apply_plan.py only ever acts on ids from it.

notes.json is optional and is how the agent adds judgement:

    {
      "headline": "87 icons on a desktop that fits about 250; 9 shortcuts lead nowhere.",
      "highlights": ["...", "..."],
      "layout": "library",                # or "desktop": loose files go into a few folders on the Desktop instead
      "drop": [12, 40],                   # suggestion ids to leave out
      "retarget": {"15": "C:\\\\Users\\\\me\\\\Documents\\\\Taxes\\\\2024\\\\p60.pdf"},
      "add": [{"op": "move", "from": "<something on the Desktop>", "to": "...", "why": "..."}]
    }

Every action is checked before a plan is written. A source must be something the survey
listed on the person's own Desktop. A destination must be a plain full path inside the
Desktop or the person's Documents, Pictures, Videos, Music or Downloads. Anything else stops
the build: there is no plan to approve.

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
from safe_apply import norm, plan_fingerprint, under  # noqa: E402  (one definition of the plan id)

TEMPLATE = os.path.join(HERE, "..", "assets", "desktop-plan-template.html")
OPS = {"move", "recycle", "remove-empty-dir"}
LAYOUTS = {"library", "desktop"}
LIBRARIES = ("Documents", "Pictures", "Videos", "Music", "Downloads")


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def bad_destination(to, roots):
    """None if `to` is a plain full path inside one of `roots` (normalized), else why not."""
    if not isinstance(to, str) or not re.match(r"^[A-Za-z]:\\", to):
        return "not a full path on a drive letter"
    if "/" in to or os.path.normpath(to) != to or ":" in to[2:] or re.search(r'[<>"|?*\x00-\x1f]', to[2:]):
        return "not written as a plain path"
    n = norm(to)
    if not any(n != r and under(n, [r]) for r in roots):
        return "outside the Desktop and your Documents, Pictures, Videos, Music and Downloads"
    return None


def build_actions(survey, notes):
    desktop = survey["desktop"]["path"]
    kf = survey.get("known_folders") or {}
    roots = [norm(desktop)] + [norm(kf[k]) for k in LIBRARIES if kf.get(k)]
    on_desktop = {norm(it["path"]): it for it in survey.get("items", []) if it.get("where") == "desktop"}
    layout = notes.get("layout", "library")
    if layout not in LAYOUTS:
        sys.exit("notes.json: layout must be one of %s" % ", ".join(sorted(LAYOUTS)))
    names = survey.get("desktop_folders") or {}
    drop = {int(i) for i in notes.get("drop", [])}
    retarget = {int(k): v for k, v in (notes.get("retarget") or {}).items()}
    out = []
    for s in survey.get("suggestions", []):
        if s["id"] in drop:
            continue
        a = dict(s)
        if a["op"] == "move" and layout == "desktop":
            if a.get("is_dir"):
                continue  # a folder on the Desktop is already "in a folder on the Desktop"
            a["to"] = os.path.join(desktop, names.get(a.get("bucket"), "Other files"), os.path.basename(a["from"]))
        if a["id"] in retarget:
            if a["op"] != "move":
                sys.exit("notes.json retarget: #%d is not a move" % a["id"])
            a["to"] = retarget[a["id"]]
            a["why"] = a.get("why", "") + " (destination set by your assistant)"
        out.append(a)
    next_id = max([s["id"] for s in survey.get("suggestions", [])] + [0]) + 1
    for extra in notes.get("add", []):
        if not isinstance(extra, dict) or extra.get("op") not in OPS or not extra.get("from") or (extra["op"] == "move" and not extra.get("to")):
            sys.exit("notes.json 'add' entries need op (%s), from, and to for moves: %r" % ("/".join(sorted(OPS)), extra))
        item = on_desktop.get(norm(extra["from"])) if isinstance(extra["from"], str) else None
        if not item:
            sys.exit("notes.json 'add': %r is not something the survey found on the Desktop. This skill only moves "
                     "things that are on the person's own Desktop." % (extra["from"],))
        a = {"id": next_id, "item": item["id"], "group": "custom", "op": extra["op"], "from": item["path"], "bytes": item.get("bytes", 0),
             "why": str(extra.get("why", ""))[:300], "default": bool(extra.get("default", False)), "risk": "medium"}
        if extra["op"] == "move":
            a["to"] = extra["to"]
        if item["kind"] == "folder":
            a["is_dir"] = True
        out = [x for x in out if norm(x["from"]) != norm(item["path"])]  # the assistant's line replaces the survey's for that thing
        out.append(a)
        next_id += 1
    for a in out:
        if norm(a["from"]) not in on_desktop:
            sys.exit("#%d: the source is not on the Desktop: %r" % (a["id"], a["from"]))
        if a["op"] == "move":
            why = bad_destination(a.get("to"), roots)
            if why:
                sys.exit("#%d: the destination is %s: %r" % (a["id"], why, a.get("to")))
    return out, layout


def page_view(survey):
    """The parts of the survey the page shows. Shortcut targets and names, never anything read from a file."""
    keep = ("generated", "desktop", "public_desktop", "screen", "counts", "duplicates", "twice", "left_alone", "for_you",
            "library_folder")
    view = {k: survey.get(k) for k in keep}
    view["profile"] = (survey.get("known_folders") or {}).get("Profile") or ""
    items = []
    for it in survey.get("items", []):
        slim = {k: it.get(k) for k in ("id", "where", "name", "kind", "bytes", "modified", "idle_days")}
        slim["name"] = it.get("shown") or it.get("name")
        sc = it.get("shortcut")
        if sc:
            slim["shortcut"] = {k: sc.get(k) for k in ("status", "to", "leads_to", "pinned", "in_start", "flag") if sc.get(k)}
        items.append(slim)
    view["items"] = items
    return view


def embed(obj):
    # Safe inside <script type="application/json">: no "<", ">" or "&" survive as raw characters,
    # and nothing outside ASCII does either (so no line separator, and no direction-changing
    # character, sits raw in the page source). JSON.parse gives every character back exactly.
    return (json.dumps(obj, ensure_ascii=True, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Build the desktop plan page and desktop-plan.json from a survey.")
    ap.add_argument("survey")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write the plan (default: desktop-plan.json next to --out)")
    ap.add_argument("--notes", help="optional notes.json with headline, highlights, layout, drop, retarget, add")
    args = ap.parse_args(argv)

    survey = load(args.survey)
    if not str(survey.get("schema", "")).startswith("desktop-icons/survey@"):
        sys.exit("%s is not a desktop-icons survey" % args.survey)
    notes = load(args.notes) if args.notes else {}
    actions, layout = build_actions(survey, notes)
    listed = {it["id"]: it for it in survey.get("items", [])}
    for a in actions:
        item = listed.get(a.get("item")) or {}
        a["name"] = item.get("shown") or item.get("name") or os.path.basename(a["from"])  # what the page prints for it
        preview = (item.get("shortcut") or {}).get("args_preview")
        if preview:  # what a flagged shortcut would run, shown as plain text next to it
            a["detail"] = preview
    pid = plan_fingerprint(actions)
    plan_path = args.plan or os.path.join(os.path.dirname(os.path.abspath(args.out)), "desktop-plan.json")
    plan = {"schema": "desktop-icons/plan@1", "plan_id": pid, "generated": datetime.now().isoformat(timespec="seconds"),
            "survey": os.path.abspath(args.survey), "layout": layout, "actions": actions}
    with open(TEMPLATE, encoding="utf-8") as f:
        html = f.read()
    if "__DESKTOP_DATA__" not in html:
        sys.exit("template is missing the __DESKTOP_DATA__ placeholder")
    data = {"plan_id": pid, "layout": layout, "actions": actions,
            "notes": {k: notes[k] for k in ("headline", "highlights") if k in notes}, "survey": page_view(survey)}
    html = html.replace("__DESKTOP_DATA__", embed(data))
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump(plan, f, ensure_ascii=False, indent=1)
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(html)
    groups = {}
    for a in actions:
        groups[a["group"]] = groups.get(a["group"], 0) + 1
    print("page: %s (%d KB)\nplan: %s  plan_id=%s  layout=%s  %d actions %s" % (
        os.path.abspath(args.out), os.path.getsize(args.out) // 1024, plan_path, pid, layout, len(actions), json.dumps(groups)))


if __name__ == "__main__":
    main()

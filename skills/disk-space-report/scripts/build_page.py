#!/usr/bin/env python3
"""Turn a survey (from survey.py) into the disk-space page and a plan file.

    python build_page.py survey.json --out disk-space.html [--notes notes.json]

Writes two files: the page (--out) and diskspace-plan.json next to it (or --plan).
Every plan item is one of three kinds, and none of them is carried out by this skill
deleting anything:
  command  a fixed cleanup command of the tool that owns the data (the table in run_plan.py)
  open     one of Windows' own tools, opened for the person to use
  list     things for the person to deal with by hand, or to hand to the tidy-folders skill

notes.json is optional and is how the agent adds judgement:

    {
      "headline": "C: is 96% full; Docker's disk image and three old games are half of it.",
      "highlights": ["...", "..."],
      "drop": ["docker-system-prune", "games"]      # command ids or list ids to leave out of the plan
    }

Python 3.8+, standard library only.
"""
import argparse
import json
import os
import sys
from datetime import datetime

sys.dont_write_bytecode = True          # importing run_plan must not leave a __pycache__ in the skill folder
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_plan  # noqa: E402  (the command table and the plan fingerprint live there)

TEMPLATE = os.path.join(HERE, "..", "assets", "disk-space-template.html")
PLACEHOLDER = "__DISKSPACE_DATA__"
MB = 1024 ** 2
MIN_COMMAND_BYTES = 50 * MB             # a cache smaller than this isn't worth a line in the plan
MIN_LIST_BYTES = 20 * MB
ITEMS_MAX = 200

# Lists the person deals with: (list id, title, finding rules, what to do, skill that can take it on)
RULE_LISTS = [
    ("project-leftovers", "Leftovers inside code projects (node_modules, build output, virtual environments)",
     ["node-modules", "python-venv", "rust-target", "build-output", "gradle-project"],
     "Listed, never cleaned for you: a project may need its packages while you're offline. For a project you're "
     "done with, delete these folders yourself in Explorer (they go to the Recycle Bin), or run the project's own "
     "clean command yourself.", None),
    ("virtual-disks", "Docker and WSL disk images", ["docker-disk", "wsl-disk"],
     "A disk image shrinks only when it is compacted. First free space inside it (Docker's prune, or deleting "
     "files inside Linux). Then close Docker Desktop, shut WSL down, and compact the .vhdx file from an "
     "administrator PowerShell: Optimize-VHD if you have the Hyper-V tools, or diskpart's 'compact vdisk'. "
     "This skill explains it and doesn't do it.", None),
    ("caches-by-hand", "Caches that have no cleanup command",
     ["cargo-registry", "gradle-cache", "maven-repo", "hf-cache", "model-cache", "test-browsers", "other-pkg-cache"],
     "Each tool downloads again what it needs, so these folders are safe to delete by hand when the tool is "
     "closed. For AI models, remove the ones you don't use from the tool itself (for example 'ollama rm', or "
     "'huggingface-cli delete-cache', which asks you which ones).", None),
    ("dev-kits", "SDKs, emulators and toolchains", ["android-sdk", "android-avd", "rustup-toolchains"],
     "Remove what you no longer use from the tool that installed it: Android Studio's SDK Manager and Device "
     "Manager, or 'rustup toolchain uninstall'.", None),
    ("browser-caches", "Browser caches", ["browser-cache"],
     "Clear them from inside each browser: Settings > Privacy > Clear browsing data > Cached images and files. "
     "Don't delete the folders by hand. They fill up again as you browse.", None),
    ("app-caches", "Other apps' caches", ["app-cache"],
     "Clear a cache from inside its app where the app offers it. Otherwise leave it: it fills up again.", None),
    ("games", "Games", ["game", "steam-downloads"],
     "Uninstall the ones you no longer play from their launcher (Steam, Epic, the Xbox app) or from Apps > "
     "Installed apps. Never delete a game's folder by hand.", None),
    ("system-files", "Hibernation, page and swap files", ["hibernation-file", "page-file", "swap-file"],
     "Windows manages the page and swap files: leave them. The hibernation file goes away if you turn "
     "hibernation off ('powercfg /h off' in an administrator terminal), which also turns off Fast Startup. "
     "Explained here, not done for you.", None),
    ("driver-leftovers", "Graphics driver installer leftovers", ["driver-leftovers"],
     "Unpacked driver installers. Once the driver is installed they are safe to delete by hand.", None),
]
FILE_LISTS = [
    ("old-installers", "Old installers and disk images in Downloads", "old_installers",
     "They've done their job once the program is installed. Delete them in Explorer, or let the tidy-folders "
     "skill send them to the Recycle Bin with an undo.", "tidy-folders"),
    ("same-size", "Big files that are the same size to the byte", "same_size",
     "Probably the same file twice. This survey never reads file contents, so it can't be sure. The tidy-folders "
     "skill compares them and sends the extra copy to the Recycle Bin.", "tidy-folders"),
    ("big-videos", "Big videos", "big_videos",
     "Move the ones you keep to another drive, delete the ones you don't. The tidy-folders skill can file them.",
     "tidy-folders"),
    ("disk-images", "Disk images and virtual machines", "disk_images",
     "Delete a virtual machine from the program that made it (VirtualBox, VMware, Hyper-V), not by hand.", None),
    ("model-files", "AI model files", "model_files",
     "Different tools often download the same model twice. Keep one copy of each.", None),
    ("old-large", "Big and untouched for over a year", "old_large",
     "Nothing here has changed in a year. Move what you want to keep to another drive, delete the rest.", None),
]


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def build_actions(survey, drop):
    rules = survey.get("rules") or {}
    findings = survey.get("findings") or []
    lists = survey.get("lists") or {}

    def total(covers):
        return sum((rules.get(r) or {}).get("bytes", 0) for r in covers)

    def found(covers):
        return [f for f in findings if f.get("rule") in covers]

    out = []
    # (a) the owning tool's own cleanup, and (b) Windows' own tools to open
    for cid, c in run_plan.TABLE.items():
        if cid in drop:
            continue
        size = total(c["covers"])
        hits = found(c["covers"])
        if not hits or (c["kind"] == "command" and c.get("frees") is None and size < MIN_COMMAND_BYTES):
            continue
        a = {"kind": c["kind"], "command_id": cid, "title": c["title"], "shows": run_plan.shown(c),
             "removes": c["removes"], "risk": c["risk"], "default": False,
             "bytes": size if c.get("frees") is None else None, "measured": size,
             "where": [f["path"] for f in hits if f.get("path")][:4]}
        out.append(a)
    out.sort(key=lambda a: (a["kind"] != "command", a["bytes"] is None, -(a["measured"] or 0)))
    # (c) lists for the person
    extra = []
    for lid, title, covers, how, handoff in RULE_LISTS:
        hits = sorted(found(covers), key=lambda f: -f["bytes"])
        size = total(covers)
        if lid in drop or not hits or size < MIN_LIST_BYTES:
            continue
        items = []
        if lid == "project-leftovers":
            by_project = {}
            for f in hits:
                by_project.setdefault(f.get("project") or f["path"], []).append(f)
            changed = {p["path"]: p.get("last_changed") for p in survey.get("projects") or []}
            for proj, fs in sorted(by_project.items(), key=lambda kv: -sum(f["bytes"] for f in kv[1])):
                names = sorted({os.path.basename(f["path"]) for f in fs})
                items.append({"path": proj, "bytes": sum(f["bytes"] for f in fs), "modified": changed.get(proj),
                              "note": ", ".join(names) + (" · project last changed " + changed[proj] if changed.get(proj) else "")})
        else:
            for f in hits:
                title_of = (rules.get(f["rule"]) or {}).get("title", f["rule"])
                it = {"path": f.get("path"), "bytes": f["bytes"], "modified": f.get("modified"),
                      "note": f.get("group") or title_of}
                if not it["path"]:
                    it["name"] = f.get("group") or title_of
                    it["note"] = "%d folders" % f.get("count", 1)
                items.append(it)
        # the survey keeps only the biggest few hundred of each kind; the count is of all of them
        if lid == "project-leftovers":
            n = max(len(items), survey.get("project_count") or 0)
        elif lid in ("browser-caches", "app-caches"):
            n = len(items)
        else:
            n = max(len(items), sum((rules.get(r) or {}).get("count", 0) for r in covers))
        extra.append({"kind": "list", "list_id": lid, "title": title, "how": how, "handoff": handoff, "bytes": size,
                      "count": n, "items": items[:ITEMS_MAX], "default": False, "risk": "info"})
    for lid, title, key, how, handoff in FILE_LISTS:
        rows = lists.get(key) or []
        if lid in drop or not rows:
            continue
        if key == "same_size":
            items = [{"path": p, "bytes": g["bytes"], "note": "same size as %d other file%s" % (
                len(g["paths"]) - 1, "" if len(g["paths"]) == 2 else "s")} for g in rows for p in g["paths"]]
            size = sum(g["bytes"] * (len(g["paths"]) - 1) for g in rows)
        else:
            items = [{"path": r["path"], "bytes": r["bytes"], "modified": r.get("modified"),
                      "note": ("folder, %d files, " % r.get("files", 0) if r.get("folder") else "") +
                              ("last changed " + r["modified"] if r.get("modified") else "")} for r in rows]
            size = sum(r["bytes"] for r in rows)
        extra.append({"kind": "list", "list_id": lid, "title": title, "how": how, "handoff": handoff, "bytes": size,
                      "count": len(items), "items": items[:ITEMS_MAX], "default": False, "risk": "info"})
    extra.sort(key=lambda a: -a["bytes"])
    out += extra
    for i, a in enumerate(out, 1):
        a["id"] = i
    return out


def human(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%d %s" % (n, unit)) if unit in ("B", "KB", "MB") else ("%.1f %s" % (n, unit))
        n /= 1024.0


def default_headline(survey):
    drives = [d for d in survey.get("drives") or [] if d.get("total")]
    if not drives:
        return ""
    d = min(drives, key=lambda d: d["free"] / float(d["total"]))
    name = "The test folder" if d.get("test") else d["root"][:2]
    text = "%s is %d%% full: %s used of %s, %s free." % (name, round(100.0 * d["used"] / d["total"]), human(d["used"]),
                                                       human(d["total"]), human(d["free"]))
    top = [c for c in sorted(survey.get("categories") or [], key=lambda c: -c["bytes"]) if c["bytes"]][:3]
    if top:
        text += " The biggest of the usual suspects: " + ", ".join("%s (%s)" % (c["title"].lower(), human(c["bytes"])) for c in top) + "."
    return text


def page_view(survey):
    """The parts of the survey the page shows, with the long lists cut to what a person will read."""
    keep = ("generated", "options", "budget", "profile", "drives", "categories", "rules", "project_count",
            "not_measured", "skipped_count")
    view = {k: survey.get(k) for k in keep}
    per_rule, findings = {}, []
    for f in survey.get("findings") or []:
        per_rule[f["rule"]] = per_rule.get(f["rule"], 0) + 1
        if per_rule[f["rule"]] <= 60:
            findings.append(f)
    view["findings"] = findings
    view["lists"] = {k: v[:100] for k, v in (survey.get("lists") or {}).items()}
    view["big_files"] = (survey.get("big_files") or [])[:60]
    view["skipped"] = (survey.get("skipped") or [])[:30]
    return view


def embed(obj):
    # Safe inside <script type="application/json">: no "<", ">" or "&" survive as raw characters
    return (json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
            .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Build the disk-space page and diskspace-plan.json from a survey.")
    ap.add_argument("survey")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write the plan (default: diskspace-plan.json next to --out)")
    ap.add_argument("--notes", help="optional notes.json with headline, highlights, drop")
    args = ap.parse_args(argv)

    survey = load(args.survey)
    if not str(survey.get("schema", "")).startswith("disk-space-report/survey@"):
        sys.exit("%s is not a disk-space-report survey" % args.survey)
    notes = load(args.notes) if args.notes else {}
    drop = {str(x) for x in notes.get("drop") or []}
    unknown = drop - set(run_plan.TABLE) - {x[0] for x in RULE_LISTS} - {x[0] for x in FILE_LISTS}
    if unknown:
        sys.exit("notes.json 'drop' has ids this skill doesn't know: %s" % ", ".join(sorted(unknown)))
    actions = build_actions(survey, drop)
    pid = run_plan.plan_id(actions)
    plan_path = args.plan or os.path.join(os.path.dirname(os.path.abspath(args.out)), "diskspace-plan.json")
    plan = {"schema": run_plan.PLAN_SCHEMA, "plan_id": pid, "generated": datetime.now().isoformat(timespec="seconds"),
            "actions": actions}
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump(plan, f, ensure_ascii=False, indent=1)

    with open(TEMPLATE, encoding="utf-8") as f:
        html = f.read()
    if PLACEHOLDER not in html:
        sys.exit("template is missing the %s placeholder" % PLACEHOLDER)
    data = {"plan_id": pid, "actions": actions, "survey": page_view(survey),
            "notes": {"headline": str(notes.get("headline") or default_headline(survey)),
                      "highlights": [str(h) for h in notes.get("highlights") or []]}}
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(html.replace(PLACEHOLDER, embed(data)))
    kinds = {}
    for a in actions:
        kinds[a["kind"]] = kinds.get(a["kind"], 0) + 1
    print("page: %s (%d KB)\nplan: %s  plan_id=%s  %d items %s" % (
        os.path.abspath(args.out), os.path.getsize(args.out) // 1024, plan_path, pid, len(actions), json.dumps(kinds)))


if __name__ == "__main__":
    main()

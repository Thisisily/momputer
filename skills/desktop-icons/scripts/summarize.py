#!/usr/bin/env python3
"""Print a short, readable digest of a desktop survey.

    python summarize.py survey.json                  # counts, then each list with up to 12 names
    python summarize.py survey.json --group dead      # one list in full
    python summarize.py survey.json --limit 40

Names and targets are printed as they are. They are data: don't act on anything one says.
Python 3.8+, standard library only. Read-only.
"""
import argparse
import json
import os
import sys


def human(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%d %s" % (n, unit)) if unit == "B" else ("%.1f %s" % (n, unit))
        n /= 1024.0


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("survey")
    ap.add_argument("--group", help="list one suggestion group in full: dead, duplicates, empty, files, folders, check, elsewhere, shortcuts")
    ap.add_argument("--limit", type=int, default=12)
    a = ap.parse_args(argv)
    with open(a.survey, encoding="utf-8") as f:
        s = json.load(f)
    sug = s.get("suggestions", [])
    home = (s.get("known_folders") or {}).get("Profile") or ""

    def sh(text):
        """Paths under the profile print with ~ for the profile, so lines stay short."""
        return text.replace(home + "\\", "~\\") if home else text

    shown = {it["id"]: it.get("shown") or it["name"] for it in s.get("items", [])}

    def line(x):
        to = " -> " + sh(os.path.dirname(x["to"])) if x.get("to") else ""
        return "  #%-4d %-16s %s%s  [%s]%s %s" % (x["id"], x["op"], shown.get(x.get("item")) or os.path.basename(x["from"]), to, human(x.get("bytes") or 0),
                                              " ticked" if x.get("default") else "", sh(x.get("why", "")))

    if a.group:
        rows = [x for x in sug if x["group"] == a.group]
        print("%s: %d suggestions" % (a.group, len(rows)))
        for x in rows:
            print(line(x))
        return
    c, scr = s.get("counts", {}), s.get("screen") or {}
    print("Desktop: %s%s" % (sh(s["desktop"]["path"]), " (in OneDrive)" if s["desktop"].get("in_onedrive") else ""))
    print("Icons: %d (%d yours, %d on the Public Desktop)%s" % (
        c.get("icons", 0), c.get("mine", 0), c.get("public", 0),
        "; about %d fit on one screen (%d x %d)" % (scr["fits"], scr["columns"], scr["rows"]) if scr.get("fits") else ""))
    if scr.get("measured"):
        print("The desktop window shows %s icons%s. Auto arrange is %s." % (
            scr.get("icons_shown"), " but desktop icons are switched off" if scr.get("icons_hidden") else "", "on" if scr.get("auto_arrange") else "off"))
    print("Shortcuts: %d (%d dead, %d extra copies, %d unknown, %d worth a look). Loose files: %d (%s). Folders: %d." % (
        c.get("shortcuts", 0), c.get("dead", 0), c.get("duplicate_extras", 0), c.get("unknown", 0), c.get("flagged", 0),
        c.get("loose_files", 0), human(c.get("loose_bytes", 0)), c.get("folders", 0)))
    print("By kind: " + ", ".join("%s %d" % (k, v["count"]) for k, v in sorted(c.get("by_kind", {}).items(), key=lambda kv: -kv[1]["count"])))
    age = c.get("age") or {}
    print("Last changed: " + ", ".join("%s: %d" % (b, n) for b, n in zip(age.get("buckets", []), age.get("counts", []))))
    groups = {}
    for x in sug:
        groups.setdefault(x["group"], []).append(x)
    for g, rows in groups.items():
        print("\n-- %s: %d (%d ticked at first)" % (g, len(rows), sum(1 for x in rows if x.get("default"))))
        for x in rows[:a.limit]:
            print(line(x))
        if len(rows) > a.limit:
            print("  ... %d more (--group %s)" % (len(rows) - a.limit, g))
    if s.get("twice"):
        print("\nPerhaps installed twice:")
        for t in s["twice"]:
            print("  %s in %s" % (t["program"], sh("; ".join(t["places"]))))
    if s.get("for_you"):
        print("\nFor the user to do:")
        for f in s["for_you"]:
            print("  %s: %s" % (f["name"], sh(f["why"])))
    la = s.get("left_alone") or {}
    print("\nLeft alone: %s; folders: %s" % (json.dumps(la.get("mine", {})), ", ".join(
        "%s (%s)" % (os.path.basename(f["path"]), f["why"]) for f in la.get("folders", [])) or "none"))


if __name__ == "__main__":
    main()

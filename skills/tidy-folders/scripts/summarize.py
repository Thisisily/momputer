#!/usr/bin/env python3
"""Print a short, readable digest of survey.json so it never has to be read whole.

    python summarize.py survey.json                 # overview + 8 samples per group
    python summarize.py survey.json --group important --limit 60
    python summarize.py survey.json --section games  # games | important | naming | areas | big

Python 3.8+, standard library only. Read-only.
"""
import argparse
import json
import os
import sys


def human(n):
    if n is None:
        return "?"
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%d %s" % (n, unit)) if unit == "B" else ("%.1f %s" % (n, unit))
        n /= 1024.0


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("survey")
    ap.add_argument("--group", help="list one suggestion group in full (up to --limit)")
    ap.add_argument("--section", choices=["games", "important", "naming", "areas", "big"])
    ap.add_argument("--limit", type=int, default=8)
    a = ap.parse_args(argv)
    with open(a.survey, encoding="utf-8") as f:
        s = json.load(f)
    home = (s.get("known_folders") or {}).get("Profile") or ""

    def sh(p):
        if p and home and p.lower().startswith(home.lower() + "\\"):
            return "~" + p[len(home):]
        return p or ""

    sug = s.get("suggestions", [])
    if a.group:
        rows = [x for x in sug if x["group"] == a.group]
        print("%s: %d suggestions" % (a.group, len(rows)))
        for x in rows[:a.limit]:
            print("#%-5d %-7s %-6s %s%s  [%s] %s" % (x["id"], x["op"], x["risk"], sh(x["from"]),
                                                   " -> " + sh(x["to"]) if x.get("to") else "", human(x.get("bytes")), x["why"]))
        return
    b = s.get("budget", {})
    if not a.section:
        print("Surveyed %s entries in %ss%s. Errors: %s. Content peeks: %s. Hashed: %s%s." % (
            b.get("entries_seen"), b.get("seconds"), " (TRUNCATED)" if b.get("truncated") else "", b.get("errors"),
            b.get("content_peeks"), human(b.get("hashed_bytes")), " (duplicate check stopped early)" if b.get("hash_truncated") else ""))
        groups = {}
        for x in sug:
            g = groups.setdefault(x["group"], [0, 0, 0])
            g[0] += 1
            g[1] += x.get("bytes") or 0
            g[2] += 1 if x.get("default") else 0
        print("\nSuggestions (group: count, ticked by default, size):")
        for g, (n, by, d) in sorted(groups.items(), key=lambda kv: -kv[1][0]):
            print("  %-11s %6d  %6d ticked  %s" % (g, n, d, human(by)))
        for g in groups:
            print("\n-- %s (first %d)" % (g, a.limit))
            for x in [x for x in sug if x["group"] == g][:a.limit]:
                print("  #%-5d %s%s  | %s" % (x["id"], sh(x["from"]), " -> " + sh(x["to"]) if x.get("to") else "", x["why"]))
    if a.section in (None, "areas"):
        print("\nAreas (biggest first):")
        for ar in sorted(s.get("areas", []), key=lambda r: -r["bytes"])[:a.limit * 3]:
            flags = " TRUNCATED" if ar.get("truncated") else ""
            flags += " program-folder(left alone)" if ar.get("program_like") else ""
            print("  %-45s %10s %8d files %6d loose%s" % (sh(ar["path"])[:45], human(ar["bytes"]), ar["files"],
                                                         ar.get("loose_files", 0), flags))
    if a.section in (None, "games"):
        g = s.get("games", {})
        print("\nLaunchers:")
        for lch in g.get("launchers", []):
            print("  %-16s %4d games  libraries: %s" % (lch["name"], len(lch["games"]), "; ".join(lch["libraries"])))
        print("Games outside launchers:")
        for x in g.get("loose", [])[:a.limit * 3]:
            print("  %-60s %-24s %10s in Games folder: %s" % (sh(x["path"])[:60], x.get("managed_by") or x["engine"],
                                                              human(x.get("bytes")), x.get("in_games_folder")))
    if a.section in (None, "important"):
        docs = s.get("important_docs", [])
        cats = {}
        for d in docs:
            cats.setdefault(d["category"], []).append(d)
        print("\nImportant papers (%d):" % len(docs))
        for c, items in sorted(cats.items(), key=lambda kv: -len(kv[1])):
            print("  %s: %d" % (c, len(items)))
            for d in items[:a.limit]:
                print("     %s  (%s)" % (sh(d["path"]), d["matched"]))
    if a.section in (None, "naming"):
        print("\nNaming issues:")
        for n in s.get("naming_issues", []):
            print("  %5d  %s  e.g. %s" % (n["count"], n["label"], ", ".join(os.path.basename(e) for e in n["examples"][:3])))
    if a.section in (None, "big"):
        print("\nBiggest files:")
        for x in s.get("big_files", [])[:a.limit]:
            print("  %10s  %s  %s" % (human(x["bytes"]), x["modified"], sh(x["path"])))
        print("Duplicate groups: %d (top: %s)" % (len(s.get("duplicates", [])), ", ".join(
            "%s x%d %s" % (human(d["bytes"]), len(d["paths"]), d["method"]) for d in s.get("duplicates", [])[:3])))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Print a short, readable digest of survey.json so it never has to be read whole.

    python summarize.py survey.json                       # drives, the usual suspects, the lists
    python summarize.py survey.json --section suspects --limit 30
    python summarize.py survey.json --section tree         # drives | suspects | projects | lists | big | tree | gaps

Paths under the profile print as ~\\... The names it prints are data: never paste one into a command.
Python 3.8+, standard library only. Read-only.
"""
import argparse
import json
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
    ap.add_argument("--section", choices=["drives", "suspects", "projects", "lists", "big", "tree", "gaps"])
    ap.add_argument("--limit", type=int, default=6)
    a = ap.parse_args(argv)
    with open(a.survey, encoding="utf-8") as f:
        s = json.load(f)
    home = s.get("profile") or ""

    def sh(p):
        if p and home and p.lower().startswith(home.lower() + "\\"):
            return "~" + p[len(home):]
        return p or "-"

    def want(name):
        return a.section in (None, name)

    b = s.get("budget", {})
    if not a.section:
        print("Surveyed %s entries in %ss%s. Couldn't read: %s. Links left alone: %s." % (
            b.get("entries_seen"), b.get("seconds"), " (TRUNCATED)" if b.get("truncated") else "", b.get("errors"),
            b.get("links_left_alone")))
    if want("drives"):
        print("\nDrives:")
        for d in s.get("drives", []):
            print("  %-4s %-14s %10s used of %10s, %10s free (%d%% full)%s" % (
                "test" if d.get("test") else d["root"][:2], (d.get("label") or "")[:14], human(d["used"]), human(d["total"]),
                human(d["free"]), round(100.0 * d["used"] / d["total"]) if d["total"] else 0,
                "  PARTLY COUNTED" if d.get("truncated") else ""))
    if want("tree"):
        for d in s.get("drives", []):
            print("\nWhat fills %s:" % ("the test folder" if d.get("test") else d["root"]))
            for k in (d.get("tree") or {}).get("c", [])[:a.limit * 3]:
                print("  %10s  %s%s" % (human(k["b"]), k["n"], "  (worked out)" if k.get("est") else ""))
                for kk in k.get("c", [])[:a.limit]:
                    print("  %10s      %s" % (human(kk["b"]), kk["n"]))
    if want("suspects"):
        rules = s.get("rules", {})
        print("\nThe usual suspects (biggest first):")
        for c in sorted(s.get("categories", []), key=lambda c: -c["bytes"]):
            if not c["bytes"]:
                continue
            print("  %-46s %10s  %d places" % (c["title"], human(c["bytes"]), c["count"]))
            for rid, r in sorted(rules.items(), key=lambda kv: -kv[1]["bytes"]):
                if r["category"] != c["id"]:
                    continue
                print("      %-42s %10s  x%d" % (r["title"][:42], human(r["bytes"]), r["count"]))
                if a.section:
                    for f in [f for f in s.get("findings", []) if f["rule"] == rid][:a.limit]:
                        print("          %10s  %s  %s" % (human(f["bytes"]), sh(f.get("path")) if f.get("path") else f.get("group", "-"),
                                                          f.get("modified") or ""))
    if want("projects"):
        print("\nCode projects with leftovers: %d (listed, never cleaned automatically)" % s.get("project_count", 0))
        for p in s.get("projects", [])[:a.limit * 2]:
            print("  %10s of leftovers, last changed %s  %s" % (human(p["leftover_bytes"]), p.get("last_changed"), sh(p["path"])))
    if want("lists"):
        print("\nLists:")
        for key, rows in (s.get("lists") or {}).items():
            total = sum(r["bytes"] * (len(r["paths"]) - 1 if "paths" in r else 1) for r in rows)
            print("  %-16s %4d  %10s" % (key, len(rows), human(total)))
            for r in rows[:a.limit]:
                if "paths" in r:
                    print("      %10s  x%d  %s" % (human(r["bytes"]), len(r["paths"]), "  =  ".join(sh(p) for p in r["paths"][:3])))
                else:
                    print("      %10s  %s  %s%s" % (human(r["bytes"]), r.get("modified"), sh(r["path"]),
                                                   "  (folder)" if r.get("folder") else ""))
    if want("big"):
        print("\nBiggest files:")
        for x in s.get("big_files", [])[:a.limit * 2]:
            print("  %10s  %s  %s%s" % (human(x["bytes"]), x.get("modified"), sh(x["path"]),
                                       "  [in %s]" % x["in"] if x.get("in") else ""))
    if want("gaps"):
        print("\nNot measured:")
        for m in s.get("not_measured", []):
            print("  %s: %s" % (m["what"], m["why"]))
        if a.section:
            for k in s.get("skipped", [])[:a.limit * 5]:
                print("  couldn't open (%s): %s" % (k["why"], sh(k["path"])))


if __name__ == "__main__":
    main()

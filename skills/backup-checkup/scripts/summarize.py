#!/usr/bin/env python3
"""Print a short, readable digest of a backup-checkup survey so it never has to be read whole.

    python summarize.py survey.json                       # the verdict, folders, who keeps copies, drives, code
    python summarize.py survey.json --section repos --limit 40
                                                           # folders | keepers | drives | repos | lose

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
    ap.add_argument("--section", choices=["folders", "keepers", "drives", "repos", "lose"])
    ap.add_argument("--limit", type=int, default=12)
    a = ap.parse_args(argv)
    with open(a.survey, encoding="utf-8") as f:
        s = json.load(f)
    home = s.get("profile") or ""

    def sh(p):
        if p and home and p.lower() == home.lower():
            return "~ (loose files)"
        if p and home and p.lower().startswith(home.lower() + "\\"):
            return "~" + p[len(home):]
        return p or "-"

    def want(name):
        return a.section in (None, name)

    b, sm = s.get("budget", {}), s.get("summary", {})
    if not a.section:
        print("%s." % sm.get("headline"))
        print("Looked at %s entries in %ss%s. No second copy: %s, old copy: %s, needs a look: %s, fine: %s. At risk: %s." % (
            b.get("entries_seen"), b.get("seconds"), " (TRUNCATED)" if b.get("truncated") else "", sm.get("none"), sm.get("old"),
            (sm.get("check") or 0) + (sm.get("partial") or 0), sm.get("ok"), human(sm.get("bytes_at_risk"))))
        since = [p for p in s.get("places", []) if p.get("counts") and (p.get("changed_since_copy") or {}).get("bytes")]
        if since:
            print("Changed since its last dated copy: " + "; ".join("%s %s in %d files since %s" % (
                sh(p["label"]), human(p["changed_since_copy"]["bytes"]), p["changed_since_copy"]["files"], p["changed_since_copy"]["since"])
                for p in sorted(since, key=lambda p: -p["changed_since_copy"]["bytes"])[:a.limit]) + ".")
        gen = str(s.get("generated") or "")[:7]
        if gen:
            y, m = int(gen[:4]), int(gen[5:7])
            recent = ["%04d-%02d" % ((y, m - i) if m - i > 0 else (y - 1, m - i + 12)) for i in range(12)]
            mine = {}
            for p in s.get("places", []):
                if p.get("counts"):
                    mine[p["verdict"]] = mine.get(p["verdict"], 0) + sum(((p.get("months") or {}).get(k) or {}).get("bytes", 0) for k in recent)
            if any(mine.values()):
                print("Changed in the last 12 months, by the folder's verdict: %s." % ", ".join(
                    "%s %s" % (v, human(n)) for v, n in sorted(mine.items(), key=lambda t: -t[1]) if n))
    if want("folders"):
        print("\nFolders (worst first):")
        order = {"none": 0, "old": 1, "check": 2, "partial": 3, "ok": 4}
        for p in sorted(s.get("places", []), key=lambda p: (not p["counts"], order[p["verdict"]], -p["level"], -p["own_bytes"]))[:a.limit * 3]:
            guard = "; ".join("%s (%s%s)" % (x["by"], x["state"], ", %s days" % x["age_days"] if x.get("age_days") is not None else "")
                              for x in p["protection"]) or "nothing"
            print("  %-8s %10s  %-30s %s%s  <- %s" % (p["verdict"] if p["counts"] else "(skip)", human(p["own_bytes"]), sh(p["path"])[:30],
                                                     p["level_words"], "  [%d repos, %d at risk]" % (p["repos"], p["repos_at_risk"]) if p["repos"] else "",
                                                     guard))
    if want("lose"):
        print("\nWould be lost (what can't be replaced first):")
        for x in s.get("lose", [])[:a.limit * 2]:
            print("  %10s  %-8s %-28s %s; %s" % (human(x["bytes"]), x["verdict"], sh(x["path"])[:28], x["level_words"], x["what"]))
    if want("keepers"):
        print("\nWho keeps copies:")
        for p in s.get("protectors", []):
            print("  %-30s %-10s %s%s%s" % (p["name"], p["state"], "running " if p.get("running") else "NOT RUNNING " if p.get("running") is False else "",
                                           "last copy %s " % p["last_copy"] if p.get("last_copy") else "",
                                           "(coverage unknown)" if p.get("coverage") == "unknown" else ""))
            for c in p.get("covers", [])[:a.limit]:
                print("        covers %s" % sh(c["path"]))
    if want("drives"):
        print("\nOther drives:")
        for d in s.get("drives", []):
            if d.get("system"):
                continue
            where = {True: "same disk as Windows (doesn't count)", False: "separate disk", None: "unknown"}[d.get("same_disk")]
            print("  %-4s %-10s %-22s %s free; looks like backups: %d" % (d["root"][:3] if not d.get("test") else "test", d["type"], where,
                                                                         human(d.get("free")), len(d.get("backups", []))))
            for bk in d.get("backups", [])[:a.limit]:
                print("        %s: %s, newest file %s" % (bk["name"], bk["kind"], bk.get("newest")))
    if want("repos"):
        st = sm.get("repo_states") or {}
        print("\nGit repositories: %s %s (read from .git files; git was never run)" % (sm.get("repos"), json.dumps(st)))
        for r in [r for r in s.get("repos", []) if r["state"] not in ("ok", "empty")][:a.limit * 2]:
            bits = ["%s:%s%s" % (br["name"], br["state"], "(%s)" % br["ahead"] if br.get("ahead") else "") for br in r["branches"]
                    if br["state"] not in ("in-step", "behind")]
            print("  #%-3d %-13s %-30s last commit %s%s%s %s" % (r["id"], r["state"], sh(r["path"])[:30], r.get("last_commit"),
                                                             ", %d stashes" % r["stashes"] if r.get("stashes") else "",
                                                             ", files changed since" if r.get("changed_since_commit") else "", " ".join(bits)[:80]))
    if not a.section and s.get("not_measured"):
        print("\nRan out of time before measuring %d folders." % len(s["not_measured"]))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Turn a survey (from survey.py) into the backup-checkup page and a plan file.

    python build_page.py survey.json --out backup-checkup.html [--notes notes.json]

Writes two files: the page (--out) and backup-plan.json next to it (or --plan).
A plan item is one of two kinds, and this skill carries out neither by copying anything:
  open    one of Windows' own pages (the table in run_plan.py), opened for the person to use
  guide   something the person does: push a repository, copy a folder, check a program

notes.json is optional and is how the agent adds judgement, what a backup tool can't tell:

    {
      "headline": "3 of your 7 folders have no second copy, and your photos are one of them.",
      "insights": [{"title": "Your photos live only on C:", "text": "..."}],   # mom's take, 3 to 6
      "places": [{"path": "C:\\\\Users\\\\you\\\\Pictures", "note": "12 years of family photos."}],
      "drives": {"D": "Your old laptop's disk: the best home for a copy."},
      "drop": ["open-file-history", "repos"]     # open-windows-backup, open-file-history, copies, repos, checks
    }
"highlights" (a list of plain sentences) still works and shows as insights without a title.

The page's charts are worked out here (coverage, the months your files changed, how fresh each copy
is); the page only draws them. Python 3.8+, standard library only.
"""
import argparse
import json
import os
import sys
from datetime import date, datetime

sys.dont_write_bytecode = True          # importing run_plan must not leave a __pycache__ in the skill folder
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_plan  # noqa: E402  (the table of Windows pages and the plan fingerprint live there)

TEMPLATE = os.path.join(HERE, "..", "assets", "backup-checkup-template.html")
PLACEHOLDER = "__BACKUP_DATA__"
DROPS = {"open-windows-backup", "open-file-history", "copies", "repos", "checks"}
COPY_MAX = 12                           # copy-a-folder items listed one by one; the rest share one line
REPO_MAX = 40
NO_GIT = ("Your assistant doesn't run Git inside a repository it found on disk: a repository's own settings can make "
          "any Git command start a program. You run these yourself, in that folder.")
REPO_HOW = {
    "no-remote": "It has no remote, so its whole history is only on this PC. Make an empty repository where you keep "
                 "code (GitHub, GitLab, your own server), then: git remote add origin <its address>, and "
                 "git push -u origin --all. " + NO_GIT,
    "local-remote": "Its only remote is another folder, not a server. If that folder is on this same disk it would be "
                    "lost with it. Add a remote on a server and push: git remote add origin <address>, "
                    "git push -u origin --all. " + NO_GIT,
    "unpushed": "Some of its commits are on no remote branch. Push them: git push, or for a branch that was never "
                "pushed, git push -u origin <branch>. " + NO_GIT,
    "stash": "Stashed work lives only in this folder and is never pushed. Turn what you want to keep into a branch "
             "(git stash branch <name>) and push it, or drop what you don't need. " + NO_GIT,
    "unknown": "This check couldn't read how its branches are stored. Look at it yourself: git status and "
               "git branch -vv show what is not pushed. " + NO_GIT,
}
REPO_WORDS = {"no-remote": "has no remote", "local-remote": "has only another folder as its remote",
              "unpushed": "has commits that were never pushed", "stash": "has stashed work", "unknown": "couldn't be read"}


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def human(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%d %s" % (n, unit)) if unit in ("B", "KB", "MB") else ("%.1f %s" % (n, unit))
        n /= 1024.0


def drive_name(d):
    if d.get("test"):
        return "the test drive " + (d.get("label") or "")
    return d["root"][:2] + (" (%s)" % d["label"] if d.get("label") else "")


def build_actions(survey, drop):
    places = [p for p in survey.get("places") or [] if p.get("counts")]
    protectors = {p["id"]: p for p in survey.get("protectors") or []}
    drives = survey.get("drives") or []
    separate = [d for d in drives if not d.get("system") and d.get("type") in ("fixed", "removable") and d.get("same_disk") is False]
    out = []

    # ---- Windows' own pages
    od = protectors.get("onedrive") or {}
    need = [p for p in places if p["kind"] == "known" and p["label"] in ("Desktop", "Documents", "Pictures")
            and p["verdict"] in ("none", "old", "check") and not any(x["by"] == "OneDrive" for x in p["protection"])]
    if need and "open-windows-backup" not in drop:
        names = " and ".join(", ".join(p["label"] for p in need).rsplit(", ", 1))
        out.append({"kind": "open", "command_id": "open-windows-backup", "risk": "low",
                    "title": ("Turn on OneDrive folder backup for " + names) if od.get("state") == "set-up"
                    else ("Set up OneDrive and turn on folder backup for " + names),
                    "shows": run_plan.shown(run_plan.TABLE["open-windows-backup"]),
                    "how": run_plan.TABLE["open-windows-backup"]["does"] + " These folders hold %s: check that your "
                           "OneDrive has that much room first." % human(sum(p["own_bytes"] for p in need)),
                    "bytes": sum(p["own_bytes"] for p in need), "default": False})
    fh = protectors.get("filehistory") or {}
    old = [p for p in places if p["verdict"] == "old"]
    uncovered = [p for p in places if p["verdict"] == "none" and p["kind"] == "known" and p["level"] >= 2]
    if "open-file-history" not in drop and (old or (uncovered and (separate or fh.get("state") == "set-up"))):
        if fh.get("state") == "set-up" and old:
            title = "Let File History catch up (its last copy is %d days old)" % max(
                [x["age_days"] or 0 for p in old for x in p["protection"]] or [0])
            how = "Connect the drive it copies to, then press Run now. "
        elif fh.get("state") == "set-up":
            title = "Add %s to File History" % " and ".join(", ".join(p["label"] for p in uncovered).rsplit(", ", 1))
            how = "File History is on but leaves these out. Look at its 'Exclude folders' list, and add a folder to a " \
                  "library to include it. "
        else:
            title = "Turn on File History to " + drive_name(max(separate, key=lambda d: d.get("free", 0)))
            how = "Choose that drive and turn it on. It then keeps copying your libraries and Desktop by itself. "
        out.append({"kind": "open", "command_id": "open-file-history", "risk": "low", "title": title,
                    "shows": run_plan.shown(run_plan.TABLE["open-file-history"]),
                    "how": how + run_plan.TABLE["open-file-history"]["does"],
                    "bytes": sum(p["own_bytes"] for p in old + uncovered), "default": False})

    # ---- copies for the person to set up
    if "copies" not in drop:
        todo = sorted([p for p in places if p["verdict"] == "none" and p["level"] >= 2], key=lambda p: (-p["level"], -p["own_bytes"]))
        room = {d["root"]: d.get("free", 0) for d in separate}      # what is left on each drive as folders are placed
        homeless = more = more_bytes = shown = 0
        for p in todo:
            fits = [d for d in separate if room[d["root"]] > p["own_bytes"] * 1.1
                    and not p["path"].upper().startswith(d["root"].upper())]
            if not fits:
                homeless += p["own_bytes"]
                continue
            d = max(fits, key=lambda d: room[d["root"]])
            room[d["root"]] -= p["own_bytes"]
            if shown >= COPY_MAX:
                more += 1
                more_bytes += p["own_bytes"]
                continue
            shown += 1
            out.append({"kind": "guide", "guide_id": "copy-folder", "risk": "info", "default": False,
                        "title": "Copy %s (%s) to %s" % (p["label"], human(p["own_bytes"]), drive_name(d)),
                        "path": p["path"], "bytes": p["own_bytes"],
                        "how": "In Explorer, drag this folder onto that drive. It has room: %s free." % human(d.get("free", 0))})
        if more:
            out.append({"kind": "guide", "guide_id": "copy-folder", "risk": "info", "default": False, "bytes": more_bytes,
                        "title": "Copy the other %d folder%s with no second copy (%s in all)" % (more, "" if more == 1 else "s", human(more_bytes)),
                        "how": "They are listed under Your folders. The same goes for them: a copy on a separate disk, "
                               "or better, something that keeps the copy up to date by itself."})
        if homeless:
            out.append({"kind": "guide", "guide_id": "get-a-drive", "risk": "info", "default": False, "bytes": homeless,
                        "title": "Get somewhere to keep a second copy of %s" % human(homeless),
                        "how": "No separate disk with enough room is connected. A second letter on the same disk doesn't "
                               "count: it dies with the disk. An external drive of at least %s, or that much cloud "
                               "space, would hold what has no copy today." % human(int(homeless * 1.5))})

    # ---- code that exists only here
    if "repos" not in drop:
        risky = [r for r in survey.get("repos") or [] if r["state"] in REPO_HOW]
        for r in risky[:REPO_MAX]:
            detail = []
            for b in r.get("branches") or []:
                if b["state"] == "ahead":
                    detail.append("branch %s: %s never pushed" % (b["name"], ("about %d commits" % b["ahead"]) if b.get("ahead") else "commits"))
                elif b["state"] == "only-here":
                    detail.append("branch %s exists only here" % b["name"])
            if r.get("stashes"):
                detail.append("%d stash%s" % (r["stashes"], "" if r["stashes"] == 1 else "es"))
            if r.get("changed_since_commit"):
                detail.append("files changed since the last commit")
            if r.get("place_verdict") == "ok":
                detail.append("its files are also in a synced folder, so this is less urgent")
            out.append({"kind": "guide", "guide_id": "push-repo", "risk": "info", "default": False,
                        "title": "%s %s" % (r["name"], REPO_WORDS[r["state"]]), "path": r["path"], "repo": r["id"],
                        "detail": "; ".join(detail), "how": REPO_HOW[r["state"]], "bytes": r.get("bytes", 0)})
        if len(risky) > REPO_MAX:
            out.append({"kind": "guide", "guide_id": "push-repo", "risk": "info", "default": False, "bytes": 0,
                        "title": "Push the other %d repositories with work that is on no remote" % (len(risky) - REPO_MAX),
                        "how": "They are in the table above this plan. " + NO_GIT})

    # ---- programs to look at
    if "checks" not in drop:
        for p in survey.get("protectors") or []:
            if p.get("running") is False:
                out.append({"kind": "guide", "guide_id": "start-client", "risk": "info", "default": False, "bytes": 0,
                            "title": "Start %s: it isn't running, so nothing is being copied" % p["name"],
                            "how": "Open it from the Start menu and wait for its icon to say it is up to date."})
            elif p.get("coverage") == "unknown":
                out.append({"kind": "guide", "guide_id": "check-client", "risk": "info", "default": False, "bytes": 0,
                            "title": "Check what %s covers" % p["name"],
                            "how": "It is installed, but its settings aren't readable from outside. Open it and look at "
                                   "which folders it includes and when it last finished."})
            elif p["id"] == "filehistory" and p["state"] == "set-up" and any("not connected" in f for f in p.get("facts") or []):
                out.append({"kind": "guide", "guide_id": "connect-drive", "risk": "info", "default": False, "bytes": 0,
                            "title": "Connect the drive File History copies to",
                            "how": "File History only copies while its drive is connected. Plug it in and leave it for a while."})
    for i, a in enumerate(out, 1):
        a["id"] = i
    return out


GROUP = {"none": "none", "old": "old", "check": "check", "partial": "check", "ok": "ok"}


def text(v, limit):
    """A note from the agent, as plain text of a sane length (the page shows it with textContent)."""
    return " ".join(str(v or "").split())[:limit]


def norm(p):
    return os.path.normcase(os.path.normpath(str(p or "")))


def drive_key(d, i):
    """How the page tells drives apart: the letter, or T0, T1... for the folders a test run uses as drives."""
    return ("T%d" % i) if d.get("test") else str(d.get("root") or "?")[:1].upper()


def page_view(survey):
    keep = ("generated", "options", "budget", "profile", "summary", "places", "lose", "protectors",
            "not_measured", "left_out", "cant_see")
    view = {k: survey.get(k) for k in keep}
    view["repos"] = (survey.get("repos") or [])[:400]
    # each disk in this PC gets a light of its own colour, in order (the page's --c1 to --c6)
    drives, slot = [], 0
    for i, d in enumerate(survey.get("drives") or []):
        d = dict(d, key=drive_key(d, i))
        if d.get("type") in ("fixed", "removable") and slot < 6:
            slot += 1
            d["slot"] = slot
        drives.append(d)
    view["drives"] = drives
    return view


def agent_notes(notes, survey):
    """What the agent wrote: mom's take, a line for a drive, notes on folders. Text only, cut to size."""
    insights = []
    for it in (notes.get("insights") or [])[:8]:
        if isinstance(it, dict) and text(it.get("text"), 600):
            insights.append({"title": text(it.get("title"), 90), "text": text(it.get("text"), 600)})
        elif isinstance(it, str) and text(it, 600):
            insights.append({"title": "", "text": text(it, 600)})
    for h in (notes.get("highlights") or [])[:8]:
        if len(insights) < 8 and text(h, 600):
            insights.append({"title": "", "text": text(h, 600)})
    keys = {drive_key(d, i) for i, d in enumerate(survey.get("drives") or [])}
    drives = {}
    for k, v in (notes.get("drives") or {}).items() if isinstance(notes.get("drives"), dict) else []:
        k = str(k).strip().rstrip(":\\").upper()
        if k in keys and text(v, 200):
            drives[k] = text(v, 200)
    ids = {norm(p.get("path")): p.get("id") for p in survey.get("places") or []}
    places = {}
    for it in (notes.get("places") or [])[:40]:
        if isinstance(it, dict) and text(it.get("note"), 300) and ids.get(norm(it.get("path"))) is not None:
            places[str(ids[norm(it.get("path"))])] = text(it.get("note"), 300)
    return insights, drives, places


def day_of(iso):
    try:
        return datetime.strptime(str(iso)[:10], "%Y-%m-%d").date()
    except ValueError:
        return None


def months_back(today, n=12):
    """The last n months as "YYYY-MM", oldest first, ending with today's month."""
    y, m, out = today.year, today.month, []
    for _ in range(n):
        out.append("%04d-%02d" % (y, m))
        y, m = (y, m - 1) if m > 1 else (y - 1, 12)
    return out[::-1]


def and_list(names):
    names = list(names)
    if len(names) < 2:
        return "".join(names)
    return ", ".join(names[:-1]) + " and " + names[-1]


def dashboard(survey, actions):
    """The numbers behind the page's charts. The page draws them and works out nothing of its own."""
    today = day_of(survey.get("generated")) or date.today()
    places = [p for p in survey.get("places") or [] if p.get("counts")]
    label_of = {norm(p["path"]): p["label"] for p in survey.get("places") or []}
    drives = survey.get("drives") or []

    coverage = {"none": 0, "old": 0, "check": 0, "ok": 0}
    for p in places:
        coverage[GROUP.get(p["verdict"], "check")] += p.get("own_bytes") or 0

    # the last 12 months: how much of your own files changed in each, and whether that change has a copy
    keys = months_back(today)
    series = {"copied": [0] * len(keys), "only_here": [0] * len(keys), "unknown": [0] * len(keys)}
    for p in places:
        cut = ((p.get("changed_since_copy") or {}).get("since") or "")[:7]
        for i, k in enumerate(keys):
            b = ((p.get("months") or {}).get(k) or {}).get("bytes") or 0
            if not b:
                continue
            v = p["verdict"]
            if v == "ok" or (v == "old" and cut and k < cut):
                series["copied"][i] += b          # changed before the copy was made, so it is in the copy
            elif v in ("none", "old"):
                series["only_here"][i] += b
            else:
                series["unknown"][i] += b
    since = sorted(({"id": p["id"], "label": p["label"], "since": p["changed_since_copy"]["since"],
                     "files": p["changed_since_copy"]["files"], "bytes": p["changed_since_copy"]["bytes"]}
                    for p in places if (p.get("changed_since_copy") or {}).get("bytes")), key=lambda x: -x["bytes"])

    # how old each dated copy is: backup programs, things on other drives named like backups, Git remotes
    fresh = []
    for p in survey.get("protectors") or []:
        d = day_of(p.get("last_copy"))
        if d and p.get("state") == "set-up":
            what = [label_of.get(norm(c["path"]), os.path.basename(c["path"].rstrip("\\/")) or c["path"]) for c in p.get("covers") or []]
            fresh.append({"name": p["name"], "kind": "backup", "date": d.isoformat(), "days": max(0, (today - d).days),
                          "about": ("Copies " + and_list(what[:4]) + (" and more" if len(what) > 4 else "")) if what
                          else "Which folders it copies can't be read from outside"})
    for i, dr in enumerate(drives):
        for b in dr.get("backups") or []:
            d = day_of(b.get("newest"))
            if not d or b.get("kind") == "File History's store":     # that one is File History's own row
                continue
            where = dr.get("label") if dr.get("test") else str(dr.get("root") or "")[:2]
            fresh.append({"name": b["name"], "kind": "hint", "drive": drive_key(dr, i), "date": d.isoformat(),
                          "days": max(0, (today - d).days),
                          "about": "%s on %s%s. Its name is a hint, not proof" % (
                              (b.get("kind") or "looks like a backup")[:1].upper() + (b.get("kind") or "looks like a backup")[1:], where,
                              ", the same disk as Windows" if dr.get("same_disk") else "")})
    pushes = [day_of(r.get("last_remote_update")) for r in survey.get("repos") or []]
    pushes = [d for d in pushes if d]
    if pushes:
        fresh.append({"name": "Git remotes", "kind": "git", "date": max(pushes).isoformat(), "days": max(0, (today - max(pushes)).days),
                      "about": "The newest push or fetch of any of your %d repositories" % len(survey.get("repos") or [])})
    fresh.sort(key=lambda x: (x["kind"] != "backup", x["days"]))

    repos = survey.get("repos") or []
    dated = [x for x in fresh if x["kind"] == "backup"]
    newest = min(dated, key=lambda x: x["days"]) if dated else None
    others = [d for d in drives if not d.get("system") and d.get("type") in ("fixed", "removable")]
    s = survey.get("summary") or {}
    facts = {"counted": len(places), "ok": len([p for p in places if p["verdict"] == "ok"]),
             "at_risk": s.get("bytes_at_risk") or 0,
             "newest_days": newest["days"] if newest else None, "newest_by": newest["name"] if newest else "",
             "repos": len(repos), "repos_at_risk": len([r for r in repos if r["state"] in ("no-remote", "local-remote", "unpushed", "stash")]),
             "separate": len([d for d in others if d.get("same_disk") is False]),
             "same_disk": len([d for d in others if d.get("same_disk") is True])}

    # which drive a "copy this folder" item points at, so the page can light it (the plan itself is untouched)
    names = [(drive_key(d, i), drive_name(d)) for i, d in enumerate(drives) if d.get("root")]
    copy_to = {}
    for a in actions:
        if a.get("guide_id") == "copy-folder":
            hit = next((k for k, n in names if a["title"].endswith(" to " + n)), None)
            if hit:
                copy_to[str(a["id"])] = hit
    return {"coverage": coverage, "months": dict(series, keys=keys), "since": since[:12], "fresh": fresh[:12],
            "facts": facts, "copy_to": copy_to, "no_git": NO_GIT}     # said once over the push items, not in each


def embed(obj):
    # Safe inside <script type="application/json">: no "<", ">" or "&" survive as raw characters
    return (json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
            .replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
            .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029"))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser(description="Build the backup-checkup page and backup-plan.json from a survey.")
    ap.add_argument("survey")
    ap.add_argument("--out", required=True, help="the HTML page to write")
    ap.add_argument("--plan", help="where to write the plan (default: backup-plan.json next to --out)")
    ap.add_argument("--notes", help="optional notes.json with headline, insights, places, drives, drop")
    args = ap.parse_args(argv)

    survey = load(args.survey)
    if not str(survey.get("schema", "")).startswith("backup-checkup/survey@"):
        sys.exit("%s is not a backup-checkup survey" % args.survey)
    notes = load(args.notes) if args.notes else {}
    if not isinstance(notes, dict):
        sys.exit("notes.json must be one JSON object")
    drop = {str(x) for x in notes.get("drop") or []}
    if drop - DROPS:
        sys.exit("notes.json 'drop' takes only: %s" % ", ".join(sorted(DROPS)))
    actions = build_actions(survey, drop)
    pid = run_plan.plan_id(actions)
    plan_path = args.plan or os.path.join(os.path.dirname(os.path.abspath(args.out)), "backup-plan.json")
    with open(plan_path, "w", encoding="utf-8") as f:
        json.dump({"schema": run_plan.PLAN_SCHEMA, "plan_id": pid, "generated": datetime.now().isoformat(timespec="seconds"),
                   "actions": actions}, f, ensure_ascii=False, indent=1)
    with open(TEMPLATE, encoding="utf-8") as f:
        page = f.read()
    if PLACEHOLDER not in page:
        sys.exit("template is missing the %s placeholder" % PLACEHOLDER)
    insights, drive_lines, place_notes = agent_notes(notes, survey)
    headline = text(notes.get("headline"), 300) or text((survey.get("summary") or {}).get("headline"), 300).rstrip(".") + "."
    data = {"plan_id": pid, "actions": actions, "survey": page_view(survey), "dash": dashboard(survey, actions),
            "notes": {"headline": headline, "insights": insights, "drives": drive_lines, "places": place_notes}}
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(page.replace(PLACEHOLDER, embed(data)))
    kinds = {}
    for a in actions:
        k = a.get("guide_id") or a["command_id"]
        kinds[k] = kinds.get(k, 0) + 1
    print("page: %s (%d KB)\nplan: %s  plan_id=%s  %d items %s" % (
        os.path.abspath(args.out), os.path.getsize(args.out) // 1024, plan_path, pid, len(actions), json.dumps(kinds)))


if __name__ == "__main__":
    main()

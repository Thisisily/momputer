#!/usr/bin/env python3
"""Carry out the approved part of a tidy plan, with an undo log. Dry run unless --apply.

    python apply_plan.py plan.json --approve "TIDY-APPROVE plan=3f9a1c07be ids=1-40,52"
    python apply_plan.py plan.json --approval tidy-approval-3f9a1c07be.json --apply
    python apply_plan.py --undo <log folder>             # dry run of the revert
    python apply_plan.py --undo <log folder> --apply

The work, and every rule it enforces whatever the plan says, is in safe_apply.py: the one
file-moving engine the skills in this collection share. Read the rules at the top of that
file. This command adds what only tidy-folders knows about:

  * never touches a registered program's install folder or a launcher's game library
    (Steam, Epic, GOG, Xbox app, EA, Ubisoft, Riot), or anything that holds one

Python 3.8+, standard library only. Windows only for --apply.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
import safe_apply  # noqa: E402  (the shared engine: guard rails, Recycle Bin, undo log, both undo routes)
import survey as sv  # noqa: E402  (finds installed programs and launcher games the same way the survey does)


def off_limits(root=None):
    """Installed programs and launcher game libraries, for safe_apply's guard. A --root run has
    none, as in survey.py: it looks at one folder, not at the registry or the launchers."""
    if root:
        return []
    kf = sv.known_folders()
    progs = sv.installed_programs(
        [kf.get(k) for k in ("Profile", "Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music", "OneDrive")])
    games = []
    for lch in sv.launchers(sv.drive_list()):
        games += [g["path"] for g in lch["games"]]
        if lch["name"] == "Steam":
            games += [os.path.join(lib, "steamapps") for lib in lch["libraries"]]
        elif lch["name"] in ("Xbox app", "Riot", "EA app"):
            games += lch["libraries"]
    # Never a drive root or a folder that holds the user's own folders (a game installed straight
    # into C:\SomeGame must not put all of C:\ off-limits)
    keep_out = [sv.norm(kf[k]) for k in ("Profile", "Desktop", "Documents", "Downloads") if kf.get(k)]
    games = [p for p in games if os.path.dirname(sv.norm(p)) != sv.norm(p)
             and not any(sv.under(k, [sv.norm(p)]) for k in keep_out)]
    holds = "contains an installed program or game"
    return [([p["path"] for p in progs], "inside an installed program's folder", holds),
            (games, "inside a launcher's game library (use the launcher's own Move)", holds)]


def main(argv=None):
    args = safe_apply.parse_args(argv, "Apply an approved tidy plan (dry run unless --apply), or undo one.")
    safe_apply.run(args, places=off_limits)


if __name__ == "__main__":
    main()

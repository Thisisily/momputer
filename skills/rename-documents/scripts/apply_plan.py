#!/usr/bin/env python3
"""Carry out the approved part of a rename plan, with an undo log. Dry run unless --apply.

    python apply_plan.py rename-plan.json --approve "RENAME-APPROVE plan=3f9a1c07be ids=1-12,15"
    python apply_plan.py rename-plan.json --approve "..." --within "D:\\Paperwork" --apply
    python apply_plan.py --undo <log folder>             # dry run of the revert
    python apply_plan.py --undo <log folder> --apply

The work, and every rule it enforces whatever the plan says, is in safe_apply.py: the one
file-moving engine the skills in this collection share. Read the rules at the top of that
file. This command adds what makes it a renaming skill:

  * --within is always set: the folders the person pointed at (give each one, as you gave
    it to the survey), or Downloads and the scan folders if you give none. A file outside
    them is never touched.
  * every action must be a rename: a file (never a folder) that stays in its own folder
  * the new name is checked again, with the same names.why_bad() that built the plan: a
    plain file name, no reserved or forbidden or invisible characters, the same extension
  * nothing inside a code project or a program's own folder is renamed
  * --undo is held to the same rules: a log row that is not such a rename is refused

Python 3.8+, standard library only. Windows only for --apply.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
import names  # noqa: E402  (the one definition of what a name may be)
import safe_apply  # noqa: E402  (the shared engine: guard rails, undo log, both undo routes)


def only_a_rename(action, guard):
    """A reason to refuse `action`, or None. The engine has already checked both paths.
    The engine asks about each row of an undo log as well (action["undo"]): the same rename,
    already done, so the file is under its new name now."""
    src, dst = action["from"], action.get("to")
    undo = bool(action.get("undo"))
    if action.get("op") != "move" or not isinstance(dst, str):
        return "not a rename: this skill only renames files"
    if safe_apply.norm(os.path.dirname(src)) != safe_apply.norm(os.path.dirname(dst)):
        return "not a rename: the file would leave its folder"
    if os.path.isdir(safe_apply.xp(dst if undo else src)):
        return "a folder: this skill only renames files"
    old, new, folder = os.path.basename(src), os.path.basename(dst), os.path.dirname(dst)
    problem = names.why_bad(old, new, folder)
    if problem and undo:
        # A name that got taken at the last moment is given " (2)" by the engine, and that is the
        # name in the log: a few characters longer than the one that was checked
        stem, ext = os.path.splitext(new)
        if re.search(r" \(\d+\)$", stem):
            problem = names.why_bad(old, re.sub(r" \(\d+\)$", "", stem) + ext, folder)
    if problem:
        return "the new name is " + problem
    where = names.off_limits(os.path.dirname(src), guard.kf["Profile"])
    if where:
        return "in %s: nothing there is renamed" % where
    return None


def main(argv=None):
    args = safe_apply.parse_args(argv, "Apply an approved rename plan (dry run unless --apply), or undo one.")
    if not args.within:
        args.within = names.default_folders(os.path.abspath(args.root) if args.root else None)
        if not args.within:
            sys.exit("Say which folders the renames are in: --within <folder> for each folder you surveyed.")
    safe_apply.run(args, check=only_a_rename)


if __name__ == "__main__":
    main()

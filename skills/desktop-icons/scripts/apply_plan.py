#!/usr/bin/env python3
"""Carry out the approved part of a desktop plan, with an undo log. Dry run unless --apply.

    python apply_plan.py desktop-plan.json --approve "DESKTOP-APPROVE plan=3f9a1c07be ids=1-12,15"
    python apply_plan.py desktop-plan.json --approval desktop-approval-3f9a1c07be.json --apply
    python apply_plan.py --undo <log folder>             # dry run of the revert
    python apply_plan.py --undo <log folder> --apply

The work, and every rule it enforces whatever the plan says, is in safe_apply.py: the one
file-moving engine the skills in this collection share. Read the rules at the top of that
file. This command adds what makes it a desktop skill:

  * it sets the engine's --within itself: the person's own Desktop, and their Documents,
    Pictures, Videos, Music and Downloads for things filed off the Desktop. Nothing else
    can be a source or a destination. The Public Desktop is never changed.
  * a source must be on the Desktop itself: this skill takes things off the Desktop, it
    doesn't rearrange the other folders
  * --undo is held to the same rules: a log row for something that didn't come off the
    Desktop is refused

Python 3.8+, standard library only. Windows only for --apply.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
import safe_apply  # noqa: E402  (the shared engine: guard rails, Recycle Bin, undo log, both undo routes)

LIBRARIES = ("Documents", "Pictures", "Videos", "Music", "Downloads")


def main(argv=None):
    args = safe_apply.parse_args(argv, "Apply an approved desktop plan (dry run unless --apply), or undo one.")
    if args.within:
        sys.exit("Leave --within out: this command limits itself to the Desktop and the folders things are filed into.")
    folders = safe_apply.known_folders(os.path.abspath(args.root) if args.root else None)
    desktop = folders.get("Desktop")
    if not desktop:
        sys.exit("No Desktop folder found.")
    on_desktop = safe_apply.limits([desktop], "The Desktop folder")
    # A library that isn't a plain folder on a local disk (redirected to a share, say) is simply
    # not a place this run may file things into; the engine would refuse it anyway
    args.within = [desktop] + [folders[k] for k in LIBRARIES if folders.get(k) and not safe_apply.not_plain(folders[k])]

    def from_the_desktop(action, guard, undoing):  # the same rule for a plan's action and for an undo-log row
        if not safe_apply.inside(action["from"], on_desktop):
            return "not on your Desktop: this skill only takes things off the Desktop"
        return None

    safe_apply.run(args, check=from_the_desktop)


if __name__ == "__main__":
    main()

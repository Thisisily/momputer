"""Runs every test in this folder against every skill and prints one line each.

    python -I run_all.py <scratch folder under %TEMP%> [--no-browser] [--keep]

Each test works only in a folder it makes inside <scratch folder> and removes it afterwards
(--keep leaves them for a look). Nothing else on the PC is created, moved or deleted.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import os
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
SKILLS = os.path.abspath(os.path.join(HERE, ".."))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("scratch")
    ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser checks of the pages")
    ap.add_argument("--keep", action="store_true", help="leave each test's work folder in place")
    args = ap.parse_args(argv)
    scratch = os.path.abspath(args.scratch)
    if not os.path.isdir(scratch):
        sys.exit("The scratch folder must already exist: %s" % scratch)
    engines = sorted(d for d in os.listdir(SKILLS) if os.path.isfile(os.path.join(SKILLS, d, "scripts", "safe_apply.py")))
    extra = (["--no-browser"] if args.no_browser else []) + (["--keep"] if args.keep else [])
    keep = ["--keep"] if args.keep else []
    jobs = [("check_shared", ["check_shared.py", SKILLS])]
    for skill in engines:
        jobs.append(("test_safe_apply %s" % skill, ["test_safe_apply.py", scratch, os.path.join(SKILLS, skill)] + keep))
    # tidy-folders' own command adds no rules in --root mode, so the whole engine test can run through it as well
    jobs.append(("test_safe_apply tidy-folders, through apply_plan.py",
                 ["test_safe_apply.py", scratch, os.path.join(SKILLS, "tidy-folders"), "--entry", "apply_plan.py"] + keep))
    for name in ("tidy_folders", "desktop_icons", "rename_documents"):
        if os.path.isdir(os.path.join(SKILLS, name.replace("_", "-"))):
            jobs.append(("test_%s" % name, ["test_%s.py" % name, scratch] + extra))
    if os.path.isdir(os.path.join(SKILLS, "inbox-cleanup")):
        jobs.append(("test_inbox_queries", ["test_inbox_queries.py", scratch] + keep))
    # two skills that move no files: one test end to end (with the page in a browser), and one for each careful part
    for name, parts in (("subscription_audit", ("reader", "detect")), ("bookmark_cleanup", ("links",))):
        if os.path.isdir(os.path.join(SKILLS, name.replace("_", "-"))):
            jobs.append(("test_%s" % name, ["test_%s.py" % name, scratch] + extra))
            for part in parts:
                jobs.append(("test_%s_%s" % (name, part), ["test_%s_%s.py" % (name, part), scratch] + keep))
    results = []
    for label, cmd in jobs:
        print("\n" + "=" * 100 + "\n" + label + "\n" + "=" * 100, flush=True)
        r = subprocess.run([sys.executable, "-I", os.path.join(HERE, cmd[0])] + cmd[1:], capture_output=True, text=True, encoding="utf-8", errors="replace")
        text = r.stdout + r.stderr
        shown = [l for l in text.splitlines() if not l.lstrip().startswith("ok:")]
        print("\n".join(shown))
        last = next((l for l in reversed(text.splitlines()) if "PASSED" in l or "FAILED" in l), "(no result line)")
        results.append((label, r.returncode, last.strip(), [l for l in text.splitlines() if l.startswith("not run:")]))
    print("\n" + "=" * 100)
    for label, code, last, skipped in results:
        print("%-4s %-52s %s" % ("ok" if code == 0 else "FAIL", label, last))
        for s in skipped:
            print("     %s" % s)
    bad = [label for label, code, _, _ in results if code != 0]
    print("\n%s (Python %s)" % ("EVERYTHING PASSED" if not bad else "%d FAILED: %s" % (len(bad), bad), sys.version.split()[0]))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

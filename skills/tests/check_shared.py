"""Fails if any skill's copy of the shared file-moving engine has drifted from tidy-folders'.

    python -I check_shared.py [<skills folder>]

scripts/safe_apply.py and scripts/revert.ps1 are one implementation, kept in tidy-folders. A
skill that moves, renames or removes files ships byte-identical copies. This compares every
skills/*/scripts/ copy with the reference, byte for byte. It reads files and changes nothing.
Python 3.8+, standard library only.
"""
import hashlib
import os
import sys

sys.dont_write_bytecode = True
SHARED = ("safe_apply.py", "revert.ps1")
REFERENCE = "tidy-folders"


def digest(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    skills = os.path.abspath(argv[0]) if argv else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    ref_dir = os.path.join(skills, REFERENCE, "scripts")
    missing = [n for n in SHARED if not os.path.isfile(os.path.join(ref_dir, n))]
    if missing:
        sys.exit("The reference copy is missing: %s in %s" % (", ".join(missing), ref_dir))
    ref = {n: digest(os.path.join(ref_dir, n)) for n in SHARED}
    bad, copies = [], 0
    for skill in sorted(os.listdir(skills)):
        scripts = os.path.join(skills, skill, "scripts")
        if skill == REFERENCE or not os.path.isdir(scripts):
            continue
        have = [n for n in SHARED if os.path.isfile(os.path.join(scripts, n))]
        if not have:
            continue  # a skill that doesn't touch files (inbox-cleanup) ships neither
        for n in SHARED:
            path = os.path.join(scripts, n)
            if n not in have:
                bad.append("%s: has %s but not %s (they go together)" % (skill, have[0], n))
            elif digest(path) != ref[n]:
                bad.append("%s: scripts/%s differs from the one in %s" % (skill, n, REFERENCE))
            else:
                copies += 1
                print("ok:   %s/scripts/%s is identical to the one in %s" % (skill, n, REFERENCE))
    for line in bad:
        print("FAIL: " + line)
    print("\n%s: %d identical copies of %s (%s %s, %s %s)" % (
        "ALL PASSED" if not bad else "%d FAILED" % len(bad), copies, " and ".join(SHARED),
        SHARED[0], ref[SHARED[0]][:12], SHARED[1], ref[SHARED[1]][:12]))
    if bad:
        print("Copy the two files from %s/scripts/ again; never edit a copy." % REFERENCE)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

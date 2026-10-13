"""Sets momputer's version everywhere it's written down.

    python scripts/version.py            # says what it is
    python scripts/version.py 0.2.1      # sets it

The version lives in two places that must agree: Cargo.toml (the whole workspace) and
app/src-tauri/tauri.conf.json (the installer). The app shows it in About and compares it with
the newest release on GitHub, so every release needs a higher one. Python 3.8+, standard library.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLACES = [
    ("Cargo.toml", r'(\[workspace\.package\]\s*version = ")(\d+\.\d+\.\d+)(")'),
    ("app/src-tauri/tauri.conf.json", r'("version": ")(\d+\.\d+\.\d+)(")'),
]


def main():
    new = sys.argv[1] if len(sys.argv) > 1 else None
    if new and not re.fullmatch(r"\d+\.\d+\.\d+", new):
        sys.exit("A version is three numbers: 0.2.1")
    seen = []
    for path, pattern in PLACES:
        full = os.path.join(ROOT, path)
        with open(full, encoding="utf-8", newline="") as f:
            text = f.read()
        m = re.search(pattern, text)
        if not m:
            sys.exit("No version found in " + path)
        seen.append(m.group(2))
        if new and m.group(2) != new:
            with open(full, "w", encoding="utf-8", newline="") as f:
                f.write(text[: m.start(2)] + new + text[m.end(2):])
    if new:
        print("momputer is now " + new + ". Run cargo build once so Cargo.lock follows.")
    elif len(set(seen)) == 1:
        print("momputer " + seen[0])
    else:
        sys.exit("The versions disagree: " + ", ".join(p + " says " + v for (p, _), v in zip(PLACES, seen)))


if __name__ == "__main__":
    main()

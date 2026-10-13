"""Draws the README's badges into docs/badges/: small pills in the app's own look (black, a tiny
light, spaced capitals), so they don't depend on anyone else's badge service.

    python scripts/badges.py

Change the words below and run it again. Python 3.8+, standard library only.
"""
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "docs", "badges")

GREEN, RED = "#2fe07a", "#ff3b47"
# name, words, light, inverted (white, for the one that's a button)
BADGES = [
    ("download", "Download for Windows", RED, True),
    ("windows", "Windows 10 · 11", GREEN, False),
    ("free", "Free for personal use", GREEN, False),
    ("big-mode", "BIG MODE", GREEN, False),
    ("skills", "11 agent skills", GREEN, False),
    ("agents", "Claude Code · Codex · Cursor · Gemini · Copilot", GREEN, False),
]

# Roughly how wide each capital is at this size. The text is then stretched to exactly that
# width (textLength), so the pill fits whatever font the reader's browser picks.
NARROW, WIDE = " I1·.,", "MW"


def width(text):
    return sum(3.7 if c in NARROW else 9.4 if c in WIDE else 7.0 for c in text) + 1.1 * (len(text) - 1)


def badge(words, light, inverted):
    text = words.upper()
    tw = round(width(text), 1)
    w = round(27 + tw + 15)
    bg, edge, ink = ("#ffffff", "#cfcfcf", "#000000") if inverted else ("#0b0b0b", "#2c2c2c", "#f4f4f4")
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="28" viewBox="0 0 {w} 28" role="img" aria-label="{words}">\n'
        f"  <title>{words}</title>\n"
        f'  <rect x="0.5" y="0.5" width="{w - 1}" height="27" rx="13.5" fill="{bg}" stroke="{edge}"/>\n'
        f'  <circle cx="15" cy="14" r="3.5" fill="{light}"/>\n'
        f'  <text x="27" y="17.9" font-family="\'Segoe UI\', \'Helvetica Neue\', Helvetica, Arial, sans-serif" font-size="10.5" font-weight="700" '
        f'fill="{ink}" textLength="{tw}" lengthAdjust="spacing">{text}</text>\n'
        "</svg>\n"
    )


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, words, light, inverted in BADGES:
        with open(os.path.join(OUT, name + ".svg"), "w", encoding="utf-8", newline="\n") as f:
            f.write(badge(words, light, inverted))
        print(f"docs/badges/{name}.svg")


if __name__ == "__main__":
    main()

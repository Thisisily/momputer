#!/usr/bin/env python3
"""Copies the page kit into every skill page that uses it, so each page stays one file.

    python -I sync.py            # writes the kit into each page that has its markers
    python -I sync.py --check    # changes nothing; exits 1 if a page's copy is out of date

A page uses the kit by holding these markers (the text between each pair is replaced):

    <style id="kit-css">/*@kit-css*/ ... /*@end*/</style>
    <script> /*@kit-js*/ ... /*@end*/  ...the page's own script... </script>
    <!--@mom--> ... <!--@end-->          (optional: mom's face)

The fonts go in as data: addresses (no network), with their licence in fonts/OFL.txt.
Python 3.8+, standard library only.
"""
import base64
import os
import re
import sys

sys.dont_write_bytecode = True
KIT = os.path.dirname(os.path.abspath(__file__))
SKILLS = os.path.dirname(KIT)
FONTS = [("Geist", "geist.woff2", "100 900", "swap"),
         ("Geist Mono", "geist-mono.woff2", "100 900", "swap"),
         ("Doto", "doto.woff2", "100 900", "block")]
BLOCKS = [("/*@kit-css*/", "/*@end*/", "css"), ("/*@kit-js*/", "/*@end*/", "js"), ("<!--@mom-->", "<!--@end-->", "mom")]


def read(name, mode="r"):
    with open(os.path.join(KIT, name), mode, **({} if "b" in mode else {"encoding": "utf-8"})) as f:
        return f.read()


def parts():
    faces = []
    for family, file, weight, display in FONTS:
        data = base64.b64encode(read(os.path.join("fonts", file), "rb")).decode("ascii")
        faces.append('@font-face { font-family: "%s"; src: url("data:font/woff2;base64,%s") format("woff2"); '
                     'font-weight: %s; font-display: %s; }' % (family, data, weight, display))
    css = "/* Fonts: Geist, Geist Mono (Vercel) and Doto, under the SIL Open Font License 1.1. */\n" + "\n".join(faces) + "\n" + read("kit.css")
    mom = read("mom.svg").strip()
    mom = re.sub(r"^<svg ", '<svg role="img" aria-label="mom" ', mom, count=1)
    return {"css": css.replace("\r\n", "\n"), "js": read("kit.js").replace("\r\n", "\n"), "mom": mom.replace("\r\n", "\n")}


def pages():
    for skill in sorted(os.listdir(SKILLS)):
        assets = os.path.join(SKILLS, skill, "assets")
        if skill.startswith(("_", ".")) or not os.path.isdir(assets):
            continue
        for name in sorted(os.listdir(assets)):
            if name.endswith(".html"):
                yield os.path.join(assets, name)


def fill(text, p):
    for begin, end, key in BLOCKS:
        start = text.find(begin)
        if start < 0:
            continue
        stop = text.find(end, start + len(begin))
        if stop < 0:
            raise ValueError("%s has no %s after it" % (begin, end))
        text = text[:start + len(begin)] + "\n" + p[key] + "\n" + text[stop:]
    return text


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    check = "--check" in argv
    p = parts()
    stale, used = [], 0
    for path in pages():
        with open(path, encoding="utf-8", newline="") as f:
            text = f.read()
        if "/*@kit-css*/" not in text:
            continue
        used += 1
        text = text.replace("\r\n", "\n")   # a Windows checkout may have turned the line ends into CRLF
        new = fill(text, p)
        if new != text:
            if check:
                stale.append(os.path.relpath(path, SKILLS))
            else:
                with open(path, "w", encoding="utf-8", newline="\n") as f:
                    f.write(new)
                print("updated: " + os.path.relpath(path, SKILLS))
    if check:
        for s in stale:
            print("FAIL: %s has an old copy of the kit (run skills/_kit/sync.py)" % s)
        print("%s: %d page(s) use the kit" % ("FAILED" if stale else "PASSED", used))
        return 1 if stale else 0
    print("%d page(s) use the kit" % used)
    return 0


if __name__ == "__main__":
    sys.exit(main())

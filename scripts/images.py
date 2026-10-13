"""Remakes the pictures in docs/ and site/ from the app itself.

    python scripts/images.py            # all of them
    python scripts/images.py hero big   # only the ones whose name contains one of these words

The app's own pages (app/ui, with its sample data) are opened in a headless Microsoft Edge and
photographed; the composed pictures (docs/art/*.html) are pages too. Nothing is drawn by hand,
so when the app changes, run this again.

Needs: Python 3.8+, Microsoft Edge, and the websocket-client package (pip install websocket-client).
The site's pictures (site/shots/) are cut from the ones in docs/, which also needs Pillow.
"""
import base64
import functools
import http.server
import json
import os
import shutil
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

import websocket

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EDGE = next((p for p in (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", r"C:\Program Files\Microsoft\Edge\Application\msedge.exe") if os.path.exists(p)), "msedge")
APP = "/app/ui/index.html"

click = lambda sel: f"(document.querySelector({json.dumps(sel)}) || {{click(){{}}}}).click()"
top = lambda sel: f"(document.querySelector({json.dumps(sel)}) || document.body).scrollIntoView({{block: 'start'}})"

# name, page, width, height, pixel ratio, light or dark, steps (JavaScript, or a number of seconds to wait)
WIDGET = (420, 760, 2)
JOBS = [
    ("screenshot-dark", APP, *WIDGET, "dark", [3]),
    ("screenshot-light", APP, *WIDGET, "light", [3, click("[data-tab=upkeep]"), 7]),
    ("screenshot-details", APP, *WIDGET, "dark", [3, click("[data-tab=upkeep]"), 2.5, click('[data-spot="claude-temp"]'), 1.5]),
    ("screenshot-health", APP, *WIDGET, "dark", [3, click("[data-tab=upkeep]"), 2.5, top("#sec-pc"), 0.6]),
    ("screenshot-rooms", APP, *WIDGET, "light", [3, click("[data-tab=rooms]"), 1.5, click("[data-act=open-room]"), 1.5]),
    ("screenshot-rules", APP, *WIDGET, "dark", [3, click("[data-tab=rules]"), 1.5]),
    ("screenshot-speed", APP, *WIDGET, "light", [3, click("[data-act=running-tab][data-tab=speed]"), 2, top(".subtabs"), 0.5]),
    ("screenshot-big", APP, 1280, 820, 2, "dark", [3, click("#btn-big"), 4, click(".part-pill[data-id=memory]"), 5.5]),
    ("screenshot-big-light", APP, 1280, 820, 2, "light", [3, click("#btn-big"), 4, click(".part-pill[data-id=disks]"), 5.5]),
    ("hero", "/docs/art/hero.html", 1600, 700, 1, "dark", [1.5]),
    ("social-preview", "/docs/art/hero.html", 1280, 640, 1, "dark", [1.5]),
    ("showcase", "/docs/art/showcase.html", 2000, 1125, 1, "dark", [1.5]),
]
# Pictures the install page keeps its own copies of.
COPIES = [("docs/social-preview.png", "site/og.png")]

# The site's own pictures, in site/shots/: cut from the screenshots above and saved as lossless
# WebP (smaller than a scaled-down copy, and it stays sharp). Nothing is resized, except that
# BIG MODE also gets a half-size copy for phones.
BELOW_MOM = (0, 584, 840, 1520)  # the tabs and what's under them, without mom's card
# name, picture in docs/, box to keep (None = all of it), widths to save (None = as it is)
SITE_SHOTS = [
    ("widget-dark", "screenshot-dark", None, [None]),
    ("widget-light", "screenshot-light", None, [None]),
    ("running", "screenshot-dark", BELOW_MOM, [None]),
    ("speed", "screenshot-speed", BELOW_MOM, [None]),
    ("upkeep", "screenshot-light", BELOW_MOM, [None]),
    ("checkup", "screenshot-health", BELOW_MOM, [None]),
    ("rooms", "screenshot-rooms", BELOW_MOM, [None]),
    ("rules", "screenshot-rules", BELOW_MOM, [None]),
    ("details", "screenshot-details", (0, 100, 840, 1520), [None]),
    ("big-dark", "screenshot-big", None, [None, 1280]),
    ("big-light", "screenshot-big-light", None, [None, 1280]),
]


def site_shots(only):
    """Cuts the site's pictures from the ones in docs/. Needs Pillow; without it they stay as they are."""
    jobs = [j for j in SITE_SHOTS if not only or any(w in j[1] for w in only)]
    if not jobs:
        return
    try:
        from PIL import Image
    except ImportError:
        print("site/shots/ not remade: that needs Pillow (pip install pillow)")
        return
    out = os.path.join(ROOT, "site", "shots")
    os.makedirs(out, exist_ok=True)
    for name, source, box, widths in jobs:
        picture = Image.open(os.path.join(ROOT, "docs", source + ".png")).convert("RGB")
        if box:
            picture = picture.crop(box)
        for width in widths:
            if width:
                small = picture.resize((width, round(picture.height * width / picture.width)), Image.LANCZOS)
                small.save(os.path.join(out, f"{name}-{width}.webp"), quality=86, method=6)
                print(f"site/shots/{name}-{width}.webp  {small.width} x {small.height}")
            else:
                picture.save(os.path.join(out, name + ".webp"), lossless=True, quality=100, method=6)
                print(f"site/shots/{name}.webp  {picture.width} x {picture.height}")


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def main():
    only = [a.lower() for a in sys.argv[1:]]
    jobs = [j for j in JOBS if not only or any(w in j[0] for w in only)]
    server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=ROOT))
    port = server.server_address[1]
    threading.Thread(target=server.serve_forever, daemon=True).start()
    profile = tempfile.mkdtemp(prefix="mom-images-")
    debug = 9500 + os.getpid() % 400
    edge = subprocess.Popen([EDGE, "--headless=new", f"--remote-debugging-port={debug}", f"--user-data-dir={profile}", "--enable-unsafe-swiftshader", "--hide-scrollbars", "about:blank"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        page = None
        for _ in range(80):
            try:
                page = next(p for p in json.load(urllib.request.urlopen(f"http://127.0.0.1:{debug}/json")) if p["type"] == "page")
                break
            except Exception:
                time.sleep(0.25)
        if not page:
            sys.exit("Microsoft Edge didn't start")
        ws = websocket.create_connection(page["webSocketDebuggerUrl"], timeout=90, suppress_origin=True)
        count = [0]

        def call(method, params=None):
            count[0] += 1
            ws.send(json.dumps({"id": count[0], "method": method, "params": params or {}}))
            while True:
                reply = json.loads(ws.recv())
                if reply.get("id") == count[0]:
                    return reply

        call("Page.enable")
        for name, path, width, height, ratio, scheme, steps in jobs:
            call("Emulation.setDeviceMetricsOverride", {"width": width, "height": height, "deviceScaleFactor": ratio, "mobile": False})
            call("Emulation.setEmulatedMedia", {"features": [{"name": "prefers-color-scheme", "value": scheme}]})
            call("Page.navigate", {"url": f"http://127.0.0.1:{port}{path}"})
            for step in steps:
                if isinstance(step, (int, float)):
                    time.sleep(step)
                else:
                    call("Runtime.evaluate", {"expression": step, "awaitPromise": True})
            shot = call("Page.captureScreenshot", {"format": "png"})
            out = os.path.join(ROOT, "docs", name + ".png")
            with open(out, "wb") as f:
                f.write(base64.b64decode(shot["result"]["data"]))
            print(f"docs/{name}.png  {width * ratio} x {height * ratio}")
        ws.close()
        for src, dst in COPIES:
            if not only or any(w in src for w in only):
                shutil.copyfile(os.path.join(ROOT, src), os.path.join(ROOT, dst))
                print(f"{dst}  (copy of {src})")
        site_shots(only)
    finally:
        edge.terminate()
        try:
            edge.wait(timeout=10)
        except Exception:
            edge.kill()
        server.shutdown()
        time.sleep(0.5)
        shutil.rmtree(profile, ignore_errors=True)


if __name__ == "__main__":
    main()

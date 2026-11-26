"""Remakes the pictures in docs/ and site/ from the app itself.

    python scripts/images.py            # all of them
    python scripts/images.py hero big   # only the ones whose name contains one of these words

The app's own pages (app/ui, with its sample data) are opened in a headless Microsoft Edge (or
Chrome) and photographed; the composed pictures (docs/art/*.html) are pages too. Nothing is drawn
by hand, so when the app changes, run this again. It makes the same pictures on Windows and on a
Mac: each page is told which system it's on, whatever the browser is really running on.

Needs: Python 3.8+, Microsoft Edge or Google Chrome, and the websocket-client package
(pip install websocket-client). The site's pictures (site/shots/) are cut from the ones in docs/,
and the moving ones (docs/*.gif) are put together from their frames; both need Pillow.
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
BROWSERS = (
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
)
EDGE = next((p for p in BROWSERS if os.path.exists(p)), None) or next((w for w in map(shutil.which, ("msedge", "microsoft-edge", "google-chrome", "chromium")) if w), "msedge")
APP = "/app/ui/index.html"

# What a page is told about the computer it's on: the app speaks Windows or Mac by this.
SYSTEMS = {
    "windows": {"userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36", "platform": "Win32"},
    "mac": {"userAgent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36", "platform": "MacIntel"},
}

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
    ("social-preview", "/docs/art/hero.html", 1280, 640, 1, "dark", [1.5]),
]
# ---------------------------------------------------------------- the README's own pictures
#
# They go in docs/readme/. The widget is photographed at the size its window really is, and
# the last word of a job says which system the page is told it's on ("windows" when there's none).
WINDOW = (500, 800, 2)
TALL = (500, 1000, 2)
inside = lambda frame, sel: f"(document.getElementById({json.dumps(frame)}).contentDocument.querySelector({json.dumps(sel)}) || {{click(){{}}}}).click()"
running = lambda tab: click(f"[data-act=running-tab][data-tab={tab}]")
upkeep = lambda tab: click(f"[data-act=upkeep-tab][data-tab={tab}]")
README = [
    ("readme/hero-dark", "/docs/art/hero.html", 1600, 700, 1, "dark", [1.5]),
    ("readme/hero-light", "/docs/art/hero.html?light", 1600, 700, 1, "light", [1.5]),
    ("readme/family", "/docs/art/family.html", 1500, 700, 1.6, "dark",
     [4, inside("big", "#btn-big"), 4, inside("big", ".part-pill[data-id=memory]"), inside("win", "[data-tab=upkeep]"), 7.5]),
    # What the tiles are cut from.
    ("readme/raw-running", APP, *TALL, "dark", [3]),
    ("readme/raw-homework", APP, *TALL, "dark", [3, running("homework"), 2.5]),
    ("readme/raw-speed", APP, *TALL, "dark", [3, running("speed"), 2.5]),
    ("readme/raw-programs", APP, *TALL, "dark", [3, running("programs"), 2.5]),
    ("readme/raw-browsers", APP, *TALL, "dark", [3, running("browsers"), 3]),
    ("readme/raw-storage", APP, *TALL, "dark", [3, click("[data-tab=upkeep]"), 7]),
    ("readme/raw-checkup", APP, *TALL, "dark", [3, click("[data-tab=upkeep]"), 2.5, upkeep("windows"), 3]),
    ("readme/raw-files", APP, *TALL, "dark", [3, click("[data-tab=upkeep]"), 2.5, upkeep("files"), 3.5]),
    ("readme/raw-rooms", APP, *TALL, "dark", [3, click("[data-tab=rooms]"), 1.5, click("[data-act=open-room]"), 1.5]),
    ("readme/raw-rules", APP, *TALL, "dark", [3, click("[data-tab=rules]"), 1.5]),
    ("readme/raw-mac-checkup", APP, *TALL, "dark", [3, click("[data-tab=upkeep]"), 2.5, upkeep("windows"), 3], "mac"),
    ("readme/raw-mac-apps", APP, *TALL, "dark", [3, click("[data-tab=upkeep]"), 2.5, upkeep("apps"), 2.5], "mac"),
]
# The tiles: the part of a picture under mom's card and the tabs, with its corners rounded.
# name, the raw picture it's cut from.
TILES = ["running", "homework", "speed", "programs", "browsers", "storage", "checkup", "files", "rooms", "rules", "mac-checkup", "mac-apps"]
TILE_TOP, TILE_HEIGHT, TILE_RADIUS = 606, 1180, 44
# The install page shows one of them too: tile, the name it has in site/shots/, how tall.
SITE_TILES = {"mac-checkup": ("mac", 1114)}

# Moving pictures: name, page, width, height, light or dark, system, steps before filming,
# what happens while filming (seconds in, JavaScript), how long, frames a second.
FILMS = [
    ("readme/cleanup", APP, 500, 800, "dark", "windows", [3.5], [(1.6, click("#btn-clean"))], 9.0, 12),
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


def tiles(only):
    """Cuts the README's tiles from the raw pictures, rounds their corners, and throws the raw ones away."""
    names = [t for t in TILES if not only or any(w in f"readme/raw-{t}" or w in f"readme/tile-{t}" for w in only)]
    if not names:
        return
    from PIL import Image, ImageDraw
    folder = os.path.join(ROOT, "docs", "readme")
    for name in names:
        raw = os.path.join(folder, f"raw-{name}.png")
        if not os.path.exists(raw):
            continue
        picture = Image.open(raw).convert("RGBA")
        picture = picture.crop((0, TILE_TOP, picture.width, min(picture.height, TILE_TOP + TILE_HEIGHT)))
        if name in SITE_TILES:
            to, tall = SITE_TILES[name]
            picture.crop((0, 0, picture.width, tall)).convert("RGB").save(os.path.join(ROOT, "site", "shots", to + ".webp"), lossless=True, quality=100, method=6)
            print(f"site/shots/{to}.webp  {picture.width} x {tall}")
        # Drawn four times the size and scaled down, so the corner is smooth.
        big = Image.new("L", (picture.width * 4, picture.height * 4), 0)
        ImageDraw.Draw(big).rounded_rectangle((0, 0, big.width - 1, big.height - 1), radius=TILE_RADIUS * 4, fill=255)
        picture.putalpha(big.resize(picture.size, Image.LANCZOS))
        edge = Image.new("RGBA", (picture.width * 4, picture.height * 4), (0, 0, 0, 0))
        ImageDraw.Draw(edge).rounded_rectangle((3, 3, edge.width - 4, edge.height - 4), radius=TILE_RADIUS * 4 - 3, outline=(255, 255, 255, 34), width=8)
        picture = Image.alpha_composite(picture, edge.resize(picture.size, Image.LANCZOS))
        picture.save(os.path.join(folder, f"tile-{name}.png"), optimize=True)
        os.remove(raw)
        print(f"docs/readme/tile-{name}.png  {picture.width} x {picture.height}")


def film(name, frames, seconds):
    """Puts a moving picture together from its frames: a GIF that plays at the speed it was filmed."""
    from PIL import Image
    import io
    pictures = [Image.open(io.BytesIO(data)).convert("RGB") for _, data in frames]
    # One set of 255 colours for the whole film, picked from frames spread across it.
    some = pictures[:: max(1, len(pictures) // 12)]
    sheet = Image.new("RGB", (some[0].width, some[0].height * len(some)))
    for i, picture in enumerate(some):
        sheet.paste(picture, (0, i * picture.height))
    palette = sheet.quantize(colors=255, method=Image.MEDIANCUT, dither=Image.NONE)
    drawn = [picture.quantize(palette=palette, dither=Image.NONE) for picture in pictures]
    times = [at for at, _ in frames] + [seconds]
    lasts = [max(20, round((times[i + 1] - times[i]) * 1000 / 10) * 10) for i in range(len(frames))]
    # Frames that are the same picture become one that lasts longer.
    kept, held = [drawn[0]], [lasts[0]]
    for picture, last in zip(drawn[1:], lasts[1:]):
        if picture.tobytes() == kept[-1].tobytes():
            held[-1] += last
        else:
            kept.append(picture)
            held.append(last)
    held[-1] += 1600  # a pause on the last picture before it starts over
    out = os.path.join(ROOT, "docs", name + ".gif")
    kept[0].save(out, save_all=True, append_images=kept[1:], duration=held, loop=0, optimize=False, disposal=1)
    print(f"docs/{name}.gif  {kept[0].width} x {kept[0].height}, {len(kept)} frames, {os.path.getsize(out) // 1024} KB")


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def main():
    only = [a.lower() for a in sys.argv[1:]]
    jobs = [j for j in JOBS + README if not only or any(w in j[0] for w in only)]
    films = [f for f in FILMS if not only or any(w in f[0] for w in only)]
    os.makedirs(os.path.join(ROOT, "docs", "readme"), exist_ok=True)
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
        for name, path, width, height, ratio, scheme, steps, *more in jobs:
            call("Emulation.setUserAgentOverride", SYSTEMS[more[0] if more else "windows"])
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
            print(f"docs/{name}.png  {round(width * ratio)} x {round(height * ratio)}")
        for name, path, width, height, scheme, system, before, story, seconds, fps in films:
            call("Emulation.setUserAgentOverride", SYSTEMS[system])
            call("Emulation.setDeviceMetricsOverride", {"width": width, "height": height, "deviceScaleFactor": 1, "mobile": False})
            call("Emulation.setEmulatedMedia", {"features": [{"name": "prefers-color-scheme", "value": scheme}]})
            call("Page.navigate", {"url": f"http://127.0.0.1:{port}{path}"})
            for step in before:
                time.sleep(step) if isinstance(step, (int, float)) else call("Runtime.evaluate", {"expression": step, "awaitPromise": True})
            frames, todo, began = [], sorted(story), time.time()
            while time.time() - began < seconds:
                at = time.time() - began
                while todo and todo[0][0] <= at:
                    call("Runtime.evaluate", {"expression": todo.pop(0)[1]})
                frames.append((time.time() - began, base64.b64decode(call("Page.captureScreenshot", {"format": "png", "optimizeForSpeed": True})["result"]["data"])))
                time.sleep(max(0.0, (len(frames) / fps) - (time.time() - began)))
            film(name, frames, seconds)
        ws.close()
        tiles(only)
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

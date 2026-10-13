"""Shared by the page checks of subscription-audit and bookmark-cleanup. Not a test itself.

Those two pages are approved with a line of their own, such as

    SUBSCRIPTIONS-APPROVE report=<id> cancel=1,4 draft=none ...
    BOOKMARKS-APPROVE report=<id> dupes=1-8 folders=none ...

and not the "<PREFIX> plan=<id> ids=..." line that check_page.py's command line checks. So each
of the two tests checks its own page. The browser is the one check_page.py starts: headless Edge
or Chrome with a throwaway profile inside the test's work folder, driven over a pipe (no port is
opened), stopped (only that one) and deleted afterwards. Here is what the two tests have in
common: opening a file, what went wrong since, what Copy approval hands over, and the checks
every page must pass at every size.
Python 3.8+, standard library only.
"""
import base64
import json
import os
import pathlib
import sys
import time

sys.dont_write_bytecode = True
from check_page import Browser, NoBrowser  # noqa: E402,F401  (NoBrowser is for the tests to catch)

SIZES = ((390, 844, "light"), (390, 844, "dark"), (1280, 900, "light"), (1280, 900, "dark"))
# Press Copy approval and give back what the page handed to the clipboard. The page's clipboard
# call is swapped for one that only keeps the text, so the real clipboard is never written.
COPY = """(async () => {
  window.__copied = null;
  if (navigator.clipboard) navigator.clipboard.writeText = (t) => { window.__copied = t; return Promise.resolve(); };
  document.getElementById('copy').click();
  await new Promise(r => setTimeout(r, 150));
  return window.__copied || document.getElementById('fallback').value;
})()"""


class Pages:
    """Local HTML files opened one at a time in a headless browser started for this test.

    Raises NoBrowser when there is no browser to start. close() stops it and deletes its profile.
    """

    def __init__(self, scratch):
        self.browser = Browser(scratch, dismiss_dialogs=True)
        self.page = self.browser.page

    def close(self):
        self.browser.close()

    def open(self, path, width, height, scheme, settle=0.5):
        """Load a file at this size, light or dark, and wait until it has finished building itself."""
        p = self.page
        del p.events[:]
        p.call("Emulation.setDeviceMetricsOverride", width=width, height=height, deviceScaleFactor=1, mobile=width < 600)
        p.call("Emulation.setEmulatedMedia", features=[{"name": "prefers-color-scheme", "value": scheme}])
        p.js("window.__leaving = true; 0")  # so the page before this one is never taken for the new one
        p.call("Page.navigate", url=pathlib.Path(os.path.abspath(path)).as_uri())
        deadline = time.time() + 30
        while not p.js("document.readyState === 'complete' && !window.__leaving"):
            if time.time() > deadline:
                raise RuntimeError("the page didn't finish loading: %s" % os.path.basename(path))
            time.sleep(0.05)
        p.settle(settle)

    def js(self, expression):
        return self.page.js(expression)

    def settle(self, seconds):
        self.page.settle(seconds)

    def copy(self):
        return self.page.js(COPY)

    def problems(self):
        """Script errors, console errors, alert() calls and every request made since open()."""
        out = {"exceptions": [], "console_errors": [], "dialogs": [], "requests": []}
        for e in self.page.events:
            m, prm = e.get("method"), e.get("params", {})
            if m == "Runtime.exceptionThrown":
                details = prm.get("exceptionDetails", {})
                out["exceptions"].append(str(details.get("exception", {}).get("description") or details)[:300])
            elif m == "Runtime.consoleAPICalled" and prm.get("type") in ("error", "assert"):
                out["console_errors"].append(json.dumps(prm.get("args"))[:300])
            elif m == "Log.entryAdded" and prm.get("entry", {}).get("level") == "error":
                out["console_errors"].append(prm["entry"].get("text", "")[:300])
            elif m == "Page.javascriptDialogOpening":
                out["dialogs"].append(prm.get("message", ""))
            elif m == "Network.requestWillBeSent":
                out["requests"].append(prm.get("request", {}).get("url", ""))
        return out

    def shot(self, path):
        """A picture of the whole page (the first 12,000 px of a very long one). A picture is not a check."""
        try:
            width, height = self.page.js("[window.innerWidth, Math.min(12000, Math.max(document.documentElement.scrollHeight, 600))]")
            data = self.page.call("Page.captureScreenshot", format="png", captureBeyondViewport=True,
                                  clip={"x": 0, "y": 0, "width": width, "height": height, "scale": 1})["data"]
            with open(path, "wb") as f:
                f.write(base64.b64decode(data))
        except (RuntimeError, OSError) as e:
            print("note: no screenshot %s (%s)" % (os.path.basename(path), str(e)[:120]))


def basics(check, pages, tag, scheme):
    """What every report page must pass at every size, light and dark. Call it right after open()."""
    p = pages.problems()
    check(tag + ": no script errors", not p["exceptions"] and not p["console_errors"], p)
    check(tag + ": no alert() ran", not p["dialogs"], p["dialogs"])
    check(tag + ": no network requests", len(p["requests"]) == 1 and p["requests"][0].startswith("file:///"), p["requests"])
    m = pages.js("({sw: document.documentElement.scrollWidth, iw: window.innerWidth, bw: document.body.scrollWidth})")
    check(tag + ": no horizontal scroll", m["sw"] <= m["iw"] and m["bw"] <= m["iw"], m)
    wide = pages.js("""Array.from(document.querySelectorAll('main *')).filter(e => { const r = e.getBoundingClientRect();
        return r.width > 0 && (r.right > window.innerWidth + 0.5 || r.left < -0.5) && !e.closest('.scroll-x') && !e.closest('svg'); })
        .slice(0, 5).map(e => e.tagName + '.' + e.className)""")
    check(tag + ": nothing sticks out of the page", not wide, wide)
    bg = pages.js("getComputedStyle(document.body).backgroundColor")
    check(tag + ": %s background" % scheme, bg == ("rgb(0, 0, 0)" if scheme == "dark" else "rgb(245, 245, 243)"), bg)

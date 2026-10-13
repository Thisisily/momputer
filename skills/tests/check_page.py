"""Opens a built plan page in headless Edge (or Chrome) and checks it the way AUTHORING.md asks.

    python -I check_page.py <page.html> <scratch folder> --prefix DESKTOP-APPROVE [--plan plan.json] [--shots <folder>]

Checks: no script errors, no network requests, no pop-ups, nothing but text made from the data
(no images, frames or outside scripts), no sideways scroll at 390 px and 1280 px, every tickable
list has Tick all and Untick all and they work, Copy approval gives "<PREFIX> plan=<id> ids=...",
light and dark both apply, ticks are remembered.

The browser gets a throwaway profile folder inside <scratch folder>; this script stops the
browser it started (only that one) and deletes the profile. With --shots it saves screenshots
there. Exit code 2 means the browser couldn't be found or started, so nothing was checked.
Python 3.8+, standard library only. It speaks the browser's debugging protocol itself, over a
pipe handed to the browser when it starts: no debugging port is opened on the PC.

The Browser class is also imported by the tests of pages with another approval line
(test_subscription_audit.py, test_bookmark_cleanup.py), through _report_page.py.
"""
import argparse
import base64
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import unquote

sys.dont_write_bytecode = True
BROWSERS = [r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe", r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"]


class Pipe:
    """The browser's debugging pipe: two anonymous pipes instead of a debugging port.

    A port (--remote-debugging-port) is a listener on 127.0.0.1 that any program or user on the PC
    could connect to while a check runs. A pipe has no address: the browser is handed its two
    ends by handle when it is started, and only those two handles are inherited. The messages are
    the same JSON as on a port, each ended by a NUL byte. A thread reads what the browser
    writes, so recv() can wait with a time limit.
    """

    def __init__(self):
        import msvcrt  # Windows only, like the browser paths above
        theirs_read, self.to_browser = os.pipe()     # we write commands, the browser reads them
        self.from_browser, theirs_write = os.pipe()  # the browser writes answers and events, we read them
        self.theirs = [theirs_read, theirs_write]
        for fd in (self.to_browser, self.from_browser):
            msvcrt.setmode(fd, os.O_BINARY)
        handles = [msvcrt.get_osfhandle(fd) for fd in self.theirs]
        for h in handles:
            os.set_handle_inheritable(h, True)
        self.switches = ["--remote-debugging-pipe", "--remote-debugging-io-pipes=%d,%d" % tuple(handles)]  # read handle, write handle
        self.startupinfo = subprocess.STARTUPINFO(lpAttributeList={"handle_list": handles})  # the browser inherits these two, nothing else of ours
        self.inbox, self.reader = queue.Queue(), None

    def started(self):
        """The browser has its own copies of its ends now: close ours, and start reading."""
        self._close(self.theirs)
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        buf = b""
        try:
            while True:
                chunk = os.read(self.from_browser, 65536)
                if not chunk:
                    break
                buf += chunk
                while b"\0" in buf:
                    message, buf = buf.split(b"\0", 1)
                    self.inbox.put(message.decode("utf-8", "replace"))
        except OSError:
            pass
        self.inbox.put(None)  # the browser closed its end

    @staticmethod
    def _close(fds):
        while fds:
            try:
                os.close(fds.pop())
            except OSError:
                pass

    def send(self, text):
        if self.to_browser is None:
            raise OSError("the pipe to the browser is closed")
        data = text.encode("utf-8") + b"\0"
        while data:
            data = data[os.write(self.to_browser, data):]

    def recv(self, timeout=20):
        try:
            text = self.inbox.get(timeout=timeout)
        except queue.Empty:
            raise TimeoutError("no answer from the browser in %s seconds" % timeout)
        if text is None:
            self.inbox.put(None)  # whoever asks next gets the same answer
            raise OSError("the browser closed the connection")
        return text

    def hang_up(self):
        """Close our end of the pipe the browser reads. A browser whose debugging pipe closes quits."""
        if self.to_browser is not None:
            self._close([self.to_browser])
            self.to_browser = None

    def close(self):
        """Call when the browser has gone: the reader has then seen the end, and the last handle can be closed."""
        self.hang_up()
        self._close(self.theirs)
        if self.reader:
            self.reader.join(timeout=5)
        if self.from_browser is not None and not (self.reader and self.reader.is_alive()):
            self._close([self.from_browser])
            self.from_browser = None


class Page:
    """One page of the browser. `session` is how the browser tells its pages apart on the one pipe."""
    patience = 20  # seconds to wait for an answer

    def __init__(self, pipe, dismiss_dialogs=False, session=None):
        self.pipe, self.n, self.events, self.dismiss_dialogs, self.session = pipe, 0, [], dismiss_dialogs, session

    def _send(self, message):
        if self.session:
            message["sessionId"] = self.session
        self.pipe.send(json.dumps(message))

    def _keep(self, m):
        self.events.append(m)
        if self.dismiss_dialogs and m.get("method") == "Page.javascriptDialogOpening":
            # An alert() ran. It stays in events for the caller to find; closing it lets the page carry on.
            self._send({"id": 999998, "method": "Page.handleJavaScriptDialog", "params": {"accept": True}})

    def call(self, method, **params):
        self.n += 1
        self._send({"id": self.n, "method": method, "params": params})
        while True:
            m = json.loads(self.pipe.recv(self.patience))
            if m.get("id") == self.n:
                if "error" in m:
                    raise RuntimeError("%s: %s" % (method, m["error"]))
                return m.get("result", {})
            self._keep(m)

    def js(self, expression):
        r = self.call("Runtime.evaluate", expression=expression, returnByValue=True, awaitPromise=True)
        if r.get("exceptionDetails"):
            raise RuntimeError("script failed: %s" % json.dumps(r["exceptionDetails"])[:400])
        return r["result"].get("value")

    def settle(self, seconds=0.4):
        end = time.time() + seconds
        while time.time() < end:
            try:
                self._keep(json.loads(self.pipe.recv(timeout=0.1)))
            except TimeoutError:
                pass


class NoBrowser(Exception):
    """No Edge or Chrome was found, or it didn't start: nothing can be checked."""


class Browser:
    """A headless Edge or Chrome started for one check, with a throwaway profile folder inside <scratch>.

    It is driven over a pipe (see Pipe): no debugging port is opened. .page is a Page with the
    Page, Runtime, Network and Log events switched on. close() stops the browser that was
    started here (only that one) and deletes the profile; .profile still exists afterwards only
    if something would not let go of it. With dismiss_dialogs, an alert() on the page is noted
    and closed instead of holding everything up.
    """

    def __init__(self, scratch, browser=None, dismiss_dialogs=False):
        browser = browser or next((b for b in BROWSERS if os.path.isfile(b)), None)
        if not browser:
            raise NoBrowser("no Edge or Chrome found")
        self.pipe = self.page = self.proc = None
        self.profile = tempfile.mkdtemp(prefix="page-check-profile-", dir=os.path.abspath(scratch))
        try:
            self.pipe = Pipe()
            self.proc = subprocess.Popen([browser, "--headless=new"] + self.pipe.switches + ["--user-data-dir=" + self.profile, "--no-first-run",
                                          "--no-default-browser-check", "--disable-extensions", "--disable-gpu", "--disable-sync",
                                          "--disable-background-networking", "--disable-component-update", "--disable-background-mode",
                                          "--disable-breakpad", "--window-size=1280,900", "about:blank"],
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, startupinfo=self.pipe.startupinfo)
            self.pipe.started()
            whole = Page(self.pipe)  # the browser itself, to ask which pages it has
            whole.patience = 30
            targets = []
            try:
                for _ in range(50):
                    targets = [t for t in whole.call("Target.getTargets").get("targetInfos", []) if t.get("type") == "page"]
                    if targets:
                        break
                    time.sleep(0.2)
            except OSError:  # it went away, or never answered on the pipe (a browser too old to know the pipe switches)
                raise NoBrowser("the browser didn't start")
            if not targets:
                raise NoBrowser("the browser opened no page")
            session = whole.call("Target.attachToTarget", targetId=targets[0]["targetId"], flatten=True)["sessionId"]
            self.page = Page(self.pipe, dismiss_dialogs, session)
            for domain in ("Page", "Runtime", "Network", "Log"):
                self.page.call(domain + ".enable")
        except BaseException:
            self.close()
            raise

    def close(self):
        """Stop the browser started here, and only that one: ask it to close (it then lets go of its
        files), end its process tree, and end anything still naming this throwaway profile."""
        pipe, proc, self.pipe, self.proc = self.pipe, self.proc, None, None
        if pipe:
            try:
                pipe.send(json.dumps({"id": 999999, "method": "Browser.close"}))
            except OSError:
                pass
            pipe.hang_up()
        if proc:
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True)
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                pass
        if pipe:
            pipe.close()
        for attempt in range(150):  # a busy PC can take a while to let go of the profile's files
            shutil.rmtree(self.profile, ignore_errors=True)
            if not os.path.exists(self.profile):
                break
            if attempt in (10, 60):
                subprocess.run(["powershell", "-NoProfile", "-Command",
                                "Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($env:PAGE_CHECK_PROFILE) } "
                                "| ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }"],
                               env=dict(os.environ, PAGE_CHECK_PROFILE=self.profile), capture_output=True)
            time.sleep(0.3)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("page")
    ap.add_argument("scratch", help="an existing folder; the browser's throwaway profile goes inside it")
    ap.add_argument("--prefix", required=True, help="what the approval line must start with, e.g. DESKTOP-APPROVE")
    ap.add_argument("--plan", help="the plan file built with the page: its plan_id and ids must match the approval")
    ap.add_argument("--shots", help="folder to save screenshots in (must exist)")
    ap.add_argument("--browser", help="path to msedge.exe or chrome.exe")
    args = ap.parse_args(argv)
    page_path = os.path.abspath(args.page)
    if not os.path.isfile(page_path) or not os.path.isdir(args.scratch):
        sys.exit("Need an existing page and an existing scratch folder.")
    browser = args.browser or next((b for b in BROWSERS if os.path.isfile(b)), None)
    if not browser:
        print("not run: no Edge or Chrome found")
        sys.exit(2)
    plan = json.load(open(args.plan, encoding="utf-8")) if args.plan else None
    fails, count = [], [0]

    def check(name, ok, detail=""):
        count[0] += 1
        print(("ok:   " if ok else "FAIL: ") + name + ((" (" + str(detail)[:500] + ")") if detail and not ok else ""))
        if not ok:
            fails.append(name)

    try:
        started = Browser(args.scratch, browser)
    except NoBrowser as e:
        print("not run: %s" % e)
        sys.exit(2)
    profile = started.profile
    try:
        p = started.page
        url = "file:///" + page_path.replace("\\", "/").replace("%", "%25").replace("#", "%23").replace("?", "%3F").replace(" ", "%20")

        def load(width, height=900, scheme=None):
            p.call("Emulation.setDeviceMetricsOverride", width=width, height=height, deviceScaleFactor=1, mobile=width < 600)
            if scheme:
                p.call("Emulation.setEmulatedMedia", features=[{"name": "prefers-color-scheme", "value": scheme}])
            p.call("Page.navigate", url=url)
            for _ in range(100):
                if p.js("document.readyState") == "complete" and p.js("!!document.getElementById('copy')"):
                    break
                time.sleep(0.1)
            p.settle()

        def shot(name):
            """Three pictures: the top of the page, the plan, and the whole page."""
            if not args.shots:
                return
            for part, script, whole in (("top", "window.scrollTo(0, 0)", False),
                                        ("plan", "document.getElementById('groups').scrollIntoView(); window.scrollBy(0, -70)", False),
                                        ("whole", "window.scrollTo(0, 0)", True)):
                p.js(script + "; 0")
                data = p.call("Page.captureScreenshot", format="png", captureBeyondViewport=whole)["data"]
                with open(os.path.join(args.shots, name.replace(".png", "-%s.png" % part)), "wb") as f:
                    f.write(base64.b64decode(data))

        load(1280, scheme="light")
        p.js("try { localStorage.clear(); } catch (e) {}")
        load(1280, scheme="light")
        check("the page loads and builds itself", p.js("document.querySelectorAll('.card').length") > 2)
        light = p.js("getComputedStyle(document.body).backgroundColor + '|' + getComputedStyle(document.body).color")
        wide = p.js("[document.documentElement.scrollWidth, window.innerWidth]")
        check("no sideways scroll at 1280 px", wide[0] <= wide[1], wide)
        shot("1280-light.png")

        boxes = "Array.from(document.querySelectorAll('ul.items input[type=checkbox]'))"
        total = p.js("JSON.parse(document.querySelector('script[type=\"application/json\"]').textContent).actions.length")
        lists = p.js("document.querySelectorAll('ul.items').length")
        check("there is a plan to tick (%d actions in %d lists)" % (total, lists), total > 0 and lists > 0)
        check("every tickable list has its own Tick all and Untick all", p.js(
            "Array.from(document.querySelectorAll('ul.items')).every(function (u) { var t = Array.from(u.parentElement.querySelectorAll('button'))"
            ".map(function (b) { return b.textContent; }); return t.indexOf('Tick all') >= 0 && t.indexOf('Untick all') >= 0; })"))
        p.js("document.getElementById('untick-all').click()")
        check("Untick all (whole plan) unticks everything", p.js(boxes + ".every(function (b) { return !b.checked; })"))
        p.js("document.getElementById('tick-all').click()")
        check("Tick all (whole plan) ticks everything", p.js(boxes + ".every(function (b) { return b.checked; })") and p.js(boxes + ".length") > 0)
        p.js("document.getElementById('untick-all').click()")
        per_list = p.js(
            "Array.from(document.querySelectorAll('ul.items')).map(function (u) {"
            " var bs = Array.from(u.parentElement.querySelectorAll('button'));"
            " bs.filter(function (b) { return b.textContent === 'Tick all'; })[0].click();"
            " var mine = Array.from(document.querySelectorAll('ul.items')).map(function (v) { return Array.from(v.querySelectorAll('input')).filter(function (b) { return b.checked; }).length; });"
            " var all = u.querySelectorAll('input').length;"
            " Array.from(u.parentElement.querySelectorAll('button')).filter(function (b) { return b.textContent === 'Untick all'; })[0].click();"
            " var after = Array.from(document.querySelectorAll('ul.items input')).filter(function (b) { return b.checked; }).length;"
            " return [all, mine.reduce(function (a, b) { return a + b; }, 0), after]; })")
        check("each list's Tick all ticks that list only, and its Untick all clears it", all(a == b and c == 0 and a > 0 for a, b, c in per_list), per_list)

        # Copy approval: read what the page hands to the clipboard
        p.js("navigator.clipboard.writeText = function (t) { window.__copied = t; return Promise.resolve(); }; 0")
        p.js("document.getElementById('tick-all').click(); document.getElementById('copy').click(); 0")
        p.settle(0.2)
        copied = p.js("window.__copied || document.getElementById('fallback').value") or ""
        first = copied.split("\n")[0]
        check("Copy approval gives a line starting \"%s plan=<id> ids=\"" % args.prefix,
              first.startswith(args.prefix + " plan=") and " ids=" in first and "ids=none" not in first, first)
        if plan:
            ids = sorted(a["id"] for a in plan["actions"])
            runs, i = [], 0
            while i < len(ids):
                j = i
                while j + 1 < len(ids) and ids[j + 1] == ids[j] + 1:
                    j += 1
                runs.append(str(ids[i]) if i == j else "%d-%d" % (ids[i], ids[j]))
                i = j + 1
            expect = "%s plan=%s ids=%s" % (args.prefix, plan["plan_id"], ",".join(runs))
            check("with everything ticked the approval names this plan's id and every action", first == expect, (first, expect))
        p.js("document.getElementById('untick-all').click(); document.getElementById('copy').click(); 0")
        p.settle(0.2)
        check("with nothing ticked the approval says ids=none", (p.js("window.__copied") or "").split("\n")[0].endswith(" ids=none"), p.js("window.__copied"))

        # ticks are remembered
        p.js("document.getElementById('untick-all').click(); " + boxes + "[0].click(); 0")
        load(1280, scheme="light")
        remembered = p.js(boxes + ".filter(function (b) { return b.checked; }).length")
        check("ticks are remembered after a reload", remembered == 1, remembered)
        p.js("try { localStorage.clear(); } catch (e) {}")

        load(390, 800, scheme="light")
        narrow = p.js("[document.documentElement.scrollWidth, window.innerWidth, document.body.scrollWidth]")
        check("no sideways scroll at 390 px", narrow[0] <= narrow[1] and narrow[2] <= narrow[1], narrow)
        tiny = p.js("Array.from(document.querySelectorAll('body *')).filter(function (e) { return e.children.length === 0 && e.textContent.trim() "
                    "&& e.getClientRects().length && parseFloat(getComputedStyle(e).fontSize) < 11; }).length")
        check("text stays readable at 390 px (nothing under 11 px)", tiny == 0, tiny)
        shot("390-light.png")
        load(390, 800, scheme="dark")
        dark = p.js("getComputedStyle(document.body).backgroundColor + '|' + getComputedStyle(document.body).color")
        check("light and dark both apply", light != dark and dark.startswith("rgb(0, 0, 0)") and not light.startswith("rgb(0, 0, 0)"), (light, dark))
        shot("390-dark.png")
        load(1280, scheme="dark")
        shot("1280-dark.png")

        foreign = p.js("document.querySelectorAll('img, iframe, object, embed, video, audio, script[src], link[href], form, [onerror], [onclick], [onload]').length")
        check("the data made nothing but text (no images, frames, forms, outside scripts or inline handlers)", foreign == 0, foreign)
        links = p.js("Array.from(document.querySelectorAll('a[href]')).filter(function (a) { return a.protocol !== 'https:' || (a.rel || '').indexOf('noopener') < 0; }).length")
        check("any link is https with rel=noopener", links == 0, links)
        check("the embedded data was not read as markup", p.js("document.querySelectorAll('script').length") == 2 and not p.js("!!window.__pwned"))
        p.settle(0.5)
        errors, requests, dialogs = [], [], []
        for e in p.events:
            m, prm = e.get("method"), e.get("params", {})
            if m == "Runtime.exceptionThrown":
                errors.append(json.dumps(prm.get("exceptionDetails", {}))[:300])
            elif m == "Runtime.consoleAPICalled" and prm.get("type") in ("error", "assert"):
                errors.append(json.dumps(prm.get("args", []))[:300])
            elif m == "Log.entryAdded" and prm.get("entry", {}).get("level") == "error":
                errors.append(prm["entry"].get("text", "")[:300])
            elif m == "Network.requestWillBeSent":
                u = prm.get("request", {}).get("url", "")
                if not (unquote(u).lower() == unquote(url).lower() or u.startswith(("data:", "blob:", "about:"))):
                    requests.append(u[:200])
            elif m == "Page.javascriptDialogOpening":
                dialogs.append(prm.get("message", "")[:100])
        check("no script errors", not errors, errors[:3])
        check("no network requests: the page is one self-contained file", not requests, requests[:5])
        check("no pop-ups", not dialogs, dialogs)
    finally:
        started.close()
    print("\n%s: %d checks on %s%s" % ("ALL PASSED" if not fails else "%d FAILED" % len(fails), count[0], os.path.basename(page_path),
                                       "" if not fails else ": %s" % fails))
    if os.path.exists(profile):
        print("note: the browser profile folder could not be removed: %s" % profile)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()

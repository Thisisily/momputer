"""Checks bookmark-cleanup's dead-link checker. No site is contacted.

    python -I test_bookmark_cleanup_links.py <scratch folder> [<skill folder>] [--keep]

The checker is the one part of the skill that touches the network, so this test never lets it:

  1, 2  the address rules are called as plain functions (which addresses are public, which
        bookmark addresses may be asked at all);
  3, 4  redirects, errors and lookups run against a pretend network made in this test;
  5     the request itself is sent over a socket pair inside this process;
  6     the real command is run on addresses that are all local, private or not web pages, next
        to a small server on 127.0.0.1 that must stay unvisited. It listens on a port the
        system picks, and on port 80 as well, because that is where a plain http://127.0.0.1/
        would land. It looks first: if anything already answers on port 80 it stays away and
        says "not run". The only request the server ever gets is this test's own, at the
        end, to show it was listening.

Names that must count as public are example.net and example.org, which are kept for examples;
the addresses they are given here exist only in the pretend network. The command is run with
LOCALAPPDATA and the home folder pointing into the work folder, so it doesn't look at a real
browser's folders. Nothing outside the work folder inside <scratch folder> is written.

--keep leaves the work folder for a look. Windows only. Python 3.8+, standard library only.
"""
import argparse
import http.server
import json
import os
import socket
import socketserver
import ssl
import sys
import threading
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, Checks, remove_work_folder, run, work_folder  # noqa: E402

MARK = "made-by-test_bookmark_cleanup_links.txt"
PUB, PUB2 = "93.184.216.34", "93.184.216.35"  # public addresses, handed out by the pretend network only
H = "shop.example.net"
U = "https://shop.example.net/page"

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "bookmark-cleanup"))
ap.add_argument("--keep", action="store_true")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
sys.stdout.reconfigure(encoding="ascii", errors="backslashreplace")  # check names hold addresses: print them spelled out
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
if not os.path.isfile(os.path.join(S, "check_links.py")):
    sys.exit("Not found: %s" % os.path.join(S, "check_links.py"))
c = Checks()
check, note = c.check, c.note
pycache_before = os.path.isdir(os.path.join(S, "__pycache__"))
T = work_folder(opts.scratch, "bookmark-cleanup-links-test", MARK)
# The skill asks where browsers keep their files so it never writes there. Here that is a place in the work folder.
ENV = dict(os.environ, LOCALAPPDATA=os.path.join(T, "no-browsers-here"), APPDATA=os.path.join(T, "no-home-here", "AppData", "Roaming"),
           USERPROFILE=os.path.join(T, "no-home-here"), HOME=os.path.join(T, "no-home-here"))


class Net:
    """A pretend network: names resolve to what the test says, and requests are answered from a table."""

    def __init__(self, names, answers):
        self.names, self.answers, self.asked, self.looked_up = names, answers, [], []

    def resolver(self, host, port):
        self.looked_up.append(host)
        v = self.names.get(host, cl.Lookup(True))
        if isinstance(v, Exception):
            raise v
        return v

    def transport(self, ip, host, port, scheme, method, target):
        self.asked.append((ip, host, method, target))
        v = self.answers.get((host, method, target), self.answers.get((host, method), self.answers.get(host, (200, None))))
        if isinstance(v, Exception):
            raise v
        return v

    def run(self, url):
        return cl.check_one(url, self.resolver, self.transport)


class Listener(socketserver.TCPServer):
    """A tiny web server on 127.0.0.1 that writes down every visit. It never shares a port another program holds."""
    allow_reuse_address = False

    def server_bind(self):
        if hasattr(socket, "SO_EXCLUSIVEADDRUSE"):
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        socketserver.TCPServer.server_bind(self)


def port_80_is_free():
    """True when nothing answers on 127.0.0.1:80. A program listening on every address would keep its
    port while a listener on 127.0.0.1 took the calls meant for it, so look before listening there."""
    try:
        socket.create_connection(("127.0.0.1", 80), timeout=0.5).close()
        return False
    except OSError:
        return True


def listen(port, hits):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_HEAD(self):
            hits.append((self.command, self.path))
            self.send_response(200)
            self.end_headers()

        do_GET = do_HEAD

        def log_message(self, *a):
            pass

    server = Listener(("127.0.0.1", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def address_rules():
    # ---------------------------------------------------------------- 1. which addresses are public
    print("\n-- 1. which addresses are public")
    public = ["93.184.216.34", "1.1.1.1", "8.8.8.8", "172.15.255.255", "172.32.0.0", "100.63.255.255", "100.128.0.0", "11.0.0.1",
              "192.167.255.255", "192.169.0.1", "169.253.0.1", "2606:4700:4700::1111", "2a00:1450:4001:81b::200e"]
    not_public = ["127.0.0.1", "127.255.255.254", "10.0.0.1", "10.255.255.255", "172.16.0.1", "172.31.255.255", "192.168.1.1",
                  "192.168.255.255", "169.254.169.254", "100.64.0.1", "100.127.255.255", "0.0.0.0", "0.1.2.3", "192.0.0.1", "192.0.2.1",
                  "198.18.0.1", "198.19.255.255", "198.51.100.7", "203.0.113.9", "224.0.0.1", "239.255.255.250", "240.0.0.1",
                  "255.255.255.255", "::1", "::", "fe80::1", "fe80::1%eth0", "fc00::1", "fd12:3456::1", "::ffff:127.0.0.1",
                  "::ffff:192.168.1.1", "::ffff:8.8.8.8", "64:ff9b::7f00:1", "2002:7f00:1::1", "2001:db8::1", "2001::1", "ff02::1",
                  "fec0::1", "100::1", "not an address", "", "999.1.1.1", "1.2.3"]
    for a in public:
        check("public: " + a, cl.is_public_ip(a) is True)
    for a in not_public:
        check("never requested: %r" % a, cl.is_public_ip(a) is False)

    # ---------------------------------------------------------------- 2. which bookmark addresses may be asked at all
    print("\n-- 2. which bookmark addresses may be asked at all")
    ok = {"https://shop.example.net/a?b=1#c": ("https", "shop.example.net", 443, "/a?b=1"),
          "http://Example.ORG": ("http", "example.org", 80, "/"),
          "https://shop.example.net:443/x%20y": ("https", "shop.example.net", 443, "/x%20y"),
          "https://8.8.8.8/": ("https", "8.8.8.8", 443, "/"),
          "http://[2606:4700:4700::1111]/": ("http", "2606:4700:4700::1111", 80, "/")}
    for u, want in ok.items():
        try:
            got = cl.vet_url(u)
        except cl.Skip as e:
            got = "Skip: %s" % e
        check("may be asked: " + u, got == want, got)
    skip = [("javascript:alert(1)", "not a web page"), ("JaVaScRiPt:alert(1)", "not a web page"), ("file:///etc/passwd", "not a web page"),
            ("data:text/html,x", "not a web page"), ("ftp://files.example.net/", "not a web page"), ("chrome://settings", "not a web page"),
            ("https://user:pw@example.net/", "user name or password"), ("https://user@example.net/", "user name or password"),
            ("https://@example.net/", "user name or password"),
            ("http://example.net:8080/", "unusual port"), ("https://example.net:80/", "unusual port"), ("http://example.net:22/", "unusual port"),
            ("http://127.0.0.1/", "private or local"), ("http://192.168.1.1/admin", "private or local"), ("http://10.0.0.2/", "private or local"),
            ("http://169.254.169.254/latest/meta-data/", "private or local"), ("http://[::1]/", "private or local"),
            ("http://[fe80::1]/", "private or local"), ("http://[::ffff:127.0.0.1]/", "private or local"), ("http://0.0.0.0/", "private or local"),
            ("http://2130706433/", "bare number"), ("http://0x7f000001/", "bare number"), ("http://0177.0.0.1/", "bare number"),
            ("http://127.1/", "bare number"), ("http://0x7f.0.0.1/", "bare number"), ("http://1.2.3/", "bare number"),
            ("http://010.0.0.1/", "bare number"), ("http://shop.example.0x7f/", "bare number"),
            ("http://localhost/", "local network"), ("http://router/", "local network"), ("http://nas/", "local network"),
            ("http://LOCALHOST/", "local network"), ("http://localhost./", "local network"),
            ("http://printer.local/", "isn't on the public internet"), ("http://db.internal/", "isn't on the public internet"),
            ("http://wiki.corp/", "isn't on the public internet"), ("http://app.localhost/", "isn't on the public internet"),
            ("http://thing.home.arpa/", "isn't on the public internet"), ("http://site.example/", "isn't on the public internet"),
            ("http://x.test/", "isn't on the public internet"), ("http://abc.onion/", "isn't on the public internet"),
            ("https://exam ple.net/", "spaces or control"), ("https://example.net/a b", "spaces or control"),
            ("https://good.example\\@evil.example/", "spaces or control"), ("https://example.net/\r\nX-Injected: 1", "spaces or control"),
            ("https://shop.example.net/reset?token=abc123", "a key, a token or an action"),
            ("https://shop.example.net/a?utm_source=x&Code=5", "a key, a token or an action"),
            ("https://shop.example.net/list/unsubscribe/abc", "a key, a token or an action"),
            ("https://shop.example.net/account/Logout", "a key, a token or an action"),
            ("", "empty"), ("https:///nowhere", "no site name"), ("http://under_score.example.net/", "local network")]
    for u, why in skip:
        try:
            got = "allowed: %r" % (cl.vet_url(u),)
        except cl.Skip as e:
            got = str(e)
        check("left alone (%s): %r" % (why, u), why in got, got)


def pretend_network():
    # ---------------------------------------------------------------- 3 and 4. a pretend network
    print("\n-- 3, 4. lookups, redirects and errors, on a pretend network")
    names = {"shop.example.net": [PUB], "new.example.net": [PUB2], "sneaky.example.net": ["10.0.0.9"],
             "both.example.net": [PUB, "192.168.0.7"], "loop.example.net": [PUB], "slow.example.net": [PUB],
             "flaky.example.net": cl.Lookup(False), "empty.example.net": []}
    for name, fn in [("one public address", lambda: cl.vet("https://shop.example.net/", Net(names, {}).resolver)[4] == PUB),
                     ("a name pointing at a private address is refused", lambda: cl.vet("https://sneaky.example.net/", Net(names, {}).resolver)),
                     ("a name pointing at a public AND a private address is refused", lambda: cl.vet("https://both.example.net/", Net(names, {}).resolver))]:
        try:
            r = fn()
            check("lookup: " + name, r is True, r)
        except cl.Skip as e:
            check("lookup: " + name, "private or local" in str(e), e)

    def case(name, url, answers, status, why="", calls=None, never=()):
        net = Net(names, answers)
        r = net.run(url)
        ok = r["status"] == status and why in r["why"] and (calls is None or [x[2] for x in net.asked] == calls)
        ok = ok and not any(x[0] in never or x[1] in never for x in net.asked) and all(cl.is_public_ip(x[0]) for x in net.asked)
        check("check: " + name, ok, (r, net.asked))
        return r, net

    case("200 on HEAD is fine, one request", U, {H: (200, None)}, "ok", calls=["HEAD"])
    case("404 is dead", U, {H: (404, None)}, "dead", "gone (404)", calls=["HEAD"])
    case("410 is dead", U, {H: (410, None)}, "dead", "gone (410)", calls=["HEAD"])
    case("HEAD refused with 405: a ranged GET, then fine", U, {(H, "HEAD"): (405, None), (H, "GET"): (206, None)}, "ok", calls=["HEAD", "GET"])
    case("HEAD refused with 501, GET says 404: dead", U, {(H, "HEAD"): (501, None), (H, "GET"): (404, None)}, "dead", calls=["HEAD", "GET"])
    for code, why in ((403, "refuses automatic checks"), (401, "sign in"), (429, "too many requests"), (500, "problem of its own"),
                      (503, "problem of its own"), (400, "answered 400"), (451, "answered 451")):
        case("%d is 'couldn't reach it', never dead, and no GET" % code, U, {H: (code, None)}, "unreachable", why, calls=["HEAD"])
    case("a redirect to another public site is followed", U, {H: (301, "https://new.example.net/x"), "new.example.net": (200, None)},
         "ok", "moved", calls=["HEAD", "HEAD"])
    case("a relative redirect is followed on the same site", U, {(H, "HEAD", "/page"): (302, "/moved"), (H, "HEAD", "/moved"): (200, None)},
         "ok", "moved", calls=["HEAD", "HEAD"])
    case("a redirect that ends in 404 is dead", U, {H: (301, "https://new.example.net/x"), "new.example.net": (404, None)}, "dead")
    for target in ("http://192.168.1.1/admin", "http://127.0.0.1/", "http://localhost/", "http://[::1]/", "http://169.254.169.254/",
                   "http://router/", "http://sneaky.example.net/", "http://both.example.net/", "file:///etc/passwd",
                   "javascript:alert(1)", "https://user:pw@new.example.net/", "http://new.example.net:8080/", "http://2130706433/",
                   "//127.0.0.1/x", "http://printer.local/"):
        case("a redirect to %s is not followed" % target, U, {H: (302, target)}, "unreachable", "redirects somewhere that isn't requested",
             calls=["HEAD"], never=("192.168.1.1", "127.0.0.1", "::1", "169.254.169.254", "10.0.0.9", "192.168.0.7", "router", "localhost"))
    r, net = case("a redirect loop stops", "https://loop.example.net/a", {"loop.example.net": (302, "https://loop.example.net/a")},
                  "unreachable", "too many times")
    check("...after %d redirects" % cl.MAX_REDIRECTS, len(net.asked) == cl.MAX_REDIRECTS + 1, len(net.asked))
    case("a domain that no longer exists is dead", "https://gone.example.net/", {}, "dead", "domain no longer exists", calls=[])
    case("a lookup that merely failed is 'couldn't reach it'", "https://flaky.example.net/", {}, "unreachable", "couldn't be looked up", calls=[])
    case("a lookup with no addresses is 'couldn't reach it'", "https://empty.example.net/", {}, "unreachable", calls=[])
    case("a redirect to a domain that doesn't exist is not 'dead'", U, {H: (302, "https://gone.example.net/")}, "unreachable")
    case("a timeout is 'couldn't reach it'", U, {H: socket.timeout()}, "unreachable", "no answer in time")
    case("a refused connection is 'couldn't reach it'", U, {H: ConnectionRefusedError()}, "unreachable", "refused the connection")
    case("a bad certificate is 'couldn't reach it'", U, {H: ssl.SSLCertVerificationError()}, "unreachable", "certificate")
    case("a dropped connection is 'couldn't reach it'", U, {H: ConnectionResetError()}, "unreachable", "couldn't connect")
    for u in ("http://192.168.1.1/admin", "http://127.0.0.1:8080/", "http://localhost/", "https://user:hunter2@vault.example/login",
              "javascript:alert(1)", "http://sneaky.example.net/", "http://both.example.net/"):
        case("never requested at all: " + u, u, {}, "skipped", calls=[])


def what_is_sent():
    # ---------------------------------------------------------------- 5. what is really sent (over a socket pair, no network)
    print("\n-- 5. what is really sent, over a socket pair inside this test")
    seen = {}

    def pair_connect(answer):
        def connect(addr, timeout):
            seen["addr"] = addr
            near, far = socket.socketpair()

            def serve():
                data = b""
                while b"\r\n\r\n" not in data:
                    chunk = far.recv(4096)
                    if not chunk:
                        break
                    data += chunk
                seen["request"] = data.decode("latin-1")
                far.sendall(answer)
                far.close()
            threading.Thread(target=serve, daemon=True).start()
            return near
        return connect

    status, loc = cl.request(PUB, "shop.example.net", 80, "http", "HEAD", "/a?b=1", connect=pair_connect(
        b"HTTP/1.1 301 Moved\r\nLocation: https://new.example.net/x\r\nSet-Cookie: track=1\r\nContent-Length: 0\r\n\r\n"))
    lines = seen["request"].split("\r\n")
    headers = {ln.split(":", 1)[0].lower(): ln.split(":", 1)[1].strip() for ln in lines[1:] if ":" in ln}
    check("request: goes to the address that was checked, not to a name", seen["addr"] == (PUB, 80), seen["addr"])
    check("request: HEAD with the path and query, nothing after #", lines[0] == "HEAD /a?b=1 HTTP/1.1", lines[0])
    check("request: exactly four headers, no cookies, no login, no referrer", sorted(headers) == ["accept", "connection", "host", "user-agent"], headers)
    check("request: honest User-Agent", headers.get("user-agent") == cl.USER_AGENT and "bookmark-cleanup" in cl.USER_AGENT and "Mozilla" not in cl.USER_AGENT)
    check("request: Host is the site's name, Connection: close", headers.get("host") == "shop.example.net" and headers.get("connection") == "close")
    check("request: status and Location come back", (status, loc) == (301, "https://new.example.net/x"), (status, loc))
    status, loc = cl.request(PUB, "shop.example.net", 80, "http", "GET", "/big", connect=pair_connect(
        b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/999999\r\nContent-Length: 1\r\n\r\nX"))
    headers = {ln.split(":", 1)[0].lower(): ln.split(":", 1)[1].strip() for ln in seen["request"].split("\r\n")[1:] if ":" in ln}
    check("request: the GET asks for one byte only", headers.get("range") == "bytes=0-0" and status == 206 and
          sorted(headers) == ["accept", "connection", "host", "range", "user-agent"], headers)


def local_server(W):
    # ---------------------------------------------------------------- 6. a local server that must stay unvisited
    print("\n-- 6. the real command, beside a local server that must stay unvisited")
    os.makedirs(W)
    hits, hits80 = [], []
    servers = [listen(0, hits)]  # a port the system picks
    port = servers[0].server_address[1]
    try:
        if port_80_is_free():
            try:
                servers.append(listen(80, hits80))  # where a plain http://127.0.0.1/ would land
            except OSError:
                pass
        on80 = len(servers) == 2
        if not on80:
            note("port 80 on 127.0.0.1 is in use or can't be opened here, so no listener stood where a plain http://127.0.0.1/ "
                 "would land (those addresses were still checked as left alone)")
        local = ["http://127.0.0.1:%d/hit" % port, "http://localhost:%d/hit" % port, "http://127.0.0.1/hit", "http://localhost/hit",
                 "http://[::1]:%d/hit" % port, "http://2130706433:%d/hit" % port, "http://2130706433/hit", "http://127.1/hit",
                 "http://0177.0.0.1/hit", "http://0x7f000001/hit", "http://192.168.1.1/admin", "http://10.0.0.1/", "http://router/",
                 "http://printer.local/", "https://user:hunter2@vault.example/login", "javascript:alert(1)", "file:///C:/secret.txt",
                 "https://site.example/page", "http://[::1]/hit"]
        nodes = [{"id": i, "type": "url", "parent": 0, "title": "t%d" % i, "url": u} for i, u in enumerate(local, 1)]
        bm = os.path.join(W, "bookmarks.json")
        json.dump({"schema": "bookmark-cleanup/bookmarks@1", "source": {"kind": "html"}, "nodes": nodes}, open(bm, "w", encoding="utf-8"))
        out = os.path.join(W, "links.json")
        r = run([PY, "-I", os.path.join(S, "check_links.py"), bm, "--out", out], env=ENV)
        nothing_to_ask = '"would_request": 0' in r.stdout
        check("without the person's yes: nothing is requested and nothing is written", r.returncode == 2 and not os.path.exists(out)
              and nothing_to_ask and not hits and not hits80, (r.stdout, r.stderr, hits, hits80))
        if nothing_to_ask:
            r = run([PY, "-I", os.path.join(S, "check_links.py"), bm, "--out", out, "--person-agreed"], env=ENV)
            res = json.load(open(out, encoding="utf-8"))["results"] if os.path.exists(out) else {}
            check("with the yes: every local, private and non-web address is still left alone", r.returncode == 0 and len(res) == len(local)
                  and all(v["status"] == "skipped" for v in res.values()), (r.stdout[-300:], r.stderr[-300:]))
        else:  # it would have asked somebody: this test never lets it
            check("with the yes: every local, private and non-web address is still left alone", False, "not tried: " + r.stdout[-300:])
        check("the local server was never visited", hits == [], hits)
        if on80:
            check("the listener on port 80 was never visited either", hits80 == [], hits80)
        check("the output prints no addresses", "127.0.0.1" not in r.stdout and "hunter2" not in r.stdout and "vault" not in r.stdout, r.stdout)
        for u in local[:10]:
            res1 = cl.check_one(u)
            check("check_one leaves %s alone" % u.replace(":%d/" % port, ":PORT/"), res1["status"] == "skipped", res1)
        check("still unvisited after calling the checker directly", hits == [] and hits80 == [], (hits, hits80))
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))  # straight to this PC, never through a proxy
        opener.open("http://127.0.0.1:%d/alive" % port, timeout=5).read()  # the test's own request: the server does log visits
        check("(the server was up and would have logged a visit)", hits == [("GET", "/alive")], hits)
        if on80:
            opener.open("http://127.0.0.1/alive", timeout=5).read()
            check("(so was the listener on port 80)", hits80 == [("GET", "/alive")], hits80)
    finally:
        for server in servers:
            server.shutdown()
            server.server_close()


try:
    sys.path.insert(0, S)
    import check_links as cl  # noqa: E402  (the skill's own)
    address_rules()
    pretend_network()
    what_is_sent()
    local_server(os.path.join(T, "work"))
    check("no __pycache__ left in the skill folder", pycache_before or not os.path.isdir(os.path.join(S, "__pycache__")))
finally:
    if not opts.keep:
        remove_work_folder(T, MARK)
    else:
        print("\nkept for a look: %s" % T)
c.finish("bookmark-cleanup's link checker")

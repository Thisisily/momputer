#!/usr/bin/env python3
"""Ask each bookmarked site whether the page is still there. Optional, and only when the person agrees.

    python check_links.py bookmarks.json --out links.json                    # says what it would do, contacts nothing
    python check_links.py bookmarks.json --out links.json --person-agreed    # does it

This is the one part of the skill that touches the network, so it is strict:

  * http and https only. Everything else is listed as "not a web page" and left alone.
  * No cookies, no saved logins, no credentials, no proxy settings. An address with a user
    name or password in it is never requested.
  * The name is looked up first, and nothing is requested unless every address it points to
    is a public one. Routers, printers, NAS boxes, localhost, company-internal pages and
    anything on a private, loopback or link-local address are never requested: on such
    pages even a plain look can change something. The request then goes to exactly the
    address that was checked, so the answer can't be switched in between.
  * Only the standard ports. No address written in an odd numeric form. No address that
    carries a key or a token, or that is an action such as /logout or /unsubscribe.
  * HEAD first. A ranged GET (one byte) only if the site refuses HEAD. The body is never read.
  * A redirect is followed only to another public web address, a few times at most.
  * Short timeouts, four at a time, one at a time per site with a pause, a cap on the total.
  * An honest User-Agent that says what this is.
  * A link is called dead only on a clear 404 or 410, or when its domain no longer exists.
    Everything else that goes wrong means "couldn't reach it: check by hand".

There is no switch that turns any of this off. The script prints counts only; addresses and
titles are other people's text and are never printed.

Python 3.8+, standard library only.
"""
import argparse
import http.client
import ipaddress
import json
import os
import re
import socket
import ssl
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import date
from urllib.parse import quote, urljoin, urlsplit

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # leave no __pycache__ behind in the skill folder
from read_bookmarks import may_write  # noqa: E402  (where a file may be written, and where never)

SCHEMA = "bookmark-cleanup/links@1"
USER_AGENT = "bookmark-cleanup-link-check/1.0 (a person checking their own saved bookmarks; one request per page)"
CONNECT_TIMEOUT = 6
READ_TIMEOUT = 8
WORKERS = 4
PER_SITE_PAUSE = 1.0
MAX_REDIRECTS = 4
DEFAULT_MAX = 500
HARD_MAX = 3000
MAX_SECONDS = 900
PORTS = {"http": 80, "https": 443}
REFUSES_HEAD = (405, 501)

# Addresses that are not the public internet. Written out, so the answer doesn't depend on the Python version.
_NOT_PUBLIC_4 = [ipaddress.ip_network(n) for n in (
    "0.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8", "169.254.0.0/16", "172.16.0.0/12", "192.0.0.0/24",
    "192.0.2.0/24", "192.88.99.0/24", "192.168.0.0/16", "198.18.0.0/15", "198.51.100.0/24", "203.0.113.0/24",
    "224.0.0.0/4", "240.0.0.0/4")]
_PUBLIC_6 = ipaddress.ip_network("2000::/3")
_NOT_PUBLIC_6 = [ipaddress.ip_network(n) for n in (
    "2001::/23", "2001:db8::/32", "2002::/16", "3fff::/20", "5f00::/16")]
# Names that never lead to a public website: the local machine, the local network, reserved endings.
_LOCAL_ENDINGS = ("localhost", "local", "localdomain", "internal", "intranet", "lan", "home", "corp", "private",
                  "home.arpa", "test", "example", "invalid", "onion", "alt")


# Addresses that carry a secret or do something when visited: magic sign-in links, password resets,
# one-click unsubscribes. Without cookies most are harmless, but "most" isn't the standard here.
_KEY_NAMES = set("token access_token auth auth_token authtoken key api_key apikey sig signature secret password passwd pwd "
                 "pass session sessionid session_id sid jwt code otp nonce ticket invite confirm confirmation reset verify "
                 "verification magic login_token".split())
_ACTION_WORDS = set("logout log-out signout sign-out unsubscribe optout opt-out".split())
_PLAIN_V4 = re.compile(r"(?:0|[1-9][0-9]{0,2})(?:\.(?:0|[1-9][0-9]{0,2})){3}")
_TLS = []


def tls():
    if not _TLS:
        _TLS.append(ssl.create_default_context())  # checks the certificate and the name; built once
    return _TLS[0]


class Skip(Exception):
    """This address must not be requested. The message says why, in words for the report."""


class Lookup(Exception):
    def __init__(self, gone):
        Exception.__init__(self, "gone" if gone else "failed")
        self.gone = gone


def is_public_ip(value):
    """True only for an address on the public internet."""
    try:
        ip = value if isinstance(value, (ipaddress.IPv4Address, ipaddress.IPv6Address)) else ipaddress.ip_address(str(value).split("%")[0])
    except ValueError:
        return False
    if isinstance(ip, ipaddress.IPv6Address):
        if ip.ipv4_mapped is not None:
            return False  # ::ffff:10.0.0.1 and friends: an IPv4 address in disguise is never requested
        if ip not in _PUBLIC_6 or any(ip in n for n in _NOT_PUBLIC_6):
            return False
    elif any(ip in n for n in _NOT_PUBLIC_4):
        return False
    return not (ip.is_private or ip.is_loopback or ip.is_link_local or ip.is_multicast or ip.is_reserved
                or ip.is_unspecified or getattr(ip, "is_site_local", False))


def vet_url(url):
    """(scheme, host, port, target) for an address that may be requested, or raise Skip with the reason."""
    if not isinstance(url, str) or not url.strip():
        raise Skip("empty address")
    url = url.strip()
    if re.search(r"[\x00-\x20\x7f\\]", url):
        raise Skip("the address has spaces or control characters in it")
    try:
        p = urlsplit(url)
        scheme, host, port = p.scheme.lower(), p.hostname, p.port
    except ValueError:
        raise Skip("the address can't be read")
    if scheme not in PORTS:
        raise Skip("not a web page")
    if "@" in p.netloc or p.username is not None or p.password is not None:
        raise Skip("has a user name or password in it")
    if not host:
        raise Skip("the address has no site name")
    if port not in (None, PORTS[scheme]):
        raise Skip("uses an unusual port")
    host = host.rstrip(".").lower()
    if ":" in host or _PLAIN_V4.fullmatch(host):
        # An address written out in full. It is read strictly (no leading zeros, no short forms) and
        # used in exactly that form, so "0177.0.0.1" can't mean one thing here and another to the system.
        try:
            ip = ipaddress.ip_address(host)
        except ValueError:
            raise Skip("the address can't be read")
        if not is_public_ip(ip):
            raise Skip("a private or local address")
        host = str(ip)
    else:
        if re.fullmatch(r"[0-9]+|0x[0-9a-f]*", host.rsplit(".", 1)[-1]):
            raise Skip("an address written as a bare number")  # 2130706433, 127.1, 0x7f.0.0.1, 0177.0.0.1 ...
        try:
            host = host.encode("idna").decode("ascii")
        except UnicodeError:
            raise Skip("the site name can't be read")
        if not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+", host):
            raise Skip("a name on the local network")  # one word such as "router", or not a proper name
        if any(host == e or host.endswith("." + e) for e in _LOCAL_ENDINGS):
            raise Skip("a name that isn't on the public internet")
    names = {kv.split("=", 1)[0].lower() for kv in re.split(r"[&;]", p.query) if kv}
    if names & _KEY_NAMES or any(part.lower() in _ACTION_WORDS for part in p.path.split("/")):
        raise Skip("has a key, a token or an action in its address")  # asking could use up a one-time link or do the action
    target = quote(p.path or "/", safe="/%:@!$&'()*+,;=~-._") + ("?" + quote(p.query, safe="/%:@!$&'()*+,;=~-._?") if p.query else "")
    return scheme, host, PORTS[scheme], target


def resolve(host, port):
    """Every address a name points to. Raises Lookup(gone=True) when the domain doesn't exist."""
    try:
        infos = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    except socket.gaierror as e:
        gone = e.errno in (getattr(socket, "EAI_NONAME", None), 11001)  # 11001: Windows "host not found"
        raise Lookup(gone)
    except (OSError, UnicodeError):
        raise Lookup(False)
    return sorted({i[4][0] for i in infos}, key=lambda a: (":" in a, a))


def vet(url, resolver=resolve):
    """Everything needed to request an address, after checking it leads only to public addresses."""
    scheme, host, port, target = vet_url(url)
    try:
        ipaddress.ip_address(host)
        addresses = [host]
    except ValueError:
        addresses = [str(a) for a in resolver(host, port)]
    if not addresses:
        raise Lookup(False)
    if not all(is_public_ip(a) for a in addresses):
        raise Skip("points to a private or local address")
    return scheme, host, port, target, addresses[0]


def request(ip, host, port, scheme, method, target, connect=socket.create_connection):
    """One HEAD or ranged GET to exactly the address that was vetted. Returns (status, Location header)."""
    sock = connect((ip, port), CONNECT_TIMEOUT)
    try:
        if scheme == "https":
            sock = tls().wrap_socket(sock, server_hostname=host)
        sock.settimeout(READ_TIMEOUT)
        conn = http.client.HTTPConnection(host, port)  # used to speak HTTP only: the socket is the one made above
        conn.sock = sock
        conn.putrequest(method, target, skip_host=True, skip_accept_encoding=True)
        conn.putheader("Host", "[%s]" % host if ":" in host else host)
        conn.putheader("User-Agent", USER_AGENT)
        conn.putheader("Accept", "*/*")
        if method == "GET":
            conn.putheader("Range", "bytes=0-0")
        conn.putheader("Connection", "close")
        conn.endheaders()
        resp = conn.getresponse()  # headers only: the body is never read
        return resp.status, resp.getheader("Location")
    finally:
        try:
            sock.close()
        except OSError:
            pass


def check_one(url, resolver=resolve, transport=request):
    """{'status': ok | dead | unreachable | skipped, 'why': ..., 'code': ...} for one bookmark address."""
    current, hops = url, 0
    while True:
        try:
            scheme, host, port, target, ip = vet(current, resolver)
        except Skip as e:
            if hops:
                return {"status": "unreachable", "why": "it redirects somewhere that isn't requested (%s)" % e}
            return {"status": "skipped", "why": str(e)}
        except Lookup as e:
            if e.gone and not hops:
                return {"status": "dead", "why": "the domain no longer exists", "host": host_of(current)}
            return {"status": "unreachable", "why": "the site's name couldn't be looked up"}
        try:
            status, location = transport(ip, host, port, scheme, "HEAD", target)
            if status in REFUSES_HEAD:
                status, location = transport(ip, host, port, scheme, "GET", target)
        except socket.timeout:
            return {"status": "unreachable", "why": "no answer in time"}
        except ssl.SSLCertVerificationError:
            return {"status": "unreachable", "why": "its security certificate isn't valid"}
        except ssl.SSLError:
            return {"status": "unreachable", "why": "a secure connection couldn't be made"}
        except ConnectionRefusedError:
            return {"status": "unreachable", "why": "the site refused the connection"}
        except (OSError, http.client.HTTPException):
            return {"status": "unreachable", "why": "couldn't connect"}
        if status in (301, 302, 303, 307, 308) and location:
            hops += 1
            if hops > MAX_REDIRECTS:
                return {"status": "unreachable", "why": "it redirects too many times", "code": status}
            current = urljoin(current, location)
            continue
        if 200 <= status < 400:
            return {"status": "ok", "why": "moved, and the new place answers" if hops else "answers", "code": status}
        if status in (404, 410):
            return {"status": "dead", "why": "the site says the page is gone (%d)" % status, "code": status}
        why = {401: "asks you to sign in", 403: "refuses automatic checks", 429: "says too many requests"}.get(
            status, "the site has a problem of its own (%d)" % status if status >= 500 else "answered %d" % status)
        return {"status": "unreachable", "why": why, "code": status}


def host_of(url):
    try:
        return (urlsplit(url).hostname or "").lower()
    except ValueError:
        return ""


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description="Check which bookmarked pages still answer (contacts every site).")
    ap.add_argument("bookmarks", help="bookmarks.json from read_bookmarks.py")
    ap.add_argument("--out", required=True, help="links.json to write, in the scratch folder")
    ap.add_argument("--person-agreed", action="store_true",
                    help="the person said yes to contacting every bookmarked site; without this nothing is requested")
    ap.add_argument("--max", type=int, default=DEFAULT_MAX, help="check at most this many pages (default %d)" % DEFAULT_MAX)
    a = ap.parse_args(argv)
    with open(a.bookmarks, encoding="utf-8") as f:
        doc = json.load(f)
    if not isinstance(doc, dict) or not str(doc.get("schema", "")).startswith("bookmark-cleanup/bookmarks@"):
        sys.exit("%s is not a bookmarks.json from read_bookmarks.py" % a.bookmarks)
    may_write(a.out, ".json", "bookmark-cleanup/links@")
    cap = max(1, min(a.max, HARD_MAX))
    by_url = {}
    for n in doc.get("nodes") or []:
        if isinstance(n, dict) and n.get("type") == "url" and isinstance(n.get("url"), str):
            by_url.setdefault(n["url"], []).append(n["id"])
    results, todo = {}, []
    for url in by_url:
        try:
            vet_url(url)
            todo.append(url)
        except Skip as e:
            results[url] = {"status": "skipped", "why": str(e)}
    capped = len(todo) > cap
    for url in todo[cap:]:
        results[url] = {"status": "skipped", "why": "not checked this time (over the limit)"}
    todo = todo[:cap]
    sites = len({host_of(u) for u in todo})
    if not a.person_agreed:
        json.dump({"would_request": len(todo), "sites": sites, "left_alone": len(by_url) - len(todo),
                   "note": "Nothing was requested. This contacts every one of those sites once. Ask the person; "
                           "if they agree, run again with --person-agreed."}, sys.stdout, indent=1)
        sys.stdout.write("\n")
        sys.exit(2)

    locks, last, guard = {}, {}, threading.Lock()
    deadline = time.time() + MAX_SECONDS

    def work(url):
        if time.time() > deadline:
            return url, {"status": "skipped", "why": "not checked this time (ran out of time)"}
        host = host_of(url)
        with guard:
            lock = locks.setdefault(host, threading.Lock())
        with lock:  # one request at a time per site, with a pause between them
            wait = last.get(host, 0) + PER_SITE_PAUSE - time.time()
            if wait > 0:
                time.sleep(wait)
            try:
                res = check_one(url)
            except Exception:  # whatever went wrong, it is not proof the page is gone
                res = {"status": "unreachable", "why": "the check itself failed"}
            last[host] = time.time()
        return url, res

    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        for url, res in pool.map(work, todo):
            results[url] = res

    # "The domain no longer exists" is a strong claim. If no name could be looked up at all, the
    # computer is offline; and a name that failed once gets a second look before it is believed.
    gone = [u for u in todo if results[u].get("status") == "dead" and "host" in results[u]]
    reached = any(results[u].get("status") in ("ok", "dead", "unreachable") and "host" not in results[u]
                  and results[u].get("why") != "the site's name couldn't be looked up" for u in todo)
    for u in gone:
        host = results[u].pop("host")
        still = False
        if reached:
            try:
                resolve(host, 443)
            except Lookup as e:
                still = e.gone
        if not still:
            results[u] = {"status": "unreachable", "why": "its name couldn't be looked up (is this computer online?)"}

    per_node, counts = {}, {}
    for url, ids in by_url.items():
        res = results.get(url, {"status": "skipped", "why": "not checked"})
        for i in ids:
            per_node[str(i)] = res
        counts[res["status"]] = counts.get(res["status"], 0) + len(ids)
    out = os.path.abspath(a.out)
    with open(out, "w", encoding="utf-8") as f:
        json.dump({"schema": SCHEMA, "checked_on": date.today().isoformat(), "requested": len(todo), "sites": sites,
                   "capped": capped, "user_agent": USER_AGENT, "results": per_node}, f, ensure_ascii=False)
    json.dump({"written": out, "pages_requested": len(todo), "sites": sites, "bookmarks": counts, "capped": capped,
               "note": "'dead' means a clear 404 or 410, or a domain that no longer exists. 'unreachable' means it "
                       "couldn't be reached this time: those are for the person to check by hand, not to remove."},
              sys.stdout, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()

"""Made-up bookmarks for the bookmark-cleanup tests. Not a test itself.

    write_bookmarks(folder)

writes a pretend browser tree, as if <folder>/local were %LOCALAPPDATA%:

  local/Microsoft/Edge/User Data/Default/Bookmarks       the big one (Chromium JSON), with a
                                                         Bookmarks.bak beside it and a guest profile
  local/Google/Chrome/User Data/Profile 1/Bookmarks      a small one

and exported.html, a bookmarks file as Firefox would export it.

Everything is invented and every site is a .example name, a local address or not a web page at
all, so nothing here leads anywhere. Some titles and addresses are hostile on purpose. The same
files come out every time.
Python 3.8+, standard library only.
"""
import html
import json
import os
import random
from datetime import datetime

LQ, RQ, LS, RS = "\u201c", "\u201d", "\u2018", "\u2019"
HOSTILE_TITLES = [
    "<img src=x onerror=alert(1)>",
    "</script><script>alert(2)</script>",
    "\"quoted\" & 'single' " + LQ + "curly" + RQ + " " + LS + "quotes" + RS,
    "$(calc) `whoami` ; rm -rf & echo",
    "</A><DT><A HREF=\"javascript:alert(3)\">injected</A>",
    "two\nlines\tand a tab",
    "",
]


def ts(y, m=1, d=1):
    return str(int((datetime(y, m, d, 12) - datetime(1601, 1, 1)).total_seconds() * 1000000))


def write_bookmarks(out):
    """Write the pretend browser tree and exported.html into the folder `out` (made if it isn't there)."""
    rnd = random.Random(7)
    ids = [0]

    def url(name, address, added=(2022, 3, 4), used=None, has_used=True):
        ids[0] += 1
        n = {"type": "url", "name": name, "url": address, "date_added": ts(*added), "guid": "g%d" % ids[0], "id": str(ids[0])}
        if has_used:
            n["date_last_used"] = ts(*used) if used else "0"
        return n

    def folder(name, children, added=(2020, 1, 1)):
        ids[0] += 1
        return {"type": "folder", "name": name, "children": children, "date_added": ts(*added), "date_modified": ts(2026, 1, 1),
                "guid": "g%d" % ids[0], "id": str(ids[0])}

    def root(name, children):
        ids[0] += 1
        return {"type": "folder", "name": name, "children": children, "date_added": ts(2018), "date_modified": ts(2026), "guid": "r%d" % ids[0], "id": str(ids[0])}

    bar = []
    for i in range(1, 21):
        used = (2026, 9, i) if i <= 8 else (None if i <= 14 else (2024, 5, i))
        bar.append(url("Bar site %d" % i, "https://bar-%d.example/" % i, (2021, 2, i), used))
    bar.append(folder("Work", [url("Tessaract dashboard", "https://tessaract.example/dash", (2023, 5, 1), (2026, 10, 1)),
                               url("Northquay docs", "https://docs.northquay.example/start", (2023, 6, 1), (2026, 8, 3)),
                               url("Marrowgate bills", "https://marrowgate.example/bills", (2024, 1, 9), (2026, 9, 15))]))
    bar.append(folder("Recipes", [url("Sablewick roast", "https://sablewick.example/roast", (2022, 11, 2), (2026, 2, 2)),
                                  url("Greengrocer soup", "https://greengrocer.example/soup", (2022, 11, 3), (2025, 12, 24))]))
    bar.append(url("Reader mode (bookmarklet)", "javascript:(function(){document.body.style.maxWidth='40em';alert('reader')})()", (2020, 4, 4), (2026, 9, 9)))
    bar.append(url("Lanternfall", "https://lanternfall.example/watch", (2021, 7, 7), (2026, 10, 10)))
    bar.append(url("Vault (old login link)", "https://user:hunter2@vault.example/login", (2019, 3, 3), None))
    bar.append(url("Router", "http://192.168.1.1/admin", (2019, 8, 8), (2026, 6, 6)))

    pile = []
    for i in range(60):
        pile.append(url("Copperline docs page %d" % i, "https://docs.copperline.example/page-%d" % i, (2019 + i % 5, 1 + i % 12, 1 + i % 27), None))
    for i in range(40):
        pile.append(url("Lanternfall show %d" % i, "https://www.lanternfall.example/show/%d" % i, (2020 + i % 4, 1 + i % 12, 2), (2023, 1 + i % 12, 5) if i % 3 == 0 else None))
    for i in range(12):
        pile.append(url("Quillhaven article %d" % i, "https://www.quillhaven.example/a/%d" % i, (2022, 6, 1 + i), (2026, 3, 1 + i)))
    for i in range(4):
        pile.append(url("Brindlewood class %d" % i, "https://brindlewood.example/class/%d" % i, (2025, 11, 1 + i), None))
    for i in range(64):
        pile.append(url("Single %d" % i, "https://site-%d.example/" % i, (2018 + i % 8, 1 + i % 12, 3), (2026, 1, 1 + i % 27) if i % 5 == 0 else None))
    rnd.shuffle(pile)

    reading = [
        url("Osprey list", "https://www.osprey-books.example/list/", (2022, 1, 1), (2025, 12, 1)),
        url("Osprey list (from a newsletter)", "http://osprey-books.example/list?utm_source=news&utm_medium=email", (2023, 2, 2), None),
        url("Osprey list, reviews", "https://osprey-books.example/list#reviews", (2023, 3, 3), (2024, 4, 4)),
        url("Osprey guide, chapter 2", "https://osprey-books.example/guide#chapter-2", (2023, 4, 4), (2026, 5, 5)),
        url("Osprey guide, chapter 5", "https://osprey-books.example/guide#chapter-5", (2023, 4, 5), (2026, 5, 6)),
        url("Mail: inbox", "https://mail-app.example/#/inbox", (2024, 1, 1), (2026, 10, 12)),
        url("Mail: settings", "https://mail-app.example/#/settings", (2024, 1, 2), (2026, 7, 7)),
    ]
    odd = [url(t, "https://odd-%d.example/page" % i, (2021, 2, 1 + i), None) for i, t in enumerate(HOSTILE_TITLES)]
    odd += [url(HOSTILE_TITLES[1], "https://twice.example/", (2022, 2, 2), (2026, 2, 2)), url(HOSTILE_TITLES[0], "https://twice.example", (2023, 2, 2), None)]
    odd += [
        url("Mixed-case script", "JaVaScRiPt:alert(4)", (2021, 1, 1), None),
        url("Script with a space in front", " javascript:alert(5)", (2021, 1, 2), None),
        url("Data page", "data:text/html,<script>alert(6)</script>", (2021, 1, 3), None),
        url("Old script kind", "vbscript:msgbox(1)", (2021, 1, 4), None),
        url("Backslash trick", "https://good.example\\@evil.example/", (2021, 1, 5), (2026, 1, 5)),
        url("Quote in the address", "https://quote.example/\"onmouseover=\"alert(7)", (2021, 1, 6), None),
        url("Tag in the address", "https://tag.example/<script>alert(8)</script>", (2021, 1, 7), None),
        url("Shouting", "HTTPS://UPPER.example/Path", (2021, 1, 8), (2026, 1, 8)),
        url("No host", "https:///nowhere", (2021, 1, 9), None),
    ]
    other = [
        folder("Recipes", [url("Sablewick roast", "https://sablewick.example/roast", (2023, 1, 2), (2024, 2, 2)),
                           url("Pinewhistle bread", "https://pinewhistle.example/bread", (2023, 1, 3), (2026, 3, 3)),
                           url("Dunmere stew", "https://dunmere.example/stew", (2023, 1, 4), None)]),
        folder("Recipes", [url("Dunmere stew", "https://dunmere.example/stew/", (2024, 6, 6), (2026, 9, 1)),
                           url("Fernhollow jam", "https://fernhollow.example/jam", (2024, 6, 7), (2026, 9, 2))]),
        folder("Empty", [folder("Also empty", [])]),
        folder("Empty too", []),
        folder(HOSTILE_TITLES[0], []),
        folder("Pile", pile),
        folder("Reading", reading),
        folder("Odd ones", odd),
        url("Local server", "http://localhost:8080/", (2025, 5, 5), (2026, 10, 1)),
        url("A file", "file:///C:/Users/someone/secret.txt", (2020, 5, 5), None),
        url("Browser settings", "chrome://settings/", (2020, 5, 6), (2026, 2, 2)),
        url("Halcyon account", "https://halcyon-vpn.example/account", (2022, 9, 9), (2026, 9, 9)),
        url("Halcyon account", "https://halcyon-vpn.example/account", (2024, 9, 9), None),
    ]
    synced = [url("Tessaract dashboard", "https://tessaract.example/dash/", (2025, 2, 2), None),
              url("Phone only", "https://thistledown.example/m", (2025, 2, 3), (2026, 10, 2))]

    edge = {"checksum": "0" * 32, "version": 1, "roots": {"bookmark_bar": root("Favorites bar", bar), "other": root("Other favorites", other),
                                                           "synced": root("Mobile favorites", synced)}}
    p = os.path.join(out, "local", "Microsoft", "Edge", "User Data", "Default")
    os.makedirs(p, exist_ok=True)
    with open(os.path.join(p, "Bookmarks"), "w", encoding="utf-8") as f:
        json.dump(edge, f, ensure_ascii=False, indent=3)
    with open(os.path.join(p, "Bookmarks.bak"), "w") as f:
        f.write("{}")
    os.makedirs(os.path.join(out, "local", "Microsoft", "Edge", "User Data", "Guest Profile"), exist_ok=True)

    chrome = {"checksum": "0" * 32, "version": 1, "roots": {
        "bookmark_bar": root("Bookmarks bar", [url("Only one", "https://only.example/", (2015, 1, 1), has_used=False)]),
        "other": root("Other bookmarks", []), "synced": root("Mobile bookmarks", [])}}
    p = os.path.join(out, "local", "Google", "Chrome", "User Data", "Profile 1")
    os.makedirs(p, exist_ok=True)
    with open(os.path.join(p, "Bookmarks"), "w", encoding="utf-8") as f:
        json.dump(chrome, f)

    # ---------------------------------------------------------------- an exported file, the way Firefox writes one
    e = html.escape
    lines = ["<!DOCTYPE NETSCAPE-Bookmark-file-1>", "<!-- This is an automatically generated file.", "     It will be read and overwritten.",
             "     DO NOT EDIT! -->", '<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">',
             '<meta http-equiv="Content-Security-Policy" content="default-src \'self\'; script-src \'none\'; img-src data: *; object-src \'none\'"></meta>',
             "<TITLE>Bookmarks</TITLE>", "<H1>Bookmarks Menu</H1>", "", "<DL><p>",
             '    <DT><A HREF="https://menu-one.example/" ADD_DATE="1600000000" LAST_MODIFIED="1600000100" ICON="data:image/png;base64,AAAA" TAGS="news,daily">Menu one</A>',
             "    <DD>A description line that is not a bookmark",
             "    <HR>",
             '    <DT><A HREF="https://menu-one.example" ADD_DATE="1650000000">Menu one again</A>',
             '    <DT><H3 ADD_DATE="1600000000" LAST_MODIFIED="1600000000" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks Toolbar</H3>',
             "    <DD>Add bookmarks to this folder to see them displayed on the Bookmarks Toolbar",
             "    <DL><p>",
             '        <DT><A HREF="https://toolbar.example/a?b=1&amp;c=2" ADD_DATE="1610000000">Toolbar &amp; friends</A>',
             '        <DT><A HREF="javascript:void(alert(%s))" ADD_DATE="1610000001">Pin it (bookmarklet)</A>' % e("'x'"),
             '        <DT><H3 ADD_DATE="1610000002">Nothing in here</H3>',
             "        <DL><p>", "        </DL><p>",
             '        <DT><H3 ADD_DATE="1610000003">Nested</H3>',
             "        <DL><p>",
             '            <DT><A HREF="https://nested.example/deep" ADD_DATE="1620000000">%s</A>' % e(HOSTILE_TITLES[0]),
             '            <DT><A HREF="https://nested.example/raw" ADD_DATE="1620000001">raw <img src=x onerror=alert(9)> tag</A>',
             '            <DT><A HREF="https://nested.example/deep/" ADD_DATE="1620000002">%s</A>' % e(HOSTILE_TITLES[2]),
             "        </DL><p>",
             "    </DL><p>",
             '    <DT><H3 ADD_DATE="1600000000" UNFILED_BOOKMARKS_FOLDER="true">Other Bookmarks</H3>',
             "    <DL><p>",
             '        <DT><A HREF="file:///home/someone/notes.txt" ADD_DATE="1630000000">Notes file</A>',
             '        <DT><A HREF="https://unfiled.example/x" ADD_DATE="1630000001">Unfiled</A>',
             "    </DL><p>",
             "</DL>"]
    with open(os.path.join(out, "exported.html"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    return os.path.join(out, "local")

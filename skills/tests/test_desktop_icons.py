"""Checks the desktop-icons skill end to end on a made-up desktop.

    python -I test_desktop_icons.py <scratch folder> [<skill folder>] [--keep] [--no-browser]

It makes a pretend profile (Desktop, Public Desktop, Documents...) in a work folder inside
<scratch folder>, with shortcut files written by hand (working, dead, cut short, hostile), web
links and loose files with awkward names. Then: survey -> page -> dry run -> apply -> revert.ps1
-> apply -> --undo, checking the tree comes back exactly. Nothing outside the work folder is
created, moved or deleted, and nothing is sent to the real Recycle Bin: the engine's Recycle Bin
call is swapped for one that moves the file to a folder inside the work folder.

--keep leaves the work folder (survey, page, plan, logs, screenshots) for a look.
Windows only. Python 3.8+, standard library only.
"""
import argparse
import contextlib
import io
import json
import os
import shutil
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
from _common import PY, X, Checks, hide, junction, make_lnk, remove_work_folder, run, set_times, tree, work_folder, write  # noqa: E402

MARK = "made-by-test_desktop_icons.txt"
SECRET = "FILE-CONTENT-MARKER-9917"
RLO = "‮"  # right-to-left override: "holiday photo" + RLO + "gpj.exe" is shown by Windows as "holiday photoexe.jpg"

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("scratch")
ap.add_argument("skill", nargs="?", default=os.path.join(HERE, "..", "desktop-icons"))
ap.add_argument("--keep", action="store_true")
ap.add_argument("--no-browser", action="store_true", help="skip the headless-browser check of the page")
opts = ap.parse_args()
if os.name != "nt":
    sys.exit("This test needs Windows.")
SKILL = os.path.abspath(opts.skill)
S = os.path.join(SKILL, "scripts")
c = Checks()
check, note = c.check, c.note
pycache_before = os.path.isdir(os.path.join(S, "__pycache__"))
T = work_folder(opts.scratch, "desktop-icons-test", MARK)
links = []

try:
    # ------------------------------------------------------------ a made-up desktop
    root = os.path.join(T, "profile")
    desk, pub = os.path.join(root, "Desktop"), os.path.join(root, "Public", "Desktop")
    for d in ("Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music", os.path.join("Public", "Desktop")):
        os.makedirs(os.path.join(root, d))
    targets = os.path.join(T, "targets")
    editor = write(os.path.join(targets, "Lanternfall", "editor.exe"), b"MZ")
    write(os.path.join(targets, "Lanternfall", "readme.txt"), "r")
    os.makedirs(os.path.join(targets, "Reports"))
    pinned_exe = write(os.path.join(targets, "Pinned", "app.exe"), b"MZ")
    start_exe = write(os.path.join(targets, "Start", "start-app.exe"), b"MZ")
    shared_exe = write(os.path.join(targets, "Shared", "shared.exe"), b"MZ")
    one, two = write(os.path.join(targets, "One", "samename.exe"), b"MZ"), write(os.path.join(targets, "Two", "samename.exe"), b"MZ")
    cmd = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "cmd.exe")
    gone = os.path.join(targets, "Copperline", "notes.exe")
    url = "[InternetShortcut]\r\nURL=%s\r\n"
    shortcuts = {
        "Lanternfall Editor.lnk": make_lnk(editor),
        "Lanternfall Editor - Copy.lnk": make_lnk(editor),
        "Lanternfall Editor (2).lnk": make_lnk(editor, args="--safe-mode"),
        "Copperline Notes.lnk": make_lnk(gone, wide=True),
        "Old \u2019Game\u2019 & $(x).lnk": make_lnk(os.path.join(targets, "Old Game", "game.exe")),
        "Reports.lnk": make_lnk(os.path.join(targets, "Reports")),
        "Relative.lnk": make_lnk(relative="..\\..\\targets\\Lanternfall\\readme.txt"),
        "Env.lnk": make_lnk(env="%SystemRoot%\\explorer.exe"),
        "Env gone.lnk": make_lnk(env="%ProgramData%\\mom-desktop-test-not-a-real-folder\\gone.exe"),
        "Env secret.lnk": make_lnk(env="%MOM_DESKTOP_TEST_KEY%"),
        "Env secret in a path.lnk": make_lnk(env="%SystemRoot%\\%MOM_DESKTOP_TEST_KEY%\\tool.exe"),
        "Env secret that is a path.lnk": make_lnk(env="%MOM_DESKTOP_TEST_PLACE%\\tool.exe"),
        "Stale relative.lnk": make_lnk(relative="..\\Nowhere\\gone.exe"),
        "Relative with folder.lnk": make_lnk(relative="..\\Nowhere\\editor.exe", workdir=os.path.join(targets, "Lanternfall")),
        "Truncated.lnk": make_lnk(editor)[:90],
        "Not a link.lnk": b"this is not a shortcut " * 10,
        "Empty.lnk": b"",
        "Hostile target.lnk": make_lnk('C:\\x\\"; calc & $(y) `z`\\evil.exe', wide=True),
        "Share.lnk": make_lnk("\\\\evil.example\\share\\x.exe"),
        "Net.lnk": make_lnk(network=True),
        "Invoice.pdf.lnk": make_lnk(cmd, args="/c echo pwned > \"%TEMP%\\mom-desktop-test-should-never-exist.txt\""),
        "Photo.jpg.lnk": make_lnk(editor),
        "Pinned App.lnk": make_lnk(pinned_exe),
        "Start App.lnk": make_lnk(start_exe),
        "Twice A.lnk": make_lnk(one),
        "Twice B.lnk": make_lnk(two),
        "Report" + RLO + "fdp.lnk": make_lnk(shared_exe),
        "Lanternfall Site.url": (url % "https://lanternfall.example/a?token=SECRET-TOKEN-1").encode(),
        "Lanternfall Site (2).url": (url % "https://lanternfall.example/a?token=SECRET-TOKEN-1").encode(),
        "Steam Game.url": (url % "steam://rungameid/12345").encode(),
        "Dead file.url": (url % ("file:///" + os.path.join(targets, "nope.pdf").replace("\\", "/"))).encode(),
        "Share.url": (url % "file://evil.example/share/x.pdf").encode(),
        "Broken.url": b"\x00\x01\x02 garbage",
    }
    for name, data in shortcuts.items():
        write(os.path.join(desk, name), data)
    old = time.time() - 400 * 86400
    stale = ["contract \u2019 signed.pdf", "a; echo pwn & b.txt", "tax $(calc) `x`.docx", "it's 'quoted'.png", "Screenshot 2024-03-14.png",
             "ends in a dot.", "setup-lanternfall-1.2.exe", "archive.zip", "clip.mp4", "song.mp3"]
    fresh = ["notes.txt", "tool.exe"]
    backwards = "holiday photo" + RLO + "gpj.exe"
    for name in stale + fresh + [backwards]:
        write(os.path.join(desk, name), SECRET + " " + name)
    for name in stale:
        set_times(os.path.join(desk, name), old)
    write(os.path.join(desk, "Project X", ".git", "config"), "[core]")
    os.makedirs(os.path.join(desk, "New folder"))
    for n in ("a.jpg", "b.jpg"):
        write(os.path.join(desk, "Holiday photos", n), SECRET)
    write(os.path.join(desk, "Some Program", "unins000.exe"), b"MZ")
    write(os.path.join(desk, "Some Program", "app.dll"), b"MZ")
    hide(write(os.path.join(desk, "desktop.ini"), "[.ShellClassInfo]"))
    if junction(os.path.join(desk, "Link to videos"), os.path.join(root, "Videos")):
        links.append(os.path.join(desk, "Link to videos"))
    write(os.path.join(root, "Videos", "clip-in-videos.mp4"), "v")
    write(os.path.join(root, "Documents", "doc.txt"), "d")
    write(os.path.join(pub, "Shared App.lnk"), make_lnk(shared_exe))
    write(os.path.join(pub, "Dead Public.lnk"), make_lnk(os.path.join(targets, "nowhere.exe")))
    write(os.path.join(pub, "Lanternfall Editor.lnk"), make_lnk(editor))
    roaming = os.path.join(root, "AppData", "Roaming", "Microsoft")
    write(os.path.join(roaming, "Internet Explorer", "Quick Launch", "User Pinned", "TaskBar", "Pinned App.lnk"), make_lnk(pinned_exe))
    write(os.path.join(roaming, "Windows", "Start Menu", "Programs", "Lanternfall", "Start App.lnk"), make_lnk(start_exe))
    outside = os.path.join(T, "outside")
    os.makedirs(outside)
    out = os.path.join(T, "out")
    os.makedirs(out)
    before = tree(root) + tree(targets)
    targets_before, pub_before = tree(targets), tree(pub)

    # ------------------------------------------------------------ 1. the survey
    print("\n-- 1. the survey looks and changes nothing")
    survey_json = os.path.join(out, "survey.json")
    # Two things in the environment that are nobody's business: a key, and a setting that happens to look like a path
    private = dict(os.environ, MOM_DESKTOP_TEST_KEY="sk-test-NEVER-IN-A-SURVEY-7731", MOM_DESKTOP_TEST_PLACE="C:\\NEVER-IN-A-SURVEY-PLACE")
    r = run([PY, "-I", os.path.join(S, "survey.py"), "--out", survey_json, "--root", root, "--screen", "1280x720", "--cell", "80x100"], env=private)
    check("the survey runs", r.returncode == 0, r.stderr or r.stdout)
    check("the survey changed nothing and wrote one file", tree(root) + tree(targets) == before and os.listdir(out) == ["survey.json"], os.listdir(out))
    sv = json.load(open(survey_json, encoding="utf-8"))
    raw = open(survey_json, encoding="utf-8").read()
    by_name = {(it["where"], it["name"]): it for it in sv["items"]}

    def status(name, where="desktop"):
        return by_name[(where, name)]["shortcut"]["status"]

    check("every shortcut and loose thing on both desktops is listed; hidden files and links are not",
          len([i for i in sv["items"] if i["where"] == "desktop"]) == len(shortcuts) + len(stale) + len(fresh) + 1 + 4 and sv["counts"]["public"] == 3
          and ("desktop", "desktop.ini") not in by_name and ("desktop", "Link to videos") not in by_name, sv["counts"])
    check("working shortcuts are 'ok' (a file, a folder, a relative path, a path with %VARIABLES%)",
          all(status(n) == "ok" for n in ("Lanternfall Editor.lnk", "Reports.lnk", "Relative.lnk", "Env.lnk", "Lanternfall Editor (2).lnk",
                                           "Relative with folder.lnk")),
          [(n, by_name[("desktop", n)]["shortcut"]) for n in ("Relative.lnk", "Env.lnk")])
    check("shortcuts whose target is gone are 'dead' (also with an awkward name, and a .url to a file)",
          all(status(n) == "dead" for n in ("Copperline Notes.lnk", "Old \u2019Game\u2019 & $(x).lnk", "Dead file.url", "Env gone.lnk"))
          and status("Dead Public.lnk", "public") == "dead")
    unknown = ("Truncated.lnk", "Not a link.lnk", "Empty.lnk", "Hostile target.lnk", "Share.lnk", "Net.lnk", "Broken.url", "Share.url",
               "Stale relative.lnk", "Env secret.lnk", "Env secret in a path.lnk", "Env secret that is a path.lnk")
    check("anything that can't be read or checked is 'unknown', never 'dead' (cut short, not a link, empty, hostile target, network, "
          "only a relative path)",
          all(status(n) == "unknown" for n in unknown), [(n, status(n)) for n in unknown])
    check("a web link and a game link are kept as their kind and host only",
          by_name[("desktop", "Lanternfall Site.url")]["shortcut"]["to"] == "lanternfall.example"
          and by_name[("desktop", "Steam Game.url")]["shortcut"]["leads_to"] == "game" and "SECRET-TOKEN" not in raw and "rungameid" not in raw)
    check("no file was read except shortcuts (nothing from inside a loose file is in the survey)", SECRET not in raw)
    check("a shortcut can't make the survey copy a variable out of the environment: only the few that name a place are filled in",
          "NEVER-IN-A-SURVEY" not in raw and "NEVER-IN-A-SURVEY" not in r.stdout + r.stderr
          and "doesn't fill in" in by_name[("desktop", "Env secret.lnk")]["shortcut"]["reason"], by_name[("desktop", "Env secret.lnk")]["shortcut"])
    flagged = {it["name"]: it["shortcut"].get("flag", "") for it in sv["items"] if it.get("shortcut", {}).get("flag")}
    check("a shortcut that runs a command, one named like a document, and a link to a share are flagged",
          "runs a command" in flagged.get("Invoice.pdf.lnk", "") and "named like a document" in flagged.get("Photo.jpg.lnk", "")
          and "network share" in flagged.get("Share.url", "") and len(flagged) == 3, flagged)
    check("the same program in two places is reported as perhaps installed twice",
          len(sv["twice"]) == 1 and sv["twice"][0]["program"] == "samename.exe" and len(sv["twice"][0]["places"]) == 2, sv["twice"])
    check("a dead shortcut on the Public Desktop is 'for you to do'", len(sv["for_you"]) == 1 and sv["for_you"][0]["name"] == "Dead Public.lnk", sv["for_you"])
    la = sv["left_alone"]
    check("left alone: the hidden file, the link, the code project and the program folder",
          la["mine"]["hidden"] == 1 and la["mine"]["links"] == len(links) and sorted(os.path.basename(f["path"]) for f in la["folders"]) == ["Project X", "Some Program"], la)
    check("icons against room on one screen", sv["screen"]["fits"] == 16 * 7 and sv["counts"]["icons"] == len(sv["items"]), sv["screen"])
    sug = {os.path.basename(s["from"]): s for s in sv["suggestions"]}
    want = {"Copperline Notes.lnk": ("dead", True), "Old \u2019Game\u2019 & $(x).lnk": ("dead", True), "Dead file.url": ("dead", True),
            "Lanternfall Editor.lnk": ("duplicates", True), "Lanternfall Editor - Copy.lnk": ("duplicates", True),
            "Lanternfall Site (2).url": ("duplicates", True), "Lanternfall Site.url": ("shortcuts", False),
            "Lanternfall Editor (2).lnk": ("shortcuts", False), "Invoice.pdf.lnk": ("check", False), "Photo.jpg.lnk": ("check", False),
            "Share.url": ("check", False), "Pinned App.lnk": ("elsewhere", False), "Start App.lnk": ("elsewhere", False),
            "Twice A.lnk": ("shortcuts", False), "Steam Game.url": ("shortcuts", False), "New folder": ("empty", True),
            "Holiday photos": ("folders", False), "notes.txt": ("files", False), "tool.exe": ("files", False)}
    want.update({n: ("shortcuts", False) for n in unknown if n != "Share.url"})
    want.update({n: ("files", True) for n in stale})
    want.update({backwards: ("check", False), "Report" + RLO + "fdp.lnk": ("check", False)})
    got = {n: (sug[n]["group"], sug[n]["default"]) for n in want if n in sug}
    check("each thing lands in the right list, ticked or not (%d checked)" % len(want), got == want, {n: (got.get(n), want[n]) for n in want if got.get(n) != want[n]})
    check("nothing on the Public Desktop, hidden, linked, or in a project or program folder is in the plan",
          all(os.path.dirname(s["from"]).lower() == desk.lower() for s in sv["suggestions"])
          and not any(n in sug for n in ("Project X", "Some Program", "desktop.ini", "Link to videos", "Dead Public.lnk")), list(sug))
    check("dead and duplicate shortcuts go to the Recycle Bin; loose files move to the matching library folder",
          all(sug[n]["op"] == "recycle" for n in ("Copperline Notes.lnk", "Lanternfall Editor - Copy.lnk"))
          and sug["contract \u2019 signed.pdf"]["to"] == os.path.join(root, "Documents", "From the Desktop", "contract \u2019 signed.pdf")
          and sug["Screenshot 2024-03-14.png"]["to"] == os.path.join(root, "Pictures", "Screenshots", "Screenshot 2024-03-14.png")
          and sug["it's 'quoted'.png"]["to"].startswith(os.path.join(root, "Pictures", "From the Desktop"))
          and sug["clip.mp4"]["to"].startswith(os.path.join(root, "Videos")) and sug["song.mp3"]["to"].startswith(os.path.join(root, "Music"))
          and sug["archive.zip"]["to"].startswith(os.path.join(root, "Downloads")) and sug["New folder"]["op"] == "remove-empty-dir")

    # ------------------------------------------------------------ 2. the page and the plan
    print("\n-- 2. the page and the plan")
    evil = "</script><img src=x onerror=\"window.__pwned=1\"> ' \u2019 $(x) `y` ; & \u2028"
    notes = {"headline": "Headline " + evil, "highlights": ["One " + evil, "Two"], "drop": [sug["Twice A.lnk"]["id"]],
             "retarget": {str(sug["notes.txt"]["id"]): os.path.join(root, "Documents", "Notes", "notes.txt")},
             "add": [{"op": "recycle", "from": os.path.join(desk, "Twice B.lnk"), "why": "Why " + evil}]}
    notes_json = os.path.join(out, "notes.json")
    json.dump(notes, open(notes_json, "w", encoding="utf-8"))
    page, plan_json = os.path.join(out, "desktop-plan.html"), os.path.join(out, "desktop-plan.json")
    build = [PY, "-I", os.path.join(S, "build_page.py"), survey_json]
    r = run(build + ["--notes", notes_json, "--out", page])
    check("the page builds", r.returncode == 0 and os.path.isfile(page) and os.path.isfile(plan_json), r.stderr or r.stdout)
    plan = json.load(open(plan_json, encoding="utf-8"))
    sys.path.insert(0, S)
    import safe_apply as sa  # noqa: E402  (the skill's own copy)
    check("the plan is desktop-icons/plan@1 and its id is the fingerprint of its actions",
          plan["schema"] == "desktop-icons/plan@1" and plan["plan_id"] == sa.plan_fingerprint(plan["actions"]))
    acts = {os.path.basename(a["from"]): a for a in plan["actions"]}
    check("notes: drop, retarget and add are applied", "Twice A.lnk" not in acts and acts["notes.txt"]["to"].endswith("\\Documents\\Notes\\notes.txt")
          and acts["Twice B.lnk"]["group"] == "custom" and acts["Twice B.lnk"]["op"] == "recycle" and len(plan["actions"]) == len(sv["suggestions"]) - 1
          and len({a["from"].lower() for a in plan["actions"]}) == len(plan["actions"]))
    html = open(page, encoding="utf-8").read()
    check("hostile text is escaped in the page: nothing can close the data block or start a tag",
          html.count("</script>") == 2 and "<img" not in html and "onerror=\\\"" in html and "\u2028" not in html and "__DESKTOP_DATA__" not in html)
    check("a name with a right-to-left override in it is printed spelled out, never as it would look",
          by_name[("desktop", backwards)].get("shown") == "holiday photo\\u202egpj.exe" and acts[backwards]["name"] == "holiday photo\\u202egpj.exe"
          and RLO in acts[backwards]["from"] and RLO not in html and "direction-changing" in acts[backwards]["why"], acts[backwards])
    check("the page shows names and targets, never file contents, link tokens or full web addresses",
          SECRET not in html and "SECRET-TOKEN" not in html and "rungameid" not in html)
    check("the page is self-contained: no outside address in it but the SVG namespace",
          html.count("http://www.w3.org/2000/svg") == html.count("http://") and "https://" not in html and "//cdn" not in html)
    check("what a flagged shortcut would run is shown as plain text next to it", "echo pwned" in acts["Invoice.pdf.lnk"].get("detail", ""))
    r = run(build + ["--notes", notes_json, "--out", os.path.join(out, "again.html"), "--plan", os.path.join(out, "again.json")])
    check("building twice from the same survey and notes gives the same plan id",
          json.load(open(os.path.join(out, "again.json"), encoding="utf-8"))["plan_id"] == plan["plan_id"])
    os.remove(os.path.join(out, "again.html"))
    os.remove(os.path.join(out, "again.json"))
    lay = os.path.join(out, "layout-desktop.json")
    json.dump({"layout": "desktop"}, open(lay, "w"))
    r = run(build + ["--notes", lay, "--out", os.path.join(out, "desktop-layout.html"), "--plan", os.path.join(out, "desktop-layout-plan.json")])
    lp = json.load(open(os.path.join(out, "desktop-layout-plan.json"), encoding="utf-8"))
    moves = [a for a in lp["actions"] if a["op"] == "move"]
    check("layout 'desktop': loose files go into a few folders on the Desktop, and folders stay", r.returncode == 0 and moves
          and all(os.path.dirname(os.path.dirname(a["to"])).lower() == desk.lower() for a in moves) and not any(a.get("is_dir") for a in moves)
          and lp["plan_id"] != plan["plan_id"], r.stderr)
    bad_notes = [
        ("a source that isn't on the Desktop", {"add": [{"op": "recycle", "from": os.path.join(root, "Documents", "doc.txt")}]}),
        ("a source on the Public Desktop", {"add": [{"op": "recycle", "from": os.path.join(pub, "Dead Public.lnk")}]}),
        ("a source in Windows", {"add": [{"op": "move", "from": "C:\\Windows\\notepad.exe", "to": os.path.join(root, "Documents", "n.exe")}]}),
        ("a source that isn't a path", {"add": [{"op": "recycle", "from": ["x"]}]}),
        ("a destination in Windows", {"add": [{"op": "move", "from": os.path.join(desk, "notes.txt"), "to": "C:\\Windows\\notes.txt"}]}),
        ("a destination outside the person's folders", {"add": [{"op": "move", "from": os.path.join(desk, "notes.txt"), "to": os.path.join(outside, "n.txt")}]}),
        ("a destination with '..' in it", {"add": [{"op": "move", "from": os.path.join(desk, "notes.txt"), "to": os.path.join(root, "Documents", "..", "..", "outside", "n.txt")}]}),
        ("a destination on a share", {"retarget": {str(sug["notes.txt"]["id"]): "\\\\evil.example\\share\\n.txt"}}),
        ("a destination on the Public Desktop", {"retarget": {str(sug["notes.txt"]["id"]): os.path.join(pub, "n.txt")}}),
        ("a retarget of something that isn't a move", {"retarget": {str(sug["Copperline Notes.lnk"]["id"]): os.path.join(root, "Documents", "x.lnk")}}),
        ("an operation that doesn't exist", {"add": [{"op": "delete", "from": os.path.join(desk, "notes.txt")}]}),
        ("a layout that doesn't exist", {"layout": "everywhere"}),
    ]
    for why, n in bad_notes:
        nb, pb = os.path.join(out, "bad-notes.json"), os.path.join(out, "bad-plan.json")
        json.dump(n, open(nb, "w", encoding="utf-8"))
        r = run(build + ["--notes", nb, "--out", os.path.join(out, "bad.html"), "--plan", pb])
        check("notes with %s stop the build before any plan exists" % why, r.returncode != 0 and not os.path.exists(pb) and not os.path.exists(os.path.join(out, "bad.html")), r.stderr or r.stdout)
    os.remove(os.path.join(out, "bad-notes.json"))

    # ------------------------------------------------------------ 3. the apply command's own limits
    print("\n-- 3. apply_plan.py can't be talked into touching anything else")
    apply_py = [PY, "-I", os.path.join(S, "apply_plan.py")]
    startup = os.path.join(root, "AppData", "Roaming", "Microsoft", "Windows", "Start Menu", "Programs", "Startup", "x.lnk")
    hostile = [
        ("a source in Documents", {"op": "move", "from": os.path.join(root, "Documents", "doc.txt"), "to": os.path.join(desk, "doc.txt")}, "not on your Desktop"),
        ("a source on the Public Desktop", {"op": "recycle", "from": os.path.join(pub, "Dead Public.lnk")}, "outside the folders"),
        ("a destination on the Public Desktop", {"op": "move", "from": os.path.join(desk, "notes.txt"), "to": os.path.join(pub, "notes.txt")}, "outside the folders"),
        ("a destination outside the person's folders", {"op": "move", "from": os.path.join(desk, "notes.txt"), "to": os.path.join(outside, "n.txt")}, ""),
        ("a source in Windows", {"op": "move", "from": "C:\\Windows\\notepad.exe", "to": os.path.join(desk, "n.exe")}, "Windows"),
        ("a destination in the profile's AppData (Startup)", {"op": "move", "from": os.path.join(desk, "notes.txt"), "to": startup}, "AppData"),
        ("the Desktop folder itself", {"op": "recycle", "from": desk}, "main folders"),
        ("a file reached through a link on the Desktop", {"op": "move", "from": os.path.join(desk, "Link to videos", "clip-in-videos.mp4"), "to": os.path.join(desk, "c.mp4")}, "outside the folders"),
        ("the link itself", {"op": "recycle", "from": os.path.join(desk, "Link to videos")}, "link"),
        ("a program's own folder", {"op": "move", "from": os.path.join(desk, "Some Program"), "to": os.path.join(root, "Documents", "Some Program")}, "installed program"),
        ("a source written with forward slashes", {"op": "recycle", "from": os.path.join(desk, "notes.txt").replace("\\", "/")}, ""),
    ]
    h_actions = [dict(a, id=i + 1) for i, (_, a, _) in enumerate(hostile)]
    h_actions.append({"id": len(h_actions) + 1, "op": "move", "from": os.path.join(desk, "notes.txt"), "to": os.path.join(root, "Documents", "ok", "notes.txt")})
    hp = os.path.join(out, "hostile-plan.json")
    json.dump({"schema": "desktop-icons/plan@1", "plan_id": sa.plan_fingerprint(h_actions), "actions": h_actions}, open(hp, "w", encoding="utf-8"), ensure_ascii=False)
    happrove = "DESKTOP-APPROVE plan=%s ids=1-%d" % (sa.plan_fingerprint(h_actions), len(h_actions))
    r = run(apply_py + [hp, "--approve", happrove, "--root", root, "--log-dir", os.path.join(out, "log-hostile")])
    lines = {int(l.split("#")[1].split()[0]): l for l in r.stdout.splitlines() if l[:8].strip() in ("WOULD", "REFUSED", "SKIP") and "#" in l}
    for i, (why, _, word) in enumerate(hostile):
        check("refused: " + why, lines.get(i + 1, "").startswith("REFUSED") and word in lines.get(i + 1, ""), lines.get(i + 1) or r.stdout[-300:] + r.stderr)
    check("...and a plain move off the Desktop in the same plan would happen", lines.get(len(h_actions), "").startswith("WOULD"), lines.get(len(h_actions)))
    r = run(apply_py + [hp, "--approve", happrove, "--root", root, "--within", outside, "--log-dir", os.path.join(out, "log-hostile")])
    check("apply_plan.py sets its own limits: a --within from outside is refused", r.returncode != 0 and tree(root) + tree(targets) == before, r.stderr or r.stdout)
    r = run(apply_py + [plan_json, "--approve", "DESKTOP-APPROVE plan=0000000000 ids=1", "--root", root])
    check("an approval for another plan id is refused", r.returncode != 0, r.stderr or r.stdout)
    os.remove(hp)

    # ------------------------------------------------------------ 4. dry run, apply, both ways back
    print("\n-- 4. survey -> page -> dry run -> apply -> revert.ps1 -> apply -> --undo")
    ticked = sorted([a["id"] for a in plan["actions"] if a["default"]] + [acts["Holiday photos"]["id"], acts["notes.txt"]["id"], acts[backwards]["id"]])
    approve = "DESKTOP-APPROVE plan=%s ids=%s\nto Recycle Bin: 0 \u00b7 moved off the desktop: 0\nnote: thanks" % (plan["plan_id"], ",".join(map(str, ticked)))
    r = run(apply_py + [plan_json, "--approve", approve, "--root", root, "--log-dir", os.path.join(out, "log-1")])
    would = sum(1 for l in r.stdout.splitlines() if l.startswith("WOULD"))
    check("dry run: every ticked action would happen, nothing is refused, nothing changes",
          r.returncode == 0 and would == len(ticked) and "REFUSED" not in r.stdout and tree(root) + tree(targets) == before and not os.path.exists(os.path.join(out, "log-1")),
          r.stdout[-1500:] + r.stderr)
    import apply_plan  # noqa: E402  (the skill's command, run in this process so the Recycle Bin call can be swapped)
    fake_bin = os.path.join(T, "pretend-recycle-bin")
    binned = []

    def pretend_recycle(path):
        os.makedirs(fake_bin, exist_ok=True)
        dest = os.path.join(fake_bin, "%03d %s" % (len(binned), os.path.basename(path)))
        shutil.move(X + path, X + dest)
        binned.append((dest, path))
        return None

    def restore_bin():
        for dest, path in reversed(binned):
            shutil.move(X + dest, X + path)
        del binned[:]
        if os.path.isdir(fake_bin):
            os.rmdir(fake_bin)

    def apply_here(argv):
        buf, old_recycle = io.StringIO(), sa.recycle
        sa.recycle = pretend_recycle
        try:
            with contextlib.redirect_stdout(buf):
                apply_plan.main(argv)
        except SystemExit as ex:
            buf.write("\nEXIT %r" % (ex.code,))
        finally:
            sa.recycle = old_recycle
        return buf.getvalue()

    n_recycle = sum(1 for a in plan["actions"] if a["id"] in ticked and a["op"] == "recycle")
    for round_no, way_back in ((1, "revert.ps1"), (2, "--undo")):
        log = os.path.join(out, "log-%d" % round_no)
        text = apply_here([plan_json, "--approve", approve, "--root", root, "--log-dir", log, "--apply"])
        now = tree(root)
        left = [e for e in os.listdir(desk)]
        check("apply (%d): done with nothing failed or refused; %d went to the (pretend) Recycle Bin" % (round_no, n_recycle),
              "EXIT" not in text and "FAILED" not in text and "REFUSED" not in text and len(binned) == n_recycle
              and sum(1 for l in text.splitlines() if l.startswith("DONE") and " mkdir " not in l) == len(ticked), text[-1500:])
        check("apply (%d): the ticked things are off the Desktop, the rest is still there" % round_no,
              not any(n in left for n in stale + ["Copperline Notes.lnk", "Lanternfall Editor - Copy.lnk", "New folder", "Holiday photos", "notes.txt"])
              and all(n in left for n in ("Lanternfall Editor (2).lnk", "Invoice.pdf.lnk", "Project X", "Some Program", "tool.exe", "desktop.ini", "Truncated.lnk"))
              and os.path.isfile(X + os.path.join(root, "Downloads", "From the Desktop", "ends in a dot."))
              and os.path.isfile(os.path.join(root, "Documents", "From the Desktop", "Holiday photos", "a.jpg"))
              and os.path.isfile(os.path.join(root, "Documents", "Notes", "notes.txt")), left)
        check("apply (%d): the Public Desktop, the targets and everything else are untouched" % round_no,
              tree(pub) == pub_before and tree(targets) == targets_before and os.listdir(outside) == [])
        if way_back == "revert.ps1":
            r = run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", os.path.join(log, "revert.ps1"), "-Root", root, "-Preview"])
            check("revert.ps1 -Preview changes nothing", r.returncode == 0 and tree(root) == now, r.stdout[-600:] + r.stderr)
            r = run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", os.path.join(log, "revert.ps1"), "-Root", root])
        else:
            r = run(apply_py + ["--undo", log, "--root", root])
            check("--undo's dry run changes nothing", r.returncode == 0 and tree(root) == now, r.stdout[-600:] + r.stderr)
            r = run(apply_py + ["--undo", log, "--root", root, "--apply"])
        check("%s: runs clean and lists the %d Recycle Bin items as MANUAL" % (way_back, n_recycle), r.returncode == 0 and r.stdout.count("MANUAL") == n_recycle,
              r.stdout[-800:] + r.stderr)
        restore_bin()
        check("%s (and the pretend Recycle Bin emptied back): the tree is exactly as it was" % way_back, tree(root) + tree(targets) == before,
              [e for e in tree(root) + tree(targets) if e not in before][:6] + [e for e in before if e not in tree(root) + tree(targets)][:6])
    check("nothing in a name or a shortcut ran", not os.path.exists(os.path.join(os.environ.get("TEMP", T), "mom-desktop-test-should-never-exist.txt"))
          and sorted(os.listdir(T)) == sorted([MARK, "out", "outside", "profile", "targets"]), os.listdir(T))

    # ------------------------------------------------------------ 5. shortcuts Windows itself made
    print("\n-- 5. shortcut files made by Windows")
    import survey as dsv  # noqa: E402  (the skill's survey: read-only helpers)
    real = os.path.join(T, "made-by-windows")
    os.makedirs(real)
    ps1 = os.path.join(real, "make.ps1")
    open(ps1, "w").write("param($link, $target, $arguments)\n$s = (New-Object -ComObject WScript.Shell).CreateShortcut($link)\n"
                         "$s.TargetPath = $target\nif ($arguments) { $s.Arguments = $arguments }\n$s.Save()\n")
    made = []
    soon_gone = write(os.path.join(real, "soon-gone.exe"), b"MZ")
    for name, target, arguments in (("alive.lnk", editor, "/from-windows now"), ("gone.lnk", soon_gone, ""),
                                    ("folder.lnk", os.path.join(targets, "Reports"), ""), ("never.lnk", os.path.join(real, "never-was.exe"), "")):
        r = run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1, os.path.join(real, name), target, arguments])
        if r.returncode == 0 and os.path.isfile(os.path.join(real, name)):
            made.append(name)
    os.remove(soon_gone)  # the target goes away after the shortcut was made, as it does in real life
    if len(made) < 4:
        note("Windows wouldn't write shortcut files here (WScript.Shell): shortcuts made by Windows were not checked")
    else:
        pr = dsv.Prober()
        res = {n: dsv.read_shortcut(os.path.join(real, n), n, os.path.getsize(os.path.join(real, n)), False, pr) for n in made}
        check("a shortcut made by Windows to an existing file is read as 'ok', with its target and that it has arguments",
              res["alive.lnk"]["status"] == "ok" and res["alive.lnk"]["target"].lower() == editor.lower() and res["alive.lnk"]["has_args"], res["alive.lnk"])
        check("...one whose target was deleted afterwards as 'dead', one to a folder as a folder",
              res["gone.lnk"]["status"] == "dead" and res["folder.lnk"]["leads_to"] == "folder", res)
        check("...and one to a file that never existed as 'dead' or 'unknown' (it says: %s)" % res["never.lnk"]["status"],
              res["never.lnk"]["status"] in ("dead", "unknown"), res["never.lnk"])
    shutil.rmtree(real)
    menu = os.path.join(os.environ.get("ProgramData", r"C:\ProgramData"), "Microsoft", "Windows", "Start Menu", "Programs")
    seen = failed = 0
    kinds = {"ok": 0, "dead": 0, "unknown": 0}
    pr = dsv.Prober()
    for d, _dirs, files in os.walk(menu):
        for f in files:
            if f.lower().endswith(".lnk") and seen < 400:
                seen += 1
                try:
                    data = open(os.path.join(d, f), "rb").read(dsv.MAX_SHORTCUT_BYTES)
                    dsv.parse_lnk(data)
                    kinds[dsv.read_shortcut(os.path.join(d, f), f, len(data), False, pr)["status"]] += 1
                except (ValueError, OSError):
                    failed += 1
    if not seen:
        note("no Start menu shortcuts to read on this PC: real shortcut files were not checked")
    else:
        check("every one of this PC's %d all-users Start menu shortcuts can be read (read only; %d ok, %d dead, %d unknown; no names kept)" % (
            seen, kinds["ok"], kinds["dead"], kinds["unknown"]), failed == 0 and kinds["ok"] > 0, failed)

    m = dsv.screen_fit(dsv.measure_desktop())
    if not m:
        note("the desktop window couldn't be asked how many icons it shows (no desktop in this session?)")
    else:
        check("the real desktop window answers two read-only questions (numbers only: %d icons, room for about %d)" % (m["icons_shown"], m["fits"]),
              m["measured"] and m["cell_w"] >= 16 and m["cell_h"] >= 16 and m["fits"] >= 1 and m["icons_shown"] >= 0, m)

    # ------------------------------------------------------------ 6. the page in a browser
    print("\n-- 6. the page in a headless browser")
    if opts.no_browser:
        note("the page was not opened in a browser (--no-browser)")
    else:
        shots = os.path.join(out, "screenshots")
        os.makedirs(shots)
        r = run([PY, "-I", os.path.join(HERE, "check_page.py"), page, T, "--prefix", "DESKTOP-APPROVE", "--plan", plan_json, "--shots", shots])
        print("\n".join("    " + l for l in r.stdout.splitlines() if l.strip()))
        if r.returncode == 2:
            note("no browser to open the page in: " + r.stdout.strip()[-120:])
        else:
            check("the page passes every browser check", r.returncode == 0, r.stdout[-600:] + r.stderr[-300:])
    check("no __pycache__ left in the skill folder", pycache_before or not os.path.isdir(os.path.join(S, "__pycache__")))
finally:
    for link in links:
        try:
            os.rmdir(link)  # the link itself, never what it points at
        except OSError:
            pass
    if not opts.keep:
        remove_work_folder(T, MARK)
    else:
        print("\nkept for a look: %s" % T)
c.finish("desktop-icons")

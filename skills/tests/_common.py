"""Shared by the skill tests: a work folder with a marker in it, checks that count, fixture helpers.

Nothing here touches anything outside the work folder a test makes for itself.
"""
import ctypes
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import time

X = "\\\\?\\"
PY = sys.executable
LNK_CLSID = bytes.fromhex("0114020000000000c000000000000046")


class Checks:
    def __init__(self):
        self.fails, self.notes, self.count = [], [], 0

    def check(self, name, ok, detail=""):
        self.count += 1
        print(("ok:   " if ok else "FAIL: ") + name + ((" (" + str(detail)[:600] + ")") if detail and not ok else ""))
        if not ok:
            self.fails.append(name)
        return bool(ok)

    def note(self, text):
        """Something that could not be checked here. Repeated at the end as "not run"."""
        self.notes.append(text)
        print("note: " + text)

    def info(self, text):
        print("note: " + text)

    def finish(self, what):
        print("\n%s: %d checks on %s%s" % ("ALL PASSED" if not self.fails else "%d FAILED" % len(self.fails), self.count, what,
                                           "" if not self.fails else ": %s" % self.fails))
        for n in self.notes:
            print("not run: " + n)
        sys.exit(1 if self.fails else 0)


def run(args, **kw):
    return subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace", **kw)


def work_folder(scratch, name, mark):
    """Make (or remake) this test's own folder inside the scratch folder. Refuses a folder it didn't make."""
    scratch = os.path.abspath(scratch)
    if not os.path.isdir(scratch):
        sys.exit("The scratch folder must already exist: %s" % scratch)
    t = os.path.join(scratch, name)
    if os.path.lexists(t):
        if not os.path.isfile(os.path.join(t, mark)):
            sys.exit("%s exists and this test didn't make it. Remove it yourself or give another scratch folder." % t)
        remove_work_folder(t, mark)
    os.makedirs(t)
    with open(os.path.join(t, mark), "w") as f:
        f.write("Made by a test in skills/tests. Safe to delete.\n")
    return t


def remove_work_folder(t, mark):
    """Delete a work folder this test made: links first (the link itself, never its target), then the rest.
    Says so if something in it is still held open and it can't all be removed."""
    if not os.path.isfile(os.path.join(t, mark)):
        return
    for d, dirs, _files in os.walk(X + t):
        for name in list(dirs):
            p = os.path.join(d, name)
            st = os.lstat(p)
            if getattr(st, "st_file_attributes", 0) & 0x400:
                os.rmdir(p)
                dirs.remove(name)
    for _ in range(40):
        shutil.rmtree(X + t, ignore_errors=True)
        if not os.path.lexists(t):
            return
        time.sleep(0.5)
    print("note: could not remove the work folder (something in it is still open): %s" % t)


def tree(root):
    """Every file (with its size) and folder under root, never walking through a link."""
    out = []
    stack = [X + root]
    while stack:
        d = stack.pop()
        with os.scandir(d) as it:
            for e in it:
                rel = e.path[len(X + root):]
                st = e.stat(follow_symlinks=False)
                if getattr(st, "st_file_attributes", 0) & 0x400:
                    out.append((rel, "link"))
                elif e.is_dir(follow_symlinks=False):
                    out.append((rel, "dir"))
                    stack.append(e.path)
                else:
                    out.append((rel, st.st_size))
    return sorted(out)


def hashes(root):
    """{relative path: SHA-256} of every file under root, to show that nothing in a folder was changed."""
    out = {}
    for d, _dirs, files in os.walk(root):
        for name in files:
            p = os.path.join(d, name)
            with open(p, "rb") as f:
                out[os.path.relpath(p, root)] = hashlib.sha256(f.read()).hexdigest()
    return out


def ranges(ids):
    """[1, 2, 3, 7] as "1-3,7": how a page writes the ticked numbers in its approval line. Nothing ticked is "none"."""
    ids, out, i = sorted(ids), [], 0
    while i < len(ids):
        j = i
        while j + 1 < len(ids) and ids[j + 1] == ids[j] + 1:
            j += 1
        out.append(str(ids[i]) if i == j else "%d-%d" % (ids[i], ids[j]))
        i = j + 1
    return ",".join(out) or "none"


def junction(link, target):
    """Make a junction at `link` that points to the folder `target`. True if it was made.
    Windows is asked directly: no command line is built from the two paths."""
    existed = os.path.lexists(link)
    try:
        import _winapi  # Windows only
        _winapi.CreateJunction(target, link)
    except (ImportError, AttributeError, OSError):
        if not existed:
            try:
                os.rmdir(link)  # a half-made link is an empty folder (or the link itself): never what it points at
            except OSError:
                pass
        return False
    return True


def streams(path):
    """The named data streams hung on one file or folder, e.g. [":hidden.html:$DATA"]. A file's own
    contents ("::$DATA") are left out, so an ordinary file gives []. Windows is asked directly."""
    class Found(ctypes.Structure):  # WIN32_FIND_STREAM_DATA
        _fields_ = [("size", ctypes.c_int64), ("name", ctypes.c_wchar * 296)]

    k = ctypes.WinDLL("kernel32", use_last_error=True)
    k.FindFirstStreamW.restype = ctypes.c_void_p
    k.FindFirstStreamW.argtypes = [ctypes.c_wchar_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    k.FindNextStreamW.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    k.FindClose.argtypes = [ctypes.c_void_p]
    found, out = Found(), []
    h = k.FindFirstStreamW(X + path, 0, ctypes.byref(found), 0)
    if h in (None, ctypes.c_void_p(-1).value):
        return out  # none at all (a folder), or a disk that has no streams
    try:
        while True:
            if found.name != "::$DATA":
                out.append(found.name)
            if not k.FindNextStreamW(h, ctypes.byref(found)):
                break
    finally:
        k.FindClose(h)
    return out


def hidden_streams(folder):
    """["name:stream:$DATA", ...] for the folder itself and everything directly in it: what `dir /r` shows."""
    out = [os.path.basename(folder) + s for s in streams(folder)]
    for name in sorted(os.listdir(folder)):
        out += [name + s for s in streams(os.path.join(folder, name))]
    return out


def write(path, data=b"x"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(X + path, "wb") as f:
        f.write(data if isinstance(data, bytes) else data.encode("utf-8"))
    return path


def set_times(path, ts):
    """Set created and modified time (os.utime can't set the creation time on Windows)."""
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    k.CreateFileW.restype = ctypes.c_void_p
    k.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    k.SetFileTime.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    k.CloseHandle.argtypes = [ctypes.c_void_p]
    h = k.CreateFileW(X + path, 0x100, 7, None, 3, 0x02000000, None)  # FILE_WRITE_ATTRIBUTES, share all, OPEN_EXISTING, backup semantics
    if h in (None, ctypes.c_void_p(-1).value):
        raise OSError("can't open %s" % path)
    ft = ctypes.c_uint64(int((ts + 11644473600) * 10000000))
    ok = k.SetFileTime(h, ctypes.byref(ft), None, ctypes.byref(ft))
    k.CloseHandle(h)
    if not ok:
        raise OSError("can't set times on %s" % path)


def hide(path):
    ctypes.windll.kernel32.SetFileAttributesW(X + path, 0x2 | 0x4)  # hidden + system


def make_lnk(target=None, args=None, relative=None, wide=False, network=False, env=None, workdir=None):
    """The bytes of a Shell Link (.lnk) written by hand from Microsoft's MS-SHLLINK layout.
    target: LinkInfo local path. wide: also store it as UTF-16 (the only place a non-ANSI path fits)."""
    flags, body = 0x80, b""  # IsUnicode: the strings after LinkInfo are UTF-16
    if target is not None or network:
        flags |= 0x2
        header = 0x24 if wide else 0x1C
        volume = struct.pack("<IIII", 0x11, 3, 0x1234ABCD, 0x10) + b"\0"  # VolumeID: fixed disk, a serial, empty label
        ansi = (target or "").encode("mbcs", "replace") + b"\0"
        info = b""
        vol_off = base_off = suffix_off = base_u = suffix_u = net_off = 0
        if target is not None:
            vol_off = header
            base_off = vol_off + len(volume)
            info += volume + ansi
        if network:
            net_off = header + len(info)
            name = b"\\\\server.example\\share\0"
            info += struct.pack("<5I", 0x14 + len(name), 0x2, 0x14, 0, 0x20000) + name  # CommonNetworkRelativeLink
        suffix_off = header + len(info)
        info += b"\0"
        if wide and target is not None:
            base_u = header + len(info)
            info += target.encode("utf-16-le") + b"\0\0"
            suffix_u = header + len(info)
            info += b"\0\0"
        li_flags = (0x1 if target is not None else 0) | (0x2 if network else 0)
        head = struct.pack("<7I", header + len(info), header, li_flags, vol_off, base_off, net_off, suffix_off)
        if wide:
            head += struct.pack("<2I", base_u, suffix_u)
        body += head + info
    for text, bit in ((relative, 0x8), (workdir, 0x10), (args, 0x20)):
        if text is not None:
            flags |= bit
            body += struct.pack("<H", len(text)) + text.encode("utf-16-le")
    if env is not None:
        flags |= 0x200
        body += struct.pack("<II", 0x314, 0xA0000001) + env.encode("mbcs", "replace").ljust(260, b"\0") + env.encode("utf-16-le").ljust(520, b"\0")
    body += struct.pack("<I", 0)  # terminal block
    head = struct.pack("<I16sIIQQQIIIHHII", 0x4C, LNK_CLSID, flags, 0x20, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0)
    assert len(head) == 76
    return head + body

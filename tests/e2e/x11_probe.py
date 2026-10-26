#!/usr/bin/env python3
"""Test-only X11 capture + XTest input helper for the Linux terminal-first probes.

Requires python3 ctypes and libX11/libXtst (no xdotool/ImageMagick). Used to capture
native screenshots and drive clicks/keys under Xvfb for the 2026-10-26 evidence. This
is a harness, not product code. Commands: find, capture FILE [window-index], focus,
type TEXT, key NAME, chord MOD+KEY, click X Y [button], drag X1 Y1 X2 Y2, close.
"""
import ctypes, ctypes.util, sys, time, zlib, struct, os

x11 = ctypes.CDLL(ctypes.util.find_library("X11"))
xtst = ctypes.CDLL(ctypes.util.find_library("Xtst"))

x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x11.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong,
                           ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
                           ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
                           ctypes.POINTER(ctypes.c_uint)]
x11.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x11.XGetGeometry.argtypes = [ctypes.c_void_p, ctypes.c_ulong,
                             ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int),
                             ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_uint),
                             ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
                             ctypes.POINTER(ctypes.c_uint)]
x11.XGetImage.restype = ctypes.c_void_p
x11.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                          ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
x11.XGetPixel.restype = ctypes.c_ulong
x11.XGetPixel.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
x11.XDestroyImage.argtypes = [ctypes.c_void_p]
x11.XFree.argtypes = [ctypes.c_void_p]
x11.XFlush.argtypes = [ctypes.c_void_p]
x11.XStringToKeysym.restype = ctypes.c_ulong
x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
x11.XKeysymToKeycode.restype = ctypes.c_ubyte
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
x11.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XMapWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XWarpPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong,
                             ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                             ctypes.c_int, ctypes.c_int]
xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]

x11.XInternAtom.restype = ctypes.c_ulong
x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]


class XClientMessageData(ctypes.Union):
    _fields_ = [("b", ctypes.c_char * 20), ("s", ctypes.c_short * 10),
                ("l", ctypes.c_long * 5)]


class XClientMessageEvent(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong),
                ("send_event", ctypes.c_int), ("display", ctypes.c_void_p),
                ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong),
                ("format", ctypes.c_int), ("data", XClientMessageData)]


def close_window(win):
    wm_protocols = x11.XInternAtom(dpy, b"WM_PROTOCOLS", 0)
    wm_delete = x11.XInternAtom(dpy, b"WM_DELETE_WINDOW", 0)
    ev = XClientMessageEvent()
    ev.type = 33  # ClientMessage
    ev.serial = 0
    ev.send_event = 1
    ev.display = dpy
    ev.window = win
    ev.message_type = wm_protocols
    ev.format = 32
    ev.data.l[0] = wm_delete
    ev.data.l[1] = CurrentTime
    x11.XSendEvent(dpy, win, 0, 0, ctypes.byref(ev))
    x11.XFlush(dpy)


ALL_PLANES = 0xFFFFFFFFFFFFFFFF
ZPIXMAP = 2
RevertToParent = 2
CurrentTime = 0

dpy = x11.XOpenDisplay(None)
if not dpy:
    print("cannot open display", os.environ.get("DISPLAY"), file=sys.stderr)
    sys.exit(1)


def children(win):
    rr = ctypes.c_ulong(); pr = ctypes.c_ulong()
    out = ctypes.POINTER(ctypes.c_ulong)(); n = ctypes.c_uint()
    if not x11.XQueryTree(dpy, win, ctypes.byref(rr), ctypes.byref(pr),
                          ctypes.byref(out), ctypes.byref(n)):
        return []
    kids = [out[i] for i in range(n.value)]
    if out:
        x11.XFree(out)
    return kids


def name(win):
    p = ctypes.c_char_p()
    if x11.XFetchName(dpy, win, ctypes.byref(p)) and p.value:
        return p.value.decode("utf-8", "replace")
    return ""


def geom(win):
    root = ctypes.c_ulong(); x = ctypes.c_int(); y = ctypes.c_int()
    w = ctypes.c_uint(); h = ctypes.c_uint(); bw = ctypes.c_uint(); depth = ctypes.c_uint()
    if not x11.XGetGeometry(dpy, win, ctypes.byref(root), ctypes.byref(x), ctypes.byref(y),
                            ctypes.byref(w), ctypes.byref(h), ctypes.byref(bw), ctypes.byref(depth)):
        return None
    return x.value, y.value, w.value, h.value


def walk(win, depth=0, out=None):
    if out is None:
        out = []
    out.append((win, depth))
    for k in children(win):
        walk(k, depth + 1, out)
    return out


def find_made():
    root = x11.XDefaultRootWindow(dpy)
    hits = []
    for win, depth in walk(root):
        g = geom(win)
        if not g:
            continue
        x, y, w, h = g
        nm = name(win)
        if nm and ("Made" in nm or nm.strip() == "made"):
            hits.append((win, x, y, w, h, nm))
    return hits


def capture(path, win=None, crop=None):
    root = x11.XDefaultRootWindow(dpy)
    if win is None:
        target = root
        g = geom(root)
        x = y = 0
        w, h = g[2], g[3]
    else:
        target = win
        gx, gy, gw, gh = geom(win)
        x, y, w, h = 0, 0, gw, gh
    img = x11.XGetImage(dpy, target, x, y, w, h, ALL_PLANES, ZPIXMAP)
    if not img:
        raise RuntimeError("XGetImage failed")
    rows = []
    for yy in range(h):
        row = bytearray()
        row.append(0)
        for xx in range(w):
            p = x11.XGetPixel(img, xx, yy)
            row += bytes(((p >> 16) & 255, (p >> 8) & 255, p & 255))
        rows.append(bytes(row))
    x11.XDestroyImage(img)
    raw = b"".join(rows)
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 6))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return w, h


def focus(win):
    x11.XRaiseWindow(dpy, win)
    x11.XMapWindow(dpy, win)
    x11.XSetInputFocus(dpy, win, RevertToParent, CurrentTime)
    x11.XFlush(dpy)


def key(name_, down=True):
    ks = x11.XStringToKeysym(name_.encode())
    kc = x11.XKeysymToKeycode(dpy, ks)
    if not kc:
        raise RuntimeError("no keycode for " + name_)
    xtst.XTestFakeKeyEvent(dpy, kc, 1 if down else 0, 0)
    x11.XFlush(dpy)


def type_text(text):
    shift = x11.XKeysymToKeycode(dpy, x11.XStringToKeysym(b"Shift_L"))
    # character -> (keysym name, shift)
    named = {" ": ("space", False), "\n": ("Return", False), "\t": ("Tab", False),
             "-": ("minus", False), ".": ("period", False), "/": ("slash", False),
             "'": ("apostrophe", False), ";": ("semicolon", False),
             "=": ("equal", False), ",": ("comma", False), "`": ("grave", False),
             "\\": ("backslash", False),
             "_": ("minus", True), ":": ("semicolon", True), "[": ("bracketleft", False),
             "]": ("bracketright", False), "{": ("bracketleft", True),
             "}": ("bracketright", True), "(": ("9", True), ")": ("0", True),
             "$": ("4", True), "!": ("1", True), "\"": ("apostrophe", True),
             "|": ("backslash", True), "<": ("comma", True), ">": ("period", True),
             "?": ("slash", True), "+": ("equal", True), "*": ("8", True),
             "&": ("7", True), "^": ("6", True), "%": ("5", True),
             "#": ("3", True), "@": ("2", True), "~": ("grave", True)}
    for ch in text:
        if ch in named:
            nm, need_shift = named[ch]
        elif ch.isupper():
            nm, need_shift = ch.lower(), True
        else:
            nm, need_shift = ch, False
        ks = x11.XStringToKeysym(nm.encode())
        if not ks and len(nm) == 1:
            ks = ord(nm)
        if not ks:
            raise RuntimeError("no keysym for " + repr(ch))
        kc = x11.XKeysymToKeycode(dpy, ks)
        if need_shift:
            xtst.XTestFakeKeyEvent(dpy, shift, 1, 0)
        xtst.XTestFakeKeyEvent(dpy, kc, 1, 0)
        xtst.XTestFakeKeyEvent(dpy, kc, 0, 0)
        if need_shift:
            xtst.XTestFakeKeyEvent(dpy, shift, 0, 0)
        x11.XFlush(dpy)
        time.sleep(0.003)


def chord(mods, keyname):
    aliases = {"Control": "Control_L", "Shift": "Shift_L", "Alt": "Alt_L", "Super": "Super_L"}
    codes = [x11.XKeysymToKeycode(dpy, x11.XStringToKeysym(aliases.get(m, m).encode())) for m in mods]
    for c in codes:
        xtst.XTestFakeKeyEvent(dpy, c, 1, 0)
    x11.XFlush(dpy)
    time.sleep(0.06)
    kc = x11.XKeysymToKeycode(dpy, x11.XStringToKeysym(keyname.encode()))
    xtst.XTestFakeKeyEvent(dpy, kc, 1, 0)
    x11.XFlush(dpy)
    time.sleep(0.03)
    xtst.XTestFakeKeyEvent(dpy, kc, 0, 0)
    x11.XFlush(dpy)
    time.sleep(0.03)
    for c in reversed(codes):
        xtst.XTestFakeKeyEvent(dpy, c, 0, 0)
    x11.XFlush(dpy)


def click(x, y, button=1):
    xtst.XTestFakeMotionEvent(dpy, 0, x, y, 0)
    x11.XFlush(dpy)
    time.sleep(0.05)
    xtst.XTestFakeButtonEvent(dpy, button, 1, 0)
    xtst.XTestFakeButtonEvent(dpy, button, 0, 0)
    x11.XFlush(dpy)


def drag(x1, y1, x2, y2, steps=12):
    xtst.XTestFakeMotionEvent(dpy, 0, x1, y1, 0)
    x11.XFlush(dpy)
    time.sleep(0.05)
    xtst.XTestFakeButtonEvent(dpy, 1, 1, 0)
    x11.XFlush(dpy)
    time.sleep(0.05)
    for i in range(1, steps + 1):
        px = int(x1 + (x2 - x1) * i / steps)
        py = int(y1 + (y2 - y1) * i / steps)
        xtst.XTestFakeMotionEvent(dpy, 0, px, py, 0)
        x11.XFlush(dpy)
        time.sleep(0.02)
    xtst.XTestFakeButtonEvent(dpy, 1, 0, 0)
    x11.XFlush(dpy)


def pixel(path, x, y):
    # quick PNG decode via this module's own output format
    data = open(path, "rb").read()
    pos = 8
    idat = b""
    w = h = 0
    while pos < len(data):
        ln = struct.unpack(">I", data[pos:pos+4])[0]
        typ = data[pos+4:pos+8]
        body = data[pos+8:pos+8+ln]
        if typ == b"IHDR":
            w, h = struct.unpack(">II", body[:8])
        elif typ == b"IDAT":
            idat += body
        pos += 12 + ln
    raw = zlib.decompress(idat)
    stride = w * 3 + 1
    row = raw[y*stride+1:(y+1)*stride]
    return tuple(row[x*3:x*3+3])


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "find":
        print(find_made())
    elif cmd == "capture":
        win = None
        if len(sys.argv) > 3:
            hits = find_made()
            win = hits[int(sys.argv[3])][0] if hits else None
        print(capture(sys.argv[2], win))
    elif cmd == "focus":
        hits = find_made()
        focus(hits[int(sys.argv[2]) if len(sys.argv) > 2 else 0][0])
    elif cmd == "type":
        type_text(sys.argv[2])
    elif cmd == "key":
        key(sys.argv[2], True); time.sleep(0.02); key(sys.argv[2], False)
    elif cmd == "chord":
        chord(sys.argv[2].split("+"), sys.argv[3])
    elif cmd == "click":
        click(int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]) if len(sys.argv) > 4 else 1)
    elif cmd == "drag":
        drag(int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]))
    elif cmd == "close":
        close_window(find_made()[0][0])
    else:
        raise SystemExit("unknown command")

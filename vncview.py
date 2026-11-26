#!/usr/bin/env python3
"""Minimal scaling VNC viewer for the Inferno iPhone screen (GTK3 + cairo).

The emulated display is 832x1792, taller than most monitors. This viewer
fits the whole screen into the window (aspect preserved) and forwards mouse
clicks/drags as touches. Usage: vncview.py [host] [port]
"""
import os, socket, struct, sys, threading
# On Wayland, GTK ignores aspect-ratio hints and the "no maximize" request.
# Run through XWayland so KWin enforces both (like the Android emulator).
os.environ.setdefault("GDK_BACKEND", "x11")
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gtk, Gdk, GLib
import cairo

HOST = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1"
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 5900


class RFB:
    def __init__(self, host, port):
        self.s = socket.create_connection((host, port), timeout=30)
        self.s.settimeout(None)
        self.lock = threading.Lock()
        self.r(12)
        self.send(b"RFB 003.008\n")
        n = self.r(1)[0]
        if n == 0:
            raise SystemExit("VNC server refused connection")
        if 1 not in self.r(n):
            raise SystemExit("VNC server requires auth (unsupported)")
        self.send(b"\x01")
        if struct.unpack(">I", self.r(4))[0] != 0:
            raise SystemExit("VNC security handshake failed")
        self.send(b"\x01")  # shared
        self.w, self.h = struct.unpack(">HH", self.r(4))
        self.r(16)
        self.r(struct.unpack(">I", self.r(4))[0])
        # 32bpp little-endian BGRX == cairo FORMAT_RGB24 on little-endian hosts
        self.send(struct.pack(">BxxxBBBBHHHBBBxxx", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
        self.send(struct.pack(">BxHii", 2, 2, 0, -258))  # raw + QEMU Extended Key Event
        self.ext_keys = False
        self.stride = cairo.ImageSurface.format_stride_for_width(cairo.FORMAT_RGB24, self.w)
        self.fb = bytearray(self.stride * self.h)

    def r(self, n):
        buf = bytearray()
        while len(buf) < n:
            c = self.s.recv(n - len(buf))
            if not c:
                raise EOFError
            buf += c
        return bytes(buf)

    def send(self, data):
        with self.lock:
            self.s.sendall(data)

    def request(self, incremental):
        self.send(struct.pack(">BBHHHH", 3, incremental, 0, 0, self.w, self.h))

    def pointer(self, mask, x, y):
        x = max(0, min(self.w - 1, int(x)))
        y = max(0, min(self.h - 1, int(y)))
        self.send(struct.pack(">BBHH", 5, mask, x, y))

    def key(self, down, keysym, xt_code=0):
        if self.ext_keys and xt_code:
            self.send(struct.pack(">BBHII", 255, 0, 1 if down else 0, int(keysym) & 0xffffffff, xt_code))
        else:
            self.send(struct.pack(">BBxxI", 4, 1 if down else 0, int(keysym) & 0xffffffff))

    def loop(self, on_update):
        self.request(0)
        while True:
            t = self.r(1)[0]
            if t == 0:
                self.r(1)
                updated = False
                for _ in range(struct.unpack(">H", self.r(2))[0]):
                    x, y, rw, rh, enc = struct.unpack(">HHHHi", self.r(12))
                    if enc == -258:  # QEMU Extended Key Event ack
                        self.ext_keys = True
                        continue
                    if enc != 0:
                        raise SystemExit("unexpected encoding %d" % enc)
                    data = self.r(rw * rh * 4)
                    for j in range(rh):
                        o = (y + j) * self.stride + x * 4
                        self.fb[o:o + rw * 4] = data[j * rw * 4:(j + 1) * rw * 4]
                    updated = True
                if updated:
                    GLib.idle_add(on_update)
                    self.request(1)
            elif t == 1:  # colour map
                self.r(3)
                self.r(struct.unpack(">H", self.r(2))[0] * 6)
            elif t == 2:  # bell
                pass
            elif t == 3:  # cut text
                self.r(3)
                self.r(struct.unpack(">I", self.r(4))[0])


class Viewer(Gtk.Window):
    def __init__(self, rfb):
        super().__init__(title="Inferno iPhone")
        self.rfb = rfb
        self.buttons = 0
        geo = Gdk.Display.get_default().get_monitor(0).get_workarea()
        scale = min(1.0, geo.height * 0.92 / rfb.h, geo.width * 0.92 / rfb.w)
        self.set_default_size(int(rfb.w * scale), int(rfb.h * scale))
        self.aspect = rfb.w / rfb.h
        hints = Gdk.Geometry()
        hints.min_aspect = hints.max_aspect = self.aspect
        hints.min_width, hints.min_height = int(rfb.w * 0.2), int(rfb.h * 0.2)
        hints.max_width, hints.max_height = rfb.w, rfb.h
        self.set_geometry_hints(None, hints, Gdk.WindowHints.ASPECT
                                | Gdk.WindowHints.MIN_SIZE | Gdk.WindowHints.MAX_SIZE)
        self._snap_id = 0
        self.connect("realize", self._no_maximize)
        self.connect("window-state-event", self._state)
        self.connect("configure-event", self._configure)
        self.area = Gtk.DrawingArea()
        self.area.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK
                             | Gdk.EventMask.POINTER_MOTION_MASK)
        self.area.connect("draw", self.draw)
        self.area.connect("button-press-event", self.btn)
        self.area.connect("button-release-event", self.btn)
        self.area.connect("motion-notify-event", self.motion)
        self.add(self.area)
        self.add_events(Gdk.EventMask.KEY_PRESS_MASK | Gdk.EventMask.KEY_RELEASE_MASK)
        self.connect("key-press-event", self.on_key)
        self.connect("key-release-event", self.on_key)
        self.connect("destroy", Gtk.main_quit)

    def _no_maximize(self, _w):
        # Remove the maximize button / action from the window manager.
        self.get_window().set_functions(Gdk.WMFunction.RESIZE | Gdk.WMFunction.MOVE
                                        | Gdk.WMFunction.MINIMIZE | Gdk.WMFunction.CLOSE)

    def _state(self, _w, ev):
        # Fallback: undo maximize/fullscreen if the WM still applies one
        # (e.g. double-click on the title bar or a keyboard shortcut).
        st = ev.new_window_state
        if st & Gdk.WindowState.MAXIMIZED:
            GLib.idle_add(self.unmaximize)
        if st & Gdk.WindowState.FULLSCREEN:
            GLib.idle_add(self.unfullscreen)
        return False

    def _configure(self, _w, ev):
        # Fallback for WMs that ignore aspect hints: once resizing stops,
        # snap the window to the phone's aspect so the screen fills it.
        if self._snap_id:
            GLib.source_remove(self._snap_id)
        self._snap_id = GLib.timeout_add(250, self._snap)
        return False

    def _snap(self):
        self._snap_id = 0
        w, h = self.get_size()
        want_h = round(w / self.aspect)
        if abs(want_h - h) > 2:
            self.resize(w, want_h)
        return False

    def geom(self):
        aw, ah = self.area.get_allocated_width(), self.area.get_allocated_height()
        s = min(aw / self.rfb.w, ah / self.rfb.h)
        return s, (aw - self.rfb.w * s) / 2, (ah - self.rfb.h * s) / 2

    def draw(self, _w, cr):
        cr.set_source_rgb(0, 0, 0)
        cr.paint()
        s, ox, oy = self.geom()
        surf = cairo.ImageSurface.create_for_data(self.rfb.fb, cairo.FORMAT_RGB24,
                                                  self.rfb.w, self.rfb.h, self.rfb.stride)
        cr.translate(ox, oy)
        cr.scale(s, s)
        cr.set_source_surface(surf, 0, 0)
        cr.get_source().set_filter(cairo.FILTER_GOOD)
        cr.paint()

    def to_fb(self, ev):
        s, ox, oy = self.geom()
        return (ev.x - ox) / s, (ev.y - oy) / s

    def btn(self, _w, ev):
        bit = {1: 1, 2: 2, 3: 4}.get(ev.button, 0)
        if ev.type == Gdk.EventType.BUTTON_PRESS:
            self.buttons |= bit
        elif ev.type == Gdk.EventType.BUTTON_RELEASE:
            self.buttons &= ~bit
        self.rfb.pointer(self.buttons, *self.to_fb(ev))

    def motion(self, _w, ev):
        self.rfb.pointer(self.buttons, *self.to_fb(ev))

    def on_key(self, _w, ev):
        # Map X11 keysyms for Inferno Apple Buttons (F1-F10) and standard evdev codes to XT keycodes
        fn_xt = {
            0xffbe: 0x3b, 0xffbf: 0x3c, 0xffc0: 0x3d, 0xffc1: 0x3e, 0xffc2: 0x3f,
            0xffc3: 0x40, 0xffc4: 0x41, 0xffc5: 0x42, 0xffc6: 0x43, 0xffc7: 0x44,
        }
        xt = fn_xt.get(ev.keyval, 0)
        if not xt and 9 <= ev.hardware_keycode <= 96:
            xt = ev.hardware_keycode - 8
        self.rfb.key(ev.type == Gdk.EventType.KEY_PRESS, ev.keyval, xt)
        return True

    def update(self):
        self.area.queue_draw()
        return False


def main():
    rfb = RFB(HOST, PORT)
    win = Viewer(rfb)
    win.show_all()

    def run():
        try:
            rfb.loop(win.update)
        except (EOFError, OSError):
            GLib.idle_add(Gtk.main_quit)

    threading.Thread(target=run, daemon=True).start()
    Gtk.main()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""A window that pastes clipboard images on request: paste_target.py OUTDIR

Each time OUTDIR/req appears, the target deletes it, reads the clipboard as an
image (one read = one paste, like a real app) and saves OUTDIR/paste-N.png. It
logs "N WxH" or "N error: ..." to OUTDIR/pastes.log. On Wayland only the focused
window may read the clipboard, so the harness focuses "Paste Target" first.
"""
import os, sys, gi
gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gtk, Gdk, GLib

out = sys.argv[1]
n = 0


def log(line):
    with open(os.path.join(out, "pastes.log"), "a") as f:
        f.write(line + "\n")


def done(clip, res):
    global n
    n += 1
    try:
        t = clip.read_texture_finish(res)
        if t is None:
            raise GLib.Error("no texture")
        t.save_to_png(os.path.join(out, f"paste-{n}.png"))
        log(f"{n} {t.get_width()}x{t.get_height()}")
    except GLib.Error as e:
        log(f"{n} error: {e.message}")


def poll(win):
    req = os.path.join(out, "req")
    if os.path.exists(req):
        os.remove(req)
        win.get_clipboard().read_texture_async(None, done)
    return True


def go(app):
    w = Gtk.ApplicationWindow(application=app, title="Paste Target")
    w.set_default_size(420, 260)
    w.set_child(Gtk.Label(label="paste target"))
    w.present()
    GLib.timeout_add(50, poll, w)


app = Gtk.Application(application_id="org.ledgegrab.PasteTarget")
app.connect("activate", go)
app.run([])

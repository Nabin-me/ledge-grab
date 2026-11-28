#!/usr/bin/env python3
"""Another app taking the clipboard (with text) when its button is clicked.

Like a real copy, the take must follow an input event in this app: mutter
ignores a new clipboard owner whose input serial is older than the current
owner's."""
import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gtk, GLib


def go(app):
    w = Gtk.ApplicationWindow(application=app, title="Taker")
    w.set_default_size(300, 200)
    b = Gtk.Button(label="Copy text")

    def take(_b):
        w.get_clipboard().set("taken")
        print("took the clipboard", flush=True)
        GLib.timeout_add(1500, lambda: app.quit() or False)

    b.connect("clicked", take)
    w.set_child(b)
    w.present()
    GLib.timeout_add(15000, lambda: app.quit() or False)


app = Gtk.Application(application_id="org.ledgegrab.Taker")
app.connect("activate", go)
app.run([])

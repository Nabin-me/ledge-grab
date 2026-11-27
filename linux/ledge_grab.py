#!/usr/bin/python3
"""Linux preview for Ledge Grab. No screenshots leave the local session."""
import argparse
import os
import sys
from pathlib import Path
import gi
for name, version in (("Gtk","3.0"),("Gdk","3.0"),("GdkPixbuf","2.0")):
    gi.require_version(name,version)
from gi.repository import Gdk, GdkPixbuf, Gio, GLib, Gtk
from portal import request_screenshot
from storage import Storage, crop_bounds, data_directory

APP_ID = "app.ledgegrab.Preview"

class Area(Gtk.Window):
    def __init__(self, pix, logical, callback):
        super().__init__(type=Gtk.WindowType.POPUP)
        self.pix,self.logical,self.callback=pix,logical,callback
        self.start=self.end=None
        self.done=False
        self.set_default_size(*logical)
        self.move(0,0)
        self.set_keep_above(True)
        self.set_modal(True)
        layers=Gtk.Overlay()
        frozen=pix if (pix.get_width(),pix.get_height())==logical else pix.scale_simple(*logical,GdkPixbuf.InterpType.BILINEAR)
        self.canvas=Gtk.EventBox()
        self.canvas.add(Gtk.Image.new_from_pixbuf(frozen))
        layers.add(self.canvas)
        self.indicators=Gtk.Fixed()
        layers.add_overlay(self.indicators)
        layers.set_overlay_pass_through(self.indicators,True)
        self.outline=Gtk.EventBox()
        self.outline.set_name("captureOutline")
        self.outline.set_no_show_all(True)
        self.indicators.put(self.outline,0,0)
        self.add(layers)
        self.canvas.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK | Gdk.EventMask.POINTER_MOTION_MASK)
        self.canvas.connect("button-press-event",self.press)
        self.canvas.connect("button-release-event",self.release)
        self.canvas.connect("motion-notify-event",self.motion)
        self.connect("key-press-event",self.key)
        self.show_all()
        self.present()
        window=self.canvas.get_window()
        seat=window.get_display().get_default_seat()
        window.set_cursor(Gdk.Cursor.new_from_name(window.get_display(),"crosshair"))
        result=seat.grab(window,Gdk.SeatCapabilities.ALL_POINTING|Gdk.SeatCapabilities.KEYBOARD,False,None,None,None,None)
        self.seat=seat if result==Gdk.GrabStatus.SUCCESS else None
        if self.seat is None:
            GLib.idle_add(self.failed)

    def failed(self):
        self.finish(None,"Couldn’t start area selection. Try Capture screen.")
        return GLib.SOURCE_REMOVE

    def update_outline(self):
        if self.start is None or self.end is None:
            self.outline.hide()
            return
        x,y,w,h=crop_bounds(self.start,self.end,self.logical,self.logical)
        if min(w,h)<3:
            self.outline.hide()
            return
        self.indicators.move(self.outline,x,y)
        self.outline.set_size_request(w,h)
        self.outline.show()

    def key(self,_widget,event):
        if event.keyval==Gdk.KEY_Escape:
            self.finish()
            return True
        return False

    def press(self,_widget,e):
        if e.button==3:
            self.finish()
        if e.button==1:
            self.start=self.end=(e.x,e.y)
        return True

    def motion(self,widget,e):
        if self.start:
            self.end=(e.x,e.y)
            self.update_outline()
        return True

    def release(self,_widget,e):
        if e.button!=1 or not self.start:
            return True
        x,y,w,h=crop_bounds(self.start,(e.x,e.y),self.logical,(self.pix.get_width(),self.pix.get_height()))
        if min(w,h)<4:
            self.start=self.end=None
            self.update_outline()
            return True
        self.finish(self.pix.new_subpixbuf(x,y,w,h).copy())
        return True

    def finish(self,pix=None,error=None):
        if self.done: return
        self.done=True
        if self.seat: self.seat.ungrab()
        self.destroy()
        self.callback(pix,error)

class Shelf(Gtk.Application):
    def __init__(self, initial_capture=None):
        super().__init__(application_id=APP_ID)
        self.win=None
        self.busy=False
        self.area=None
        self.initial_capture=initial_capture

    def do_startup(self):
        Gtk.Application.do_startup(self)
        action=Gio.SimpleAction.new("capture",GLib.VariantType.new("s"))
        action.connect("activate",self.on_capture)
        self.add_action(action)

    def on_capture(self,_action,param):
        mode=param.get_string()
        if mode not in ("area","screen"):
            return
        if self.win is None:
            self.activate()
        if self.win is not None:
            self.capture(mode)

    def run_initial_capture(self):
        mode=self.initial_capture
        self.initial_capture=None
        if self.win is not None:
            self.capture(mode)
        return GLib.SOURCE_REMOVE

    def do_activate(self):
        if self.win is not None:
            if not self.busy: self.win.present()
            return
        disp=Gdk.Display.get_default()
        if disp is None:
            print("Ledge Grab requires a graphical Linux session.",file=sys.stderr)
            return
        if not os.environ.get("LEDGE_GRAB_DIR"):
            pictures=GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_PICTURES)
            root=(Path(pictures) / "Ledge Grab") if pictures else data_directory()
        else:
            root=data_directory()
        try:
            self.storage=Storage(root)
        except OSError as e:
            print(f"Cannot open capture folder: {e}",file=sys.stderr)
            return
        self.x11=disp.__gtype__.name=="GdkX11Display" and os.environ.get("XDG_SESSION_TYPE","").lower()!="wayland"
        self.win=Gtk.ApplicationWindow(application=self,title="Ledge Grab")
        self.win.set_resizable(False)
        self.win.set_default_size(360,-1)
        if self.x11:
            self.win.set_keep_above(True)
            self.win.set_gravity(Gdk.Gravity.NORTH)
            self.win.set_skip_taskbar_hint(False)
        self.win.connect("destroy",self.closed)
        self.win.set_icon_name("app.ledgegrab.Preview")
        self.win.set_name("ledgeWindow")
        box=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=9,margin=12)
        self.win.add(box)
        bar=Gtk.Box(spacing=8)
        box.pack_start(bar,False,False,0)
        info=Gtk.Label(label="Ledge Grab",xalign=0)
        info.get_style_context().add_class("title")
        bar.pack_start(info,True,True,0)
        self.toggle=Gtk.Button(label="Recent")
        self.toggle.connect("clicked",self.flip)
        bar.pack_start(self.toggle,False,False,0)
        folder=Gtk.Button.new_from_icon_name("folder-open-symbolic",Gtk.IconSize.BUTTON)
        folder.set_tooltip_text("Open captures folder")
        folder.connect("clicked",lambda b:self.open_path(self.storage.root))
        bar.pack_start(folder,False,False,0)
        capture=Gtk.Box(spacing=7)
        box.pack_start(capture,False,False,0)
        main=Gtk.Button(label="Capture area" if self.x11 else "Take screenshot")
        main.get_style_context().add_class("suggested-action")
        main.connect("clicked",lambda b:self.capture("area"))
        capture.pack_start(main,True,True,0)
        if self.x11:
            whole=Gtk.Button(label="Capture screen")
            whole.connect("clicked",lambda b:self.capture("screen"))
            capture.pack_start(whole,True,True,0)
        self.status=Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self.status.set_max_width_chars(40)
        self.status.get_style_context().add_class("dim-label")
        box.pack_start(self.status,False,False,0)
        self.set_status("X11 preview · Saves locally and copies" if self.x11 else "Desktop screenshot chooser · Saves and copies")
        self.revealer=Gtk.Revealer()
        self.revealer.set_transition_duration(140)
        self.revealer.set_transition_type(Gtk.RevealerTransitionType.SLIDE_DOWN)
        box.pack_start(self.revealer,True,True,0)
        scroll=Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER,Gtk.PolicyType.AUTOMATIC)
        scroll.set_max_content_height(274)
        scroll.set_propagate_natural_height(True)
        self.revealer.add(scroll)
        self.list=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=5)
        scroll.add(self.list)
        css=Gtk.CssProvider()
        css.load_from_data(b"#ledgeWindow { background-color: #202223; color: #f1f0ec; } .title { font-size: 15px; font-weight: bold; } #captureOutline { border: 2px solid #a5eaff; background-color: rgba(0,100,185,0.14); }")
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(),css,Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.win.show_all()
        self.refresh()
        if self.x11:
            monitor=disp.get_primary_monitor() or disp.get_monitor(0)
            if monitor:
                work=monitor.get_workarea()
                self.win.move(work.x+max(0,(work.width-self.win.get_size().width)//2),work.y+8)
        if self.initial_capture:
            GLib.idle_add(self.run_initial_capture)

    def closed(self,_window):
        self.win=None
        if self.area is not None:
            self.area.finish()

    def set_status(self,text):
        self.status.set_text(text)

    def flip(self,_button):
        opened=not self.revealer.get_reveal_child()
        self.revealer.set_reveal_child(opened)
        self.toggle.set_label("Close shelf" if opened else "Recent")

    def open_path(self,path):
        try:
            Gio.AppInfo.launch_default_for_uri(Path(path).resolve().as_uri(),None)
        except GLib.Error:
            self.set_status("Couldn’t open the saved capture folder.")

    def refresh(self):
        for old in self.list.get_children(): self.list.remove(old)
        recent=self.storage.recent()
        if not recent:
            self.list.pack_start(Gtk.Label(label="No recent captures",xalign=0),False,False,2)
        for path in recent:
            row=Gtk.Box(spacing=6)
            try:
                pix=GdkPixbuf.Pixbuf.new_from_file_at_scale(str(path),72,44,True)
            except GLib.Error:
                continue
            thumb=Gtk.Button()
            thumb.set_image(Gtk.Image.new_from_pixbuf(pix))
            thumb.set_tooltip_text("Open capture")
            thumb.connect("clicked",lambda _b,p=path:self.open_path(p))
            row.pack_start(thumb,False,False,0)
            date=Gtk.Label(label=path.name[10:18]+"  "+path.name[19:21]+":"+path.name[21:23],xalign=0)
            row.pack_start(date,True,True,0)
            for title,action in (("Copy",self.copy_path),("Hide",self.hide_path)):
                b=Gtk.Button(label=title)
                b.connect("clicked",lambda _b,p=path,a=action:a(p))
                row.pack_start(b,False,False,0)
            self.list.pack_start(row,False,False,0)
        self.list.show_all()

    def hide_path(self,path):
        try:
            self.storage.hide(path)
            self.refresh()
            self.set_status("Hidden from Recent. File kept.")
        except (ValueError,OSError):
            self.set_status("Couldn’t hide this preview.")

    def copy(self,pix):
        clip=Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        clip.set_image(pix)
        clip.store()

    def copy_path(self,path):
        try:
            self.copy(GdkPixbuf.Pixbuf.new_from_file(str(path)))
            self.set_status("Copied image to clipboard.")
        except GLib.Error:
            self.set_status("Couldn’t copy this image.")

    def capture(self,mode):
        if self.busy: return
        self.busy=True
        self.win.hide()
        GLib.timeout_add(220,self.acquire,mode)

    def acquire(self,mode):
        if self.win is None:
            return GLib.SOURCE_REMOVE
        if not self.x11:
            request_screenshot(self.portal_done)
        else:
            root=Gdk.get_default_root_window()
            if root is None:
                self.completed(None,"The X11 screen isn’t available.")
                return GLib.SOURCE_REMOVE
            logical=(root.get_width(),root.get_height())
            pixels=Gdk.pixbuf_get_from_window(root,0,0,*logical)
            if pixels is None:
                self.completed(None,"Couldn’t read the X11 screen.")
            elif mode=="screen":
                self.completed(pixels)
            else:
                self.area=Area(pixels,logical,self.completed)
        return GLib.SOURCE_REMOVE

    def portal_done(self,uri,error):
        if uri is None:
            self.completed(None,error)
            return
        file=Gio.File.new_for_uri(uri)
        path=file.get_path()
        if file.get_uri_scheme()!="file" or not path:
            self.completed(None,"The screenshot portal returned a non-local file.")
            return
        try:
            pix=GdkPixbuf.Pixbuf.new_from_file(path)
        except GLib.Error:
            self.completed(None,"Couldn’t open the portal screenshot.")
            return
        self.completed(pix)

    def completed(self,pix=None,error=None):
        self.busy=False
        self.area=None
        if self.win is None: return
        self.win.show()
        if pix is None:
            self.set_status(error or "Capture canceled.")
            return
        try:
            path=self.storage.save(lambda tmp:pix.savev(tmp,"png",[],[]))
            try:
                self.copy(pix)
                self.set_status("Saved and copied.")
            except GLib.Error:
                self.set_status(f"Saved {path.name}. Clipboard unavailable.")
            self.refresh()
        except (OSError,ValueError,GLib.Error) as e:
            self.set_status("Couldn’t save capture: "+str(e))

def main(argv=None):
    parser=argparse.ArgumentParser(prog="ledge-grab-linux",description="Capture, copy and revisit screenshots.")
    parser.add_argument("--capture",choices=["area","screen"],help="Request capture from the running shelf")
    args=parser.parse_args(argv)
    if not Gtk.init_check()[0]:
        print("Ledge Grab needs a graphical Linux desktop session (X11 or Wayland).",file=sys.stderr)
        return 1
    app=Shelf(args.capture)
    try:
        app.register(None)
        if app.get_is_remote() and args.capture:
            app.activate_action("capture",GLib.Variant("s",args.capture))
            bus=app.get_dbus_connection()
            if bus is not None:
                bus.flush_sync(None)
    except GLib.Error as e:
        print("Couldn’t connect to the desktop session: "+str(e),file=sys.stderr)
        return 1
    return app.run([sys.argv[0]])

if __name__=="__main__":
    sys.exit(main())

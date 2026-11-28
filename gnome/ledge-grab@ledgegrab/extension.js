import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// GNOME doesn't let an app place its own windows, keep them above, or take the
// clipboard while unfocused on Wayland. This extension does those for Ledge Grab:
//  - the ledge stays above everything, on every workspace, centred under the top bar;
//  - the region picker gets keyboard focus (so Esc works);
//  - the app can take a screenshot without a permission prompt, and hand over a
//    just-saved screenshot for the shell to put on the clipboard.
// Both D-Bus methods answer only the ledge-grab binary itself.
const APP_ID = 'org.ledgegrab.LedgeGrab';
const LEDGE_TITLE = 'Ledge Grab';
const PICKER_TITLE = 'Ledge Grab region';

const IFACE = `<node>
  <interface name="org.ledgegrab.Shell">
    <method name="Screenshot">
      <arg type="s" direction="in" name="path"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="SetClipboardImage">
      <arg type="s" direction="in" name="path"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

export default class LedgeGrabExtension extends Extension {
    enable() {
        this._watched = new Map();
        this._created = global.display.connect('window-created', (_d, win) => this._adopt(win));
        for (const actor of global.get_window_actors())
            this._adopt(actor.meta_window);
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, '/org/ledgegrab/Shell');
    }

    disable() {
        this._dbus?.unexport();
        this._dbus = null;
        global.display.disconnect(this._created);
        for (const [win, ids] of this._watched)
            for (const id of ids) win.disconnect(id);
        this._watched = null;
    }

    get Version() {
        return 1;
    }

    // Is the caller the Ledge Grab app? Same-user, unsandboxed processes only;
    // sandboxed apps can't reach the shell's bus name at all.
    _fromApp(invocation) {
        try {
            const [pid] = Gio.DBus.session.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'GetConnectionUnixProcessID',
                new GLib.Variant('(s)', [invocation.get_sender()]), null, Gio.DBusCallFlags.NONE, 1000, null).deep_unpack();
            return GLib.path_get_basename(GLib.file_read_link(`/proc/${pid}/exe`)) === 'ledge-grab';
        } catch (e) {
            return false;
        }
    }

    _reply(invocation, ok) {
        invocation.return_value(new GLib.Variant('(b)', [ok]));
    }

    ScreenshotAsync([path], invocation) {
        if (!this._fromApp(invocation) || !path.endsWith('.png'))
            return this._reply(invocation, false);
        let stream;
        try {
            stream = Gio.File.new_for_path(path).replace(null, false, Gio.FileCreateFlags.PRIVATE, null);
        } catch (e) {
            return this._reply(invocation, false);
        }
        new Shell.Screenshot().screenshot(false, stream, (shooter, res) => {
            let ok = false;
            try {
                shooter.screenshot_finish(res);
                ok = true;
            } catch (e) {
                console.warn(`Ledge Grab: screenshot: ${e.message}`);
            }
            stream.close(null);
            this._reply(invocation, ok);
        });
    }

    SetClipboardImageAsync([path], invocation) {
        // Only PNGs the app itself just wrote are accepted.
        if (!this._fromApp(invocation) || !path.endsWith('.png') || !GLib.file_test(path, GLib.FileTest.IS_REGULAR))
            return this._reply(invocation, false);
        try {
            const [, bytes] = GLib.file_get_contents(path);
            St.Clipboard.get_default().set_content(St.ClipboardType.CLIPBOARD, 'image/png', new GLib.Bytes(bytes));
            this._reply(invocation, true);
        } catch (e) {
            console.warn(`Ledge Grab: clipboard: ${e.message}`);
            this._reply(invocation, false);
        }
    }

    _adopt(win) {
        if (this._watched.has(win))
            return;
        // On Wayland the app id and title arrive after the window is created.
        const ids = [
            win.connect('notify::wm-class', () => this._place(win)),
            win.connect('notify::gtk-application-id', () => this._place(win)),
            win.connect('notify::title', () => this._place(win)),
            win.connect('size-changed', () => this._place(win)),
            // Anything else moving the ledge (initial X11 placement, monitor changes): put it back.
            win.connect('position-changed', () => this._later(win)),
            // X11 windows get their initial placement after they're shown; move after it.
            win.connect('shown', () => this._later(win)),
            win.connect('unmanaged', () => {
                for (const id of ids) win.disconnect(id);
                this._watched?.delete(win);
            }),
        ];
        this._watched.set(win, ids);
        this._place(win);
    }

    _later(win) {
        GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            if (this._watched?.has(win))
                this._place(win);
            return GLib.SOURCE_REMOVE;
        });
    }

    _place(win) {
        // Wayland reports the app id as the class; GTK on X11 only sets its own property.
        if (win.get_gtk_application_id() !== APP_ID && win.get_wm_class() !== APP_ID)
            return;
        const title = win.get_title();
        if (title === PICKER_TITLE) {
            if (!win.has_focus())
                win.activate(global.get_current_time());
            return;
        }
        if (title !== LEDGE_TITLE)
            return;
        if (!win.is_above()) win.make_above();
        if (!win.is_on_all_workspaces()) win.stick();
        const mon = Main.layoutManager.primaryMonitor;
        if (!mon) return;
        const r = win.get_frame_rect();
        const top = Main.panel.visible ? mon.y + Main.panel.height : mon.y;
        const x = mon.x + Math.round((mon.width - r.width) / 2);
        if (r.x !== x || r.y !== top)
            win.move_frame(true, x, top);
    }
}

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// Test sessions only: unlock org.gnome.Shell.Eval / Introspect / Screenshot for the
// harness, and close the startup overview so test windows are visible.
export default class TestHelper extends Extension {
    enable() {
        global.context.unsafe_mode = true;
        // lgctl.py reads the panel and layout through this.
        global._lgMain = Main;
        const hide = () => Main.overview.hide();
        if (Main.layoutManager._startingUp)
            this._id = Main.layoutManager.connect('startup-complete', hide);
        else
            hide();
    }
    disable() {
        if (this._id) Main.layoutManager.disconnect(this._id);
        this._id = null;
    }
}

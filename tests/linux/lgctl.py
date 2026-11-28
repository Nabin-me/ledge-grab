#!/usr/bin/env python3
"""Drive the headless GNOME test session through org.gnome.Shell.Eval.

  lgctl.py rect TITLE          -> "x y w h" of the window with that title
  lgctl.py focus TITLE         -> activate that window
  lgctl.py move X Y            -> move the virtual pointer (stage coordinates)
  lgctl.py click X Y           -> move and click the left button
  lgctl.py drag X0 Y0 X1 Y1    -> press, move in steps, release
  lgctl.py key Escape          -> tap a key by its Clutter keyval name
  lgctl.py panel               -> height of the top bar

Needs the lg-test-helper extension (it turns on unsafe mode).
"""
import json, subprocess, sys, time

PRELUDE = """
const Clutter = imports.gi.Clutter;
const seat = Clutter.get_default_backend().get_default_seat();
global._lgPtr ??= seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
global._lgKbd ??= seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
const P = global._lgPtr, K = global._lgKbd;
const win = t => global.get_window_actors().map(a => a.meta_window).find(w => w.get_title() === t);
"""


def ev(js):
    r = subprocess.run(["gdbus", "call", "--session", "--dest", "org.gnome.Shell", "--object-path",
                        "/org/gnome/Shell", "--method", "org.gnome.Shell.Eval", " ".join((PRELUDE + js).splitlines()).strip()],
                       capture_output=True, text=True)
    out = r.stdout.strip()
    if r.returncode != 0 or not out.startswith("(true"):
        raise SystemExit(f"eval failed: {out} {r.stderr.strip()}")
    # (true, '"value"')
    raw = out[out.index(",") + 1:].strip().rstrip(")").strip()
    raw = raw[1:-1] if raw[:1] in "'\"" else raw
    raw = raw.encode().decode("unicode_escape")
    try:
        return json.loads(raw) if raw else None
    except json.JSONDecodeError:
        return raw


def move(x, y):
    ev(f"P.notify_absolute_motion(Clutter.CURRENT_TIME, {x}, {y}); 1")


def button(down):
    st = "PRESSED" if down else "RELEASED"
    ev(f"P.notify_button(Clutter.CURRENT_TIME, Clutter.BUTTON_PRIMARY, Clutter.ButtonState.{st}); 1")


def main(a):
    cmd = a[0]
    if cmd == "rect":
        print(ev(f"const w = win({json.dumps(a[1])}); if (!w) ''; else {{ const r = w.get_frame_rect(); `${{r.x}} ${{r.y}} ${{r.width}} ${{r.height}} ${{w.is_above()}} ${{w.is_on_all_workspaces()}}` }}"))
    elif cmd == "focus":
        print(ev(f"const w = win({json.dumps(a[1])}); if (w) w.activate(global.get_current_time()); !!w"))
    elif cmd == "move":
        move(float(a[1]), float(a[2]))
    elif cmd == "click":
        move(float(a[1]), float(a[2])); time.sleep(0.12)
        button(True); time.sleep(0.06); button(False)
    elif cmd == "drag":
        x0, y0, x1, y1 = map(float, a[1:5])
        move(x0, y0); time.sleep(0.15); button(True); time.sleep(0.1)
        for i in range(1, 11):
            move(x0 + (x1 - x0) * i / 10, y0 + (y1 - y0) * i / 10); time.sleep(0.03)
        time.sleep(0.1); button(False)
    elif cmd == "key":
        ev(f"K.notify_keyval(Clutter.CURRENT_TIME, Clutter.KEY_{a[1]}, Clutter.KeyState.PRESSED);"
           f"K.notify_keyval(Clutter.CURRENT_TIME, Clutter.KEY_{a[1]}, Clutter.KeyState.RELEASED); 1")
    elif cmd == "panel":
        print(ev("global._lgMain.panel.height"))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])

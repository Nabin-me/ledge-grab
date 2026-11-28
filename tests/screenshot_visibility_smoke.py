"""Interactive Windows smoke: python tests/screenshot_visibility_smoke.py path/to/ledge-grab.exe.

Uses an isolated settings/index directory. Tests through the production notch
message handler while leaving your own saved screenshot history untouched.
"""
import sqlite3
import sys
import tempfile
import time
from pathlib import Path

from drive import App, MOVE, RUP

WM_NOTCH_KEY = 0x8000 + 7
CONTROL, SHIFT = 2, 4


def rows(root):
    with sqlite3.connect(root / "index.db") as db:
        return db.execute("SELECT id, path, hidden FROM items WHERE kind=0 ORDER BY id").fetchall()


def wait_for(fn, description, seconds=5.0):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = fn()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError(description)


def key(app, char, mods=0):
    app.post(WM_NOTCH_KEY, ord(char), wp=mods)
    time.sleep(0.65)


def menu(app):
    app.post(RUP, 105, 4)
    wait_for(lambda: app.size()[1] >= 250, "capture menu did not open")


def capture(app, root, count):
    app.mode("Screen")
    def saved():
        items = rows(root)
        return items if len(items) == count and all(Path(p).is_file() for _, p, _ in items) else None
    return wait_for(saved, "capture did not reach the index and disk")


def smoke(exe, root):
    app = App(str(exe), str(root))
    try:
        capture(app, root, 1)
        wait_for(lambda: app.size()[1] <= 10, "first thumbnail did not settle")
        app.peek()
        app.post(MOVE, 70, 45)
        time.sleep(0.12)
        key(app, "H")
        assert [i[2] for i in rows(root)] == [1], "hovered screenshot was not hidden"
        key(app, "H", CONTROL)
        app.post(MOVE, 70, 45)
        time.sleep(0.12)
        key(app, "H")
        assert [i[2] for i in rows(root)] == [0], "hidden screenshot did not restore"
        key(app, "H", CONTROL | SHIFT)
        # Verify save-only produces no large pop-up frame.
        items = capture(app, root, 2)
        assert [i[2] for i in items] == [0, 1], "save-only did not hide only the new screenshot"
        assert app.size()[1] <= 10, "save-only opened a screenshot preview"
        menu(app)
        key(app, "H", CONTROL | SHIFT)
        items = capture(app, root, 3)
        assert [i[2] for i in items] == [0, 1, 0], "disabling save-only did not restore future previews"
        assert all(Path(p).is_file() for _, p, _ in items), "a screenshot was removed from disk"
        wait_for(lambda: app.size()[1] <= 10, "latest thumbnail did not settle")
        app.peek()
        key(app, "3", CONTROL)
        assert app.size()[0] > 400, "could not open the app-folder shelf"
        # Opening a folder switches the bottom bar from filters to a back control;
        # the first app folder has two visible screenshots, one row either way.
        app.click(72, 42, wait=0.6)
        assert app.alive(), "the notch exited when opening a folder"
    finally:
        app.quit()


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="ledge-hidden-ui-") as tmp:
        smoke(Path(sys.argv[1]).resolve(), Path(tmp) / "instance")
    print("Screenshot hide, restore, save-only and folder-click smoke passed.")

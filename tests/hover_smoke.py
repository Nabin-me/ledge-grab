"""Windows smoke test: python tests/hover_smoke.py path/to/ledge-grab.exe."""
from pathlib import Path
import sys
import tempfile
import time

from drive import App, MOVE, RUP

LEAVE = 0x02A3


def wait_for(check, message, timeout=1.8):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if check():
            return
        time.sleep(0.02)
    raise AssertionError(message)


def smoke(app):
    rest = app.size()
    assert rest[1] <= 10, f"unexpected collapsed size: {rest}"
    collapsed = lambda: app.size()[1] <= 10
    expanded = lambda: app.size()[1] > 45

    app.post(MOVE, 8, 4)
    time.sleep(0.31)
    assert collapsed(), "grazing the outer edge armed hover"

    app.post(MOVE, 105, 4)
    time.sleep(0.06)
    app.post(LEAVE)
    time.sleep(0.27)
    assert collapsed(), "a quick crossing opened the notch"

    # Once genuinely hovered, it should open, and small moves in the zone
    # should not keep restarting the deadline.
    wait_until = time.monotonic() + 0.31
    while time.monotonic() < wait_until and not expanded():
        app.post(MOVE, 103, 4)
        time.sleep(0.025)
    assert expanded(), "normal hover did not open while the pointer moved in the zone"

    app.post(LEAVE)
    time.sleep(0.26)
    assert expanded(), "the shelf closed before its grace period"
    app.post(MOVE, app.size()[0] // 2, 32)
    time.sleep(0.42)
    assert expanded(), "entering the shelf didn't cancel the previous close"
    app.post(LEAVE)
    wait_for(collapsed, "the shelf stayed open after the next deliberate exit")

    app.post(RUP, 105, 4)
    wait_for(expanded, "the capture menu did not open")
    app.post(LEAVE)
    time.sleep(0.57)
    assert expanded(), "the explicit menu closed before its longer grace period"
    wait_for(collapsed, "the capture menu stayed open after the longer grace")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="ledge-hover-") as tmp:
        app = App(str(Path(sys.argv[1]).resolve()), tmp)
        try:
            smoke(app)
        finally:
            app.quit()
    print("Hover, re-entry, and close grace passed.")

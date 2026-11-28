"""Log Win32 animation frame submissions; run on an interactive Windows desktop.

Usage: py tests/frame_pacing_smoke.py PATH_TO_EXE [--refresh-hz 60]
Set Windows to a fixed refresh rate for a baseline comparison. This measures
app-side submission cadence, not DWM's actual displayed frames.
"""
import argparse
import csv
import math
import os
from pathlib import Path
import statistics
import tempfile
import time

from drive import App, MOVE, RUP


def started(log):
    if not log.exists():
        return []
    with log.open(newline="") as f:
        return [
            (int(start) / 1000, int(render) / 1000)
            for start, render, notch, overlay in csv.reader(f)
            if notch == "1" and overlay == "0"
        ]


def p95(values):
    return sorted(values)[math.ceil(len(values) * 0.95) - 1]


def run(exe, refresh_hz):
    target_ms = 1000 / refresh_hz
    with tempfile.TemporaryDirectory(prefix="ledge-pacing-") as tmp:
        root = Path(tmp)
        log = root / "frames.csv"
        os.environ["LEDGE_GRAB_FRAME_LOG"] = str(log)
        app = App(str(exe), str(root / "isolated-app"))
        try:
            base = len(started(log))  # don't mix app launch with the trial
            samples, render_ms = [], []
            for _ in range(3):
                app.post(RUP, 105, 4)
                # Keep the animation alive and exercise the Win32 message pump.
                until = time.monotonic() + 0.40
                while time.monotonic() < until:
                    app.post(MOVE, 100, 5)
                    time.sleep(0.002)
                time.sleep(0.35)  # settle and flush the trace before asking it to close
                assert app.size()[1] > 45, "capture menu did not open"
                frames = started(log)[base:]
                assert len(frames) >= 6, f"too few samples from one opening: {len(frames)}"
                samples.extend(b[0] - a[0] for a, b in zip(frames, frames[1:]))
                render_ms.extend(ms for _, ms in frames)
                app.post(0x02A3)  # WM_MOUSELEAVE
                time.sleep(1.45)  # 800 ms grace plus 400 ms closing motion
                assert app.size()[1] <= 10, "capture menu did not close"
                base = len(started(log))
            median = statistics.median(samples)
            tail = p95(samples)
            print(f"{len(samples)} gaps, target {target_ms:.2f} ms; median {median:.2f} ms, "
                  f"p95 {tail:.2f} ms, worst {max(samples):.2f} ms; "
                  f"submit work p95 {p95(render_ms):.2f} ms")
            print(f"Late intervals (>1.5 refreshes): {sum(d > target_ms * 1.5 for d in samples)}")
        finally:
            app.quit()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("--refresh-hz", type=float, default=60)
    args = parser.parse_args()
    if not args.exe.is_file() or args.refresh_hz <= 0:
        parser.error("provide a Windows EXE and a positive refresh rate")
    run(args.exe.resolve(), args.refresh_hz)

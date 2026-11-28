#!/usr/bin/env python3
"""Save a screenshot of the headless GNOME test session: grab.py OUT.png

Uses org.gnome.Shell.Screenshot, which the lg-test-helper extension unlocks by
turning on unsafe mode in test sessions. Works for both Wayland and X11 sessions.
"""
import os, subprocess, sys

out = os.path.abspath(sys.argv[1])
r = subprocess.run(["gdbus", "call", "--session", "--dest", "org.gnome.Shell.Screenshot",
                    "--object-path", "/org/gnome/Shell/Screenshot",
                    "--method", "org.gnome.Shell.Screenshot.Screenshot", "false", "false", out],
                   capture_output=True, text=True)
if r.returncode != 0 or "true" not in r.stdout or not os.path.exists(out):
    print("screenshot failed:", r.stdout.strip(), r.stderr.strip(), file=sys.stderr)
    sys.exit(1)

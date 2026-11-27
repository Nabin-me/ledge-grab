# Ledge Grab / Linux preview

A separate GTK 3/Python desktop implementation of the core Ledge Grab workflow. Based on the Windows app behavior in main at `5d59eb37046c908514a852b7f7f61a8633c88e67`. Linux files do not change the Windows Rust crate, build, release, or website.

## Run on a desktop
- Debian/Ubuntu, GTK 3 available: download the preview .deb and run `sudo apt install ./ledge-grab-linux-preview_0.1.0~preview2_all.deb`. Then open **Ledge Grab Preview** from the app menu, or run `ledge-grab-linux`.
- From source: on Debian/Ubuntu install `python3-gi gir1.2-gtk-3.0`, then run `/usr/bin/python3 linux/ledge_grab.py`. On other distributions use their system Python and matching PyGObject/GTK 3 packages; no other packaging formats are built yet.
- Saved PNGs go in **Pictures/Ledge Grab** (or the configured XDG Pictures directory), with a file named .hidden-previews.json for shelf visibility. Use **Hide** to keep the file but leave it out of Recent. Nothing is uploaded.
- For a custom desktop keyboard shortcut, bind `ledge-grab-linux --capture area`. With X11, `ledge-grab-linux --capture screen` is also available. Do not override a system screenshot binding unless you want to.

## Session support in this preview
- X11: GTK root capture, full screen or drag-select a region, display-scaled crop coordinates, top-centered always-on-top shelf where the window manager honors placement, local PNG and image clipboard. Esc or right-click cancels selection. Test multi-monitor and mixed-scale configurations before promising them.
- Wayland: the system screenshot chooser via XDG Desktop Portal; exact area/window/screen choices depend on the desktop and installed portal backend. Requires a working session bus and a compatible Screenshot portal. The shelf is an ordinary window rather than an always-on-top notch. No hidden capture or attempts to bypass compositor restrictions.
- Not ported: Windows scrolling/recording, markup, OCR/redaction, app-specific archives, advanced rendering, custom global-shortcut registration, animated notch parity, or auto-updates. Use the inspectable screenshot and clipboard workflow first.

## Verify locally
In a desktop session capture an area of a window with recognizable pixels, confirm the PNG size and location, paste in a second image-capable app, hide the preview, restart and confirm the PNG is still there and the preview stays hidden. On X11 try full-screen and Escape cancellation, then run the shortcut command while the shelf is already open; only one shelf should exist. On Wayland try screenshot approval and cancellation, and confirm a missing portal shows an error without creating a file. Manually check positioning on each desktop; this package is an engineering preview, not a supported desktop matrix.

Automated source checks: `/usr/bin/python3 -m unittest discover -s linux -p 'test_*.py'`. Build a new local artifact: `sh packaging/linux/build-deb.sh`. See QA.md for the virtual X11, Ubuntu userspace, and portal fixture results. A native Wayland session and physical mixed-DPI displays still need verification.

## Signing
This local preview is unsigned. No Apple notarization or Microsoft certificate is needed for this standalone Linux app. Signing distribution repos and update metadata is a later packaging choice (APT repositories verify signed Release metadata). An AppImage signature is optional. SHA-256 files for this handoff are integrity checks, not a publisher signature.

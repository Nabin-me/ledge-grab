# Linux (GNOME) port plan

Target: GNOME on Wayland and on X11. Where GNOME can't do what Windows does, the
Linux build uses the route GNOME sanctions, and the feature table below says what
differs. Other desktops are out of scope for now.

Test environment: WSL2 Ubuntu 24.04 (GNOME 46, the last Ubuntu LTS GNOME with an X11
session) on the GPU box, running gnome-shell headless with a virtual monitor for
Wayland, and Xvfb + mutter for X11. Tests drive the session through mutter's own
RemoteDesktop/ScreenCast D-Bus API, the same API the portals use.

## Code layout

The platform-neutral modules stay shared: `store`, `paste_queue`, `scroll_pdf_core`,
`pdf`, `anim`, `pointer_timing`, `secrets`, `redact`, `pretty`, `keys` (logic only).
Windows code moves behind `cfg(windows)`; Linux gets its own platform layer:

| Windows module | Linux replacement |
|---|---|
| `gfx` (Direct2D, DirectWrite, DirectComposition) | Same `Canvas` API on tiny-skia (software) with bundled Inter font and drawn icons; frames shown in a GTK4 window |
| window/message loop in `main` | GTK4 main loop; timers via glib |
| `capture` (Windows Graphics Capture, GDI) | X11: XShm/XGetImage. Wayland: Screenshot portal (full screen, then crop) |
| `overlay` (region picker) | Fullscreen GTK window over a frozen screenshot |
| `clip` (delayed rendering, CF_DIBV5/PNG) | GdkContentProvider serving `image/png`; each read request is a paste |
| global hotkeys (RegisterHotKey) | X11: XGrabKey. GNOME Wayland 46: custom keybinding in GNOME settings that runs `ledge-grab --region` etc.; the running instance receives it over D-Bus |
| paste hook (WH_KEYBOARD_LL) | Not needed: the clipboard source sees every read, in order |
| `rec` (Media Foundation) | ScreenCast portal + PipeWire + GStreamer x264 (Wayland); ximagesrc (X11) |
| `scroll_pdf` (SetCursorPos, SendInput wheel) | X11: XTest. Wayland: RemoteDesktop portal pointer motion + axis |
| `ocr` (Windows.Media.Ocr) | tesseract if installed, else naming falls back to app name |
| tray, launch at login | AppIndicator if present; `~/.config/autostart/ledge-grab.desktop` |

## Feature table (filled in as features land and pass tests)

| Feature | GNOME X11 | GNOME Wayland | Notes |
|---|---|---|---|
| Ledge at top centre, always on top | | | Wayland: no client positioning or keep-above; needs the bundled GNOME Shell extension |
| Shelf, select, copy, delete, pin, hide | | | |
| Region / screen capture | | | Wayland: first use asks for screenshot permission |
| Window capture | | | Wayland: no foreground-window query; uses the portal's interactive window pick |
| Global shortcuts | | | Wayland: set as GNOME custom shortcuts |
| Sequential Copy (A, B, C on rapid paste) | | | One clipboard source serves the next image per read |
| Copied-image capture from other apps | | | Wayland: only while the ledge has focus |
| Merge PDF, Stack, Background, GIF | | | Pure code |
| Float above windows | | | Wayland: needs the extension |
| Scroll to PDF | | | Wayland: RemoteDesktop permission prompt each session |
| Clip recording (MP4) | | | |

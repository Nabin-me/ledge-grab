# Linux (GNOME) port plan

Target: GNOME on Wayland and on X11. Where GNOME can't do what Windows does, the
Linux build uses the route GNOME sanctions, and the feature table below says what
differs. Other desktops are out of scope for now.

Test environment: WSL2 Ubuntu 24.04 (GNOME 46, the last Ubuntu LTS GNOME with an X11
session) on the GPU box, running gnome-shell headless with a virtual monitor for
Wayland, and Xvfb + gnome-shell for X11. `tests/linux/e2e.sh` drives the app with
mutter's virtual pointer and keyboard (through `org.gnome.Shell.Eval`, unlocked by a
test-only extension), pastes with a separate GTK app, and checks files, clipboard
contents and image sizes. Run it with:

    tests/linux/session.sh wayland tests/linux/e2e.sh target/debug/ledge-grab ~/e2e-wayland
    tests/linux/session.sh x11     tests/linux/e2e.sh target/debug/ledge-grab ~/e2e-x11

## GNOME Shell extension

`gnome/ledge-grab@ledgegrab` does what GNOME won't let an app do for itself:

- keeps the ledge above all windows, on every workspace, centred under the top bar,
  and puts it back if anything moves it;
- gives the region picker keyboard focus, so Esc works;
- takes screenshots for the app on Wayland (no portal prompt), and puts a saved
  screenshot on the clipboard. On Wayland only the focused app may take the
  clipboard, and after a shortcut capture that is someone else.

Its D-Bus methods (`org.ledgegrab.Shell` at `/org/ledgegrab/Shell` on the shell's
bus name) answer only a caller whose executable is `ledge-grab`. Without the
extension the app still runs: captures fall back to the Screenshot portal, and the
clipboard is taken directly, which on Wayland works only while the ledge has focus.

## What GNOME does differently from Windows

- **Clipboard manager reads.** GNOME copies new clipboard content (a few MB at most)
  the moment an app takes the clipboard, so it outlives the app. That read is not
  a paste: the app ignores a read in the first 400 ms after taking the clipboard.
  Mouse Without Borders does the same thing on Windows.
- **Clipboard ownership and input serials.** Mutter ignores a new clipboard owner
  whose last input event is older than the current owner's. Real copies (Ctrl+C,
  a click) always carry a fresh one; test apps must click before they copy.
- **Every read is a paste.** There is no global paste hook on Wayland and none is
  needed: the clipboard source sees each read in order, which advances Sequential
  Copy directly. An app that reads the image twice for one paste would skip an
  image. GTK's paste path reads once; Chromium and Electron paste paths are not tested on Linux yet.

## Code layout

The platform-neutral modules stay shared: `store`, `paste_queue`, `scroll_pdf_core`,
`pdf`, `anim`, `pointer_timing`, `secrets`, `redact`, `pretty`, `keys` (logic only).
Windows code moves behind `cfg(windows)`; Linux gets its own platform layer:

| Windows module | Linux replacement |
|---|---|
| `gfx` (Direct2D, DirectWrite, DirectComposition) | Same `Canvas` API on tiny-skia (software) with bundled Inter font and drawn icons; frames shown in a GTK4 window |
| window/message loop in `main` | GTK4 main loop; timers via glib |
| `capture` (Windows Graphics Capture, GDI) | X11: GetImage on the root window. Wayland: the Shell extension's screenshot, else the Screenshot portal (full screen, then crop) |
| `overlay` (region picker) | Fullscreen GTK window over a frozen screenshot |
| `clip` (delayed rendering, CF_DIBV5/PNG) | GdkContentProvider serving `image/png`; each read request is a paste |
| global hotkeys (RegisterHotKey) | GNOME custom keybinding (both sessions) that runs `ledge-grab --region` / `--screen`; the running instance receives it over D-Bus (GApplication command line) |
| paste hook (WH_KEYBOARD_LL) | Not needed: the clipboard source sees every read, in order |
| `rec` (Media Foundation) | ScreenCast portal + PipeWire + GStreamer x264 (Wayland); ximagesrc (X11) |
| `scroll_pdf` (SetCursorPos, SendInput wheel) | X11: XTest. Wayland: RemoteDesktop portal pointer motion + axis |
| `ocr` (Windows.Media.Ocr) | tesseract if installed, else naming falls back to app name |
| tray, launch at login | AppIndicator if present; `~/.config/autostart/ledge-grab.desktop` |

## Feature table

Phase 1 status. "tested" means covered by `tests/linux/e2e.sh` passing in a headless GNOME 46 session.

| Feature | GNOME X11 | GNOME Wayland | Notes |
|---|---|---|---|
| Ledge at top centre, always on top | ✅ tested | ✅ tested | Hangs under the top bar. Both sessions use the Shell extension |
| Rest / shelf on hover | ✅ tested | ✅ tested | Recency dots are drawn but not checked by the test |
| Shelf: select, copy, delete | ✅ tested | ✅ tested | Pin, hide, rename: later phase |
| Region / screen capture | ✅ tested | ✅ tested | Region: drag on a frozen frame, Esc cancels. Without the extension, Wayland asks for screenshot permission once |
| Capture copies to clipboard | ✅ tested | ✅ tested | Wayland: through the Shell extension |
| Window capture | | | Wayland: no foreground-window query; uses the portal's interactive window pick |
| Global shortcuts | partial | partial | `--region` / `--screen` reach the running app (tested); installing them as GNOME custom shortcuts: later phase |
| Sequential Copy (A, B, C on rapid paste) | ✅ tested | ✅ tested | Pastes come out in order; the last image stays after the queue; the queue ends when another app copies |
| Copied-image capture from other apps | | | Wayland: only while the ledge has focus |
| Merge PDF, Stack, Background, GIF | | | Pure code |
| Float above windows | | | Wayland: needs the extension |
| Scroll to PDF | | | Wayland: RemoteDesktop permission prompt each session |
| Clip recording (MP4) | | | |

# Linux preview 2: compatibility checks
Tested on amd64. Xorg used an isolated, authenticated virtual desktop; the program was driven through its packaged command, received actual X11 pointer/key input, and an independent GTK process requested the image clipboard. PNGs and clipboard pixels had to match and contain known test pixels. Test results below are not claims about a physical GPU, every window manager, or native Wayland.

| Environment | Result |
|---|---|
| Debian 13 / Xorg + Xfwm4 | **Passed:** full-screen PNG + clipboard, crossing an output seam with region capture, Escape cancellation, launcher forwarding into the existing process, and Recent/Hide without deleting files. |
| Debian 13 / Xorg + Openbox | **Passed:** same checks. Both window managers used two virtual outputs: 1280×720 at (0,0) plus 1024×768 at (1280,144), combined 2304×912. Cross-output crop was 190×160 with matching clipboard pixels. |
| Debian 13 / Xorg with GDK_SCALE=2 | **Passed at GDK_SCALE=2:** full screen 1280×720 and area 330×200, clipboard and shelf checks. This was a uniform test, not two different physical display scale factors. |
| Ubuntu 24.04.5 LTS userspace against virtual Xorg | **Passed app execution from the unpacked .deb payload:** full screen 1280×720 and area 330×200; both PNGs matched the clipboard. APT resolved and unpacked dependencies from official Ubuntu repositories. Full `apt install` configuration did not pass in the unprivileged test root: distro dependency scripts need system account/group setup and permissions the container doesn't supply. No claim of a fully configured Ubuntu Desktop install. |
| Portal client over a test D-Bus session | **Passed:** success imported the fixture PNG and copied it; cancellation, rejection, and missing service restored the shelf and wrote no PNG. This used a test responder, not a real compositor's permission dialog. |
| Native Wayland on GNOME / KDE Plasma / wlroots | **Unverified.** No native graphical Wayland session was available. The preview asks the desktop screenshot portal, uses a normal window, and leaves available screenshot choices to the compositor and its backend. |
| Physical mixed-DPI displays, Debian/Ubuntu desktop install, Fedora, Arch, ARM | **Not certified.** The package targets Debian-family systems. The included Python/GI/GTK source is portable in principle, not a guarantee. |

## Bugs found by running the UI
- Region selection originally sent the pointer grab to the popup rather than the input surface. The release now targets the input surface; the same real drag reproducer passes.
- The second launcher invocation could stall while forwarding command-line streams. It now sends a small application action to the running shelf. Capturing again through the CLI was tested on both window managers and at 2×.
- Recent now orders same-second captures by write time rather than the random filename suffix. A regression test covers this.

## Next acceptance checks
1. Install the .deb on a fully booted Ubuntu or Debian desktop using normal package configuration.
2. On GNOME Wayland and KDE Plasma Wayland, try permission approval, rejection, and cancellation; check paste into a second desktop app. Verify the screenshot chooser and allowed capture types on each.
3. Test physical displays with different scaling, vertical offset, changing primary output, and a region straddling both screens. Keep the unverified limitations on the download page until this is done.

QA scripts and short results are bundled. `qa/virtual-x11.sh` needs Xorg, dummy driver, desktop/X11 utilities, a session D-Bus, and optional Xfwm4/Openbox. Do not expose an Xorg TCP listener on a shared machine; use an isolated test environment. Linux work remains separate from the Windows source and site.

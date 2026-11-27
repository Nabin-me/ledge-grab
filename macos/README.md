# Ledge Grab for macOS · Developer Preview

The desktop app lives in the menu bar. Capture a region or the display under your pointer, then paste the PNG or grab it again from the recent-capture shelf. It uses Apple’s ScreenCaptureKit and stores captures locally in `~/Pictures/Ledge Grab`, by date. No server, background recording, clipboard watching, or analytics.

## Build and try it

On macOS 14 Sonoma or newer with Xcode/command-line tools installed:

```sh
swift test --package-path macos
bash packaging/macos/build-preview.sh
```

Unzip `macos/dist/Ledge-Grab-macOS-Preview.zip` and open **Ledge Grab.app**. Set `LEDGE_UNIVERSAL=1` when packaging with both the Apple Silicon and Intel toolchains available. The workflow creates a universal build as a CI artifact only, not a GitHub Release.

The preview is ad-hoc signed, not Developer ID signed or notarized. Building on the test Mac is the recommended way to try it. Do not disable Gatekeeper. A public download should be Developer ID signed and notarized.

1. Use the menu-bar icon, or press **Option + Shift + 4** for a region if the shortcut could register. If another app uses it, start from the menu.
2. On first use, grant **Screen & System Audio Recording** permission in System Settings, then quit and reopen Ledge Grab. The app captures no audio.
3. Drag over the frozen display, or press Esc to cancel. **Display** saves the screen under the pointer without selecting a region.
4. Paste with **Command + V**. Click a shelf tile to copy it again, or drag a tile into a compatible drop target.

## Before publishing

Use a real logged-in Mac session. CI tests crop coordinates and local storage and verifies the bundle, not screen permission or the interactive UI.

- Grant, deny, and revoke permission; relaunch; confirm the app makes no capture when denied.
- Select a visibly asymmetric crop at 1× and 2×. Compare the saved pixel bounds with the drag and paste into another app.
- Test menu and shortcut on separate displays of differing scale and orientation, in a full-screen Space, and on a notched MacBook.
- Cancel via Escape and right click; confirm no new disk entry or clipboard change. Confirm history persists after relaunch.
- Confirm drag-out in Finder and a chat app, initial focus, and that the shelf hides without blocking normal paste.
- Verify native launch on Apple Silicon and Intel. Obtain Developer ID signing/notarization before promoting a stable download.

This is a focused port, not Windows parity. Markup, OCR/redaction, recording, scrolling, pinning and PDF export are not in this preview. A menu-bar-anchored shelf works on Macs with and without a physical notch.

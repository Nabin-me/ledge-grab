# Scroll to PDF Windows prototype

This is a local prototype based on the released Windows v0.1.2 source. It is not a replacement for the Windows installer.

## Run the test

1. Extract the preview ZIP. Quit the installed Ledge Grab first so it doesn't compete for the single app instance.
2. Run **Start Preview.cmd** from the extracted folder. A **Preview is ready** dialog confirms startup; dismiss it and look for the thin ledge at the top center of your primary screen, not a tray icon. The launcher uses separate `preview-data`, does not register global capture hotkeys, and does not monitor other applications' clipboard copies. Quit the preview and start your normal copy again when done.
3. Open **scroll_pdf_fixture.html** in a Windows browser window and select **Start at message 06**. Use the ledge **+ → Scroll to PDF**. Drag a rectangle over just the message pane, below the ledge, excluding the scrollbar, border, any fixed header and the composer. Keep the pane visible, foreground, and stationary as it scrolls.
4. Choose **Stop & Save** or press **Esc** if you want to finish early. Moving the pointer from the parked center, changing focus, an uncertain seam, or reaching the cap stops the run rather than guessing. A run that has captured something but stopped early saves a PDF with `- partial.pdf` in the name. The completion chip opens the PDF, or use the shelf or `preview-data/shots`.
5. Check the finished PDF starts at message 06, preserves the order, has no duplicate messages, and reaches **END OF TEST THREAD**. Repeat from a different starting point and with early Stop & Save. When finished, close the preview before reopening the installed app.

## What this pilot supports

One normal Windows window, one stable pane with overlapping downward scroll steps, image-only US Letter pages, and local export. It looks for a clear pixel match and cuts pages near quiet rows when possible; it does not understand message boundaries. It stops after repeated lack of visible movement, not after reading the app's scrollbar API. It does not capture history above the starting position. Infinite loading, animation, multiple panes and apps that cause large scroll jumps may produce a partial result. Limits: 60 seconds of scrolling, 180 steps, and bounded screenshot dimensions. Select only content you intend to save.

## Windows desktop results

Checked on a native Windows 11 desktop (build 26300) with `tests/scroll_pdf_e2e.ps1`, which drives Edge and the preview build with real mouse and wheel input:

- Startup with no other Ledge Grab: **Preview is ready**, ledge stays up.
- Startup while another instance runs: the **already running** message, exit code 2, and `preview-launch.log` records the stop.
- `scroll_pdf_fixture.html` from message 06: one complete PDF, messages 06 to 26 in order, no duplicates or gaps, ending at **END OF TEST THREAD**.
- Esc mid-run: partial PDF, reason `UserStopped`.
- A window placed over the pane mid-run: partial PDF, reason `PaneCovered`.
- Moving the browser window mid-run: partial PDF, reason `TargetChanged`.
- `scroll_pdf_repeat.html` (identical rows): partial PDF, reason `NoSafeOverlap`.

Not yet checked on Windows: a real chat app pane, Stop & Save from the ledge button (Esc and the stop flag share one path), high-DPI or multi-monitor setups, and the packaged `Start Preview.cmd` launcher.

The launch log ends each run with one line such as `Scroll PDF ended: wheel=16 reason=NoMoreMovement rows=2131`. `wheel=0` means the run stopped before the first scroll.

## If it does not open

If regular Ledge Grab or an earlier preview owns the single-instance lock, the new preview shows a clear **already running** message. Quit the existing app and retry once. A graphics or data-folder failure shows its stage and error, and the launcher keeps an unsuccessful exit visible. If needed, share the exact message or the `preview-launch.log` created beside the launcher. If the EXE is missing, extract the entire ZIP first; don't run the CMD from the ZIP viewer. The preview is unsigned. If Windows blocks it, use the source patch and build locally rather than changing security settings.

If the preview opens but the capture fails, the ledge keeps a red error card with the reason and a next step. Click it to dismiss. The launch log also records the capture reason and dimensions, not the pixels or chat contents.

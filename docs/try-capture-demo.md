# Ledge Grab playground (hero demo)

The hero's main hands-on CTA. It replaces the old auto-play hero stage and the
duplicate "Try a capture" button. Visitors work a mock Windows desktop the way the
app works:

| Step | What the visitor does | App feature it shows |
| --- | --- | --- |
| Freeze | **Try a capture** or `Ctrl`+`Shift`+`S` | Frozen screen + capture toolbar |
| Pick | Drag over anything, click a window, or press Screen | Region / Window / Screen modes |
| Redact | `B` outlines emails, the card and the API key; `B` again blurs them | Auto secret detection + blur |
| Mark up | `A` arrow, `X` box, right-click or `Ctrl`+`Z` undo | Markup tools |
| Copy | `Enter` | Copied to clipboard, shot tucks under the notch |
| Ledge | Hover or click the notch; click a shot to copy it | The ledge |
| Share | Drag a shot into the chat, or press Enter on it | Drag a file out into Slack/Discord |
| Paste | Click the message box, `Ctrl`+`V` (or the Paste button) | Everything is on the clipboard |
| Float | Float on a shot; drag, scroll to resize, `Ctrl`+scroll to fade, right-click closes | Float above windows |
| Merge | Tick two shots, press Merge | Merge into a PDF |

## Keep captures inside the playground

The demo's own notch shows captures and copies them to the playground clipboard.
The page navigation has no separate Recent Shots notch or capture overlay.

A "Things to try" counter (0 of 7) tracks what the visitor has discovered and the
status line always names the next thing to try. The chat replies react to what was
sent (blurred, marked up, a PDF, or customer emails left readable).

## Files

| File | Size (raw) | Loaded when |
| --- | --- | --- |
| `site/demo/tryit.js` | ~1.7 KB | With the page (deferred module). Sets up an IntersectionObserver. |
| `site/demo/tryit-app.js` | ~38 KB | At idle time once the hero is on screen. |
| `site/demo/tryit.css` | ~25 KB | Same moment. |

No dependencies. All classes start with `ti-`. Gzipped, the two lazy files are
roughly 10 KB together.

## How it is wired into index.html

All tagged `try-capture demo`:

1. A `<style>` in `<head>` reserves the playground box (`#tryit`: 1180 by 660 on
   desktop, 940 px tall under 860 px), so the hero never shifts when it loads.
2. In the hero: `<div id="tryit">` followed by `<template id="tryitFallback">`,
   which holds the original hero stage.
3. `<script type="module" src="demo/tryit.js">` before `</body>`.
4. The playground owns its own capture interactions; main.js has no capture listener for it.

## Turn it off or remove it

- **One visit:** `?tryit=0` puts the original hero stage back from the template and
  fetches nothing else. If the playground fails to load, the same fallback runs.
- **For good:** replace the `#tryit` div and the template with the template's
  contents, delete the reserve `<style>` and the script tag, delete `site/demo/`
  and this note. (Optionally restore the hero's "Try a capture" button and keys.)

## How it stays cheap

- Before the visitor sees it, only the 1.7 KB loader is fetched.
- Moving pieces animate `transform` and `opacity` with the Web Animations API.
- No idle `requestAnimationFrame` loop. rAF runs only while a selection, mark,
  tile or float is being dragged, batched to one update per frame.
- The stage has `contain: layout paint`; the chat has `contain: size layout paint`,
  so new messages never grow the page.
- The button's animated border pauses when the playground is off screen.
- Each shot stores one frozen copy of the desktop DOM; tiles, chat images, floats
  and the drag ghost are scaled clones of it. No canvas and no image encoding.
- `prefers-reduced-motion` collapses the timings.

## Verify it does not slow the page

Use Chrome, not the in-app preview (it throttles frames when unfocused).

1. `python -m http.server 5178 --directory site`
2. **Network:** reload at the top. You should see `demo/tryit.js` first, then
   `tryit.css` and `tryit-app.js` after first paint.
3. **Lighthouse A/B** (incognito, Mobile, Performance, 3 runs each, compare
   medians): `http://localhost:5178/` against `http://localhost:5178/?tryit=0`.
   LCP (the hero title), TBT and CLS should be within noise.
4. **CLS:** in the Console, before reloading:
   ```js
   new PerformanceObserver(l => l.getEntries().forEach(e => console.log('shift', e.value.toFixed(4), e.sources.map(s => s.node))))
     .observe({ type: 'layout-shift', buffered: true });
   ```
   No entry should point at `#tryit` or a `ti-` node.
5. **Frames:** Performance panel, 4x CPU slowdown, record a full session
   (capture, blur, arrow, Enter, drag to chat, float, merge). No long tasks over
   50 ms; layout and paint stay inside the playground.
6. **Idle:** record 5 s with no input after a session. The only work should be the
   button border (while on screen) and the typing dots (while a reply is pending).

## Measured on this branch (in-app preview, 1440 by 900)

- Load: the playground mounts above the fold with CLS 0 and no long tasks.
  `tryit.css` and `tryit-app.js` start about 450 ms after navigation.
- Checked working: region drag, window click (box matches the window exactly),
  B/B blur, arrow, Enter, notch peek, drag to chat (blur and arrow survive into
  the chat image), Ctrl+V paste, Float, Merge into PDF, `?tryit=0` fallback,
  mobile layout (375 px, no horizontal overflow).
- Frame timing: not measurable in the preview (it ran at 2 to 3 fps). Step 5 above
  still needs a run in Chrome.

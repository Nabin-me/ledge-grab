#!/usr/bin/env bash
# End-to-end test of the GNOME app inside a headless session.
#   session.sh wayland|x11 tests/linux/e2e.sh BIN OUTDIR
# Writes PASS/FAIL lines to OUTDIR/results.txt and screenshots to OUTDIR/*.png.
# Logs hold only app events and image sizes, never image or clipboard content.
set -u
bin="$1"; out="$2"; here="$(cd "$(dirname "$0")" && pwd)"
rm -rf "$out"; mkdir -p "$out/data" "$out/config" "$out/pics"
ctl() { python3 "$here/lgctl.py" "$@"; }
shot() { python3 "$here/grab.py" "$out/$1.png"; }
t0=$(date +%s.%N)
now() { printf "%6.2f" "$(echo "$(date +%s.%N) - $t0" | bc)"; }
res() { echo "$1 $2" | tee -a "$out/results.txt"; echo "$(now) $1 $2" >> "$out/timeline.log"; }
check() { if eval "$2"; then res PASS "$1"; else res FAIL "$1"; fi; }
wait_for() { # wait_for FILE PATTERN [tenths]
  for _ in $(seq 1 "${3:-80}"); do grep -q -- "$2" "$1" 2>/dev/null && return 0; sleep 0.1; done; return 1; }
spot() { awk -v b="$1" '$1==b {printf "%d %d\n", $2+$4/2, $3+$5/2}' "$out/spots"; }
ledge() { read -r lx ly lw lh _ < <(ctl rect "Ledge Grab"); }
click_spot() { # click a ledge button by name, in screen coordinates
  ledge; local sx sy; read -r sx sy < <(spot "$1")
  [ -n "${sx:-}" ] || { echo "no spot $1"; return 1; }
  ctl click $(( lx + sx )) $(( ly + sy )); sleep 0.3; }
hover() { ledge; ctl move $(( lx + lw / 2 )) $(( ly + 4 )); wait_for "$out/spots" "^view Shelf" 30; sleep 0.3; }
tiles() { awk '$1 ~ /^Tile/ {print $1}' "$out/spots"; }
paste() { # paste N: focus the target, ask it to paste, wait for result line N
  ctl focus "Paste Target" >/dev/null; sleep 0.3; touch "$out/req"
  wait_for "$out/pastes.log" "^$1 " 60; }
pasted() { awk -v n="$1" '$1==n {print $2}' "$out/pastes.log"; }

export XDG_DATA_HOME="$out/data" XDG_CONFIG_HOME="$out/config"
echo "XDG_PICTURES_DIR=\"$out/pics\"" > "$out/config/user-dirs.dirs"
export LEDGE_GRAB_LOG="$out/app.log" LEDGE_GRAB_SPOTS="$out/spots"
: > "$out/app.log"; : > "$out/pastes.log"; : > "$out/results.txt"
# Timeline of app events and checks, to see where time goes.
tail -n +1 -F "$out/app.log" 2>/dev/null | while IFS= read -r l; do echo "$(now) app: $l"; done >> "$out/timeline.log" & tl=$!
if [ "$LG_MODE" = wayland ]; then
  # Let unsandboxed apps take screenshots without GNOME's one-time prompt.
  gdbus call --session --dest org.freedesktop.impl.portal.PermissionStore \
    --object-path /org/freedesktop/impl/portal/PermissionStore \
    --method org.freedesktop.impl.portal.PermissionStore.SetPermission \
    screenshot true screenshot '' "['yes']" >/dev/null
fi

python3 "$here/paste_target.py" "$out" >"$out/target.log" 2>&1 & tgt=$!
sleep 1.5
GDK_DEBUG="${LG_GDK_DEBUG:-}" "$bin" >"$out/stderr.log" 2>&1 & app=$!
check "app starts" 'wait_for "$out/app.log" "^ready: $LG_MODE"'
sleep 1.5
panel=$(ctl panel)
read -r x y w h above sticky < <(ctl rect "Ledge Grab")
check "ledge centred under the top bar (at $x,$y size ${w}x$h, panel $panel)" \
  '[ "$y" = "$panel" ] && [ $(( x * 2 + w )) -ge 1278 ] && [ $(( x * 2 + w )) -le 1282 ]'
check "ledge kept above and on all workspaces" '[ "$above" = true ] && [ "$sticky" = true ]'
shot 01-rest

# Whole screen: saved, filed, on the clipboard.
"$bin" --screen
check "screen capture saved" 'wait_for "$out/app.log" "^captured 1280x800 as 1"'
check "PNG written to Pictures" '[ "$(find "$out/pics" -name "*.png" | wc -l)" -ge 1 ]'
sleep 0.4; shot 02-flash
check "screen capture pastes as 1280x800" 'paste 1 && [ "$(pasted 1)" = 1280x800 ]'

# Region: drag 400x300 on the frozen frame.
"$bin" --region
check "region picker opens" 'wait_for "$out/app.log" "region picker open"'
sleep 1; shot 03-picker
ctl drag 200 200 600 500
check "region capture saved at 400x300" 'wait_for "$out/app.log" "^captured 400x300 as 2"'
check "region pastes as 400x300" 'paste 2 && [ "$(pasted 2)" = 400x300 ]'

# Region cancelled with Esc: nothing filed.
"$bin" --region
wait_for "$out/app.log" "region picker open" 40; sleep 1
ctl key Escape
check "Esc cancels the region" 'wait_for "$out/app.log" "region canceled"'

# A third shot so Sequential Copy has three images.
"$bin" --screen; wait_for "$out/app.log" "as 3;" 80; sleep 2

hover
check "hover opens the shelf" 'grep -q "^view Shelf" "$out/spots"'
shot 04-shelf
check "shelf shows 3 tiles" '[ "$(tiles | wc -l)" = 3 ]'

# Copy one picked tile.
click_spot "$(tiles | head -1)"
check "picking a tile offers Copy" 'grep -q "^Copy" "$out/spots"'
click_spot Copy
check "Copy puts it on the clipboard" 'wait_for "$out/app.log" "^copy .*: ok"'
check "copied tile pastes" 'paste 3 && [ -n "$(pasted 3)" ] && [ "$(pasted 3)" != error: ]'

# Sequential Copy over all three: each paste gives the next image.
hover
for t in $(tiles | tail -n +2); do click_spot "$t"; done
check "three picked tiles offer Sequential Copy" 'grep -q "^Sequential" "$out/spots"'
click_spot Sequential
check "Sequential Copy starts" 'wait_for "$out/app.log" "^queue start"'
sleep 0.4; shot 05-queue
paste 4; paste 5; paste 6
check "queue ends after the last image" 'wait_for "$out/app.log" "queue ended: finished"'
sizes="$(pasted 4) $(pasted 5) $(pasted 6)"
echo "queue pastes: $sizes; log: $(grep -o "queue placed [0-9]*: [A-Za-z]*" "$out/app.log" | tr '\n' ' ')" >> "$out/results.txt"
# Shots 3, 2, 1 were full screen, region, full screen.
check "queue pastes the three images in order" '[ "$sizes" = "1280x800 400x300 1280x800" ]'
paste 7
check "after the queue, the last image stays on the clipboard" '[ "$(pasted 7)" = "$(pasted 6)" ]'

# A queue stops when another app takes the clipboard.
hover
for t in $(tiles | head -2); do click_spot "$t"; done
click_spot Sequential; wait_for "$out/app.log" "queue start" 30
python3 "$here/taker.py" >"$out/taker.log" 2>&1 &
for _ in $(seq 1 40); do [ -n "$(ctl rect Taker)" ] && break; sleep 0.1; done; sleep 0.5
read -r tx ty tw th _ < <(ctl rect Taker); ctl click $(( tx + tw / 2 )) $(( ty + th / 2 ))
check "queue ends when another app takes the clipboard" \
  'wait_for "$out/app.log" "queue ended: clipboard replaced elsewhere" 60'
paste 8
check "the other app's clipboard is left alone" '[ "$(pasted 8)" = error: ]'
sleep 3

# Delete removes the tile and its file.
hover
n0=$(find "$out/pics" -name "*.png" | wc -l)
click_spot "$(tiles | head -1)"
click_spot Delete
check "Delete removes the screenshot and its file" \
  'wait_for "$out/app.log" "^deleted" && [ "$(find "$out/pics" -name "*.png" | wc -l)" = $((n0 - 1)) ] && [ "$(tiles | wc -l)" = 2 ]'

ctl move 640 700
check "leaving closes the shelf" 'wait_for "$out/spots" "^view Rest" 30'

"$bin" --quit
gone() { for _ in $(seq 1 30); do kill -0 "$1" 2>/dev/null || return 0; sleep 0.1; done; return 1; }
check "--quit stops the running app" 'gone $app'
kill $tgt $tl 2>/dev/null
echo "done: $(grep -c ^PASS "$out/results.txt") passed, $(grep -c ^FAIL "$out/results.txt") failed"

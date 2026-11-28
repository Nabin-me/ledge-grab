#!/usr/bin/env bash
# usage: session.sh wayland|x11 COMMAND...
# Starts a private D-Bus session, PipeWire, and a headless GNOME Shell (Wayland
# compositor with a virtual monitor, or an X11 WM on Xvfb), runs COMMAND inside it,
# then tears everything down. COMMAND sees WAYLAND_DISPLAY / DISPLAY and LG_MODE.
set -u
mode="$1"; shift
if [ -z "${LG_INNER:-}" ]; then
  export LG_INNER=1
  exec dbus-run-session -- bash "$0" "$mode" "$@"
fi
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export XDG_SESSION_TYPE="$mode" XDG_CURRENT_DESKTOP=GNOME GDK_BACKEND="$mode" LG_MODE="$mode"
export NO_AT_BRIDGE=1 GSK_RENDERER=cairo LIBGL_ALWAYS_SOFTWARE=1
# Test helper extension: closes the startup overview so test windows are visible.
gsettings set org.gnome.shell enabled-extensions "['lg-test-helper@ledgegrab', 'ledge-grab@ledgegrab']"
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.desktop.interface enable-animations false
log="$HOME/lg-session-$mode.log"; : > "$log"
pipewire >>"$log" 2>&1 & pw=$!
sleep 0.5; wireplumber >>"$log" 2>&1 & wp=$!
if [ "$mode" = wayland ]; then
  name="lg-wl-$$"
  gnome-shell --headless --wayland --no-x11 --wayland-display="$name" --virtual-monitor 1280x800 >>"$log" 2>&1 & sh=$!
  export WAYLAND_DISPLAY="$name"; unset DISPLAY
  for i in $(seq 1 100); do [ -S "$XDG_RUNTIME_DIR/$name" ] && break; sleep 0.1; done
else
  num=$(( 90 + $$ % 9 ))
  Xvfb ":$num" -screen 0 1280x800x24 -nolisten tcp >>"$log" 2>&1 & xv=$!
  export DISPLAY=":$num"; unset WAYLAND_DISPLAY
  for i in $(seq 1 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.1; done
  gnome-shell --x11 --replace >>"$log" 2>&1 & sh=$!
fi
# Wait for the shell to own its D-Bus name.
for i in $(seq 1 150); do
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner org.gnome.Shell 2>/dev/null | grep -q true && break
  sleep 0.1
done
dbus-update-activation-environment WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE GDK_BACKEND 2>/dev/null
sleep 1.5
echo "session $mode up (shell pid $sh)"
"$@"; rc=$?
kill $sh 2>/dev/null; sleep 0.5
[ -n "${xv:-}" ] && kill $xv 2>/dev/null
kill $wp $pw 2>/dev/null
exit $rc

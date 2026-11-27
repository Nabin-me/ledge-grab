#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
dist=${1:-"$root/out"}
mkdir -p -- "$dist"
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
install -d -- "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/ledge-grab-linux-preview" \
 "$stage/usr/share/applications" "$stage/usr/share/icons/hicolor/scalable/apps" \
 "$stage/usr/share/doc/ledge-grab-linux-preview"
install -m644 "$root/linux/ledge_grab.py" "$root/linux/storage.py" "$root/linux/portal.py" "$stage/usr/share/ledge-grab-linux-preview/"
install -m644 "$root/linux/README.md" "$root/linux/QA.md" "$root/LICENSE" "$stage/usr/share/doc/ledge-grab-linux-preview/"
install -m644 "$root/packaging/linux/app.ledgegrab.Preview.desktop" "$stage/usr/share/applications/"
install -m644 "$root/packaging/linux/app.ledgegrab.Preview.svg" "$stage/usr/share/icons/hicolor/scalable/apps/"
cat >"$stage/usr/bin/ledge-grab-linux" <<'WRAPPER'
#!/bin/sh
set -eu
path="$(dirname -- "$(readlink -f -- "$0")")/../share/ledge-grab-linux-preview/ledge_grab.py"
exec /usr/bin/python3 "$path" "$@"
WRAPPER
chmod 755 "$stage/usr/bin/ledge-grab-linux"
cat >"$stage/DEBIAN/control" <<'CONTROL'
Package: ledge-grab-linux-preview
Version: 0.1.0~preview2
Section: graphics
Priority: optional
Architecture: all
Maintainer: Ledge Grab contributors
Depends: python3 (>= 3.9), python3-gi, gir1.2-gtk-3.0
Recommends: xdg-desktop-portal
Description: Ledge Grab Linux screenshot shelf preview
 Save local PNG screenshots, copy them to the desktop clipboard,
 and browse or hide recent previews without deleting the files.
CONTROL
dpkg-deb --root-owner-group --build "$stage" "$dist/ledge-grab-linux-preview_0.1.0~preview2_all.deb"

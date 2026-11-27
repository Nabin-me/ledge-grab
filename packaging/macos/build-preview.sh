#!/usr/bin/env bash
set -euo pipefail
if [[ $(uname -s) != Darwin ]]; then
    echo "Build the macOS app on a Mac with Xcode (or its command-line tools)." >&2
    exit 1
fi
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=${1:-"$ROOT/macos/dist"}
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
if [[ ${LEDGE_UNIVERSAL:-0} == 1 ]]; then
    BUILD_ARCH=(--arch arm64 --arch x86_64)
else
    BUILD_ARCH=(--arch "$(uname -m)")
fi
swift build --package-path "$ROOT/macos" -c release "${BUILD_ARCH[@]}"
BIN=$(swift build --package-path "$ROOT/macos" -c release "${BUILD_ARCH[@]}" --show-bin-path)
APP="$TMP/Ledge Grab.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$BIN/LedgeGrab" "$APP/Contents/MacOS/LedgeGrab"
cp "$ROOT/packaging/macos/Info.plist" "$APP/Contents/"
plutil -lint "$APP/Contents/Info.plist"
ICONSET="$TMP/LedgeGrab.iconset"
mkdir "$ICONSET"
for SIZE in 16 32 128 256 512; do
    sips -z "$SIZE" "$SIZE" "$ROOT/assets/brand/ledge-grab.png" --out "$ICONSET/icon_${SIZE}x${SIZE}.png" >/dev/null
    DOUBLE=$((SIZE * 2))
    sips -z "$DOUBLE" "$DOUBLE" "$ROOT/assets/brand/ledge-grab.png" --out "$ICONSET/icon_${SIZE}x${SIZE}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/LedgeGrab.icns"
# Render the existing monochrome notch as an alpha template for light and dark menu bars.
swift "$ROOT/packaging/macos/make-menu-icon.swift" "$APP/Contents/Resources/MenuMark.png"
codesign --force --sign - "$APP"
codesign --verify --strict "$APP"
lipo -archs "$APP/Contents/MacOS/LedgeGrab"
if [[ ${LEDGE_UNIVERSAL:-0} == 1 ]]; then
    lipo "$APP/Contents/MacOS/LedgeGrab" -verify_arch arm64 x86_64
fi
ZIP="$OUT/Ledge-Grab-macOS-Preview.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
(cd "$OUT" && shasum -a 256 "$(basename "$ZIP")" > SHA256SUMS.txt)
echo "Created $ZIP (ad-hoc signed developer preview; not notarized)"

#!/usr/bin/env bash
# PREVIEW: build dist/Keywhere-<version>-macos-<arch>.dmg (Keywhere.app plus
# an Applications link to drag it onto) from a release build, and
# dist/Keywhere-<version>-macos-<arch>.app.zip, the same bundle for the
# in-app updater (listed in the signed latest.json).
#
#   packaging/macos/build-app.sh [aarch64-apple-darwin | x86_64-apple-darwin]
#
# Default: the machine's own architecture. Ad-hoc signed only (no Developer
# ID, not notarized): users remove the quarantine mark or choose "Open
# Anyway" once. Run on macOS with Xcode (Metal shader compiler).
set -euo pipefail
cd "$(dirname "$0")/../.."
VERSION=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="keywhere"))')
HOST=$(rustc -vV | sed -n 's/^host: //p')
TARGET=${1:-$HOST}
case "$TARGET" in
    aarch64-apple-darwin) ARCH=arm64 ;;
    x86_64-apple-darwin) ARCH=x64 ;;
    *) echo "unsupported target $TARGET" >&2; exit 1 ;;
esac
cargo build --locked --release -p keywhere --target "$TARGET"
BIN=target/$TARGET/release/keywhere
# The icon renderer runs here, so build it for this machine.
cargo build --locked --release -p keywhere --example render_icon

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APP=$WORK/dmg/Keywhere.app
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m755 "$BIN" "$APP/Contents/MacOS/keywhere"

# .icns from the logo SVG.
target/release/examples/render_icon "$WORK/png" --macos-plate >/dev/null
SET=$WORK/keywhere.iconset
mkdir -p "$SET"
for s in 16 32 128 256 512; do
    cp "$WORK/png/icon_$s.png" "$SET/icon_${s}x${s}.png"
    cp "$WORK/png/icon_$((s * 2)).png" "$SET/icon_${s}x${s}@2x.png"
done
iconutil -c icns -o "$APP/Contents/Resources/keywhere.icns" "$SET"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Keywhere</string>
  <key>CFBundleDisplayName</key><string>Keywhere</string>
  <key>CFBundleIdentifier</key><string>io.github.vcostin.keywhere</string>
  <key>CFBundleExecutable</key><string>keywhere</string>
  <key>CFBundleIconFile</key><string>keywhere</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Password Safe database</string>
      <key>CFBundleTypeExtensions</key><array><string>psafe3</string><string>dat</string></array>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
# Only runnable here if it's this machine's architecture.
if [ "$TARGET" = "$HOST" ]; then
    "$APP/Contents/MacOS/keywhere" --version | grep "^Keywhere $VERSION"
fi

mkdir -p dist
# For the in-app updater, which unpacks it with ditto too.
ZIP=dist/Keywhere-$VERSION-macos-$ARCH.app.zip
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"

ln -s /Applications "$WORK/dmg/Applications"
OUT=dist/Keywhere-$VERSION-macos-$ARCH.dmg
rm -f "$OUT"
hdiutil create -volname "Keywhere $VERSION" -srcfolder "$WORK/dmg" -fs HFS+ \
    -format UDZO -imagekey zlib-level=9 -ov "$OUT" >/dev/null
hdiutil verify "$OUT" >/dev/null
echo "$OUT"
echo "$ZIP"

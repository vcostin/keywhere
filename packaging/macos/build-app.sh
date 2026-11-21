#!/usr/bin/env bash
# PREVIEW: build dist/Keywhere-<version>-macos-<arch>-preview.zip (Keywhere.app)
# from a release build. Ad-hoc signed only (no Developer ID, not notarized),
# so Gatekeeper asks the user to allow it. Run on macOS.
set -euo pipefail
cd "$(dirname "$0")/../.."
VERSION=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="keywhere"))')
ARCH=$(uname -m)
cargo build --locked --release -p keywhere
cargo build --locked --release -p keywhere --example render_icon

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APP=$WORK/Keywhere.app
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m755 target/release/keywhere "$APP/Contents/MacOS/keywhere"

# .icns from the logo SVG.
target/release/examples/render_icon "$WORK/png" >/dev/null
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
"$APP/Contents/MacOS/keywhere" --version | grep "^Keywhere $VERSION"

mkdir -p dist
OUT=dist/Keywhere-$VERSION-macos-$ARCH-preview.zip
rm -f "$OUT"
ditto -c -k --keepParent "$APP" "$OUT"
echo "$OUT"

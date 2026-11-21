#!/usr/bin/env bash
# Build dist/Keywhere-<version>-x86_64.AppImage from a release build.
#
#   packaging/linux/build-appimage.sh
#
# appimagetool and the type-2 runtime are pinned by version and SHA-256 and
# cached in target/appimage-tools. No libraries are bundled: the binary only
# links libc, libxcb and libxkbcommon (present on every desktop) and loads
# Wayland/Vulkan from the host at run time.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD

ARCH=x86_64
TOOL_URL=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage
TOOL_SHA=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
RUNTIME_URL=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
RUNTIME_SHA=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

VERSION=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="keywhere"))')
OUT=$ROOT/dist/Keywhere-$VERSION-$ARCH.AppImage

fetch() { # url sha dest
  if [ ! -f "$3" ] || ! echo "$2  $3" | sha256sum -c --status; then
    curl -fsSL --retry 3 -o "$3.part" "$1"
    echo "$2  $3.part" | sha256sum -c --quiet
    mv "$3.part" "$3"
  fi
}
TOOLS=$ROOT/target/appimage-tools
mkdir -p "$TOOLS" dist
fetch "$TOOL_URL" "$TOOL_SHA" "$TOOLS/appimagetool"
fetch "$RUNTIME_URL" "$RUNTIME_SHA" "$TOOLS/runtime"
chmod +x "$TOOLS/appimagetool"

cargo build --release --locked -p keywhere

APPDIR=$ROOT/target/Keywhere.AppDir
rm -rf "$APPDIR"
install -Dm755 target/release/keywhere "$APPDIR/usr/bin/keywhere"
install -Dm755 packaging/linux/AppRun "$APPDIR/AppRun"
install -Dm644 packaging/linux/keywhere.desktop "$APPDIR/keywhere.desktop"
install -Dm644 packaging/linux/keywhere.desktop "$APPDIR/usr/share/applications/keywhere.desktop"
install -Dm644 assets/keywhere-key.svg "$APPDIR/keywhere.svg"
install -Dm644 assets/keywhere-key.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/keywhere.svg"
ln -s keywhere.svg "$APPDIR/.DirIcon"

# Reproducible-ish: fixed mtimes inside the image.
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
ARCH=$ARCH APPIMAGE_EXTRACT_AND_RUN=1 "$TOOLS/appimagetool" --no-appstream \
  --runtime-file "$TOOLS/runtime" "$APPDIR" "$OUT" >/dev/null 2>&1
chmod 755 "$OUT"

# The updater requires the new file to report its version before swapping.
"$OUT" --version | grep -q "^Keywhere $VERSION " || {
  echo "error: $OUT --version did not report $VERSION" >&2; exit 1; }
echo "$OUT"

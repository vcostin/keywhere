#!/usr/bin/env bash
# Write dist/latest.json for the in-app updater from the built artifacts.
#
#   packaging/make-update-manifest.sh <version>
#
# Sign it afterwards (release workflow):
#   rsign sign -W -s key -x dist/latest.json.minisig -t "keywhere <version>" dist/latest.json
set -euo pipefail
cd "$(dirname "$0")/.."
V=${1:?version}
BASE=https://github.com/vcostin/keywhere/releases/download/v$V
entry() { # platform file
  local f=$2
  printf '"%s":{"url":"%s/%s","sha256":"%s","size":%s}' \
    "$1" "$BASE" "$(basename "$f")" "$(sha256sum "$f" | cut -d' ' -f1)" "$(stat -c %s "$f")"
}
platforms=()
# Keys are `<std::env::consts::OS>-<ARCH>` (update::platform_key).
add() { # platform file
  if [ -f "$2" ]; then platforms+=("$(entry "$1" "$2")"); fi
}
add linux-x86_64 "dist/Keywhere-$V-x86_64.AppImage"
add macos-aarch64 "dist/Keywhere-$V-macos-arm64.app.zip"
add macos-x86_64 "dist/Keywhere-$V-macos-x64.app.zip"
add windows-x86_64 "dist/Keywhere-$V-windows-x64-portable.exe"
[ ${#platforms[@]} -gt 0 ] || { echo "no artifacts for $V in dist/" >&2; exit 1; }
IFS=,
printf '{"version":"%s","pub_date":"%s","notes_url":"https://github.com/vcostin/keywhere/releases/tag/v%s","platforms":{%s}}\n' \
  "$V" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$V" "${platforms[*]}" > dist/latest.json
python3 -m json.tool dist/latest.json >/dev/null
cat dist/latest.json

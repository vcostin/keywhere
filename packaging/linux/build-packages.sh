#!/usr/bin/env bash
# Build dist/keywhere_<version>_amd64.deb and dist/keywhere-<version>-1.x86_64.rpm
# from target/release/keywhere (run build-appimage.sh first, or cargo build
# --release). nfpm is pinned by version and SHA-256.
#
# deb/rpm installs belong to the system package manager: the in-app updater
# only tells their users about new versions, it never replaces the files.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD

NFPM_URL=https://github.com/goreleaser/nfpm/releases/download/v2.49.0/nfpm_2.49.0_Linux_x86_64.tar.gz
NFPM_SHA=2afaa5ffb5e3493402e31e7c67a5a1fb42133e3f66fc315ca0051934125561c7

TOOLS=$ROOT/target/package-tools
mkdir -p "$TOOLS" "$ROOT/dist"
if [ ! -x "$TOOLS/nfpm" ]; then
  curl -fsSL --retry 3 -o "$TOOLS/nfpm.tar.gz" "$NFPM_URL"
  echo "$NFPM_SHA  $TOOLS/nfpm.tar.gz" | sha256sum -c --quiet
  tar -xzf "$TOOLS/nfpm.tar.gz" -C "$TOOLS" nfpm
  rm "$TOOLS/nfpm.tar.gz"
fi

[ -x target/release/keywhere ] || cargo build --release --locked -p keywhere
export KEYWHERE_VERSION=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="keywhere"))')
target/release/keywhere --version | grep -q "^Keywhere $KEYWHERE_VERSION " || {
  echo "error: target/release/keywhere is not version $KEYWHERE_VERSION" >&2; exit 1; }

export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
for fmt in deb rpm; do
  "$TOOLS/nfpm" package --config packaging/linux/nfpm.yaml --packager "$fmt" --target dist/
done
ls -1 dist/*.deb dist/*.rpm

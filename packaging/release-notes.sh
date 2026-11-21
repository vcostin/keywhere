#!/usr/bin/env bash
# Release notes for tag $1 (e.g. v0.2.0): the commits since the previous
# release tag, plus download hints. Keywhere commits straight to main, so
# GitHub's PR-based "generate notes" would be empty; this lists commits.
#
#   packaging/release-notes.sh v0.2.0 > notes.md
set -euo pipefail
cd "$(dirname "$0")/.."
TAG=${1:?tag}
V=${TAG#v}
PREV=$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname --merged "$TAG^{commit}" \
  | grep -vx "$TAG" | head -1 || true)

if [ -n "$PREV" ]; then
  echo "## Changes since $PREV"
  echo
  # Subjects only; review-log and version-bump commits are bookkeeping.
  git log "$PREV..$TAG" --no-merges --reverse --format='- %s (%h)' \
    | grep -vE '^- (Cursor Auto review|Release v[0-9])' || echo "- No code changes."
else
  echo "First release."
fi
echo
cat <<NOTES
## Downloads

* **Keywhere-$V-x86_64.AppImage**: any Linux distribution; updates itself from the app.
* **keywhere_$V-1_amd64.deb**: Debian, Ubuntu 22.04+.
* **keywhere-$V-1.x86_64.rpm**: Fedora, openSUSE.

Checksums are in \`SHA256SUMS\`. The in-app updater checks the signed \`latest.json\`.
NOTES
if [ -n "$PREV" ]; then
  echo
  echo "**Full changelog:** https://github.com/vcostin/keywhere/compare/$PREV...$TAG"
fi

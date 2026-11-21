#!/usr/bin/env bash
# Download section for the release of tag $1 (e.g. v0.2.0). The release
# workflow passes it to `gh release create --notes-file … --generate-notes`;
# GitHub prepends it to its generated "What's Changed" list of merged pull
# requests (one entry per squash-merged PR) and the full-changelog link.
#
#   packaging/release-notes.sh v0.2.0
set -euo pipefail
V=${1:?tag}
V=${V#v}
cat <<NOTES
## Downloads

* **Keywhere-$V-x86_64.AppImage**: any Linux distribution; updates itself from the app.
* **keywhere_$V-1_amd64.deb**: Debian, Ubuntu 22.04+.
* **keywhere-$V-1.x86_64.rpm**: Fedora, openSUSE.

Checksums are in \`SHA256SUMS\`. The in-app updater checks the signed \`latest.json\`.
NOTES

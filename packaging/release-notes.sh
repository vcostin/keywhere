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

**Preview, unsigned** (not code-signed or notarized yet; no tray icon and
no in-app updates yet). They lock with the screen lock and before sleep,
like the Linux build:

* **Keywhere-$V-macos-arm64.dmg** (Apple silicon) /
  **Keywhere-$V-macos-x64.dmg** (Intel): open the .dmg and drag Keywhere to
  Applications, then run once in Terminal (macOS blocks unsigned downloads
  until this mark is removed):

  \`\`\`sh
  xattr -dr com.apple.quarantine /Applications/Keywhere.app
  \`\`\`

  Or open it once, then System Settings → Privacy & Security → Open Anyway.
* **Keywhere-$V-windows-x64-portable.exe**: Windows 10/11. A single
  program, nothing to install: put it anywhere and run it (settings go to
  \`%APPDATA%\\keywhere\`). SmartScreen warns on first start: "More info" →
  "Run anyway" (or run \`Unblock-File\` on it in PowerShell first).

Checksums are in \`SHA256SUMS\`: check a preview file against it before you
allow it to run. The in-app updater checks the signed \`latest.json\`.
NOTES

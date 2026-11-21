# Releasing Keywhere

A release is a `vX.Y.Z` tag. The [release workflow](../.github/workflows/release.yml) builds
the Linux AppImage, `.deb` and `.rpm` (all from one binary on one runner), writes and signs `latest.json` for the in‑app updater, and publishes a
GitHub Release with `SHA256SUMS`. The release notes are GitHub's generated list of pull requests merged since
the previous tag, after a download section from `packaging/release-notes.sh`.

## Changes go through pull requests

`main` is protected: changes land through pull requests that pass the CI checks, merged with
**squash**. The PR title becomes the commit subject and the release‑notes entry, so write it as
a sentence a user would understand. Admins can bypass in an emergency. There is no update server besides GitHub Releases.

## Cut a release

1. Smoke test locally:

   ```sh
   packaging/linux/build-appimage.sh        # → dist/Keywhere-<version>-x86_64.AppImage
   packaging/linux/build-packages.sh        # → dist/keywhere_<version>-1_amd64.deb, .rpm
   ./dist/Keywhere-*-x86_64.AppImage        # unlock, tray, Settings → Updates
   ```

2. Bump `version` in the root `Cargo.toml` (`[workspace.package]`), run `cargo check` so
   `Cargo.lock` follows, commit.
3. Tag and push:

   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```

The workflow runs the full release‑mode tests and lints on macOS and Windows in parallel with
the Linux package build (pull requests only check Linux, with a lighter build profile). Linux
refuses a tag that doesn't match the workspace version, then tests, builds the AppImage/`.deb`/`.rpm`,
runs the AppImage end‑to‑end test, and installs the packages in containers. A final job waits for
both sides, signs `latest.json`, verifies the signature against the committed public key, and only
then publishes. The release is marked *latest*; the updater reads
`https://github.com/vcostin/keywhere/releases/latest/download/latest.json`.

To test an in‑app update end to end: run the previous release's AppImage, publish the new tag,
then Settings → Updates → **Check now** → **Update and restart**.

## Update signing (minisign / Ed25519)

* Public key: `packaging/keywhere-update.pub`, compiled into the app. Changing it means
  releases signed with the new key are only accepted by builds that contain it.
* Private key: repository secret **`KEYWHERE_UPDATE_KEY`** (the whole key file), plus an offline
  copy in a password manager. Never commit it.

Generate a key pair (once):

```sh
cargo install --locked rsign2
rsign generate -W -p packaging/keywhere-update.pub -s ~/.keywhere-release/keywhere-update.key
gh secret set KEYWHERE_UPDATE_KEY < ~/.keywhere-release/keywhere-update.key
```

Sign and check by hand (what the workflow does):

```sh
packaging/make-update-manifest.sh 0.2.0
rsign sign -W -s ~/.keywhere-release/keywhere-update.key -x dist/latest.json.minisig \
  -t "keywhere 0.2.0" dist/latest.json
rsign verify -p packaging/keywhere-update.pub -x dist/latest.json.minisig dist/latest.json
```

### Manifest

```json
{"version":"0.2.0","pub_date":"…","notes_url":"https://github.com/vcostin/keywhere/releases/tag/v0.2.0",
 "platforms":{"linux-x86_64":{"url":"https://github.com/vcostin/keywhere/releases/download/v0.2.0/Keywhere-0.2.0-x86_64.AppImage",
   "sha256":"…","size":12345678}}}
```

The app rejects a manifest whose URLs leave `github.com/vcostin/keywhere/releases/download/v<version>/`,
whose release page isn't that version's tag page, or whose version isn't newer than its own.

## .deb and .rpm

Built with [nfpm](https://nfpm.goreleaser.com) (pinned by SHA‑256 in
`packaging/linux/build-packages.sh`) from `packaging/linux/nfpm.yaml`. They install the binary,
desktop entry, icon and an `application/x-psafe3` MIME type. The `.rpm` requires library
sonames rather than package names, so one file works on Fedora and openSUSE. Packages aren't
GPG‑signed; `SHA256SUMS` on the release covers them. The Linux job installs the `.deb`
on Ubuntu 22.04 and the `.rpm` in Fedora and openSUSE containers and runs `keywhere --version` before
the Publish job creates the GitHub Release. The package manager owns these installs, so the in‑app updater only shows a notice
and a download link for them.

## AppImage notes

* Built on Ubuntu 22.04 (glibc 2.35) so it runs on older distributions.
* `appimagetool` 1.9.1 and the type‑2 runtime are pinned by SHA‑256 in
  `packaging/linux/build-appimage.sh`.
* Nothing is bundled besides the binary, the desktop entry and the icon: Keywhere links only
  libc, libxcb and libxkbcommon and loads Wayland/Vulkan from the system.
* The type‑2 runtime needs FUSE 2 (`libfuse2`) at run time, or
  `APPIMAGE_EXTRACT_AND_RUN=1`.

## macOS and Windows previews

The release's `platforms` jobs also build unsigned preview packages:

* `packaging/macos/build-app.sh <target>` builds `Keywhere-<v>-macos-arm64.dmg` and
  `Keywhere-<v>-macos-x64.dmg` (both on the Apple silicon runner; the Intel one is
  cross-compiled). Each is a drag-to-Applications disk image holding a `Keywhere.app` bundle
  (icon rendered from the logo by the `render_icon` example), ad-hoc signed only: no Developer
  ID, no notarization, so users remove the quarantine mark or choose "Open Anyway" once.
* `packaging/windows/build-exe.ps1` builds `Keywhere-<v>-windows-x64-portable.exe`: the bare
  exe, which needs no install. It has a static C runtime (`.cargo/config.toml`), and `build.rs`
  embeds the icon (`packaging/windows/keywhere.ico`, rendered from the logo) and version info.
  The script fails if the exe imports VCRUNTIME140.dll or its version resource is wrong. The
  exe isn't code-signed, so SmartScreen warns.
* `packaging/windows/build-installers.ps1` wraps that exe in two installers:
  * `Keywhere-<v>-windows-x64-setup.exe` (NSIS, `keywhere.nsi`): per-user, into
    `%LOCALAPPDATA%\Programs\Keywhere`, no admin rights. It adds a Start menu shortcut, an
    uninstaller and "Open with" for `.psafe3`, and becomes the default app only if no other
    program is. That folder is the user's own, so the in-app updater replaces `Keywhere.exe`
    there, and the app refreshes the version shown in Settings → Apps.
  * `Keywhere-<v>-windows-x64.msi` (WiX v5, pinned, `keywhere.wxs`): per-machine, in Program
    Files. Users can't write there, so the app only shows a notice; the next MSI upgrades it
    (MajorUpgrade, with a fixed UpgradeCode). It never takes over the default app for `.psafe3`.

  NSIS comes with GitHub's Windows images (Chocolatey is the fallback); WiX is installed as a
  pinned `dotnet` tool.

`build-app.sh` also writes `Keywhere-<v>-macos-<arch>.app.zip` (the same bundle, zipped with
`ditto`) for the in-app updater. The release gets seven macOS/Windows files (two `.dmg`, two
`.app.zip`, the portable exe, setup exe and MSI). The Publish job downloads them with the Linux
packages, writes `SHA256SUMS`, and attaches everything to the GitHub Release.
`make-update-manifest.sh` lists the `.app.zip` files and the portable exe in
`latest.json` (`macos-aarch64`, `macos-x86_64`, `windows-x86_64`) next to the AppImage. To
build them without a release, run the
**Preview packages** workflow from the Actions tab. It also runs on PRs that change these
scripts.

They stay "preview" until they are code-signed (Apple Developer ID plus notarization, and
Windows code signing), have been tested on real machines, and have single-instance handling on
Windows like Linux.

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

The workflow first runs the full release‑mode tests and lints on macOS and Windows (pull
requests only check Linux, with a lighter build profile). It then refuses a tag that doesn't match
the workspace version, runs the Linux tests, builds,
signs, verifies the signature against the committed public key, and only then publishes. The
release is marked *latest*; the updater reads
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
GPG‑signed; `SHA256SUMS` on the release covers them. The release job installs the `.deb`
on Ubuntu 22.04 and the `.rpm` in Fedora and openSUSE containers and runs `keywhere --version` before
publishing. The package manager owns these installs, so the in‑app updater only shows a notice
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

* `packaging/macos/build-app.sh` builds `Keywhere-<v>-macos-arm64-preview.zip`: a
  `Keywhere.app` bundle (icon rendered from the logo by the `render_icon` example), ad-hoc
  signed only. It has no Developer ID signature and no notarization, so Gatekeeper asks the
  user to allow it.
* `packaging/windows/build-zip.ps1` builds `Keywhere-<v>-windows-x64-preview.zip` with
  `keywhere.exe`, the licence and a README. It isn't code-signed, so SmartScreen warns.

The Linux job adds both to `SHA256SUMS` and the release. They aren't in `latest.json`: the
in-app updater only replaces AppImages. To build them without a release, run the
**Preview packages** workflow from the Actions tab. It also runs on PRs that change these
scripts.

They stay "preview" until they are code-signed (Apple Developer ID plus notarization, and
Windows code signing), have been tested on real machines, and have a tray and single-instance
handling like Linux.

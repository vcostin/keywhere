# Cursor review — Keywhere M5 packages (.deb / .rpm)

Independent review of commit `ba83eb859fb0a1438968e759a7245d197dcec3a7`
(`Release: .deb and .rpm from the same binary in the one Linux job`).

Scope: `packaging/linux/nfpm.yaml`, `packaging/linux/build-packages.sh`,
`packaging/linux/keywhere-mime.xml`, `.github/workflows/release.yml`,
`README.md`, `docs/releasing.md`. Cross-checked against
`packaging/linux/build-appimage.sh`, `packaging/linux/keywhere.desktop`,
`ldd target/release/keywhere`, and a local `packaging/linux/build-packages.sh`
run (artifacts `dist/keywhere_0.1.0-1_amd64.deb`,
`dist/keywhere-0.1.0-1.x86_64.rpm`).

Did not modify sources (only this review file). Did not push, tag, create
releases, or touch secrets.

## Summary

nfpm packaging is close: asset names match what nfpm emits, the SHA-256 pin for
nfpm v2.49.0 matches GitHub’s published digest, Debian Depends match the
dynamic libraries on Ubuntu 22.04+, file layout and MIME XML look right, and the
release job order (AppImage → packages → install-test → checksums → explicit
upload) is sound. Block shipping the openSUSE claim until RPM Requires resolve
there (or drop openSUSE from the docs).

## Findings

1. **High** — RPM Depends are Fedora-only; README/docs claim openSUSE
   ([`packaging/linux/nfpm.yaml:35-36`](../../packaging/linux/nfpm.yaml),
   [`README.md:17-18`](../../README.md))  
   Requires `libxcb`, `libxkbcommon`, `libxkbcommon-x11`, `vulkan-loader`,
   Recommends `libwayland-client`. Those names exist on Fedora 41; on openSUSE
   the runtime packages are `libxcb1`, `libxkbcommon0`, `libxkbcommon-x11-0`,
   `libvulkan1`, `libwayland-client0` (soname packages do not Provide the
   Fedora names). `zypper install ./keywhere-*.rpm` will fail dependency
   resolution. README also shows `dnf` for the rpm path.  
   **Fix:** Prefer ELF soname Requires that both distros Provide, e.g.
   `libxcb.so.1()(64bit)`, `libxkbcommon.so.0()(64bit)`,
   `libxkbcommon-x11.so.0()(64bit)`, `libvulkan.so.1()(64bit)` (and the
   Wayland soname as a recommend); or keep Fedora package names and narrow
   README/`docs/releasing.md` to Fedora only (and say AppImage for openSUSE).

2. **Medium** — Cached nfpm binary is never re-hashed
   ([`packaging/linux/build-packages.sh:17-22`](../../packaging/linux/build-packages.sh))  
   Download + `sha256sum -c` runs only when `$TOOLS/nfpm` is missing. A stale
   or swapped binary in `target/package-tools/`, or a pin bump that leaves the
   old file in place, skips verification. `build-appimage.sh` re-checks SHA on
   every run. Fresh GHA runners hide this; local/reusable caches do not.  
   **Fix:** Mirror the AppImage `fetch()` pattern (always verify SHA, or store
   under a versioned path like `nfpm-2.49.0` and key the `! -x` check on that).

3. **Medium** — MIME / desktop packages omit their trigger owners
   ([`packaging/linux/nfpm.yaml:32-36`](../../packaging/linux/nfpm.yaml))  
   The package ships `/usr/share/mime/packages/keywhere.xml` and a `.desktop`
   file but does not Depend/Recommend `shared-mime-info` or
   `desktop-file-utils`. Without those packages the files sit inert (no
   `update-mime-database` / `update-desktop-database` triggers). Debian
   practice is to Depend on `shared-mime-info` when shipping MIME info.  
   **Fix:** Add `shared-mime-info` and `desktop-file-utils` to deb Depends (or
   at least Recommends); on rpm use the same package names (identical on
   Fedora/openSUSE) or soname-irrelevant package Depends.

4. **Low** — Script header names the `.deb` without the release suffix
   ([`packaging/linux/build-packages.sh:2`](../../packaging/linux/build-packages.sh))  
   Comment says `dist/keywhere_<version>_amd64.deb`; nfpm with `release: 1`
   produces `keywhere_<version>-1_amd64.deb` (confirmed locally; workflow paths
   are correct).  
   **Fix:** Align the comment with `keywhere_<version>-1_amd64.deb`.

5. **Low** — Release job display name still “Linux AppImage”
   ([`.github/workflows/release.yml:18`](../../.github/workflows/release.yml))  
   Job now publishes AppImage + deb + rpm.  
   **Fix:** Rename to e.g. `Linux` or `Linux packages`.

6. **Low** — Install-test pins `fedora:41` (EOL by review date)
   ([`.github/workflows/release.yml:63-64`](../../.github/workflows/release.yml))  
   Image tag still resolves today, but F41 is past the usual ~13‑month window
   (review date 2026-11-21). Mirrors and base packages will rot.  
   **Fix:** Move to a maintained tag (`fedora:42` / `fedora:43` or
   `fedora:latest`) once install-tested.

## Reviewed clean (no change requested)

- **nfpm pin:** URL `v2.49.0` / `nfpm_2.49.0_Linux_x86_64.tar.gz` SHA-256
  `2afaa5ffb5e3493402e31e7c67a5a1fb42133e3f66fc315ca0051934125561c7` matches
  the GitHub release `digest` and a fresh download.
- **Asset names:** Local build produced exactly
  `keywhere_${V}-1_amd64.deb` and `keywhere-${V}-1.x86_64.rpm`; workflow
  install-test, `SHA256SUMS` globs, and `gh release create` paths match.
- **Debian Depends:** `libc6 (>= 2.35)`, `libxcb1`, `libxkbcommon0`,
  `libxkbcommon-x11-0`, `libvulkan1` + Recommends `libwayland-client0` match
  `ldd` (libc, libxcb, libxkbcommon, libxkbcommon-x11; Wayland/Vulkan at
  runtime) and the Ubuntu 22.04 build baseline.
- **File layout:** `/usr/bin/keywhere` (0755), desktop entry, MIME package,
  hicolor scalable icon, `/usr/share/doc/keywhere/LICENSE` — present in both
  packages.
- **MIME XML:** `application/x-psafe3`, `*.psafe3`, magic `PWS3` @0 priority
  50; desktop `MimeType=` matches.
- **Script robustness:** `set -euo pipefail`; refuses a missing binary unless
  it builds; `--version` must match `cargo metadata` (`^Keywhere $VER `);
  `SOURCE_DATE_EPOCH` from git. CI rebuilds the binary in the AppImage step
  before packaging, so same-version local staleness is not a release issue.
- **Workflow:** Package build after AppImage; install-test before publish;
  `$V` expanded in the docker `sh -c` double-quoted string; unsigned local
  `.rpm` via `dnf install` is fine on F41; checksums and explicit upload list
  include deb/rpm.
- **Updater boundary:** Existing `Install::Manual` path for non-AppImage
  installs; docs correctly say deb/rpm users only get a notice.

VERDICT: CHANGES REQUESTED

## Re-check (HEAD)

Re-check of commit `a457ea0eb122f5834d6895dc30f4d55350b1c824`
(`Packages: fixes from Cursor review (findings 1-6)`), against the findings
above on `ba83eb8`. Did not modify sources (only this review file). Did not
push, tag, create releases, or touch secrets. Ran `packaging/linux/build-packages.sh`
twice, parsed `dist/keywhere-0.1.0-1.x86_64.rpm` Requires/Recommends and the
`.deb` control stanza. No local Docker/zypper to re-run the container
install-tests.

### Finding status

1. **Resolved** — RPM Requires are sonames
   `libxcb.so.1()(64bit)`, `libxkbcommon.so.0()(64bit)`,
   `libxkbcommon-x11.so.0()(64bit)`, `libvulkan.so.1()(64bit)`; Recommend
   `libwayland-client.so.0()(64bit)`. Old Fedora package names are gone from
   the RPM header. Workflow install-tests on `fedora:latest` and
   `opensuse/tumbleweed` with `zypper -n --quiet install --allow-unsigned-rpm`;
   README shows the matching `zypper install --allow-unsigned-rpm` command;
   `docs/releasing.md` documents sonames + unsigned + SHA256SUMS.

2. **Resolved** — `build-packages.sh` keeps
   `target/package-tools/nfpm_2.49.0_Linux_x86_64.tar.gz`, re-checks SHA-256
   every run (`sha256sum -c --status`), re-downloads on miss/mismatch via
   `.part` then `mv`, and always `rm -f` + extracts `nfpm` fresh. Confirmed
   archive digest
   `2afaa5ffb5e3493402e31e7c67a5a1fb42133e3f66fc315ca0051934125561c7`; a
   deliberately corrupted `$TOOLS/nfpm` was restored on the next run.

3. **Resolved** — Both formats Recommend `shared-mime-info` and
   `desktop-file-utils` (deb control + RPM RecommendName). Acceptable vs the
   “Depends or at least Recommends” ask.

4. **Resolved** — Script header comment is
   `dist/keywhere_<version>-1_amd64.deb`.

5. **Resolved** — Job name is `Linux (AppImage, .deb, .rpm)`.

6. **Resolved** — Install-test image is `fedora:latest`.

### New findings

None. Docker `sh -c` strings correctly expand `$RPM` / `$V` in the host
double-quoted command so the inner shell sees a concrete path and
`grep '^Keywhere <ver> '`; zypper global (`-n --quiet`) vs command
(`--allow-unsigned-rpm`) flag order is valid.

VERDICT: APPROVE

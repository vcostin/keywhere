# Cursor review — Keywhere M5 (in-app updater)

Independent security-minded review of commit `8bc4e29` (`M5: AppImage packaging, signed releases and in-app updates`).
Scope: `crates/keywhere-app/src/update.rs`, `update_view.rs`, settings/opt-in wiring, `lib.rs`/`main.rs` install + `--after-update`, `packaging/**`, `.github/workflows/release.yml`, `docs/releasing.md`, `SECURITY.md` Updates section, `deny.toml`.
Checks: `cargo test -p keywhere` — 25 passed; `cargo clippy --workspace --all-targets -- -D warnings` — clean.
Did not modify sources. Did not push, create releases, or touch signing secrets.

## Summary

The trust model is sound in outline: opt-in networking, minisign over `latest.json` with a compiled-in key (`allow_legacy = false` → prehashed only), URL pin to this repo’s release assets for that version, streaming size cap + SHA-256, no downgrade, Flatpak/manual installs not overwritten, release workflow verifies the signature against the committed public key before `gh release create --latest`. Tests cover signature tampering, URL pinning, and replace-on-verify.

Before shipping the updater to users, fix the temp-file TOCTOU (verify-then-exec/rename by path) and blind `$APPIMAGE` trust. The after-update single-instance handoff can also silently fail on a slow quit.

## Findings

1. **High** — TOCTOU between checksum and exec/rename (`crates/keywhere-app/src/update.rs:315-320`)  
   After `sync_all` / `drop(file)`, `make_executable`, `check_reports_version`, and `fs::rename` all re-open the temp path by name. A writer with directory write permission (world/group-writable install dir, or sticky-bit-free shared folder) can unlink the 0600 temp inode and replace it with their own file after the SHA-256 check; the process then executes and installs the replacement. Same-UID malware is already in the out-of-scope tray, but a cross-user shared directory is not.  
   **Fix:** Keep the `File` open; re-hash from that fd (or `File::sync_all` + `std::os::fd`); run version check via `/proc/self/fd/N` (or `fexecve`); `rename`/`linkat` that same inode over `target`. Optionally refuse to update if `dir` mode has group/other write.

2. **High** — `$APPIMAGE` trusted without proving this process is that image (`crates/keywhere-app/src/update.rs:110-143`)  
   `install_method` only checks: not Flatpak, `APPIMAGE` set, absolute path, `symlink_metadata` is a regular file, parent is `W_OK`. A cargo/distro binary (or wrapper) with a forged or inherited `APPIMAGE=/path/to/any/writable/file` will offer “Update and restart” and atomically overwrite that path with the downloaded payload. Symlinks are correctly rejected; arbitrary regular files are not.  
   **Fix:** Require AppImage type-2 magic on the file (e.g. `AI\x02` at offset 8), and that `/proc/self/exe` lives under the FUSE/extract mount for that image (or that the running inode matches the AppImage’s). Refuse update if those checks fail → `Install::Manual`.

3. **Medium** — `--after-update` handoff can forward to the old instance and exit (`crates/keywhere-app/src/main.rs:9-14`, `update.rs:401-409`, `lib.rs:451-466`)  
   New process waits up to 15s for `/proc/<pid>` to vanish, then calls `run()` → `forward_to_running("show")`. If quit is slow (or hangs), the new process talks to the still-running old instance, returns, and exits: disk already has the new AppImage, but the user keeps the old in-memory process with no error. PID reuse can also make `wait_for_exit` wait on an unrelated process.  
   **Fix:** Prefer `pidfd_open` + poll, or wait until the socket is gone and bind succeeds. On `--after-update`, do not `forward_to_running`; retry listen for a few seconds, then surface a hard error. Optionally have the old process `exec` the new binary instead of spawn+quit.

4. **Medium** — Every tagged release is forced `--latest` (`.github/workflows/release.yml:114-120`)  
   `gh release create … --latest` runs for every `v*` tag. Publishing an older patch after a newer tag (or a mistaken tag) demotes GitHub `releases/latest`, so `latest.json` can hide a newer signed release from anyone still on an older build (they only see the demoted “latest”; clients already on the real newest correctly refuse a downgrade). Matches the documented “hide updates” threat, but easy to cause by mistake.  
   **Fix:** Only pass `--latest` when `$GITHUB_REF_NAME` is the highest semver among existing releases (or omit `--latest` and set it in a follow-up step after comparing tags).

5. **Medium** — Lock-before-restart does not finish the lock flush path (`crates/keywhere-app/src/lib.rs:709-722`, `336-349`)  
   `install_update` calls `shell.lock` (which spawns async `flush_frames`), then immediately `drop_hidden_session`, `relaunch`, `cx.quit()`. Unlock UI + quit can race the frame flush / layout zeroization that normal lock relies on. Vault drop still runs when the `Browser` entity is replaced.  
   **Fix:** Reuse a synchronous lock+sweep helper before relaunch (await flush, or call `secret_text::sweep` + layout clear inline), then spawn the new process.

6. **Low** — Manual network failures never appear in the toast (`crates/keywhere-app/src/update_view.rs:92`)  
   Toast matches only `Failed(Verify|Install)`; `UpdateError::Network` after Settings → Check now is Settings-only. Easy to miss offline failures.  
   **Fix:** Include `UpdateError::Network` in the toast arm (or map it to the same dismissible card).

7. **Low** — `wait_for_exit` only checks `/proc/<pid>` existence (`crates/keywhere-app/src/update.rs:403-409`)  
   No check that the pid was the parent Keywhere, same uid, or same executable. Harmless delay if abused; interacts with finding 3 on PID reuse.  
   **Fix:** Use `pidfd` from the parent before spawn, or pass a one-time socket/lock file the old process removes on exit.

8. **Low** — Release `dist/*` upload is an unscoped glob (`.github/workflows/release.yml:116`)  
   On a dirty self-hosted or reused workspace, unexpected files under `dist/` would be published. Hosted `ubuntu-22.04` + fresh checkout makes this unlikely.  
   **Fix:** Upload an explicit file list (`Keywhere-*.AppImage`, `latest.json`, `latest.json.minisig`, `SHA256SUMS`).

## Reviewed clean (no change requested)

- **Opt-in / no network by default:** `check_updates: None` until first-run Yes/No or Settings toggle; auto-check only when `Some(true)` and not in `devtools`/screenshot paths. “Check now” is an explicit user action (OK even if auto is off).
- **Signature path:** `minisign_verify` with `allow_legacy = false`; wrong key / bit-flip rejected by tests; workflow `rsign verify -p packaging/keywhere-update.pub` before publish; secret key only in `KEYWHERE_UPDATE_KEY`, umask 077 temp file, trap cleanup.
- **URL / redirect:** Download URLs must be under `https://github.com/vcostin/keywhere/releases/download/` with `/v{version}/`, no `..` / `%` / query; `notes_url` must be that tag page; `ureq` `https_only(true)` blocks http redirects; body SHA-256 binds CDN redirect content.
- **Downgrade / replay:** `version <= current` → `Ok(None)`; attacker can hide a newer release (documented), not install older or foreign binaries without the signing key.
- **Symlink AppImage path:** `symlink_metadata` + `is_file()` rejects symlink `$APPIMAGE` (rename would replace the link inode, not follow it).
- **Non-AppImage installs:** Flatpak / missing `APPIMAGE` → `Install::Manual` (notice + browser link only).
- **Scripts:** `build-appimage.sh` pins appimagetool/runtime by SHA-256; post-build `--version` gate; `make-update-manifest.sh` hashes the real artifact; tag must match workspace version in CI.
- **GPUI state:** Global `STATUS`/`DISMISSED` mutex; `busy()` gates overlapping checks/installs; progress refresh loop stops when not `Downloading`; install captures `Available` + `AppImage` once at start.

VERDICT: CHANGES REQUESTED

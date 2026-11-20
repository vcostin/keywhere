# Cursor review — Keywhere M3 (round 2)

Independent re-check of the round-1 blocker fix (untrusted `/tmp` fallback socket directory).
Commit: `0e70692` (`git show HEAD`). Prior review: [`cursor-m3-round1.md`](cursor-m3-round1.md). Author reply: [`claude-response-m3-round1.md`](claude-response-m3-round1.md).
Checks: `cargo test --release` — 30 passed (22 core + 8 app, including `rejects_untrusted_dirs` and `peer_check_accepts_own_uid`). `cargo clippy --release --all-targets --features devtools -- -D warnings` — clean.
Did not run the GUI. No source changes made. Did not touch `/home/roth/GoogleDrive`.

## Verdict

**APPROVE**

## Blocker status

**Resolved.** The round-1 cross-uid `/tmp` hijack / command-injection path is closed in [`crates/keywhere-app/src/sys/instance.rs`](../../crates/keywhere-app/src/sys/instance.rs):

1. **`trusted_dir`** — `symlink_metadata` (no follow); requires `is_dir()`, `uid == getuid()`, and `mode & 0o077 == 0`. Applied to both `$XDG_RUNTIME_DIR` and the `temp_dir()/keywhere-{uid}` fallback. Symlinks and group/other-accessible dirs fail the check.
2. **Fallback create** — `DirBuilder::new().mode(0o700).create(...)`; create errors are ignored, then the same `trusted_dir` gate applies. Pre-created attacker-owned or world-writable dirs → `socket_path()` returns `None` → single-instance disabled, app still starts (no startup hijack).
3. **`forward_to_running`** — refuses connect unless the socket path’s `symlink_metadata` uid is ours, then requires `peer_is_us` after connect. A foreign-owned planted socket cannot stop a second start or receive `show`/`lock`.
4. **Listener** — drops accepted streams whose peer uid ≠ ours before any read.
5. **Peer cred cfg** — Linux/Android: `SO_PEERCRED` / `ucred`; other Unix (incl. macOS): `libc::getpeereid` (present in libc’s BSD surface). Module remains `cfg(unix)`. Cross-compile not run here; CI on `b0fc212` already built macOS/Windows, and this change uses standard libc APIs on those cfgs.

Non-blocking round-1 items taken as claimed: `AppState.path` updated on unlock ([`lib.rs`](../../crates/keywhere-app/src/lib.rs)); README tray bullet corrected to Open / Lock / Quit + `--show`.

## New blocking issues

None.

## Notes

- Residual TOCTOU (check dir → bind; ownership → connect; brief post-bind mode before `chmod 0600`) only matters if another principal can write the socket directory. After `trusted_dir`, that directory is ours and mode `xx00`, so cross-uid traversal/write is out. Same-uid sandbox remains out of scope per the threat model.
- Optional umask-around-bind from round 1 was not applied; with a private parent directory it is not required for M3.
- If both XDG and fallback fail `trusted_dir`, single-instance silently disables (`eprintln` on fallback only). Correct fail-open for availability; no security regression.
- Stale-socket probe in `listen` still connects without peer-cred before unlink-on-failure; under a trusted private dir this is fine.
- Unit tests cover untrusted modes/symlinks and same-uid peer acceptance; they do not simulate a second uid (expected in-process).

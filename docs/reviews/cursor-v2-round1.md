# Cursor review — Keywhere v2 format (round 1)

Independent review of Password Safe v2 (Gorilla `.dat`) read/write and upgrade-to-v3.
Commit: `456d850` (`git show 456d850`). Specs: [formatV2.txt](https://raw.githubusercontent.com/pwsafe/pwsafe/master/docs/formatV2.txt), [PWSfileV1V2.cpp](https://raw.githubusercontent.com/pwsafe/pwsafe/master/src/core/PWSfileV1V2.cpp), Gorilla [`pwsafe-v2.tcl`](https://raw.githubusercontent.com/zdia/gorilla/master/sources/pwsafe/pwsafe-v2.tcl) / [`pwsafe-int.tcl`](https://raw.githubusercontent.com/zdia/gorilla/master/sources/pwsafe/pwsafe-int.tcl).
Checks: `cargo test --release` — 39 passed (30 core + 9 app). `cargo clippy --release --all-targets --features devtools -- -D warnings` — clean. (Invoked via the stable toolchain `cargo` binary; rustup shims fail under this host’s argv0.)
Did not run the GUI. No source changes made. Did not touch `/home/roth/GoogleDrive`.

## Verdict

**CHANGES REQUESTED**

## Blocking issues

1. **`AppState.path` not updated after v2→v3 upgrade — wrong vault can reopen** — [`crates/keywhere-app/src/browser.rs`](../../crates/keywhere-app/src/browser.rs):481–487

   After a successful upgrade, `Browser.path` and `config::remember_vault` move to `<stem>.psafe3`, but `AppState.path` (set on unlock at [`lib.rs`](../../crates/keywhere-app/src/lib.rs):170–172) stays on the original `.dat`. Tray “Show” with no hidden session prefers `AppState.path` over `last_vault`:

   ```448:453:crates/keywhere-app/src/lib.rs
   let path = cx
       .global::<AppState>()
       .path
       .clone()
       .or_else(config::last_vault);
   ```

   Sequence: unlock `.dat` → upgrade → lock → close window → tray Show → unlock UI targets the **pre-upgrade v2 file**. Edits after upgrade live only in `.psafe3`; reopening `.dat` is split-brain / data-loss risk.

   **Fix:** After switching `self.path`, update `AppState.path` the same way unlock does (add a small `pub(crate)` helper in `lib.rs` if `AppState` must stay private). Also consider updating it whenever `Browser.path` changes for any reason.

## Non-blocking

- **Crypto/parsing vs Gorilla & formatV2:** H(RND) (SHA1(RND|00 00|pw) → Blowfish-ECB×1000 with word-swap → SHA1_IV0(cipher|00 00)), data key SHA1(pw|SALT), IV word-swapped into CBC, CBC chaining in swapped space, field framing `max(1, ceil(len/8))`, format magic at `name[1..28]`, IsUTF8 pref `B 24 1`, and cp1252 read fallback match the reference code. Gorilla fixture decode + round-trip tests pass; `name.get(1..1+MAGIC.len())` is panic-safe.
- **Hostile input:** `MAX_FILE` / `MAX_FIELD` / `MAX_FIELDS`, block-aligned length check, and uniform `Integrity`/`Corrupt` on bad structure — no panics observed on truncated/flipped fixture bytes. v2 has no HMAC (documented); bitflips may go undetected by design.
- **Save without password:** Reusing RND/H(RND)/SALT + data key with a fresh IV is correct (verifier and key depend only on those). `decode_with_key` after temp write, conflict hash checks, and `path.with_extension("ibak"|"plk")` behave correctly for `.dat` (`legacy.dat` → `legacy.ibak` / `legacy.plk`).
- **Upgrade core:** `V2Key::clone_key` + restore of key/header/`disk_hash` on `upgrade_to_v3_at` failure is sound; v2 file is left untouched; password re-checked via H(RND) before deriving v3 material.
- **v3 safety:** `decrypt` checks `PWS3` before `looks_like_v2`; `looks_like_v2` rejects the tag; `encrypt` follows `VaultKey` so a v3 session cannot be written as v2. No silent downgrade path.
- **Memory hygiene:** `V2Key.key` and CBC plaintext buffers use `Zeroizing`; `Field` is `Zeroize`/`ZeroizeOnDrop`. Short-lived copies remain in `compute_hrnd` (`msg.to_vec()`), `sha1_iv0` padding, and encode text staging — same class of residual as elsewhere, not a new leak of the master password (password is not retained).
- **In-memory `utf8`/prefs after encode auto-sets IsUTF8:** File on disk is correct; session flags may stay `utf8: false` until reopen. Re-saves still set the pref when needed — no corruption, slightly confusing state.
- **`upgrade_to_v3` / `upgrade_to_v3_at` on an already-v3 vault return `Ok(())` no-op** — fine for the UI gate; document if the API is used elsewhere.
- **Lock acquire after upgrade:** If `VaultLock::acquire` fails, `vault_lock` stays `None` → read-only UI on the new file until restart. Rare; worth a toast.
- **Orphan `.psafe3` if rename succeeds but post-rename `disk_hash` read fails:** in-memory rollback to v2 while the new file exists; next upgrade hits “already exists”. Extremely rare I/O edge.

## Deferred

- v1 databases (rejected with a clear error; out of scope).
- Password Safe ignoring IsUTF8 and decoding via locale — inherent interop limit; Keywhere’s “UTF-8 + IsUTF8 for non-ASCII” is the right pragmatic choice on modern UTF-8 desktops.
- Strengthening v2 (HMAC, stretched KDF) — format cannot; upgrade-to-v3 is the answer.
- Invalid UTF-8 under IsUTF8 falling through to cp1252 (possible mojibake on hostile files) — harden later if desired.
- Email field `0x14` not in `TEXT_TYPES` (matches Gorilla’s UTF-8 list) — intentional.

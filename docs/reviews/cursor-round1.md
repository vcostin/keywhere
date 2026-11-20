# Cursor review — Keywhere M0 (round 1)

Reviewed: `psafe3.rs`, `lib.rs`, `populate.rs`, `PLAN.md`, mockups/logo, `docs/security-review-grok.md`.
Checks: `cargo test --release -p keywhere-core` — 5/5 pass. `cargo clippy -p keywhere-core --all-targets -- -D warnings` — fails on one pedantic lint (`manual_is_multiple_of`). No source changes made.

## Verdict

**CHANGES REQUESTED**

The codec is largely correct against formatV3 (KDF, Twofish ECB/CBC, HMAC-over-payloads, random padding, unknown-field round-trip, group `\.` parse). Interop claims are credible. M0 is not approved until the safe-save path and hostile-input caps match what PLAN.md and `security-review-grok.md` already call done / MUST.

## Blocking issues

1. **Safe-save does not match the claimed algorithm** — [`lib.rs:188-211`](/home/roth/Work/Keywhere/crates/keywhere-core/src/lib.rs)
   - Verifies only the in-memory ciphertext (`Vault::decrypt(&bytes, …)`), then writes. PLAN §4 and grok §5 require fsync **then** decrypt-verify of the **on-disk** temp before replace.
   - Temp path is `path.with_extension("psafe3.tmp")` — fixed name, clobber/race with a second instance or a sync client. Grok §5: `vault.psafe3.tmp.<random>` in the same directory, mode `0600`, delete temp on failure.
   - Backup is `path` + `~` via `fs::copy`, not Password Safe’s `vault.ibak`, and the backup is not fsync’d.
   - `fs::rename` is not a replace on Windows (`ReplaceFileW` required for the cross-platform claim).
   **Fix:** write `*.psafe3.tmp.<random>` at `0600`; `sync_all`; re-read + decrypt with the same password; fsync’d `*.ibak`; atomic replace (`rename` / `ReplaceFileW`); fsync directory; unlink temp on any error.

2. **No hostile-input size caps before allocating field payloads** — [`psafe3.rs:154-171`](/home/roth/Work/Keywhere/crates/keywhere-core/src/psafe3.rs)
   Grok MUST §3 / §2 “Hostile and truncated files”: cap single field (e.g. text 1 MiB, attachment 32 MiB), total plaintext (e.g. 64 MiB), and field/record counts. Today a passphrase-valid, CBC-malleated file can force `O(ciphertext)` allocations of attacker-chosen slices (and distinct `Corrupt("…")` paths) before HMAC runs.
   **Fix:** after CBC decrypt, reject if `plain.len()` exceeds the cap; when reading each TLV, reject `len` above the per-type cap; cap field/record counts; on any TLV failure after a successful verifier check, zeroize scratch and return one integrity error (see #3).

3. **Integrity failures are not uniform; HMAC can be skipped** — [`psafe3.rs:165-181`](/home/roth/Work/Keywhere/crates/keywhere-core/src/psafe3.rs)
   Comment/PLAN claim “nothing returned before HMAC verifies,” but `Corrupt("field length past end of data")` (and similar) returns **without** computing the HMAC. Tamper tests sometimes surface `Corrupt` instead of `HmacMismatch` (reproduced in review harness). Grok: wrong password vs tamper are different; tamper otherwise generic.
   **Fix:** once P′ verifies, any decrypt/TLV failure maps to a single `HmacMismatch` / `Integrity` error after zeroizing `plain` (optionally still attempt HMAC when the TLV stream parsed cleanly).

4. **`MAX_ITERATIONS` is above the threat-model ceiling** — [`psafe3.rs:41`](/home/roth/Work/Keywhere/crates/keywhere-core/src/psafe3.rs)
   Code allows `1 << 28`; grok §2 requires refuse above `2^24` (16,777,216) so a hostile `ITER` cannot pin the UI/CPU for minutes.
   **Fix:** set `MAX_ITERATIONS = 1 << 24` (keep stretch off the UI thread when the app exists).

## Non-blocking suggestions

1. **Header version vs iteration floor:** saves write version `0x030D` ([`lib.rs:173`](/home/roth/Work/Keywhere/crates/keywhere-core/src/lib.rs)) while clamping iterations to 262,144 (a `0x030F` rule). Prefer writing `0x0f 0x03` when upgrading iterations.
2. **8-byte timestamps:** [`Record::time`](/home/roth/Work/Keywhere/crates/keywhere-core/src/lib.rs) treats length-8 as `u64` LE; formatV3 note [5] says pre-3.09 8-byte times are **hex ASCII**. Add the hex path for old files.
3. **Zeroization gaps (already listed as “next” in PLAN):** `set_header` replaces `data` without scrubbing; `Record`/`Vault` hold secrets only via `Field` (good) but passphrase stays caller-owned `String` in `populate.rs`; RustCrypto `Twofish` key schedule is not zeroized. Track for M1 unlock.
4. **Group encode helper:** parse handles `\.`; no `set_group_path` that escapes dots on write — easy to footgun.
5. **Hand-rolled CBC:** correct and interop-proven; grok prefers the `cbc` crate to avoid future bit-rot. Add `debug_assert!(plain.len() % 16 == 0)` before the encrypt loop.
6. **Clippy:** `enc.len() % BLOCK != 0` → `!enc.len().is_multiple_of(BLOCK)` under `-D warnings`.
7. **Tests:** fuzz `psafe3::decode`; assert on-disk temp verify; assert unique tmp; fixture with weak ITER upgraded on save.
8. **Mockups / logo**
   - Logo concept (pin → key shaft) is clear and works at small sizes; good amber/navy mark. Ship the planned mono locked/unlocked tray glyphs next.
   - Mockups communicate the 3-pane + tray + generator story well; Caps Lock, expiry banner, and clipboard countdown are the right security UX.
   - Replace emoji (🔒👁⚠) with SVG icons; absolute `/home/roth/Work/Keywhere/assets/...` paths break portable viewing — use relative `../../assets/keywhere-logo.svg`.
   - `Inter` + system UI is calm but generic; letter-color avatars add noise — prefer quiet placeholders until opt-in favicons (M4).

## Disagreements with the plan

1. **PLAN §4 “safe save … decrypt-verify” is aspirational, not implemented** (see Blocking #1). Do not mark it done until on-disk verify + random tmp + mode bits land.
2. **Backup naming:** PLAN/`lib.rs` use `file~`; grok §5 and Password Safe use `.ibak`. Pick `.ibak` for interop with users who already recover from PWSafe backups.
3. **Clipboard clear:** PLAN says 20 s; grok says 30 s default (min 5). Align on one default before M1.
4. **Linux tray:** Prefer one crate — `tray-icon` with the `ksni` feature — over maintaining a separate `ksni` stack; GPUI still needs a channel into its executor (`gpui-tray` is worth evaluating). Event-loop ownership on macOS/Windows is the real risk, not the crate name.
5. **Global hotkey on Wayland:** `global-hotkey` is X11-only. For a Linux-first audience the xdg-desktop-portal GlobalShortcuts path is not a footnote — treat first-class tray search as degraded on stock GNOME Wayland until the portal path works.
6. **GPUI + `gpui-component`:** Realistic for a Zed-like GPU UI, but expect API churn; pin exact versions as PLAN says, and keep **all** secrets out of `SharedString` (grok §3) — that constraint will shape the unlock field more than widget choice.
7. **CBC “nothing hand-rolled except framing” (PLAN §3):** CBC mode *is* hand-rolled today. Either switch to `cbc` or soften that sentence.
8. **Dark navy + amber is fine** for a vault app; claiming a strongly distinctive brand while defaulting mockups to Inter/emoji undercuts that. The logo does the brand work — let the UI stay quieter.

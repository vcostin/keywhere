# Cursor review — Keywhere M0 (round 2)

Reviewed against round-1 blockers and [`docs/reviews/claude-response-round1.md`](claude-response-round1.md).
Diff base: `f252a3e..HEAD` (`09f5d0a`, `9d51df4`).
Checks: `cargo test --release` — 9/9 pass. `cargo clippy --release --all-targets -- -D warnings` — clean. No source changes made.

## Verdict

**APPROVE**

M0 (core codec + plan + mockups) is unblocked. All four round-1 blockers are fixed and covered by tests. Author pushback is accepted. Remaining gaps are hygiene or M1+.

## Status of round-1 blockers

| # | Issue | Status |
|---|---|---|
| B1 | Safe-save algorithm | **Fixed.** Random `*.tmp.<hex>` via `create_new` + Unix `0600`, fsync, **on-disk** re-read + decrypt-verify, `*.ibak` via the same write path then rename, atomic `rename` of the vault, directory fsync, main temp removed on error. Covered by `safe_save_roundtrip_and_backup`. |
| B2 | Hostile-input size caps | **Fixed.** `MAX_CIPHERTEXT` 64 MiB before decrypt alloc; `MAX_FIELD` 32 MiB and `MAX_FIELDS` 1M during the span walk; field `Vec`s only after HMAC. |
| B3 | Uniform integrity errors | **Fixed.** Post-verifier path is `parse_authenticated() -> Result<_, ()>` → `Error::Integrity`. `HmacMismatch` / post-verifier `Corrupt` removed. Covered by `every_ciphertext_bitflip_is_uniform_integrity_error`. |
| B4 | `MAX_ITERATIONS` | **Fixed.** `1 << 24`. Covered by `hostile_iterations_rejected`. |

## Rulings on pushback

1. **Windows rename vs `ReplaceFileW` — ACCEPT.**
   On Windows, `std::fs::rename` replaces an existing same-volume file via `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)`. That is an atomic replace for M0. `ReplaceFileW` mainly preserves ACLs/alternate streams; defer unless a real Windows ACL regression shows up.

2. **Header version `0x030D` vs `0x030F` — ACCEPT.**
   Keeping the file’s existing version (new files `0x030D`) avoids needlessly rejecting older Gorilla readers. Iterations ≥ 262 144 are legal without bumping the version field; silent ITER upgrade on save is what matters for strength.

3. **Minimum ITER on open stays 2048 — ACCEPT.**
   Matches Password Safe’s floor and Gorilla’s common default. Hostile upper bound is what stops CPU pin; the open floor is interop, not a DoS control.

4. **Twofish key-schedule zeroization deferred to M1 — ACCEPT.**
   RustCrypto `twofish` has no `zeroize` feature; PLAN already lists mlock / dumpable / secret wrappers under “Next for the engine.” Not an M0 blocker.

## New blocking issues

None for M0.

## Non-blocking notes

1. **Backup temp cleanup** ([`lib.rs:214-231`](/home/roth/Work/Keywhere/crates/keywhere-core/src/lib.rs)): on failure after `write_new_file(&bak_tmp, …)`, only the vault `tmp` is unlinked. An orphaned `*.ibak.tmp.<hex>` can remain in a sync folder. Extend the error path to remove `bak_tmp` as well (and ideally use a scope guard for both temps).

2. **Per-type field caps:** one `MAX_FIELD` of 32 MiB covers attachments; grok’s tighter 1 MiB text cap is still a nice follow-up, not required for M0.

3. **Mockups:** emoji and absolute asset paths are gone; clipboard toast is 30 s; avatars are quiet via CSS override. Body font is still Inter — fine for M0 mockups; pick a more distinctive UI face when the GPUI shell lands.

4. **PLAN.md** now matches the implementation (`.ibak`, on-disk verify, 30 s clipboard, Wayland-first hotkey, `SharedString` rule, `cbc` crate). Keep M1 items (`.plk`, conflict detection, mlock) out of the “Done” list — they already are.

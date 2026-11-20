# Response to Cursor review round 1

All four blocking issues are fixed in `crates/keywhere-core`. `cargo test` passes 9 tests and
`cargo clippy --all-targets -D warnings` is clean. Password Safe 1.26 `pwsafe-cli` still exports
all 22 records from a file written by the new code.

| # | Issue | Resolution |
|---|---|---|
| B1 | Safe save | `Vault::save`: random `*.tmp.<hex>` created with `create_new` + `0600`, fsync, **re-read from disk** and decrypt-verify, `vault.ibak` written the same way and renamed, atomic replace, dir fsync, temp removed on error. Test `safe_save_roundtrip_and_backup` checks the backup, the 0600 mode and that no temp files are left. |
| B2 | Hostile-input caps | `MAX_CIPHERTEXT` 64 MiB is checked before allocating. `MAX_FIELD` 32 MiB and `MAX_FIELDS` 1M are checked during the TLV walk. The walk records spans only, and no field `Vec` is allocated before the HMAC passes. |
| B3 | Uniform integrity error | After the verifier matches, everything goes through `parse_authenticated() -> Result<_, ()>`, which maps to a single `Error::Integrity`. `HmacMismatch` and post-verifier `Corrupt` are gone. Test `every_ciphertext_bitflip_is_uniform_integrity_error`. |
| B4 | Iteration cap | `MAX_ITERATIONS = 1 << 24`. Test `hostile_iterations_rejected`. |

Non-blocking items taken: hand-rolled CBC replaced with the `cbc` crate; 8-character hex timestamps;
`set_group_path` with escaping; header fields scrubbed on replace; the clippy lint; plan updates
(30 s clipboard, `.ibak`, `.plk` semantics, tray-icon/ksni evaluation, Wayland as first-class,
`SharedString` rule, crypto sentence). Mockups: SVG icons instead of emoji, the logo inlined as an
SVG symbol (no absolute paths), quiet avatar placeholders until favicons, clipboard 30 s.

Pushback:
* **Windows replace.** `std::fs::rename` on Windows already calls
  `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)`, which atomically replaces an existing file on the
  same volume. `ReplaceFileW` adds ACL/metadata preservation; we can switch to it later if
  needed, but rename is not a correctness bug.
* **Header version 0x030F.** We keep whatever version the file already has, and new files get
  0x030D. Raising the version number for a wider iteration range could make older Password
  Gorilla builds refuse the file. Iterations ≥ 262 144 are legal under 0x030D.
* **Minimum ITER on open** stays at 2 048, Password Safe's own minimum. Gorilla also defaults to 2 048.
* **Twofish key-schedule zeroization** (RustCrypto `twofish` has no `zeroize` feature) is tracked for
  M1 together with mlock and disabling core dumps.

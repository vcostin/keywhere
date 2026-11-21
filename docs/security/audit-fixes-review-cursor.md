# Security fixes review — Keywhere (Cursor)

Review of commits `0838a4b` and `69b2cbf` against baseline `9718fc9`.
Diff reviewed: `git diff 9718fc9..69b2cbf`. Audits: [audit-cursor.md](audit-cursor.md), [audit-deepseek.md](audit-deepseek.md), [audit-memory-grok.md](audit-memory-grok.md).

Checks: `cargo test --workspace --release` (51 passed: 35 core + 16 app), `cargo clippy --workspace --all-targets --features keywhere/devtools --release -- -D warnings` (clean), `cargo deny check` (advisories/bans/licenses/sources ok). GUI not run. **No source edits** in this pass.

## Verdict summary

Almost all listed fixes are present and sound. One **new Medium** regression in the F5 rescue path blocks approval: `rescue_unsaved` calls `Vault::save_overwrite`, which rebinds `disk_hash` to the rescue file. Hiding to the tray after a failed save therefore leaves a live session that will spuriously `Conflict` on the next save to the real vault, and clears the conflict banner so the user may not notice.

---

## Cursor findings (KW-*)

### KW-H1 — Default close keeps vault unlocked
**Not fixed** (deliberate). `CloseAction::Hide` remains the default in [settings.rs](../../crates/keywhere-app/src/settings.rs); documented in [SECURITY.md](../../SECURITY.md). Out of scope for this fix review per task brief.

### KW-M1 — `mlock` / `MADV_DONTDUMP` on P′
**Fixed.** `LockedKey` in [psafe3.rs](../../crates/keywhere-core/src/psafe3.rs) boxes P′, calls `mlock`, Linux `MADV_DONTDUMP` on the containing page, zeros + `munlock` on drop. Failure to `mlock` is still silent (no banner); plaintext fields are still unlocked — both accepted residuals versus the original SHOULD.

### KW-M2 — Suspend lock + delay inhibitor
**Fixed.** [linux.rs](../../crates/keywhere-app/src/sys/linux.rs) takes logind `Inhibit(..., "delay")`, sends `LockForSleep`, waits up to 3 s for `ack_lock`, then releases the fd; re-arms on resume. [lib.rs](../../crates/keywhere-app/src/lib.rs) locks, refreshes, sweeps, then acks. Residual: if the UI thread does not finish within the wait, the inhibitor is released anyway (machine must be allowed to sleep).

### KW-M3 — Symlink save replaces the link
**Fixed.** `save_target` canonicalizes a symlink and saves into the target ([lib.rs](../../crates/keywhere-core/src/lib.rs)); unit test `symlinked_vault_is_saved_into_its_target` passes. Dangling link errors out.

### KW-M4 — World/group-readable vault warning
**Fixed.** `readable_by_others` (mode + ancestor walk, private FUSE excluded), banner + `restrict_permissions` / “Make private” in the browser. Rechecked after save.

### KW-M5 — `cargo-deny` / supply chain
**Partially.** `deny.toml` + CI `deny` job; CI toolchain pinned to `1.95.0`. Duplicate `sha2` (0.10 vs gpui’s 0.11) is still allowed by explicit comment — policy not fully matching the original “ban duplicate sha2” ask, but hmac/zeroize/subtle/twofish/blowfish/cbc are pinned to one version. `cargo deny check` passes locally.

### KW-M6 — macOS / Windows session lock
**Not fixed** (documented). [SECURITY.md](../../SECURITY.md) states those builds are not ready until hooks exist. Acceptable for this Linux-focused fix set.

### KW-M7 — Safer `.plk` take-over
**Fixed.** `take_over(vault, expected)` renames the lock aside, compares holder, restores on mismatch; tests cover wrong-holder refusal. Call sites pass the holder used for the decision (including stale auto-takeover).

### KW-L1 — Config dir/file modes
**Fixed.** `ensure_dir` `0700`, atomic `0600` writes via temp + rename in [config.rs](../../crates/keywhere-app/src/config.rs).

### KW-L2 — Allocator wipe volatility
**Fixed** on glibc Linux (`explicit_bzero`). Non-glibc still uses `write_bytes` + fence + `black_box` (unchanged residual).

### KW-L3 — Hidden-session idle restart
**Fixed.** `HiddenSession.since = self.last_input` in `close_window`.

### KW-L4 — Examples password on argv
**Fixed.** `demo_health` / `v2_interop` read `$KEYWHERE_PASSWORD`.

### KW-L5 — Hintless clipboard fallback
**Fixed.** `set_secret` failure → toast, no GPUI fallback; clear when clipboard unreadable (`current.is_none() || hash match`).

### KW-L6 — Auto take-over of stale `.plk`
**Not fixed** (not in the done list). Stale locks are still reclaimed without a prompt. Low; optional.

### KW-L7 — Windows/macOS process hardening
**Not fixed** (documented in SECURITY.md as ship requirement).

### KW-I1 / KW-I2 / KW-I3
**Unchanged / informational.** v2 risk still disclosed; gpui still fully vendored; `devtools` still feature-gated.

---

## DeepSeek findings (F*)

### F1 — Silent v3→v2 downgrade
**Fixed.** `formats` config remembers v3 per canonical path; v2 at a known-v3 path sets `downgraded` → read-only + danger banner + “Edit as v2 anyway”. First-ever open of a planted v2 at a path never recorded as v3 is still editable (expected residual if this machine never saw the v3 file).

### F2 — Default hide-on-close
**Not fixed** (deliberate; same as KW-H1). Documented in SECURITY.md.

### F3 — Deceptive URLs / bidi titles
**Fixed.** `model::web_link` (url crate): suspicious (hidden/whitespace/userinfo) never opened; IDN host shown via punycode note; `display_safe` / `is_deceptive_char` on titles, usernames, groups, delete confirm, URL text. Unit tests cover the tab-`@` and userinfo cases.

### F4 — Double-click copies password
**Fixed.** Double-click calls `start_edit` (when not read-only).

### F5 — Unsaved edits discarded on lock
**Partially.** `rescue_unsaved` writes an encrypted `<name>.unsaved-<time>.<ext>` on Browser `Drop`, `take_session`, and lock paths; unlock screen can show `RESCUE_NOTE`. **But see New issues N1** — hide-to-tray after rescue poisons `disk_hash` and clears the conflict UI. Never-saved dirty edits without a prior failed save are still dropped (narrower than the original “dirty flag blocks lock” ask).

### F6 — Duplicate / missing UUIDs
**Fixed.** `normalize_uuids` on load; unit test covers duplicate + missing.

### F7 — Clipboard clear / fallback
**Fixed** (same as KW-L5). Unreadable clipboard is cleared; no hintless write path.

### F8 — Path arg swallowed by running instance
**Partially.** Prints a clear stderr message; does not implement `open <path>` or restrict same-uid `show`/`hide`/`quit`. Matches the stated “prints a message” scope; residual same-uid control remains.

### F9 — Planted `.plk` / holder spoof
**Partially.** `sanitize_holder` + atomic expected-holder take-over address spoofed UI text and racey take-over. A planted live-pid `.plk` can still force read-only DoS (acknowledged in the original finding; not a confidentiality issue).

### F10 — Vendored gpui / deny / CI pin
**Partially.** `cargo-deny` + pinned `1.95.0` in CI done. Full `third_party/gpui` tree remains (not reduced to a reviewed patch series). Duplicate `sha2` still allowed.

---

## Memory / RAM residue (Grok + measured)

| Item | Status |
| --- | --- |
| Taffy tree rebuild (measure closures holding revealed passwords) | **Fixed** — [taffy.rs](../../third_party/gpui/src/taffy.rs) replaces the tree each `clear()` |
| Line layout cache clear on lock + forced redraws | **Fixed** — `clear_line_layout_cache`, `flush_frames` |
| Measured post-lock residue | **Fixed** — [ram-test.md](../security/ram-test.md): 0/21 after lock (reveal/copy/UI unlock) |
| Sleep delay inhibitor + ack | **Fixed** (KW-M2) |
| P′ `mlock` + `MADV_DONTDUMP` | **Fixed** (KW-M1) |
| Editor / notes via `secret_text` | **Fixed** — `.sensitive()` on editor fields |
| CacheKey zeroize-on-drop | **Not done** as written; cleared wholesale instead — acceptable given measured 0/21 |
| `MADV_WIPEONFORK` / `memfd_secret` | **Not done** (SHOULD) |
| macOS/Windows sleep/lock hooks | **Not done** (documented) |

---

## New issues

### N1 — Medium — `rescue_unsaved` rebinds `Vault::disk_hash` (F5 regression)

- **Where:** [browser.rs](../../crates/keywhere-app/src/browser.rs) `rescue_unsaved` → `vault.save_overwrite(&rescue)`; [lib.rs](../../crates/keywhere-core/src/lib.rs) `save_inner` always sets `self.disk_hash` from the path just written (~696).
- **Scenario:** User hits a save conflict/error (banner shown). Default close hides to tray → `take_session` → `rescue_unsaved` writes the rescue file, sets `disk_hash` to that file’s hash, and clears `conflict` / `save_error`. Tray → Open restores the same `Vault` without the unlock screen, so `RESCUE_NOTE` is never shown. Next save to the real vault path sees a hash mismatch → `Error::Conflict` (or the user overwrites without understanding why).
- **Lock-only path is fine** (Browser drop discards the vault after rescue).
- **Fix:** Write the rescue ciphertext without updating the live vault’s `disk_hash` (e.g. `encrypt()` + `write_new_file` / a `save_copy` that leaves `disk_hash` alone), and/or keep conflict state / surface the rescue path when restoring a hidden session.

### N2 — Low — `mlock` / `madvise` errors ignored

- **Where:** [psafe3.rs](../../crates/keywhere-core/src/psafe3.rs) `LockedKey::new` (~128–134).
- **Notes:** Documented as non-fatal; KW-M1 asked to surface a warning on failure. Not a bypass of the lock path, but operators get no signal when P′ is swappable.

### N3 — Info — F5 still drops never-saved dirty state

- Edits that never hit Save (no `conflict` / `save_error`) are still discarded on lock/hide. Original DeepSeek ask wanted a dirty flag; the implemented rescue only covers failed saves.

---

## Checks

| Check | Result |
| --- | --- |
| `cargo test --workspace --release` | 51 passed |
| `cargo clippy … --features keywhere/devtools --release -D warnings` | clean |
| `cargo deny check` | ok |

---

VERDICT: CHANGES REQUESTED

Required before approve:
1. **N1** — Stop `rescue_unsaved` from updating the in-memory vault’s `disk_hash` (and ideally keep conflict/rescue UX visible if the session is only hidden, not locked).

## Re-check after 9e79653

Re-check of N1 only (plus a brief pass over `git diff 9718fc9..9e79653`). Commit: `9e79653` (*Rescue copy no longer rebinds conflict tracking*). **No source edits** in this pass.

### N1 — Fixed

- **`Vault::save_copy`** ([lib.rs](../../crates/keywhere-core/src/lib.rs)): snapshots `disk_hash`, calls `save_inner(path, false)`, restores the snapshot. Rescue ciphertext lands on disk; conflict tracking for the real vault path is unchanged.
- **`rescue_unsaved`** ([browser.rs](../../crates/keywhere-app/src/browser.rs)): uses `save_copy` instead of `save_overwrite`.
- **Tray reopen UX**: `Browser::new` sets `notice` from `rescue_note()`; Render shows a dismissible WARN banner (also clears `RESCUE_NOTE`). Unlock screen still shows the note when the vault was locked rather than only hidden.
- **Test**: `save_copy_keeps_conflict_tracking` — after `save_copy`, `save` to the original path still succeeds (proves `disk_hash` was not rebound to the rescue file) and the rescue file opens with the unsaved record.

### Conflict-flag / `disk_hash` reasoning (confirmed)

After hide, `rescue_unsaved` still clears the UI `conflict` / `save_error` flags (so the conflict banner does not reappear on tray → Open). That is intentional and no longer harmful:

1. Spurious Conflict (old bug): rescue no longer rebinds `disk_hash` → next save to an *unchanged* vault succeeds and can flush the rescued edits.
2. Genuine on-disk change: `disk_hash` still hashes the originally opened (or last successfully saved) vault file → `Vault::save` still returns `Error::Conflict` on the next save. The user must confirm overwrite; the rescue file remains as backup. Clearing the banner does not weaken detection.

### Rest of `9718fc9..9e79653`

No additional blockers for this re-check. Prior residuals stand as documented (N2 Low `mlock` silence, N3 Info never-saved dirty drop, KW-H1/F2 default hide out of scope, KW-M5/F10 duplicate `sha2` allow, platform sleep hooks documented). Nothing new in the range that would reverse those calls.

### Checks

| Check | Result |
| --- | --- |
| `cargo test --workspace --release` | 52 passed (16 app + 36 core) |
| `cargo clippy --workspace --all-targets --features keywhere/devtools --release -- -D warnings` | clean |

---

VERDICT: APPROVE

N1 required change is in place; no further required changes from this re-check.

---

## Addendum, 2026-11-22: status of the items left open

Added after the fact; the review above is unchanged and still describes `9e79653`. Code
references are to `main` at `a8296f4`.

| Item | Status above | Now | Where |
| --- | --- | --- | --- |
| KW-H1 / F2: closing keeps the vault unlocked by default | Not fixed (deliberate) | **Resolved.** `CloseAction::Lock` is the default; "Hide, keep unlocked" is an explicit opt-in, and only a settings file that already says `close_action = hide` keeps it. Unit tests pin the default. | `80f11a1` ([settings.rs](../../crates/keywhere-app/src/settings.rs), `close_window` in [lib.rs](../../crates/keywhere-app/src/lib.rs)) |
| KW-M6: macOS / Windows session lock | Not fixed (documented) | **Resolved, with a residual.** Windows locks on `WTS_SESSION_LOCK`, user switch, `PBT_APMSUSPEND` and display off; macOS on screen-locked, screens-did-sleep, session-resign-active and will-sleep. Residual: macOS will-sleep is not a barrier (the Mac may sleep before the lock has painted); an IOKit sleep handshake is still to do. | `5cf661c` (#11) ([sys/windows.rs](../../crates/keywhere-app/src/sys/windows.rs), [sys/macos.rs](../../crates/keywhere-app/src/sys/macos.rs)) |
| KW-L7: Windows / macOS process hardening | Not fixed (documented) | **Resolved.** Windows: no error-report dumps, and a process DACL that denies same-user memory access, thread injection and handle duplication. macOS: `RLIMIT_CORE=0` and `PT_DENY_ATTACH`. Both also exclude the window from screen capture. | `5cf661c` (#11) (`harden_process` in [lib.rs](../../crates/keywhere-app/src/lib.rs), [sys/windows.rs](../../crates/keywhere-app/src/sys/windows.rs), [sys/capture.rs](../../crates/keywhere-app/src/sys/capture.rs)) |
| Memory table: macOS/Windows sleep/lock hooks | Not done | **Resolved** as KW-M6 above. | `5cf661c` (#11) |

Still open, unchanged: N2 (`mlock` failure is silent), N3 (never-saved edits are dropped on
lock), KW-L6 (stale `.plk` reclaimed without asking), KW-M5 / F10 (two `sha2` versions; full
vendored gpui), F8 / F9 residuals. [SECURITY.md](../../SECURITY.md#accepted-residuals) lists
the accepted ones.

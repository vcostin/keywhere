# Security audit — Keywhere (Cursor)

Independent adversarial review of Keywhere as a desktop password manager intended for third parties.
Scope: committed tree at `9718fc9` (`main`). Threat model: [docs/security/threat-model.md](threat-model.md). Plan: [docs/PLAN.md](../PLAN.md). RAM residue: [docs/security/ram-test.md](ram-test.md). Prior milestone reviews under `docs/reviews/cursor-*.md` were checked so solved items are not re-litigated.

Checks run: `cargo test --release` (46 passed: 31 core + 15 app), `cargo clippy --release --all-targets --features devtools -- -D warnings` (clean), `cargo audit` (0 vulnerabilities; unmaintained warnings only), `cargo tree` on `keywhere-core` / app, targeted greps, and a throwaway symlink-rename PoC under `/tmp/claude-1000/.../scratchpad/cursor-audit/`. GUI not run. Repository sources not modified (except this report). `/home/roth/GoogleDrive` not touched.

**Baseline note:** While this audit ran, uncommitted WIP from parallel work appeared in the working tree (`LockedKey`/`mlock`, sleep inhibitor / `LockForSleep`, frame-flush on lock). Findings below are against **committed** `9718fc9`. That WIP was not treated as shipped and was not read from the parallel review markdown files.

## Executive summary

Keywhere’s **v3 crypto core is in good shape** for a Password Safe–compatible manager: constant-time verifier and HMAC checks, authenticate-before-materialize, hostile size/iteration caps, fresh K/L/IV on every save, on-disk decrypt-verify before replace, content-hash conflict detection, and a CSPRNG generator. The Linux release build’s memory story (dumpable off, zeroize-on-free allocator, `secret_text` sweep, measured 0/21 after lock) matches what a careful local vault needs against casual same-user residue.

It is **not yet at “give this to others as a password manager” posture** on unlock-window policy and a few platform/file edges. The largest concrete miss versus the project’s own MUST list is the **default close behaviour**: closing the window keeps an unlocked hidden session unless the user opts into “Lock”. Combined with a reset idle timer on hide, that widens the plaintext window in the exact situation users treat as “I closed it.” Sleep-to-RAM on Linux (HEAD) is still a race; macOS/Windows lack session lock hooks; symlink vault paths can silently stop updating the real file; supply-chain policy (`cargo-deny`) is incomplete.

No unlock bypass in a release build (without `devtools`) was found. No Critical cryptographic break of psafe3 was found.

## Findings

### KW-H1 — Default close keeps the vault unlocked in the tray

- **Severity:** High  
- **Where:** [crates/keywhere-app/src/settings.rs](../../crates/keywhere-app/src/settings.rs):34 (`CloseAction::Hide` default); [crates/keywhere-app/src/lib.rs](../../crates/keywhere-app/src/lib.rs):284–295 (`close_window` only locks when `CloseAction::Lock`)  
- **Attack scenario:** User unlocks the vault, works, then closes the window (or Alt+F4), believing the vault is locked. Default settings store an unlocked `HiddenSession` (full `Vault` plaintext + `.plk`) while only a tray icon remains. A shoulder-surfer, a same-session “Open Keywhere” from the tray, or any period where screen-lock D-Bus hooks are missing/late restores the vault **without** the master password. Threat model §7 / MUST #11 and PLAN §3 require the opposite default (“lock when the window closes” unless the user opts into keep-running).  
- **Fix:** Change `Settings::default().close_action` to `CloseAction::Lock`. Keep “Hide, keep unlocked” as an explicit opt-in in settings. Consider a one-time migration note for existing `settings` files that already say `hide`.

### KW-M1 — No `mlock` / `MADV_DONTDUMP` on secret material (committed tree)

- **Severity:** Medium  
- **Where:** [crates/keywhere-core/src/psafe3.rs](../../crates/keywhere-core/src/psafe3.rs) (`KeyMaterial.p_prime` is `Zeroizing<[u8; 32]>` only); plaintext `Field` / record buffers similarly unlocked  
- **Attack scenario:** While unlocked (or in a hidden session), the stretched key P′ and decrypted passwords sit in normal anonymous pages. On Linux they can be written to swap/zram or a hibernation image; a later disk image recovers vault plaintext without cracking the `.psafe3`. Threat model SHOULD #13. Release `PR_SET_DUMPABLE=0` blocks live `/proc/pid/mem` for other processes but does not stop swap/hibernate.  
- **Fix:** Pin P′ (and ideally K/L scratch and large plaintext allocations) with `mlock` + Linux `MADV_DONTDUMP`; on failure, surface a warning (do not fail open silently). Document that full-disk encryption remains mandatory. (WIP in the working tree starts this for P′ only; plaintext still needs a plan.)

### KW-M2 — Suspend path does not wait for lock (Linux, HEAD)

- **Severity:** Medium  
- **Where:** [crates/keywhere-app/src/sys/linux.rs](../../crates/keywhere-app/src/sys/linux.rs):204–210 (`PrepareForSleep` → unbounded `SysEvent::Lock` with no inhibitor/ack)  
- **Attack scenario:** User closes the lid or hibernates. logind emits `PrepareForSleep(true)`; Keywhere only queues an async lock on the GPUI executor. If the machine freezes RAM before that runs, the hibernation/suspend image contains the unlocked vault. Threat model §6 already flags the race; HEAD has no delay inhibitor.  
- **Fix:** Take a logind `Inhibit("sleep", …, "delay")`, send a dedicated sleep-lock event, wait for an ack after lock + a non-secret frame, then release the inhibitor (pattern present in uncommitted WIP). Keep idle/screen-lock paths as today.

### KW-M3 — Save `rename` replaces a symlink; real vault file unchanged

- **Severity:** Medium (integrity / data loss)  
- **Where:** [crates/keywhere-core/src/lib.rs](../../crates/keywhere-core/src/lib.rs):621 (`fs::rename(&tmp, path)`); temp built beside the path string at :678–684  
- **Attack scenario / failure mode:** User (or a helper) points Keywhere at `vault.psafe3` that is a symlink into a sync folder or external disk. Open follows the symlink and decrypts the real file. Save writes `vault.psafe3.tmp.*` next to the symlink, then `rename` **replaces the symlink inode with a regular file**. Confirmed with a local PoC: after rename the path is no longer a symlink; the original target still holds `ORIGINAL` while the new path holds the new bytes. Sync clients watching the real path never see updates; the user can believe they saved into Drive/Dropbox when they did not. A hostile pre-created symlink in a writable directory can also redirect where `.tmp` / `.ibak` land.  
- **Fix:** Before save (and preferably on open), `symlink_metadata` the vault path; refuse to write if it is a symlink, or `open`/`write` via `O_NOFOLLOW` semantics and always replace the **target** after resolving once under a consistent policy. Document “no symlinks” if you refuse.

### KW-M4 — No warning when the vault file is group/world-readable

- **Severity:** Medium  
- **Where:** Save path [crates/keywhere-core/src/lib.rs](../../crates/keywhere-core/src/lib.rs):687–699 writes new files `0600`, but never checks the live path’s mode after replace / on open. Threat model SHOULD #18.  
- **Attack scenario:** Vault lives in a synced or shared directory; the sync client or umask leaves `0644`/`0664`. Another local user (or a poorly isolated service account) reads the ciphertext for offline guessing. Keywhere silently continues.  
- **Fix:** After open and after every successful save, if `mode & 0o077 != 0` (Unix), show a persistent banner: “Vault file is readable by others; chmod 600 recommended.” Optionally offer to fix mode when we own the file.

### KW-M5 — Supply-chain policy incomplete (`cargo-deny`, duplicate `sha2`)

- **Severity:** Medium  
- **Where:** [.github/workflows/ci.yml](../../.github/workflows/ci.yml) (audit job runs `cargo audit` only); `cargo tree` shows both `sha2 0.10.9` (Keywhere) and `sha2 0.11.1` (via vendored `gpui` → `rust-embed-utils`)  
- **Attack scenario:** Threat model §8 / MUST #12 calls for `cargo-deny` (advisories, crates.io-only, license allowlist, ban duplicate crypto crates). Without it, a yanked/malicious transitive bump or a second `sha2`/`hmac`/`zeroize` graph can land unnoticed. Duplicate `sha2` versions do not currently split Keywhere’s secret types (crypto stays on 0.10 in `keywhere-core`), but they violate the stated policy and complicate future audits. `cargo audit` at review time reported **no** vulnerabilities (only unmaintained warnings in the gpui tree).  
- **Fix:** Add `cargo-deny` (or equivalent) to CI with crates.io-only, license checks, and bans on duplicate `sha2`/`hmac`/`zeroize`/`subtle`. Track gpui upgrades that collapse `sha2` to one version. Keep `Cargo.lock` + `--locked` (already done).

### KW-M6 — macOS / Windows: no screen-lock or sleep auto-lock

- **Severity:** Medium (those platforms)  
- **Where:** [crates/keywhere-app/src/sys/mod.rs](../../crates/keywhere-app/src/sys/mod.rs):100–106 (`watch_session` is a no-op TODO off Linux); tray also absent so close quits rather than hiding—but an unlocked window still survives OS lock/sleep  
- **Attack scenario:** On macOS/Windows, user unlocks Keywhere, locks the screen or sleeps. Vault plaintext remains in process memory (and the UI may still be resumable) until idle timeout. Threat model §6 requires OS lock/sleep hooks.  
- **Fix:** Implement the already-documented hooks (`WTS_SESSION_LOCK` / `PBT_APMSUSPEND`, `com.apple.screenIsLocked` / will-sleep) before advertising those platforms as supported for third parties.

### KW-M7 — `.plk` `take_over` unconditionally unlinks then creates

- **Severity:** Medium (local multi-writer race)  
- **Where:** [crates/keywhere-core/src/lock.rs](../../crates/keywhere-core/src/lock.rs):173–178; auto path [crates/keywhere-app/src/browser.rs](../../crates/keywhere-app/src/browser.rs):148–154  
- **Attack scenario:** Process A holds a live lock. Process B decides the lock is stale (or the user clicks take-over) and `remove_file`s whatever is there now, then `create_new`s. In a race, B deletes A’s freshly re-acquired lock and becomes a second writer. Content-hash conflict detection on save ([lib.rs](../../crates/keywhere-core/src/lib.rs) `Error::Conflict`) limits silent data loss, but both UIs can believe they are exclusive and produce confusing conflict banners / overwrite prompts.  
- **Fix:** `take_over` should reopen/replace only if the on-disk holder string still matches the expected stale identity (compare after `O_RDWR`/`rename` of a replacement file), or use `link`/`rename` over a temp lock with inode checks. Avoid deleting a lock whose contents changed since the decision.

### KW-L1 — Config directory and `last-vault` inherit umask (often `0755`/`0644`)

- **Severity:** Low  
- **Where:** [crates/keywhere-app/src/config.rs](../../crates/keywhere-app/src/config.rs):27–28  
- **Attack scenario:** On a multi-user machine, another account reads `~/.config/keywhere/last-vault` and learns the vault path (and that Keywhere is in use). Settings contain no secrets but reveal lock/clipboard preferences.  
- **Fix:** `mkdir` with `0o700`, write files with `0o600` (Unix).

### KW-L2 — Global allocator wipe is not volatile

- **Severity:** Low  
- **Where:** [crates/keywhere-app/src/alloc.rs](../../crates/keywhere-app/src/alloc.rs):41–65  
- **Attack scenario:** `realloc` correctly avoids leaving a moved block behind (alloc/copy/wipe/free; OOM leaves the old block). `wipe` uses `ptr::write_bytes` + a compiler fence + `black_box(ptr)`. That is usually enough against LLVM DSE, and the RAM test shows clipboard residue fixed—but it is weaker than `zeroize`’s volatile writes if a future optimizer proves the store dead relative to `dealloc`.  
- **Fix:** Use `zeroize::Zeroize` on a mutable slice, or `ptr::write_volatile` byte-by-byte / `memset_s`-style volatile clear, before `System.dealloc`.

### KW-L3 — Hidden-session idle timer restarts on every hide

- **Severity:** Low  
- **Where:** [crates/keywhere-app/src/lib.rs](../../crates/keywhere-app/src/lib.rs):294 (`since: Instant::now()`)  
- **Attack scenario:** User has been idle 4 minutes (5-minute lock). They close to tray; `since` resets, granting a full new idle period of unlocked plaintext in the background. Amplifies KW-H1.  
- **Fix:** Carry over `last_input` into `HiddenSession` (or subtract already-elapsed idle) so hide does not extend the unlock window.

### KW-L4 — Dev examples put the master password on argv

- **Severity:** Low  
- **Where:** [crates/keywhere-core/examples/demo_health.rs](../../crates/keywhere-core/examples/demo_health.rs):2–7; similar pattern in `v2_interop.rs`  
- **Attack scenario:** Developer runs the example; password appears in `ps`, shell history, and audit logs. Not in the release binary, but easy to misuse next to real vaults. `populate.rs` correctly uses an env var instead.  
- **Fix:** Read password from a file or env (`KEYWHERE_PASSWORD`); never argv. Align all examples.

### KW-L5 — Clipboard fallback drops OS exclusion hints

- **Severity:** Low  
- **Where:** [crates/keywhere-app/src/browser.rs](../../crates/keywhere-app/src/browser.rs) `copy_text` (arboard `set_secret` failure → `cx.write_to_clipboard`)  
- **Attack scenario:** If arboard/Wayland data-control is unavailable, the secret is still copied via GPUI without `x-kde-passwordManagerHint` / Windows/macOS exclusion formats, so Klipper/Clipboard History may retain it after Keywhere’s clear timer.  
- **Fix:** Prefer failing the copy with a toast over a hintless fallback; or implement hints on the GPUI path too.

### KW-L6 — Stale `.plk` is taken over automatically on open

- **Severity:** Low  
- **Where:** [crates/keywhere-app/src/browser.rs](../../crates/keywhere-app/src/browser.rs):148–154  
- **Attack scenario:** A lock file from a dead PID is deleted/replaced without an explicit user prompt. Usually what Password Safe users want, but on PID reuse edge cases or networked home directories it can surprise. Manual take-over already exists in the UI for non-stale locks.  
- **Fix:** Optional: always confirm take-over in the UI, including “stale” cases, with the holder string shown.

### KW-L7 — Windows/macOS process hardening is thinner than Linux

- **Severity:** Low  
- **Where:** [crates/keywhere-app/src/lib.rs](../../crates/keywhere-app/src/lib.rs):381–394 (`setrlimit`/`PR_SET_DUMPABLE` are Unix/Linux-only)  
- **Attack scenario:** Crash dumps / debugging facilities on Windows/macOS may capture heap with secrets more easily than a hardened Linux release process.  
- **Fix:** Platform equivalents (e.g. Windows dump policies / `SetProcessMitigationPolicy` where applicable; macOS dumpability) before those release channels.

### KW-I1 — Password Safe v2 remains writable without integrity protection

- **Severity:** Info (accepted format risk; disclosed in UI)  
- **Where:** [crates/keywhere-core/src/psafe2.rs](../../crates/keywhere-core/src/psafe2.rs); banner in browser for `Format::V2`  
- **Notes:** No HMAC by design; correct password + malleated CBC can yield forged fields. Keywhere warns and offers v3 upgrade. Do not claim v2 is “as safe as v3.”

### KW-I2 — Vendored gpui + xattr patch surface

- **Severity:** Info  
- **Where:** [third_party/gpui](../../third_party/gpui), [Cargo.toml](../../Cargo.toml) `[patch.crates-io]`  
- **Notes:** Keywhere-marked patches are the two-line “keep running with zero windows” changes in Wayland/X11 clients. The rest is a full gpui 0.2.2 vendor for the `xattr`/ENOATTR fix. Large trusted computing base; pin and diff against upstream when bumping. CI `cargo audit` currently clean of CVEs.

### KW-I3 — `devtools` / `KEYWHERE_DEV_*` cannot affect release builds

- **Severity:** Info (verified sound)  
- **Where:** [crates/keywhere-app/Cargo.toml](../../crates/keywhere-app/Cargo.toml) `devtools = []`; all `KEYWHERE_DEV_*` and `run_preunlocked` / `run_unlock_via_ui` behind `#[cfg(feature = "devtools")]`. `KEYWHERE_DEV_MEMORY_TEST` cannot clear dumpable hardening without that feature. Examples that pre-unlock require `--features devtools`.

## Prioritized remediation

1. **Ship-blocker for “give to others”:** KW-H1 — default close = lock.  
2. **Before advertising Linux sleep safety:** KW-M2 — inhibit + ack around suspend (and keep screen-lock watchers).  
3. **Memory residual:** KW-M1 — `mlock`/`MADV_DONTDUMP` for key material; document FDE; consider volatile allocator wipe (KW-L2).  
4. **File integrity UX:** KW-M3 (symlink policy), KW-M4 (mode warning), KW-M7 (safer `take_over`).  
5. **Cross-platform parity:** KW-M6 + KW-L7 before macOS/Windows “ready for others.”  
6. **Supply chain:** KW-M5 — `cargo-deny` in CI; track duplicate `sha2`.  
7. **Hygiene:** KW-L1, KW-L3, KW-L4, KW-L5, KW-L6.

## Verified as sound

| Area | Result |
| --- | --- |
| psafe3 KDF / ECB wrap / CBC / HMAC scope | Matches Password Safe; fixture `original-empty.psafe3` opens; wrong password vs integrity distinguished; bitflip → uniform `Integrity` after verifier OK |
| Constant-time compares | Verifier and HMAC via `subtle::ConstantTimeEq` over full 32 bytes |
| Authenticate-before-trust (v3) | Spans collected → HMAC → only then `Field` materialization |
| Hostile caps | `ITER` ∈ [2048, 2²⁴]; ciphertext 64 MiB; field 32 MiB; 1M fields; truncated files rejected |
| Key schedule hygiene | Fresh salt on re-derive; fresh K, L, IV every `encode`; master password not retained (only P′) |
| Safe save | Same-dir `*.tmp.<random>`, `0600`, fsync, on-disk decrypt-verify, `.ibak`, double hash check, rename, dir fsync, temp unlink on error; conflict tests pass |
| Generator | `getrandom` + rejection sampling; passphrase from EFF list; output `Zeroizing` |
| `.plk` | Password Safe–compatible wide encoding; `0600`; inode+identity check before delete; foreign lock not removed |
| Single-instance socket | Trusted dir (owner, `0700`, not symlink); peer uid; own-uid socket metadata; commands limited to show/lock/hide/quit; dev socket name isolated under `devtools` |
| Clipboard | Exclusion hints on Linux/Windows/macOS via arboard; clear only if SHA-256 still matches; clear on lock/quit/`take_session` |
| Secret UI | Masked field, no undo; reveal timeout; passwords/notes/username/url/email via `secret_text`; EntryVm excludes passwords |
| URL open | `http://` / `https://` only |
| Tray | Open / Lock / Quit only; no titles/passwords |
| Release hardening (Linux) | `RLIMIT_CORE=0`, `PR_SET_DUMPABLE=0` unless `devtools`+`KEYWHERE_DEV_MEMORY_TEST` |
| Zeroize-on-free allocator | `dealloc`/`realloc` wipe old block; RAM test documents post-fix clipboard residue fixed |
| Unlock bypass | No release path unlocks without successful `Vault::open`; backoff on wrong password; stretch off UI thread |
| Build integrity basics | `Cargo.lock` committed; CI `--locked`; `rust-toolchain.toml` pins `1.95.0`; crypto confined to RustCrypto in `keywhere-core` |

## Residual risk (accepted / out of scope)

Same-user malware with ptrace while unlocked still wins (dumpable=0 raises the bar on Linux only). Sync providers keep ciphertext history forever; passphrase change does not re-encrypt old cloud versions. v2 has no MAC. GPU/compositor clipboard copies and swap without FDE remain outside what Keywhere can fully close. No Critical issues found in this pass; address KW-H1 before treating the app as safe defaults for non-expert recipients.

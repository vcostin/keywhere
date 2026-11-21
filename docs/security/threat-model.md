# Keywhere threat model and security requirements

Status: design requirements for the v1 desktop app. Not a code audit.
Scope: a local Password Safe v3 (`.psafe3`) vault, no cloud backend, Rust + GPUI, interoperable with Password Safe (Windows 3.x and the Linux 1.x port, which already uses this file) and Password Gorilla.

Where the shipped code deliberately differs from a requirement below, or hasn't met it yet, see
[Status against the code](#status-against-the-code) at the end. [SECURITY.md](../../SECURITY.md)
describes the current behaviour.

Accepted limits of the format: iterated SHA-256 is the only KDF, Twofish-CBC is unauthenticated until the trailing HMAC is checked, and a sync provider keeps every historical ciphertext forever. Keywhere must not invent a new KDF or framing.

Crypto belongs in `keywhere-core` (already `twofish`, `sha2`, `hmac`, `subtle`, `zeroize`, `getrandom`). The UI receives plaintext only after authentication, and only for as long as the vault stays unlocked.

## 1. Assets and adversaries

Assets, in order: master passphrase; stretched key P′; record key K and HMAC key L; decrypted fields (passwords, TOTP seeds `0x1b`, YubiKey secret `0x12`, password history, passkey private keys, attachments); clipboard contents; unlock state (search index, reveal buffers, tray menu).

The salt, iteration count, and stored SHA-256(P′) are not secret. They are password verifiers.

| Adversary | What they can do | What Keywhere can still do |
| --- | --- | --- |
| Same-user malware | Read process memory, hook the clipboard, capture the screen, ptrace | While unlocked, plaintext is exposed. Shrink that window. After lock, no secret bytes should remain. |
| Other local users | Read a world-readable vault, core dump, or sync folder | Mode `0600`, dumpable off, encrypted file. They still cannot open a strong vault. |
| Sync provider (Drive, Dropbox) | Offline guess, tamper, roll back, keep deleted versions, see size and timing | HMAC rejects tampering. Iterations and a long passphrase slow guessing. Rotating K/L limits a cracked old copy. |
| Stolen laptop | Image the disk, swap, and hibernation file | Full-disk encryption is the real control. `mlock` covers only the pages we lock. |
| Shoulder-surfer / screen share | See the display and window titles | Mask by default. No secrets in titles, tooltips, or notifications. |
| Clipboard snoopers | History, cloud clipboard, Klipper, middle-click selection | Auto-clear, OS exclusion hints, never the primary selection. |
| Screen capture | OS screenshot, OBS, Game Bar | Mask by default. Windows display-affinity where it exists. A camera is out of scope. |

Out of scope: evil-maid firmware, a compromised compiler, and a sync client that replaces the Keywhere binary. Supply-chain controls below only raise that bar.

## 2. psafe3: what is weak, and the required behavior

Layout (format v3.31): `PWS3 ‖ salt(32) ‖ ITER u32le ‖ SHA-256(P′) ‖ EK(K) ‖ EK(L) ‖ IV ‖ CBC fields ‖ "PWS3-EOFPWS3-EOF" ‖ HMAC-SHA256`. K and L are 32-byte keys, each Twofish-256-ECB under P′ as two blocks, and they must be independent `getrandom` draws. Record bytes are Twofish-256-CBC under K. The EOF marker is 16 bytes of plaintext and is not part of the HMAC.

Stretch exactly as Password Safe does, or the file will not open elsewhere. Passphrase bytes are UTF-8 with no trailing NUL and no Unicode normalization. `H0 = SHA-256(passphrase ‖ salt)`, then `Hi = SHA-256(H{i-1})` for `i = 1..=ITER`, `P′ = H_ITER`, verifier `SHA-256(P′)`. An off-by-one is a silent interop break. Pin this with the known-answer file in `fixtures/`.

HMAC-SHA256 under L covers field payloads only, in file order, from the header version through the last record. It does not cover the 4-byte length, the type byte, CBC padding, the EOF marker, or the header prefix. Empty fields (including `0xff` END) add nothing. Getting this wrong either rejects good files or accepts tampered ones.

### Iterations

- Read files with any `ITER ≥ 1` that we allow below. Old databases used 2,048.
- On every writable save of a file under the format minimum, rewrite `ITER` to at least **262,144** (format `0x030F`). Password Safe upgrades silently; so should we. Do not upgrade a read-only open.
- Default for new files: **262,144**. That matches the current minimum and stays tolerable for Password Gorilla’s pure-Tcl SHA-256. A “slower” preference may raise it. Warn above **1,048,576** if the user also uses Gorilla.
- `ITER` is attacker-controlled. Refuse `0` and refuse anything above **16,777,216** (`2^24`) instead of hashing on the UI thread for minutes. Run stretching off the UI thread.

### Compare and authenticate before use

- Compare the 32-byte verifier and the 32-byte HMAC with `subtle::ConstantTimeEq` over the whole value. Never `==` on those bytes.
- Wrong passphrase (verifier mismatch) and tampered file (verifier ok, HMAC mismatch) are different errors. Both are generic otherwise: no field dump, no partial record.
- Decrypt into a scratch buffer, compute the HMAC, constant-time compare, and only then move records into the vault model. A UI that paints “as it goes” will flash forged plaintext. Twofish-CBC is malleable until this check.
- Do not trust length, type, group names, or URLs from an unverified buffer. In particular, never follow record URL (`0x0d`) or Run Command (`0x12`) automatically.

### Hostile and truncated files

Minimum well-formed size is the 152-byte prefix through the IV, plus one EOF block and 32-byte HMAC (200 bytes) and a real header. Reject short files, a bad tag, a missing EOF, and a short HMAC.

- Cap a single field (text: 1 MiB; attachment `0x29`: 32 MiB) and the whole plaintext (64 MiB). The spec allows a 4 GiB attachment; that is a memory bomb.
- Cap the number of fields and records. A length that runs past the EOF is an error, not a read.
- Stop at the first exact EOF marker. Random padding inside a partial block must be fresh `getrandom` output on write, never zeroes.
- Check CBC length arithmetic for overflow before allocating.
- On failure, zeroize the scratch buffer and leave the vault locked. No recovery that “opens what we could.”

### Round-trip

Preserve unknown field types, including `0xe0`–`0xfe`, repeated empty groups (`0x11`), attachments, TOTP, passkeys, and the YubiKey header field. Keep type, payload, and order. Older readers use the `0xff` terminator to skip what they do not know; Keywhere must not be the writer that strips them.

Write header field `0x06` (“what performed last save”) as `Keywhere <version>`. Do not add a cleartext password hint. Do not normalize or rewrite text on a no-op save.

Password history, alias forms (`[[uuid]]`), and shortcut forms (`[~uuid~]`) are secrets or pointers to secrets. Do not copy the literal alias token to the clipboard. Do not log them.

V1/V2 Blowfish databases are a different format. Do not claim to read them and do not write them.

## 3. Memory hygiene

`keywhere-core` should keep `#![forbid(unsafe_code)]` except one small platform module for locking pages.

- Secrets are `zeroize::Zeroizing` or `secrecy::Secret` / `SecretVec` (Debug redacted, zeroize on drop). That includes the passphrase, P′, K, L, field payloads, and the HMAC key. No `#[derive(Debug)]` on those types. `thiserror` messages must not embed secrets.
- After K and L are unwrapped, zeroize the passphrase. Keep P′, K, and L only while unlocked; zeroize all three, plus every plaintext field and the search index, on lock. `String` does not zeroize: do not store a secret in `String`.
- `mlock` the secret allocations (Linux `mlock` + `madvise(MADV_DONTDUMP)`, macOS `mlock`, Windows `VirtualLock` after raising the working set). Failure to lock is a warning, not a silent success. This does not cover GPUI’s text cache, the GPU, or the clipboard daemon.
- GPUI `SharedString` is interned for the process lifetime. Never put a passphrase, password, or TOTP seed in one. The master-password field must not keep an undo stack. Reveal uses a masked default and a short-lived buffer that is zeroized when the reveal times out or the vault locks. Bullet characters are not the secret.
- Release builds: `setrlimit(RLIMIT_CORE, 0)` and Linux `prctl(PR_SET_DUMPABLE, 0)` so a crash dump and same-user ptrace do not see the heap. Debug builds may leave this off. Hibernation and swap of unlocked pages are residual; full-disk encryption is the assumed control, and it should be stated in the user docs.
- No secret in `tracing`, panic payloads, window titles, or crash dialogs.

## 4. Clipboard

Copy is explicit. Clear after **30 seconds** by default (user-configurable, minimum 5). Clear on lock, on exit, and only if the clipboard still holds what we placed (compare the change count / sequence, then clear). Never also set the X11 primary selection.

Place the OS exclusion hint on the same write as the text:

- **Windows.** `CF_UNICODETEXT` plus registered formats `ExcludeClipboardContentFromMonitorProcessing` (data `"1"`), `CanIncludeInClipboardHistory` (DWORD 0), and `CanUploadToCloudClipboard` (DWORD 0). The first format is what keeps Clipboard History and Cloud Clipboard from storing the clip.
- **macOS.** `public.utf8-plain-text` plus `org.nspasteboard.ConcealedType` on the same pasteboard item. Universal Clipboard may still sync; the hint is necessary but not sufficient. Auto-clear is the backstop.
- **Linux (X11 and Wayland).** Offer MIME type `x-kde-passwordManagerHint` with value `secret`, alongside `text/plain;charset=utf-8`. Klipper, CopyQ, and cliphist honor it. GNOME Clipboard Indicator and some GTK paths do not. On Wayland the source process holds the bytes until clear or the next copy; dropping them on the timer matters. Do not depend on a portal to add this type.

`arboard` alone does not set these formats. Use a small platform writer, or extend the clipboard call so the hint and the text are one ownership transfer. User-driven Cmd/Ctrl-C inside a text field bypasses this; password fields stay masked and non-selectable by default so that path does not exist for secrets.

## 5. Safe save in a sync folder

The provider will upload partial files if we write in place. Save never truncates the live file.

1. Read-only opens do not create a lock, do not update last-access time (`0x09`), and do not rewrite iterations.
2. Before writing, hash the ciphertext (SHA-256) and record mtime/size at open. If either changed, abort and tell the user the file moved under them. Do not merge and do not overwrite. Mention a sibling Dropbox/Drive conflict copy if one is present; do not open it silently.
3. Lock file, compatible with Password Safe: for `vault.psafe3` the lock is `vault.plk` (replace the suffix; a name ending in `.cfg` instead gets `.plk` appended). Contents are the text `user@host:pid`. Create it with `0600`. If it exists and names a live process on this machine, open read-only. If the pid is dead or the host is another machine, warn and let the user take the lock; a synced `.plk` is often stale because the other client deletes it on close and the sync races. Remove `vault.plk` on close only if the contents are still ours. Read-only never deletes someone else’s lock.
4. Copy the current file to `vault.ibak` (Password Safe’s intermediate backup name), `fsync` it, then write `vault.psafe3.tmp.<random>` in the same directory at mode `0600`. Same directory is required for an atomic replace. Never use `/tmp` (other users, different filesystem).
5. `fsync` the temp file. POSIX: `rename` over the destination, then `fsync` the directory. Windows: `ReplaceFileW`. Delete the temp file on any error.
6. If the resulting mode is group- or world-readable, warn. Sync clients sometimes loosen ACLs.
7. On master-passphrase change, generate a new salt, K, L, and IV, then stretch the new passphrase. Reusing K would let anyone who later cracks an old synced version decrypt the new file. Ordinary saves also get a new IV and new padding; rotating K and L on ordinary saves is preferred for the same reason and does not break readers.

Backups are ciphertext under the passphrase that was current at that save. Changing the passphrase does not re-encrypt provider-side history. Say that in the UI when the passphrase changes. Do not write a backup into a world-readable location.

## 6. Auto-lock

Lock means: zeroize P′, K, L, and all plaintext; drop the search index and reveal buffers; clear our clipboard; set the tray to locked. Do not write the file as a side effect of locking (that races the sync client and updates last-access).

Must lock on:

- Idle, default **5 minutes**, reset only by input in our windows (not by any global keystroke, which is itself a keylogger-shaped permission).
- Screen lock: Windows `WTSRegisterSessionNotification` / `WTS_SESSION_LOCK`; macOS distributed notification `com.apple.screenIsLocked`; Linux logind `Lock` on the session plus `org.freedesktop.ScreenSaver` / `org.gnome.ScreenSaver` `ActiveChanged`. Subscribe to more than one. Desktops disagree.
- Suspend: Windows `PBT_APMSUSPEND`, macOS `NSWorkspace` will-sleep, Linux logind `PrepareForSleep`. The machine may freeze before the handler runs; `mlock` plus disk encryption covers that gap, not the handler.
- Fast-user switch and explicit Lock.

Should lock on shutdown. Should not lock merely because the window lost focus.

A failed unlock sleeps with a small backoff (for example 1 s, capped) so a passer-by cannot spin guesses. This does nothing against an offline attacker who has the file.

## 7. Tray icon

The tray is another unlocked surface, and on some Linux desktops it is reachable from the lock screen.

- Screen lock and suspend must lock the vault before, or as, the lock screen appears. A locked tray offers only “open Keywhere.” No search, no titles, no copy.
- The menu may show entry titles for quick copy. It must not show passwords, usernames-as-secrets, notes, or TOTP codes. Copy still follows the clipboard policy.
- No password in the tooltip, the notification, or the accessible name. AT-SPI and Windows UI Automation can read menu text from any process in the session.
- The icon may show locked vs unlocked. Keep that discreet; it tells a camera the vault is open.
- Closing the last window must not leave the vault unlocked in the tray unless the user opted into “keep running.” Default is to lock when the window closes. A stale `.plk` from a tray process that did not exit is how Password Safe users get stuck read-only; unlock and exit must both clear it.

## 8. Supply chain

`keywhere-core` stays on RustCrypto: `sha2` and `hmac` for the KDF and MAC, `twofish` plus the `cbc` crate for CBC (do not hand-roll the mode), `cipher` block API for the two ECB blocks, `subtle` for compares, `zeroize` / `secrecy` for secrets, `getrandom` for salt, K, L, IV, and padding. No OpenSSL. No application crypto in the GPUI crate.

- Commit `Cargo.lock`. CI builds with `--locked` and a pinned Rust toolchain.
- `cargo-audit` in CI. `cargo-deny`: known advisories, crates.io-only sources, and licenses compatible with GPL-3.0-or-later (MIT and Apache-2.0 are fine). Ban duplicate copies of `sha2` / `hmac` / `zeroize` that would split secret types.
- `cargo-auditable` (or CycloneDX) on release binaries so a later advisory maps to a shipped build.
- Reproducible release: `SOURCE_DATE_EPOCH`, no build script that embeds the local path, publish the lockfile hash next to the binary. A signature on the release artifact should be the goal once there is a release key.
- GPUI pulls a large tree. It must not be a dependency of `keywhere-core`. Review new core dependencies deliberately; the current set is the right size.
- `twofish` 0.7 is the interoperability choice, not a modern AEAD. Do not “upgrade” the construction.

## 9. Priority

**Must**

1. Stretch, HMAC payload scope, EOF, and ECB wrapping match Password Safe, with a fixture test.
2. Constant-time compare of the verifier and the HMAC; no record reaches the UI or the clipboard before the HMAC matches.
3. Bounds on `ITER`, field size, and record count; reject truncated files; zeroize scratch buffers on failure.
4. Preserve unknown fields and their order on save.
5. On save, `ITER` at least 262,144; new files default to 262,144.
6. Zeroize passphrase, P′, K, L, and plaintext on lock and on drop. No secrets in `String`, `SharedString`, logs, or titles.
7. New salt, K, and L when the master passphrase changes.
8. Save via same-directory temp file, `fsync`, atomic replace; abort if the file changed underneath; `0600`.
9. `.plk` naming and `user@host:pid` contents compatible with Password Safe; read-only when the lock is live; never delete a foreign lock.
10. Clipboard auto-clear, no primary selection, plus the Windows, macOS, and `x-kde-passwordManagerHint` hints.
11. Auto-lock on idle, screen lock, and suspend, including the tray.
12. `cargo-audit` and `cargo-deny` in CI; `--locked` builds; crypto confined to RustCrypto in `keywhere-core`.

**Should**

13. `mlock` / `VirtualLock` / `MADV_DONTDUMP`, and core dumps disabled in release builds.
14. Rotate K and L on ordinary saves, not only on passphrase change.
15. Intermediate `vault.ibak`, and a warning that cloud history survives a passphrase change.
16. Master-password field without undo; reveal auto-hides.
17. Windows `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on vault windows.
18. Warn on group/world-readable mode and on sync conflict siblings.
19. Unlock backoff. Stretching off the UI thread.
20. Round-trip tests against current Password Safe and Password Gorilla before the first release that writes files.

**Could**

21. User-chosen higher iteration count, with a Gorilla slowness warning.
22. Optional “lock when the window loses focus.”
23. A zeroizing allocator for the core crate.
24. Signed, `SOURCE_DATE_EPOCH` release builds and `cargo-vet` for new crates.

## Residual risk

A same-user process can read an unlocked vault. Iterated SHA-256 will not stop a determined offline attack on a weak passphrase, and a sync provider already has the ciphertext. Disk encryption, a long passphrase, a short unlock window, and not clobbering a newer synced copy are the controls that match this product. A future V4 file or Argon2 would break Password Safe and Gorilla, so they are not options for the vault format.

## Status against the code

Checked against `main` at `a8296f4` (2026-11-22). The requirements above are kept as written;
this table records where the code differs.

| Requirement | Code today |
| --- | --- |
| §2 "V1/V2 Blowfish databases … do not claim to read them and do not write them." | Superseded by an owner decision (M4): Password Gorilla **v2** files are read and saved back as v2 ([psafe2.rs](../../crates/keywhere-core/src/psafe2.rs)), behind a weakness banner with **Upgrade to v3…**; a path once seen as v3 that turns into v2 opens read‑only. V1 is not supported. |
| §3 `#![forbid(unsafe_code)]` in `keywhere-core` except one platform module | Not set. `unsafe` is confined to `LockedKey` in [psafe3.rs](../../crates/keywhere-core/src/psafe3.rs) (`mlock`/`madvise`/`munlock`) and two libc calls in [lock.rs](../../crates/keywhere-core/src/lock.rs) (`gethostname`, `kill(pid, 0)`). |
| §3 `mlock` the secret allocations; failure is a warning | Only P′ is locked (K and L live only during a decrypt or save; plaintext fields are not locked). Linux and macOS use `mlock` (Linux adds `MADV_DONTDUMP`); Windows has no `VirtualLock` yet. A v2 vault's data key is zeroized but not locked. A failed lock is not reported (audit N2; SECURITY.md accepted residuals). |
| §3 release builds disable core dumps and ptrace | Done on Linux (`RLIMIT_CORE=0`, `PR_SET_DUMPABLE=0`) and macOS (`RLIMIT_CORE=0`, `PT_DENY_ATTACH`); Windows turns off error‑report dumps and denies same‑user memory access with a process DACL. |
| §5.5 Windows `ReplaceFileW` | `std::fs::rename`, which on Windows is `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`; still atomic on the same volume. |
| §6 / §7 lock on screen lock and suspend, lock on close | Done on all three platforms; closing the window locks by default (`80f11a1`). macOS will‑sleep is not a barrier: the lock may not have painted before the Mac sleeps (IOKit handling is to do). |
| §8 licenses "compatible with GPL‑3.0‑or‑later" | Keywhere is MIT; [deny.toml](../../deny.toml) allows a permissive list (plus MPL‑2.0 and Mozilla's CA data licence). |
| §8 ban duplicate `sha2` / `hmac` / `zeroize` | `hmac`, `zeroize`, `subtle`, `twofish`, `blowfish` and `cbc` are pinned to one version. `sha2` 0.11 is pulled only by gpui's asset hashing; all Keywhere crypto uses 0.10. |
| §8 `cargo-auditable`, reproducible builds | Not yet (M5). Releases ship `SHA256SUMS` and a minisign‑signed update manifest. |
| §9 Could 23 zeroizing allocator | Done for the whole app process (`alloc.rs`), not only the core crate. |

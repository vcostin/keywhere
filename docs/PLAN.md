# Keywhere — product & technical plan

Keywhere is a free, open‑source, cross‑platform desktop password manager. It works on
Password Safe v3 files (`.psafe3`), the format Password Safe and Password Gorilla share, so an
existing vault opens unchanged and stays usable in those apps. There is no cloud service, no
account and no "Pro" upsell. Sync is whatever folder you keep the file in (Google Drive, Dropbox,
Syncthing).

It is meant for people who already have a psafe3 or Password Gorilla vault, not as a
general password manager. Positioning and priorities: [STRATEGY.md](STRATEGY.md); roadmap: §7.

## 1. What we learned from the apps we're replacing

| | Password Gorilla | Password Safe 1.x | **Keywhere takeaway** |
|---|---|---|---|
| Stack / look | Tcl/Tk, 2000s widgets, project discontinued | wxWidgets, dense toolbar, many modal dialogs | GPU‑rendered GPUI UI, one consistent look on every OS |
| Formats | psafe3 + v2 | psafe3 (reference implementation) | psafe3 first‑class; v1/v2 import for old Gorilla files |
| Navigation | Single tree | Tree **or** list toggle | Sidebar + list + detail. Smart views: Favorites, Expiring, Weak/Reused |
| Search | Basic find dialog | Find toolbar | Instant fuzzy search and a Ctrl/⌘‑K command palette |
| Quick access | none | System tray menu with entry tree | Tray popover with search, ⏎ copies the password, then lock |
| Security UX | Clipboard clear | Lock on idle/minimise, YubiKey | Auto‑lock on idle/sleep/screen‑lock, clipboard auto‑clear; no keychain unlock (decision 2026‑11‑20) |
| Pain points | Unmaintained, ugly, HiDPI broken | Cluttered, dated dialogs | Fast, calm, keyboard‑first, no nags |

## 2. Visual direction

* Navy colour range: bg `#0A1631`, surface `#0F2042`, panel `#132852`, selection `#244A8F`,
  focus `#5B8CFF`. A single warm accent, amber `#F2B24C`, is reserved for the main action
  and for the active row marker. A light theme comes later and uses the same tokens.
* Logo: a map pin whose point becomes a key shaft (*Key‑where*): `assets/keywhere-key.svg` (flat amber key, tilted 30°, no background or outline; drawn in‑app as a crisp vector).
  A monochrome tray glyph is derived from it, with a "locked" and an "unlocked" variant.
* Mockups: `docs/mockups/keywhere-mockups.html` (identity, unlock, main window, tray, generator).

## 3. Architecture

```
crates/
  keywhere-core/   # no UI deps. psafe3 codec, typed Record model, safe save, generator,
                   #   password health, legacy v1/v2 import. 100 % unit/fuzz tested.
  keywhere-app/    # GPUI binary: windows, views, actions, keymap, theme
  keywhere-sys/    # OS glue behind traits: tray, clipboard, idle/lock events,
                   #   global hotkey, file watching
```

* **UI:** `gpui` (crates.io 0.2.x) plus `gpui-component` for inputs, lists, tree, dialogs.
  Pin exact versions. GPUI has no tray or clipboard‑policy APIs, so those live in `keywhere-sys`.
* **Tray:** prefer a single crate, `tray-icon`. On Linux, evaluate its StatusNotifierItem/`ksni`
  backend against using `ksni` directly; we want no GTK main loop. Tray events reach GPUI over a
  channel into its executor. The real risk is who owns the event loop on macOS and Windows, so
  M3 starts with a spike. A locked tray offers only "Open Keywhere": no titles, no search, no copy.
  Closing the last window locks by default unless "keep running in tray" is on.
* **Clipboard:** `arboard`, plus per‑OS hints. Windows: `ExcludeClipboardContentFromMonitorProcessing`
  and `CanIncludeInClipboardHistory=0`. macOS: `org.nspasteboard.ConcealedType`. KDE:
  `x-kde-passwordManagerHint=secret`. `arboard` alone does not set these formats, so a small
  per‑OS writer puts the hint and the text in one ownership transfer. Auto‑clear after **30 s**
  (configurable 5–120 s), and only if the clipboard still holds what we put there.
* **Global hotkey:** `global-hotkey` on Windows/macOS/X11. **Wayland is first‑class**, not a
  footnote: use the xdg‑desktop‑portal GlobalShortcuts portal (KDE and recent GNOME). Where the
  portal is missing, say so in settings and fall back to the tray click.
* **Secrets in the UI:** GPUI draws text only from `SharedString` (an `Arc<str>` that isn't
  zeroized on drop). Secret display text (revealed passwords, notes, the master password while
  revealed) goes through `secret_text::display`, which keeps a reference to every such `Arc` and
  zeroizes it in place once GPUI has released it (a sweep every 1 s and on lock). The master‑password field and password
  reveal use a custom masked input backed by `Zeroizing<Vec<u8>>`, with no undo stack. Revealed
  text is zeroized when the reveal times out or the vault locks. Key stretching runs off the UI thread.
* **Crypto:** RustCrypto only (`twofish`, `cbc`, `sha2`, `hmac`, `subtle`, `zeroize`, `getrandom`).
  The only crypto code we write is the psafe3 key‑wrap (two Twofish‑ECB blocks) and the record
  framing. CBC comes from the `cbc` crate.

## 4. psafe3 engine (built, `crates/keywhere-core`)

Done and verified:

* Decrypt files written by Password Safe 1.26 (fixture `fixtures/original-empty.psafe3`).
* Write files that Password Safe's own `pwsafe-cli` reads back (22 records exported correctly).
* Constant‑time password‑hash and HMAC checks. No field is built until the HMAC verifies. Once the
  password is known to be right, every failure becomes one uniform `Integrity` error (a test flips
  bits across the whole ciphertext). Caps are checked before allocating: ciphertext 64 MiB,
  field 32 MiB, 1M fields, iterations from 2 048 to 2²⁴. Truncated files are rejected.
* Unknown header/record fields are preserved verbatim, so other apps' data survives a save.
* Iteration floor of 262 144 on save. Fresh salt, K, L and IV on every save. Random padding.
* Safe save: write `vault.psafe3.tmp.<random>` (create‑new, mode 0600), fsync, **re‑read from
  disk and decrypt‑verify**, write `vault.ibak` (Password Safe's backup name) the same way,
  atomic replace (`rename`; on Windows Rust's std uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`),
  fsync the directory, delete the temp file on any error (test).
* Old 8‑character hex timestamps (pre‑3.09) parse. `set_group_path` escapes literal dots.

Next for the engine:

1. `secrecy`/`Zeroizing` wrappers for the master password and for decrypted strings in the model.
   `mlock` the key pages. Disable core dumps (`prctl(PR_SET_DUMPABLE,0)` / `setrlimit`).
2. Detect changes made elsewhere: record mtime + SHA‑256 on open. If either changed by save time,
   offer **Merge / Overwrite / Save as**. Password Safe lock: `vault.plk` containing
   `user@host:pid`, mode 0600. A live local lock opens the vault read‑only. A stale or remote
   lock shows a warning and lets you take it over. A lock that isn't ours is never deleted.
   Read‑only opens create no lock, don't touch the access time and never rewrite the file.
   Rotate salt/K/L on every save, including password changes (already done).
3. Typed accessors for every v3 field: policy, autotype, run command, protected, symbol set,
   attachments (preserved).
4. Password Gorilla legacy: read v1/v2 (Blowfish) files and offer to save them as v3.
5. `cargo fuzz` targets on `psafe3::decode`, tampered real files, the psafe3 plaintext parser, encode/decode
   round trip and `psafe2::decode` (done: [fuzz/README.md](../fuzz/README.md), 60 s per target in CI on engine PRs).
   `cargo-deny` and `cargo-audit` in CI (done).

## 5. Features and milestones

| Milestone | Scope |
|---|---|
| **M0 (done)** | Core codec, sample vault, logo, mockups, security review |
| **M1 Read‑only viewer** | GPUI unlock screen, 3‑pane browse, search, copy with auto‑clear, auto‑lock |
| **M2 Editor** | Add/edit/delete/move, groups drag & drop, generator, password history, expiry, safe save + conflict dialog |
| **M3 Tray & quick access** | Tray popover, global hotkey, lock‑on‑sleep/screen‑lock, start minimised |
| **M4 Polish (done)** | Settings page (auto‑lock, clipboard, close behaviour, theme, language), English/Romanian UI, dropdown + group combobox, group fold/new/delete, "No group" view, health views (weak/reused/expiring), favorites, light theme, Password Gorilla v2 read/write + upgrade. Favicons deferred. |
| **M5 Ship** | Packages (done): AppImage, .deb, .rpm, macOS .dmg, Windows portable exe, setup and MSI. macOS and Windows are unsigned. In‑app updates with a minisign‑signed manifest and opt‑in checks (done). Next: reproducible builds. Flatpak only if asked. Code signing and notarization deferred (see §7) |

## 6. Security posture (summary; full threat model in `docs/security/threat-model.md`)

* MUST: constant‑time comparisons, authenticate before parsing, zeroize secrets, clipboard
  auto‑clear with history exclusion, auto‑lock, atomic save with a backup, no secrets in logs,
  argv or crash reports, no network by default.
* SHOULD: mlock, no core dumps, conflict detection, `.plk` lock, fuzzing, dependency auditing.
* WON'T (for now, owner decision 2026‑11‑20): OS keychain / biometric quick‑unlock. The master password
  stays the only way in; nothing derived from it is handed to KWallet/Secret Service/Keychain.
* WON'T (owner decision 2026‑11‑26): YubiKey challenge‑response. Vaults that need a YubiKey can't be
  opened.

## 7. Roadmap after v0.1.6

Why this order: [STRATEGY.md](STRATEGY.md). The project runs at maintenance pace: small releases
when something needs fixing, no feature schedule.

| When | What |
|---|---|
| **Now** | README says who Keywhere is for and what's not planned · SECURITY.md calls the AI reviews "AI‑assisted", not audits · keep dependency updates merged |
| **When something bites** | Bugs from real users, Linux first · platform gaps when someone hits them: Windows single instance, Windows `VirtualLock`, macOS hardware test pass (#33) · reproducible builds (#31) |
| **Only if people ask** (a GitHub issue counts) | CSV import · KDBX import · Flatpak (#28). A CSV export writes every password in plain text, so it needs a warning and a mode‑0600 file |
| **Deferred** | Code signing and notarization (#29, #30), until there's a budget |
| **Not planned** | Browser autofill, a mobile app, TOTP, passkeys, YubiKey unlock, cloud sync, accounts, sharing |
| **Waiting** | GPUI or Tauri 2 for the UI: decide after Discord publishes its GPUI migration. `keywhere-core` has no UI dependency and stays either way |

# Keywhere — product & technical plan

Keywhere is a free, open‑source, cross‑platform desktop password manager. It works on
Password Safe v3 files (`.psafe3`), the format Password Safe and Password Gorilla share, so an
existing vault opens unchanged and stays usable in those apps. There is no cloud service, no
account and no "Pro" upsell. Sync is whatever folder you keep the file in (Google Drive, Dropbox,
Syncthing).

## 1. What we learned from the apps we're replacing

| | Password Gorilla | Password Safe 1.x | Strongbox (macOS) | **Keywhere takeaway** |
|---|---|---|---|---|
| Stack / look | Tcl/Tk, 2000s widgets, project discontinued | wxWidgets, dense toolbar, many modal dialogs | Native AppKit/SwiftUI, modern, 3‑pane | GPU‑rendered GPUI UI, one consistent look on every OS |
| Formats | psafe3 + v2 | psafe3 (reference implementation) | KeePass + psafe3 | psafe3 first‑class; v1/v2 import for old Gorilla files |
| Navigation | Single tree | Tree **or** list toggle | Sidebar (groups/tags/smart views) + list + detail | Sidebar + list + detail. Smart views: Favorites, Expiring, Weak/Reused |
| Search | Basic find dialog | Find toolbar | Instant search | Instant fuzzy search and a Ctrl/⌘‑K command palette |
| Quick access | none | System tray menu with entry tree | Menu‑bar quick search | Tray popover with search, ⏎ copies the password, then lock |
| Security UX | Clipboard clear | Lock on idle/minimise, YubiKey | Auto‑lock, Touch ID | Auto‑lock on idle/sleep/screen‑lock, clipboard auto‑clear, OS keychain unlock later |
| Pain points | Unmaintained, ugly, HiDPI broken | Cluttered, dated dialogs | Nags for Pro, macOS only | Fast, calm, keyboard‑first, no nags |

## 2. Visual direction

* Navy colour range: bg `#0A1631`, surface `#0F2042`, panel `#132852`, selection `#244A8F`,
  focus `#5B8CFF`. A single warm accent, amber `#F2B24C`, is reserved for the main action
  and for the active row marker. A light theme comes later and uses the same tokens.
* Logo: a map pin whose point becomes a key shaft (*Key‑where*): `assets/keywhere-logo.svg`.
  A monochrome tray glyph is derived from it, with a "locked" and an "unlocked" variant.
* Mockups: `docs/mockups/keywhere-mockups.html` (identity, unlock, main window, tray, generator).

## 3. Architecture

```
crates/
  keywhere-core/   # no UI deps. psafe3 codec, typed Record model, safe save, generator,
                   #   password health, legacy v1/v2 import. 100 % unit/fuzz tested.
  keywhere-app/    # GPUI binary: windows, views, actions, keymap, theme
  keywhere-sys/    # OS glue behind traits: tray, clipboard, idle/lock events,
                   #   global hotkey, keychain, file watching
```

* **UI:** `gpui` (crates.io 0.2.x) plus `gpui-component` for inputs, lists, tree, dialogs.
  Pin exact versions. GPUI has no tray or clipboard‑policy APIs, so those live in `keywhere-sys`.
* **Tray:** `tray-icon` on Windows/macOS, `ksni` (StatusNotifierItem over D‑Bus, no GTK) on Linux.
  The tray process shows no secrets until the vault is unlocked.
* **Clipboard:** `arboard`, plus per‑OS hints. Windows: `ExcludeClipboardContentFromMonitorProcessing`
  and `CanIncludeInClipboardHistory=0`. macOS: `org.nspasteboard.ConcealedType`. KDE:
  `x-kde-passwordManagerHint=secret`. Auto‑clear after 20 s, but only if the clipboard still
  holds what we put there.
* **Global hotkey:** `global-hotkey` on Windows/macOS/X11. On Wayland, use the
  xdg‑desktop‑portal GlobalShortcuts portal and degrade gracefully where it is missing.
* **Crypto:** RustCrypto only (`twofish`, `sha2`, `hmac`, `subtle`, `zeroize`, `getrandom`).
  Nothing hand‑rolled except the format framing.

## 4. psafe3 engine (built, `crates/keywhere-core`)

Done and verified:

* Decrypt files written by Password Safe 1.26 (fixture `fixtures/original-empty.psafe3`).
* Write files that Password Safe's own `pwsafe-cli` reads back (22 records exported correctly).
* Constant‑time password‑hash and HMAC checks. Nothing is returned before the HMAC verifies.
  Every length is bounds‑checked. Truncated or tampered files are rejected (tests).
* Unknown header/record fields are preserved verbatim, so other apps' data survives a save.
* Iteration floor of 262 144 on save. Fresh salt, K, L and IV on every save. Random padding.
* Safe save: write `.tmp`, fsync, decrypt‑verify, keep `file~`, atomic rename, fsync the directory.

Next for the engine:

1. `secrecy`/`Zeroizing` wrappers for the master password and for decrypted strings in the model.
   `mlock` the key pages. Disable core dumps (`prctl(PR_SET_DUMPABLE,0)` / `setrlimit`).
2. Detect changes made elsewhere: record mtime + SHA‑256 on open. If either changed by save time,
   offer **Merge / Overwrite / Save as**. Honour and create Password Safe's `.plk` lock file.
3. Typed accessors for every v3 field: policy, autotype, run command, protected, symbol set,
   attachments (preserved).
4. Password Gorilla legacy: read v1/v2 (Blowfish) files and offer to save them as v3.
5. `cargo fuzz` target on `psafe3::decode`. `cargo-deny` and `cargo-audit` in CI.

## 5. Features and milestones

| Milestone | Scope |
|---|---|
| **M0 (done)** | Core codec, sample vault, logo, mockups, security review |
| **M1 Read‑only viewer** | GPUI unlock screen, 3‑pane browse, search, copy with auto‑clear, auto‑lock |
| **M2 Editor** | Add/edit/delete/move, groups drag & drop, generator, password history, expiry, safe save + conflict dialog |
| **M3 Tray & quick access** | Tray popover, global hotkey, lock‑on‑sleep/screen‑lock, start minimised |
| **M4 Polish** | Health report (weak/reused/expired), favicons (opt‑in, offline cache), light theme, i18n, keychain/biometric unlock (opt‑in) |
| **M5 Ship** | Packages: AppImage/Flatpak, .dmg (notarised), MSI. Reproducible builds and signed releases |

## 6. Security posture (summary; full threat model in `docs/security-review-grok.md`)

* MUST: constant‑time comparisons, authenticate before parsing, zeroize secrets, clipboard
  auto‑clear with history exclusion, auto‑lock, atomic save with a backup, no secrets in logs,
  argv or crash reports, no network by default.
* SHOULD: mlock, no core dumps, conflict detection, `.plk` lock, fuzzing, dependency auditing.
* COULD: OS keychain / biometric quick‑unlock (stores a wrapped key, never the password), YubiKey.

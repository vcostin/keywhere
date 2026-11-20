# Security

Keywhere is a password manager. Please report vulnerabilities privately through
[GitHub security advisories](https://github.com/vcostin/keywhere/security/advisories/new),
not as public issues.

## What Keywhere protects

* **The vault file** (Password Safe v3): SHA‑256 key stretching (at least 262 144
  rounds on save), Twofish‑CBC, HMAC‑SHA256, all checked in constant time.
  Nothing is parsed before the HMAC verifies. Every save uses a fresh salt and
  fresh keys, and the file is written atomically with a `.ibak` backup.
  Conflicting changes made by another device are detected.
* **Memory while unlocked.** Only the stretched key is kept (in an `mlock`ed
  page that core dumps exclude), never the master password. Decrypted fields
  are zeroized when the vault locks. A zeroizing global allocator wipes every
  freed heap block, including copies made by libraries such as the GUI text
  cache or the clipboard backend. On Linux, release builds turn off ptrace and
  core dumps (`PR_SET_DUMPABLE=0`, `RLIMIT_CORE=0`).
  [docs/security/ram-test.md](docs/security/ram-test.md) shows a measured
  result: after lock, none of the vault's passwords are left in the process.
* **Locking.** Keywhere locks after idle time, on screen lock, and before
  suspend. On Linux it holds a logind delay inhibitor until the locked screen
  has been drawn.
* **Clipboard.** Copied secrets carry "exclude from history/cloud" hints and
  are cleared after 30 s (configurable), on lock and on quit. If the clipboard
  can't be read back, it is cleared anyway. When the hints can't be set, the
  copy is refused.
* **Deceptive vault content.** Titles, usernames, group names and URLs are
  shown without control, bidi or zero‑width characters. A URL with hidden
  characters or `user@host` userinfo is never opened, and an internationalised
  host is shown in punycode next to the link.
* **File handling.**
  * A vault reached through a symlink is saved into the link's target.
  * Keywhere warns when other local users can read the vault file and offers
    to fix it.
  * A vault that was v3 and has turned into an old v2 file (possibly a planted
    copy) opens read‑only.
  * The `.plk` lock is only taken over if it still names the holder you saw.
  * Changes that fail to save are written, still encrypted, to
    `<name>.unsaved-<time>.psafe3` before the vault locks.

## What it cannot protect against

* **Malware running as your user while the vault is unlocked.** Such malware
  can read the screen, inject input, log keys (any app on X11 can), or load
  code into Keywhere with `LD_PRELOAD` or a modified `.desktop` file.
  Keywhere raises the bar, but it is not a sandbox.
* **Swap and hibernation without disk encryption.** The key page is locked in
  RAM, but the rest of the unlocked vault can be paged out. Use full‑disk
  encryption with encrypted swap. Hibernating writes RAM to disk; Keywhere
  locks before sleep, but an encrypted disk is the real protection.
* **Weak master passwords.** Key stretching slows guessing; it doesn't stop it.
* **Sync history.** Cloud folders keep old versions of the file. Changing the
  master password doesn't re‑encrypt copies the provider already has.
* **Password Safe v2 files** (Password Gorilla) have weak key derivation and no
  tamper detection. Keywhere warns about this and offers an upgrade to v3.
* **Copies outside the process**: the compositor or X server holding the
  clipboard selection, GPU memory, and screenshots or screen recorders.

## Platform status

Linux (Wayland and X11) is the reference platform. **The macOS and Windows
builds are not ready to give to others** until these are in place:

* screen‑lock and sleep auto‑lock (`WTS_SESSION_LOCK` / `PBT_APMSUSPEND` on
  Windows; screen‑locked / will‑sleep notifications on macOS);
* process hardening equivalent to Linux (crash‑dump policy, no debugger
  attach);
* exclusion from screen capture.

## Defaults

Closing the window with a tray available keeps the vault unlocked in the
background ("Hide, keep unlocked") until the auto‑lock time, a screen lock,
sleep, or tray → Lock. Choose "Lock" under Settings → On close for the stricter
behaviour.

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
* **Memory while unlocked.** Only the stretched key is kept, never the master
  password. On Linux and macOS it sits in an `mlock`ed page (on Linux also
  excluded from core dumps); Windows doesn't lock it yet. Decrypted fields
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

## Updates

* **No network by default.** Update checks start only after you say yes (first‑run question
  or Settings → Updates). A check is two HTTPS requests to github.com for this repository's
  latest release; nothing about your vaults is sent.
* **Signed.** The release workflow signs `latest.json` with a minisign (Ed25519) key whose
  public half is compiled into Keywhere (`packaging/keywhere-update.pub`). An unsigned or
  differently signed manifest is rejected.
* **Pinned.** The manifest's download URL must be a release file of this repository for that
  exact version, and its size and SHA‑256 must match before the file is used.
* **No downgrades.** Only a version newer than the running one is offered.
* **Safe swap.** The update is downloaded next to the running copy (mode 0600 on Unix),
  verified, must report the expected version, and only then goes in place:
  * the Linux AppImage by an atomic rename;
  * the Windows portable exe by renaming the running exe aside and the new one into its name
    (the old file is deleted at the next start);
  * the macOS `Keywhere.app` by unpacking the verified zip with `ditto` next to it and
    swapping the two bundles by rename.

  The vault is locked before the restart. Flatpak, distro packages, source builds and copies in
  folders Keywhere can't write (Program Files, a mounted disk image, a quarantined app that
  macOS runs from a translocated copy) are never modified.
* **Limits.** Whoever holds the signing key or the GitHub account can ship an update. An
  attacker who can block or replay GitHub responses can hide a new release (they can't
  install an old or foreign one).

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

## Security documents

All in [docs/security](docs/security):

* [Threat model](docs/security/threat-model.md): the requirements the app is built against.
* [RAM residue test](docs/security/ram-test.md): measured, repeatable.
* Independent code audits: [Cursor](docs/security/audit-cursor.md),
  [DeepSeek](docs/security/audit-deepseek.md), and a memory and hardware focused one by
  [Grok](docs/security/audit-memory-grok.md).
* [Review of the fixes](docs/security/audit-fixes-review-cursor.md) for those audits.

Each audit describes the code at the commit it names. The fixes review records what was fixed
and which residuals were accepted; this file describes the current behaviour and limits.

### Accepted residuals

* The key page can stay unlocked: when `mlock` fails (for example a low `RLIMIT_MEMLOCK`), on
  Windows (no `VirtualLock` yet), and for v2 vaults. Unlocking still works; Keywhere logs one
  warning to stderr and shows a note under Settings → Security while that vault is open.
  Decrypted fields are never `mlock`ed, so they can reach swap (use encrypted swap).
* A stale `.plk` lock from a crashed Keywhere is reclaimed without asking.
* Another program running as you can still send `show`, `hide`, `lock` or `quit` to the running
  instance over its private socket.
* A planted v2 file opens editable if this machine never opened the v3 vault at that path.
* The v3 HMAC covers field contents, not field type bytes, so someone who can edit the file can
  change the type of its first header field through the IV without being detected. Contents
  can't be changed, and turning that field into an end-of-entry marker is refused.

## Platform status

Linux (Wayland and X11) is the reference platform. macOS and Windows ship
as **unsigned previews**. They have:

* auto-lock on screen lock, user switch and sleep, and on Windows also when
  the display turns off (`WTS_SESSION_LOCK`, `PBT_APMSUSPEND`,
  `GUID_CONSOLE_DISPLAY_STATE`; the macOS screen-locked, screens-did-sleep
  and session-resign-active notifications, and IOKit's system-will-sleep,
  which, like the logind delay inhibitor on Linux, holds sleep until the
  app has locked and had time to draw the locked view);
* process hardening:
  * Windows: no error-report dumps, and a process access list that denies
    other programs of the same user memory access, thread injection and
    handle duplication;
  * macOS: no core dumps and `PT_DENY_ATTACH`;
* exclusion from screen capture (Windows display affinity; macOS sharing
  type, which recent macOS doesn't apply to every capture API).

Known gaps:

* Sleep waits at most about 3 seconds for the lock (Windows: 1.5 s). If the
  app doesn't answer in time, the system sleeps anyway, and the window may
  show the unlocked vault briefly on wake.
* The macOS build is only partly tested on real hardware (#33).
* Neither build is code-signed.

## Defaults

Closing the window locks the vault ("Lock and hide"). "Hide, keep unlocked"
is an opt‑in under Settings → Closing the window: the vault then stays unlocked in the
background until the auto‑lock time, a screen lock, sleep, or tray → Lock.
Settings files that already say `close_action = hide` keep that choice.

# Keywhere — independent security review (adversarial, deepseek)

Scope: the committed tree at `9718fc9` (line numbers below refer to it). While
this review was being written, an unreviewed change set appeared in the working
tree (mlock'd key material, a logind sleep inhibitor, `sensitive` text fields);
it targets memory residue and does not change any finding here. Reviewed against
`docs/PLAN.md` and `docs/security/threat-model.md`. Code only: the GUI was not
run, nothing was executed against `/home/roth/GoogleDrive`, and nothing outside
this file was modified.

Method: read the codec, the vault/save path, the lock, the app shell, the
browser, the OS glue, the vendored crates and CI; then tried to make each
plausible attack actually work in this code. Where an attack depended on
WHATWG URL behaviour I verified it against a spec-compliant parser.

---

## Findings

### F1 — Medium — a one-file rollback silently downgrades the vault to v2 (weak KDF, no integrity), and edits then persist in v2

Where: `crates/keywhere-core/src/lib.rs:344-372` (v3 only when the first four
bytes are `PWS3`, otherwise `looks_like_v2`), `psafe2.rs:165-171` (detection is
length shape only), `lib.rs:509-517` (`encrypt` writes v2 whenever the key is
v2), `crates/keywhere-app/src/browser.rs:2417-2428` (banner + manual upgrade),
`browser.rs:588-591` (upgrade writes a *new* `.psafe3`, the v2 file stays).

Attack. The attacker needs write access to the folder the vault is synced from —
a shared Drive folder, a compromised sync account, or anyone who kept a copy —
but **not** the master password:

1. Before the user upgraded, the vault existed as a v2 `.dat`/`.psafe3`
   (Password Gorilla). The attacker keeps a copy.
2. Later they write that copy over the current v3 file.
3. `Vault::decrypt` sees no `PWS3` magic, `looks_like_v2` accepts the shape, and
   the v2 file opens and checks out. The UI shows a warning banner, and editing
   is enabled.
4. Every later save re-encrypts **as v2**: single SHA-1 of `password || salt`
   for the data key plus the cheap `H(RND)` verifier, no HMAC at all.

Impact. From that point the on-disk vault is protected by an unstretched
SHA-1-derived key — several orders of magnitude cheaper to brute-force than the
262,144-round v3 KDF — and has no MAC, so tampering or corruption is
undetectable in principle (see "considered" 11 for what is practically
targetable). The user has no way to notice the downgrade beyond a banner they
must read on every open, and the app never upgrades the file on its own. The
rollback needs the v2 copy to predate any master-password change (otherwise the
file simply fails to open, which is a lockout/DoS rather than a downgrade).
A related confusion: because v2 detection is shape-only, *any* non-`PWS3` file
of the right length is reported as "Wrong master password", which is also a
usable social-engineering line ("the file is fine, it's your password").

Fix. Treat v2 as a read-only import format: open it, show it, but refuse to
edit/save until the user runs "Upgrade to v3…" (or an explicit "keep editing as
v2" opt-in per file). Remember the last format seen for a path and, on a
v3 → v2 transition, require an explicit confirmation naming the risk. When v2
editing is allowed at all, do not write it back without a fresh confirmation.
Separately, do not fall through from "not v3" to v2 on files that were opened as
v3 in this session.

### F2 — Medium — closing the window leaves the vault decrypted in the background by default, contradicting PLAN and the threat model

Where: `crates/keywhere-app/src/settings.rs:33` (`close_action:
CloseAction::Hide` is the default), `crates/keywhere-app/src/lib.rs:274-296`
(`close_window` keeps a `HiddenSession` holding the live `Vault`),
`lib.rs:504-517` (the hidden session only dies on the idle timer),
`settings.rs:45-48` + `settings.rs:47` (`idle_lock_minutes = 0` means *never*),
`crates/keywhere-app/src/i18n.rs:269` ("Hide, keep unlocked"),
`docs/PLAN.md:47` ("Closing the last window locks by default unless 'keep
running in tray' is on"), `docs/security/threat-model.md:130`.

Attack. A user closes the window on a shared or unattended machine and assumes
it is locked. It is not: the window is removed but the decrypted `Vault` is
retained in `AppState.hidden`, and the tray still exists. One tray click (or
`keywhere --show`, or any same-uid process sending `show` on the socket — see
F8) reopens the full vault with no password (`lib.rs:592-600` takes
`h.vault` verbatim). Only the idle timer protects the gap, and the settings
offer "never" (`IDLE_CHOICES` includes `0`), in which case `idle_lock()` returns
`None` and the vault stays decrypted in memory indefinitely.

Impact. Full disclosure of every stored password to anyone who reaches the
session: a passer-by, a screen-share, or a tray reachable from a lock screen on
desktops that allow it. The project's own plan and the threat model both say the
default must be lock-on-close, so this is a spec regression rather than a
deliberate residual risk.

Fix. Default `CloseAction::Lock` (and keep `Hide` as the opt-in), as
planned. If "Hide, keep unlocked" is chosen, require a confirmation that states
that the vault stays decrypted, and force a non-zero idle timeout for that mode
(refuse to combine it with "never"). Also check the idle deadline in the `Show`
handler before handing back `h.vault`.

### F3 — Medium — "open website" trusts the raw vault string: the visible URL can be the trusted host while the OS opens the attacker's

Where: `crates/keywhere-app/src/browser.rs:1898-1916` (the raw URL is rendered
and the link button appears when it starts with `http(s)://`),
`browser.rs:1241-1249` (same prefix test, then `cx.open_url(url)`),
`browser.rs:1777-1785` (the value column is `overflow_hidden` +
`text_ellipsis`, so long URLs are cut at the *end*),
`third_party/gpui/src/platform/linux/platform.rs:596-631` (the string is handed
to a WHATWG `Url::parse`, then the portal / `open::commands`).

Attack. Any entry author — a co-holder of the vault password in a shared vault,
or the user themselves after a phishing mail — can store a URL whose visible
part is a trusted host and whose parsed host is the attacker's:

- `https://accounts.google.com/<filler>\t@evil.example` — the detail row
  ellipsis hides everything after the filler, the tab is invisible, and WHATWG
  parsing strips it, giving userinfo `accounts.google.com/<filler>` and host
  `evil.example`. Verified: `new URL(...)` → `host = evil.example`.
- `https://аpple.com` (Cyrillic `а`) and `https://gооgle.com` (Cyrillic `о`)
  render pixel-identical to the real hosts and open `xn--pple-43d.com` /
  `xn--ggle-55da.com`. There is no punycode or mixed-script hint anywhere in the
  UI.
- `https://good.example@evil.example` needs no obfuscation at all; nothing in
  the UI marks the userinfo.

The same class applies to titles: `browser.rs:1670` and `browser.rs:1998`
render the vault string as-is, so U+202E/U+2066 and other bidi/control
characters can make a hostile entry look exactly like an existing one in the
list, the detail header and the delete confirmation.

Impact. Credential phishing from inside the password manager: the user clicks
the external-link icon on an entry that looks like their bank and lands on an
attacker page, with the vault's username/password one copy away. The prefix
check only guarantees the *scheme*; it says nothing about the host the browser
will actually be sent to.

Fix. Sanitise at the display boundary: strip C0/C1, bidi controls
(U+202A–U+202E, U+2066–U+2069, U+200E/F) and zero-width characters from every
vault-derived string used for layout. For URLs, parse once and render
`scheme://` + the normalised `host` (punycode) + a truncated path, and open the
*serialised* URL, not the raw field. Refuse (or warn loudly) when the host
contains non-ASCII/punycode or mixed scripts, when the URL carries userinfo, or
when the parsed host differs from what the label shows.

### F4 — Low-Medium — double-clicking an entry copies the password

Where: `crates/keywhere-app/src/browser.rs:1633-1641`
(`if ev.click_count() >= 2 { this.copy(rec::PASSWORD, …) }`).

Attack/impact. Double-click is the universal "open this" gesture; users will do
it on the list. The password then lands on the clipboard — excluded from history
only where the OS/desktop honours the hint (`sys/clipboard.rs:29-52`, KDE and
most GTK apps; many X11/Wayland clipboard managers, `xclip` consumers, remote
desktop clipboard sync and VMs do not). It stays there for the clear window
(default 30 s, `settings.rs:35`) and, per F7, may stay longer. A `copy` toast is
the only feedback, and it is the same toast as an intentional copy.

Fix. Make double-click open the editor (or do nothing); keep copying explicit
(Ctrl+C, the copy button, the context menu). If double-click-to-copy is kept,
at least make it copy the *username* and require a modifier for secrets.

### F5 — Low-Medium — a failed save is silently discarded by idle-lock, lock or quit

Where: `crates/keywhere-app/src/browser.rs:256-282` (a `Conflict`/I/O error
leaves the edit only in memory and shows a banner), `crates/keywhere-app/src/lib.rs:333-345`
(`lock` drops the `Browser` without asking), `lib.rs:504-517` (hidden session
auto-lock), `lib.rs:274-280` (close-without-tray quits).

Attack/impact. In a sync folder, save failures are routine: Drive holds the
file, replaces it mid-save, or the `.ibak`/temp rename hits a busy file. The
user changes a password on the website, the in-app save fails, they walk away,
the 5-minute idle timer fires and the `Vault` is dropped — the new password is
gone with no prompt, and the in-memory history entry too. An attacker with
folder write access can *force* this by continually touching the file, then let
auto-lock throw the edit away (a destructive denial of service against the
edit, not against the vault).

Fix. Track a dirty flag (set on every mutation, cleared on a successful save)
and refuse to lock/quit/close while it is set, offering "Retry", "Overwrite",
"Save as…" and "Discard". Auto-lock may keep the *plaintext* only if a
persistent draft is written first; otherwise it must prompt.

### F6 — Low — duplicate or missing record UUIDs make edits and deletes hit the wrong entry

Where: `crates/keywhere-core/src/lib.rs:673-675` (`find` returns the first
record whose UUID matches; the UUID is any `&[u8]`, any length),
`crates/keywhere-app/src/browser.rs:406-413` (`form.apply` on
`records[find(uuid)]`), `browser.rs:491-503` (delete removes
`records[find(uuid)]`), `browser.rs:405-412` (a record with no UUID is treated
as *new* and duplicated instead of updated).

Attack. Two records that share a UUID (a buggy merge tool, a hand-built file, or
someone with the password) or a record without one. The user selects entry B,
edits it, saves: the fields — including the new password — are written to entry
A, and the toast says "Saved". Deleting B deletes A. Editing a UUID-less entry
silently appends a copy.

Impact. The wrong credential is overwritten or removed; the user stores a new
password against the wrong account and may lock themselves out of the real one.
It is quiet, looks successful, and later saves make it permanent.

Fix. Validate on load: a UUID must be 16 bytes; duplicate UUIDs get a fresh one
(and are marked read-only until then) or the vault is flagged. Bind the editor
to both the index and the UUID and verify the UUID still matches at commit time.

### F7 — Low — the clipboard secret can outlive its timer, and the fallback path is unprotected

Where: `crates/keywhere-app/src/browser.rs:1136-1154`
(`clip_hash.take()`; if both `sys::clipboard::get_text()` and GPUI's read
return `None`, nothing is cleared and `clip_until` is dropped),
`crates/keywhere-app/src/sys/clipboard.rs:49` (`text.to_string()` — a plain
`String` handed to arboard, not zeroized), `browser.rs:1214-1220`
(`set_secret` failure falls back to `cx.write_to_clipboard`, which sets no
history/cloud exclusion hints), `browser.rs:138-141` (the only retry is the
1 Hz tick while `clip_until` is set).

Attack/impact. If reading the clipboard fails at expiry (backend gone, X11
selection owner lost, Wayland data-control hiccup), the password stays on the
clipboard forever and the app never retries. On the fallback path the password
is written with no exclusion hint, so clipboard managers archive it and, on
Windows/macOS, cloud clipboard may sync it. The allocator's own documentation
records a measured case at `0897388` where a copied password survived in the
clipboard backend's freed buffer (`alloc.rs:6-9`); the zeroize-on-free
allocator added in `9718fc9` mitigates that, but the clipboard code should not
depend on it.

Fix. If the clipboard cannot be read at expiry, treat the secret as possibly
still present: keep retrying, and clear unconditionally after a short grace
period. Do not fall back to an unprotected clipboard write for secrets — show
"copy failed" instead, or use a `Zeroizing` buffer and a platform writer that
sets the hints. Log/notify when the clear could not be confirmed.

### F8 — Low — the single-instance socket gives any same-uid process `show`/`lock`/`hide`/`quit`, and swallows a vault path argument

Where: `crates/keywhere-app/src/sys/instance.rs:124-142` (connect + write a
command), `instance.rs:145-187` (`show [token]`, `lock`, `hide`, `quit`),
`crates/keywhere-app/src/lib.rs:408-419` (`forward_to_running(show_command())`
runs before anything else, so the `path` argument is dropped),
`lib.rs:570-600` (`Show` focuses the search/password field, or reopens the
hidden session).

Attack. Any process running as the user — a downloaded script, an editor
plugin, a compromised Flatpak with socket access — can pop the vault window at
a chosen moment (training the user to type the master password into a window
that appeared by itself, or catching a screen share), lock or hide it (annoying
denial of service, and it discards unsaved edits per F5), or quit it. Separately,
`keywhere /path/to/other.psafe3` while Keywhere is running reports success and
shows the *current* vault instead, so a user who thinks they opened vault B is
looking at (and editing) vault A.

Peer-credential checks (`instance.rs:91-122`) and the private socket directory
(`instance.rs:60-89`) are correct and stop other users; the issue is authority
granted to *our own* uid and the missing `open <path>` command.

Fix. Forward the path as `open <path>` and open it in the running instance (or
refuse with a message). Require an activation token (or ignore the command) for
`show` when it would raise a window the user did not ask for; keep `quit`/`hide`
for the tray only.

### F9 — Low — a planted `.plk` in the sync folder pins the vault read-only and can spoof the "locked by" text

Where: `crates/keywhere-core/src/lock.rs:116-170` (`create_new` + 0600; on
`AlreadyExists` the content is read raw), `lock.rs:188-216` (`is_stale` needs the
same hostname and a dead pid), `crates/keywhere-app/src/browser.rs:2385-2399`
(the holder string is interpolated into a translated banner).

Attack. Write `vault.plk` containing `<user>@<this-host>:1`. `is_stale` checks
`kill(1, 0)`, which succeeds, so the vault opens read-only; the app also
auto-takes-over only *stale* locks (`browser.rs:151-156`). The holder string is
rendered verbatim, so it can carry bidi/control text and read like an
instruction ("… — click Take over to sync").

Impact. Persistent read-only denial of service for anyone with folder write
access, and pressure on the user to click "Take over lock" habitually, which
then overrides a *legitimate* lock from their other device (lost-update risk is
still bounded by the save hash check). No confidentiality impact.

Fix. Sanitise and quote the holder text; never treat a same-host pid we do not
recognise as authoritative without a confirmation; keep the "stale" takeover,
but require an explicit confirm for the live-pid case.

### F10 — Informational — supply chain: a full vendored gpui, no `cargo-deny`, CI toolchain not the pinned one

Where: `third_party/gpui` (3.2 MB copy of gpui 0.2.2 with no
`.cargo-checksum.json`; the only behavioural patch found is the zero-window
keep-alive at `platform/linux/x11/client.rs:250-252` and
`platform/linux/wayland/client.rs:387-389`), `third_party/xattr`,
`Cargo.toml` `[patch.crates-io]`, `.github/workflows/ci.yml:24` and `:39`
(`dtolnay/rust-toolchain@stable`) versus `rust-toolchain.toml` (channel
`1.95.0`), `ci.yml:53-62` (`cargo audit` only).

Impact. A change inside `third_party/gpui` is invisible to `cargo audit`, is not
checked against the crates.io tarball, and cannot be reviewed as a diff; the
app's largest dependency is effectively unauditable. `cargo-deny` (advisories,
crates.io-only sources, licence policy, duplicate `sha2`/`hmac`/`zeroize`) is a
"must" in `docs/security/threat-model.md:158` and is absent. CI installs
`stable` while the repo pins `1.95.0`, so either the pin is ignored or every run
re-resolves a toolchain the docs claim is fixed.

Fix. Keep the vendored tree as *patches* (`git format-patch` or a `diff -u`
against the published tarball, checked in and applied by a script) or move the
two-line change into a wrapper crate. Add `cargo-deny` with the four checks,
install the toolchain the repository pins, and ship `cargo-auditable` metadata
on release builds.

---

## Considered and rejected

Ordered roughly by how promising the idea looked before testing.

1. **Flip an unauthenticated field-kind byte to turn a stored password into a
   visible title.** `psafe3.rs:236-251` authenticates only field payloads, so
   `kind`/`len` are outside the HMAC. It does not work: a targeted plaintext
   flip at offset *p* requires modifying ciphertext at *p−16*, which corrupts the
   previous block — and that block always contains the tail of the previous
   field's payload (padding is at most 15 bytes), which *is* authenticated.
   Corrupting the field's own block destroys the kind/len instead of changing
   them, and any change that alters the span walk changes the MAC input. The
   first field's kind is at offset 4, so there is no preceding block to abuse.
   Result: an attacker without the password can destroy the file, not steer it.

2. **Malicious v3 file as a memory/CPU bomb.** Capped: 64 MiB ciphertext,
   32 MiB per field, 1e6 fields, `ITER` in 2048..2^24 (`psafe3.rs:39-47`), spans
   are walked before any field is materialised, and the KDF runs on a background
   executor with a 1–5 s backoff (`unlock.rs:88-101`). A crafted file can make
   the app work hard, but not freeze it, and the user chose to open it.

3. **Pin/unpin the vault through `.plk` semantics** — see F9; the stale case is
   handled correctly (`lock.rs:188-216`), the live-pid case is only a DoS.

4. **Symlink write-through at `vault.plk`, `vault.ibak`, the temp file or the
   vault itself.** Checked all three writers: `.plk` uses `create_new`
   (`lock.rs:128-135`), `take_over` unlinks the link itself
   (`lock.rs:178`), temp files use `create_new` + 0600 (`lib.rs:687-700`), and
   both replacements are `rename` over the name, which replaces a symlink
   instead of following it (`lib.rs:605`, `lib.rs:621`). No write-through.

5. **TOCTOU between the save hash check and `fs::rename`** (`lib.rs:614-621`,
   documented at `lib.rs:579-580`). Real window, but the attacker must already
   have write access to the folder — which lets them replace the vault at any
   moment — so it grants no new capability. The hash check does what it can.

6. **Planted `.dat`/`.psafe3` file that is neither v3 nor v2.** Falls through to
   "wrong master password" (F1's tail), i.e. a confusing DoS, not a bypass.

7. **Poisoning `~/.config/keywhere` (`settings`, `last-vault`) or the config
   symlinks.** `fs::write` is non-atomic and follows symlinks
   (`crates/keywhere-app/src/config.rs:25-43`), so a crash can truncate the
   settings (the app then falls back to defaults, i.e. safe direction) and a
   writer can point `last-vault` at an attacker-chosen file. All of it needs
   write access to the user's own config directory, i.e. same-uid code
   execution, which already owns the process. Not a boundary; only the
   non-atomicity is worth cleaning up.

8. **`--show` token as an injection or memory-safety vector.** The token is
   limited to 1–256 printable ASCII bytes (`instance.rs:23-25`) and only reaches
   `xdg_activation_v1.activate` (`sys/wayland.rs:88`). The surrounding code is
   the fragile part — a second `Backend`/`Connection` built from GPUI's foreign
   display and `ObjectId::from_ptr` (`wayland.rs:69-90`) — but the worst a bad
   token can plausibly do is a Wayland protocol error that drops our own
   connection, which `quit` already achieves. No confidentiality or integrity
   impact; worth a comment and a test, not a finding.

9. **Vendored-crate backdoor.** The two files that carry the documented patch
   were reviewed, and the tree was spot-checked for unexpected
   network/exec/obfuscation code; only the keep-alive and the `xattr` `ENOATTR`
   fix were found. That is not a full audit of 3.2 MB: the copy still cannot be
   verified cheaply against upstream — that is F10.

10. **Generator weaknesses.** `uniform()` is correct rejection sampling over
    `u32` (`generator.rs:110-123`), output is `Zeroizing`, the wordlist is
    loaded and counted (1296), and PIN mode is 10^n by construction.
    `entropy_bits` slightly overstates Random mode because it ignores the
    "every selected class present" resampling (`generator.rs:93-103` versus
    `:143-160`) — a fraction of a bit at length 20 — and `no_lookalikes` misses
    uppercase `L`, `5/S`, `2/Z`, `8/B` (`generator.rs:12`). Cosmetic.

11. **v2 CBC bit-flipping without the password.** Structurally possible (flip
    ciphertext block *i* to XOR plaintext block *i+1*, garbling block *i*), but
    without the plaintext the attacker cannot target a field, and any change to a
    length byte desynchronises the parse. They can already corrupt the file by
    writing it. The real v2 problem is F1.

12. **Secrets in GPUI's freed memory.** `SharedString` is `Arc<str>` and
    `secret_text` holds one reference so it can zeroize in place once GPUI drops
    its clones (`secret_text.rs:30-44`), with a 1 Hz sweep and a sweep on lock;
    the zeroize-on-free allocator added in `9718fc9` covers everything else that
    is freed process-wide. Notes are drawn through the same path despite not
    being masked (`editor.rs:122`, `field.rs:261-267`). Residual: a revealed
    password lives as long as GPUI's text layout cache holds it, and
    `copy_text`'s fallback allocates an unzeroized `String` (F7).

13. **Tray leakage.** The Linux tray menu is deliberately minimal
    (`sys/linux.rs:78-106`) and the tooltip only says locked/unlocked, as
    required. The tray surface is fine; the problem is that it can hold a live
    session at all (F2).

14. **`vault.ibak` restore, `.ibak` symlink, `.ibak` as a directory.** No restore
    code exists; a pre-created *directory* at `vault.ibak` makes every save fail
    (DoS), and a `.ibak` symlink is replaced, not followed. DoS only.

15. **Upgrade edge cases.** `new_path = path.with_extension("psafe3")`
    (`browser.rs:544`) plus the `exists()` check means a v2 file *named*
    `*.psafe3` can never be upgraded in place — it reports "already exists" about
    itself. A file created between the check and the rename is silently
    replaced (`lib.rs:392-396` + `save`). Cosmetic/UX; note only.

16. **No warning for a group/world-readable vault** (`docs/security/threat-model.md:167`,
    "should"). The file is recreated 0600 on every save (`lib.rs:687-700`), but
    an externally loosened file is used as-is at open. One-line fix, no
    attack chain found beyond the usual "sync client loosens the mode".

---

## Top 5 recommendations

1. **Make v2 an import format, not a working format.** Open v2 read-only,
   require the explicit upgrade before enabling edit/save, and remember the
   last-seen format so a v3 → v2 rollback is a blocking, explicit decision
   (F1). This is the only finding that turns one file swap into a lasting
   cryptographic downgrade.

2. **Default to lock on close, and make "keep unlocked" a deliberate,
   constrained opt-in** (F2). Reject `close_action = hide` together with
   `idle_lock_minutes = 0`, and re-check the idle deadline in the `Show`
   handler. The plan already promises this; the default is the whole
   difference between a decrypted vault and a locked one.

3. **Sanitise and normalise everything that comes out of the vault before it is
   drawn or opened** (F3). Strip control/bidi/zero-width characters at the
   display boundary, show the punycode host instead of the raw URL, and refuse
   or warn on userinfo and IDN/mixed-script hosts. This closes the in-app
   phishing primitive and the "identical twin entry" trick at the same time.

4. **Treat "changes not saved" as a blocking state, and keep secret copying
   explicit** (F4, F5, F7). Add a dirty flag that stops lock/quit/auto-lock,
   make double-click open rather than copy, and make the clipboard clear
   reliable (clear anyway after a grace period; never silently fall back to an
   unprotected clipboard write for a secret).

5. **Bring the supply chain up to the review's own "must" list, and make UUIDs
   trustworthy** (F6, F10). `cargo-deny` with advisories/sources/licences/bans,
   the pinned toolchain actually installed in CI, `cargo-auditable` on
   releases, and the vendored gpui kept as a reviewed patch series; plus
   duplicate/missing-UUID detection so an edit or delete can never land on the
   wrong credential.

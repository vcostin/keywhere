# Cursor review — Keywhere M1 (round 1)

Independent review of the read-only viewer milestone.
Diff base: `a75808f..HEAD` (`ef88c1c`, `1419b60`).
Checks: `cargo test --release` — 11/11 pass (9 core + 2 app). `cargo clippy --release --all-targets -- -D warnings` — clean.
Did not run the GUI. No source changes made. Screenshots inspected: `docs/screenshots/unlock.png`, `docs/screenshots/browser.png`.

## Verdict

**CHANGES REQUESTED**

M1 delivers a coherent unlock + 3-pane browse + search + copy/auto-clear + idle lock shell, and the vault drop / clipboard / URL / core-dump paths are largely right. One UI-layer security rule from the threat model is broken in the reveal and notes paths, so M1 is not approved yet.

## Blocking issues

1. **Passwords and other secrets are put into `SharedString` for on-screen display** ([`field.rs:132-138`](../../crates/keywhere-app/src/field.rs), [`browser.rs:468-470`](../../crates/keywhere-app/src/browser.rs), [`browser.rs:521`](../../crates/keywhere-app/src/browser.rs)).

   PLAN §3 and threat model §3 / MUST #6 forbid passphrase, password, and TOTP-class secrets in `SharedString`. The detail pane does `pw.to_string().into()` whenever reveal is on; `TextField` in secret mode does `SharedString::from(self.buf…)` when the eye is open; notes always go through `SharedString::from(...)`.

   In gpui 0.2.2, `SharedString` is an `Arc<str>` (`ArcCow`), not a process-global intern table — the PLAN’s “interned for the process lifetime” wording is slightly strong for this crate version. That does **not** make this OK: `Arc<str>` is not zeroized on drop, so reveal / notes copies survive in freed heap after hide, lock, or quit, and the project’s own rule is unconditional.

   **Fix:** Never build a `SharedString` (or other non-zeroizing owned string) from master-password, record-password, or notes plaintext. Keep the source in `Zeroizing` / vault `Field` data. For reveal, either (a) paint from a short-lived zeroizing buffer via a custom element / per-glyph path that does not Arc the secret, or (b) drop on-screen plaintext reveal for M1 and keep clipboard-only copy with the existing 15s/30s timers. Same for notes: a zeroizing display widget, or mask notes until an explicit reveal that uses the same non-`SharedString` path. Update the misleading “Short-lived” comment at `browser.rs:469` once the buffer actually zeroizes.

## Non-blocking suggestions

1. **Clipboard OS exclusion hints** (`browser.rs:196` TODO): auto-clear + hash compare + clear on lock/quit are correct for M1. Windows/macOS/KDE history hints remain MUST in the threat model but are scoped to `keywhere-sys` / M3 in PLAN — keep that explicit so CI/README don’t claim “clipboard-history exclusion” as done ([`README.md`](../../README.md) features list still says it).

2. **`run(..., preunlocked, ...)` footgun** ([`lib.rs:134-136`](../../crates/keywhere-app/src/lib.rs), [`screenshot_demo.rs`](../../crates/keywhere-app/examples/screenshot_demo.rs)): the shipping `main` passes `None`; the example is clearly DEV ONLY and is not part of the release binary. Prefer `#[cfg(feature = "devtools")]` (or `debug_assertions`) around `preunlocked` so a downstream binary cannot accidentally ship a pre-unlocked entrypoint.

3. **Master-password eye toggle has no reveal timeout** ([`field.rs:178-194`](../../crates/keywhere-app/src/field.rs)): browser password reveal is capped at 15s; unlock-field reveal stays until toggled. After the SharedString fix, still auto-mask the unlock field after a short timeout (or remove plaintext reveal there entirely).

4. **`secondary-c` while search is focused**: Browser `key_context` is an ancestor of the search field, so Ctrl/⌘C still runs `CopyPassword` (and Enter copies too). That matches the README and is fine because `TextField` has no selection/copy-out. Optional: only bind `CopyPassword` when the Browser focus handle itself is focused, if search-focused Ctrl+C feels surprising later.

5. **Idle lock is input-on-our-window only** ([`lib.rs:45-55`](../../crates/keywhere-app/src/lib.rs), [`lib.rs:113-115`](../../crates/keywhere-app/src/lib.rs)): correct vs threat model (do not listen globally). Screen-lock / suspend hooks are absent — expected for M3.

6. **Unlock backoff** (threat model SHOULD #19): failed unlock returns immediately with no delay. Add a small capped sleep before re-enabling the button.

7. **CI Linux deps** ([`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)): the apt set is in the right family for gpui 0.2.2 (xkbcommon, wayland, x11-xcb, fontconfig/freetype, vulkan, zstd). Other gpui CI recipes also pull `clang`, `libxcb-shape0-dev`, `libxcb-xfixes0-dev`, and sometimes `libasound2-dev`. Add those if the Ubuntu app job fails to link. Prefer `cargo … --locked` on all jobs (threat model MUST #12). macOS/Windows app jobs look fine (no extra system packages required for a compile/test).

8. **Core-dump hardening is Unix-only** ([`lib.rs:123-132`](../../crates/keywhere-app/src/lib.rs)): good `RLIMIT_CORE` + Linux `PR_SET_DUMPABLE`. Windows dump exclusion is still open (SHOULD).

9. **Icon-button tooltips are discarded** ([`browser.rs:422-423`](../../crates/keywhere-app/src/browser.rs)): `_ = tip;` — wire real tooltips when GPUI tooling allows, or drop the parameter.

10. **`TextField::backspace` via `String::pop`** leaves the popped byte in capacity until the next realloc/`clear`. Minor; prefer rebuilding a fresh `Zeroizing` buffer on delete for secret mode.

## Deferred to M2 / M3 (agree)

| Item | Milestone |
| --- | --- |
| OS clipboard concealment formats (Windows / macOS ConcealedType / `x-kde-passwordManagerHint`) | M3 (`keywhere-sys`) |
| Auto-lock on screen lock, suspend, fast-user-switch | M3 |
| Tray, global hotkey, lock-on-last-window-close policy | M3 |
| `mlock` / `VirtualLock` / `MADV_DONTDUMP` on secret pages | engine “next” / with M2+ |
| Edit/add/delete, generator, password history UI, conflict/`.plk` | M2 |
| Favorites / Weak smart views (in mockups; Weak tied to health report) | M4 (Weak); Favorites can land with M2 navigation polish |
| Command palette, IME/composition in `TextField` | later |
| Light theme, i18n, keychain unlock | M4 |
| `cargo-deny`, auditable/reproducible releases | M5 / supply-chain track |

## What looks good (for the record)

- Unlock stretches off the UI thread; passphrase is copied into `Zeroizing`, field cleared before the task, wrong-password error is generic ([`unlock.rs:62-90`](../../crates/keywhere-app/src/unlock.rs)).
- Lock replaces `Screen::Vault` with a new unlock view after `clear_clipboard_if_ours`; dropping `Browser` drops `Vault` → `Field` (`ZeroizeOnDrop`). No lasting `Entity<Browser>` from the shell after lock.
- Clipboard: SHA-256 compare before clear; 30s timer; clear on lock and `on_app_quit` ([`browser.rs:138-151`](../../crates/keywhere-app/src/browser.rs), [`browser.rs:90-94`](../../crates/keywhere-app/src/browser.rs)).
- URL open restricted to `http://` / `https://` ([`browser.rs:210-217`](../../crates/keywhere-app/src/browser.rs)).
- `EntryVm` keeps passwords/notes out of the list model ([`model.rs:8-18`](../../crates/keywhere-app/src/model.rs)).
- Search Enter → copy password, Escape → clear + refocus list; `uniform_list` + scroll handle are used appropriately.
- `screenshot_demo` is an example with an explicit env password; not linked from `main`.
- Patched `third_party/xattr` for `ENOATTR`/`ENODATA` is a reasonable gpui 0.2.2 workaround.
- UX vs mockups for M1: unlock and 3-pane browser screenshots match the navy/amber direction; Expiring smart view, groups, strength bars, clipboard countdown, and Lock control are present. Gaps that remain are mostly M2–M4 features above, not M1 blockers.

## Re-review

After the SharedString/secret-display fix (blocking #1), a short round-2 pass on the reveal/notes path and a re-run of `cargo test` / `clippy` should be enough for APPROVE.

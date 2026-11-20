# Cursor review — Keywhere v2 format (round 2)

Independent re-check of the round-1 blocker fix at HEAD `c2e654c` (`git show HEAD`). Author reply: [claude-response-v2-round1.md](claude-response-v2-round1.md). Round 1: [cursor-v2-round1.md](cursor-v2-round1.md).
Checks: `cargo test --release` — 39 passed (30 core + 9 app). `cargo clippy --release --all-targets --features devtools -- -D warnings` — clean. (Invoked via the stable toolchain `cargo` binary; rustup shims fail under this host’s argv0.)
Did not run the GUI. No source changes made. Did not touch `/home/roth/GoogleDrive`.

## Verdict

**APPROVE**

## Blocker status

**Resolved.** Round-1 blocker (`AppState.path` not updated after v2→v3 upgrade) is fixed:

- [`browser.rs`](../../crates/keywhere-app/src/browser.rs) `do_upgrade`: after a successful `upgrade_to_v3_at`, sets `self.path` to the new `.psafe3`, calls `config::remember_vault(&new_path)`, and emits `BrowserEvent::PathChanged(new_path)`.
- [`lib.rs`](../../crates/keywhere-app/src/lib.rs) `show_vault` subscription: on `PathChanged(p)` sets `AppState.path = Some(p.clone())`, same pattern as unlock.

**Tray Show / reopen paths no longer reopen the pre-upgrade `.dat`:**

- No hidden session: `handle_sys_event` → Show uses `AppState.path.or_else(config::last_vault)`; both are updated to `.psafe3` (global + `last-vault` file).
- Hidden session: `close_window` snapshots `Browser.path` into `HiddenSession`; that path is already the `.psafe3` after upgrade.
- In-window lock: `Shell::lock` builds unlock from `Browser.path`, which is already switched.

**Lock-failure banner:** `VaultLock::acquire` failure (or no lock taken) sets `save_error` with a clear message; the existing `saveerr` DANGER banner renders it. Non-blocking round-1 item taken as claimed.

## New blocking issues

None.

## Notes

- `self.path = …` only occurs in `do_upgrade` (aside from construction), so PathChanged covers the only session path move.
- Matching `_` on `VaultLock::acquire` also banners when another process holds the lock (`Ok((None, _))`) — correct for the read-only outcome.
- Left as previously noted (non-blocking / deferred): stale in-memory `utf8` after auto IsUTF8; upgrade no-op on already-v3; orphan `.psafe3` on failed post-rename `disk_hash` read.

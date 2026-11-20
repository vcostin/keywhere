# Cursor review — Keywhere M1 (round 2)

Independent re-review of the SharedString / secret-display fix and round-1 follow-ups.
Base: `HEAD` = `79be2b5` (diff `1419b60..79be2b5`). Author reply: `docs/reviews/claude-response-m1-round1.md`.
Checks: `cargo test --release` — 13/13 pass (9 core + 4 app, including `secret_text` tests). `cargo clippy --release -p keywhere --all-targets --features devtools -- -D warnings` — clean.
Did not run the GUI. No source changes made.

## Verdict

**APPROVE**

Nothing blocking remains for a read-only M1 viewer. The round-1 blocker is addressed with a sound registry + in-place wipe, wired on every secret display path and actually driven by the shell ticker / lock. Non-blocking items from round 1 that were in scope were taken; deferred M2/M3 items stay deferred.

## Blocker status

**Resolved.** Round-1 blocking #1 (passwords / notes / revealed master-password in a non-zeroizing `SharedString`).

### Fix soundness (`secret_text.rs`)

- `display` builds `Arc::from(secret)`, keeps a clone in `GRAVEYARD`, and hands GPUI the same `Arc` via `SharedString::from` (`ArcCow::Owned` — no content copy). Confirmed by `wipes_once_unreferenced` (`Arc::ptr_eq`).
- `sweep` uses `Arc::get_mut`; success means strong count is 1 (registry alone). `str::as_bytes_mut().fill(0)` is sound: unique owner, and all-NUL keeps valid UTF-8. The Arc is then dropped from the vec.
- Known residual (documented in PLAN §3 and the author’s reply): copies GPUI makes internally (e.g. shaper / glyph runs) and OS clipboard buffers are outside this registry. Acceptable for M1 given reveal timeouts, lock, and the 30 s clipboard clear.

### Call sites

| Path | Status |
| --- | --- |
| Revealed record password (`browser.rs`) | `secret_text::display(pw)` |
| Notes (`browser.rs`) | CRLF normalize into `Zeroizing`, then `display` |
| Secret `TextField` while revealed (`field.rs`) | `secret_text::display(&self.buf)` |
| Secret field masked | bullets only (not the secret) |
| Non-secret search field | plain `SharedString` (not a secret) |
| Username / URL / email | still plain `SharedString` (not in the round-1 secret class) |

No remaining path puts master-password, record-password, or notes plaintext into a bare `SharedString` / lasting non-zeroizing `String` for display. Clipboard still does `text.to_string()` for the OS write — expected, covered by hash-compare clear + lock/quit clear.

### Sweep reachability

- Shell idle loop ticks every **1 s** and calls `secret_text::sweep()` before the idle-lock check (`lib.rs`).
- `lock` calls `sweep()` immediately after replacing `Screen::Vault` with unlock (`lib.rs`).
- So reveal hide, navigation away, and lock all get a wipe within about a second once GPUI drops the frame’s `Arc`; lock also tries immediately.

## New blocking issues

None.

## Non-blocking notes

1. **Per-frame `display` while revealed / notes visible:** each `render` allocates and registers a new `Arc`. Between 1 s sweeps the graveyard can hold many unique copies of the same secret. Prefer caching one `SharedString` for the current reveal (or notes for the selected record) and only re-`display` when the text or reveal state changes.
2. **No `sweep` on app quit:** quit clears the clipboard but does not wipe the graveyard; process exit relies on the OS reclaiming pages. A final `sweep()` in `on_app_quit` would close that gap cheaply.
3. **Poisoned `GRAVEYARD` mutex:** if `lock()` fails, `display` still returns the `SharedString` without registering it, so that copy is never wiped. Unlikely unless a prior panicking holder; `expect` / `unwrap_or_else` recovery would be safer.
4. **PLAN wording drift:** PLAN §3 still says the master-password field is backed by `Zeroizing<Vec<u8>>`; the field is `Zeroizing<String>`. Behavior is fine; align the doc when convenient.
5. Round-1 deferred items remain deferred (Ctrl+C focus scoping, screen-lock/suspend hooks, Windows dump exclusion, M3 clipboard concealment, etc.) — no change needed for M1 approve.

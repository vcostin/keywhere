# Cursor review — Keywhere M3 (round 1)

Independent review of system integration (tray, single instance, lock-on-sleep/screen-lock, clipboard history exclusion).
Commit: `b0fc212` (diff `5a727ae..b0fc212` for `crates/` + `README.md`). Threat model: [`docs/security-review-grok.md`](../security-review-grok.md) §§4–7.
Checks: `cargo test --release` — 28 passed (22 core + 6 app). `cargo clippy --release --all-targets --features devtools -- -D warnings` — clean.
Did not run the GUI. No source changes made. Did not touch `/home/roth/GoogleDrive`.

## Verdict

**CHANGES REQUESTED**

M3’s Linux integration matches the threat-model shape: clipboard goes through arboard with `exclude_from_history` (Clipboard selection only, not primary), tray menu has no vault content, close locks then minimises when a tray exists, and logind `PrepareForSleep` / session `Lock` plus ScreenSaver `ActiveChanged` feed a single `SysEvent` channel. Author’s nested-KWin checks (hint MIME, clear-on-lock, ~3 ms second start, `0600` socket) are credible from the code. One cross-user socket issue on the `/tmp` fallback path is blocking.

## Blocking issues

1. **`/tmp` fallback socket dir is not ownership-safe — another local user can hijack startup or send `show`/`lock`** — [`crates/keywhere-app/src/sys/instance.rs:18-34`](../../crates/keywhere-app/src/sys/instance.rs) (`socket_path`) and [`forward_to_running`](../../crates/keywhere-app/src/sys/instance.rs) / [`listen`](../../crates/keywhere-app/src/sys/instance.rs).

   When `$XDG_RUNTIME_DIR` is unset or the path exceeds 100 bytes, the code uses `temp_dir()/keywhere-{uid}/keywhere.sock`. It `create_dir`s (errors ignored if the path already exists), then best-effort `chmod 0700` (fails if the caller does not own the dir), then binds/connects.

   On a sticky world-writable temp dir another user can pre-create `keywhere-<victim-uid>` (world-writable or a symlink to a directory they control). Then:

   - **Startup hijack:** if they bind `keywhere.sock` first, the victim’s `run()` → `forward_to_running("show")` connects, writes successfully, and **exits without starting** ([`lib.rs:231-233`](../../crates/keywhere-app/src/lib.rs)). Confirmed with a local AF_UNIX stand-in.
   - **Command injection:** if the victim binds inside the attacker-controlled directory, the attacker can connect and send `show` / `lock` (window activate or forced lock). Same-user sandboxes that can see `$XDG_RUNTIME_DIR` are out of scope per the threat model; **cross-uid** is not.

   The primary `$XDG_RUNTIME_DIR/keywhere.sock` path (0700 runtime dir) is fine; the 64-byte read cap and 2s read timeout on the listener are fine. Default umask leaves a freshly bound socket mode `0755` until `chmod 0600` ([`instance.rs:53-58`](../../crates/keywhere-app/src/sys/instance.rs)); that race only matters when the parent directory is traversable by others — i.e. the same fallback.

   **Fix (pick one stack):**
   - Prefer: refuse the unsafe fallback (abort single-instance / print a clear error) unless you can create a **new** `0700` directory you own; `lstat` the directory and require `S_ISDIR`, owner `== getuid()`, and mode `0700` (no group/other bits); reject symlinks (`O_NOFOLLOW` / `lstat`).
   - Before `bind`, set umask `0o077` (restore afterward) so the socket is never briefly world-accessible.
   - On connect (both `forward_to_running` and accept), check `SO_PEERCRED` / `getpeereid` and ignore peers whose uid ≠ ours (defense in depth on XDG too).
   - Do not treat “connect + write succeeded” as proof of a live Keywhere without peer-cred (or a short authenticated handshake).

## Non-blocking

1. **Clipboard secret buffers are not zeroized end-to-end** — [`clipboard.rs:49`](../../crates/keywhere-app/src/sys/clipboard.rs) `text.to_string()` into arboard / wl-clipboard-rs, plus the GPUI fallback [`browser.rs:603-604`](../../crates/keywhere-app/src/browser.rs). Expected for OS clipboard APIs; `get_text` correctly wraps in `Zeroizing`, and clear-on-timer/lock/quit remains the backstop. Fine for M3.

2. **`PrepareForSleep(true)` → async `SysEvent::Lock`** — lock runs on the GPUI executor after an unbounded channel hop ([`linux.rs:192-197`](../../crates/keywhere-app/src/sys/linux.rs), [`lib.rs:296-302`](../../crates/keywhere-app/src/lib.rs)). Threat model already notes the machine may freeze before the handler; `mlock`/disk encryption cover the gap. Timely enough for M3.

3. **Tray / D-Bus absence is handled** — `Tray::start` → `None` ⇒ `available() == false` ⇒ close quits ([`mod.rs:59-68`](../../crates/keywhere-app/src/sys/mod.rs), [`lib.rs:167-173`](../../crates/keywhere-app/src/lib.rs)). Logind/ScreenSaver watcher failures only `eprintln` and leave idle lock + manual lock. `ksni` features `async-io` + `blocking` match the blocking tray API; zbus `MatchRule` for session `Lock` is path-scoped to `GetSessionByPID` (falls back to sleep-only). Good.

4. **Close → lock + minimise** — correct for GPUI 0.2.2 (last window removed stops the Linux loop). Vault is locked before minimise; `.plk` drops with `Browser`. Show activates + `focus_search` / unlock focus; if the window handle is dead, a fresh **locked** window opens via `path` / `last_vault`. Note: `AppState.path` is set once at startup and not updated on unlock — mitigated by `or_else(config::last_vault)` and by minimise keeping the window alive.

5. **Tray does not leak vault content** — Open / Lock / Quit only; tooltip is “Vault locked/unlocked” only ([`linux.rs:63-106`](../../crates/keywhere-app/src/sys/linux.rs)). Matches threat-model “no titles/passwords”; unlocked icon state is the accepted discreet signal. Tray UI itself not tested here (headless).

6. **Non-Linux cfg / CI** — `instance` is `cfg(unix)`; tray + session watch are `cfg(linux)`; clipboard uses `SetExtLinux` / `SetExtWindows` / `SetExtApple` behind `target_os` gates; non-Linux depends on arboard without `wayland-data-control`. Looks compile-clean; CI builds the app on macOS and Windows ([`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)). Tray / lock-on-sleep remain TODO there (README says so).

7. **Thread / channel model** — instance, logind, screensaver, and ksni blocking tray each use dedicated threads; events enter GPUI via `futures::channel::mpsc::unbounded` + `cx.spawn` + `StreamExt::next`. Detached threads die with the process; `on_app_quit` removes the socket file. No crash hazard spotted. Many slow connections can stall only the instance thread (2s read timeout each), not the UI.

8. **README aspirational bullet** — features list still says “Tray icon with quick search, password generator, expiry and weak-password views” while M3’s tray is Open/Lock/Quit only. Tighten when convenient.

9. **Devtools** — `run_preunlocked` / `select` / `copy:` remain behind `feature = "devtools"`; socket listen is kept for `copy:` so tests can `lock`. Not in default release builds. No production leak path.

## Deferred

- macOS / Windows tray and lock-on-sleep / screen-lock hooks (`mod.rs` TODOs).
- True “hide to tray” if/when GPUI can keep the Linux event loop alive with zero windows (minimise is the current workaround).
- Zeroizing arboard/wl-clipboard internal buffers (OS/API limitation).
- Optional `SO_PEERCRED` on the XDG socket even after the fallback fix.
- Idle-lock dirty-editor discard (accepted since M2); Favorites / Weak / tray quick-copy titles (threat model allows titles later — not in this diff).
- `mlock` of key material (earlier deferral).

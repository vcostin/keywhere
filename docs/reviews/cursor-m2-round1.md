# Cursor review — Keywhere M2 (round 1)

Independent review of the editor milestone (everything after `e2c9d2d`).
Commits: `e5ef150`, `524b021`, `de85708` (Grok follow-ups). Diff scope: `crates/`, `README.md`.
Checks: `cargo test --release` — 27 passed (22 core + 5 app). `cargo clippy --release --all-targets --features devtools -- -D warnings` — clean.
Did not run the GUI. No source changes made. Did not touch `/home/roth/GoogleDrive`.

## Verdict

**CHANGES REQUESTED**

Grok’s core MUST/SHOULD fixes in `de85708` are present and look correct (conflict window, `.plk` ownership, wide lock encoding, history flag / non-BMP, `encode` iteration floor). App edit paths generally respect `read_only()`, secrets in the editor use `Zeroizing` + `secret_text`, TextField caret math is UTF-8-safe under its cursor invariant, and screenshots match the mockup closely enough for M2. One silent **group-path corruption** bug on save is blocking.

## Blocking issues

1. **Group field round-trip splits on every `/`, corrupting segment names that contain a slash** — [`crates/keywhere-app/src/editor.rs:181-188`](../../crates/keywhere-app/src/editor.rs) (`form`) and display via `group.join(" / ")` in `EntryEditor::new` / `EntryVm`.

   Core correctly uses `.` with `\.` escaping ([`set_group_path`](../../crates/keywhere-core/src/lib.rs) / `group_path`). The UI then joins segments with `" / "` and re-parses with `.split('/')` + trim. Any Password Safe / Gorilla group segment that contains `/` (e.g. `CI/CD`) becomes multiple segments on the next Save — even if the user only changed the password — because `Form::apply` always rewrites `GROUP`.

   Example: stored `Work.CI\/CD` → displayed `Work / CI/CD` → parsed `["Work","CI","CD"]` → written `Work.CI.CD`.

   **Fix:** Parse/format with a delimiter that does not appear inside segments, or escape `/` in the text field. Simplest improvement: split on the literal `" / "` (and join the same way), and treat a typed `Foo/Bar` without spaces as a single segment. Better: keep an internal `Vec<String>` for the path and only use the text field as a view with escaping. Add a round-trip test with a segment containing `/`.

## Non-blocking suggestions

1. **“Discard & reload” locks instead of reloading** — [`browser.rs:1396-1398`](../../crates/keywhere-app/src/browser.rs) emits `LockRequested`, and [`lib.rs:117`](../../crates/keywhere-app/src/lib.rs) drops the vault and returns to the unlock screen. That discards the unsaved in-memory edit (good) but does not reload the file while staying unlocked; the user must re-enter the master password. The `persist` comment even says “Overwrite / Lock & reload”. Rename the button to “Discard & lock” or implement a real reload (`Vault::open` with retained `KeyMaterial` is impossible without the password — so either keep the password ephemerally for reload, or unlock-gate and rename honestly).

2. **Unsaved editor drafts are dropped with no prompt** on Lock, idle auto-lock ([`lib.rs:53-56`](../../crates/keywhere-app/src/lib.rs)), and `Ctrl+N` / + New while the editor is already open (`start_edit` replaces `self.editor`). Vault bytes are fine; the draft is not. Consider refusing New while dirty, or a confirm dialog (idle lock may still hard-discard for security).

3. **`DuplicateEntry` / `NewEntry` while the editor is open** — Delete is gated with `editor.is_none()` ([`browser.rs:1281-1284`](../../crates/keywhere-app/src/browser.rs)); Duplicate and New are not. Ctrl+D persists a duplicate under the open editor; Ctrl+N silently replaces the form.

4. **`commit_editor` does not re-check `read_only()`** — [`browser.rs:257`](../../crates/keywhere-app/src/browser.rs). Safe today because only `start_edit` opens the editor and that checks the lock, but a belt-and-braces check (and on the delete confirm continuation) would close a future hole.

5. **Delete keybinding context is correct** — [`lib.rs:204-208`](../../crates/keywhere-app/src/lib.rs) uses `Browser && !TextField && !Editor`, and the Delete action also requires `editor.is_none()`. Good. Ctrl+C / Ctrl+B remain scoped only to `Browser` (not excluding `TextField` / `Editor`), so they still fire while typing in the editor and copy the *selected vault entry*, not the field — same M1 deferral; more surprising now that an editor exists.

6. **TextField caret** — [`field.rs`](../../crates/keywhere-app/src/field.rs): `splice` / `prev_boundary` / `next_boundary` / `line_start` / `line_end` / `vertical` all assume `cursor` is on a char boundary (documented). Under that invariant there is no panic on UTF-8 (including emoji). Click-to-place caret is still missing (focus only). No IME. Acceptable for M2; a `debug_assert!(buf.is_char_boundary(cursor))` in `splice` would make mistakes loud.

7. **Secrets hygiene in the editor** — `Form.password` / `notes` and `gen_preview` are `Zeroizing`; generator preview and revealed secret fields go through `secret_text::display`. Password `TextField` buffer is always zeroizing. Unlock still clears the field after submit. Residual: per-frame `display` while the generator is open (same graveyard growth noted in M1). Fine for M2.

8. **Duplicate semantics** — [`browser.rs:298-318`](../../crates/keywhere-app/src/browser.rs): new UUID via `Record::new()`, skips `UUID` / `CTIME` / `MTIME` / `PTIME` / `PW_HISTORY`, copies other fields, titles with ` (copy)`. Correct: no shared UUID, no copied history.

9. **`persist` error paths** — Conflict keeps the change in memory and sets the banner; other errors set `save_error`; success toasts and clears flags. Editor is closed before `persist`, so a failed save still leaves the edit applied in the `Vault` (status bar “Unsaved change”). Good.

10. **Interop of what the editor writes** — Empty text fields removed (`set_or_clear`); expiry as little-endian `u32` via `parse_iso_date` (1970–2105); `MTIME` always updated on apply; `PTIME` only when the password actually changes (`set_password` early-return). Notes CRLF not normalized on write (detail view normalizes for display) — Password Safe often uses `\r\n`; consider writing `\r\n` for notes if peers care. `.` in group segments still escaped by core.

11. **UX vs mockups / screenshots** — [`docs/screenshots/editor.png`](../screenshots/editor.png) and [`generator.png`](../screenshots/generator.png) match the three-pane shell and the inline generator (Random / Passphrase / PIN, chips, entropy, Use password). Mockup tray + Favorites / Weak are out of M2. Detail “Generate new” on the expiry banner and password-history reveal in the mockup meta line are not required; history is a masked, copy-only list — good security choice.

## Grok M2 core — verification

| Finding | Status in tree |
| --- | --- |
| MUST: conflict hash before `.ibak` + again before `rename`; `disk_hash` from post-rename read | Done in `save_inner` ([`lib.rs:378-413`](../../crates/keywhere-core/src/lib.rs)); test `conflict_detected_even_without_prior_check` |
| MUST: one `VaultLock` per path; Drop only if same inode + identity; failed write unlinks | Done ([`lock.rs`](../../crates/keywhere-core/src/lock.rs)); no `flock` (author’s choice; inode check covers the described race) |
| SHOULD: UTF-8 / UTF-16LE / UTF-32LE lock decode; write wchar_t | Done (`encode_wide` / `decode_holder` + tests) |
| SHOULD: history append only if status `1` / absent; skip non-BMP and len > 0xFFFF | Done (`set_password` + `history_respects_disabled_flag_and_skips_non_bmp`) |
| SHOULD: `encode` rejects weak iterations | Done (`psafe3.rs` floor check) |
| SHOULD: `entropy_bits` overestimate | Still accepted / M4 |

Residual race between the pre-rename hash and `rename` remains as documented; `.plk` is local-only; content hash is the sync-folder safety net. Agreed.

## Deferred (agree for M3+)

- Clipboard OS history-exclusion hints (`browser.rs` TODO / `keywhere-sys`).
- Idle lock on sleep / screen lock (PLAN); prompt-or-save for dirty editor on intentional Lock may stay M2 follow-up, but suspend hooks are M3.
- Tray, Favorites / Weak smart views, drag-and-drop groups (PLAN M2 listed drag-drop; not in this diff — treat as later).
- IME / dead keys in `TextField`; click-to-position caret; undo.
- `mlock` of key material; `KeyMaterial` doc says “Zeroized on drop” but only `p_prime` is `Zeroizing` (salt left as non-secret header material — OK, tighten the comment).
- `.plk` naming for rare `*.cfg` vaults (`with_extension` vs Password Safe append) — Grok SHOULD; product is `.psafe3`.
- Synced `.plk` remains advisory; UI Take over is enough.
- Generator `entropy_bits` vs rejection sampling — M4 health.
- Per-frame `secret_text::display` caching while generator / reveal is open.

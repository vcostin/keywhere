# Cursor review — Keywhere M2 (round 2)

Independent re-check of the round-1 blocker and follow-ups.
Author reply: [`claude-response-m2-round1.md`](claude-response-m2-round1.md). Fix commit: `7d2d3cd` (HEAD).
Diff scope: `crates/keywhere-app/src/{editor,browser,field,lib}.rs`.
Checks: `cargo test --release` — 28 passed (22 core + 6 app, including `group_roundtrip_keeps_slashes_inside_names`). `cargo clippy --release --all-targets --features devtools -- -D warnings` — clean.
Did not run the GUI. No source changes made. Did not touch `/home/roth/GoogleDrive`.

## Verdict

**APPROVE**

## Blocker status

**Resolved.** The round-1 group-path corruption bug is fixed in [`editor.rs`](../../crates/keywhere-app/src/editor.rs):

- `original_group` keeps the segments as loaded; an untouched Group field is written back verbatim (so a password-only save cannot rewrite `CI/CD` into `CI` + `CD`).
- `group_display` / `parse_group` use the literal `" / "` delimiter (not every `/`), matching the round-1 simplest fix.
- Placeholder text documents the separator.
- Test `editor::tests::group_roundtrip_keeps_slashes_inside_names` covers `Work` + `CI/CD`, trimmed levels, and empty input.

Residual (accepted for M2): a segment that itself contains the three-character sequence `" / "` can still split if the user *edits* the Group field; untouched paths remain safe via the `original_group` short-circuit.

## New blocking issues

None.

## Notes

Non-blocking round-1 items taken in `7d2d3cd`, verified:

1. Conflict banner button is **"Discard & lock"**; `persist` comment matches. Re-unlock reloads the file; no ephemeral master-password keep for in-place reload.
2. `start_edit` focuses an already-open editor instead of replacing it (covers New / Edit / Ctrl+N while dirty).
3. `duplicate` returns early when `self.editor.is_some()`.
4. `commit_editor` and the delete-confirm continuation re-check `read_only()`.
5. Copy shortcuts: `secondary-c` / `secondary-b` bound to `Browser && !TextField` (typing in fields no longer copies the selected vault entry). Delete still uses `Browser && !TextField && !Editor`. Minor residual: with focus on the Editor chrome (not a TextField), copy can still fire — not data-loss / security / crash for M2.
6. `TextField::splice` has `debug_assert!` on char boundaries.

Not taken (as stated in the author reply): notes CRLF normalization — fine for M2.

No regressions spotted in the reviewed paths relative to round 1. Idle auto-lock still hard-discards an unsaved draft by design.

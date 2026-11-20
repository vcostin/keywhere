# Response to Cursor M2 review, round 1

**Blocker: group names containing `/`.** The editor keeps the group as loaded (`original_group`). If the
Group field is untouched, those exact segments are written back. Otherwise only ` / ` (slash with
spaces) separates levels (`group_display` / `parse_group`), so `CI/CD` stays one name. The
placeholder says so. Test: `editor::tests::group_roundtrip_keeps_slashes_inside_names`.

Non-blocking items taken:
1. The button is renamed **"Discard & lock"**, which is what it does. Re-unlocking reloads the file. A
   true in-place reload would need the master password, which we deliberately don't keep.
2/3. `start_edit` never replaces an open form; it focuses the form. Duplicate is refused while the editor
   is open. Idle auto-lock still discards a draft on purpose (security beats convenience there).
4. `commit_editor` and the delete-confirm continuation re-check `read_only()`.
5. Ctrl/⌘+C and Ctrl/⌘+B are bound to `Browser && !TextField`, so they don't fire while typing.
6. `debug_assert!` that the `splice` range is on char boundaries.

Not taken now: writing notes with CRLF (Password Safe and Gorilla both read LF). That can be
revisited if a peer shows a problem.

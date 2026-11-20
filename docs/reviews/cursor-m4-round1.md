# Cursor review — Keywhere M4 (round 1)

Independent review of milestone M4 (“polish”) and UX work since M3.
Range: `c2e654c..HEAD` (`9ba8c4f`). Scope: `crates/`, `assets/`, `README.md`, `docs/PLAN.md`, `rust-toolchain.toml`.
Prior approvals: M0–M3 and v2 format (`docs/reviews/cursor-*.md`). Owner decision: no OS keychain unlock (`docs/PLAN.md`).
Checks: `cargo test --release` — 44 passed (31 core + 13 app). `cargo clippy --release --all-targets --features devtools -- -D warnings` — clean.
Did not run the GUI. No source changes made. Did not touch `/home/roth/GoogleDrive`.

## Verdict

**APPROVE**

Settings stay free of vault content; reuse detection hashes passwords only for the duration of `model::build`; empty-group header `0x11` prune/delete semantics match Password Safe’s “drop the marker once the group has records (or descendant records)”; favorites on `0xe0` are a documented Keywhere extension that unknown-field round-trip already preserves; Select / group combobox keyboard handling and read-only menu gating look correct; no new panic/index bugs turned up in the group tree or post-`take_session` paths under the existing “window is closing” contract.

## Blocking issues

None.

## Non-blocking

1. **Toasts store English literals, so Romanian never sees them** — [`browser.rs`](../../crates/keywhere-app/src/browser.rs) `persist` uses `self.toast("Saved")` and `take_over_lock` uses `self.toast("Lock taken over — editing enabled")` without `i18n::t`. Both keys exist in [`i18n.rs`](../../crates/keywhere-app/src/i18n.rs). The status-bar “Saved” path already goes through `t`; fix the two toast call sites the same way (translate at emit time).

2. **`ACCENT_INK` has no light-theme mapping** — [`theme.rs`](../../crates/keywhere-app/src/theme.rs) `light()` maps `ACCENT` / palette / frame accents but not `ACCENT_INK` (`0x2A1A00`). Primary buttons in light mode keep dark-on-amber ink that works by accident; add an arm (and extend `light_mapping_differs_for_every_palette_color`) so a future palette tweak cannot leave unreadable contrast.

3. **Hostile / hand-edited `idle_lock_minutes` can disagree with the Select** — [`settings.rs`](../../crates/keywhere-app/src/settings.rs) accepts any `u32` up to 24×60; [`settings_view.rs`](../../crates/keywhere-app/src/settings_view.rs) `idle_index` falls back to the 5‑minute chip when the value is not in `IDLE_CHOICES`. Auto-lock still uses the parsed value. Clamp on parse to `IDLE_CHOICES` (or snap in `get`) if you want the UI and the timer to always match.

4. **Clipboard clear duration is snapshotted at copy time** — [`browser.rs` `copy_text`](../../crates/keywhere-app/src/browser.rs) sets `clip_until` from `settings::get().clipboard_clear()` once. Changing “Clear clipboard after” while a secret is on the clipboard does not shorten/extend the active timer. Acceptable; document or re-read settings in `tick` if you want live updates.

5. **Read-only group context menu can be empty** — when `read_only()` and the group has no children, New / Subgroup / Delete are gated off and Expand/Collapse is also absent (`has_children` false), so right-click opens an empty panel. Hide the menu entirely in that case, or keep a disabled “Read-only” row.

6. **Select filter UI is indicator-only** — typing filters options ([`select.rs`](../../crates/keywhere-app/src/select.rs)) but there is no focused text field / screen-reader name for the filter buffer (only a `⌕ …` hint). Fine for language/theme lists; worth a proper filter field if the control is reused for longer lists.

## Deferred

1. **Favorites on record field `0xe0`** — deliberate Keywhere extension ([`rec::KW_FAVORITE`](../../crates/keywhere-core/src/lib.rs)); `is_favorite` only treats data `b"1"` as on; unknown fields (including this type) already round-trip. [`docs/security-review-grok.md`](../security-review-grok.md) asks writers to *preserve* `0xe0`–`0xfe` from other clients — starring therefore overwrites any foreign payload already at `0xe0`. No evidence Password Safe uses that type; revisit only if another client collides.

2. **Empty groups are not re-created as `0x11` when the last entry leaves a group** — same as Password Safe: markers are pruned when records appear; deleting the last record removes the group from the tree unless an empty-group header remains. No change requested.

3. **v2 empty groups are session-only** — [`create_group`](../../crates/keywhere-app/src/browser.rs) skips `persist` on v2 and toasts; `encrypt` for v2 ignores the header. Correct and already disclosed in the UI.

4. **Fold state is in-memory only** — intentional (group names are vault content; must not land in `~/.config/keywhere/settings`).

5. **`Option<Vault>` / `expect("vault taken")` after `take_session`** — unchanged M3 contract: window is removed immediately after the take. `tick` / clipboard clear do not touch the vault. Residual same-thread race only if a render ran after take; not introduced by M4.

## Security notes (reviewed, clean)

| Area | Finding |
|---|---|
| Settings file | Only idle / clipboard / close / language / theme. No titles, groups, or secrets. Parse ignores unknown keys; clipboard clamped 5–120 s. |
| Reuse detection | SHA-256 digests of non-empty passwords inside `model::build`; counts → `reused_with`; digests dropped when `build` returns. Passwords never enter `EntryVm` / haystack. |
| Socket | Dev harness uses `keywhere-dev.sock` via `use_dev_socket`; production socket still peer-cred + trusted-dir gated. Commands remain `show` / `lock` / `hide` / `quit` only. |
| Favorites / secret UI | Star toggle is `read_only`-gated; no new plaintext password surfaces in settings. Generator preview and reveals still go through `secret_text`. |
| Clipboard | Clear delay now from settings; clear still only if clipboard hash matches what we copied. |

## Correctness notes (reviewed, clean)

- **`prune_empty_groups`**: removes `0x11` when any record path `starts_with` the empty path (covers the group itself and descendants) — aligns with Password Safe.
- **`delete_empty_group`**: refuses if any entry’s group starts with the path; also removes nested empty-group headers under that path; rebuild clears a now-missing group filter.
- **i18n**: English keys + Romanian column; missing keys fall back to English; `t`/`tf` call-site keys match the table (including non-breaking hyphens). Live language/theme apply via `settings_view::change` → `apply_setting` / `theme::apply` + `window.refresh()`, and Select option labels are relabelled.
- **Group combobox Enter/Esc**: Enter picks the highlighted suggestion when the menu has a choice, otherwise saves; Esc closes the menu first, then cancels the editor. Editor-level ↑↓ move the highlight without stealing field input otherwise.
- **Group tree indexing**: fold visibility walks ancestor prefixes; `has_children` uses BTree neighbour order; `pl` uses `path.len() - 1` only for non-empty paths from `groups()`.

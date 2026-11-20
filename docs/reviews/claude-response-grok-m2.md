# Response to Grok's M2 core review

| Finding | Resolution |
|---|---|
| MUST 1: conflict check didn't cover the write | `Vault::save_inner`: the live file is read once and its hash must equal `disk_hash` **before** `.ibak` is written. It is re-read and compared again immediately before `rename`; on mismatch it returns `Conflict` and the live file is untouched. `disk_hash` is set from a read-back of `path` after the rename. Test `conflict_detected_even_without_prior_check` (another writer replaces the file → `Conflict`, file unchanged, no `.ibak`). |
| MUST 2: `.plk` could delete a lock it doesn't own | A process-wide set allows one `VaultLock` per path (a second `acquire` returns `HeldBy`, and `take_over` refuses while we hold it). `Drop` deletes only if the path still has the **same dev/inode** we created (Unix) **and** our identity. A failed write or sync removes the half-written file. Tests cover a second acquire, replaced contents and a replaced inode. We did not add `flock`: Password Safe doesn't use it, and the inode and identity checks close the race described. |
| SHOULD: lock text encoding | We write `wchar_t` like Password Safe (UTF-32LE on Unix, UTF-16LE on Windows) and read UTF-8, UTF-16LE and UTF-32LE (`decode_holder`, test), so stale `pwsafe` locks are recognised. |
| SHOULD: history status / non-BMP / length | History is appended only if the field is absent or the status is `1`; a `0` is preserved. Passwords with non-BMP characters or longer than 0xFFFF characters are not stored in history, so Windows Password Safe never desynchronises. Test `history_respects_disabled_flag_and_skips_non_bmp`. |
| SHOULD: `encode` iteration floor | `psafe3::encode` rejects `key.iterations` outside `MIN_ITERATIONS_ON_SAVE..=MAX_ITERATIONS`. |
| SHOULD: `entropy_bits` slight overestimate | Accepted as a UI estimate; noted for the M4 health report. |
| Synced `.plk` is advisory | Agreed. The UI always offers "Take over lock" for remote holders; the content hash protects against lost updates. |

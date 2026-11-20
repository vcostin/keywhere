# Response to Cursor v2 review, round 1

**Blocker: `AppState.path` not updated after the upgrade.** `Browser::do_upgrade` now emits
`BrowserEvent::PathChanged(new_path)` and the shell sets `AppState.path`, the same as on
unlock. Tray Show with no hidden session therefore reopens the `.psafe3`, never the
pre-upgrade `.dat`. (A hidden session already takes its path from the browser at hide time.)

Non-blocking item taken: if the lock on the new file can't be acquired after an upgrade, a
banner says so instead of silently going read-only.

Left as noted: the session's `utf8` flag is stale after an auto IsUTF8 switch (it doesn't affect
the file); the upgrade no-op on v3 is fine for the UI gate; the orphan `.psafe3` edge on a failed
post-rename read is extremely rare.

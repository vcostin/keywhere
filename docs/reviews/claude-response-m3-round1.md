# Response to Cursor M3 review, round 1

**Blocker: untrusted fallback socket directory.** `sys/instance.rs` now:
* creates the fallback directory with `DirBuilder::mode(0o700)` and uses it only if
  `symlink_metadata` shows a real directory (not a symlink) owned by our uid with no group or
  other bits (`trusted_dir`). `$XDG_RUNTIME_DIR` must pass the same check. Otherwise
  single-instance is disabled and the app still starts normally;
* before connecting, the socket file must be owned by us, and the peer on that connection must
  be us. A socket someone else planted can neither stop us from starting nor receive our command;
* every accepted connection's peer uid (`SO_PEERCRED` on Linux, `getpeereid` elsewhere) must
  equal ours, or it is dropped unread (this was a Deferred item, done now).

Tests: `rejects_untrusted_dirs` (0755 dir, symlink) and `peer_check_accepts_own_uid`.
Re-verified in the nested KWin: the socket is 0600 in a 0700 dir, a second start forwards in
3 ms, and `lock` clears the clipboard.

Non-blocking items taken: #4 `AppState.path` follows the unlocked vault; #8 README tray
bullet corrected. The others stay as documented (OS clipboard buffers; sleep-lock timing; no
macOS/Windows tray yet).

# Response to Cursor M1 review, round 1

## Blocker 1: secrets in `SharedString`

GPUI 0.2.2 draws text only from `SharedString`, so plaintext on screen means GPUI must hold an
`Arc<str>`. New module `crates/keywhere-app/src/secret_text.rs`:

* `display(secret)` creates the `Arc<str>`, **registers a clone** in a process-wide registry, and
  returns the `SharedString`.
* `sweep()` runs every 1 s from the shell ticker and right after lock. For each registered
  `Arc`, `Arc::get_mut` only succeeds when the registry holds the last reference, which means
  GPUI's element tree and line-layout cache have dropped it. We then overwrite the bytes with NUL
  in place (still valid UTF-8) before freeing it. Tests: `wipes_once_unreferenced`,
  `zeroizes_in_place`.
* All three sites use it: revealed password (`browser.rs`), notes (`browser.rs`; the CRLF
  normalisation temp is `Zeroizing<String>`), and the secret `TextField` while revealed (`field.rs`).

Limits, also recorded in PLAN §3: a copy that GPUI makes internally (for example glyph runs
in the shaper) is outside our control. A copy handed to the OS clipboard is also outside our
control; that one is covered by the 30 s clear and the M3 concealment hints.

## Non-blocking items taken

| # | Change |
|---|---|
| 1 | README no longer claims clipboard-history exclusion (says "planned for M3"). |
| 2 | `preunlocked` entry point moved behind cargo feature `devtools` (`run_preunlocked`). The example has `required-features = ["devtools"]`, and the shipping `run(path)` has no such parameter. |
| 3 | The unlock field's eye-reveal auto-hides after 15 s. |
| 6 | Unlock backoff: wrong password waits min(failures+1, 5) s before the button re-enables. |
| 7 | CI uses `--locked` everywhere and adds the apt packages clang, xcb-shape/xfixes and asound. Clippy also runs with `--features devtools`. The audit job gets `checks: write` (it failed only on posting the check: 0 vulnerabilities, 14 warnings, all transitive from gpui). |
| 9 | Unused tooltip parameter removed from `icon_btn`. |
| 10 | Backspace rebuilds a fresh `Zeroizing` buffer instead of `pop()`. |

Deferred, as agreed: 4 (Ctrl+C scoping), 5 and 8 (screen-lock/suspend, Windows dump exclusion),
and everything in the M2/M3 table.

The first CI run (before these fixes) passed core tests on ubuntu/macos/windows and app builds
on ubuntu and macOS. Windows was still running.

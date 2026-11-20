# RAM residue test — results (2026-11-20, Linux/KDE Wayland)

Method: `tools/memtest/scan.py` launches Keywhere (dev build) as a child,
reads `/proc/<pid>/maps` + `/proc/<pid>/mem` (≈ 614 MB scanned), and searches
for the master password and all 20 passwords of the demo vault (UTF‑8 and
UTF‑16LE).

| Scenario | Unlocked | 4 s after lock | 10 s after lock |
|---|---|---|---|
| Release hardening (`PR_SET_DUMPABLE=0`) | memory unreadable (EACCES), even for the parent | unreadable | unreadable |
| Real unlock path (password typed into the field) | master password **absent**; each vault password **exactly once** | **0 / 21** | **0 / 21** |
| Copy a password, then lock — before the zeroizing allocator | — | 1 vault password left (clipboard backend's freed buffer) | 1 left |
| Copy a password, then lock — with `ZeroizeOnFree` allocator | — | **0 vault passwords** | **0** |

Notes
* Only the stretched key is kept while unlocked; the master password itself is
  wiped right after key derivation.
* `alloc.rs` installs a global allocator that zeroes every freed block, so
  copies made by libraries (clipboard backend, GPUI text caches, `String`
  growth) don't survive in freed heap.
* Outside the process: the compositor holds the clipboard selection until it
  is cleared (30 s by default, on lock and on quit); clipboard managers are
  told to ignore it (`x-kde-passwordManagerHint`, etc.).
* Not covered by this test: swap/hibernation (see the memory audit), GPU
  memory, and other processes' copies.

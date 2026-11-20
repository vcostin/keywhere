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
| Reveal a password, then lock — before the layout fix | — | 2 copies of the revealed password (live `Arc<str>`) | 2 |
| Reveal a password, then lock — with the layout fix | — | **0 vault passwords** | **0** |

Notes
* Only the stretched key is kept while unlocked; the master password itself is
  wiped right after key derivation.
* `alloc.rs` installs a global allocator that zeroes every freed block, so
  copies made by libraries (clipboard backend, GPUI text caches, `String`
  growth) don't survive in freed heap.
* The reveal residue was a dependency bug: taffy 0.9's `TaffyTree::clear()`
  keeps its `node_context_data`, so GPUI's text‑measure closures (each owning
  a clone of the displayed `SharedString`) outlived their frame until their
  slot was reused. A locked screen has fewer nodes, so they lived forever and
  `secret_text` could never wipe them. The vendored GPUI now builds a fresh
  layout tree per frame (`third_party/gpui/src/taffy.rs`), and lock also clears
  GPUI's line‑layout cache.
* Outside the process: the compositor holds the clipboard selection until it
  is cleared (30 s by default, on lock and on quit); clipboard managers are
  told to ignore it (`x-kde-passwordManagerHint`, etc.).
* Not covered by this test: swap/hibernation (see the memory audit), GPU
  memory, and other processes' copies.

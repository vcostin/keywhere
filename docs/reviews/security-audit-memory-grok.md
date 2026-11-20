# Memory and endpoint audit (Keywhere)

Status: code read of the tree as of this review. No GUI run. Threat model: `docs/security-review-grok.md`. Question: can vault secrets be fished from RAM, hardware, or other software on the machine?

Verdict: after `harden_process`, a later same-user process on Linux cannot `ptrace`, `process_vm_readv`, or read `/proc/<pid>/mem`. That does not cover launch-time injection, the input method, the clipboard, or anything that was painted. Lock zeroizes `Field` and `KeyMaterial::p_prime`. It does not zeroize GPUI’s line-layout copy, cosmic-text’s shape buffer, arboard’s offer, or KWin’s clipboard. Sleep locking is signal-only and can lose the race. `mlock` is still absent. macOS and Windows do not lock on sleep or screen lock.

## 1. Same-user malware

`harden_process` (`crates/keywhere-app/src/lib.rs`) runs at `run_inner` start, before any vault exists. On Linux it sets `RLIMIT_CORE` current and max to 0 and `prctl(PR_SET_DUMPABLE, 0)`. Dumpable is an mm flag, so every thread is covered. The kernel’s `ptrace_may_access` then returns `EPERM` for a same-uid caller who lacks `CAP_SYS_PTRACE` in the target user namespace. That check applies to `PTRACE_ATTACH`, `process_vm_readv` (`PTRACE_MODE_ATTACH_REALCREDS`), and `/proc/<pid>/mem` (`PTRACE_MODE_ATTACH_FSCREDS`). Yama does not weaken it. With `ptrace_scope=0` the parent is still refused after `prctl`. Arch’s usual `ptrace_scope=1` only adds a second restriction (ancestors, or `PR_SET_PTRACER`) for processes that are still dumpable. `/proc/<pid>/maps`, `environ`, and `fd` follow the same gate. Root, and a process that already has `CAP_SYS_PTRACE`, still can read the memory. The target can turn dumpable back on if it is exploited.

What dumpable does not stop:

- Launch. `LD_PRELOAD` / `LD_AUDIT` run in the dynamic linker before `main`. A same-user process can also replace `PATH`, the binary, or `~/.local/share/applications/*.desktop` (`Exec=env LD_PRELOAD=…`). A parent can `ptrace` in the window before `prctl`; preload already won. No check of `/proc/self/maps` will fix this.
- The control socket (`crates/keywhere-app/src/sys/instance.rs`). Directory mode, ownership, and `SO_PEERCRED` keep other uids out. The peer uid is ours, and the commands are only `show`, `lock`, `hide`, and `quit`. There is no “read password” command. `show` raises the window (useful together with screen capture). `hide` parks an unlocked `HiddenSession` until the idle timer. That is control, not a memory read.
- Tray D-Bus (`crates/keywhere-app/src/sys/linux.rs`, ksni). The menu is Open / Lock / Quit. Tooltip is “Vault locked” or “Vault unlocked”. Any session peer can call those methods and read the tooltip. None of them returns a secret. A second SNI named `keywhere` can phish a click; it does not receive this process’s vault.
- Screen-saver and logind signals. `PrepareForSleep(true)`, logind `Lock` on our session path, and `ActiveChanged(true)` only enqueue `SysEvent::Lock`. `false` is ignored, so a spoof cannot unlock. Session-bus `ActiveChanged` is unauthenticated (any peer can force a lock). System-bus sender is set by the daemon; spoofing logind well enough to inject a secret is not this code’s bug. Missing sender filters are a nuisance, not a leak.
- Wayland keys. GPUI takes `wl_keyboard` only for the focused surface. Another client cannot keylog a native Wayland window. GPUI still binds `zwp_text_input_v3` and, on enter, calls `enable` with `ContentHint::None` and `ContentPurpose::Normal` (`third_party/gpui/src/platform/linux/wayland/client.rs`). It does not send surrounding text. The compositor’s IME (fcitx5, IBus, a replaced one) still sees every key, including the master password. `TextField` never marks the field sensitive. On X11, and on XWayland for X11 clients, `XRecord` / `XISelectEvents` let any client keylog the session. Keywhere on XWayland is an X11 client if `WAYLAND_DISPLAY` is unset; GPUI prefers Wayland when it is set.
- AT-SPI. Vendored GPUI has no AccessKit / AT-SPI registration. The a11y bus does not get password text. A later accesskit port must mark secret fields protected (`ATK_STATE_SENSITIVE` off / password role).

Windows: `OpenProcess` + `ReadProcessMemory` works for the same user. There is no user-mode equivalent of dumpable. macOS: `task_for_pid` fails for a same-user process without the debugger entitlement; `DYLD_INSERT_LIBRARIES` works until the binary is signed with the hardened runtime and library validation.

## 2. What survives lock

Lock (`Shell::lock`) drops `Browser`, which drops `Vault`. `Field` is `ZeroizeOnDrop`, `KeyMaterial.p_prime` is `Zeroizing<[u8; 32]>`, and the passphrase `Zeroizing<Vec<u8>>` is cleared in the unlock field before stretch and dropped when the background task finishes. K and L are not retained. `EntryVm` keeps titles and usernames, not passwords or notes. History rows on screen are dates; `Record::history` builds `Zeroizing` passwords and drops them at the end of the statement. The raw history field inside `Field` is wiped with the vault.

Copies that are not wiped:

| Copy | Lifetime after lock | Practical mitigation |
| --- | --- | --- |
| glibc free list of a `Zeroizing` buffer | Bytes already zero | Done. Keep secrets in `Zeroizing` / `Field`. |
| Free list of a plain `String` / `Arc<str>` | Until malloc reuses the chunk | Stop creating those copies (below). |
| `secret_text` graveyard `Arc<str>` | Until GPUI drops its clone and `sweep` runs (1 s tick, plus once in `lock`) | Works only for that Arc. |
| GPUI `CacheKey.text` | Current frame and previous frame, then `free` with the secret intact | `layout_line` does `SharedString::new(text)`, a second allocation the graveyard never sees (`third_party/gpui/src/text_system.rs`, `line_layout.rs`). Zero it in `CacheKey`’s drop. |
| cosmic-text `ShapeLine` | Freed at the end of `layout_line`, unzeroized, on every cache miss | One block per reshape. No crate API to wipe it. Accept, or patch the dep. |
| Shaped glyph-id sequence in `LineLayout` | Same two-frame cache | Dropping `CacheKey` drops this too. Zero the key text; glyph ids are a second reconstruction. Zero the `runs` vec on drop if you touch that file. |
| Glyph atlas (VRAM) | Process lifetime, unordered rasters | Not the string. Leave it. |
| Swapchain + KWin’s last buffer | A few frames | Lock must present one non-secret frame before suspend continues (§4). |
| arboard / wl-clipboard-rs `Source::Bytes` | Until `clear`, then unzeroized free | `set_secret` moves `text.to_string()` into the offer (`clipboard.rs`). `clear` drops it. Cannot zero inside arboard 3.7. |
| KWin via `ext-data-control` / wlr-data-control | Live selection until `clear`; history skipped if `x-kde-passwordManagerHint: secret` is honored | Feature `wayland-data-control` pushes the bytes into the compositor. Hint does not stop a peer reading the live clipboard. Kernel pipe buffers are freed, not wiped. No userspace fix. |
| Editor notes | Whole edit, plain `SharedString` | `TextField::new(..., false)` in `editor.rs`. Bullets and `secret_text` are not used. |
| Notes on the detail card | While the entry is selected | `secret_text::display`, then the layout cache copies them anyway. |
| Revealed password | Up to 15 s, then the same cache | Masked path paints bullets only. Good. |
| `clip_hash` | SHA-256 while the clip is live | Not plaintext. Dropped in `clear_clipboard_if_ours`. |
| Twofish schedule, SHA-256 state | Stack during decrypt/save | `twofish` 0.7 has no `zeroize`. Gone once the UI thread reuses the stack. Not a post-lock heap copy. |

`mlock` does not cover the table rows that are not our allocations (GPUI, cosmic-text, GPU, KWin).

## 3. Swap, zram, hibernation, core dumps

`mlock` keeps those pages out of swap and zram. It does not keep them out of a hibernation image: the kernel writes mlocked RAM into the image. `memfd_secret` (Linux 5.14+, `secretmem` enabled) removes the page from the kernel direct map and locks it. It still sits in DRAM for cold boot and for DMA. It counts against `RLIMIT_MEMLOCK`. It does not replace dumpable.

Do not mlock the GPUI heap. Cap the locked set at the key plus password/notes/TOTP/history fields, and stop on `ENOMEM`.

- Linux, P′ first (one page): `mmap(MAP_PRIVATE|MAP_ANONYMOUS)` + `mlock` + `madvise(MADV_DONTDUMP)` + `madvise(MADV_WIPEONFORK)`. `MADV_DONTDUMP` is redundant with dumpable 0 and still helps if dumpable is reset. `MADV_WIPEONFORK` zeros the page in a child so a forked helper does not inherit P′. If `memfd_secret` succeeds, use that mapping for P′ instead; on `ENOSYS` / `ENOMEM`, fall back to `mlock`. Unprivileged `RLIMIT_MEMLOCK` is often 64 KiB (old pam) or 8 MiB (systemd user). A vault at the 64 MiB plaintext cap will not fit. Warn once, keep `Zeroizing`. Do not raise `rlim_max` (needs `CAP_SYS_RESOURCE`). No new crate: a small `unsafe` module in `keywhere-core`, as the threat model already allows. `memsec` is unmaintained; skip libsodium.
- macOS: `mlock` the same pages. `setrlimit(RLIMIT_CORE, 0)` is already set for all Unix. Skip `ptrace(PT_DENY_ATTACH)`; it is trivial to bypass and `task_for_pid` is already gated. FileVault covers the sleep image at rest.
- Windows: `CryptProtectMemory(..., CRYPTPROTECTMEMORY_SAME_PROCESS)` on P′ and secret fields (length multiple of 16). The blob in a dump, pagefile, or `ReadProcessMemory` is ciphertext; only this process can unprotect. Also `SetProcessWorkingSetSizeEx` then `VirtualLock`, or `VirtualLock` fails quietly against the default working set. `VirtualLock` still lands in `hiberfil.sys`. BitLocker is the control for that file. There is no documented way for an unsigned app to refuse same-user `MiniDumpWriteDump`.
- Core dumps today: Linux is in good shape (`RLIMIT_CORE` 0 and dumpable 0; the kernel will not invoke systemd-coredump). macOS gets `RLIMIT_CORE` 0 only. Windows gets nothing.

## 4. Physical attacks and suspend

Cold boot and DMA read DRAM. `mlock` and `memfd_secret` do not stop them. One zeroize pass on lock is the userspace control; cooling can sometimes recover the previous pattern anyway. IOMMU / Thunderbolt DMA protection is a kernel and firmware setting. Evil maid (firmware, replaced binary) is outside this process; dumpable and `mlock` do not apply.

Stolen machine:

- Screen locked, vault locked, zeroize finished: disk encryption is the barrier. Leftovers are §2.
- Suspend-to-RAM: DRAM stays powered. If lock did not finish, the vault is intact for the whole sleep. GPU buffers often stay powered too.
- Hibernate, swap encrypted with a passphrase typed at resume: the image is protected at rest.
- Hibernate or disk with TPM auto-unlock: power-on decrypts the image. Whatever was in RAM is on disk.

`PrepareForSleep` handling is not timely. `watch_logind` sends `SysEvent::Lock` on the bool `true` and returns. logind does not wait unless this process holds a **delay** inhibitor. With none, the CPU can freeze as the signal is still in the channel (`linux.rs` → `lib.rs` `handle_sys_event`). Stretch itself is off the UI thread, so the UI can lock if it gets the turn. It often will not get one.

macOS and Windows: `watch_session` is a stub (`sys/mod.rs`). Lid close does not lock.

What to do on suspend: take logind `Inhibit("sleep", "Keywhere", "Lock the vault", "delay")` for the process lifetime cycle; on `PrepareForSleep(true)`, lock, present one frame of the unlock UI (so the swapchain and KWin drop the revealed frame), then close the inhibitor fd so the machine can sleep; re-arm on `PrepareForSleep(false)`. `InhibitDelayMaxSec` is 5 s by default. Holding the fd across the sleep delays every suspend by that cap, so release it after the frame. macOS: `IORegisterForSystemPower` and call `IOAllowPowerChange` only after that same lock-and-repaint. `NSWorkspace` will-sleep is not a barrier. Windows: handle `PBT_APMSUSPEND` on the UI thread and finish zeroize inside the ~2 s budget.

## 5. Screen capture

Revealed passwords and editor notes are ordinary pixels. PipeWire, the desktop portal, and Spectacle do not tell the target that capture has started. Gating reveal on “sharing is active” will miss `kmsgrab` and will false-positive. Leave reveal as an explicit 15 s action.

Always-on OS flags, not a detector:

- Windows: `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` on vault windows (10 2004+). The capture sees black. A camera still works.
- macOS: `NSWindow.sharingType = .none`.
- Linux: no equivalent on core Wayland. Mask by default (already), short reveal (already), and do not put secrets in the title (already “Keywhere”).

Titles, usernames, URLs, and notes are on screen whenever the vault is open. Capture gets those without reveal. Notes in the editor are the worst of these because they are unmasked for the whole edit.

## 6. Changes

**MUST**

1. `sys/linux.rs`: logind delay inhibitor; release it only after lock and one non-secret present. Re-arm on resume.
2. `third_party/gpui/src/text_system/line_layout.rs`: zero `CacheKey.text` (and shaped glyph buffers) on drop. That is the copy `secret_text` cannot see.
3. `editor.rs`: notes field `secret: true` (same bullets / `secret_text` path as the password). Today it is `secret: false`.
4. `sys/mod.rs` plus a macOS and a Windows file: lock on sleep and on screen lock before those builds are shipped. Linux-only today.

**SHOULD**

5. `keywhere-core` platform module: `mlock` + `MADV_DONTDUMP` + `MADV_WIPEONFORK` for P′ and secret field bytes, `memfd_secret` for P′ with `mlock` fallback, warn once on `ENOMEM`. Windows: `CryptProtectMemory` and `VirtualLock` after raising the working set. macOS: `mlock`.
6. Vendored Wayland client: `ContentPurpose::Password` and `ContentHint::SensitiveData` while a secret `TextField` is focused; `disable` the text-input object on the master-password field if composition stays unimplemented.
7. Windows display affinity and macOS `sharingType = .none` on the vault window.
8. `browser.rs` detail rows: username, URL, and email go through `secret_text` (or stay borrowed) so lock does not leave a fresh `String` per paint.
9. User-facing note: full-disk encryption and encrypted swap; hibernate with TPM auto-unlock keeps RAM; X11 sessions are keyloggable; a malicious `.desktop` / `LD_PRELOAD` is game over.

**COULD**

10. Warn once when the Linux backend is X11.
11. Match logind signals on the name owner’s unique name. Spoofing can force a lock, not read a secret.
12. Static tray tooltip so the session bus does not advertise unlocked state.
13. `sha2` zeroize-on-drop for stretch state, if the feature wipes the hasher and not only `Output`. `twofish` 0.7 cannot wipe its schedule without a fork.

Accepted, no code: same-user malware that starts Keywhere; live clipboard readers; KWin’s copy of a selection; kernel pipe buffers; glyph-atlas rasters; cosmic-text’s one freed shape buffer; DMA and evil maid.

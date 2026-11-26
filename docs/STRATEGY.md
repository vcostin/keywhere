# Keywhere strategy

From a product review, 2026-11. The roadmap that follows from it is in
[PLAN.md §7](PLAN.md#7-roadmap-after-v016).

## Positioning

**Keywhere is a calm desktop app for Password Safe v3 and Password Gorilla vaults**: one local
file, no cloud, no account, no nags. It doesn't try to be a general password manager that
competes on features.

| Pitch that works | Pitch that doesn't |
|---|---|
| A maintained, calm app for your `.psafe3` vault | A modern password manager |

**Main audience:** people who already use a Password Safe or Password Gorilla vault, mainly on
Linux and macOS, and people leaving the discontinued Password Gorilla.

**Second audience:** people who want one local file and a quiet UI, and accept the psafe3 format
as it is.

Public text (README, website, docs) describes the format and the scope. It doesn't name or
compare other password managers.

## Product thesis

Be good at **everyday desktop use of an existing vault**: unlock, browse, search, edit, generate,
copy, lock, save safely, work well in a synced folder.

Feature parity with other password managers is not a goal. The file format is the reason to pick
Keywhere, and it is also the way out: the same file still opens in Password Safe, Password Gorilla
and other psafe3 apps, so trying Keywhere costs nothing.

## What "better" means

Raise trust and usefulness **inside the niche**, without widening the scope.

| Goal | Meaning |
|---|---|
| Stay compatible | Open and save files Password Safe and Gorilla read; keep unknown fields |
| Feel better every day | Calm UI, keyboard first, health views, fewer dialogs |
| Be safe for real vaults over time | Solid engine, honest security language, a growing track record |
| Linux first | Linux is the reference platform; macOS and Windows improve as time allows |

## Focus order

1. **Clear, honest language (cheap, does the most).** Say who Keywhere is for. Name the
   non‑goals. Call the AI reviews "AI‑assisted reviews", not audits. For phones, point to
   psafe3‑compatible mobile apps in general instead of building one.
2. **The everyday desktop loop.** Browse, search, edit, drag and drop, generator, health views,
   settings, tray and lock. Keep Linux rock solid. Fix macOS and Windows gaps when they break
   daily use (Windows single instance, the macOS hardware pass), not as a signing project.
3. **Safe files in synced folders.** Atomic save, `.ibak`, conflict detection and `.plk` are a
   strength. Document "keep the file in your sync folder; Keywhere notices conflicts".
4. **Security engineering that fits the threat model.** Keep fuzzing, audit/deny CI, the RAM
   test, constant‑time checks and clipboard hygiene current. Close the gaps that are code, not
   certificates (Windows `VirtualLock`). Reproducible builds belong here: they are evidence
   anyone can check, and they cost far less than signing.
5. **Time.** Longevity is part of the product: small releases, a clear changelog, a security
   advisory channel, and round trips with real vaults. Dependency updates count as product work.

## Only when someone asks

A GitHub issue is the signal; there is no telemetry.

| Item | Note |
|---|---|
| CSV import | Helps the second audience try Keywhere |
| CSV export | Writes every password in plain text: needs a warning and a mode‑0600 file. Less urgent, because the psafe3 file already opens elsewhere |
| KDBX import | Only on real demand |
| Flatpak | Packaging polish |

## Deferred

**Code signing and notarization for macOS and Windows.** It's useful and can be added at any
time, but it costs money. The current audience accepts the first‑start warnings of unsigned open
source apps. Revisit when there's a budget or a concrete distribution need.

## Not planned

- Browser autofill
- A mobile app
- TOTP codes
- Passkeys
- YubiKey or other hardware unlock
- Cloud sync, accounts, sharing, family plans
- Competing on feature count

A silent gap looks unfinished; a named non‑goal looks intentional.

## Platform risk

The biggest long‑term risk is the UI stack, not the engine. GPUI has had no crates.io release
since 0.2.2, Keywhere carries vendored patches, and some dependency majors (RustCrypto) are held
back for it. The choice between staying on GPUI and moving to Tauri 2 waits until Discord publishes
its GPUI migration, which should show whether GPUI gets real releases. `keywhere-core` has no UI
dependency and stays either way.

## Security stance

- **Design:** modern and right for a local psafe3 client: RustCrypto, authenticate before parse,
  safe save, lock and clipboard hygiene, supply‑chain CI.
- **Assurance:** still young. No human audit and a short track record; that is the real gap.
- **Format limits stay:** the iterated SHA‑256 KDF and the v3 HMAC quirks come with the format.
  Changing them would break compatibility with Password Safe.

How security improves without spending on signing:

1. Honest docs
2. CI, fuzzing and RAM test evidence kept current
3. Reproducible builds
4. A small paid review of `keywhere-core` when there's a budget
5. Signing later, as a distribution upgrade, not a condition for correctness

See [SECURITY.md](../SECURITY.md) and [docs/security](security/).

## Validation

| Layer | Role |
|---|---|
| Public engineering evidence | Compatibility tests, fuzzing, RAM test, threat model |
| Users | Round trips with real Password Safe and Gorilla vaults |
| Paid core review (optional) | Parser, KDF, save, lock, clipboard, when affordable |
| Full firm audit | Later, after real usage and a frozen core |
| Code signing | A separate trust channel; deferred |

Nobody certifies password managers. Credibility comes from honest claims, evidence anyone can
reproduce, and time without incidents.

## In one sentence

**Be the best maintained, calm desktop home for Password Safe and Gorilla vaults; earn trust with
clarity and engineering, not with more features or early signing spend.**

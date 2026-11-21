<p align="center"><img src="assets/keywhere-key.svg" width="96" alt="Keywhere logo"></p>
<h1 align="center">Keywhere</h1>
<p align="center">A modern, cross‑platform password manager for <b>Password Safe v3</b> (<code>.psafe3</code>) vaults.<br>
Works with files from Password Safe and Password Gorilla. No cloud account, no subscription, no nags.</p>

> **Status: early development (M3, tray & system integration).** The vault engine is tested against Password Safe;
> the app can unlock, browse, search, copy, add, edit, duplicate and delete entries, and generate
> passwords. Every save keeps the previous file as `.ibak`. Keep backups anyway.

<p align="center"><img src="docs/screenshots/browser.png" width="820" alt="Keywhere vault browser"></p>

<p align="center"><img src="docs/screenshots/generator.png" width="820" alt="New entry with password generator"></p>

## Using it

**Linux:** from [Releases](https://github.com/vcostin/keywhere/releases/latest) take
* the **AppImage** (any distribution; updates itself): `chmod +x Keywhere-*.AppImage` and run it, or
* the **.deb** (Debian, Ubuntu 22.04+) / **.rpm** (Fedora, openSUSE): install with your package
  manager (`sudo apt install ./keywhere_*.deb`, `sudo dnf install ./keywhere-*.rpm`, or
  `sudo zypper install --allow-unsigned-rpm ./keywhere-*.rpm`).

**macOS and Windows, preview:** not code-signed yet, so the OS asks before the first start.
* macOS (`Keywhere-*-macos-arm64.dmg` for Apple silicon, `-macos-x64.dmg` for Intel): open the
  .dmg, drag **Keywhere** to Applications, then run once in Terminal

  ```sh
  xattr -dr com.apple.quarantine /Applications/Keywhere.app
  ```

  (or open it once and choose System Settings → Privacy & Security → **Open Anyway**).
* Windows (`Keywhere-*-windows-x64-portable.exe`): a single program, nothing to install. Put it
  anywhere (even a USB stick) and run it; at the SmartScreen warning choose **More info → Run
  anyway**. Settings go to `%APPDATA%\keywhere`.

`SHA256SUMS` on the release lists the checksums: check the file before you allow it to run.

From source:

```sh
cargo run --release -p keywhere -- path/to/vault.psafe3
```

| Shortcut (Ctrl → ⌘ on macOS) | Action |
|---|---|
| `Ctrl+F` / `Ctrl+K` | Search |
| `↑` `↓` | Move selection |
| `Enter` in search, double‑click | Copy password |
| `Ctrl+C` / `Ctrl+B` | Copy password / username (clipboard clears after 30 s) |
| `Ctrl+R` | Reveal password for 15 s |
| `Ctrl+U` | Open website |
| `Ctrl+N` / `Ctrl+E` / `Ctrl+D` | New / edit / duplicate entry |
| `Delete` | Delete entry (asks first) |
| `Enter` / `Ctrl+Enter` in editor | Save (`Esc` cancels) |
| `Ctrl+L` | Lock (also automatic after 5 min idle) |

### Tray, global shortcut and auto‑lock (M3)

* **Tray icon** (Linux: KDE, GNOME with the AppIndicator extension, etc.): left‑click opens
  Keywhere; the menu has *Open*, *Lock* and *Quit*. It never shows entry titles.
* **Closing the window hides Keywhere completely** (no taskbar entry) and keeps the vault unlocked in
  the background; the tray icon brings it straight back. The hidden session still auto‑locks after
  5 min, on screen lock / sleep, via tray → *Lock*, and on quit. Without a tray, closing quits.
* **Global shortcut:** Keywhere is single‑instance. Bind any key to the command `keywhere --show`
  (KDE: *System Settings → Keyboard → Shortcuts → Add New → Command*; GNOME: *Settings → Keyboard →
  Custom Shortcuts*). It brings Keywhere to the front with search focused, or the unlock field if locked.
* **Auto‑lock** after 5 min idle, when the screen locks (logind / ScreenSaver) and before the system
  sleeps.
* **Clipboard:** copied secrets are marked *exclude from history* (KDE Klipper hint, Windows
  history/cloud exclusion, macOS concealed type) and cleared after 30 s, on lock and on quit.

macOS/Windows: auto‑lock on screen lock, user switch and sleep (Windows also when the display
turns off), clipboard protection and the tray icon (menu bar on macOS, notification area on
Windows: Open / Lock / Quit) work; the window is kept out of screen captures. Closing the window
hides Keywhere to the tray as on Linux; on macOS the Dock icon brings it back.

### Settings and language

The gear button (or `Ctrl+,` / `⌘,`) opens **Settings**: auto‑lock time (1–60 min or never),
clipboard clear time (5–120 s), what closing the window does (lock and hide — the default — or
hide and keep unlocked), and the **language** (System, English, Română). Settings apply immediately and are stored in
`~/.config/keywhere/settings` (never secrets). Translations live in
`crates/keywhere-app/src/i18n.rs`; adding a language is one column there.

### Updates

On first start Keywhere asks whether it may check GitHub for new versions at startup; nothing
happens on the network until you answer. Settings → Updates has the same switch and a **Check
now** button. When a new version is out, the AppImage offers **Update and restart**: it downloads
the new AppImage, checks it against a manifest signed with Keywhere's release key (the public key
is built into the app), swaps it in place, locks and restarts. Copies installed by a package
manager or built from source only get a notice with a link. Details:
[SECURITY.md](SECURITY.md#updates) and [docs/releasing.md](docs/releasing.md).

### Password Gorilla / Password Safe v2 files

Keywhere opens legacy **v2** files (Password Gorilla's `.dat`) and **saves them back as v2**, so
older apps keep working with the same file. Keywhere still shows and stores every field (URL,
e‑mail, history…); older apps simply don't display the extras. Text is written as UTF‑8 (Gorilla
honours this; Password Safe 3 reads it with a UTF‑8 system locale). A banner explains v2's
weaknesses (unstretched SHA‑1 key, no tamper detection) and offers **Upgrade to v3…**, which writes
`name.psafe3` next to the original and switches to it, leaving the v2 file untouched.

Saving is safe for synced folders: Keywhere refuses to overwrite a vault that changed on disk
since you opened it (another device, Password Safe, Gorilla) and asks you what to do. It also
honours Password Safe's `.plk` lock file and opens read‑only while another app has the vault.

## Why

* **Password Gorilla** is discontinued and its Tcl/Tk UI shows its age.
* **Password Safe** is solid but cluttered.
* **Strongbox** looks great but is macOS‑only and keeps pushing Pro.

Keywhere keeps the format you already have and adds a calm, keyboard‑first UI built with
[GPUI](https://gpui.rs) (the GPU‑accelerated Rust UI framework behind Zed).

## Features (planned → see [docs/PLAN.md](docs/PLAN.md))

- Open/save `.psafe3` files, interoperable with Password Safe 1.x and Password Gorilla
- Three‑pane browser: smart views, group tree, instant search, detail card
- One‑click copy with clipboard auto‑clear and clipboard‑history exclusion
- Auto‑lock on idle, sleep and screen lock
- Tray icon (Open / Lock / Quit), `keywhere --show` for a global shortcut, password generator, expiry view
- Linux, macOS, Windows

Mockups: [docs/mockups/keywhere-mockups.html](docs/mockups/keywhere-mockups.html)

## Security

Threat model: [docs/security/threat-model.md](docs/security/threat-model.md). The engine uses the
RustCrypto crates (`twofish`, `cbc`, `sha2`, `hmac`), constant‑time checks, authenticate‑before‑parse,
input size caps, zeroized secrets and an atomic, verified save with a `.ibak` backup.
Please report vulnerabilities privately via GitHub Security Advisories.

## Build

```sh
cargo test -p keywhere-core      # vault engine tests
cargo run --release -p keywhere  # desktop app (Linux needs wayland/x11, xkbcommon, vulkan dev libs)
```

On macOS, building needs full Xcode for the Metal shader compiler (`metal`); the Command Line
Tools alone aren't enough. Without Xcode, add `--features gpui/runtime_shaders` (shaders are then
compiled when the app starts). The prebuilt app doesn't need Xcode.

## Fonts

The UI bundles [Inter](https://rsms.me/inter/) (SIL Open Font License 1.1, `assets/fonts/Inter-LICENSE.txt`).

## License

MIT, see [LICENSE](LICENSE). `third_party/xattr` (MIT/Apache‑2.0) and `third_party/gpui` (Apache‑2.0, from Zed) are vendored copies with small, marked patches ("Keywhere patch").

# UI screenshots

`shoot.sh` takes screenshots of Keywhere on a hidden, headless KDE screen, to
check how a screen looks (layout, wrapping, translations) without a person at
the desktop. It needs KDE Plasma's `kwin_wayland` and `spectacle`, plus
`dbus-daemon`, `dbus-send`, `timeout` (coreutils) and ImageMagick.

```sh
tools/screenshots/shoot.sh /tmp/shots                 # every shot
tools/screenshots/shoot.sh /tmp/shots rekey settings  # some
UI_LANG=ro tools/screenshots/shoot.sh /tmp/shots      # in Romanian (→ *-ro.png)
UI_THEME=light SHOT_WAIT=12 tools/screenshots/shoot.sh /tmp/shots
```

| Shot | Screen |
|---|---|
| `unlock-create` | "Create new vault" form, both fields filled |
| `rekey` | Change master password dialog, strong new password |
| `rekey-short` | the same with a too-short new password (warning) |
| `settings` | Settings, with the *Vault* section |
| `browser` | vault browser, `GitHub` entry selected |
| `new-entry` | new entry form with the generator |

Images are 1234×777 PNGs. The script prints `ok <file>` per shot and exits
non-zero if any failed.

## How it works

1. Builds the devtools harness (`screenshot_demo`, feature `devtools`, never
   in releases) and a demo vault from sample data (`e2e_vault` + `populate`).
2. Starts a private `dbus-daemon` and `kwin_wayland --virtual
   --no-lockscreen` on it, with a throw-away `HOME` and XDG dirs, so real
   settings, vaults, clipboard and desktop are never touched.
3. For each shot, runs `screenshot_demo` as a Wayland client of that kwin, in
   the state the shot names, waits `SHOT_WAIT` seconds, and has `spectacle -b`
   capture the screen through kwin's screenshot D-Bus interface.

The app has no input automation here; the state comes from the harness:
`screenshot_demo <vault> <select>` (see `Shell::new` in
`crates/keywhere-app/src/lib.rs` for the selects, e.g. `settings`, `new`,
`edit:<title>`, `rekey`) and `screenshot_demo --create-form <path>`. Add a
select there to screenshot a new screen.

## Safety checks, and why

- Before each capture, `org.kde.KWin` on the private bus must belong to the
  headless kwin's PID, and every image must be exactly 1234×777; anything
  else is deleted unseen.
- `--no-lockscreen` matters: without it the headless kwin locks together with
  the real login session (it follows logind), and every capture shows a lock
  screen.

## What didn't work (don't retry)

- X11 instead of Wayland: under kwin's rootless Xwayland `import -window root`
  hangs; under a rootful `Xwayland :N` the window captures solid black
  (GPUI's Vulkan frames don't reach `XGetImage`).
- `kwin_wayland --exit-with-session "<command with args>"`: the string is taken
  as one path; the session never starts.
- Matching processes with `pkill -f` / `pgrep -f` on a pattern that is also in
  the shell's own command line kills that shell. Stop processes by PID.

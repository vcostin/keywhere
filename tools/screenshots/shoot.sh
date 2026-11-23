#!/usr/bin/env bash
# UI screenshots of Keywhere on a hidden, headless KDE screen.
#
#   tools/screenshots/shoot.sh <out dir> [shot...]
#
# Shots (default: all): unlock-create, rekey, rekey-short, settings, browser,
# new-entry. UI_LANG=ro takes them in Romanian. SHOT_WAIT=<s> (default 8) is
# how long the app gets to draw before the capture.
#
# Needs KDE Plasma's kwin_wayland and spectacle, plus dbus-daemon and
# ImageMagick (identify). Builds the devtools screenshot harness and a
# throw-away demo vault itself. See tools/screenshots/README.md.
#
# Safety: everything runs with a private HOME, XDG dirs, D-Bus session and
# Wayland socket, so the real settings, vaults and desktop are never used.
# Before each capture the script checks that org.kde.KWin on the private bus
# is *our* headless kwin, and every image must have the headless screen's
# odd size (1234x777); anything else is deleted unseen. (An earlier ad-hoc
# setup once captured the real desktop when its private session had died.)
set -euo pipefail

OUT=${1:?usage: shoot.sh <out dir> [shot...]}
shift
SHOTS=("$@")
[ ${#SHOTS[@]} -gt 0 ] || SHOTS=(unlock-create rekey rekey-short settings browser new-entry)
W_PX=1234 H_PX=777
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

for tool in kwin_wayland spectacle dbus-daemon dbus-send identify timeout; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 2; }
done

echo "building the screenshot harness…"
(cd "$ROOT" && cargo build -q -p keywhere --features devtools --example screenshot_demo \
    && cargo build -q -p keywhere-core --example e2e_vault --example populate)
APP=$ROOT/target/debug/examples/screenshot_demo

W=$(mktemp -d)
H=$W/home
mkdir -p "$H/config/keywhere" "$H/data" "$H/cache" "$H/run"
chmod 700 "$H/run"
cat >"$H/config/keywhere/settings" <<EOF
check_updates = false
menu_entry = off
language = ${UI_LANG:-en}
theme = ${UI_THEME:-dark}
EOF

pids=()
cleanup() {
    for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
    sleep 0.5
    for p in "${pids[@]}"; do kill -9 "$p" 2>/dev/null || true; done
    rm -rf "$W"
}
trap cleanup EXIT
# Forget a PID once it has been reaped (exact match).
untrack() {
    local keep=() p
    for p in "${pids[@]}"; do [ "$p" = "$1" ] || keep+=("$p"); done
    pids=("${keep[@]}")
}

# Only the private session from here on.
unset DISPLAY WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS
export HOME=$H XDG_CONFIG_HOME=$H/config XDG_DATA_HOME=$H/data XDG_CACHE_HOME=$H/cache \
    XDG_RUNTIME_DIR=$H/run

# Demo vault (sample data only).
DEMO=$W/demo.psafe3
DEMO_PW=demo-password-123
KEYWHERE_PASSWORD=$DEMO_PW E2E_TITLE=GitHub E2E_ENTRY_PASSWORD=sample \
    "$ROOT/target/debug/examples/e2e_vault" create "$DEMO"
KEYWHERE_PASSWORD=$DEMO_PW "$ROOT/target/debug/examples/populate" "$DEMO" >/dev/null

# Private D-Bus session, then the headless compositor on it.
BUS=unix:path=$W/bus
dbus-daemon --session --address="$BUS" --nofork --nopidfile >"$W/dbus.log" 2>&1 &
pids+=($!)
for _ in $(seq 50); do [ -S "$W/bus" ] && break; sleep 0.1; done
[ -S "$W/bus" ] || { echo "private D-Bus did not start" >&2; exit 1; }
export DBUS_SESSION_BUS_ADDRESS=$BUS
# --no-lockscreen: otherwise the headless kwin locks along with the real
# login session (it follows logind), and every capture is its lock screen.
kwin_wayland --virtual --no-lockscreen --width $W_PX --height $H_PX --socket kw-shot \
    >"$W/kwin.log" 2>&1 &
KWIN=$!
pids+=("$KWIN")
for _ in $(seq 100); do [ -S "$H/run/kw-shot" ] && break; sleep 0.1; done
[ -S "$H/run/kw-shot" ] || { echo "headless kwin did not start" >&2; cat "$W/kwin.log" >&2; exit 1; }
export WAYLAND_DISPLAY=kw-shot

# Is org.kde.KWin on our bus served by our kwin process?
our_kwin() {
    local pid
    pid=$(dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus.GetConnectionUnixProcessID string:org.kde.KWin 2>/dev/null |
        awk '/uint32/ {print $2}')
    [ "$pid" = "$KWIN" ]
}
for _ in $(seq 50); do our_kwin && break; sleep 0.2; done
our_kwin || { echo "org.kde.KWin on the private bus is not the headless kwin" >&2; exit 1; }

shot_args() {
    case "$1" in
        unlock-create) echo "--create-form $H/Personal.psafe3" ;;
        rekey) echo "$DEMO rekey" ;;
        rekey-short) echo "$DEMO rekey-short" ;;
        settings) echo "$DEMO settings" ;;
        browser) echo "$DEMO GitHub" ;;
        new-entry) echo "$DEMO new" ;;
        *) return 1 ;;
    esac
}

failed=0
for s in "${SHOTS[@]}"; do
    args=$(shot_args "$s") || { echo "unknown shot: $s" >&2; failed=1; continue; }
    png=$OUT/$s${UI_LANG:+-$UI_LANG}.png
    rm -f "$png"
    # shellcheck disable=SC2086
    KEYWHERE_DEMO_PASSWORD=$DEMO_PW "$APP" $args >"$W/$s.log" 2>&1 &
    app=$!
    # Tracked only while it runs: a reaped PID can be reused by another
    # process, which the EXIT trap must never signal.
    pids+=("$app")
    sleep "${SHOT_WAIT:-8}"
    if ! kill -0 "$app" 2>/dev/null; then
        wait "$app" 2>/dev/null || true
        untrack "$app"
        echo "FAIL $s: the app exited; log:" >&2
        cat "$W/$s.log" >&2
        failed=1
        continue
    fi
    if our_kwin && kill -0 "$KWIN" 2>/dev/null; then
        timeout 20 spectacle -b -n -f -o "$png" >"$W/$s.spectacle.log" 2>&1 || true
    fi
    kill "$app" 2>/dev/null || true
    wait "$app" 2>/dev/null || true
    untrack "$app"
    size=$( [ -f "$png" ] && identify -format '%wx%h' "$png" 2>/dev/null || echo none)
    if [ "$size" = "${W_PX}x${H_PX}" ]; then
        echo "ok   $png"
    else
        rm -f "$png" # not our headless screen (or nothing): never keep it
        echo "FAIL $s: capture was $size, expected ${W_PX}x${H_PX}; deleted" >&2
        failed=1
    fi
done
exit $failed

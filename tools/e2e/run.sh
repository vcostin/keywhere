#!/usr/bin/env bash
# End-to-end test of a built Keywhere binary on a virtual X display.
#
#   tools/e2e/run.sh <keywhere binary> <e2e_vault helper>
#
# The helper is `cargo build --release -p keywhere-core --example e2e_vault`.
# Needs Xvfb, xdotool, xclip, dbus-run-session and a Vulkan driver (Mesa
# lavapipe works). Everything runs in a throw-away HOME, so it never touches
# the real settings, vaults or clipboard of the machine.
#
# Checks are black-box, through the keyboard and the clipboard:
#   1. the process survives startup;
#   2. a wrong master password unlocks nothing;
#   3. the right one does, and Ctrl+C copies the entry's password;
#   4. the clipboard is emptied after the auto-clear time;
#   5. after Ctrl+L, Ctrl+C copies nothing;
#   6. a second `keywhere other.psafe3` hands the file to the running app;
#   7. a new entry is saved: the core library reads it back, and the
#      `.ibak` backup exists.
set -euo pipefail

if [ -z "${KEYWHERE_E2E_INNER:-}" ]; then
    # Private session bus (tray, portals); the app must not reach the host's.
    exec dbus-run-session -- env KEYWHERE_E2E_INNER=1 "$0" "$@"
fi

APP=$(realpath "$1")
VAULT_TOOL=$(realpath "$2")

W=$(mktemp -d)
LOG=$W/keywhere.log
export HOME=$W/home XDG_CONFIG_HOME=$W/config XDG_DATA_HOME=$W/data \
    XDG_CACHE_HOME=$W/cache XDG_RUNTIME_DIR=$W/run
mkdir -p "$HOME" "$XDG_CONFIG_HOME/keywhere" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY
export LANG=C.UTF-8
cat > "$XDG_CONFIG_HOME/keywhere/settings" <<'EOF'
clipboard_clear_secs = 5
check_updates = false
menu_entry = off
language = en
EOF

pids=()
cleanup() {
    for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
    rm -rf "$W"
}
trap cleanup EXIT

failures=0
pass() { echo "ok   - $1"; }
fail() {
    echo "FAIL - $1"
    failures=$((failures + 1))
}
die() {
    echo "FAIL - $1"
    echo "--- keywhere log ---"
    cat "$LOG" || true
    exit 1
}

# ---- display ---------------------------------------------------------------
export DISPLAY=:${E2E_DISPLAY:-87}
Xvfb "$DISPLAY" -screen 0 1280x900x24 -nolisten tcp >"$W/xvfb.log" 2>&1 &
pids+=($!)
for _ in $(seq 50); do
    xdotool getdisplaygeometry >/dev/null 2>&1 && break
    sleep 0.1
done
xdotool getdisplaygeometry >/dev/null 2>&1 || die "Xvfb did not start"

# ---- vaults ----------------------------------------------------------------
MAIN=$W/vaults/main.psafe3
OTHER=$W/vaults/other.psafe3
mkdir -p "$W/vaults"
MAIN_PW=e2e-main-master
OTHER_PW=e2e-other-master
MAIN_SECRET=main-entry-S3cret
OTHER_SECRET=other-entry-S3cret
KEYWHERE_PASSWORD=$MAIN_PW E2E_TITLE=Main E2E_ENTRY_PASSWORD=$MAIN_SECRET \
    "$VAULT_TOOL" create "$MAIN"
KEYWHERE_PASSWORD=$OTHER_PW E2E_TITLE=Other E2E_ENTRY_PASSWORD=$OTHER_SECRET \
    "$VAULT_TOOL" create "$OTHER"

# ---- helpers ---------------------------------------------------------------
clip() { xclip -o -selection clipboard 2>/dev/null || true; }

# Wait up to $1 seconds for the clipboard to equal $2 (empty: no text).
# While waiting, run the rest of the arguments (if any) every half second.
wait_clip() {
    local secs=$1 want=$2
    shift 2
    local end=$((SECONDS + secs))
    while [ "$SECONDS" -lt "$end" ]; do
        [ "$#" -gt 0 ] && "$@"
        [ "$(clip)" = "$want" ] && return 0
        sleep 0.5
    done
    [ "$(clip)" = "$want" ]
}

alive() { kill -0 "$APP_PID" 2>/dev/null; }

focus_app() {
    local win
    win=$(xdotool search --sync --classname keywhere 2>/dev/null | tail -n1)
    [ -n "$win" ] || die "no Keywhere window"
    xdotool windowfocus --sync "$win" 2>/dev/null || xdotool windowfocus "$win"
}

key() { xdotool key --clearmodifiers "$@"; }
type_text() { xdotool type --delay 20 --clearmodifiers "$1"; }
copy_selected() { key Down ctrl+c; }

# Type a master password into the (focused) unlock field and press Enter.
unlock_with() {
    key ctrl+BackSpace
    type_text "$1"
    key Return
}

# ---- 1. startup ------------------------------------------------------------
"$APP" "$MAIN" >"$LOG" 2>&1 &
APP_PID=$!
pids+=("$APP_PID")
win=""
for _ in $(seq 300); do
    alive || die "keywhere exited during startup"
    win=$(xdotool search --classname keywhere 2>/dev/null | tail -n1 || true)
    [ -n "$win" ] && break
    sleep 0.1
done
[ -n "$win" ] || die "no window after 30 s"
sleep 2
alive || die "keywhere exited after showing its window"
pass "starts and shows its window"
focus_app

# ---- 2. wrong password -----------------------------------------------------
unlock_with "not-the-password"
# Give the key derivation time to fail, while poking at Ctrl+C.
if wait_clip 8 "$MAIN_SECRET" copy_selected; then
    fail "a wrong master password unlocked the vault"
else
    [ -z "$(clip)" ] && pass "wrong master password unlocks nothing" \
        || fail "clipboard is not empty after a wrong password: $(clip)"
fi

# ---- 3. unlock + copy ------------------------------------------------------
focus_app
unlock_with "$MAIN_PW"
if wait_clip 30 "$MAIN_SECRET" copy_selected; then
    pass "unlocks, and Ctrl+C copies the entry's password"
else
    die "Ctrl+C did not copy the password after unlocking (clipboard: '$(clip)')"
fi

# ---- 4. auto-clear ---------------------------------------------------------
# clipboard_clear_secs = 5; allow for a slow software renderer.
if wait_clip 15 ""; then
    pass "clipboard is cleared after the auto-clear time"
else
    fail "clipboard still holds '$(clip)' 15 s after copying"
fi

# ---- 5. lock ---------------------------------------------------------------
focus_app
key ctrl+l
sleep 1
copy_selected
sleep 1
copy_selected
if [ -z "$(clip)" ]; then
    pass "after Ctrl+L, Ctrl+C copies nothing"
else
    fail "Ctrl+C copied '$(clip)' while locked"
fi

# ---- 6. second launch opens another vault ----------------------------------
"$APP" "$OTHER" >"$W/second.log" 2>&1 &
second=$!
for _ in $(seq 100); do kill -0 "$second" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$second" 2>/dev/null; then
    kill "$second"
    fail "the second launch did not hand over and exit"
fi
alive || die "the first instance exited when a second one started"
focus_app
unlock_with "$OTHER_PW"
if wait_clip 30 "$OTHER_SECRET" copy_selected; then
    pass "a second launch opens the other vault in the running app"
else
    fail "the other vault did not open (clipboard: '$(clip)')"
fi

# ---- 7. add an entry and save ----------------------------------------------
focus_app
key ctrl+n
sleep 1
type_text "Added by e2e"
key Return
sleep 2
key ctrl+q
for _ in $(seq 100); do alive || break; sleep 0.1; done
if alive; then
    fail "Ctrl+Q did not quit"
fi
titles=$(KEYWHERE_PASSWORD=$OTHER_PW "$VAULT_TOOL" titles "$OTHER" || true)
if printf '%s\n' "$titles" | grep -qx "Added by e2e" && printf '%s\n' "$titles" | grep -qx "Other"; then
    pass "a new entry is saved, and the core library reads it back"
else
    fail "saved vault does not contain the new entry (titles: $(echo "$titles" | tr '\n' ','))"
fi
if [ -s "$W/vaults/other.ibak" ]; then
    pass "the save left an .ibak backup"
else
    fail "no .ibak backup next to the vault"
fi

if [ "$failures" -gt 0 ]; then
    echo "$failures check(s) failed"
    echo "--- keywhere log ---"
    cat "$LOG" || true
    exit 1
fi
echo "all end-to-end checks passed"

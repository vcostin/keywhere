#!/usr/bin/env bash
# End-to-end test of a built Keywhere binary on a virtual X display.
#
#   tools/e2e/run.sh <keywhere binary> <e2e_vault helper> <e2e_portal helper>
#
# The helpers are `cargo build --release -p keywhere-core --example e2e_vault`
# and `cargo build --release -p keywhere --example e2e_portal` (a stand-in for
# the desktop portal's file dialogs, answering with a path the test picks).
# Needs Xvfb, xdotool, xclip, dbus-run-session and a Vulkan driver (Mesa
# lavapipe works). Everything runs in a throw-away HOME, so it never touches
# the real settings, vaults or clipboard of the machine.
#
# Checks are black-box, through the keyboard and the clipboard:
#   1. the process survives startup;
#   2. a wrong master password unlocks nothing;
#   3. the right one does, and Ctrl+C copies the entry's password;
#   4. the clipboard is emptied after the auto-clear time;
#   5. Ctrl+L clears the copied password, and then Ctrl+C copies nothing;
#   6. a second `keywhere other.psafe3` hands the file to the running app,
#      both while locked and while unlocked (Enter on "Lock and open");
#   7. a new entry is saved: the core library reads it back, and the
#      `.ibak` backup exists;
#   8. Ctrl+N on the unlock screen creates a new vault: it never replaces an
#      existing file, it gets the `.psafe3` extension, it opens straight away,
#      locks and unlocks again with the new master password, and an entry
#      added to it is saved;
#   9. Ctrl+Shift+P changes that vault's master password: a wrong current
#      password changes nothing; afterwards only the new password opens the
#      file, the `.ibak` backup still opens with the old one, and the app
#      locks and unlocks with the new one.
set -euo pipefail

if [ -z "${KEYWHERE_E2E_INNER:-}" ]; then
    # Private session bus (tray, portals); the app must not reach the host's.
    exec dbus-run-session -- env KEYWHERE_E2E_INNER=1 "$0" "$@"
fi

APP=$(realpath "$1")
VAULT_TOOL=$(realpath "$2")
PORTAL=$(realpath "$3")

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
clip() { timeout 3 xclip -o -selection clipboard 2>/dev/null || true; }

# What the clipboard holds, without printing secrets into CI logs.
clip_desc() {
    local c
    c=$(clip)
    case "$c" in
        "") echo "empty" ;;
        "$MAIN_SECRET") echo "main.psafe3's entry password" ;;
        "$OTHER_SECRET") echo "other.psafe3's entry password" ;;
        *) echo "other text (${#c} characters)" ;;
    esac
}

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

# Start Keywhere (arguments passed on) and wait for its window.
start_app() {
    local win=""
    "$APP" "$@" >>"$LOG" 2>&1 &
    APP_PID=$!
    pids+=("$APP_PID")
    for _ in $(seq 300); do
        alive || die "keywhere exited during startup"
        win=$(xdotool search --classname keywhere 2>/dev/null | tail -n1 || true)
        [ -n "$win" ] && break
        sleep 0.1
    done
    [ -n "$win" ] || die "no window after 30 s"
    sleep 2
    alive || die "keywhere exited after showing its window"
}

# Wait up to $1 seconds for file $2 to exist (or, with $3 = gone, to vanish).
wait_file() {
    local end=$((SECONDS + $1))
    while [ "$SECONDS" -lt "$end" ]; do
        if [ "${3:-}" = gone ]; then [ ! -e "$2" ] && return 0; else [ -e "$2" ] && return 0; fi
        sleep 0.5
    done
    return 1
}

quit_app() {
    key ctrl+q
    for _ in $(seq 100); do alive || break; sleep 0.1; done
    if alive; then
        fail "Ctrl+Q did not quit"
    fi
}

# ---- 1. startup ------------------------------------------------------------
start_app "$MAIN"
pass "starts and shows its window"
focus_app

# ---- 2. wrong password -----------------------------------------------------
unlock_with "not-the-password"
# Give the key derivation time to fail, while poking at Ctrl+C. (Check 3
# then proves that the same keys do copy once the vault is unlocked.)
if wait_clip 8 "$MAIN_SECRET" copy_selected; then
    fail "a wrong master password unlocked the vault"
elif [ -n "$(clip)" ]; then
    fail "after a wrong password the clipboard holds $(clip_desc)"
else
    pass "wrong master password unlocks nothing"
fi

# ---- 3. unlock + copy ------------------------------------------------------
focus_app
unlock_with "$MAIN_PW"
if wait_clip 30 "$MAIN_SECRET" copy_selected; then
    pass "unlocks, and Ctrl+C copies the entry's password"
else
    die "Ctrl+C did not copy the password after unlocking (clipboard: $(clip_desc))"
fi

# ---- 4. auto-clear ---------------------------------------------------------
# clipboard_clear_secs = 5; allow for a slow software renderer.
if wait_clip 15 ""; then
    pass "clipboard is cleared after the auto-clear time"
else
    fail "15 s after copying the clipboard still holds $(clip_desc)"
fi

# ---- 5. lock ---------------------------------------------------------------
# Copy again first, so this check can't pass just because copying broke.
focus_app
wait_clip 10 "$MAIN_SECRET" copy_selected || die "Ctrl+C stopped copying before the lock check"
key ctrl+l
# 2 s: well inside the 5 s auto-clear, so only the lock can have cleared it.
if wait_clip 2 ""; then
    pass "Ctrl+L locks and clears the password it copied"
else
    fail "after Ctrl+L the clipboard still holds $(clip_desc)"
fi
copy_selected
sleep 1
copy_selected
sleep 1
if [ -z "$(clip)" ]; then
    pass "after Ctrl+L, Ctrl+C copies nothing"
else
    fail "Ctrl+C copied while locked: $(clip_desc)"
fi

# ---- 6. a second launch opens another vault --------------------------------
# Start `keywhere <file>` again; it must hand the file over and exit.
second_launch() {
    local p
    "$APP" "$1" >>"$W/second.log" 2>&1 &
    p=$!
    for _ in $(seq 100); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
    if kill -0 "$p" 2>/dev/null; then
        kill "$p"
        fail "the second launch did not hand $1 over and exit"
    fi
    alive || die "the first instance exited when a second one started"
}

# 6a. While locked: the unlock screen switches to the other file.
second_launch "$OTHER"
focus_app
unlock_with "$OTHER_PW"
if wait_clip 30 "$OTHER_SECRET" copy_selected; then
    pass "while locked, a second launch switches to the other vault"
else
    fail "the other vault did not open (clipboard: $(clip_desc))"
fi

# 6b. While unlocked: the app asks first; Enter answers "Lock and open".
second_launch "$MAIN"
focus_app
sleep 1
key Return
sleep 1
unlock_with "$MAIN_PW"
if wait_clip 30 "$MAIN_SECRET" copy_selected; then
    pass "while unlocked, a second launch opens the other vault after asking"
else
    fail "the vault from the second launch did not open (clipboard: $(clip_desc))"
fi

# ---- 7. add an entry and save ----------------------------------------------
focus_app
key ctrl+n
sleep 1
type_text "Added by e2e"
key Return
sleep 2
quit_app
titles=$(KEYWHERE_PASSWORD=$MAIN_PW "$VAULT_TOOL" titles "$MAIN" || true)
if printf '%s\n' "$titles" | grep -qx "Added by e2e" && printf '%s\n' "$titles" | grep -qx "Main"; then
    pass "a new entry is saved, and the core library reads it back"
else
    fail "saved vault does not contain the new entry (titles: $(echo "$titles" | tr '\n' ','))"
fi
if [ -s "$W/vaults/main.ibak" ]; then
    pass "the save left an .ibak backup"
else
    fail "no .ibak backup next to the vault"
fi

# ---- 8. create a new vault -------------------------------------------------
# The fake portal answers each file dialog with the path in $ANSWER.
ANSWER=$W/portal-answer
: >"$ANSWER"
"$PORTAL" "$ANSWER" >>"$W/portal.log" 2>&1 &
pids+=($!)
sleep 1
NEW_PW=e2e-new-master-pw
NEW=$W/vaults/created.psafe3

# Starts on the unlock screen of the last vault (main.psafe3).
start_app
focus_app

# Wait up to $1 seconds for the fake portal to log a SaveFile answer of $2.
wait_dialog() {
    local end=$((SECONDS + $1))
    while [ "$SECONDS" -lt "$end" ]; do
        grep -qxF "e2e_portal: SaveFile -> $2" "$W/portal.log" 2>/dev/null && return 0
        sleep 0.5
    done
    return 1
}

# 8a. An existing file is never replaced, even if the dialog said "replace".
# The app must stay on the unlock screen (the passwords typed below then
# only make a failed unlock attempt); a create form would overwrite main.
main_sum=$(sha256sum "$MAIN" | cut -d' ' -f1)
echo "$MAIN" >"$ANSWER"
key ctrl+n
wait_dialog 10 "$MAIN" || die "Ctrl+N on the unlock screen did not open the save dialog"
sleep 1
type_text "$NEW_PW"
key Return
type_text "$NEW_PW"
key Return
sleep 4
if [ "$(sha256sum "$MAIN" | cut -d' ' -f1)" = "$main_sum" ] &&
    KEYWHERE_PASSWORD=$MAIN_PW "$VAULT_TOOL" titles "$MAIN" >/dev/null; then
    pass "creating a vault over an existing file leaves the file alone"
else
    fail "creating a vault over main.psafe3 changed it"
fi

# 8b. A new name without extension: created as created.psafe3 and opened.
focus_app
echo "$W/vaults/created" >"$ANSWER"
key ctrl+n
wait_dialog 10 "$W/vaults/created" || die "the second Ctrl+N did not open the save dialog"
sleep 1
: >"$ANSWER" # any further dialog is cancelled
type_text "$NEW_PW"
key Return
type_text "$NEW_PW"
key Return
if wait_file 30 "$NEW" && wait_file 10 "${NEW%.psafe3}.plk" &&
    KEYWHERE_PASSWORD=$NEW_PW "$VAULT_TOOL" titles "$NEW" >/dev/null; then
    pass "Ctrl+N creates created.psafe3, and it opens straight away"
else
    die "the new vault was not created and opened ($(ls "$W/vaults" | tr '\n' ' '))"
fi
if [ -e "$W/vaults/created" ]; then
    fail "a file without the .psafe3 extension was created too"
fi
mode=$(stat -c %a "$NEW")
if [ "$mode" = 600 ]; then
    pass "the new vault file is owner-only"
else
    fail "the new vault file has mode $mode"
fi

# 8c. Lock, unlock with the new master password, add an entry, quit.
focus_app
key ctrl+l
if wait_file 10 "${NEW%.psafe3}.plk" gone; then
    pass "the new vault locks"
else
    fail "Ctrl+L did not lock the new vault"
fi
focus_app
unlock_with "$NEW_PW"
if wait_file 30 "${NEW%.psafe3}.plk"; then
    pass "the new vault unlocks again with its master password"
else
    die "the new vault did not unlock again"
fi
key ctrl+n
sleep 1
type_text "In the new vault"
key Return
sleep 2
quit_app
titles=$(KEYWHERE_PASSWORD=$NEW_PW "$VAULT_TOOL" titles "$NEW" || true)
if [ "$titles" = "In the new vault" ]; then
    pass "an entry added to the new vault is saved"
else
    fail "the new vault holds: $(echo "$titles" | tr '\n' ',')"
fi
if [ "$(cat "$XDG_CONFIG_HOME/keywhere/last-vault" 2>/dev/null)" = "$NEW" ]; then
    pass "the new vault is remembered as the current one"
else
    fail "last-vault is not the new vault"
fi

# ---- 9. change the master password -----------------------------------------
CHANGED_PW=e2e-changed-master-pw
opens_with() { KEYWHERE_PASSWORD=$2 "$VAULT_TOOL" titles "$1" >/dev/null 2>&1; }
# The three fields: current, new, repeat (Enter moves to the next one).
change_pw() {
    type_text "$1"
    key Return
    type_text "$2"
    key Return
    type_text "$2"
    key Return
}

start_app "$NEW"
focus_app
unlock_with "$NEW_PW"
wait_file 30 "${NEW%.psafe3}.plk" || die "the new vault did not unlock for check 9"
sleep 1
before=$(sha256sum "$NEW" | cut -d' ' -f1)

# 9a. Wrong current password: refused, the file is untouched.
key ctrl+shift+p
sleep 1
change_pw "not-the-current-pw" "$CHANGED_PW"
sleep 6
if [ "$(sha256sum "$NEW" | cut -d' ' -f1)" = "$before" ] && opens_with "$NEW" "$NEW_PW"; then
    pass "a wrong current password changes nothing"
else
    fail "a wrong current password changed the vault file"
fi

# 9b. The dialog is still open (fields cleared, current focused): retry.
change_pw "$NEW_PW" "$CHANGED_PW"
changed=""
for _ in $(seq 60); do
    opens_with "$NEW" "$CHANGED_PW" && changed=1 && break
    sleep 0.5
done
if [ -n "$changed" ]; then
    pass "Ctrl+Shift+P changes the master password, and the file opens with the new one"
else
    die "the master password was not changed"
fi
if opens_with "$NEW" "$NEW_PW"; then
    fail "the old master password still opens the vault"
else
    pass "the old master password no longer opens the vault"
fi
if opens_with "${NEW%.psafe3}.ibak" "$NEW_PW"; then
    pass "the .ibak backup still opens with the old master password"
else
    fail "the .ibak backup does not open with the old master password"
fi
titles=$(KEYWHERE_PASSWORD=$CHANGED_PW "$VAULT_TOOL" titles "$NEW" || true)
if [ "$titles" = "In the new vault" ]; then
    pass "the entries are kept"
else
    fail "after the change the vault holds: $(echo "$titles" | tr '\n' ',')"
fi

# 9c. Lock, and unlock with the new password.
focus_app
key ctrl+l
wait_file 10 "${NEW%.psafe3}.plk" gone || fail "Ctrl+L did not lock after the change"
focus_app
unlock_with "$CHANGED_PW"
if wait_file 30 "${NEW%.psafe3}.plk"; then
    pass "the app unlocks with the new master password"
else
    fail "the app did not unlock with the new master password"
fi
quit_app

if [ "$failures" -gt 0 ]; then
    echo "$failures check(s) failed"
    echo "--- keywhere log ---"
    cat "$LOG" || true
    echo "--- portal log ---"
    cat "$W/portal.log" 2>/dev/null || true
    exit 1
fi
echo "all end-to-end checks passed"

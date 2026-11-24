//! Clipboard for secrets, with history/cloud exclusion hints:
//! * KDE Klipper & friends: `x-kde-passwordManagerHint: secret`
//! * Windows: `ExcludeClipboardContentFromMonitorProcessing`,
//!   `CanIncludeInClipboardHistory=0`, `CanUploadToCloudClipboard=0`
//! * macOS: `org.nspasteboard.ConcealedType`
//!
//! (all via `arboard`'s `exclude_from_history`). On Wayland, arboard serves
//! the selection from a thread in this process (no fork).
//!
//! One long-lived `arboard::Clipboard` is kept: on X11 the owner must stay
//! alive to keep serving what we copied.

use std::sync::Mutex;

use zeroize::Zeroizing;

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<R>) -> Option<R> {
    let mut g = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = arboard::Clipboard::new().ok();
    }
    f(g.as_mut()?)
}

/// Put a secret on the clipboard, excluded from clipboard history where the
/// OS supports it. Returns false if no OS clipboard is reachable.
pub fn set_secret(text: &str) -> bool {
    set_secret_epoch(text).is_some()
}

/// [`set_secret`], returning the clipboard epoch of this copy. Only a
/// successful copy voids earlier field timers: after a failed one the old
/// secret may still be there and its timer must still run.
fn set_secret_epoch(text: &str) -> Option<u64> {
    with(|cb| {
        let set = cb.set();
        #[cfg(target_os = "linux")]
        let set = {
            use arboard::SetExtLinux;
            set.exclude_from_history()
        };
        #[cfg(target_os = "windows")]
        let set = {
            use arboard::SetExtWindows;
            set.exclude_from_monitoring()
                .exclude_from_history()
                .exclude_from_cloud()
        };
        #[cfg(target_os = "macos")]
        let set = {
            use arboard::SetExtApple;
            set.exclude_from_history()
        };
        set.text(text.to_string()).ok()
    })
    .map(|()| bump())
}

/// Current clipboard text (for "is it still ours?" checks).
pub fn get_text() -> Option<Zeroizing<String>> {
    with(|cb| cb.get_text().ok().map(Zeroizing::new))
}

/// Bumped by every field copy and every clear: a field copy's own timer
/// only acts while nothing has happened to the clipboard through us since.
static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn bump() -> u64 {
    EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

/// Empty the clipboard.
pub fn clear() {
    bump();
    with(|cb| cb.clear().ok());
}

/// SHA-256 of vault text a text field just copied (a revealed password, a
/// note…), waiting for the unlocked view to adopt it into its clipboard
/// auto-clear timer. Only the digest is kept, never the text.
static FIELD_COPY: Mutex<Option<[u8; 32]>> = Mutex::new(None);

fn digest(text: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(text).into()
}

/// Copy vault text out of a text field through the secure path (history
/// exclusion). Returns a ticket for [`clear_if`] (the field's own auto-clear
/// timer), or `None` if no OS clipboard is reachable. The unlocked view also
/// adopts the digest, for its countdown and to clear on lock.
pub fn copy_from_field(text: &str) -> Option<([u8; 32], u64)> {
    let epoch = set_secret_epoch(text)?;
    let h = digest(text);
    *FIELD_COPY.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
    Some((h, epoch))
}

/// The timer of the field copy with this ticket ran out: empty the
/// clipboard if it still holds that text (or can't be read, so it might).
/// A no-op once we cleared or copied again since: then that copy is gone
/// or has its own timer, and a stale timer must not wipe what the user
/// copied later.
pub fn clear_if((h, epoch): ([u8; 32], u64)) {
    if EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch {
        return;
    }
    match get_text() {
        Some(t) if digest(&t) != h => {}
        _ => clear(),
    }
}

/// The digest of the last field copy not yet put on an auto-clear timer.
pub fn take_field_copy() -> Option<[u8; 32]> {
    FIELD_COPY.lock().unwrap_or_else(|e| e.into_inner()).take()
}

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
    .is_some()
}

/// Current clipboard text (for "is it still ours?" checks).
pub fn get_text() -> Option<Zeroizing<String>> {
    with(|cb| cb.get_text().ok().map(Zeroizing::new))
}

/// Empty the clipboard.
pub fn clear() {
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
/// exclusion). Returns its digest for [`clear_if`] (the field's own
/// auto-clear timer), or `None` if no OS clipboard is reachable. The
/// unlocked view also adopts it, for its countdown and to clear on lock.
pub fn copy_from_field(text: &str) -> Option<[u8; 32]> {
    if !set_secret(text) {
        return None;
    }
    let h = digest(text);
    *FIELD_COPY.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
    Some(h)
}

/// Empty the clipboard if it still holds the text with digest `h` (or
/// can't be read, so it might).
pub fn clear_if(h: [u8; 32]) {
    match get_text() {
        Some(t) if digest(&t) != h => {}
        _ => clear(),
    }
}

/// The digest of the last field copy not yet put on an auto-clear timer.
pub fn take_field_copy() -> Option<[u8; 32]> {
    FIELD_COPY.lock().unwrap_or_else(|e| e.into_inner()).take()
}

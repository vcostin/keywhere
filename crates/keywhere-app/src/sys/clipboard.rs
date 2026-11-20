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

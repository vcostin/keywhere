//! OS integration that GPUI doesn't provide: secure clipboard, tray,
//! lock-on-sleep/screen-lock, single instance (`keywhere --show`).
//!
//! Everything here reports into the app through one channel of [`SysEvent`]s.

pub mod capture;
pub mod clipboard;
#[cfg(unix)]
pub mod instance;
#[cfg(target_os = "linux")]
pub mod integrate;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "linux")]
pub mod picker;
#[cfg(any(target_os = "macos", windows))]
pub mod tray;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(windows)]
pub mod windows;

use futures::channel::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SysEvent {
    /// Bring the window to the front (tray click, `keywhere --show`), with
    /// the Wayland activation token from that user action if there is one.
    Show(Option<String>),
    /// Open this vault file (a second start with a file argument), plus the
    /// activation token as for `Show`.
    Open(std::path::PathBuf, Option<String>),
    /// Lock now (tray menu, screen locked).
    Lock,
    /// System is about to sleep: lock, then confirm via [`ack_lock`] so the
    /// sleep inhibitor can be released.
    LockForSleep,
    /// Same as the window's close button: lock and hide to the tray.
    Hide,
    /// Quit the app (tray menu).
    Quit,
}

static LOCK_ACK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The app has finished locking (sleep handshake).
pub fn ack_lock() {
    LOCK_ACK.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

pub fn lock_ack() -> u64 {
    LOCK_ACK.load(std::sync::atomic::Ordering::SeqCst)
}

pub type EventTx = mpsc::UnboundedSender<SysEvent>;
pub type EventRx = mpsc::UnboundedReceiver<SysEvent>;

pub fn channel() -> (EventTx, EventRx) {
    mpsc::unbounded()
}

/// Lock/unlock state shown by the tray (never any vault content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Locked,
    Unlocked,
}

/// Handle to this platform's tray: StatusNotifierItem on Linux, the menu
/// bar on macOS, the notification area on Windows.
pub struct Tray {
    #[cfg(target_os = "linux")]
    inner: Option<linux::TrayHandle>,
    #[cfg(any(target_os = "macos", windows))]
    inner: Option<tray::TrayHandle>,
}

impl Tray {
    pub fn start(tx: EventTx) -> Self {
        #[cfg(target_os = "linux")]
        {
            Tray {
                inner: linux::start_tray(tx),
            }
        }
        #[cfg(any(target_os = "macos", windows))]
        {
            Tray {
                inner: tray::start_tray(tx),
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = tx;
            Tray {}
        }
    }

    /// Is a tray icon actually showing? (Decides close-to-tray vs quit.)
    pub fn available(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            self.inner.is_some()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            false
        }
    }

    pub fn set_state(&self, state: TrayState) {
        #[cfg(target_os = "linux")]
        if let Some(h) = &self.inner {
            linux::set_tray_state(h, state);
        }
        #[cfg(any(target_os = "macos", windows))]
        if let Some(h) = &self.inner {
            tray::set_tray_state(h, state);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        let _ = state;
    }
}

/// Start listeners that request a lock when the session locks or sleeps.
pub fn watch_session(tx: EventTx) {
    #[cfg(target_os = "linux")]
    linux::watch_session(tx);
    #[cfg(target_os = "macos")]
    macos::watch_session(tx);
    #[cfg(windows)]
    windows::watch_session(tx);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let _ = tx;
}

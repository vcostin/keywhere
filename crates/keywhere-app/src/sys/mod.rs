//! OS integration that GPUI doesn't provide: secure clipboard, tray,
//! lock-on-sleep/screen-lock, single instance (`keywhere --show`).
//!
//! Everything here reports into the app through one channel of [`SysEvent`]s.

pub mod clipboard;
#[cfg(unix)]
pub mod instance;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod wayland;

use futures::channel::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SysEvent {
    /// Bring the window to the front (tray click, `keywhere --show`), with
    /// the Wayland activation token from that user action if there is one.
    Show(Option<String>),
    /// Lock now (tray menu, screen locked, system going to sleep).
    Lock,
    /// Same as the window's close button: lock and hide to the tray.
    Hide,
    /// Quit the app (tray menu).
    Quit,
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

/// Handle to whatever tray this platform has (none on macOS/Windows yet).
pub struct Tray {
    #[cfg(target_os = "linux")]
    inner: Option<linux::TrayHandle>,
}

impl Tray {
    pub fn start(tx: EventTx) -> Self {
        #[cfg(target_os = "linux")]
        {
            Tray {
                inner: linux::start_tray(tx),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = tx;
            Tray {}
        }
    }

    /// Is a tray icon actually showing? (Decides close-to-tray vs quit.)
    pub fn available(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.inner.is_some()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub fn set_state(&self, state: TrayState) {
        #[cfg(target_os = "linux")]
        if let Some(h) = &self.inner {
            linux::set_tray_state(h, state);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = state;
    }
}

/// Start listeners that request a lock when the session locks or sleeps.
pub fn watch_session(tx: EventTx) {
    #[cfg(target_os = "linux")]
    linux::watch_session(tx);
    #[cfg(not(target_os = "linux"))]
    let _ = tx; // TODO: macOS NSWorkspace / Windows WTS notifications.
}

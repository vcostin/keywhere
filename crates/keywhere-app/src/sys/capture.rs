//! Keep the vault window out of screenshots, screen recordings and screen
//! sharing where the OS lets an app ask for that:
//! * Windows 10 2004+: `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`
//!   (the window shows as empty in captures); older versions fall back to
//!   `WDA_MONITOR` (black).
//! * macOS: `NSWindow.sharingType = .none`. Recent macOS versions don't
//!   apply it to every capture API (ScreenCaptureKit), so this is a hint,
//!   not a guarantee.
//!
//! Linux has no equivalent: on Wayland the compositor asks the user before
//! anything can capture the screen.

use gpui::Window;

pub fn exclude(window: &Window) {
    #[cfg(any(windows, target_os = "macos"))]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        match handle.as_raw() {
            #[cfg(windows)]
            RawWindowHandle::Win32(h) => {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_MONITOR,
                };
                let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
                // SAFETY: GPUI's live top-level window, owned by this thread.
                unsafe {
                    if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) == 0 {
                        SetWindowDisplayAffinity(hwnd, WDA_MONITOR);
                    }
                }
            }
            #[cfg(target_os = "macos")]
            RawWindowHandle::AppKit(h) => {
                use objc2_app_kit::{NSView, NSWindowSharingType};
                // SAFETY: GPUI's live content view; we are on the main thread.
                let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
                if let Some(w) = view.window() {
                    w.setSharingType(NSWindowSharingType::None);
                }
            }
            _ => {}
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = window;
}

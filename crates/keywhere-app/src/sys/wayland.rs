//! Raise our window on Wayland with an XDG activation token.
//!
//! Wayland compositors (KWin, Mutter) only let an app focus itself with a
//! token tied to a real user action. GPUI 0.2.2 asks for its own token using
//! its last input serial, which the compositor rejects after a click on the
//! tray, so the window only "demands attention". The tray host (KDE: via
//! StatusNotifierItem `ProvideXdgActivationToken`) and launchers
//! (`XDG_ACTIVATION_TOKEN` for `keywhere --show`) give us a valid token; we
//! hand it to the compositor ourselves, for GPUI's own surface.
//!
//! GPUI uses the system libwayland backend, so its raw display/surface
//! pointers are real libwayland objects we can address from a private queue.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{self, XdgActivationV1};

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XdgActivationV1,
        _: xdg_activation_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Plausible token: printable ASCII, bounded length. (Tokens are opaque
/// strings chosen by the compositor; this only guards our socket input.)
pub fn valid_token(t: &str) -> bool {
    (1..=256).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_graphic())
}

/// Activate `window` with `token`. Returns false when not on Wayland or when
/// the compositor has no xdg-activation support (caller falls back).
pub fn activate_with_token(window: &gpui::Window, token: &str) -> bool {
    if !valid_token(token) {
        return false;
    }
    let (Ok(wh), Ok(dh)) = (
        HasWindowHandle::window_handle(window),
        HasDisplayHandle::display_handle(window),
    ) else {
        return false;
    };
    let (RawWindowHandle::Wayland(w), RawDisplayHandle::Wayland(d)) = (wh.as_raw(), dh.as_raw())
    else {
        return false;
    };
    // SAFETY: GPUI's wl_display stays alive for the life of the app, and a
    // foreign Backend never closes it.
    let backend = unsafe { Backend::from_foreign_display(d.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let Ok((globals, mut queue)) = registry_queue_init::<State>(&conn) else {
        return false;
    };
    let qh = queue.handle();
    let Ok(activation) = globals.bind::<XdgActivationV1, _, _>(&qh, 1..=1, ()) else {
        return false;
    };
    // SAFETY: the pointer is GPUI's live wl_surface proxy for this window.
    let Ok(id) = (unsafe { ObjectId::from_ptr(WlSurface::interface(), w.surface.as_ptr().cast()) })
    else {
        return false;
    };
    let Ok(surface) = WlSurface::from_id(&conn, id) else {
        return false;
    };
    activation.activate(token.to_string(), &surface);
    activation.destroy();
    queue.roundtrip(&mut State).is_ok()
}

#[cfg(test)]
mod tests {
    use super::valid_token;

    #[test]
    fn token_validation() {
        assert!(valid_token("kwin-1234_abc"));
        assert!(!valid_token(""));
        assert!(!valid_token("has space"));
        assert!(!valid_token("new\nline"));
        assert!(!valid_token(&"x".repeat(300)));
    }
}

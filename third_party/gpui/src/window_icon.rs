//! Keywhere patch: an application-wide window icon, set once by the app
//! before windows open. Wayland compositors that support
//! `xdg-toplevel-icon-v1` and X11 window managers (`_NET_WM_ICON`) show it in
//! the task bar; elsewhere it is ignored.

use std::sync::OnceLock;

/// One square icon image: `size`×`size` pixels, straight (not premultiplied)
/// RGBA, row by row.
#[derive(Clone, Debug)]
pub struct WindowIconImage {
    /// Edge length in pixels.
    pub size: u32,
    /// `size * size * 4` bytes, RGBA.
    pub rgba: Vec<u8>,
}

static ICON: OnceLock<(Option<String>, Vec<WindowIconImage>)> = OnceLock::new();

/// Set the icon for all windows opened afterwards. `name` is an XDG icon
/// theme name compositors may prefer (e.g. "keywhere"); the images are the
/// fallback pixel data. Only the first call has an effect.
pub fn set_window_icon(name: Option<&str>, images: Vec<WindowIconImage>) {
    let images = images
        .into_iter()
        .filter(|i| i.size > 0 && i.rgba.len() == (i.size * i.size * 4) as usize)
        .collect();
    let _ = ICON.set((name.map(str::to_string), images));
}

#[allow(dead_code)]
pub(crate) fn window_icon() -> Option<&'static (Option<String>, Vec<WindowIconImage>)> {
    ICON.get()
}

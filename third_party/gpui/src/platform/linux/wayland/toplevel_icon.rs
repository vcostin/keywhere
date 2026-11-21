//! Keywhere patch: `xdg-toplevel-icon-v1` (staging), so the task bar shows the
//! app's own icon even without an installed `.desktop` file. The protocol is
//! generated from the vendored XML because the `wayland-protocols` version
//! gpui uses predates it.

#![allow(
    missing_docs,
    clippy::all,
    non_upper_case_globals,
    non_camel_case_types
)]

use std::os::fd::AsFd;

use wayland_client::QueueHandle;
use wayland_client::protocol::{wl_buffer, wl_shm, wl_shm_pool};
use wayland_protocols::xdg::shell::client::xdg_toplevel;

pub mod protocol {
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::xdg::shell::client::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::xdg::shell::client::__interfaces::*;
        wayland_scanner::generate_interfaces!(
            "src/platform/linux/wayland/xdg-toplevel-icon-v1.xml"
        );
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("src/platform/linux/wayland/xdg-toplevel-icon-v1.xml");
}

pub use protocol::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;

use super::client::WaylandClientStatePtr;

/// Give `toplevel` the app icon set with [`crate::set_window_icon`], if any.
pub(crate) fn apply(
    manager: &XdgToplevelIconManagerV1,
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<WaylandClientStatePtr>,
    toplevel: &xdg_toplevel::XdgToplevel,
) {
    let Some((name, images)) = crate::window_icon::window_icon() else {
        return;
    };
    if name.is_none() && images.is_empty() {
        return;
    }
    let icon = manager.create_icon(qh, ());
    if let Some(name) = name {
        icon.set_name(name.clone());
    }
    // Buffers must outlive the icon object; destroyed below, after it.
    let mut buffers: Vec<(wl_shm_pool::WlShmPool, wl_buffer::WlBuffer)> = Vec::new();
    for img in images {
        if let Some(b) = shm_buffer(shm, qh, img) {
            icon.add_buffer(&b.1, 1);
            buffers.push(b);
        }
    }
    manager.set_icon(toplevel, Some(&icon));
    // "The icon must still remain set on every toplevel it was assigned to."
    icon.destroy();
    for (pool, buffer) in buffers {
        buffer.destroy();
        pool.destroy();
    }
}

/// A wl_shm ARGB8888 buffer (little-endian BGRA bytes, premultiplied).
fn shm_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<WaylandClientStatePtr>,
    img: &crate::WindowIconImage,
) -> Option<(wl_shm_pool::WlShmPool, wl_buffer::WlBuffer)> {
    use std::io::Write;
    let size = img.size as i32;
    let len = (size * size * 4) as usize;
    let mut bytes = Vec::with_capacity(len);
    for px in img.rgba.chunks_exact(4) {
        let a = px[3] as u32;
        let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        bytes.extend_from_slice(&[pm(px[2]), pm(px[1]), pm(px[0]), px[3]]);
    }
    // SAFETY: valid NUL-terminated name, standard flags.
    let fd = unsafe { libc::memfd_create(c"gpui-window-icon".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` was just created and is owned here.
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    file.write_all(&bytes).ok()?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, size, size, size * 4, wl_shm::Format::Argb8888, qh, ());
    Some((pool, buffer))
}

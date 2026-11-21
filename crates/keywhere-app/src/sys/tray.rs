//! macOS menu-bar / Windows notification-area icon, with the same menu as
//! the Linux tray (Open Keywhere, Lock, Quit). Linux uses ksni
//! (StatusNotifierItem) instead; see `linux.rs`.
//!
//! Must be created on the main thread after the app has started: GPUI's
//! run closure. AppKit / the Win32 message loop of that thread deliver the
//! menu and click events, which are forwarded to the app as [`SysEvent`]s.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use super::{EventTx, SysEvent, TrayState};

const TRAY_SVG: &[u8] = include_bytes!("../../../../assets/tray/keywhere-tray.svg");

pub struct TrayHandle {
    icon: TrayIcon,
    lock: MenuItem,
}

const OPEN: &str = "open";
const LOCK: &str = "lock";
const QUIT: &str = "quit";

/// What a menu item id asks for.
fn menu_event(id: &str) -> Option<SysEvent> {
    match id {
        OPEN => Some(SysEvent::Show(None)),
        LOCK => Some(SysEvent::Lock),
        QUIT => Some(SysEvent::Quit),
        _ => None,
    }
}

fn tooltip(state: TrayState) -> &'static str {
    match state {
        TrayState::Locked => "Keywhere: vault locked",
        TrayState::Unlocked => "Keywhere: vault unlocked",
    }
}

pub fn start_tray(tx: EventTx) -> Option<TrayHandle> {
    let open = MenuItem::with_id(OPEN, "Open Keywhere", true, None);
    let lock = MenuItem::with_id(LOCK, "Lock", false, None);
    let quit = MenuItem::with_id(QUIT, "Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&open, &lock, &PredefinedMenuItem::separator(), &quit])
        .ok()?;

    let t = tx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Some(ev) = menu_event(&e.id.0) {
            let _ = t.unbounded_send(ev);
        }
    }));
    // Windows convention: left click opens the app, right click shows the
    // menu. macOS: the menu opens on click, like other menu-bar items.
    #[cfg(windows)]
    tray_icon::TrayIconEvent::set_event_handler(Some(move |e: tray_icon::TrayIconEvent| {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            let _ = tx.unbounded_send(SysEvent::Show(None));
        }
    }));
    #[cfg(not(windows))]
    drop(tx);

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(cfg!(target_os = "macos"))
        .with_tooltip(tooltip(TrayState::Locked))
        .with_icon(icon()?)
        .build();
    match icon {
        Ok(icon) => Some(TrayHandle { icon, lock }),
        Err(e) => {
            eprintln!("keywhere: no tray icon: {e}");
            None
        }
    }
}

pub fn set_tray_state(h: &TrayHandle, state: TrayState) {
    h.lock.set_enabled(state == TrayState::Unlocked);
    let _ = h.icon.set_tooltip(Some(tooltip(state)));
}

/// The tray key, rendered from its SVG (32 px: Windows' largest small-icon
/// size at 200 % scaling; macOS scales it to the menu-bar height).
fn icon() -> Option<Icon> {
    use resvg::{tiny_skia, usvg};
    const SIZE: u32 = 32;
    let tree = usvg::Tree::from_data(TRAY_SVG, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let view = tree.size();
    let scale = SIZE as f32 / view.width().max(view.height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Icon::from_rgba(rgba, SIZE, SIZE).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_map_to_events() {
        assert_eq!(menu_event(OPEN), Some(SysEvent::Show(None)));
        assert_eq!(menu_event(LOCK), Some(SysEvent::Lock));
        assert_eq!(menu_event(QUIT), Some(SysEvent::Quit));
        assert_eq!(menu_event("other"), None);
    }

    #[test]
    fn tray_icon_renders() {
        assert!(icon().is_some());
    }
}

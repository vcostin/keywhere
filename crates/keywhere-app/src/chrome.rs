//! Client-side window chrome: our own title bar, window buttons, resize edges.
//!
//! * Linux / Windows: no system title bar; we draw min/max/close and handle
//!   drag, double-click-to-maximise and edge resizing ourselves. On Windows the
//!   buttons are also tagged as native hit-test areas (snap layouts etc.).
//! * macOS: the native traffic lights stay, embedded in our bar
//!   (`appears_transparent`), so we only reserve space for them.

use gpui::{
    canvas, div, point, prelude::*, px, svg, Bounds, CursorStyle, Decorations, Div, HitboxBehavior,
    MouseButton, Pixels, Point, ResizeEdge, Size, Stateful, Window, WindowControlArea,
};

use crate::theme::*;

gpui::actions!(keywhere_chrome, [CloseWindow]);

/// Invisible resize margin around a floating client-decorated window.
pub const RESIZE_INSET: Pixels = px(10.);
const RADIUS: Pixels = px(10.);

/// Corner radius content touching the window corners must use, so nothing
/// pokes out of the rounded frame (GPUI clips children to rectangles, not
/// rounded corners). 0 when maximised/tiled or server-decorated.
pub fn corner_radius(window: &Window) -> Pixels {
    match window.window_decorations() {
        Decorations::Client { tiling } if !tiling.is_tiled() && !window.is_maximized() => {
            RADIUS - px(1.)
        }
        _ => px(0.),
    }
}

/// Space the native traffic lights need on macOS.
pub fn leading_inset() -> Pixels {
    if cfg!(target_os = "macos") {
        px(76.)
    } else {
        px(0.)
    }
}

/// A region that moves the window when dragged and maximises on double-click.
pub fn drag_area(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |e, window, _| {
            if e.click_count >= 2 {
                if cfg!(target_os = "macos") {
                    window.titlebar_double_click();
                } else {
                    window.zoom_window();
                }
            } else if !cfg!(target_os = "windows") {
                // Windows drags through the native hit-test area instead.
                window.start_window_move();
            }
        })
        .on_mouse_down(MouseButton::Right, |e, window, _| {
            window.show_window_menu(e.position)
        })
}

fn control_button(
    id: &'static str,
    icon: &'static str,
    area: WindowControlArea,
    danger: bool,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(area)
        .w(px(40.))
        .h(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(c(MUTED))
        .hover(move |d| {
            if danger {
                d.bg(c(0xD9534F)).text_color(c(0xFFFFFF))
            } else {
                d.bg(c(RAISED)).text_color(c(TEXT))
            }
        })
        // Don't let a press on a button start a window drag.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(
            svg()
                .path(icon)
                .size(px(14.))
                .text_color(c(if danger { TEXT_SOFT } else { MUTED })),
        )
}

/// Minimise / maximise-restore / close. Empty on macOS (native lights).
pub fn window_controls(window: &Window) -> Div {
    let row = div().flex().flex_none().items_center().gap(px(2.));
    if cfg!(target_os = "macos") {
        return row;
    }
    let caps = window.window_controls();
    let max_icon = if window.is_maximized() {
        "icons/win-restore.svg"
    } else {
        "icons/win-maximize.svg"
    };
    row.when(caps.minimize, |r| {
        r.child(control_button(
            "win-min",
            "icons/win-minimize.svg",
            WindowControlArea::Min,
            false,
            |w, _| w.minimize_window(),
        ))
    })
    .when(caps.maximize, |r| {
        r.child(control_button(
            "win-max",
            max_icon,
            WindowControlArea::Max,
            false,
            |w, _| w.zoom_window(),
        ))
    })
    // Close = lock; the shell then hides to the tray (or quits without one).
    .child(control_button(
        "win-close",
        "icons/win-close.svg",
        WindowControlArea::Close,
        true,
        |w, cx| w.dispatch_action(Box::new(CloseWindow), cx),
    ))
}

fn resize_edge(pos: Point<Pixels>, inset: Pixels, size: Size<Pixels>) -> Option<ResizeEdge> {
    let corner = inset * 2.;
    let (l, r) = (pos.x < corner, pos.x > size.width - corner);
    let (t, b) = (pos.y < corner, pos.y > size.height - corner);
    let edge = match () {
        _ if pos.y < inset && l || pos.x < inset && t => ResizeEdge::TopLeft,
        _ if pos.y < inset && r || pos.x > size.width - inset && t => ResizeEdge::TopRight,
        _ if pos.y > size.height - inset && l || pos.x < inset && b => ResizeEdge::BottomLeft,
        _ if pos.y > size.height - inset && r || pos.x > size.width - inset && b => {
            ResizeEdge::BottomRight
        }
        _ if pos.y < inset => ResizeEdge::Top,
        _ if pos.y > size.height - inset => ResizeEdge::Bottom,
        _ if pos.x < inset => ResizeEdge::Left,
        _ if pos.x > size.width - inset => ResizeEdge::Right,
        _ => return None,
    };
    Some(edge)
}

/// Wrap the app content in a client-decorated frame: rounded corners, border,
/// shadow and resize edges while floating; edge-to-edge when maximised/tiled.
pub fn frame(window: &mut Window, content: impl IntoElement) -> Div {
    let decorations = window.window_decorations();
    let Decorations::Client { tiling } = decorations else {
        return div().size_full().child(content);
    };
    let floating = !tiling.is_tiled() && !window.is_maximized();
    let inset = if floating { RESIZE_INSET } else { px(0.) };
    window.set_client_inset(inset);

    div()
        .size_full()
        .bg(gpui::transparent_black())
        .when(floating, |d| {
            d.p(inset)
                .child(
                    canvas(
                        |_, window, _| {
                            window.insert_hitbox(
                                Bounds::new(
                                    point(px(0.), px(0.)),
                                    window.window_bounds().get_bounds().size,
                                ),
                                HitboxBehavior::Normal,
                            )
                        },
                        move |_, hitbox, window, _| {
                            let size = window.window_bounds().get_bounds().size;
                            if let Some(edge) = resize_edge(window.mouse_position(), inset, size) {
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => {
                                            CursorStyle::ResizeUpDown
                                        }
                                        ResizeEdge::Left | ResizeEdge::Right => {
                                            CursorStyle::ResizeLeftRight
                                        }
                                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                            CursorStyle::ResizeUpLeftDownRight
                                        }
                                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                            CursorStyle::ResizeUpRightDownLeft
                                        }
                                    },
                                    &hitbox,
                                );
                            }
                        },
                    )
                    .size_full()
                    .absolute(),
                )
                .on_mouse_move(|_, window, _| window.refresh())
                .on_mouse_down(MouseButton::Left, move |e, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(e.position, inset, size) {
                        window.start_window_resize(edge);
                    }
                })
        })
        .child(
            div()
                .size_full()
                .overflow_hidden()
                .cursor(CursorStyle::Arrow)
                .when(floating, |d| {
                    // Visible on any wallpaper: a soft dark shadow for light
                    // backgrounds plus a faint light rim for dark/black ones.
                    d.rounded(RADIUS)
                        .border_1()
                        .border_color(c(0x3D62AB))
                        .shadow(vec![
                            gpui::BoxShadow {
                                color: gpui::hsla(0., 0., 0., 0.55),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                                offset: point(px(0.), px(2.)),
                            },
                            gpui::BoxShadow {
                                color: gpui::hsla(0.61, 0.6, 0.85, 0.12),
                                blur_radius: px(0.),
                                spread_radius: px(1.),
                                offset: point(px(0.), px(0.)),
                            },
                        ])
                })
                // Keep resize-edge mouse handling out of the content.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
}

//! Dropdown select: a trigger showing the current value and a floating list
//! (painted above everything via `deferred` + `anchored`). Keyboard: Enter /
//! Space / ↓ open, ↑↓ move, Enter pick, Esc close, typing filters (handy for
//! long lists such as languages). Click outside closes.

use gpui::{
    anchored, deferred, div, prelude::*, px, svg, App, Context, EventEmitter, FocusHandle,
    Focusable, KeyDownEvent, MouseButton, SharedString, Window,
};

use crate::theme::*;

pub struct SelectChanged(pub usize);

pub struct Select {
    focus: FocusHandle,
    options: Vec<SharedString>,
    selected: usize,
    open: bool,
    highlighted: usize,
    filter: String,
    width: f32,
}

impl EventEmitter<SelectChanged> for Select {}

impl Focusable for Select {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Visible list height before it scrolls.
const MAX_LIST: f32 = 260.;
const FILTER_FROM: usize = 8;

impl Select {
    pub fn new(cx: &mut Context<Self>, options: Vec<SharedString>, selected: usize) -> Self {
        Select {
            focus: cx.focus_handle(),
            selected: selected.min(options.len().saturating_sub(1)),
            options,
            open: false,
            highlighted: 0,
            filter: String::new(),
            width: 220.,
        }
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    /// Replace options/selection (e.g. after a language switch).
    pub fn set_options(
        &mut self,
        options: Vec<SharedString>,
        selected: usize,
        cx: &mut Context<Self>,
    ) {
        self.options = options;
        self.selected = selected.min(self.options.len().saturating_sub(1));
        cx.notify();
    }

    #[cfg(feature = "devtools")]
    pub fn dev_open(&mut self, cx: &mut Context<Self>) {
        self.set_open(true, cx);
    }

    fn visible(&self) -> Vec<usize> {
        let f = self.filter.to_lowercase();
        (0..self.options.len())
            .filter(|&i| f.is_empty() || self.options[i].to_lowercase().contains(&f))
            .collect()
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        self.filter.clear();
        self.highlighted = self
            .visible()
            .iter()
            .position(|&i| i == self.selected)
            .unwrap_or(0);
        cx.notify();
    }

    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.set_open(false, cx);
        cx.emit(SelectChanged(index));
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        if !self.open {
            if matches!(key, "enter" | "space" | "down") {
                self.set_open(true, cx);
                cx.stop_propagation();
            }
            return;
        }
        let vis = self.visible();
        match key {
            "escape" => self.set_open(false, cx),
            "down" => self.highlighted = (self.highlighted + 1).min(vis.len().saturating_sub(1)),
            "up" => self.highlighted = self.highlighted.saturating_sub(1),
            "enter" => {
                if let Some(&i) = vis.get(self.highlighted) {
                    self.pick(i, cx);
                }
            }
            "backspace" => {
                self.filter.pop();
                self.highlighted = 0;
            }
            _ => match &ev.keystroke.key_char {
                Some(ch)
                    if self.options.len() >= FILTER_FROM && !ch.chars().any(char::is_control) =>
                {
                    self.filter.push_str(ch);
                    self.highlighted = 0;
                }
                _ => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Select {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let label = self.options.get(self.selected).cloned().unwrap_or_default();
        let vis = self.visible();

        let trigger = div()
            .id("select-trigger")
            .w(px(self.width))
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(8.))
            .bg(c(BG))
            .border_1()
            .border_color(c(if focused || self.open { FOCUS } else { BORDER }))
            .cursor_pointer()
            .hover(|d| d.border_color(c(0x4E7BE0)))
            .on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.focus);
                let open = !this.open;
                this.set_open(open, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.5))
                    .text_color(c(TEXT))
                    .child(label),
            )
            .child(
                svg()
                    .path("icons/chevron-down.svg")
                    .size(px(14.))
                    .text_color(c(MUTED)),
            );

        let list = self.open.then(|| {
            deferred(
                anchored()
                    // Below the trigger (32px tall + 2px gap); snaps inside the window.
                    .offset(gpui::point(px(0.), px(34.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("select-list")
                            .occlude()
                            .w(px(self.width))
                            .max_h(px(MAX_LIST))
                            .overflow_y_scroll()
                            .p_1()
                            .rounded(px(9.))
                            .bg(c(PANEL))
                            .border_1()
                            .border_color(c(0x2A4A85))
                            .shadow_lg()
                            .on_mouse_down_out(
                                cx.listener(|this, _, _, cx| this.set_open(false, cx)),
                            )
                            .when(!self.filter.is_empty(), |d| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_size(px(11.))
                                        .text_color(c(DIM))
                                        .child(SharedString::from(format!("⌕ {}", self.filter))),
                                )
                            })
                            .children(vis.iter().enumerate().map(|(pos, &i)| {
                                let on = i == self.selected;
                                let hl = pos == self.highlighted;
                                div()
                                    .id(("opt", i))
                                    .h(px(30.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .text_size(px(12.5))
                                    .text_color(c(if on { TEXT } else { TEXT_SOFT }))
                                    .when(hl, |d| d.bg(c(SELECTION)))
                                    .when(!hl, |d| d.hover(|d| d.bg(c(HOVER))))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| this.pick(i, cx)))
                                    .child(div().w(px(14.)).flex_none().when(on, |d| {
                                        d.child(
                                            svg()
                                                .path("icons/check.svg")
                                                .size(px(13.))
                                                .text_color(c(FOCUS)),
                                        )
                                    }))
                                    .child(self.options[i].clone())
                            })),
                    ),
            )
            .with_priority(1)
        });

        div()
            .key_context("Select")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .child(trigger)
            .children(list)
    }
}

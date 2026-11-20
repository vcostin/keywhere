//! Minimal single-line text field.
//!
//! Deliberately simple: the buffer is a `Zeroizing<String>`, there is no undo
//! stack, no selection and no copy-out. In `secret` mode the content is drawn
//! as bullets and only turned into a displayable string while revealed.
//! (No IME composition yet; dead keys/compose are an M2 item.)

use gpui::{
    div, prelude::*, px, svg, App, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    SharedString, Window,
};
use zeroize::Zeroizing;

use crate::theme::*;

pub enum FieldEvent {
    Changed,
    Submit,
    Escape,
}

pub struct TextField {
    focus: FocusHandle,
    buf: Zeroizing<String>,
    placeholder: SharedString,
    secret: bool,
    revealed: bool,
    icon: Option<&'static str>,
    large: bool,
}

impl EventEmitter<FieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    pub fn new(cx: &mut Context<Self>, placeholder: &str, secret: bool) -> Self {
        Self {
            focus: cx.focus_handle(),
            buf: Zeroizing::new(String::new()),
            placeholder: SharedString::from(placeholder.to_string()),
            secret,
            revealed: false,
            icon: None,
            large: false,
        }
    }

    pub fn with_icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// Borrow the content. For secrets, callers copy into their own
    /// `Zeroizing` buffer and then call [`clear`].
    pub fn text(&self) -> &str {
        &self.buf
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        // Zeroizing<String> zeroes the old allocation on drop.
        self.buf = Zeroizing::new(String::new());
        self.revealed = false;
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }

    fn insert(&mut self, s: &str) {
        // Grow via a fresh zeroizing buffer so no stale copy is left behind
        // when String reallocates.
        let mut next = Zeroizing::new(String::with_capacity(self.buf.len() + s.len() + 16));
        next.push_str(&self.buf);
        next.push_str(s);
        self.buf = next;
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        match ks.key.as_str() {
            "enter" => {
                cx.emit(FieldEvent::Submit);
                return;
            }
            "escape" => {
                cx.emit(FieldEvent::Escape);
                return;
            }
            "backspace" => {
                if m.secondary() || m.alt {
                    self.buf = Zeroizing::new(String::new());
                } else {
                    self.buf.pop();
                }
            }
            "v" if m.secondary() && !m.alt => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let t = Zeroizing::new(t);
                    let line = t.lines().next().unwrap_or("");
                    self.insert(line);
                }
            }
            _ => {
                if m.control || m.platform || m.function {
                    return; // let shortcuts bubble
                }
                match &ks.key_char {
                    Some(ch) if !ch.chars().any(char::is_control) => self.insert(ch),
                    _ => return,
                }
            }
        }
        cx.stop_propagation();
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let empty = self.buf.is_empty();
        let shown: SharedString = if empty {
            self.placeholder.clone()
        } else if self.secret && !self.revealed {
            "•".repeat(self.buf.chars().count()).into()
        } else {
            SharedString::from(self.buf.as_str().to_string())
        };
        let h = if self.large { px(40.) } else { px(28.) };

        div()
            .id("field")
            .key_context("TextField")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_click(cx.listener(|this, _, window, _| window.focus(&this.focus)))
            .h(h)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .rounded(px(if self.large { 9. } else { 7. }))
            .bg(c(if self.large { SURFACE } else { PANEL }))
            .border_1()
            .border_color(c(if focused { FOCUS } else { BORDER }))
            .when(focused && self.large, |d| d.shadow_sm())
            .cursor_text()
            .when_some(self.icon, |d, icon| {
                d.child(svg().path(icon).size(px(14.)).text_color(c(MUTED)))
            })
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(if self.large { 14. } else { 12.5 }))
                    .text_color(c(if empty { DIM } else { TEXT }))
                    .when(self.secret && !self.revealed && !empty, |d| d.text_size(px(13.)))
                    .when(focused && empty, |d| d.child(div().mr(px(2.)).w(px(1.5)).h(px(15.)).bg(c(FOCUS))))
                    .child(shown)
                    .when(focused && !empty, |d| {
                        d.child(div().ml(px(1.)).w(px(1.5)).h(px(15.)).bg(c(FOCUS)))
                    })
                    .flex()
                    .items_center(),
            )
            .when(self.secret, |d| {
                d.child(
                    div()
                        .id("reveal")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.revealed = !this.revealed;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .path(if self.revealed { "icons/eye-off.svg" } else { "icons/eye.svg" })
                                .size(px(15.))
                                .text_color(c(MUTED)),
                        ),
                )
            })
    }
}

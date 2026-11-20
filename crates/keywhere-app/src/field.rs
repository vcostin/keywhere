//! Minimal text field (single- or multi-line).
//!
//! Deliberately simple: the buffer is a `Zeroizing<String>` that is rebuilt
//! (never grown in place) on every edit, there is no undo stack, no selection
//! and no copy-out. In `secret` mode the content is drawn as bullets and only
//! turned into displayable text while revealed (through `secret_text`).
//! Caret moves with arrows / Home / End; clicking focuses. No IME composition
//! yet (dead keys / compose are a later item).

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
    /// Byte offset of the caret; always on a char boundary.
    cursor: usize,
    /// English text; translated at render so a language switch applies live.
    placeholder: &'static str,
    secret: bool,
    revealed: bool,
    multiline: bool,
    icon: Option<&'static str>,
    large: bool,
    _hide_task: Option<gpui::Task<()>>,
}

/// Plaintext reveal in a secret field auto-hides after this long.
const REVEAL_FOR: std::time::Duration = std::time::Duration::from_secs(15);

impl EventEmitter<FieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    pub fn new(cx: &mut Context<Self>, placeholder: &'static str, secret: bool) -> Self {
        Self {
            focus: cx.focus_handle(),
            buf: Zeroizing::new(String::new()),
            cursor: 0,
            placeholder,
            secret,
            revealed: false,
            multiline: false,
            icon: None,
            large: false,
            _hide_task: None,
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

    /// Enter inserts a newline; Ctrl/⌘+Enter submits.
    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    /// Borrow the content. For secrets, callers copy into their own
    /// `Zeroizing` buffer.
    pub fn text(&self) -> &str {
        &self.buf
    }

    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.buf = Zeroizing::new(s.to_string());
        self.cursor = self.buf.len();
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        // Zeroizing<String> zeroes the old allocation on drop.
        self.buf = Zeroizing::new(String::new());
        self.cursor = 0;
        self.revealed = false;
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }

    /// Replace `range` of the buffer with `s`, via a fresh zeroizing buffer
    /// so no stale copy is left in spare capacity.
    fn splice(&mut self, range: std::ops::Range<usize>, s: &str) {
        debug_assert!(
            self.buf.is_char_boundary(range.start) && self.buf.is_char_boundary(range.end)
        );
        let mut next = Zeroizing::new(String::with_capacity(self.buf.len() + s.len() + 16));
        next.push_str(&self.buf[..range.start]);
        next.push_str(s);
        next.push_str(&self.buf[range.end..]);
        self.cursor = range.start + s.len();
        self.buf = next;
    }

    fn prev_boundary(&self, i: usize) -> usize {
        self.buf[..i]
            .char_indices()
            .next_back()
            .map_or(0, |(j, _)| j)
    }

    fn next_boundary(&self, i: usize) -> usize {
        self.buf[i..].chars().next().map_or(i, |c| i + c.len_utf8())
    }

    fn line_start(&self, i: usize) -> usize {
        self.buf[..i].rfind('\n').map_or(0, |j| j + 1)
    }

    fn line_end(&self, i: usize) -> usize {
        self.buf[i..].find('\n').map_or(self.buf.len(), |j| i + j)
    }

    /// Move to the previous/next line, keeping the column (in chars).
    fn vertical(&mut self, down: bool) {
        let start = self.line_start(self.cursor);
        let col = self.buf[start..self.cursor].chars().count();
        let target_start = if down {
            let end = self.line_end(self.cursor);
            if end >= self.buf.len() {
                return;
            }
            end + 1
        } else {
            if start == 0 {
                return;
            }
            self.line_start(start - 1)
        };
        let target_end = self.line_end(target_start);
        let line = &self.buf[target_start..target_end];
        self.cursor = target_start + line.char_indices().nth(col).map_or(line.len(), |(j, _)| j);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        let mut changed = true;
        match ks.key.as_str() {
            "enter" if self.multiline && !m.secondary() => {
                self.splice(self.cursor..self.cursor, "\n")
            }
            "enter" => {
                cx.emit(FieldEvent::Submit);
                cx.stop_propagation();
                return;
            }
            "escape" => {
                cx.emit(FieldEvent::Escape);
                return;
            }
            "backspace" if m.secondary() || m.alt => {
                let start = if self.multiline {
                    self.line_start(self.cursor)
                } else {
                    0
                };
                self.splice(start..self.cursor, "");
            }
            "backspace" => {
                if self.cursor > 0 {
                    let p = self.prev_boundary(self.cursor);
                    self.splice(p..self.cursor, "");
                }
            }
            "delete" => {
                let n = self.next_boundary(self.cursor);
                self.splice(self.cursor..n, "");
            }
            "left" => {
                self.cursor = self.prev_boundary(self.cursor);
                changed = false;
            }
            "right" => {
                self.cursor = self.next_boundary(self.cursor);
                changed = false;
            }
            "home" => {
                self.cursor = if self.multiline && !m.secondary() {
                    self.line_start(self.cursor)
                } else {
                    0
                };
                changed = false;
            }
            "end" => {
                self.cursor = if self.multiline && !m.secondary() {
                    self.line_end(self.cursor)
                } else {
                    self.buf.len()
                };
                changed = false;
            }
            "up" | "down" if self.multiline => {
                self.vertical(ks.key == "down");
                changed = false;
            }
            "v" if m.secondary() && !m.alt => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let t = Zeroizing::new(t);
                    if self.multiline {
                        let norm = Zeroizing::new(t.replace("\r\n", "\n"));
                        self.splice(self.cursor..self.cursor, &norm);
                    } else {
                        self.splice(self.cursor..self.cursor, t.lines().next().unwrap_or(""));
                    }
                }
            }
            _ => {
                if m.control || m.platform || m.function {
                    return; // let shortcuts bubble
                }
                match &ks.key_char {
                    Some(ch) if !ch.chars().any(char::is_control) => {
                        let ch = ch.clone();
                        self.splice(self.cursor..self.cursor, &ch);
                    }
                    _ => return,
                }
            }
        }
        cx.stop_propagation();
        if changed {
            cx.emit(FieldEvent::Changed);
        }
        cx.notify();
    }

    /// Display text for `s` according to secret/reveal state.
    fn shown(&self, s: &str) -> SharedString {
        if self.secret && !self.revealed {
            "•".repeat(s.chars().count()).into()
        } else if self.secret {
            crate::secret_text::display(s)
        } else {
            SharedString::from(s.to_string())
        }
    }

    fn caret() -> gpui::Div {
        div().flex_none().w(px(1.5)).h(px(15.)).bg(c(FOCUS))
    }

    /// One visual line; `caret_at` = byte offset inside `line` to draw the caret.
    fn line_row(&self, line: &str, caret_at: Option<usize>) -> gpui::Div {
        let row = div()
            .flex()
            .items_center()
            .min_h(px(17.))
            .whitespace_nowrap();
        match caret_at {
            Some(i) => row
                .when(i > 0, |r| r.child(self.shown(&line[..i])))
                .child(Self::caret())
                .when(i < line.len(), |r| r.child(self.shown(&line[i..]))),
            None => row.child(if line.is_empty() {
                SharedString::from(" ")
            } else {
                self.shown(line)
            }),
        }
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let empty = self.buf.is_empty();
        let font = px(if self.large { 14. } else { 12.5 });

        let body = if empty {
            // The caret is an overlay so the placeholder doesn't shift when
            // the field gains focus.
            div()
                .relative()
                .flex()
                .items_center()
                .min_h(px(17.))
                .text_color(c(DIM))
                .child(crate::i18n::t(self.placeholder))
                .when(focused, |d| {
                    d.child(Self::caret().absolute().left(px(-1.)).top(px(1.)))
                })
        } else {
            let mut col = div().flex().flex_col().text_color(c(TEXT));
            let mut offset = 0;
            for line in self.buf.split('\n') {
                let end = offset + line.len();
                let caret = (focused && self.cursor >= offset && self.cursor <= end)
                    .then(|| self.cursor - offset);
                col = col.child(self.line_row(line, caret));
                offset = end + 1;
            }
            col
        };

        div()
            .id("field")
            .key_context("TextField")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_click(cx.listener(|this, _, window, _| window.focus(&this.focus)))
            .w_full()
            .when(!self.multiline, |d| {
                d.h(px(if self.large { 40. } else { 30. })).items_center()
            })
            .when(self.multiline, |d| d.min_h(px(96.)).py_2().items_start())
            .flex()
            .gap_2()
            .px_3()
            .rounded(px(if self.large { 9. } else { 7. }))
            .bg(c(if self.large { SURFACE } else { BG }))
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
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(font)
                    .child(body),
            )
            .when(self.secret, |d| {
                d.child(
                    div()
                        .id("reveal")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.revealed = !this.revealed;
                            this._hide_task = this.revealed.then(|| {
                                cx.spawn(async move |this, cx| {
                                    cx.background_executor().timer(REVEAL_FOR).await;
                                    this.update(cx, |t, cx| {
                                        t.revealed = false;
                                        cx.notify();
                                    })
                                    .ok();
                                })
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .path(if self.revealed {
                                    "icons/eye-off.svg"
                                } else {
                                    "icons/eye.svg"
                                })
                                .size(px(15.))
                                .text_color(c(MUTED)),
                        ),
                )
            })
    }
}

//! Text field (single- or multi-line) with selection, clipboard and undo.
//!
//! The buffer is a `Zeroizing<String>` that is rebuilt (never grown in place)
//! on every edit; undo snapshots are `Zeroizing` too, so no copy outlives the
//! field. In `secret` mode the content is drawn as bullets and only turned
//! into displayable text while revealed (through `secret_text`); a masked
//! secret can be selected and edited but never copied or cut, and word-wise
//! moves treat it as one word (they would reveal its structure). Copies out
//! of `secret`/`sensitive` fields go through the secure clipboard path with
//! auto-clear. No IME composition yet (dead keys / compose are a later item).
//!
//! Keys (⌘ instead of Ctrl, ⌥ for word moves on macOS): arrows, Home/End,
//! Shift to extend, Ctrl+arrows by word, Ctrl+A, Ctrl+C/X/V, Ctrl+Insert,
//! Shift+Insert, Shift+Delete, Ctrl+Backspace/Delete (word), Ctrl+Z,
//! Ctrl+Shift+Z / Ctrl+Y. Mouse: click, drag, Shift+click, double-click
//! (word), triple-click (line).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    canvas, div, prelude::*, px, svg, App, Bounds, ClipboardItem, Context, EventEmitter,
    FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ShapedLine, SharedString, TextRun, Window,
};
use zeroize::Zeroizing;

use crate::theme::*;

pub enum FieldEvent {
    Changed,
    Submit,
    Escape,
}

/// One drawn line, recorded at prepaint for mouse hit-testing.
struct Row {
    bounds: Bounds<Pixels>,
    /// Byte range of the line in the buffer.
    start: usize,
    end: usize,
    /// What is drawn (bullets for a masked secret) and its shaped layout.
    shown: SharedString,
    shaped: ShapedLine,
}

struct Snapshot {
    buf: Zeroizing<String>,
    cursor: usize,
    anchor: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    /// Consecutive typed characters undo as one step.
    Typing,
    Other,
}

const UNDO_LIMIT: usize = 100;

pub struct TextField {
    focus: FocusHandle,
    buf: Zeroizing<String>,
    /// Byte offset of the caret; always on a char boundary.
    cursor: usize,
    /// Other end of the selection (the caret is one end); `None` or equal
    /// to `cursor` = no selection.
    anchor: Option<usize>,
    /// Mouse button held after pressing in the field: moves extend from here.
    drag_from: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<EditKind>,
    rows: Rc<RefCell<Vec<Row>>>,
    /// English text; translated at render so a language switch applies live.
    placeholder: &'static str,
    secret: bool,
    /// Not masked, but drawn through `secret_text` so the display copies are
    /// wiped (notes, usernames… in the editor).
    sensitive: bool,
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

/// Character class for word moves: space, word characters, punctuation.
fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

impl TextField {
    pub fn new(cx: &mut Context<Self>, placeholder: &'static str, secret: bool) -> Self {
        Self {
            focus: cx.focus_handle(),
            buf: Zeroizing::new(String::new()),
            cursor: 0,
            anchor: None,
            drag_from: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            rows: Default::default(),
            placeholder,
            secret,
            sensitive: false,
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

    pub fn sensitive(mut self) -> Self {
        self.sensitive = true;
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
        self.checkpoint(EditKind::Other);
        self.buf = Zeroizing::new(s.to_string());
        self.cursor = self.buf.len();
        self.anchor = None;
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        // Zeroizing<String> zeroes the old allocations on drop.
        self.buf = Zeroizing::new(String::new());
        self.cursor = 0;
        self.anchor = None;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
        self.revealed = false;
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }

    /// Bullets on screen: no copying out, no word structure.
    fn masked(&self) -> bool {
        self.secret && !self.revealed
    }

    fn selection(&self) -> Option<Range<usize>> {
        let a = self.anchor?;
        (a != self.cursor).then(|| a.min(self.cursor)..a.max(self.cursor))
    }

    /// Remember the state before an edit (consecutive typing is one step).
    fn checkpoint(&mut self, kind: EditKind) {
        if !(kind == EditKind::Typing && self.last_edit == Some(EditKind::Typing)) {
            if self.undo.len() >= UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.undo.push(self.snapshot());
        }
        self.redo.clear();
        self.last_edit = Some(kind);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            buf: Zeroizing::new(self.buf.to_string()),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.buf = s.buf;
        self.cursor = s.cursor.min(self.buf.len());
        self.anchor = s.anchor.filter(|a| *a <= self.buf.len());
    }

    /// Undo (or redo); false if there is nothing to go back to.
    fn step_history(&mut self, back: bool) -> bool {
        let next = if back {
            self.undo.pop()
        } else {
            self.redo.pop()
        };
        let Some(s) = next else { return false };
        let current = self.snapshot();
        if back {
            self.redo.push(current);
        } else {
            self.undo.push(current);
        }
        self.restore(s);
        self.last_edit = None;
        true
    }

    /// Replace `range` of the buffer with `s`, via a fresh zeroizing buffer
    /// so no stale copy is left in spare capacity.
    fn splice(&mut self, range: Range<usize>, s: &str) {
        debug_assert!(
            self.buf.is_char_boundary(range.start) && self.buf.is_char_boundary(range.end)
        );
        let mut next = Zeroizing::new(String::with_capacity(self.buf.len() + s.len() + 16));
        next.push_str(&self.buf[..range.start]);
        next.push_str(s);
        next.push_str(&self.buf[range.end..]);
        self.cursor = range.start + s.len();
        self.anchor = None;
        self.buf = next;
    }

    /// Type or paste `s` over the selection (or at the caret).
    fn insert(&mut self, s: &str, kind: EditKind) {
        let kind = if self.selection().is_some() {
            EditKind::Other
        } else {
            kind
        };
        self.checkpoint(kind);
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        self.splice(range, s);
    }

    /// Delete the selection, or else `range` (if not empty).
    fn delete(&mut self, range: Range<usize>) -> bool {
        let range = self.selection().unwrap_or(range);
        if range.is_empty() {
            return false;
        }
        self.checkpoint(EditKind::Other);
        self.splice(range, "");
        true
    }

    /// Move the caret, extending the selection or dropping it.
    fn move_to(&mut self, i: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = i;
        self.last_edit = None;
    }

    fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.buf.len();
        self.last_edit = None;
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

    /// Start of the word before `i` (skipping spaces first).
    fn word_left(&self, i: usize) -> usize {
        if self.masked() {
            return self.line_start(i);
        }
        let mut it = self.buf[..i].char_indices().rev().peekable();
        let mut j = i;
        while let Some(&(k, ch)) = it.peek() {
            if class(ch) != 0 {
                break;
            }
            j = k;
            it.next();
        }
        if let Some(&(_, ch)) = it.peek() {
            let cl = class(ch);
            while let Some(&(k, ch)) = it.peek() {
                if class(ch) != cl {
                    break;
                }
                j = k;
                it.next();
            }
        }
        j
    }

    /// End of the word after `i` (skipping spaces first).
    fn word_right(&self, i: usize) -> usize {
        if self.masked() {
            return self.line_end(i);
        }
        let mut it = self.buf[i..].char_indices().peekable();
        let mut j = i;
        while let Some(&(k, ch)) = it.peek() {
            if class(ch) != 0 {
                break;
            }
            j = i + k + ch.len_utf8();
            it.next();
        }
        if let Some(&(_, ch)) = it.peek() {
            let cl = class(ch);
            while let Some(&(k, ch)) = it.peek() {
                if class(ch) != cl {
                    break;
                }
                j = i + k + ch.len_utf8();
                it.next();
            }
        }
        j
    }

    /// The word (or run of spaces / punctuation) around `i`.
    fn word_at(&self, i: usize) -> Range<usize> {
        if self.masked() {
            return self.line_start(i)..self.line_end(i);
        }
        let ch = self.buf[i..]
            .chars()
            .next()
            .filter(|c| *c != '\n')
            .or_else(|| self.buf[..i].chars().next_back());
        let Some(ch) = ch.filter(|c| *c != '\n') else {
            return i..i;
        };
        let cl = class(ch);
        let start = self.buf[..i]
            .char_indices()
            .rev()
            .take_while(|(_, c)| *c != '\n' && class(*c) == cl)
            .last()
            .map_or(i, |(k, _)| k);
        let end = self.buf[i..]
            .char_indices()
            .take_while(|(_, c)| *c != '\n' && class(*c) == cl)
            .last()
            .map_or(i, |(k, c)| i + k + c.len_utf8());
        start..end
    }

    /// The caret position on the previous/next line, keeping the column.
    fn vertical(&self, down: bool) -> usize {
        let start = self.line_start(self.cursor);
        let col = self.buf[start..self.cursor].chars().count();
        let target_start = if down {
            let end = self.line_end(self.cursor);
            if end >= self.buf.len() {
                return self.buf.len();
            }
            end + 1
        } else {
            if start == 0 {
                return 0;
            }
            self.line_start(start - 1)
        };
        let target_end = self.line_end(target_start);
        let line = &self.buf[target_start..target_end];
        target_start + line.char_indices().nth(col).map_or(line.len(), |(j, _)| j)
    }

    /// Buffer offset under a window position (clamped to the text).
    fn index_at(&self, p: Point<Pixels>) -> usize {
        let rows = self.rows.borrow();
        let Some(row) = rows
            .iter()
            .find(|r| p.y < r.bounds.bottom())
            .or(rows.last())
        else {
            return self.buf.len();
        };
        let di = row.shaped.closest_index_for_x(p.x - row.bounds.left());
        let chars = row.shown[..di.min(row.shown.len())].chars().count();
        let line = &self.buf[row.start..row.end.min(self.buf.len())];
        row.start
            + line
                .char_indices()
                .nth(chars)
                .map_or(line.len(), |(j, _)| j)
    }

    /// Put the selection on the clipboard. Masked secrets never leave;
    /// vault text uses the secure path with auto-clear.
    fn copy(&self, cx: &mut Context<Self>) {
        let Some(r) = self.selection() else { return };
        if self.masked() {
            return;
        }
        let text = Zeroizing::new(self.buf[r].to_string());
        if self.secret || self.sensitive {
            crate::sys::clipboard::copy_from_field(&text);
        } else {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
            let t = Zeroizing::new(t);
            if self.multiline {
                let norm = Zeroizing::new(t.replace("\r\n", "\n"));
                self.insert(&norm, EditKind::Other);
            } else {
                self.insert(t.lines().next().unwrap_or(""), EditKind::Other);
            }
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        let mac = cfg!(target_os = "macos");
        // Word-wise: ⌥ on macOS, Ctrl elsewhere. To the line's end: ⌘ on macOS.
        let word = if mac { m.alt } else { m.control };
        let whole_line = mac && m.platform;
        let shortcut = m.secondary() && !m.alt;
        let shift = m.shift;
        let before = self.buf.len();
        let mut changed = false;
        match ks.key.as_str() {
            "a" if shortcut && !shift => self.select_all(),
            "c" if shortcut && !shift => {
                if self.selection().is_none() {
                    return; // nothing selected: let app shortcuts have it
                }
                self.copy(cx);
            }
            "insert" if m.control && !shift => self.copy(cx),
            "x" if shortcut && !shift => {
                if self.selection().is_some() && !self.masked() {
                    self.copy(cx);
                    changed = self.delete(self.cursor..self.cursor);
                }
            }
            "delete" if shift && !mac && self.selection().is_some() => {
                if !self.masked() {
                    self.copy(cx);
                    changed = self.delete(self.cursor..self.cursor);
                }
            }
            "v" if shortcut && !shift => {
                self.paste(cx);
                changed = true;
            }
            "insert" if shift => {
                self.paste(cx);
                changed = true;
            }
            "z" if shortcut => changed = self.step_history(!shift),
            "y" if m.control && !mac => changed = self.step_history(false),
            "enter" if self.multiline && !m.secondary() => {
                self.insert("\n", EditKind::Other);
                changed = true;
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
            "backspace" => {
                let start = if whole_line {
                    self.line_start(self.cursor)
                } else if word {
                    self.word_left(self.cursor)
                } else {
                    self.prev_boundary(self.cursor)
                };
                changed = self.delete(start..self.cursor);
            }
            "delete" => {
                let end = if whole_line {
                    self.line_end(self.cursor)
                } else if word {
                    self.word_right(self.cursor)
                } else {
                    self.next_boundary(self.cursor)
                };
                changed = self.delete(self.cursor..end);
            }
            "left" => {
                let i = match self.selection() {
                    Some(r) if !shift => r.start,
                    _ if whole_line => self.line_start(self.cursor),
                    _ if word => self.word_left(self.cursor),
                    _ => self.prev_boundary(self.cursor),
                };
                self.move_to(i, shift);
            }
            "right" => {
                let i = match self.selection() {
                    Some(r) if !shift => r.end,
                    _ if whole_line => self.line_end(self.cursor),
                    _ if word => self.word_right(self.cursor),
                    _ => self.next_boundary(self.cursor),
                };
                self.move_to(i, shift);
            }
            "home" => {
                let i = if self.multiline && !m.secondary() {
                    self.line_start(self.cursor)
                } else {
                    0
                };
                self.move_to(i, shift);
            }
            "end" => {
                let i = if self.multiline && !m.secondary() {
                    self.line_end(self.cursor)
                } else {
                    self.buf.len()
                };
                self.move_to(i, shift);
            }
            "up" | "down" if self.multiline => {
                let down = ks.key == "down";
                let i = if whole_line {
                    if down {
                        self.buf.len()
                    } else {
                        0
                    }
                } else {
                    self.vertical(down)
                };
                self.move_to(i, shift);
            }
            _ => {
                if m.control || m.platform || m.function {
                    return; // let shortcuts bubble
                }
                match &ks.key_char {
                    Some(ch) if !ch.chars().any(char::is_control) => {
                        let ch = ch.clone();
                        self.insert(&ch, EditKind::Typing);
                        changed = true;
                    }
                    _ => return,
                }
            }
        }
        cx.stop_propagation();
        if changed || self.buf.len() != before {
            cx.emit(FieldEvent::Changed);
        }
        cx.notify();
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let i = self.index_at(ev.position);
        match ev.click_count {
            2 => {
                let r = self.word_at(i);
                self.anchor = Some(r.start);
                self.cursor = r.end;
                self.drag_from = None;
            }
            n if n >= 3 => {
                let (s, e) = if self.multiline {
                    (self.line_start(i), self.line_end(i))
                } else {
                    (0, self.buf.len())
                };
                self.anchor = Some(s);
                self.cursor = e;
                self.drag_from = None;
            }
            _ => {
                let from = if ev.modifiers.shift {
                    *self.anchor.get_or_insert(self.cursor)
                } else {
                    i
                };
                self.cursor = i;
                self.anchor = (from != i).then_some(from);
                self.drag_from = Some(from);
            }
        }
        self.last_edit = None;
        cx.notify();
    }

    fn on_drag_select(&mut self, p: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(from) = self.drag_from else { return };
        let i = self.index_at(p);
        if i != self.cursor || self.anchor != Some(from) {
            self.cursor = i;
            self.anchor = (from != i).then_some(from);
            cx.notify();
        }
    }

    /// Display text for `s` according to secret/reveal state.
    fn shown(&self, s: &str) -> SharedString {
        if self.masked() {
            "•".repeat(s.chars().count()).into()
        } else if self.secret || self.sensitive {
            crate::secret_text::display(s)
        } else {
            SharedString::from(s.to_string())
        }
    }

    fn caret() -> gpui::Div {
        div()
            .flex_none()
            .w(px(2.))
            .h(px(16.))
            .rounded(px(1.))
            .bg(c(FOCUS))
    }

    /// One visual line starting at buffer offset `start`: text pieces split
    /// at the selection edges, the selected ones highlighted, and the caret.
    fn line_row(
        &self,
        start: usize,
        line: &str,
        caret_at: Option<usize>,
        sel: Option<Range<usize>>,
    ) -> gpui::Div {
        // Selection clipped to this line, relative to it.
        let sel = sel.map(|r| {
            r.start.saturating_sub(start).min(line.len())
                ..r.end.saturating_sub(start).min(line.len())
        });
        let mut cuts = vec![0, line.len()];
        cuts.extend(sel.iter().flat_map(|r| [r.start, r.end]));
        cuts.extend(caret_at);
        cuts.sort_unstable();
        cuts.dedup();

        let mut row = div()
            .relative()
            .flex()
            .items_center()
            .min_h(px(17.))
            .whitespace_nowrap();
        let selected = |a: usize| sel.as_ref().is_some_and(|r| r.start <= a && a < r.end);
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if caret_at == Some(a) {
                row = row.child(Self::caret());
            }
            row = row.child(
                div()
                    .when(selected(a), |d| d.bg(ca(FOCUS, 0x4d)))
                    .child(self.shown(&line[a..b])),
            );
        }
        if caret_at == Some(line.len()) {
            row = row.child(Self::caret());
        }
        row
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let empty = self.buf.is_empty();
        let font = px(if self.large { 14. } else { 12.5 });
        self.rows.borrow_mut().clear();

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
            let sel = self.selection().filter(|_| focused);
            let mut col = div().flex().flex_col().text_color(c(TEXT));
            let mut offset = 0;
            for line in self.buf.split('\n') {
                let end = offset + line.len();
                let caret =
                    (focused && sel.is_none() && self.cursor >= offset && self.cursor <= end)
                        .then(|| self.cursor - offset);
                let empty_selected = line.is_empty()
                    && sel
                        .as_ref()
                        .is_some_and(|r| r.start <= offset && offset < r.end);
                // Record where the line is drawn, for mouse hit-testing.
                let rows = self.rows.clone();
                let shown = self.shown(line);
                let (start, stop) = (offset, end);
                let probe = canvas(
                    move |bounds, window, _| {
                        let style = window.text_style();
                        let run = TextRun {
                            len: shown.len(),
                            font: style.font(),
                            color: style.color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let size = style.font_size.to_pixels(window.rem_size());
                        let shaped =
                            window
                                .text_system()
                                .shape_line(shown.clone(), size, &[run], None);
                        rows.borrow_mut().push(Row {
                            bounds,
                            start,
                            end: stop,
                            shown,
                            shaped,
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full();
                col = col.child(
                    self.line_row(offset, line, caret, sel.clone())
                        .when(empty_selected, |d| d.bg(ca(FOCUS, 0x4d)).w(px(6.)))
                        .child(probe),
                );
                offset = end + 1;
            }
            col
        };

        // While the button is held after pressing in the field, follow the
        // mouse anywhere in the window (dragging past the edge keeps
        // selecting); release ends it.
        let dragging = self.drag_from.is_some();
        let this = cx.entity().downgrade();
        let drag_tracker = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let on_move = this.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                    if phase.bubble() && ev.pressed_button == Some(MouseButton::Left) {
                        let _ = on_move.update(cx, |t, cx| t.on_drag_select(ev.position, cx));
                    }
                });
                let on_up = this.clone();
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                    if phase.bubble() && ev.button == MouseButton::Left {
                        let _ = on_up.update(cx, |t, cx| {
                            t.drag_from = None;
                            cx.notify();
                        });
                    }
                });
            },
        )
        .absolute()
        .size_0();

        div()
            .id("field")
            .key_context("TextField")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .w_full()
            .when(!self.multiline, |d| {
                d.h(px(if self.large { 40. } else { 30. })).items_center()
            })
            .when(self.multiline, |d| d.min_h(px(96.)).py_2().items_start())
            .flex()
            .gap_2()
            .px_3()
            .rounded(px(if self.large { 9. } else { 7. }))
            // Light mode: white like a sheet of paper, so a field never
            // reads as greyed-out (disabled) on a white dialog.
            .bg(if is_light() {
                c(PANEL)
            } else {
                c(if self.large { SURFACE } else { BG })
            })
            .border_1()
            .border_color(c(if focused { FOCUS } else { BORDER }))
            // Focus ring: the field taking typing has to be obvious at a glance.
            .when(focused, |d| {
                d.shadow(vec![gpui::BoxShadow {
                    color: ca(FOCUS, 0x55).into(),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(1.),
                    spread_radius: px(2.),
                }])
            })
            .cursor_text()
            .child(drag_tracker)
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
                        // Not a click in the text: keep caret and selection.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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

//! Unlocked vault: sidebar (smart views + groups) · entry list · detail card.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    actions, div, prelude::*, px, svg, uniform_list, App, ClipboardItem, Context, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, ScrollStrategy, SharedString, Task,
    UniformListScrollHandle, Window,
};
use keywhere_core::{now_secs, rec, Vault};
use sha2::{Digest, Sha256};

use crate::field::{FieldEvent, TextField};
use crate::model::{self, EntryVm, Filter, GroupNode};
use crate::{config, theme::*};

actions!(
    keywhere,
    [Lock, FocusSearch, SelectNext, SelectPrev, CopyPassword, CopyUsername, OpenUrl, ToggleReveal]
);

pub const CLIPBOARD_CLEAR: Duration = Duration::from_secs(30);
pub const REVEAL_FOR: Duration = Duration::from_secs(15);

pub enum BrowserEvent {
    LockRequested,
}

pub struct Browser {
    pub path: PathBuf,
    vault: Vault,
    entries: Vec<EntryVm>,
    groups: Vec<GroupNode>,
    filter: Filter,
    visible: Vec<usize>, // indices into `entries`
    selected: Option<usize>, // index into `entries`
    search: Entity<TextField>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    revealed: bool,
    reveal_until: Option<Instant>,
    /// Hash (not the secret) of what we last put on the clipboard.
    clip_hash: Option<[u8; 32]>,
    clip_until: Option<Instant>,
    toast: Option<(SharedString, Instant)>,
    _ticker: Task<()>,
    _on_quit: gpui::Subscription,
}

impl EventEmitter<BrowserEvent> for Browser {}

impl Focusable for Browser {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn hash(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

impl Browser {
    pub fn new(path: PathBuf, vault: Vault, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let entries = model::build(&vault);
        let groups = model::groups(&entries);
        let search = cx.new(|cx| TextField::new(cx, "Search", false).with_icon("icons/search.svg"));
        cx.subscribe_in(&search, window, |this, _, ev, window, cx| match ev {
            FieldEvent::Changed => this.refilter(cx),
            FieldEvent::Submit => {
                if this.selected.is_some() {
                    this.copy(rec::PASSWORD, "Password", cx);
                }
            }
            FieldEvent::Escape => {
                this.search.update(cx, |f, cx| f.clear(cx));
                window.focus(&this.focus);
            }
        })
        .detach();

        // 1 Hz housekeeping: clipboard clear, reveal timeout, toast expiry.
        let ticker = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(1000)).await;
            if this.update_in(cx, |this, _, cx| this.tick(cx)).is_err() {
                break;
            }
        });

        // Don't leave a copied secret behind when the app exits.
        let on_quit = cx.on_app_quit(|this, cx| {
            this.clear_clipboard_if_ours(cx);
            async {}
        });

        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut this = Self {
            path,
            vault,
            entries,
            groups,
            filter: Filter::All,
            visible: vec![],
            selected: None,
            search,
            focus,
            scroll: UniformListScrollHandle::new(),
            revealed: false,
            reveal_until: None,
            clip_hash: None,
            clip_until: None,
            toast: None,
            _ticker: ticker,
            _on_quit: on_quit,
        };
        this.refilter(cx);
        this
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.clip_until.is_some_and(|t| now >= t) {
            self.clear_clipboard_if_ours(cx);
        }
        if self.reveal_until.is_some_and(|t| now >= t) {
            self.revealed = false;
            self.reveal_until = None;
        }
        if self.toast.as_ref().is_some_and(|(_, t)| now >= *t) {
            self.toast = None;
        }
        if self.clip_until.is_some() || self.reveal_until.is_some() {
            cx.notify();
        }
    }

    /// Only clears if the clipboard still holds what we put there.
    pub fn clear_clipboard_if_ours(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.clip_hash.take() {
            let current = cx.read_from_clipboard().and_then(|c| c.text());
            if current.as_deref().map(hash) == Some(h) {
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
            if let Some(mut s) = current {
                zeroize::Zeroize::zeroize(&mut s);
            }
        }
        self.clip_until = None;
        cx.notify();
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let q = self.search.read(cx).text().to_lowercase();
        let now = now_secs() as u64;
        let keep = self.selected.map(|s| self.entries[s].index);
        self.visible = (0..self.entries.len())
            .filter(|&i| model::matches(&self.entries[i], &self.filter, &q, now))
            .collect();
        self.selected = keep
            .and_then(|k| self.visible.iter().copied().find(|&i| self.entries[i].index == k))
            .or_else(|| self.visible.first().copied());
        self.revealed = false;
        cx.notify();
    }

    fn set_filter(&mut self, f: Filter, cx: &mut Context<Self>) {
        self.filter = f;
        self.refilter(cx);
    }

    fn select_visible(&mut self, pos: usize, cx: &mut Context<Self>) {
        if let Some(&i) = self.visible.get(pos) {
            self.selected = Some(i);
            self.revealed = false;
            self.scroll.scroll_to_item(pos, ScrollStrategy::Top);
            cx.notify();
        }
    }

    pub fn select_title(&mut self, title: &str, cx: &mut Context<Self>) {
        if let Some(pos) = self.visible.iter().position(|&i| self.entries[i].title == title) {
            self.select_visible(pos, cx);
        }
    }

    fn selected_pos(&self) -> Option<usize> {
        let s = self.selected?;
        self.visible.iter().position(|&i| i == s)
    }

    fn copy(&mut self, kind: u8, label: &str, cx: &mut Context<Self>) {
        let Some(sel) = self.selected else { return };
        let r = &self.vault.records[self.entries[sel].index];
        let Some(text) = r.text(kind).filter(|t| !t.is_empty()) else { return };
        // TODO(M3): per-OS "concealed / exclude from history" clipboard hints (keywhere-sys).
        self.clip_hash = Some(hash(text));
        self.clip_until = Some(Instant::now() + CLIPBOARD_CLEAR);
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        self.toast = Some((format!("{label} copied").into(), Instant::now() + Duration::from_secs(3)));
        cx.notify();
    }

    fn toggle_reveal(&mut self, cx: &mut Context<Self>) {
        self.revealed = !self.revealed;
        self.reveal_until = self.revealed.then(|| Instant::now() + REVEAL_FOR);
        cx.notify();
    }

    fn open_url(&mut self, cx: &mut Context<Self>) {
        let Some(sel) = self.selected else { return };
        if let Some(url) = self.vault.records[self.entries[sel].index].text(rec::URL) {
            // Only hand web URLs to the OS opener.
            if url.starts_with("https://") || url.starts_with("http://") {
                cx.open_url(url);
            }
        }
    }

    // ---------- rendering ----------

    #[allow(clippy::too_many_arguments)]
    fn nav_item(
        &self,
        id: SharedString,
        icon: Option<&'static str>,
        label: SharedString,
        count: usize,
        depth: usize,
        filter: Filter,
        badge: Option<u32>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.filter == filter;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .h(px(28.))
            .pl(px(8. + depth as f32 * 14.))
            .pr_2()
            .rounded(px(6.))
            .cursor_pointer()
            .text_color(c(if on { TEXT } else { TEXT_SOFT }))
            .when(on, |d| d.bg(c(SELECTION)))
            .when(!on, |d| d.hover(|d| d.bg(c(HOVER))))
            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter.clone(), cx)))
            .when_some(icon, |d, i| d.child(svg().path(i).size(px(13.)).text_color(c(if on { TEXT } else { MUTED }))))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().child(label))
            .child(match badge {
                Some(col) if count > 0 => div()
                    .px(px(6.))
                    .rounded(px(8.))
                    .text_size(px(10.5))
                    .bg(ca(col, 0x2a))
                    .text_color(c(col))
                    .child(count.to_string()),
                _ => div().text_size(px(11.)).text_color(c(DIM)).child(count.to_string()),
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let now = now_secs() as u64;
        let expiring = self.entries.iter().filter(|e| model::is_expiring(e, now)).count();
        let mut col = div()
            .id("sidebar")
            .w(px(200.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .bg(c(SURFACE))
            .border_r_1()
            .border_color(c(LINE))
            .p_2()
            .flex()
            .flex_col()
            .gap(px(1.))
            .text_size(px(12.5))
            .child(self.nav_item("all".into(), Some("icons/list.svg"), "All entries".into(), self.entries.len(), 0, Filter::All, None, cx))
            .child(self.nav_item("exp".into(), Some("icons/clock.svg"), "Expiring".into(), expiring, 0, Filter::Expiring, Some(WARN), cx))
            .child(
                div()
                    .mt_3()
                    .mb_1()
                    .px_2()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(DIM))
                    .child("GROUPS"),
            );
        for (i, g) in self.groups.iter().enumerate() {
            let label: SharedString = g.path.last().cloned().unwrap_or_default().into();
            col = col.child(self.nav_item(
                SharedString::from(format!("g{i}")),
                (g.path.len() == 1).then_some("icons/folder.svg"),
                label,
                g.count,
                g.path.len() - 1,
                Filter::Group(g.path.clone()),
                None,
                cx,
            ));
        }
        col
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title: SharedString = match &self.filter {
            Filter::All => "All entries".into(),
            Filter::Expiring => "Expiring".into(),
            Filter::Group(p) => p.join(" / ").into(),
        };
        let n = self.visible.len();
        div()
            .w(px(270.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(c(BG))
            .border_r_1()
            .border_color(c(LINE))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .px_3()
                    .h(px(36.))
                    .items_center()
                    .border_b_1()
                    .border_color(c(LINE))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(c(TEXT)).child(title))
                    .child(div().text_size(px(11.)).text_color(c(DIM)).child(format!("{n}"))),
            )
            .child(if n == 0 {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c(DIM))
                    .child("No matching entries")
                    .into_any_element()
            } else {
                uniform_list(
                    "entries",
                    n,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|pos| this.render_row(pos, cx).into_any_element())
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .flex_1()
                .into_any_element()
            })
    }

    fn render_row(&self, pos: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let i = self.visible[pos];
        let e = &self.entries[i];
        let on = self.selected == Some(i);
        let now = now_secs() as u64;
        let initial: SharedString = e.title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into();
        let sub: SharedString = match (e.username.is_empty(), e.group_label.is_empty(), matches!(self.filter, Filter::Group(_))) {
            (false, false, false) => format!("{} · {}", e.username, e.group_label).into(),
            (false, _, _) => e.username.clone(),
            (true, false, _) => e.group_label.clone(),
            _ => "".into(),
        };
        let exp_badge = e.expiry.filter(|_| model::is_expiring(e, now)).map(|t| {
            if t <= now { ("expired".to_string(), DANGER) } else { (format!("{}d", (t - now).div_ceil(86_400)), WARN) }
        });
        div()
            .id(pos)
            .w_full()
            .h(px(48.))
            .flex()
            .items_center()
            .gap(px(10.))
            .px_3()
            .cursor_pointer()
            .border_b_1()
            .border_color(ca(BORDER, 0x55))
            .when(on, |d| d.bg(c(SELECTION)).border_l_2().border_color(c(ACCENT)))
            .when(!on, |d| d.hover(|d| d.bg(c(HOVER))))
            .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                this.select_visible(pos, cx);
                if ev.click_count() >= 2 {
                    this.copy(rec::PASSWORD, "Password", cx);
                }
            }))
            .child(
                div()
                    .size(px(30.))
                    .flex_none()
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .bg(c(if on { ACCENT } else { AVATAR }))
                    .text_color(c(if on { ACCENT_INK } else { AVATAR_INK }))
                    .child(initial),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(c(TEXT)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(e.title.clone()))
                    .child(div().text_size(px(11.)).text_color(c(MUTED)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(sub)),
            )
            .when_some(exp_badge, |d, (t, col)| {
                d.child(div().px(px(6.)).rounded(px(8.)).text_size(px(10.)).bg(ca(col, 0x2a)).text_color(c(col)).child(t))
            })
    }

    fn icon_btn(id: &'static str, icon: &'static str, tip: &'static str) -> gpui::Stateful<gpui::Div> {
        let _ = tip;
        div()
            .id(id)
            .size(px(24.))
            .flex_none()
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|d| d.bg(c(RAISED)))
            .child(svg().path(icon).size(px(14.)).text_color(c(MUTED)))
    }

    fn field_row(label: &'static str, value: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_h(px(36.))
            .px_3()
            .border_b_1()
            .border_color(c(LINE))
            .child(div().w(px(76.)).flex_none().text_size(px(11.)).text_color(c(DIM)).child(label))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(value))
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let base = div().flex_1().h_full().bg(c(PANEL));
        let Some(sel) = self.selected else {
            return base
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(DIM))
                .child("Select an entry")
                .into_any_element();
        };
        let e = &self.entries[sel];
        let r = &self.vault.records[e.index];
        let now = now_secs() as u64;
        let pw = r.text(rec::PASSWORD).unwrap_or("");
        let bits = model::strength_bits(pw);
        let bars = if pw.is_empty() { 0 } else if bits < 40. { 1 } else if bits < 60. { 2 } else if bits < 80. { 3 } else { 4 };
        let bar_col = match bars { 1 => DANGER, 2 => WARN, _ => OK };
        let pw_shown: SharedString = if self.revealed {
            // Short-lived: hidden again after REVEAL_FOR or on lock.
            pw.to_string().into()
        } else if pw.is_empty() {
            "—".into()
        } else {
            "••••••••••••".into()
        };

        let mut fields = div().mt_3().rounded(px(9.)).border_1().border_color(c(LINE)).bg(c(SURFACE)).overflow_hidden();
        if let Some(u) = r.text(rec::USERNAME).filter(|s| !s.is_empty()) {
            fields = fields.child(
                Self::field_row("Username", div().text_color(c(TEXT)).child(SharedString::from(u.to_string())))
                    .child(Self::icon_btn("cu", "icons/copy.svg", "Copy username").on_click(cx.listener(|t, _, _, cx| t.copy(rec::USERNAME, "Username", cx)))),
            );
        }
        fields = fields.child(
            Self::field_row(
                "Password",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(c(TEXT)).font_family("monospace").child(pw_shown))
                    .when(bars > 0, |d| {
                        d.child(div().flex().gap(px(3.)).children((0..4).map(move |i| {
                            div().w(px(14.)).h(px(4.)).rounded(px(2.)).bg(c(if i < bars { bar_col } else { RAISED }))
                        })))
                    }),
            )
            .when(!pw.is_empty(), |d| {
                d.child(
                    Self::icon_btn("rv", if self.revealed { "icons/eye-off.svg" } else { "icons/eye.svg" }, "Reveal")
                        .on_click(cx.listener(|t, _, _, cx| t.toggle_reveal(cx))),
                )
                .child(Self::icon_btn("cp", "icons/copy.svg", "Copy password").on_click(cx.listener(|t, _, _, cx| t.copy(rec::PASSWORD, "Password", cx))))
            }),
        );
        if let Some(u) = r.text(rec::URL).filter(|s| !s.is_empty()) {
            let web = u.starts_with("http://") || u.starts_with("https://");
            fields = fields.child(
                Self::field_row("Website", div().text_color(c(if web { LINK } else { TEXT })).child(SharedString::from(u.to_string())))
                    .child(Self::icon_btn("cl", "icons/copy.svg", "Copy URL").on_click(cx.listener(|t, _, _, cx| t.copy(rec::URL, "URL", cx))))
                    .when(web, |d| d.child(Self::icon_btn("ou", "icons/external.svg", "Open").on_click(cx.listener(|t, _, _, cx| t.open_url(cx))))),
            );
        }
        if let Some(m) = r.text(rec::EMAIL).filter(|s| !s.is_empty()) {
            fields = fields.child(
                Self::field_row("Email", div().text_color(c(TEXT)).child(SharedString::from(m.to_string())))
                    .child(Self::icon_btn("ce", "icons/copy.svg", "Copy email").on_click(cx.listener(|t, _, _, cx| t.copy(rec::EMAIL, "Email", cx)))),
            );
        }

        let notes = r.text(rec::NOTES).filter(|s| !s.is_empty()).map(|n| SharedString::from(n.replace("\r\n", "\n")));
        let mut meta: Vec<String> = vec![];
        if let Some(t) = r.time(rec::CTIME) { meta.push(format!("Created {}", model::fmt_date(t))); }
        if let Some(t) = r.time(rec::MTIME) { meta.push(format!("Modified {}", model::fmt_date(t))); }
        if let Some(t) = r.time(rec::PTIME) { meta.push(format!("Password changed {}", model::fmt_date(t))); }
        let hist = model::history_count(r);
        if hist > 0 { meta.push(format!("Password history · {hist}")); }

        let initial: SharedString = e.title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into();

        base.id("detail")
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().size(px(44.)).rounded(px(10.)).bg(c(ACCENT)).text_color(c(ACCENT_INK)).flex().items_center().justify_center().text_size(px(18.)).font_weight(FontWeight::BOLD).child(initial))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).text_color(c(TEXT)).child(e.title.clone()))
                            .child(div().text_size(px(11.5)).text_color(c(MUTED)).child(if e.group_label.is_empty() { SharedString::from("No group") } else { e.group_label.clone() })),
                    ),
            )
            .when_some(e.expiry, |d, exp| {
                let (msg, past) = model::fmt_relative_expiry(exp, now);
                let col = if past { DANGER } else { WARN };
                d.when(model::is_expiring(e, now), |d| {
                    d.child(
                        div()
                            .mt_3()
                            .px_3()
                            .py_2()
                            .rounded(px(8.))
                            .bg(ca(col, 0x1a))
                            .border_1()
                            .border_color(ca(col, 0x60))
                            .text_color(c(col))
                            .text_size(px(12.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(svg().path("icons/clock.svg").size(px(13.)).text_color(c(col)))
                            .child(msg),
                    )
                })
            })
            .child(fields)
            .when_some(notes, |d, n| {
                d.child(
                    div()
                        .mt_3()
                        .p_3()
                        .rounded(px(9.))
                        .border_1()
                        .border_color(c(LINE))
                        .bg(c(SURFACE))
                        .text_color(c(TEXT_SOFT))
                        .child(div().text_size(px(10.5)).text_color(c(DIM)).mb_1().child("NOTES"))
                        .child(n),
                )
            })
            .child(div().mt_3().flex().flex_wrap().gap_x_4().gap_y_1().text_size(px(11.)).text_color(c(DIM)).children(meta))
            .into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clip_left = self.clip_until.map(|t| t.saturating_duration_since(Instant::now()).as_secs());
        let file: SharedString = self
            .path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| config::display_path(&self.path))
            .into();

        div()
            .id("browser")
            .key_context("Browser")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(c(BG))
            .text_color(c(TEXT))
            .text_size(px(12.5))
            .on_action(cx.listener(|_, _: &Lock, _, cx| cx.emit(BrowserEvent::LockRequested)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| window.focus(&this.search.focus_handle(cx))))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                let p = this.selected_pos().map_or(0, |p| (p + 1).min(this.visible.len().saturating_sub(1)));
                this.select_visible(p, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                let p = this.selected_pos().map_or(0, |p| p.saturating_sub(1));
                this.select_visible(p, cx);
            }))
            .on_action(cx.listener(|this, _: &CopyPassword, _, cx| this.copy(rec::PASSWORD, "Password", cx)))
            .on_action(cx.listener(|this, _: &CopyUsername, _, cx| this.copy(rec::USERNAME, "Username", cx)))
            .on_action(cx.listener(|this, _: &OpenUrl, _, cx| this.open_url(cx)))
            .on_action(cx.listener(|this, _: &ToggleReveal, _, cx| this.toggle_reveal(cx)))
            // toolbar
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .bg(c(SURFACE))
                    .border_b_1()
                    .border_color(c(LINE))
                    .child(crate::assets::logo(20.).flex_none())
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Keywhere"))
                    .child(div().flex_none().text_color(c(DIM)).text_size(px(11.5)).child(file))
                    .child(div().flex_1().flex().justify_center().child(div().w(px(340.)).child(self.search.clone())))
                    .child(
                        div()
                            .id("lock")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .h(px(26.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(c(BORDER))
                            .bg(c(RAISED))
                            .cursor_pointer()
                            .hover(|d| d.bg(c(HOVER)))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(BrowserEvent::LockRequested)))
                            .child(svg().path("icons/lock.svg").size(px(13.)).text_color(c(TEXT_SOFT)))
                            .child(div().text_size(px(11.5)).child("Lock")),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_sidebar(cx))
                    .child(self.render_list(cx))
                    .child(self.render_detail(cx)),
            )
            // status bar
            .child(
                div()
                    .h(px(26.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_3()
                    .bg(c(SURFACE))
                    .border_t_1()
                    .border_color(c(LINE))
                    .text_size(px(11.))
                    .text_color(c(DIM))
                    .child(div().flex().items_center().gap_1().child(div().size(px(7.)).rounded_full().bg(c(OK))).child("Read‑only · psafe3"))
                    .child(format!("{} entries", self.entries.len()))
                    .child(div().flex_1())
                    .when_some(self.toast.as_ref().map(|t| t.0.clone()), |d, t| {
                        d.child(div().flex().items_center().gap_1().text_color(c(OK)).child(svg().path("icons/check.svg").size(px(12.)).text_color(c(OK))).child(t))
                    })
                    .when_some(clip_left, |d, s| d.child(format!("Clipboard clears in {s}s")))
                    .child(if cfg!(target_os = "macos") {
                        "⌘C password · ⌘B user · ⌘L lock"
                    } else {
                        "Ctrl+C password · Ctrl+B user · Ctrl+L lock"
                    }),
            )
    }
}

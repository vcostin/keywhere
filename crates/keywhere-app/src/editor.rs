//! Entry editor (add / edit) with an inline password generator.

use gpui::{
    div, prelude::*, px, svg, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, SharedString, Window,
};
use keywhere_core::generator::{self, Mode, Options};
use keywhere_core::{rec, Record};
use zeroize::Zeroizing;

use crate::field::{FieldEvent, TextField};
use crate::i18n::tf;
use crate::model;
use crate::theme::*;

pub enum EditorEvent {
    Save,
    Cancel,
}

pub struct EntryEditor {
    /// UUID of the record being edited; `None` for a new entry.
    pub target: Option<Vec<u8>>,
    /// The group as loaded, so an untouched group is written back verbatim
    /// (segments may themselves contain '/', e.g. "CI/CD").
    original_group: Vec<String>,
    title: Entity<TextField>,
    group: Entity<TextField>,
    username: Entity<TextField>,
    password: Entity<TextField>,
    url: Entity<TextField>,
    email: Entity<TextField>,
    expiry: Entity<TextField>,
    notes: Entity<TextField>,
    gen_open: bool,
    gen_opts: Options,
    gen_preview: Zeroizing<String>,
    pub error: Option<SharedString>,
    /// Existing groups (display form "Work / Dev") for the group combobox.
    groups: Vec<String>,
    group_menu: bool,
    group_hl: usize,
    /// Date picker for "Expires": open + the month shown (year, month).
    cal: Option<(i64, u32)>,
    focus: FocusHandle,
}

impl EventEmitter<EditorEvent> for EntryEditor {}

impl Focusable for EntryEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Validated form contents, ready to apply to a record.
pub struct Form {
    pub title: String,
    pub group: Vec<String>,
    pub username: String,
    pub password: Zeroizing<String>,
    pub url: String,
    pub email: String,
    pub expiry: Option<u32>,
    pub notes: Zeroizing<String>,
}

impl Form {
    pub fn apply(&self, r: &mut Record, now: u32) {
        r.set_or_clear(rec::TITLE, &self.title);
        let segs: Vec<&str> = self.group.iter().map(String::as_str).collect();
        if segs.is_empty() {
            r.remove(rec::GROUP);
        } else {
            r.set_group_path(&segs);
        }
        r.set_or_clear(rec::USERNAME, &self.username);
        r.set_or_clear(rec::URL, &self.url);
        r.set_or_clear(rec::EMAIL, &self.email);
        r.set_or_clear(rec::NOTES, &self.notes);
        match self.expiry {
            Some(t) => r.set_time(rec::EXPIRY, t),
            None => r.remove(rec::EXPIRY),
        }
        r.set_password(&self.password, now);
        r.set_time(rec::MTIME, now);
    }
}

/// Levels are shown separated by " / " (slash with spaces), so a single
/// group name may itself contain '/' (e.g. "CI/CD").
pub fn group_display(path: &[String]) -> String {
    path.join(" / ")
}

pub fn parse_group(text: &str) -> Vec<String> {
    text.split(" / ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

impl EntryEditor {
    pub fn new(
        record: Option<&Record>,
        default_group: &[String],
        groups: Vec<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = |cx: &mut Context<Self>, ph: &'static str, secret: bool| {
            // Every editor field shows vault content: use the wiped path.
            cx.new(|cx| TextField::new(cx, ph, secret).sensitive())
        };
        let title = field(cx, "Title (required)", false);
        let group = field(cx, "Group — pick or type, levels separated by “ / ”", false);
        let username = field(cx, "Username", false);
        let password = field(cx, "Password", true);
        let url = field(cx, "https://…", false);
        let email = field(cx, "Email", false);
        let expiry = field(cx, "YYYY-MM-DD (optional)", false);
        let notes = cx.new(|cx| TextField::new(cx, "Notes", false).multiline().sensitive());

        let get = |k| record.and_then(|r| r.text(k)).unwrap_or("").to_string();
        let set = |e: &Entity<TextField>, s: &str, cx: &mut Context<Self>| {
            e.update(cx, |f, cx| f.set_text(s, cx))
        };
        set(&title, &get(rec::TITLE), cx);
        let g = record
            .map(|r| r.group_path())
            .unwrap_or_else(|| default_group.to_vec());
        set(&group, &group_display(&g), cx);
        set(&username, &get(rec::USERNAME), cx);
        {
            let pw = Zeroizing::new(get(rec::PASSWORD));
            set(&password, &pw, cx);
        }
        set(&url, &get(rec::URL), cx);
        set(&email, &get(rec::EMAIL), cx);
        let exp = record
            .and_then(|r| r.time(rec::EXPIRY))
            .filter(|t| *t > 0)
            .map(model::fmt_iso_date)
            .unwrap_or_default();
        set(&expiry, &exp, cx);
        {
            let n = Zeroizing::new(get(rec::NOTES));
            set(&notes, &n, cx);
        }

        // Group field: a combobox (suggestions from existing groups).
        cx.subscribe_in(&group, window, |this, _, ev, _, cx| {
            match ev {
                FieldEvent::Changed => {
                    this.group_menu = true;
                    this.group_hl = 0;
                    this.error = None;
                }
                FieldEvent::Submit => {
                    if this.group_menu && this.pick_group(cx) {
                        return;
                    }
                    cx.emit(EditorEvent::Save);
                }
                FieldEvent::Escape => {
                    if this.group_menu {
                        this.group_menu = false;
                    } else {
                        cx.emit(EditorEvent::Cancel);
                    }
                }
            }
            cx.notify();
        })
        .detach();
        for f in [&title, &username, &password, &url, &email, &expiry, &notes] {
            cx.subscribe_in(f, window, |this, _, ev, _, cx| match ev {
                FieldEvent::Submit => cx.emit(EditorEvent::Save),
                FieldEvent::Escape => cx.emit(EditorEvent::Cancel),
                FieldEvent::Changed => {
                    if this.error.is_some() {
                        this.error = None;
                    }
                    cx.notify();
                }
            })
            .detach();
        }
        window.focus(&title.focus_handle(cx));

        let mut this = Self {
            target: record.and_then(|r| r.uuid()).map(<[u8]>::to_vec),
            original_group: g,
            groups: groups.iter().map(|g| group_display(g)).collect(),
            group_menu: false,
            group_hl: 0,
            cal: None,
            title,
            group,
            username,
            password,
            url,
            email,
            expiry,
            notes,
            gen_open: record.is_none(),
            gen_opts: Options::default(),
            gen_preview: Zeroizing::new(String::new()),
            error: None,
            focus: cx.focus_handle(),
        };
        this.regenerate();
        if record.is_none() {
            // New entries start with a strong generated password.
            let p = this.gen_preview.clone();
            this.password.update(cx, |f, cx| f.set_text(&p, cx));
        }
        this
    }

    /// DEV: reproduce the "12-word passphrase" layout case for screenshots.
    #[cfg(feature = "devtools")]
    pub fn dev_long_passphrase(&mut self, cx: &mut Context<Self>) {
        self.gen_opts.mode = Mode::Passphrase;
        self.gen_opts.length = 12;
        self.gen_open = true;
        self.regenerate();
        // Exact preview from KEYWHERE_DEV_PREVIEW for layout sweeps, else an
        // overlong one so wrapping is exercised.
        let long = std::env::var("KEYWHERE_DEV_PREVIEW").unwrap_or_else(|_| {
            format!(
                "{}-cause-ranch-woven-humid-blimp-pest-yoga-icing",
                self.gen_preview.as_str()
            )
        });
        self.gen_preview = Zeroizing::new(long);
        cx.notify();
    }

    #[cfg(feature = "devtools")]
    pub fn dev_group_menu(&mut self, typed: &str, cx: &mut Context<Self>) {
        self.group.update(cx, |f, cx| f.set_text(typed, cx));
        self.group_menu = true;
        self.group_hl = 0;
        cx.notify();
    }

    /// Existing groups matching what's typed (all of them when empty), and
    /// the typed text as a new group if it isn't one already.
    fn group_suggestions(&self, cx: &App) -> (Vec<String>, Option<String>) {
        let typed = self.group.read(cx).text().trim().to_string();
        let q = typed.to_lowercase();
        let matches: Vec<String> = self
            .groups
            .iter()
            .filter(|g| q.is_empty() || g.to_lowercase().contains(&q))
            .cloned()
            .collect();
        let normalized = group_display(&parse_group(&typed));
        let new =
            (!normalized.is_empty() && !self.groups.contains(&normalized)).then_some(normalized);
        (matches, new)
    }

    /// Apply the highlighted suggestion. Returns false if there was nothing
    /// to pick (Enter then saves the form).
    fn pick_group(&mut self, cx: &mut Context<Self>) -> bool {
        let (matches, new) = self.group_suggestions(cx);
        let choice = matches
            .get(self.group_hl)
            .cloned()
            .or(if self.group_hl == matches.len() {
                new
            } else {
                None
            });
        self.group_menu = false;
        match choice {
            Some(g) => {
                self.group.update(cx, |f, cx| f.set_text(&g, cx));
                self.group_menu = false; // set_text emitted Changed
                cx.notify();
                true
            }
            None => false,
        }
    }

    fn render_group(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (matches, new) = if self.group_menu {
            self.group_suggestions(cx)
        } else {
            (vec![], None)
        };
        let total = matches.len() + new.is_some() as usize;
        let hl = self.group_hl.min(total.saturating_sub(1));
        let row = |id: (&'static str, usize), on: bool| {
            div()
                .id(id)
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(c(TEXT_SOFT))
                .when(on, |d| d.bg(c(SELECTION)).text_color(c(TEXT)))
                .when(!on, |d| d.hover(|d| d.bg(c(HOVER))))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let list = (self.group_menu && total > 0).then(|| {
            gpui::deferred(
                gpui::anchored()
                    .offset(gpui::point(px(0.), px(34.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("group-list")
                            .occlude()
                            .w(px(360.))
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .p_1()
                            .rounded(px(9.))
                            .bg(c(PANEL))
                            .border_1()
                            .border_color(c(0x2A4A85))
                            .shadow_lg()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.group_menu = false;
                                cx.notify();
                            }))
                            .children(matches.iter().enumerate().map(|(i, g)| {
                                let g2 = g.clone();
                                row(("grp", i), i == hl)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.group.update(cx, |f, cx| f.set_text(&g2, cx));
                                        this.group_menu = false;
                                        cx.notify();
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/folder.svg")
                                            .size(px(13.))
                                            .text_color(c(MUTED)),
                                    )
                                    .child(SharedString::from(g.clone()))
                            }))
                            .when_some(new.clone(), |d, n| {
                                let n2 = n.clone();
                                let i = matches.len();
                                d.child(
                                    row(("grp-new", i), i == hl)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.group.update(cx, |f, cx| f.set_text(&n2, cx));
                                            this.group_menu = false;
                                            cx.notify();
                                        }))
                                        .child(div().text_color(c(ACCENT)).child("+"))
                                        .child(SharedString::from(tf(
                                            "New group: {name}",
                                            &[("name", &n)],
                                        ))),
                                )
                            }),
                    ),
            )
            .with_priority(1)
        });
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(self.group.clone()))
                    .child(
                        div()
                            .id("group-toggle")
                            .size(px(30.))
                            .flex_none()
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(c(RAISED))
                            .border_1()
                            .border_color(c(BORDER))
                            .hover(|d| d.bg(c(HOVER)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.group_menu = !this.group_menu;
                                this.group_hl = 0;
                                window.focus(&this.group.focus_handle(cx));
                                cx.notify();
                            }))
                            .child(
                                svg()
                                    .path("icons/chevron-down.svg")
                                    .size(px(14.))
                                    .text_color(c(MUTED)),
                            ),
                    ),
            )
            .children(list)
    }

    #[cfg(feature = "devtools")]
    pub fn dev_open_calendar(&mut self, cx: &mut Context<Self>) {
        self.open_calendar(cx);
    }

    fn open_calendar(&mut self, cx: &mut Context<Self>) {
        let secs = model::parse_iso_date(self.expiry.read(cx).text())
            .map(u64::from)
            .unwrap_or(keywhere_core::now_secs() as u64);
        let (y, m, _) = model::civil(secs);
        self.cal = Some((y, m));
        cx.notify();
    }

    fn set_expiry(&mut self, secs: Option<u64>, cx: &mut Context<Self>) {
        let text = secs.map(model::fmt_iso_date).unwrap_or_default();
        self.expiry.update(cx, |f, cx| f.set_text(&text, cx));
        self.cal = None;
        cx.notify();
    }

    fn render_expiry(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle = div()
            .id("cal-toggle")
            .size(px(30.))
            .flex_none()
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(c(RAISED))
            .border_1()
            .border_color(c(BORDER))
            .hover(|d| d.bg(c(HOVER)))
            .on_click(cx.listener(|this, _, _, cx| {
                if this.cal.is_some() {
                    this.cal = None;
                    cx.notify();
                } else {
                    this.open_calendar(cx);
                }
            }))
            .child(
                svg()
                    .path("icons/calendar.svg")
                    .size(px(14.))
                    .text_color(c(MUTED)),
            );
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(self.expiry.clone()))
                    .child(toggle),
            )
            .children(self.cal.map(|(y, m)| self.render_calendar(y, m, cx)))
    }

    fn render_calendar(&self, y: i64, m: u32, cx: &mut Context<Self>) -> impl IntoElement {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
        let today = keywhere_core::now_secs() as u64;
        let (ty, tm, td) = model::civil(today);
        let selected =
            model::parse_iso_date(self.expiry.read(cx).text()).map(|s| model::civil(s as u64));
        let first = model::weekday_mon0(y, m, 1);
        let days = model::days_in_month(y, m);

        let nav = |id: &'static str, icon: &'static str| {
            div()
                .id(id)
                .size(px(26.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|d| d.bg(c(RAISED)))
                .child(svg().path(icon).size(px(14.)).text_color(c(MUTED)))
        };
        let preset = |id: &'static str, label: String| {
            div()
                .id(id)
                .px(px(8.))
                .py(px(3.))
                .rounded(px(12.))
                .border_1()
                .border_color(c(BORDER))
                .text_size(px(11.))
                .text_color(c(TEXT_SOFT))
                .cursor_pointer()
                .hover(|d| d.border_color(c(0x4E7BE0)))
                .child(label)
        };

        let mut grid = div().flex().flex_wrap().w(px(7. * 34.));
        for wd in WEEKDAYS {
            grid = grid.child(
                div()
                    .w(px(34.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(10.5))
                    .text_color(c(DIM))
                    .child(crate::i18n::t(wd)),
            );
        }
        for _ in 0..first {
            grid = grid.child(div().w(px(34.)).h(px(30.)));
        }
        for d in 1..=days {
            let is_today = (y, m, d) == (ty, tm, td);
            let is_sel = selected == Some((y, m, d));
            let secs = (model::days_from_civil(y, m, d) * 86_400) as u64;
            grid = grid.child(
                div()
                    .id(("day", d as usize))
                    .w(px(34.))
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .cursor_pointer()
                            .text_color(c(if is_sel { ACCENT_INK } else { TEXT_SOFT }))
                            .when(is_sel, |x| {
                                x.bg(c(ACCENT)).font_weight(FontWeight::SEMIBOLD)
                            })
                            .when(!is_sel && is_today, |x| x.border_1().border_color(c(FOCUS)))
                            .when(!is_sel, |x| x.hover(|x| x.bg(c(HOVER))))
                            .child(d.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.set_expiry(Some(secs), cx))),
            );
        }

        let shift = move |dm: i32| {
            let idx = y * 12 + (m as i64 - 1) + dm as i64;
            (idx.div_euclid(12), (idx.rem_euclid(12) + 1) as u32)
        };
        let (py, pm) = shift(-1);
        let (ny, nm) = shift(1);
        let in_days = move |n: u64| today - today % 86_400 + n * 86_400;

        gpui::deferred(
            gpui::anchored()
                .offset(gpui::point(px(0.), px(34.)))
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("calendar")
                        .occlude()
                        .p_3()
                        .rounded(px(10.))
                        .bg(c(PANEL))
                        .border_1()
                        .border_color(c(0x2A4A85))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.cal = None;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(nav("cal-prev", "icons/chevron-left.svg").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.cal = Some((py, pm));
                                        cx.notify();
                                    }),
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .flex()
                                        .justify_center()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(c(TEXT))
                                        .child(SharedString::from(format!(
                                            "{} {y}",
                                            crate::i18n::t(MONTHS[(m - 1) as usize])
                                        ))),
                                )
                                .child(nav("cal-next", "icons/chevron-right.svg").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.cal = Some((ny, nm));
                                        cx.notify();
                                    }),
                                )),
                        )
                        .child(grid)
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(px(6.))
                                .pt_1()
                                .border_t_1()
                                .border_color(c(LINE))
                                .child(
                                    preset("cal-30", tf("In {n} days", &[("n", "30")])).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_expiry(Some(in_days(30)), cx)
                                        }),
                                    ),
                                )
                                .child(
                                    preset("cal-90", tf("In {n} days", &[("n", "90")])).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_expiry(Some(in_days(90)), cx)
                                        }),
                                    ),
                                )
                                .child(
                                    preset("cal-365", crate::i18n::t("In 1 year").to_string())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_expiry(Some(in_days(365)), cx)
                                        })),
                                )
                                .child(
                                    preset("cal-clear", crate::i18n::t("Clear").to_string())
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.set_expiry(None, cx)),
                                        ),
                                ),
                        ),
                ),
        )
        .with_priority(1)
    }

    fn regenerate(&mut self) {
        if let Ok(p) = generator::generate(&self.gen_opts) {
            self.gen_preview = p;
        }
    }

    /// Validate and collect the form.
    pub fn form(&self, cx: &App) -> Result<Form, String> {
        let t = |e: &Entity<TextField>| e.read(cx).text().trim().to_string();
        let title = t(&self.title);
        if title.is_empty() {
            return Err(crate::i18n::t("Title is required.").into());
        }
        let group_txt = self.group.read(cx).text();
        let group = if group_txt == group_display(&self.original_group) {
            self.original_group.clone()
        } else {
            parse_group(group_txt)
        };
        let exp_txt = t(&self.expiry);
        let expiry = if exp_txt.is_empty() {
            None
        } else {
            Some(
                model::parse_iso_date(&exp_txt)
                    .ok_or(crate::i18n::t("Expiry must be a date like 2027-03-31."))?,
            )
        };
        Ok(Form {
            title,
            group,
            username: t(&self.username),
            password: Zeroizing::new(self.password.read(cx).text().to_string()),
            url: t(&self.url),
            email: t(&self.email),
            expiry,
            notes: Zeroizing::new(self.notes.read(cx).text().trim_end().to_string()),
        })
    }

    fn row(label: &'static str, field: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .w(px(76.))
                    .pt(px(8.))
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(c(DIM))
                    .child(label),
            )
            // overflow_hidden + min_w_0: long content (e.g. a passphrase) must
            // never set this column's minimum width (GPUI measures text's
            // min-content as the whole unwrapped line).
            .child(div().flex_1().min_w_0().overflow_hidden().child(field))
    }

    fn chip(id: &'static str, label: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px(px(9.))
            .py(px(3.))
            .rounded(px(12.))
            .border_1()
            .cursor_pointer()
            .text_size(px(11.))
            .border_color(c(if on { 0x4E7BE0 } else { BORDER }))
            .bg(if on { ca(FOCUS, 0x2e) } else { ca(BG, 0) })
            .text_color(c(if on { 0xCFDCFF } else { MUTED }))
            .child(label)
    }

    fn small_btn(
        id: &'static str,
        label: impl Into<SharedString>,
        primary: bool,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(px(12.))
            .when(primary, |d| {
                d.bg(c(ACCENT))
                    .text_color(c(ACCENT_INK))
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!primary, |d| {
                d.bg(c(RAISED))
                    .border_1()
                    .border_color(c(BORDER))
                    .text_color(c(TEXT))
                    .hover(|d| d.bg(c(HOVER)))
            })
            .child(label.into())
    }

    fn render_generator(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let o = &self.gen_opts;
        let bits = o.entropy_bits();
        let label = match bits as u32 {
            0..=49 => crate::i18n::t("Weak"),
            50..=79 => crate::i18n::t("Fair"),
            80..=109 => crate::i18n::t("Strong"),
            _ => crate::i18n::t("Excellent"),
        };
        let seg = |id: &'static str, text: &'static str, mode: Mode, cx: &mut Context<Self>| {
            let on = self.gen_opts.mode == mode;
            div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .id(id)
                .flex_1()
                .py(px(4.))
                .rounded(px(6.))
                .flex()
                .justify_center()
                .cursor_pointer()
                .text_size(px(11.5))
                .text_color(c(if on { TEXT } else { MUTED }))
                .when(on, |d| d.bg(c(RAISED)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.gen_opts.mode = mode;
                    this.gen_opts.length = match mode {
                        Mode::Random => 20,
                        Mode::Passphrase => 5,
                        Mode::Pin => 6,
                    };
                    this.regenerate();
                    cx.notify();
                }))
                .child(text)
        };
        let toggle = |id: &'static str,
                      text: &'static str,
                      on: bool,
                      f: fn(&mut Options),
                      cx: &mut Context<Self>| {
            Self::chip(id, text, on).on_click(cx.listener(move |this, _, _, cx| {
                f(&mut this.gen_opts);
                this.regenerate();
                cx.notify();
            }))
        };
        let unit = if o.mode == Mode::Passphrase {
            crate::i18n::t("words")
        } else {
            crate::i18n::t("characters")
        };

        div()
            .mt_2()
            .p_3()
            .rounded(px(9.))
            .bg(c(SURFACE))
            .border_1()
            .border_color(c(LINE))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .bg(c(BG))
                    .border_1()
                    .border_color(c(BORDER))
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_normal()
                    .font_family("monospace")
                    .text_size(px(14.))
                    .text_color(c(TEXT))
                    .child(crate::secret_text::display(&self.gen_preview)),
            )
            .child(
                div().h(px(5.)).rounded(px(3.)).bg(c(RAISED)).child(
                    div()
                        .h_full()
                        .rounded(px(3.))
                        .w(gpui::relative((bits / 128.).clamp(0.05, 1.) as f32))
                        .bg(c(if bits < 50. {
                            DANGER
                        } else if bits < 80. {
                            WARN
                        } else {
                            OK
                        })),
                ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(11.))
                    .text_color(c(MUTED))
                    .child(tf(
                        "{label} · {n} bits",
                        &[("label", label), ("n", &bits.round().to_string())],
                    ))
                    .child(
                        div()
                            .id("regen")
                            .cursor_pointer()
                            .text_color(c(LINK))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.regenerate();
                                cx.notify();
                            }))
                            .child(crate::i18n::t("Regenerate")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .p(px(3.))
                    .rounded(px(8.))
                    .bg(c(BG))
                    .border_1()
                    .border_color(c(LINE))
                    .child(seg("m-rand", crate::i18n::t("Random"), Mode::Random, cx))
                    .child(seg(
                        "m-pass",
                        crate::i18n::t("Passphrase"),
                        Mode::Passphrase,
                        cx,
                    ))
                    .child(seg("m-pin", crate::i18n::t("PIN"), Mode::Pin, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.5))
                    .text_color(c(MUTED))
                    .child(crate::i18n::t("Length"))
                    .child(Self::small_btn("len-", "−", false).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.gen_opts.length = this
                                .gen_opts
                                .length
                                .saturating_sub(1)
                                .max(this.gen_opts.min_len());
                            this.regenerate();
                            cx.notify();
                        },
                    )))
                    .child(
                        div()
                            .w(px(28.))
                            .flex()
                            .justify_center()
                            .text_color(c(TEXT))
                            .child(o.length.to_string()),
                    )
                    .child(Self::small_btn("len+", "+", false).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.gen_opts.length =
                                (this.gen_opts.length + 1).min(this.gen_opts.max_len());
                            this.regenerate();
                            cx.notify();
                        },
                    )))
                    .child(unit),
            )
            .when(o.mode == Mode::Random, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(toggle("c-up", "A‑Z", o.upper, |o| o.upper = !o.upper, cx))
                        .child(toggle("c-lo", "a‑z", o.lower, |o| o.lower = !o.lower, cx))
                        .child(toggle(
                            "c-di",
                            "0‑9",
                            o.digits,
                            |o| o.digits = !o.digits,
                            cx,
                        ))
                        .child(toggle(
                            "c-sy",
                            "!@#",
                            o.symbols,
                            |o| o.symbols = !o.symbols,
                            cx,
                        ))
                        .child(toggle(
                            "c-la",
                            crate::i18n::t("No look‑alikes"),
                            o.no_lookalikes,
                            |o| o.no_lookalikes = !o.no_lookalikes,
                            cx,
                        )),
                )
            })
            .child(div().flex().justify_end().child(
                Self::small_btn("use", crate::i18n::t("Use password"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        let p = this.gen_preview.clone();
                        this.password.update(cx, |f, cx| f.set_text(&p, cx));
                        this.gen_open = false;
                        cx.notify();
                    },
                )),
            ))
    }
}

impl Render for EntryEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (bits, pw_empty) = {
            let pw = self.password.read(cx).text();
            (model::strength_bits(pw), pw.is_empty())
        };
        let bars = if pw_empty {
            0
        } else if bits < 40. {
            1
        } else if bits < 60. {
            2
        } else if bits < 80. {
            3
        } else {
            4
        };
        let bar_col = match bars {
            1 => DANGER,
            2 => WARN,
            _ => OK,
        };
        let new = self.target.is_none();
        // Characters beyond U+FFFF (emoji etc.) don't survive Password
        // Gorilla (Tcl 8.x only handles the Basic Multilingual Plane).
        let beyond_bmp = [
            &self.title,
            &self.group,
            &self.username,
            &self.password,
            &self.url,
            &self.email,
            &self.notes,
        ]
        .iter()
        .any(|f| f.read(cx).text().chars().any(|c| c as u32 > 0xFFFF));

        div()
            .id("editor")
            .key_context("Editor")
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _, cx| {
                if !this.group_menu {
                    return;
                }
                let (m, n) = this.group_suggestions(cx);
                let total = m.len() + n.is_some() as usize;
                match ev.keystroke.key.as_str() {
                    "down" => this.group_hl = (this.group_hl + 1).min(total.saturating_sub(1)),
                    "up" => this.group_hl = this.group_hl.saturating_sub(1),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .overflow_x_hidden()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .mb_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(17.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(c(TEXT))
                            .child(if new {
                                crate::i18n::t("New entry")
                            } else {
                                crate::i18n::t("Edit entry")
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(DIM))
                            .child(crate::i18n::t("Ctrl+Enter save · Esc cancel")),
                    ),
            )
            .child(Self::row(crate::i18n::t("Title"), self.title.clone()))
            .child(Self::row(crate::i18n::t("Group"), self.render_group(cx)))
            .child(Self::row(crate::i18n::t("Username"), self.username.clone()))
            .child(Self::row(
                crate::i18n::t("Password"),
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(self.password.clone()))
                            .child(
                                Self::small_btn("gen", crate::i18n::t("Generate"), false).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.gen_open = !this.gen_open;
                                        if this.gen_open {
                                            this.regenerate();
                                        }
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .children((0..4).map(move |i| {
                                div()
                                    .w(px(22.))
                                    .h(px(4.))
                                    .rounded(px(2.))
                                    .bg(c(if i < bars { bar_col } else { RAISED }))
                            }))
                            .child(div().ml_2().text_size(px(10.5)).text_color(c(DIM)).child(
                                if pw_empty {
                                    String::from(crate::i18n::t("No password"))
                                } else {
                                    tf("~{n} bits", &[("n", &bits.round().to_string())])
                                },
                            )),
                    )
                    .when(self.gen_open, |d| d.child(self.render_generator(cx))),
            ))
            .child(Self::row(crate::i18n::t("Website"), self.url.clone()))
            .child(Self::row(crate::i18n::t("Email"), self.email.clone()))
            .child(Self::row(crate::i18n::t("Expires"), self.render_expiry(cx)))
            .child(Self::row(crate::i18n::t("Notes"), self.notes.clone()))
            // Quiet compatibility hint (no colour, no icon, never blocks).
            .when(beyond_bmp, |d| {
                d.child(
                    div()
                        .mt_1()
                        .text_size(px(11.))
                        .text_color(c(DIM))
                        .child(crate::i18n::t(
                            "Note: emoji and similar characters don't display in Password Gorilla.",
                        )),
                )
            })
            .child(
                div()
                    .mt_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(self.error.clone(), |d, e| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(11.5))
                                .text_color(c(DANGER))
                                .child(
                                    svg()
                                        .path("icons/warning.svg")
                                        .size(px(12.))
                                        .text_color(c(DANGER)),
                                )
                                .child(e),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Self::small_btn("cancel", crate::i18n::t("Cancel"), false)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Cancel))),
                    )
                    .child(
                        Self::small_btn(
                            "save",
                            if new {
                                crate::i18n::t("Add entry")
                            } else {
                                crate::i18n::t("Save")
                            },
                            true,
                        )
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Save))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_roundtrip_keeps_slashes_inside_names() {
        let g = vec!["Work".to_string(), "CI/CD".to_string()];
        assert_eq!(parse_group(&group_display(&g)), g);
        assert_eq!(parse_group("  Home  /  Wi-Fi "), vec!["Home", "Wi-Fi"]);
        assert_eq!(parse_group(""), Vec::<String>::new());
    }
}

//! Entry editor (add / edit) with an inline password generator.

use gpui::{
    div, prelude::*, px, svg, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, SharedString, Window,
};
use keywhere_core::generator::{self, Mode, Options};
use keywhere_core::{rec, Record};
use zeroize::Zeroizing;

use crate::field::{FieldEvent, TextField};
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = |cx: &mut Context<Self>, ph: &str, secret: bool| {
            cx.new(|cx| TextField::new(cx, ph, secret))
        };
        let title = field(cx, "Title (required)", false);
        let group = field(
            cx,
            "Group — levels separated by “ / ”, e.g. Work / Dev",
            false,
        );
        let username = field(cx, "Username", false);
        let password = field(cx, "Password", true);
        let url = field(cx, "https://…", false);
        let email = field(cx, "Email", false);
        let expiry = field(cx, "YYYY-MM-DD (optional)", false);
        let notes = cx.new(|cx| TextField::new(cx, "Notes", false).multiline());

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

        for f in [
            &title, &group, &username, &password, &url, &email, &expiry, &notes,
        ] {
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
        // Make it overlong on purpose so wrapping is exercised.
        let mut long = self.gen_preview.to_string();
        long.push_str("-cause-ranch-woven-humid-blimp-pest-yoga-icing");
        self.gen_preview = Zeroizing::new(long);
        cx.notify();
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
            return Err("Title is required.".into());
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
            Some(model::parse_iso_date(&exp_txt).ok_or("Expiry must be a date like 2027-03-31.")?)
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
            .child(div().flex_1().min_w_0().child(field))
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
            0..=49 => "Weak",
            50..=79 => "Fair",
            80..=109 => "Strong",
            _ => "Excellent",
        };
        let seg = |id: &'static str, text: &'static str, mode: Mode, cx: &mut Context<Self>| {
            let on = self.gen_opts.mode == mode;
            div()
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
            "words"
        } else {
            "characters"
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
                    .child(format!("{label} · {} bits", bits.round()))
                    .child(
                        div()
                            .id("regen")
                            .cursor_pointer()
                            .text_color(c(LINK))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.regenerate();
                                cx.notify();
                            }))
                            .child("Regenerate"),
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
                    .child(seg("m-rand", "Random", Mode::Random, cx))
                    .child(seg("m-pass", "Passphrase", Mode::Passphrase, cx))
                    .child(seg("m-pin", "PIN", Mode::Pin, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.5))
                    .text_color(c(MUTED))
                    .child("Length")
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
                            "No look‑alikes",
                            o.no_lookalikes,
                            |o| o.no_lookalikes = !o.no_lookalikes,
                            cx,
                        )),
                )
            })
            .child(div().flex().justify_end().child(
                Self::small_btn("use", "Use password", true).on_click(cx.listener(
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

        div()
            .id("editor")
            .key_context("Editor")
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
                            .child(if new { "New entry" } else { "Edit entry" }),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(DIM))
                            .child("Ctrl+Enter save · Esc cancel"),
                    ),
            )
            .child(Self::row("Title", self.title.clone()))
            .child(Self::row("Group", self.group.clone()))
            .child(Self::row("Username", self.username.clone()))
            .child(Self::row(
                "Password",
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1().child(self.password.clone()))
                            .child(Self::small_btn("gen", "Generate", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.gen_open = !this.gen_open;
                                    if this.gen_open {
                                        this.regenerate();
                                    }
                                    cx.notify();
                                }),
                            )),
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
                                    String::from("No password")
                                } else {
                                    format!("~{} bits", bits.round())
                                },
                            )),
                    )
                    .when(self.gen_open, |d| d.child(self.render_generator(cx))),
            ))
            .child(Self::row("Website", self.url.clone()))
            .child(Self::row("Email", self.email.clone()))
            .child(Self::row("Expires", self.expiry.clone()))
            .child(Self::row("Notes", self.notes.clone()))
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
                        Self::small_btn("cancel", "Cancel", false)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Cancel))),
                    )
                    .child(
                        Self::small_btn("save", if new { "Add entry" } else { "Save" }, true)
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

//! Settings overlay (opened from the gear button or Ctrl/⌘+,).

use gpui::{
    div, prelude::*, px, svg, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, KeyDownEvent, SharedString, Window,
};

use crate::i18n::{self, t, tf, Lang};
use crate::select::{Select, SelectChanged};
use crate::settings::{self, CloseAction, CLIPBOARD_MAX, CLIPBOARD_MIN, IDLE_CHOICES};
use crate::theme::{ThemeChoice, *};

pub enum SettingsEvent {
    Close,
}

pub struct SettingsView {
    focus: FocusHandle,
    idle: Entity<Select>,
    language: Entity<Select>,
    theme: Entity<Select>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn theme_labels() -> Vec<SharedString> {
    ThemeChoice::ALL
        .iter()
        .map(|t| {
            SharedString::from(crate::i18n::t(match t {
                ThemeChoice::System => "System",
                ThemeChoice::Dark => "Dark",
                ThemeChoice::Light => "Light",
            }))
        })
        .collect()
}

fn theme_index(t: ThemeChoice) -> usize {
    ThemeChoice::ALL.iter().position(|x| *x == t).unwrap_or(0)
}

fn idle_labels() -> Vec<SharedString> {
    IDLE_CHOICES
        .iter()
        .map(|&m| {
            if m == 0 {
                t("Never").into()
            } else {
                tf("{n} min", &[("n", &m.to_string())]).into()
            }
        })
        .collect()
}

fn idle_index(minutes: u32) -> usize {
    IDLE_CHOICES.iter().position(|&m| m == minutes).unwrap_or(2)
}

/// "System (…)" first, then every language by its own name.
fn language_labels() -> Vec<SharedString> {
    std::iter::once(tf("System ({lang})", &[("lang", Lang::system().native_name())]).into())
        .chain(
            Lang::ALL
                .iter()
                .map(|l| SharedString::from(l.native_name())),
        )
        .collect()
}

fn language_index(choice: Option<Lang>) -> usize {
    choice
        .and_then(|l| Lang::ALL.iter().position(|x| *x == l))
        .map_or(0, |i| i + 1)
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let st = settings::get();
        let idle = cx
            .new(|cx| Select::new(cx, idle_labels(), idle_index(st.idle_lock_minutes)).width(200.));
        let language = cx
            .new(|cx| Select::new(cx, language_labels(), language_index(st.language)).width(240.));
        cx.subscribe_in(&idle, window, |this, _, SelectChanged(i), window, cx| {
            let m = IDLE_CHOICES[*i];
            Self::change(|s| s.idle_lock_minutes = m, window);
            this.relabel(cx);
        })
        .detach();
        cx.subscribe_in(
            &language,
            window,
            |this, _, SelectChanged(i), window, cx| {
                let choice = if *i == 0 {
                    None
                } else {
                    Lang::ALL.get(*i - 1).copied()
                };
                Self::change(|s| s.language = choice, window);
                this.relabel(cx);
            },
        )
        .detach();
        let theme = cx.new(|cx| Select::new(cx, theme_labels(), theme_index(st.theme)).width(200.));
        cx.subscribe_in(&theme, window, |this, _, SelectChanged(i), window, cx| {
            let choice = ThemeChoice::ALL[*i];
            Self::change(|s| s.theme = choice, window);
            this.relabel(cx);
        })
        .detach();
        SettingsView {
            focus,
            idle,
            language,
            theme,
        }
    }

    #[cfg(feature = "devtools")]
    pub fn dev_open_language(&mut self, cx: &mut Context<Self>) {
        self.language.update(cx, |s, cx| s.dev_open(cx));
    }

    /// Option labels are translated; refresh them after a language switch.
    fn relabel(&mut self, cx: &mut Context<Self>) {
        let st = settings::get();
        self.idle.update(cx, |s, cx| {
            s.set_options(idle_labels(), idle_index(st.idle_lock_minutes), cx)
        });
        self.language.update(cx, |s, cx| {
            s.set_options(language_labels(), language_index(st.language), cx)
        });
        self.theme.update(cx, |s, cx| {
            s.set_options(theme_labels(), theme_index(st.theme), cx)
        });
    }

    fn change(f: impl FnOnce(&mut settings::Settings), window: &mut Window) {
        settings::update(f);
        i18n::apply_setting(settings::get().language);
        crate::theme::apply(settings::get().theme, window.appearance());
        // Language and other settings affect every view: redraw all.
        window.refresh();
    }

    fn chip(id: SharedString, label: SharedString, on: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px(px(10.))
            .py(px(4.))
            .rounded(px(12.))
            .border_1()
            .cursor_pointer()
            .text_size(px(12.))
            .border_color(c(if on { 0x4E7BE0 } else { BORDER }))
            .bg(if on { ca(FOCUS, 0x2e) } else { ca(BG, 0) })
            .text_color(c(if on { 0xCFDCFF } else { TEXT_SOFT }))
            .hover(|d| d.border_color(c(0x4E7BE0)))
            .child(label)
    }

    fn row(label: &'static str, control: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .items_start()
            .gap_4()
            .py_2()
            .child(
                div()
                    .w(px(170.))
                    .flex_none()
                    .pt(px(4.))
                    .text_size(px(12.5))
                    .text_color(c(TEXT_SOFT))
                    .child(t(label)),
            )
            .child(div().flex_1().min_w_0().child(control))
    }

    fn section(title: &'static str) -> gpui::Div {
        div()
            .mt_3()
            .mb_1()
            .text_size(px(10.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(DIM))
            .child(t(title).to_uppercase())
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = settings::get();

        let step = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .size(px(28.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(c(RAISED))
                .border_1()
                .border_color(c(BORDER))
                .text_color(c(TEXT))
                .hover(|d| d.bg(c(HOVER)))
                .child(label)
        };
        let clip = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("clip-", "−").on_click(|_, window, _| {
                Self::change(
                    |s| {
                        s.clipboard_clear_secs =
                            s.clipboard_clear_secs.saturating_sub(5).max(CLIPBOARD_MIN)
                    },
                    window,
                )
            }))
            .child(
                div()
                    .w(px(56.))
                    .flex()
                    .justify_center()
                    .text_color(c(TEXT))
                    .child(tf("{n} s", &[("n", &s.clipboard_clear_secs.to_string())])),
            )
            .child(step("clip+", "+").on_click(|_, window, _| {
                Self::change(
                    |s| s.clipboard_clear_secs = (s.clipboard_clear_secs + 5).min(CLIPBOARD_MAX),
                    window,
                )
            }));

        let close = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(
                Self::chip(
                    "close-lock".into(),
                    t("Lock and hide").into(),
                    s.close_action == CloseAction::Lock,
                )
                .on_click(|_, window, _| {
                    Self::change(|s| s.close_action = CloseAction::Lock, window)
                }),
            )
            .child(
                Self::chip(
                    "close-hide".into(),
                    t("Hide, keep unlocked").into(),
                    s.close_action == CloseAction::Hide,
                )
                .on_click(|_, window, _| {
                    Self::change(|s| s.close_action = CloseAction::Hide, window)
                }),
            );

        div()
            .id("settings-card")
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    cx.emit(SettingsEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .on_click(|_, _, cx| cx.stop_propagation())
            .w(px(560.))
            .max_w_full()
            .p_5()
            .rounded(px(12.))
            .bg(c(PANEL))
            .border_1()
            .border_color(c(0x2A4A85))
            .shadow_lg()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        svg()
                            .path("icons/gear.svg")
                            .size(px(16.))
                            .text_color(c(MUTED)),
                    )
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(c(TEXT))
                            .child(t("Settings")),
                    ),
            )
            .child(Self::section("Security"))
            .child(Self::row("Auto‑lock after", self.idle.clone()))
            .child(Self::row("Clear clipboard after", clip))
            .child(Self::row("Closing the window", close))
            .child(Self::section("Appearance"))
            .child(Self::row("Theme", self.theme.clone()))
            .child(Self::row("Language", self.language.clone()))
            .child(
                div()
                    .mt_4()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(c(DIM))
                            .child(t(
                                "Settings are saved immediately and never contain secrets.",
                            )),
                    )
                    .child(
                        div()
                            .id("settings-done")
                            .h(px(32.))
                            .px_4()
                            .flex()
                            .items_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .bg(c(ACCENT))
                            .text_color(c(ACCENT_INK))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12.5))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)))
                            .child(t("Done")),
                    ),
            )
    }
}

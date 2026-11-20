//! Settings overlay (opened from the gear button or Ctrl/⌘+,).

use gpui::{
    div, prelude::*, px, svg, App, Context, EventEmitter, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, SharedString, Window,
};

use crate::i18n::{self, t, tf, Lang};
use crate::settings::{self, CloseAction, CLIPBOARD_MAX, CLIPBOARD_MIN, IDLE_CHOICES};
use crate::theme::*;

pub enum SettingsEvent {
    Close,
}

pub struct SettingsView {
    focus: FocusHandle,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        SettingsView { focus }
    }

    fn change(f: impl FnOnce(&mut settings::Settings), window: &mut Window) {
        settings::update(f);
        i18n::apply_setting(settings::get().language);
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

        let idle = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(IDLE_CHOICES.iter().map(|&m| {
                let label: SharedString = if m == 0 {
                    t("Never").into()
                } else {
                    tf("{n} min", &[("n", &m.to_string())]).into()
                };
                Self::chip(
                    SharedString::from(format!("idle-{m}")),
                    label,
                    s.idle_lock_minutes == m,
                )
                .on_click(move |_, window, _| Self::change(|s| s.idle_lock_minutes = m, window))
            }));

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
                    "close-hide".into(),
                    t("Hide, keep unlocked").into(),
                    s.close_action == CloseAction::Hide,
                )
                .on_click(|_, window, _| {
                    Self::change(|s| s.close_action = CloseAction::Hide, window)
                }),
            )
            .child(
                Self::chip(
                    "close-lock".into(),
                    t("Lock and hide").into(),
                    s.close_action == CloseAction::Lock,
                )
                .on_click(|_, window, _| {
                    Self::change(|s| s.close_action = CloseAction::Lock, window)
                }),
            );

        let sys = Lang::system();
        let lang = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(
                Self::chip(
                    "lang-system".into(),
                    tf("System ({lang})", &[("lang", sys.native_name())]).into(),
                    s.language.is_none(),
                )
                .on_click(|_, window, _| Self::change(|s| s.language = None, window)),
            )
            .children(Lang::ALL.into_iter().map(|l| {
                Self::chip(
                    SharedString::from(format!("lang-{}", l.code())),
                    l.native_name().into(),
                    s.language == Some(l),
                )
                .on_click(move |_, window, _| Self::change(|s| s.language = Some(l), window))
            }));

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
            .child(Self::row("Auto‑lock after", idle))
            .child(Self::row("Clear clipboard after", clip))
            .child(Self::row("Closing the window", close))
            .child(Self::section("Language"))
            .child(Self::row("Language", lang))
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

//! Unlock screen. Shows nothing about the vault except its path.

use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, svg, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    PathPromptOptions, SharedString, Window,
};
use keywhere_core::{Error, Vault};
use zeroize::Zeroizing;

use crate::field::{FieldEvent, TextField};
use crate::{chrome, config, theme::*};

pub enum UnlockEvent {
    Unlocked,
}

pub struct UnlockView {
    pub path: Option<PathBuf>,
    password: Entity<TextField>,
    error: Option<SharedString>,
    busy: bool,
    failures: u32,
    /// Filled on success; taken by the shell.
    pub result: Option<Vault>,
}

impl EventEmitter<UnlockEvent> for UnlockView {}

impl Focusable for UnlockView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.password.focus_handle(cx)
    }
}

impl UnlockView {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            TextField::new(cx, "Master password", true)
                .with_icon("icons/lock.svg")
                .large()
        });
        cx.subscribe_in(&password, window, |this, _, ev, window, cx| match ev {
            FieldEvent::Submit => this.unlock(window, cx),
            FieldEvent::Changed => {
                if this.error.is_some() {
                    this.error = None;
                    cx.notify();
                }
            }
            FieldEvent::Escape => {}
        })
        .detach();
        window.focus(&password.focus_handle(cx));
        Self {
            path,
            password,
            error: None,
            busy: false,
            failures: 0,
            result: None,
        }
    }

    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.choose(window, cx);
            return;
        };
        let pw = Zeroizing::new(self.password.read(cx).text().as_bytes().to_vec());
        if pw.is_empty() {
            return;
        }
        self.password.update(cx, |f, cx| f.clear(cx));
        self.busy = true;
        self.error = None;
        cx.notify();
        // Key stretching (hundreds of thousands of SHA-256 rounds) runs off the UI thread.
        let task = cx
            .background_executor()
            .spawn(async move { Vault::open(&path, &pw) });
        let failures = self.failures;
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            if matches!(res, Err(Error::WrongPassword)) {
                // Small, capped backoff against rapid guessing at the UI.
                let secs = (failures + 1).min(5) as u64;
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(secs))
                    .await;
            }
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match res {
                    Ok(v) => {
                        this.failures = 0;
                        this.result = Some(v);
                        cx.emit(UnlockEvent::Unlocked);
                    }
                    Err(e) => {
                        if matches!(e, Error::WrongPassword) {
                            this.failures += 1;
                        }
                        this.error = Some(
                            match e {
                                Error::WrongPassword => "Wrong master password.".to_string(),
                                other => other.to_string(),
                            }
                            .into(),
                        );
                        window.focus(&this.password.focus_handle(cx));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open vault".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    this.update_in(cx, |this, window, cx| {
                        config::remember_vault(&p);
                        this.path = Some(p);
                        this.error = None;
                        window.focus(&this.password.focus_handle(cx));
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }
}

fn link(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .text_color(c(DIM))
        .hover(|d| d.text_color(c(TEXT_SOFT)))
        .child(label)
}

impl Render for UnlockView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path_label: SharedString = match &self.path {
            Some(p) => config::display_path(p).into(),
            None => "No vault selected".into(),
        };
        let caps = window.capslock().on;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(c(BG))
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pr(px(6.))
                    .child(chrome::drag_area("unlock-drag").h_full().flex_1())
                    .child(chrome::window_controls(window)),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pb(px(40.))
                    .child(
                        div()
                            .w(px(340.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(crate::assets::logo(76.))
                            .child(
                                div()
                                    .mt_3()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(c(TEXT))
                                    .child("Unlock vault"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_size(px(12.))
                                    .text_color(c(MUTED))
                                    .child(
                                        svg()
                                            .path("icons/file.svg")
                                            .size(px(12.))
                                            .text_color(c(DIM)),
                                    )
                                    .child(path_label),
                            )
                            .child(div().mt_5().w_full().child(self.password.clone()))
                            .child(
                                div()
                                    .h(px(22.))
                                    .mt_1()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_size(px(11.5))
                                    .when_some(self.error.clone(), |d, e| {
                                        d.text_color(c(DANGER))
                                            .child(
                                                svg()
                                                    .path("icons/warning.svg")
                                                    .size(px(12.))
                                                    .text_color(c(DANGER)),
                                            )
                                            .child(e)
                                    })
                                    .when(self.error.is_none() && caps, |d| {
                                        d.text_color(c(WARN))
                                            .child(
                                                svg()
                                                    .path("icons/warning.svg")
                                                    .size(px(12.))
                                                    .text_color(c(WARN)),
                                            )
                                            .child("Caps Lock is on")
                                    }),
                            )
                            .child(
                                div()
                                    .id("unlock")
                                    .w_full()
                                    .h(px(38.))
                                    .rounded(px(9.))
                                    .bg(c(if self.busy { RAISED } else { ACCENT }))
                                    .text_color(c(if self.busy { MUTED } else { ACCENT_INK }))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|d| d.opacity(0.92))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.unlock(window, cx)),
                                    )
                                    .child(if self.busy {
                                        "Unlocking…"
                                    } else if self.path.is_none() {
                                        "Choose vault…"
                                    } else {
                                        "Unlock"
                                    }),
                            )
                            .child(div().mt_4().flex().gap_4().text_size(px(11.5)).child(
                                link("choose", "Open another vault…").on_click(
                                    cx.listener(|this, _, window, cx| this.choose(window, cx)),
                                ),
                            )),
                    ),
            )
    }
}

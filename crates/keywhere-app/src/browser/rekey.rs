//! "Change master password" dialog of the unlocked vault.
//!
//! The current password is checked and the new key derived off the UI
//! thread ([`keywhere_core::PasswordChange`]); the vault then takes the new
//! key and is saved the usual safe way. If that save fails, the old key is
//! put back, so the file and the session never disagree about the password.

use gpui::{
    div, prelude::*, px, svg, Context, Entity, Focusable, FontWeight, SharedString, Window,
};
use keywhere_core::{Error, Format};
use zeroize::Zeroizing;

use super::Browser;
use crate::field::{FieldEvent, TextField};
use crate::i18n::{t, tf};
use crate::theme::*;
use crate::unlock::{strength_hint, MIN_NEW_PASSWORD_CHARS};

pub(super) struct Rekey {
    current: Entity<TextField>,
    new: Entity<TextField>,
    confirm: Entity<TextField>,
    error: Option<SharedString>,
    busy: bool,
}

#[derive(Clone, Copy)]
enum Which {
    Current,
    New,
    Confirm,
}

impl Browser {
    /// Can the master password be changed now? (v3, writable, no form open.)
    pub fn can_change_password(&self) -> bool {
        !self.read_only() && self.vault().format() == Format::V3
    }

    pub fn open_change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_none()
            || !self.can_change_password()
            || self.editor.is_some()
            || self.rekey.is_some()
            || self.upgrade.is_some()
            || self.confirm_delete.is_some()
            || self.new_group.is_some()
        {
            return;
        }
        let field = |placeholder: &'static str, cx: &mut Context<Self>| {
            cx.new(|cx| TextField::new(cx, placeholder, true).with_icon("icons/lock.svg"))
        };
        let current = field("Current master password", cx);
        let new = field("New master password", cx);
        let confirm = field("Repeat new master password", cx);
        for (f, which) in [
            (&current, Which::Current),
            (&new, Which::New),
            (&confirm, Which::Confirm),
        ] {
            cx.subscribe_in(f, window, move |this, f, ev, window, cx| match ev {
                FieldEvent::Submit => match which {
                    Which::Current => this.rekey_focus(Which::New, window, cx),
                    Which::New => this.rekey_focus(Which::Confirm, window, cx),
                    Which::Confirm => this.change_password(window, cx),
                },
                FieldEvent::Escape => this.close_change_password(window, cx),
                FieldEvent::Changed => {
                    // Only typing hides the error (the app clearing a field
                    // must not); also redraws the strength hint.
                    if let Some(r) = &mut this.rekey {
                        if !f.read(cx).text().is_empty() {
                            r.error = None;
                        }
                    }
                    cx.notify();
                }
            })
            .detach();
        }
        self.group_menu = None;
        window.focus(&current.focus_handle(cx));
        self.rekey = Some(Rekey {
            current,
            new,
            confirm,
            error: None,
            busy: false,
        });
        cx.notify();
    }

    fn close_change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rekey.as_ref().is_some_and(|r| r.busy) {
            return;
        }
        // Dropping the fields zeroizes what was typed.
        self.rekey = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn rekey_focus(&mut self, which: Which, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = &self.rekey {
            let f = match which {
                Which::Current => &r.current,
                Which::New => &r.new,
                Which::Confirm => &r.confirm,
            };
            window.focus(&f.focus_handle(cx));
        }
    }

    fn rekey_fail(
        &mut self,
        msg: impl Into<SharedString>,
        focus: Which,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(r) = &mut self.rekey {
            r.error = Some(msg.into());
        }
        self.rekey_focus(focus, window, cx);
        cx.notify();
    }

    fn change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = &self.rekey else {
            return;
        };
        if r.busy {
            return;
        }
        let (current, new, confirm) = (r.current.clone(), r.new.clone(), r.confirm.clone());
        let cur = Zeroizing::new(current.read(cx).text().as_bytes().to_vec());
        let pw = Zeroizing::new(new.read(cx).text().to_string());
        if cur.is_empty() {
            return self.rekey_focus(Which::Current, window, cx);
        }
        if pw.chars().count() < MIN_NEW_PASSWORD_CHARS {
            let n = MIN_NEW_PASSWORD_CHARS.to_string();
            let msg = tf("Use at least {n} characters.", &[("n", &n)]);
            return self.rekey_fail(msg, Which::New, window, cx);
        }
        if confirm.read(cx).text() != pw.as_str() {
            confirm.update(cx, |f, cx| f.clear(cx));
            let msg = t("The passwords don't match.");
            return self.rekey_fail(msg, Which::Confirm, window, cx);
        }
        if pw.as_bytes() == cur.as_slice() {
            let msg = t("The new password is the same as the current one.");
            return self.rekey_fail(msg, Which::New, window, cx);
        }
        let Some(job) = self.vault().password_change() else {
            return;
        };
        let pw = Zeroizing::new(pw.as_bytes().to_vec());
        for f in [&current, &new, &confirm] {
            f.update(cx, |f, cx| f.clear(cx));
        }
        if let Some(r) = &mut self.rekey {
            r.busy = true;
            r.error = None;
        }
        cx.notify();
        // Two key stretches: off the UI thread. The job holds no key.
        let task = cx
            .background_executor()
            .spawn(async move { job.run(&cur, &pw) });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            this.update_in(cx, |this, window, cx| {
                // Dialog gone (session handed to the tray) or vault taken:
                // drop the result, the session keeps its password.
                if this.vault.is_none() {
                    return;
                }
                let Some(r) = &mut this.rekey else {
                    return;
                };
                r.busy = false;
                match res {
                    Ok(key) => this.save_new_key(key, window, cx),
                    Err(Error::WrongPassword) => {
                        let msg = t("The current master password is wrong.");
                        this.rekey_fail(msg, Which::Current, window, cx)
                    }
                    Err(e) => {
                        let msg = tf(
                            "Couldn't change the password: {e}",
                            &[("e", &e.to_string())],
                        );
                        this.rekey_fail(msg, Which::Current, window, cx)
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Switch to the new key and save; on failure go back to the old key.
    fn save_new_key(
        &mut self,
        key: keywhere_core::NewKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old = self.vault_mut().replace_key(key);
        let path = self.path.clone();
        match self.vault_mut().save(&path) {
            Ok(()) => {
                drop(old); // zeroized
                self.saved(cx);
                crate::memlock::record(self.vault().key_in_locked_memory());
                self.rekey = None;
                let bak = path
                    .with_extension("ibak")
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.toast(tf(
                    "Master password changed. The backup {name} still opens with the old one.",
                    &[("name", &bak)],
                ));
                window.focus(&self.focus);
                cx.notify();
            }
            Err(e) => {
                drop(self.vault_mut().replace_key(old));
                let msg = match e {
                    Error::Conflict => t("The vault file changed on disk. Lock, unlock it again and retry. The master password is unchanged.").to_string(),
                    e => tf(
                        "Couldn't save: {e}. The master password is unchanged.",
                        &[("e", &e.to_string())],
                    ),
                };
                self.rekey_fail(msg, Which::Current, window, cx);
            }
        }
    }

    pub(super) fn render_rekey(
        &self,
        corner: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let r = self.rekey.as_ref()?;
        let btn = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(32.))
                .px_4()
                .flex()
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(primary, |d| {
                    d.bg(c(if r.busy { RAISED } else { ACCENT }))
                        .text_color(c(if r.busy { MUTED } else { ACCENT_INK }))
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!primary, |d| {
                    d.bg(c(RAISED))
                        .border_1()
                        .border_color(c(BORDER))
                        .text_color(c(TEXT))
                })
                .child(label)
        };
        let hint: Option<(SharedString, u32)> = match &r.error {
            Some(e) => Some((e.clone(), DANGER)),
            None => strength_hint(r.new.read(cx).text())
                .map(|(s, weak)| (s.into(), if weak { WARN } else { MUTED })),
        };
        let bak = self
            .path
            .with_extension("ibak")
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(
            div()
                .id("rekey-backdrop")
                .absolute()
                .inset_0()
                .rounded(corner)
                .flex()
                .items_center()
                .justify_center()
                .bg(ca(0x050B1A, 0xB0))
                .occlude()
                .child(
                    div()
                        .id("rekey-card")
                        .w(px(440.))
                        .p_5()
                        .rounded(px(12.))
                        .bg(c(PANEL))
                        .border_1()
                        .border_color(c(0x2A4A85))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap_3()
                        // Tab / Shift+Tab: next / previous field.
                        .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                            let m = &ev.keystroke.modifiers;
                            if ev.keystroke.key != "tab" || m.secondary() || m.alt {
                                return;
                            }
                            let Some(r) = &this.rekey else { return };
                            let order = [&r.current, &r.new, &r.confirm];
                            let at = order
                                .iter()
                                .position(|f| f.focus_handle(cx).is_focused(window))
                                .unwrap_or(0);
                            let next = (at + if m.shift { 2 } else { 1 }) % 3;
                            window.focus(&order[next].focus_handle(cx));
                            cx.stop_propagation();
                        }))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(c(TEXT))
                                .child(t("Change master password")),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(c(TEXT_SOFT))
                                .line_height(px(19.))
                                .child(SharedString::from(tf(
                                    "The vault is saved with the new password straight away. Its backup, {name}, keeps the previous version, which still opens with the old password: delete it if the old password may be known to someone else.",
                                    &[("name", &bak)],
                                ))),
                        )
                        .child(r.current.clone())
                        .child(div().mt_1().child(r.new.clone()))
                        .child(r.confirm.clone())
                        .child(
                            div()
                                .h(px(18.))
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(11.5))
                                .when_some(hint, |d, (text, color)| {
                                    d.text_color(c(color))
                                        .when(color != MUTED, |d| {
                                            d.child(
                                                svg()
                                                    .path("icons/warning.svg")
                                                    .size(px(12.))
                                                    .text_color(c(color)),
                                            )
                                        })
                                        .child(text)
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .text_size(px(11.5))
                                .text_color(c(TEXT_SOFT))
                                .child(
                                    svg()
                                        .flex_none()
                                        .mt(px(2.))
                                        .path("icons/shield.svg")
                                        .size(px(12.))
                                        .text_color(c(WARN)),
                                )
                                .child(div().flex_1().child(t(
                                    "If you forget the master password, the vault can't be opened. Nobody can recover it, not even Keywhere. Write it down and keep it somewhere safe.",
                                ))),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(btn("rekey-cancel", t("Cancel"), false).when(r.busy, |d| d.opacity(0.5)).on_click(
                                    cx.listener(|this, _, w, cx| this.close_change_password(w, cx)),
                                ))
                                .child(
                                    btn(
                                        "rekey-go",
                                        if r.busy {
                                            t("Changing…")
                                        } else {
                                            t("Change password")
                                        },
                                        true,
                                    )
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.change_password(w, cx)
                                    })),
                                ),
                        ),
                ),
        )
    }
}

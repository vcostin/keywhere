//! Unlock screen. Shows nothing about the vault except its path. Also hosts
//! "Create new vault…": a save dialog, then a master password form; the new
//! vault opens straight away.

use std::path::{Path, PathBuf};

use gpui::{
    div, prelude::*, px, svg, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Window,
};
use keywhere_core::{Error, Vault};
use zeroize::Zeroizing;

use crate::field::{FieldEvent, TextField};
use crate::{chrome, config, theme::*};

pub enum UnlockEvent {
    Unlocked,
}

/// A new master password needs at least this many characters.
pub const MIN_NEW_PASSWORD_CHARS: usize = 8;

/// File name the save dialog suggests for a new vault.
const NEW_VAULT_NAME: &str = "Keywhere.psafe3";

pub struct UnlockView {
    pub path: Option<PathBuf>,
    password: Entity<TextField>,
    /// Second entry of a new master password (create mode only).
    confirm: Entity<TextField>,
    /// Create mode: the file chosen for the new vault (not written yet).
    creating: Option<PathBuf>,
    error: Option<SharedString>,
    busy: bool,
    failures: u32,
    /// Filled on success; taken by the shell.
    pub result: Option<Vault>,
    /// A vault opened from the file manager while a new one was being
    /// written: shown next if creating fails, else taken by the shell.
    pub queued_open: Option<PathBuf>,
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
        let confirm = cx.new(|cx| {
            TextField::new(cx, "Repeat master password", true)
                .with_icon("icons/lock.svg")
                .large()
        });
        cx.subscribe_in(&password, window, |this, _, ev, window, cx| match ev {
            FieldEvent::Submit if this.creating.is_some() => {
                window.focus(&this.confirm.focus_handle(cx))
            }
            FieldEvent::Submit => this.unlock(window, cx),
            FieldEvent::Changed => {
                // Typing hides the error; a field emptied by the code
                // (clear) keeps it. Also redraws the strength hint.
                if !this.password.read(cx).text().is_empty() {
                    this.error = None;
                }
                cx.notify();
            }
            FieldEvent::Escape => this.cancel_create(window, cx),
        })
        .detach();
        cx.subscribe_in(&confirm, window, |this, _, ev, window, cx| match ev {
            FieldEvent::Submit => this.create(window, cx),
            FieldEvent::Changed => {
                if this.error.is_some() && !this.confirm.read(cx).text().is_empty() {
                    this.error = None;
                    cx.notify();
                }
            }
            FieldEvent::Escape => this.cancel_create(window, cx),
        })
        .detach();
        window.focus(&password.focus_handle(cx));
        Self {
            path,
            password,
            confirm,
            creating: None,
            error: None,
            busy: false,
            failures: 0,
            result: None,
            queued_open: None,
        }
    }

    #[cfg(feature = "devtools")]
    pub fn dev_type_and_unlock(&mut self, pw: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.password.update(cx, |f, cx| f.set_text(pw, cx));
        self.unlock(window, cx);
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
                        crate::browser::clear_rescue_note();
                        cx.emit(UnlockEvent::Unlocked);
                    }
                    Err(e) => {
                        if matches!(e, Error::WrongPassword) {
                            this.failures += 1;
                        }
                        this.error = Some(
                            match e {
                                Error::WrongPassword => {
                                    crate::i18n::t("Wrong master password.").to_string()
                                }
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
        let title = crate::i18n::t("Open vault");
        let accept = crate::i18n::t("Open");
        // Linux: a helper process asks the portal (see sys/picker.rs).
        #[cfg(target_os = "linux")]
        let picked = cx
            .background_executor()
            .spawn(async move { crate::sys::picker::pick_vault(title, accept) });
        #[cfg(not(target_os = "linux"))]
        let picked = {
            let _ = accept;
            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some(title.into()),
            });
            async move {
                match rx.await {
                    Ok(Ok(paths)) => Ok(paths.and_then(|mut p| p.pop())),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("the file dialog closed unexpectedly".to_string()),
                }
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let picked = picked.await;
            let _ = this.update_in(cx, |this, window, cx| match picked {
                Ok(Some(p)) => this.set_path(p, window, cx),
                Ok(None) => {} // cancelled
                Err(e) => {
                    // Never fail silently: say why no dialog appeared.
                    eprintln!("keywhere: file dialog failed: {e}");
                    this.error = Some(
                        crate::i18n::tf("Couldn't open the file dialog: {e}", &[("e", &e)]).into(),
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Point the unlock screen at another vault (file dialog, or a vault
    /// opened from the file manager while Keywhere is running).
    pub fn set_path(&mut self, p: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy && self.creating.is_some() {
            // A new vault is being written; decide once that is done.
            self.queued_open = Some(p);
            return;
        }
        self.leave_create(cx);
        config::remember_vault(&p);
        self.path = Some(p);
        self.error = None;
        window.focus(&self.password.focus_handle(cx));
        cx.notify();
    }

    /// "Create new vault…": ask where to save it, then show the form.
    fn choose_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let title = crate::i18n::t("Create new vault");
        let accept = crate::i18n::t("Create");
        // Suggest the folder of the current vault, else the home folder.
        let folder = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(home_dir);
        #[cfg(target_os = "linux")]
        let picked = cx.background_executor().spawn(async move {
            crate::sys::picker::pick_new_vault(title, accept, NEW_VAULT_NAME, folder.as_deref())
        });
        #[cfg(not(target_os = "linux"))]
        let picked = {
            let _ = (title, accept);
            let rx = cx.prompt_for_new_path(&folder.unwrap_or_default(), Some(NEW_VAULT_NAME));
            async move {
                match rx.await {
                    Ok(Ok(p)) => Ok(p),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("the file dialog closed unexpectedly".to_string()),
                }
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let picked = picked.await;
            let _ = this.update_in(cx, |this, window, cx| match picked {
                Ok(Some(p)) => this.start_create(with_vault_extension(p), window, cx),
                Ok(None) => {} // cancelled
                Err(e) => {
                    eprintln!("keywhere: file dialog failed: {e}");
                    this.error = Some(
                        crate::i18n::tf("Couldn't open the file dialog: {e}", &[("e", &e)]).into(),
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Show the new-vault form for `path`, unless something is already there.
    fn start_create(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        // The dialog may have asked "Replace?" and been told yes; a vault
        // never replaces a file. (save_new checks again, atomically.)
        if std::fs::symlink_metadata(&path).is_ok() {
            self.error = Some(already_exists(&path));
            cx.notify();
            return;
        }
        self.password.update(cx, |f, cx| f.clear(cx));
        self.confirm.update(cx, |f, cx| f.clear(cx));
        self.creating = Some(path);
        self.error = None;
        window.focus(&self.password.focus_handle(cx));
        cx.notify();
    }

    /// Back from the new-vault form to unlocking the current vault.
    fn cancel_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.creating.is_none() {
            return;
        }
        self.leave_create(cx);
        self.error = None;
        window.focus(&self.password.focus_handle(cx));
        cx.notify();
    }

    fn leave_create(&mut self, cx: &mut Context<Self>) {
        if self.creating.take().is_some() {
            self.password.update(cx, |f, cx| f.clear(cx));
            self.confirm.update(cx, |f, cx| f.clear(cx));
        }
    }

    /// Check the two entries, then create and write the vault off the UI
    /// thread (key stretching) and open it.
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(path) = self.creating.clone() else {
            return;
        };
        let pw = Zeroizing::new(self.password.read(cx).text().to_string());
        if pw.chars().count() < MIN_NEW_PASSWORD_CHARS {
            self.error = Some(
                crate::i18n::tf(
                    "Use at least {n} characters.",
                    &[("n", &MIN_NEW_PASSWORD_CHARS.to_string())],
                )
                .into(),
            );
            window.focus(&self.password.focus_handle(cx));
            cx.notify();
            return;
        }
        if self.confirm.read(cx).text() != pw.as_str() {
            self.confirm.update(cx, |f, cx| f.clear(cx));
            self.error = Some(crate::i18n::t("The passwords don't match.").into());
            window.focus(&self.confirm.focus_handle(cx));
            cx.notify();
            return;
        }
        let pw = Zeroizing::new(pw.as_bytes().to_vec());
        self.password.update(cx, |f, cx| f.clear(cx));
        self.confirm.update(cx, |f, cx| f.clear(cx));
        self.busy = true;
        self.error = None;
        cx.notify();
        let target = path.clone();
        let task = cx.background_executor().spawn(async move {
            let mut v = Vault::create(&pw)?;
            v.save_new(&target)?;
            Ok::<_, Error>(v)
        });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match res {
                    Ok(v) => {
                        this.creating = None;
                        this.path = Some(path);
                        this.failures = 0;
                        this.result = Some(v);
                        crate::browser::clear_rescue_note();
                        cx.emit(UnlockEvent::Unlocked);
                    }
                    Err(e) => {
                        eprintln!("keywhere: creating {} failed: {e}", path.display());
                        this.error = Some(match &e {
                            Error::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists => {
                                already_exists(&path)
                            }
                            _ => crate::i18n::tf(
                                "Couldn't create the vault: {e}",
                                &[("e", &e.to_string())],
                            )
                            .into(),
                        });
                        window.focus(&this.password.focus_handle(cx));
                        if let Some(p) = this.queued_open.take() {
                            let err = this.error.take();
                            this.set_path(p, window, cx);
                            // Still say why the new vault isn't there.
                            this.error = err;
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn on_key(&mut self, ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        match ks.key.as_str() {
            // Ctrl+N (⌘N): new vault, as "new database" in Password Safe.
            "n" if m.secondary() && !m.alt && !m.shift && self.creating.is_none() => {
                self.choose_new(window, cx);
                cx.stop_propagation();
            }
            // Tab / Shift+Tab between the two password fields.
            "tab" if self.creating.is_some() && !m.secondary() && !m.alt => {
                let to = if self.password.focus_handle(cx).is_focused(window) {
                    &self.confirm
                } else {
                    &self.password
                };
                window.focus(&to.focus_handle(cx));
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// A name typed without an extension gets `.psafe3`, so file managers know
/// the file (and open it with Keywhere).
fn with_vault_extension(p: PathBuf) -> PathBuf {
    if p.extension().is_some_and(|e| !e.is_empty()) {
        return p;
    }
    let mut s = p.into_os_string();
    if !s.to_string_lossy().ends_with('.') {
        s.push(".");
    }
    s.push("psafe3");
    PathBuf::from(s)
}

fn already_exists(path: &Path) -> SharedString {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    crate::i18n::tf(
        "{name} already exists. Keywhere never replaces a file: choose another name.",
        &[("name", &name)],
    )
    .into()
}

/// Rough strength label for a new master password (UI hint only).
pub(crate) fn strength_hint(pw: &str) -> Option<(String, bool)> {
    if pw.is_empty() {
        return None;
    }
    if pw.chars().count() < MIN_NEW_PASSWORD_CHARS {
        let n = MIN_NEW_PASSWORD_CHARS.to_string();
        return Some((
            crate::i18n::tf("At least {n} characters", &[("n", &n)]),
            true,
        ));
    }
    let bits = crate::model::strength_bits(pw);
    let label = match bits as u32 {
        0..=49 => crate::i18n::t("Weak"),
        50..=79 => crate::i18n::t("Fair"),
        80..=109 => crate::i18n::t("Strong"),
        _ => crate::i18n::t("Excellent"),
    };
    let n = (bits as u32).to_string();
    Some((
        crate::i18n::tf("{label} · {n} bits", &[("label", label), ("n", &n)]),
        bits < 50.0,
    ))
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
        let creating = self.creating.is_some();
        let path_label: SharedString = match (&self.creating, &self.path) {
            (Some(p), _) | (None, Some(p)) => config::display_path(p).into(),
            (None, None) => crate::i18n::t("No vault selected").into(),
        };
        let caps = window.capslock().on;
        // Below the field: an error, else Caps Lock, else (creating) strength.
        let hint: Option<(SharedString, u32)> = if let Some(e) = self.error.clone() {
            Some((e, DANGER))
        } else if caps {
            Some((crate::i18n::t("Caps Lock is on").into(), WARN))
        } else if creating {
            strength_hint(self.password.read(cx).text())
                .map(|(s, weak)| (s.into(), if weak { WARN } else { MUTED }))
        } else {
            None
        };
        let button_label = match (self.busy, creating) {
            (true, true) => crate::i18n::t("Creating…"),
            (true, false) => crate::i18n::t("Unlocking…"),
            (false, true) => crate::i18n::t("Create vault"),
            (false, false) if self.path.is_none() => crate::i18n::t("Choose vault…"),
            (false, false) => crate::i18n::t("Unlock"),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .rounded(chrome::corner_radius(window))
            .bg(c(BG))
            .on_key_down(cx.listener(Self::on_key))
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pr(px(6.))
                    .child(chrome::drag_area("unlock-drag").h_full().flex_1())
                    .child(chrome::settings_button("unlock-settings").mr_1())
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
                            .child(crate::assets::logo(88.))
                            .child(
                                div()
                                    .mt_3()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(c(TEXT))
                                    .child(if creating {
                                        crate::i18n::t("Create new vault")
                                    } else {
                                        crate::i18n::t("Unlock vault")
                                    }),
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
                            .when(creating, |d| {
                                d.child(div().mt_2().w_full().child(self.confirm.clone()))
                            })
                            .child(
                                div()
                                    .h(px(22.))
                                    .mt_1()
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
                            .when(creating, |d| {
                                d.child(
                                    div()
                                        .w_full()
                                        .mb_3()
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
                                        .child(div().flex_1().child(crate::i18n::t(
                                            "If you forget the master password, the vault can't be opened. Nobody can recover it, not even Keywhere. Write it down and keep it somewhere safe.",
                                        ))),
                                )
                            })
                            .when(!creating, |d| {
                                d.when_some(crate::browser::rescue_note(), |d, note| {
                                    d.child(
                                        div()
                                            .mt_1()
                                            .text_size(px(11.5))
                                            .text_color(c(WARN))
                                            .child(note),
                                    )
                                })
                            })
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
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.creating.is_some() {
                                            this.create(window, cx)
                                        } else {
                                            this.unlock(window, cx)
                                        }
                                    }))
                                    .child(button_label),
                            )
                            .child(
                                div()
                                    .mt_4()
                                    .flex()
                                    .gap_4()
                                    .text_size(px(11.5))
                                    .when(creating, |d| {
                                        d.child(link("cancel-create", crate::i18n::t("Cancel")).on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.cancel_create(window, cx)
                                            }),
                                        ))
                                    })
                                    .when(!creating, |d| {
                                        d.child(
                                            link("choose", crate::i18n::t("Open another vault…"))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.choose(window, cx)
                                                })),
                                        )
                                        .child(
                                            link("create", crate::i18n::t("Create new vault…"))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.choose_new(window, cx)
                                                })),
                                        )
                                    }),
                            ),
                    ),
            )
            // Build watermark.
            .child(
                div()
                    .flex_none()
                    .pb_3()
                    .flex()
                    .justify_center()
                    .text_size(px(10.5))
                    .text_color(c(DIM))
                    .child(concat!(
                        "Keywhere ",
                        env!("CARGO_PKG_VERSION"),
                        " · ",
                        env!("KEYWHERE_COMMIT"),
                        " · built ",
                        env!("KEYWHERE_BUILD_DATE")
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_vault_gets_psafe3_extension_only_when_it_has_none() {
        let ext = |s: &str| with_vault_extension(PathBuf::from(s));
        assert_eq!(ext("/v/Home"), PathBuf::from("/v/Home.psafe3"));
        assert_eq!(ext("/v/Home."), PathBuf::from("/v/Home.psafe3"));
        assert_eq!(ext("/v/Home.psafe3"), PathBuf::from("/v/Home.psafe3"));
        assert_eq!(ext("/v/Home.dat"), PathBuf::from("/v/Home.dat"));
        assert_eq!(ext("/v/my.vault"), PathBuf::from("/v/my.vault"));
    }

    #[test]
    fn short_new_passwords_are_flagged() {
        let _g = crate::i18n::TEST_LANG_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_current(crate::i18n::Lang::En);
        assert_eq!(strength_hint(""), None);
        assert_eq!(
            strength_hint("short"),
            Some(("At least 8 characters".into(), true))
        );
        assert!(
            strength_hint("abcdefgh").unwrap().1,
            "8 lowercase letters is weak"
        );
        let (label, weak) = strength_hint("correct horse battery staple").unwrap();
        assert!(!weak && label.starts_with("Excellent"), "{label}");
    }
}

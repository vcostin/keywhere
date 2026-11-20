//! Unlocked vault: sidebar (smart views + groups) · entry list · detail card.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    actions, div, prelude::*, px, svg, uniform_list, App, ClipboardItem, Context, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, ScrollStrategy, SharedString, Task,
    UniformListScrollHandle, Window,
};
use keywhere_core::lock::{LockState, VaultLock};
use keywhere_core::{now_secs, rec, Error, Format, Record, Vault};
use sha2::{Digest, Sha256};

use crate::editor::{EditorEvent, EntryEditor};
use crate::field::{FieldEvent, TextField};
use crate::i18n::tf;
use crate::model::{self, EntryVm, Filter, GroupNode};
use crate::{chrome, config, theme::*};

actions!(
    keywhere,
    [
        Lock,
        FocusSearch,
        SelectNext,
        SelectPrev,
        CopyPassword,
        CopyUsername,
        OpenUrl,
        ToggleReveal,
        NewEntry,
        EditEntry,
        DuplicateEntry,
        DeleteEntry
    ]
);

pub const REVEAL_FOR: Duration = Duration::from_secs(15);

pub enum BrowserEvent {
    LockRequested,
    /// The session moved to another file (v2 → v3 upgrade).
    PathChanged(PathBuf),
}

pub struct Browser {
    pub path: PathBuf,
    /// `None` only after `take_session` (the window is about to close).
    vault: Option<Vault>,
    entries: Vec<EntryVm>,
    groups: Vec<GroupNode>,
    filter: Filter,
    visible: Vec<usize>,     // indices into `entries`
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
    editor: Option<(Entity<EntryEditor>, gpui::Subscription)>,
    /// Our Password Safe `.plk` lock; `None` when read-only.
    vault_lock: Option<VaultLock>,
    /// Who holds the lock when we are read-only.
    locked_by: Option<String>,
    /// The file changed on disk since we opened it; our edit is unsaved.
    conflict: bool,
    save_error: Option<SharedString>,
    /// Pending delete confirmation: (uuid, title).
    confirm_delete: Option<(Vec<u8>, SharedString)>,
    /// "Upgrade to v3" dialog: master-password field + error.
    upgrade: Option<(Entity<TextField>, Option<SharedString>)>,
    dialog_focus: FocusHandle,
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
    pub fn new(
        path: PathBuf,
        vault: Vault,
        existing_lock: Option<VaultLock>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let entries = model::build(&vault);
        let groups = model::groups(&entries);
        let search = cx.new(|cx| TextField::new(cx, "Search", false).with_icon("icons/search.svg"));
        cx.subscribe_in(&search, window, |this, _, ev, window, cx| match ev {
            FieldEvent::Changed => this.refilter(cx),
            FieldEvent::Submit => {
                if this.selected.is_some() {
                    this.copy(rec::PASSWORD, crate::i18n::t("Password"), cx);
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
            cx.background_executor()
                .timer(Duration::from_millis(1000))
                .await;
            if this.update_in(cx, |this, _, cx| this.tick(cx)).is_err() {
                break;
            }
        });

        // Don't leave a copied secret behind when the app exits.
        let on_quit = cx.on_app_quit(|this, cx| {
            this.clear_clipboard_if_ours(cx);
            async {}
        });

        // Password Safe compatible lock: someone else holding it => read-only.
        let (vault_lock, locked_by) = match existing_lock
            .map(|l| Ok((Some(l), LockState::Acquired)))
            .unwrap_or_else(|| VaultLock::acquire(&path))
        {
            Ok((lock, LockState::Acquired)) => (lock, None),
            // A lock left by a crashed/killed process on this machine is
            // safe to reclaim; anything else is the user's call.
            Ok((_, LockState::HeldBy(h))) if VaultLock::is_stale(&h) => {
                match VaultLock::take_over(&path) {
                    Ok(l) => (Some(l), None),
                    Err(_) => (None, Some(h)),
                }
            }
            Ok((_, LockState::HeldBy(h))) => (None, Some(h)),
            Err(e) => (None, Some(format!("? ({e})"))),
        };

        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut this = Self {
            path,
            vault: Some(vault),
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
            editor: None,
            vault_lock,
            locked_by,
            conflict: false,
            save_error: None,
            confirm_delete: None,
            upgrade: None,
            dialog_focus: cx.focus_handle(),
            _ticker: ticker,
            _on_quit: on_quit,
        };
        this.refilter(cx);
        this
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_none() && self.confirm_delete.is_none() {
            window.focus(&self.search.focus_handle(cx));
        }
    }

    fn vault(&self) -> &Vault {
        self.vault.as_ref().expect("vault taken")
    }

    fn vault_mut(&mut self) -> &mut Vault {
        self.vault.as_mut().expect("vault taken")
    }

    /// Hand the unlocked vault (and our `.plk` lock) to the app so the window
    /// can close while the session stays unlocked in the background. Clears
    /// the clipboard first if it still holds something we copied (this
    /// view's 30 s timer goes away with it).
    pub fn take_session(&mut self, cx: &mut Context<Self>) -> Option<(Vault, Option<VaultLock>)> {
        self.clear_clipboard_if_ours(cx);
        self.editor = None;
        self.confirm_delete = None;
        let v = self.vault.take()?;
        Some((v, self.vault_lock.take()))
    }

    pub fn read_only(&self) -> bool {
        self.vault_lock.is_none()
    }

    fn selected_record(&self) -> Option<&Record> {
        self.selected
            .map(|s| &self.vault().records[self.entries[s].index])
    }

    /// Rebuild the view model after the vault changed and re-select `uuid`.
    fn rebuild(&mut self, select_uuid: Option<Vec<u8>>, cx: &mut Context<Self>) {
        self.entries = model::build(self.vault());
        self.groups = model::groups(&self.entries);
        if let Filter::Group(p) = &self.filter {
            if !self.groups.iter().any(|g| &g.path == p) {
                self.filter = Filter::All;
            }
        }
        self.selected = select_uuid.and_then(|u| {
            let idx = self.vault().find(&u)?;
            self.entries.iter().position(|e| e.index == idx)
        });
        self.refilter(cx);
        if let Some(pos) = self.selected_pos() {
            self.scroll.scroll_to_item(pos, ScrollStrategy::Center);
        }
    }

    fn toast(&mut self, msg: impl Into<SharedString>) {
        self.toast = Some((msg.into(), Instant::now() + Duration::from_secs(3)));
    }

    /// Write the vault to disk (safe save). On a conflict the change stays in
    /// memory and a banner offers Overwrite / Discard & lock (re-unlock reloads the file).
    fn persist(&mut self, overwrite: bool, cx: &mut Context<Self>) -> bool {
        cx.notify();
        let res = if overwrite {
            {
                let path = self.path.clone();
                self.vault_mut().save_overwrite(&path)
            }
        } else {
            {
                let path = self.path.clone();
                self.vault_mut().save(&path)
            }
        };
        match res {
            Ok(()) => {
                self.conflict = false;
                self.save_error = None;
                self.toast("Saved");
                true
            }
            Err(Error::Conflict) => {
                self.conflict = true;
                false
            }
            Err(e) => {
                self.save_error = Some(tf("Save failed: {e}", &[("e", &e.to_string())]).into());
                false
            }
        }
    }

    #[cfg(feature = "devtools")]
    pub fn dev_long_passphrase(&mut self, cx: &mut Context<Self>) {
        if let Some((ed, _)) = &self.editor {
            ed.update(cx, |e, cx| e.dev_long_passphrase(cx));
        }
    }

    pub fn start_edit(&mut self, new: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        // Never silently replace an open form; focus it instead.
        if let Some((ed, _)) = &self.editor {
            window.focus(&ed.focus_handle(cx));
            return;
        }
        let default_group = match &self.filter {
            Filter::Group(p) => p.clone(),
            _ => vec![],
        };
        let record = if new {
            None
        } else {
            self.selected_record().cloned()
        };
        if !new && record.is_none() {
            return;
        }
        let ed = cx.new(|cx| EntryEditor::new(record.as_ref(), &default_group, window, cx));
        let sub = cx.subscribe_in(&ed, window, |this, ed, ev, window, cx| match ev {
            EditorEvent::Cancel => {
                this.editor = None;
                window.focus(&this.focus);
                cx.notify();
            }
            EditorEvent::Save => this.commit_editor(ed.clone(), window, cx),
        });
        self.editor = Some((ed, sub));
        cx.notify();
    }

    fn commit_editor(
        &mut self,
        ed: Entity<EntryEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only() {
            return;
        }
        let (form, target) = {
            let e = ed.read(cx);
            (e.form(cx), e.target.clone())
        };
        let form = match form {
            Ok(f) => f,
            Err(msg) => {
                ed.update(cx, |e, cx| {
                    e.error = Some(msg.into());
                    cx.notify();
                });
                return;
            }
        };
        let now = now_secs();
        let uuid = match target.as_deref().and_then(|u| self.vault().find(u)) {
            Some(i) => {
                form.apply(&mut self.vault_mut().records[i], now);
                target.unwrap()
            }
            None => {
                let Ok(mut r) = Record::new() else { return };
                form.apply(&mut r, now);
                let u = r.uuid().map(<[u8]>::to_vec).unwrap_or_default();
                self.vault_mut().records.push(r);
                u
            }
        };
        self.editor = None;
        self.rebuild(Some(uuid), cx);
        self.persist(false, cx);
        window.focus(&self.focus);
        cx.notify();
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        if self.read_only() || self.editor.is_some() {
            return;
        }
        let Some(src) = self.selected_record().cloned() else {
            return;
        };
        let Ok(mut r) = Record::new() else { return };
        let uuid = r.uuid().map(<[u8]>::to_vec).unwrap_or_default();
        for f in &src.fields {
            if !matches!(
                f.kind,
                rec::UUID | rec::CTIME | rec::MTIME | rec::PTIME | rec::PW_HISTORY
            ) {
                r.fields.push(f.clone());
            }
        }
        r.set_text(rec::TITLE, &format!("{} (copy)", src.title()));
        self.vault_mut().records.push(r);
        self.rebuild(Some(uuid), cx);
        self.persist(false, cx);
    }

    /// Ask (in-app dialog) before deleting the selected entry.
    pub fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() || self.editor.is_some() {
            return;
        }
        let Some(r) = self.selected_record() else {
            return;
        };
        let Some(uuid) = r.uuid().map(<[u8]>::to_vec) else {
            return;
        };
        self.confirm_delete = Some((uuid, r.title().to_string().into()));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    fn cancel_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((uuid, _)) = self.confirm_delete.take() else {
            return;
        };
        window.focus(&self.focus);
        if self.read_only() {
            return;
        }
        if let Some(i) = self.vault().find(&uuid) {
            let pos = self.selected_pos().unwrap_or(0);
            self.vault_mut().records.remove(i); // fields zeroize on drop
            self.rebuild(None, cx);
            if !self.visible.is_empty() {
                self.select_visible(pos.min(self.visible.len() - 1), cx);
            }
            self.persist(false, cx);
        }
        cx.notify();
    }

    pub fn open_upgrade(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() || self.editor.is_some() {
            return;
        }
        let f =
            cx.new(|cx| TextField::new(cx, "Master password", true).with_icon("icons/lock.svg"));
        cx.subscribe_in(&f, window, |this, _, ev, window, cx| match ev {
            FieldEvent::Submit => this.do_upgrade(window, cx),
            FieldEvent::Escape => {
                this.upgrade = None;
                window.focus(&this.focus);
                cx.notify();
            }
            FieldEvent::Changed => {}
        })
        .detach();
        window.focus(&f.focus_handle(cx));
        self.upgrade = Some((f, None));
        cx.notify();
    }

    /// v2 → v3: re-check the password, write `<name>.psafe3` next to the v2
    /// file (which stays untouched), and continue on the new file.
    fn do_upgrade(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((field, _)) = &self.upgrade else {
            return;
        };
        let pw = zeroize::Zeroizing::new(field.read(cx).text().as_bytes().to_vec());
        field.update(cx, |f, cx| f.clear(cx));
        let new_path = self.path.with_extension("psafe3");
        let fail = |this: &mut Self, msg: String, cx: &mut Context<Self>| {
            if let Some((_, err)) = &mut this.upgrade {
                *err = Some(msg.into());
            }
            cx.notify();
        };
        if new_path.exists() {
            let name = new_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            return fail(
                self,
                tf(
                    "{name} already exists; move it away first.",
                    &[("name", &name)],
                ),
                cx,
            );
        }
        // All-or-nothing in the core: on failure the vault stays v2.
        match self.vault_mut().upgrade_to_v3_at(&pw, &new_path) {
            Ok(()) => {}
            Err(Error::WrongPassword) => {
                return fail(self, crate::i18n::t("Wrong master password.").into(), cx)
            }
            Err(e) => {
                return fail(
                    self,
                    tf("Could not write the v3 file: {e}", &[("e", &e.to_string())]),
                    cx,
                )
            }
        }
        // Switch to the new file: release the v2 lock, take the v3 one.
        self.vault_lock = None;
        self.path = new_path.clone();
        match VaultLock::acquire(&new_path) {
            Ok((Some(l), _)) => self.vault_lock = Some(l),
            _ => {
                self.save_error =
                    Some(crate::i18n::t("Upgraded, but the new file's lock could not be taken; read-only until you reopen it.").into())
            }
        }
        config::remember_vault(&new_path);
        cx.emit(BrowserEvent::PathChanged(new_path.clone()));
        self.upgrade = None;
        self.toast(tf(
            "Upgraded — now using {name} (the v2 file is unchanged)",
            &[(
                "name",
                &new_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )],
        ));
        window.focus(&self.focus);
        cx.notify();
    }

    fn render_upgrade(
        &self,
        corner: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let (field, err) = self.upgrade.as_ref()?;
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
                    d.bg(c(ACCENT))
                        .text_color(c(ACCENT_INK))
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
        let target = self.path.with_extension("psafe3");
        let target = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(
            div()
                .id("upgrade-backdrop")
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
                        .id("upgrade-card")
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
                        .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(c(TEXT)).child(crate::i18n::t("Upgrade to Password Safe v3")))
                        .child(
                            div().text_size(px(12.5)).text_color(c(TEXT_SOFT)).line_height(px(19.)).child(SharedString::from(tf(
                                "Keywhere writes a new file, {target}, with modern encryption (262,144 key‑stretching rounds and tamper detection) and continues with it. The v2 file stays exactly as it is. Password Gorilla 1.5+ and Password Safe 3 open v3.",
                                &[("target", &target)],
                            ))),
                        )
                        .child(field.clone())
                        .when_some(err.clone(), |d, e| d.child(div().text_size(px(11.5)).text_color(c(DANGER)).child(e)))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(btn("up-cancel", crate::i18n::t("Cancel"), false).on_click(cx.listener(|t, _, w, cx| {
                                    t.upgrade = None;
                                    w.focus(&t.focus);
                                    cx.notify();
                                })))
                                .child(btn("up-go", crate::i18n::t("Upgrade"), true).on_click(cx.listener(|t, _, w, cx| t.do_upgrade(w, cx)))),
                        ),
                ),
        )
    }

    fn render_confirm_delete(
        &self,
        corner: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let (_, title) = self.confirm_delete.as_ref()?;
        let btn = |id: &'static str, label: &'static str, danger: bool| {
            div()
                .id(id)
                .h(px(32.))
                .px_4()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(danger, |d| {
                    d.bg(c(0xD9534F))
                        .text_color(c(0xFFFFFF))
                        .font_weight(FontWeight::SEMIBOLD)
                        .hover(|d| d.bg(c(0xE5605C)))
                })
                .when(!danger, |d| {
                    d.bg(c(RAISED))
                        .border_1()
                        .border_color(c(BORDER))
                        .text_color(c(TEXT))
                        .hover(|d| d.bg(c(HOVER)))
                })
                .child(label)
        };
        Some(
            div()
                .id("confirm-backdrop")
                .absolute()
                .inset_0()
                .rounded(corner)
                .flex()
                .items_center()
                .justify_center()
                .bg(ca(0x050B1A, 0xB0))
                .occlude()
                .on_click(cx.listener(|t, _, w, cx| t.cancel_delete(w, cx)))
                .child(
                    div()
                        .id("confirm-card")
                        .key_context("ConfirmDialog")
                        .track_focus(&self.dialog_focus)
                        .on_key_down(cx.listener(|t, ev: &gpui::KeyDownEvent, w, cx| {
                            match ev.keystroke.key.as_str() {
                                "escape" => t.cancel_delete(w, cx),
                                "enter" => t.confirm_delete(w, cx),
                                _ => return,
                            }
                            cx.stop_propagation();
                        }))
                        // Clicks inside the card must not reach the backdrop.
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .w(px(400.))
                        .p_5()
                        .rounded(px(12.))
                        .bg(c(PANEL))
                        .border_1()
                        .border_color(c(0x2A4A85))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .size(px(36.))
                                        .flex_none()
                                        .rounded(px(9.))
                                        .bg(ca(DANGER, 0x26))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(svg().path("icons/warning.svg").size(px(18.)).text_color(c(DANGER))),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(15.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(c(TEXT))
                                        .child(SharedString::from(tf("Delete “{title}”?", &[("title", title)]))),
                                ),
                        )
                        .child(
                            div().text_size(px(12.5)).text_color(c(TEXT_SOFT)).line_height(px(19.)).child(
                                crate::i18n::t("The entry is removed from the vault file. The previous version of the file is kept next to it as a .ibak backup."),
                            ),
                        )
                        .child(
                            div()
                                .mt_1()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(btn("confirm-cancel", crate::i18n::t("Cancel"), false).on_click(cx.listener(|t, _, w, cx| t.cancel_delete(w, cx))))
                                .child(btn("confirm-delete", crate::i18n::t("Delete"), true).on_click(cx.listener(|t, _, w, cx| t.confirm_delete(w, cx)))),
                        )
                        .child(div().text_size(px(10.5)).text_color(c(DIM)).flex().justify_end().child(crate::i18n::t("Enter delete · Esc cancel"))),
                ),
        )
    }

    fn take_over_lock(&mut self, cx: &mut Context<Self>) {
        match VaultLock::take_over(&self.path) {
            Ok(l) => {
                self.vault_lock = Some(l);
                self.locked_by = None;
                self.toast("Lock taken over — editing enabled");
            }
            Err(e) => {
                self.save_error =
                    Some(tf("Could not take lock: {e}", &[("e", &e.to_string())]).into())
            }
        }
        cx.notify();
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
            let current = crate::sys::clipboard::get_text().or_else(|| {
                cx.read_from_clipboard()
                    .and_then(|c| c.text())
                    .map(zeroize::Zeroizing::new)
            });
            if current.as_deref().map(|s| hash(s)) == Some(h) {
                crate::sys::clipboard::clear();
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
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
            .and_then(|k| {
                self.visible
                    .iter()
                    .copied()
                    .find(|&i| self.entries[i].index == k)
            })
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
        if let Some(pos) = self
            .visible
            .iter()
            .position(|&i| self.entries[i].title == title)
        {
            self.select_visible(pos, cx);
        }
    }

    fn selected_pos(&self) -> Option<usize> {
        let s = self.selected?;
        self.visible.iter().position(|&i| i == s)
    }

    fn copy(&mut self, kind: u8, label: &'static str, cx: &mut Context<Self>) {
        let Some(sel) = self.selected else { return };
        let r = &self.vault().records[self.entries[sel].index];
        let Some(text) = r.text(kind).filter(|t| !t.is_empty()) else {
            return;
        };
        let text = zeroize::Zeroizing::new(text.to_string());
        self.copy_text(&text, label, cx);
    }

    pub fn copy_password(&mut self, cx: &mut Context<Self>) {
        self.copy(rec::PASSWORD, crate::i18n::t("Password"), cx);
    }

    fn copy_text(&mut self, text: &str, label: &'static str, cx: &mut Context<Self>) {
        self.clip_hash = Some(hash(text));
        self.clip_until = Some(Instant::now() + crate::settings::get().clipboard_clear());
        // Secure path first (clipboard-history exclusion); GPUI as fallback.
        if !crate::sys::clipboard::set_secret(text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        }
        self.toast(tf("{label} copied", &[("label", crate::i18n::t(label))]));
        cx.notify();
    }

    fn copy_history(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(r) = self.selected_record() else {
            return;
        };
        let Some(e) = r.history().into_iter().rev().nth(i) else {
            return;
        };
        self.copy_text(&e.password, crate::i18n::t("Old password"), cx);
    }

    fn toggle_reveal(&mut self, cx: &mut Context<Self>) {
        self.revealed = !self.revealed;
        self.reveal_until = self.revealed.then(|| Instant::now() + REVEAL_FOR);
        cx.notify();
    }

    fn open_url(&mut self, cx: &mut Context<Self>) {
        let Some(sel) = self.selected else { return };
        if let Some(url) = self.vault().records[self.entries[sel].index].text(rec::URL) {
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
            .when_some(icon, |d, i| {
                d.child(
                    svg()
                        .path(i)
                        .size(px(13.))
                        .text_color(c(if on { TEXT } else { MUTED })),
                )
            })
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label),
            )
            .child(match badge {
                Some(col) if count > 0 => div()
                    .px(px(6.))
                    .rounded(px(8.))
                    .text_size(px(10.5))
                    .bg(ca(col, 0x2a))
                    .text_color(c(col))
                    .child(count.to_string()),
                _ => div()
                    .text_size(px(11.))
                    .text_color(c(DIM))
                    .child(count.to_string()),
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let now = now_secs() as u64;
        let expiring = self
            .entries
            .iter()
            .filter(|e| model::is_expiring(e, now))
            .count();
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
            .child(self.nav_item(
                "all".into(),
                Some("icons/list.svg"),
                crate::i18n::t("All entries").into(),
                self.entries.len(),
                0,
                Filter::All,
                None,
                cx,
            ))
            .child(self.nav_item(
                "exp".into(),
                Some("icons/clock.svg"),
                crate::i18n::t("Expiring").into(),
                expiring,
                0,
                Filter::Expiring,
                Some(WARN),
                cx,
            ))
            .child(
                div()
                    .mt_3()
                    .mb_1()
                    .px_2()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(DIM))
                    .child(crate::i18n::t("GROUPS")),
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
            Filter::All => crate::i18n::t("All entries").into(),
            Filter::Expiring => crate::i18n::t("Expiring").into(),
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
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(DIM))
                            .child(format!("{n}")),
                    ),
            )
            .child(if n == 0 {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c(DIM))
                    .child(crate::i18n::t("No matching entries"))
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
        let initial: SharedString = e
            .title
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into();
        let sub: SharedString = match (
            e.username.is_empty(),
            e.group_label.is_empty(),
            matches!(self.filter, Filter::Group(_)),
        ) {
            (false, false, false) => format!("{} · {}", e.username, e.group_label).into(),
            (false, _, _) => e.username.clone(),
            (true, false, _) => e.group_label.clone(),
            _ => "".into(),
        };
        let exp_badge = e.expiry.filter(|_| model::is_expiring(e, now)).map(|t| {
            if t <= now {
                (crate::i18n::t("expired").to_string(), DANGER)
            } else {
                (format!("{}d", (t - now).div_ceil(86_400)), WARN)
            }
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
            .when(on, |d| {
                d.bg(c(SELECTION)).border_l_2().border_color(c(ACCENT))
            })
            .when(!on, |d| d.hover(|d| d.bg(c(HOVER))))
            .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                this.select_visible(pos, cx);
                if ev.click_count() >= 2 {
                    this.copy(rec::PASSWORD, crate::i18n::t("Password"), cx);
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
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(e.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(MUTED))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(sub),
                    ),
            )
            .when_some(exp_badge, |d, (t, col)| {
                d.child(
                    div()
                        .px(px(6.))
                        .rounded(px(8.))
                        .text_size(px(10.))
                        .bg(ca(col, 0x2a))
                        .text_color(c(col))
                        .child(t),
                )
            })
    }

    fn icon_btn(id: &'static str, icon: &'static str) -> gpui::Stateful<gpui::Div> {
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

    fn text_btn(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(7.))
            .border_1()
            .border_color(c(BORDER))
            .bg(c(RAISED))
            .text_size(px(11.5))
            .text_color(c(TEXT_SOFT))
            .cursor_pointer()
            .hover(|d| d.bg(c(HOVER)))
            .child(crate::i18n::t(label))
    }

    fn banner(id: &'static str, col: u32, msg: SharedString) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py(px(7.))
            .bg(ca(col, 0x1c))
            .border_b_1()
            .border_color(ca(col, 0x55))
            .text_size(px(12.))
            .text_color(c(col))
            .child(
                svg()
                    .path("icons/warning.svg")
                    .size(px(13.))
                    .text_color(c(col)),
            )
            .child(div().flex_1().child(msg))
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
            .child(
                div()
                    .w(px(76.))
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(c(DIM))
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(value),
            )
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // min_w_0: a flex child must be allowed to shrink below its content
        // (e.g. a long unbroken passphrase), or the whole pane overflows.
        let base = div().flex_1().min_w_0().h_full().bg(c(PANEL));
        if let Some((ed, _)) = &self.editor {
            return base.child(ed.clone()).into_any_element();
        }
        let Some(sel) = self.selected else {
            return base
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(DIM))
                .child(crate::i18n::t("Select an entry"))
                .into_any_element();
        };
        let e = &self.entries[sel];
        let r = &self.vault().records[e.index];
        let now = now_secs() as u64;
        let pw = r.text(rec::PASSWORD).unwrap_or("");
        let bits = model::strength_bits(pw);
        let bars = if pw.is_empty() {
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
        let pw_shown: SharedString = if self.revealed {
            // Hidden again after REVEAL_FOR or on lock; the Arc is wiped by
            // secret_text::sweep once GPUI drops it.
            crate::secret_text::display(pw)
        } else if pw.is_empty() {
            "—".into()
        } else {
            "••••••••••••".into()
        };

        let mut fields = div()
            .mt_3()
            .rounded(px(9.))
            .border_1()
            .border_color(c(LINE))
            .bg(c(SURFACE))
            .overflow_hidden();
        if let Some(u) = r.text(rec::USERNAME).filter(|s| !s.is_empty()) {
            fields = fields.child(
                Self::field_row(
                    crate::i18n::t("Username"),
                    div()
                        .text_color(c(TEXT))
                        .child(SharedString::from(u.to_string())),
                )
                .child(Self::icon_btn("cu", "icons/copy.svg").on_click(
                    cx.listener(|t, _, _, cx| {
                        t.copy(rec::USERNAME, crate::i18n::t("Username"), cx)
                    }),
                )),
            );
        }
        fields =
            fields.child(
                Self::field_row(
                    crate::i18n::t("Password"),
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_color(c(TEXT))
                                .font_family("monospace")
                                .child(pw_shown),
                        )
                        .when(bars > 0, |d| {
                            d.child(div().flex().gap(px(3.)).children((0..4).map(move |i| {
                                div()
                                    .w(px(14.))
                                    .h(px(4.))
                                    .rounded(px(2.))
                                    .bg(c(if i < bars { bar_col } else { RAISED }))
                            })))
                        }),
                )
                .when(!pw.is_empty(), |d| {
                    d.child(
                        Self::icon_btn(
                            "rv",
                            if self.revealed {
                                "icons/eye-off.svg"
                            } else {
                                "icons/eye.svg"
                            },
                        )
                        .on_click(cx.listener(|t, _, _, cx| t.toggle_reveal(cx))),
                    )
                    .child(Self::icon_btn("cp", "icons/copy.svg").on_click(cx.listener(
                        |t, _, _, cx| t.copy(rec::PASSWORD, crate::i18n::t("Password"), cx),
                    )))
                }),
            );
        if let Some(u) = r.text(rec::URL).filter(|s| !s.is_empty()) {
            let web = u.starts_with("http://") || u.starts_with("https://");
            fields = fields.child(
                Self::field_row(
                    crate::i18n::t("Website"),
                    div()
                        .text_color(c(if web { LINK } else { TEXT }))
                        .child(SharedString::from(u.to_string())),
                )
                .child(
                    Self::icon_btn("cl", "icons/copy.svg")
                        .on_click(cx.listener(|t, _, _, cx| t.copy(rec::URL, "URL", cx))),
                )
                .when(web, |d| {
                    d.child(
                        Self::icon_btn("ou", "icons/external.svg")
                            .on_click(cx.listener(|t, _, _, cx| t.open_url(cx))),
                    )
                }),
            );
        }
        if let Some(m) = r.text(rec::EMAIL).filter(|s| !s.is_empty()) {
            fields = fields.child(
                Self::field_row(
                    crate::i18n::t("Email"),
                    div()
                        .text_color(c(TEXT))
                        .child(SharedString::from(m.to_string())),
                )
                .child(Self::icon_btn("ce", "icons/copy.svg").on_click(
                    cx.listener(|t, _, _, cx| t.copy(rec::EMAIL, crate::i18n::t("Email"), cx)),
                )),
            );
        }

        let notes = r.text(rec::NOTES).filter(|s| !s.is_empty()).map(|n| {
            crate::secret_text::display(&zeroize::Zeroizing::new(n.replace("\r\n", "\n")))
        });
        let mut meta: Vec<String> = vec![];
        if let Some(t) = r.time(rec::CTIME) {
            meta.push(tf("Created {date}", &[("date", &model::fmt_date(t))]));
        }
        if let Some(t) = r.time(rec::MTIME) {
            meta.push(tf("Modified {date}", &[("date", &model::fmt_date(t))]));
        }
        if let Some(t) = r.time(rec::PTIME) {
            meta.push(tf(
                "Password changed {date}",
                &[("date", &model::fmt_date(t))],
            ));
        }
        // Newest first; passwords stay masked, copy-only.
        let history: Vec<String> = r
            .history()
            .iter()
            .rev()
            .map(|h| model::fmt_date(h.changed))
            .collect();

        let initial: SharedString = e
            .title
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into();

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
                    .child(
                        div()
                            .size(px(44.))
                            .rounded(px(10.))
                            .bg(c(ACCENT))
                            .text_color(c(ACCENT_INK))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD)
                            .child(initial),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(c(TEXT))
                                    .child(e.title.clone()),
                            )
                            .child(div().text_size(px(11.5)).text_color(c(MUTED)).child(
                                if e.group_label.is_empty() {
                                    SharedString::from(crate::i18n::t("No group"))
                                } else {
                                    e.group_label.clone()
                                },
                            )),
                    )
                    .when(!self.read_only(), |d| {
                        d.child(
                            div()
                                .flex()
                                .gap_1()
                                .child(Self::text_btn("ed", crate::i18n::t("Edit")).on_click(
                                    cx.listener(|t, _, w, cx| t.start_edit(false, w, cx)),
                                ))
                                .child(
                                    Self::text_btn("dup", crate::i18n::t("Duplicate"))
                                        .on_click(cx.listener(|t, _, _, cx| t.duplicate(cx))),
                                )
                                .child(
                                    Self::text_btn("del", crate::i18n::t("Delete"))
                                        .on_click(cx.listener(|t, _, w, cx| t.delete(w, cx))),
                                ),
                        )
                    }),
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
                            .child(
                                svg()
                                    .path("icons/clock.svg")
                                    .size(px(13.))
                                    .text_color(c(col)),
                            )
                            .child(msg),
                    )
                })
            })
            .child(fields)
            .when(!history.is_empty(), |d| {
                d.child(
                    div()
                        .mt_3()
                        .rounded(px(9.))
                        .border_1()
                        .border_color(c(LINE))
                        .bg(c(SURFACE))
                        .overflow_hidden()
                        .child(
                            div()
                                .px_3()
                                .pt_2()
                                .pb_1()
                                .text_size(px(10.5))
                                .text_color(c(DIM))
                                .child(tf(
                                    "PASSWORD HISTORY · {n}",
                                    &[("n", &history.len().to_string())],
                                )),
                        )
                        .children(history.into_iter().enumerate().map(|(i, date)| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .h(px(32.))
                                .border_t_1()
                                .border_color(c(LINE))
                                .child(
                                    div()
                                        .w(px(110.))
                                        .text_size(px(11.))
                                        .text_color(c(MUTED))
                                        .child(tf("set {date}", &[("date", &date)])),
                                )
                                .child(div().flex_1().text_color(c(DIM)).child("••••••••••••"))
                                .child(
                                    div()
                                        .id(("hist", i))
                                        .size(px(24.))
                                        .rounded(px(6.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(c(RAISED)))
                                        .on_click(
                                            cx.listener(move |t, _, _, cx| t.copy_history(i, cx)),
                                        )
                                        .child(
                                            svg()
                                                .path("icons/copy.svg")
                                                .size(px(14.))
                                                .text_color(c(MUTED)),
                                        ),
                                )
                        })),
                )
            })
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
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(c(DIM))
                                .mb_1()
                                .child(crate::i18n::t("NOTES")),
                        )
                        .child(n),
                )
            })
            .child(
                div()
                    .mt_3()
                    .flex()
                    .flex_wrap()
                    .gap_x_4()
                    .gap_y_1()
                    .text_size(px(11.))
                    .text_color(c(DIM))
                    .children(meta),
            )
            .into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.vault.is_none() {
            // Session handed off (window closing): draw nothing.
            return div().size_full().bg(c(BG)).into_any_element();
        }
        let corner = chrome::corner_radius(window);
        let clip_left = self
            .clip_until
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs());
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
            .relative()
            .rounded(corner)
            .flex()
            .flex_col()
            .bg(c(BG))
            .text_color(c(TEXT))
            .text_size(px(12.5))
            .on_action(cx.listener(|_, _: &Lock, _, cx| cx.emit(BrowserEvent::LockRequested)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                window.focus(&this.search.focus_handle(cx))
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                let p = this
                    .selected_pos()
                    .map_or(0, |p| (p + 1).min(this.visible.len().saturating_sub(1)));
                this.select_visible(p, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                let p = this.selected_pos().map_or(0, |p| p.saturating_sub(1));
                this.select_visible(p, cx);
            }))
            .on_action(
                cx.listener(|this, _: &CopyPassword, _, cx| {
                    this.copy(rec::PASSWORD, crate::i18n::t("Password"), cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &CopyUsername, _, cx| {
                    this.copy(rec::USERNAME, crate::i18n::t("Username"), cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenUrl, _, cx| this.open_url(cx)))
            .on_action(cx.listener(|this, _: &ToggleReveal, _, cx| this.toggle_reveal(cx)))
            .on_action(cx.listener(|this, _: &NewEntry, w, cx| this.start_edit(true, w, cx)))
            .on_action(cx.listener(|this, _: &EditEntry, w, cx| {
                if this.editor.is_none() {
                    this.start_edit(false, w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &DuplicateEntry, _, cx| this.duplicate(cx)))
            .on_action(cx.listener(|this, _: &DeleteEntry, w, cx| {
                if this.editor.is_none() {
                    this.delete(w, cx)
                }
            }))
            // toolbar = title bar (drag on logo/empty space, window buttons at right)
            .child(
                div()
                    .h(px(46.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pr(px(6.))
                    .rounded_t(corner)
                    .bg(c(SURFACE))
                    .border_b_1()
                    .border_color(c(LINE))
                    .child(
                        chrome::drag_area("tb-left")
                            .h_full()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_3()
                            .pl(px(12.) + chrome::leading_inset())
                            .pr_3()
                            .child(crate::assets::logo(20.).flex_none())
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Keywhere"))
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(c(DIM))
                                    .text_size(px(11.5))
                                    .child(file),
                            ),
                    )
                    .child(chrome::drag_area("tb-mid-l").h_full().flex_1())
                    .child(div().w(px(340.)).flex_none().child(self.search.clone()))
                    .child(chrome::drag_area("tb-mid-r").h_full().flex_1())
                    .child(chrome::settings_button("tb-settings").mr_1())
                    .when(!self.read_only(), |d| {
                        d.child(
                            div()
                                .id("new")
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .mr_2()
                                .h(px(26.))
                                .rounded(px(7.))
                                .bg(c(ACCENT))
                                .text_color(c(ACCENT_INK))
                                .font_weight(FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .hover(|d| d.opacity(0.9))
                                .on_click(cx.listener(|t, _, w, cx| t.start_edit(true, w, cx)))
                                .child(div().text_size(px(15.)).child("+"))
                                .child(div().text_size(px(11.5)).child(crate::i18n::t("New"))),
                        )
                    })
                    .child(
                        div()
                            .id("lock")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .mr_2()
                            .h(px(26.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(c(BORDER))
                            .bg(c(RAISED))
                            .cursor_pointer()
                            .hover(|d| d.bg(c(HOVER)))
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(BrowserEvent::LockRequested)),
                            )
                            .child(
                                svg()
                                    .path("icons/lock.svg")
                                    .size(px(13.))
                                    .text_color(c(TEXT_SOFT)),
                            )
                            .child(div().text_size(px(11.5)).child(crate::i18n::t("Lock"))),
                    )
                    .child(chrome::window_controls(window)),
            )
            .when_some(self.locked_by.clone(), |d, who| {
                let stale = VaultLock::is_stale(&who);
                d.child(
                    Self::banner(
                        "ro",
                        WARN,
                        if stale {
                            tf("Read-only: the vault is open in another app ({who}) — that process is no longer running.", &[("who", &who)])
                        } else {
                            tf("Read-only: the vault is open in another app ({who}).", &[("who", &who)])
                        }
                        .into(),
                    )
                    .child(Self::text_btn("takeover", crate::i18n::t("Take over lock")).on_click(cx.listener(|t, _, _, cx| t.take_over_lock(cx)))),
                )
            })
            .when(self.conflict, |d| {
                d.child(
                    Self::banner(
                        "conflict",
                        DANGER,
                        crate::i18n::t("The vault file changed on disk (another device or app). Your last change is NOT saved yet.").into(),
                    )
                    .child(Self::text_btn("overwrite", crate::i18n::t("Overwrite file")).on_click(cx.listener(|t, _, _, cx| {
                        t.persist(true, cx);
                        cx.notify();
                    })))
                    .child(Self::text_btn("reload", crate::i18n::t("Discard & lock")).on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(BrowserEvent::LockRequested)
                    }))),
                )
            })
            .when(self.vault().format() == Format::V2, |d| {
                d.child(
                    Self::banner(
                        "v2",
                        WARN,
                        crate::i18n::t("Legacy Password Safe v2 file (Password Gorilla). Kept in v2 so older apps can still open it, but v2 has weak key derivation and no tamper detection.").into(),
                    )
                    .when(!self.read_only(), |b| {
                        b.child(Self::text_btn("upgrade", crate::i18n::t("Upgrade to v3…")).on_click(cx.listener(|t, _, w, cx| t.open_upgrade(w, cx))))
                    }),
                )
            })
            .when_some(self.save_error.clone(), |d, e| d.child(Self::banner("saveerr", DANGER, e)))
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
                    .rounded_b(corner)
                    .bg(c(SURFACE))
                    .border_t_1()
                    .border_color(c(LINE))
                    .text_size(px(11.))
                    .text_color(c(DIM))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().size(px(7.)).rounded_full().bg(c(if self.conflict {
                                DANGER
                            } else if self.read_only() {
                                WARN
                            } else {
                                OK
                            })))
                            .child(format!(
                                "{} · {}",
                                if self.conflict {
                                    crate::i18n::t("Unsaved change")
                                } else if self.read_only() {
                                    crate::i18n::t("Read‑only")
                                } else {
                                    crate::i18n::t("Saved")
                                },
                                match self.vault().format() {
                                    Format::V3 => "psafe3",
                                    Format::V2 => crate::i18n::t("Password Safe v2 (legacy)"),
                                }
                            )),
                    )
                    .child(tf("{n} entries", &[("n", &self.entries.len().to_string())]))
                    .child(div().flex_1())
                    .when_some(self.toast.as_ref().map(|t| t.0.clone()), |d, t| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_color(c(OK))
                                .child(
                                    svg()
                                        .path("icons/check.svg")
                                        .size(px(12.))
                                        .text_color(c(OK)),
                                )
                                .child(t),
                        )
                    })
                    .when_some(clip_left, |d, s| {
                        d.child(tf("Clipboard clears in {s}s", &[("s", &s.to_string())]))
                    })
                    .child(if cfg!(target_os = "macos") {
                        crate::i18n::t("⌘N new · ⌘E edit · ⌘C password · ⌘B user · ⌘L lock")
                    } else {
                        crate::i18n::t("Ctrl+N new · Ctrl+E edit · Ctrl+C password · Ctrl+B user · Ctrl+L lock")
                    })
                    // Build watermark: which commit this binary is.
                    .child(
                        div()
                            .pl_3()
                            .border_l_1()
                            .border_color(c(LINE))
                            .font_family("monospace")
                            .text_color(c(if crate::COMMIT.ends_with("-dirty") { WARN } else { DIM }))
                            .child(crate::COMMIT),
                    ),
            )
            .children(self.render_confirm_delete(corner, cx))
            .children(self.render_upgrade(corner, cx))
            .into_any_element()
    }
}

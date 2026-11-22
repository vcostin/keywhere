//! Keywhere desktop app (GPUI) as a library, so dev tools/examples can drive it.

pub mod alloc;
pub mod app_icon;
pub mod assets;

/// Freed memory is zeroed process-wide (see alloc.rs).
#[global_allocator]
static ALLOC: alloc::ZeroizeOnFree = alloc::ZeroizeOnFree;
pub mod browser;
pub mod chrome;
pub mod config;
pub mod editor;
pub mod field;
pub mod i18n;
pub mod memlock;
pub mod model;
pub mod secret_text;
pub mod select;
pub mod settings;
pub mod settings_view;
pub mod sys;
pub mod theme;
pub mod unlock;
pub mod update;
pub mod update_view;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    div, prelude::*, px, size, App, Application, Bounds, Context, Entity, FocusHandle, Focusable,
    Global, KeyBinding, Window, WindowBounds, WindowHandle, WindowOptions,
};
use keywhere_core::lock::VaultLock;
use keywhere_core::Vault;

use browser::{Browser, BrowserEvent};

/// e.g. "0.1.0 · a4cd086 · 2026-11-20 18:09 UTC" ("-dirty" if built with
/// uncommitted changes).
pub const VERSION_LABEL: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " · ",
    env!("KEYWHERE_COMMIT"),
    " · ",
    env!("KEYWHERE_BUILD_DATE")
);
pub const COMMIT: &str = env!("KEYWHERE_COMMIT");
use unlock::{UnlockEvent, UnlockView};

/// Idle time before locking (from settings; `None` = never).
fn idle_lock() -> Option<Duration> {
    settings::get().idle_lock()
}

/// App-wide state that outlives any one window (the tray keeps the app
/// running after the window is closed).
struct AppState {
    path: Option<PathBuf>,
    tray: sys::Tray,
    window: Option<WindowHandle<Shell>>,
    /// Unlocked vault kept while the window is hidden to the tray. Dropped
    /// (zeroized) on lock, after the auto-lock time, on screen lock / sleep, on quit.
    hidden: Option<HiddenSession>,
}

struct HiddenSession {
    path: PathBuf,
    vault: Vault,
    lock: Option<VaultLock>,
    since: Instant,
}

/// Type for handing an unlocked vault to a new window.
type Preunlocked = Option<(Vault, Option<VaultLock>)>;

fn drop_hidden_session(cx: &mut App) {
    if cx.has_global::<AppState>() && cx.global_mut::<AppState>().hidden.take().is_some() {
        secret_text::sweep();
        set_tray(sys::TrayState::Locked, cx);
    }
}

impl Global for AppState {}

fn set_tray(state: sys::TrayState, cx: &mut App) {
    if cx.has_global::<AppState>() {
        cx.global::<AppState>().tray.set_state(state);
    }
}

enum Screen {
    Unlock(Entity<UnlockView>),
    Vault(Entity<Browser>),
}

struct Shell {
    screen: Screen,
    /// A vault opened from the file manager while another one is unlocked:
    /// ask before locking it.
    pending_open: Option<PathBuf>,
    /// Keyboard focus for that question (Enter opens, Escape cancels).
    open_focus: FocusHandle,
    settings: Option<Entity<settings_view::SettingsView>>,
    last_input: Instant,
    _idle: gpui::Task<()>,
    _on_quit: gpui::Subscription,
}

impl Shell {
    fn new(
        path: Option<PathBuf>,
        preunlocked: Preunlocked,
        select: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        sys::capture::exclude(window);
        let idle = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            secret_text::sweep();
            let r = this.update_in(cx, |this, window, cx| {
                if matches!(this.screen, Screen::Vault(_))
                    && idle_lock().is_some_and(|d| this.last_input.elapsed() >= d)
                {
                    this.lock(window, cx);
                }
            });
            if r.is_err() {
                break;
            }
        });
        // Theme: resolve now and follow the desktop's light/dark switch live.
        theme::apply(settings::get().theme, window.appearance());
        cx.observe_window_appearance(window, |_, window, _| {
            theme::apply(settings::get().theme, window.appearance());
            window.refresh();
        })
        .detach();
        // Alt+F4 / taskbar "Close" go through the compositor, not our button:
        // route them to the same lock-and-hide logic and veto GPUI's default.
        window.on_window_should_close(cx, |window, cx| {
            window.dispatch_action(Box::new(chrome::CloseWindow), cx);
            false
        });
        let mut this = Self {
            screen: Screen::Unlock(Self::unlock_view(path.clone(), window, cx)),
            pending_open: None,
            open_focus: cx.focus_handle(),
            settings: None,
            last_input: Instant::now(),
            _idle: idle,
            // Best-effort final wipe of secret display strings on exit.
            _on_quit: cx.on_app_quit(|_, _| {
                secret_text::sweep();
                async {}
            }),
        };
        #[cfg(feature = "devtools")]
        if let Some(pw) = DEV_TYPED.lock().unwrap_or_else(|e| e.into_inner()).take() {
            if let Screen::Unlock(u) = &this.screen {
                u.update(cx, |u, cx| u.dev_type_and_unlock(&pw, window, cx));
            }
        }
        if let (Some((v, lock)), Some(p)) = (preunlocked, path) {
            this.show_vault(p, v, lock, window, cx);
            if matches!(
                select.as_deref(),
                Some("settings" | "settings-lang" | "settings-update")
            ) {
                this.open_settings(window, cx);
                #[cfg(feature = "devtools")]
                if select.as_deref() == Some("settings-lang") {
                    if let Some(v) = &this.settings {
                        v.update(cx, |v, cx| v.dev_open_language(cx));
                    }
                }
            }
            // Dev screenshots: "Title", "edit:Title", "delete:Title", "copy:Title", "settings" or "new".
            if let (Some(t), Screen::Vault(b)) = (select, &this.screen) {
                b.update(cx, |b, cx| match t.split_once(':') {
                    Some(("delete", title)) => {
                        b.select_title(title, cx);
                        b.delete(window, cx);
                    }
                    Some(("reveal", title)) => {
                        b.select_title(title, cx);
                        b.toggle_reveal(cx);
                    }
                    Some(("copy", title)) => {
                        b.select_title(title, cx);
                        b.copy_password(cx);
                    }
                    Some(("edit", title)) => {
                        b.select_title(title, cx);
                        b.start_edit(false, window, cx);
                    }
                    _ if t == "new" => b.start_edit(true, window, cx),
                    _ if t == "upgrade" => b.open_upgrade(window, cx),
                    #[cfg(feature = "devtools")]
                    _ if t.starts_with("fold:") => b.dev_fold(&t["fold:".len()..], cx),
                    #[cfg(feature = "devtools")]
                    _ if t == "nogroup" => b.dev_no_group(cx),
                    #[cfg(feature = "devtools")]
                    _ if t.starts_with("cal:") => {
                        b.select_title(&t["cal:".len()..], cx);
                        b.start_edit(false, window, cx);
                        b.dev_calendar(cx);
                    }
                    #[cfg(feature = "devtools")]
                    _ if t.starts_with("newgroup:") => {
                        b.dev_new_group(&t["newgroup:".len()..], window, cx)
                    }
                    #[cfg(feature = "devtools")]
                    _ if t.starts_with("menu:") => b.dev_group_menu_open(&t["menu:".len()..], cx),
                    #[cfg(feature = "devtools")]
                    _ if t.starts_with("group:") => {
                        b.start_edit(true, window, cx);
                        b.dev_group_menu(&t["group:".len()..], cx);
                    }
                    _ if t == "settings" || t == "settings-lang" => {}
                    #[cfg(feature = "devtools")]
                    _ if t == "update" || t == "settings-update" => update::dev_fake_available(),
                    #[cfg(feature = "devtools")]
                    _ if t == "newpass" => {
                        b.start_edit(true, window, cx);
                        b.dev_long_passphrase(cx);
                    }
                    _ => b.select_title(&t, cx),
                });
            }
        }
        this
    }

    fn unlock_view(
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<UnlockView> {
        let view = cx.new(|cx| UnlockView::new(path, window, cx));
        cx.subscribe_in(&view, window, |this, view, ev, window, cx| match ev {
            UnlockEvent::Unlocked => {
                let (vault, path, queued) = view.update(cx, |v, _| {
                    (v.result.take(), v.path.clone(), v.queued_open.take())
                });
                if let (Some(vault), Some(path)) = (vault, path) {
                    config::remember_vault(&path);
                    if cx.has_global::<AppState>() {
                        cx.global_mut::<AppState>().path = Some(path.clone());
                    }
                    this.show_vault(path, vault, None, window, cx);
                    // Opened from the file manager while the new vault was
                    // being created: ask, as for any open while unlocked.
                    if let Some(p) = queued {
                        this.request_open(p, window, cx);
                    }
                }
            }
        })
        .detach();
        view
    }

    fn show_vault(
        &mut self,
        path: PathBuf,
        vault: Vault,
        lock: Option<VaultLock>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let b = cx.new(|cx| Browser::new(path, vault, lock, window, cx));
        cx.subscribe_in(&b, window, |this, _, ev, window, cx| match ev {
            BrowserEvent::LockRequested => this.lock(window, cx),
            BrowserEvent::PathChanged(p) => {
                if cx.has_global::<AppState>() {
                    cx.global_mut::<AppState>().path = Some(p.clone());
                }
            }
        })
        .detach();
        // Replacing the screen drops the previous Browser (which clears the
        // memlock state), so record the new vault's state only afterwards.
        let locked = b.read(cx).vault().key_in_locked_memory();
        self.screen = Screen::Vault(b);
        memlock::record(locked);
        self.last_input = Instant::now();
        set_tray(sys::TrayState::Unlocked, cx);
        cx.notify();
    }

    /// Bring-to-front from the tray / `keywhere --show`: quick search when
    /// unlocked, password field otherwise.
    fn focus_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.screen {
            Screen::Vault(b) => b.update(cx, |b, cx| b.focus_search(window, cx)),
            Screen::Unlock(u) => window.focus(&u.focus_handle(cx)),
        }
    }

    /// Window close button / Alt+F4. With a tray: the window is removed
    /// entirely (no taskbar entry); by default the vault locks first. With
    /// "Hide, keep unlocked" chosen, an unlocked session is kept in the
    /// background (auto-locks after the auto-lock time, on screen lock / sleep, tray
    /// → Lock, quit); tray → Open brings it back unlocked. Without a tray:
    /// quit. (Relies on the vendored gpui keep-alive patch.)
    pub(crate) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tray = cx.has_global::<AppState>() && cx.global::<AppState>().tray.available();
        if !tray {
            self.lock(window, cx);
            cx.quit();
            return;
        }
        if settings::get().close_action == settings::CloseAction::Lock {
            self.lock(window, cx);
        }
        if let Screen::Vault(b) = &self.screen {
            let path = b.read(cx).path.clone();
            if let Some((vault, lock)) = b.update(cx, |b, cx| b.take_session(cx)) {
                cx.global_mut::<AppState>().hidden = Some(HiddenSession {
                    path,
                    vault,
                    lock,
                    // Idle time already spent counts: hiding must not
                    // grant a fresh unlocked period.
                    since: self.last_input,
                });
            }
        }
        cx.global_mut::<AppState>().window = None;
        window.remove_window();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let v = cx.new(|cx| settings_view::SettingsView::new(window, cx));
        cx.subscribe_in(&v, window, |this, _, ev, window, cx| match ev {
            settings_view::SettingsEvent::Close => this.close_settings(window, cx),
        })
        .detach();
        self.settings = Some(v);
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings = None;
        self.focus_primary(window, cx);
        cx.notify();
    }

    /// Drop the vault (zeroizing all decrypted fields) and return to unlock.
    /// Show `path` on the unlock screen. If a different vault is unlocked,
    /// ask first (locking it is the user's call).
    fn request_open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match &self.screen {
            Screen::Unlock(u) => {
                self.pending_open = None;
                u.update(cx, |u, cx| u.set_path(path, window, cx));
            }
            Screen::Vault(b) => {
                // Same file only if both resolve to the same path (or, if
                // either can't be resolved, the paths are identical).
                let same = |a: &std::path::Path| match (
                    std::fs::canonicalize(a),
                    std::fs::canonicalize(&path),
                ) {
                    (Ok(x), Ok(y)) => x == y,
                    _ => a == path.as_path(),
                };
                if !same(&b.read(cx).path) {
                    self.pending_open = Some(path);
                    window.focus(&self.open_focus);
                    cx.notify();
                }
            }
        }
    }

    fn accept_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.pending_open.take() else {
            return;
        };
        self.lock(window, cx);
        if let Screen::Unlock(u) = &self.screen {
            u.update(cx, |u, cx| u.set_path(path, window, cx));
        }
    }

    fn cancel_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_open = None;
        if let Screen::Vault(b) = &self.screen {
            window.focus(&b.focus_handle(cx));
        }
        cx.notify();
    }

    fn render_pending_open(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use theme::*;
        let path = self.pending_open.as_ref()?;
        let name = path
            .file_name()
            .map(|n| keywhere_core::display_safe(&n.to_string_lossy()).into_owned())
            .unwrap_or_default();
        let btn = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(7.))
                .cursor_pointer()
                .text_size(px(12.))
                .when(primary, |d| {
                    d.bg(c(ACCENT))
                        .text_color(c(ACCENT_INK))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                })
                .when(!primary, |d| {
                    d.border_1()
                        .border_color(c(BORDER))
                        .bg(c(RAISED))
                        .text_color(c(TEXT_SOFT))
                        .hover(|d| d.bg(c(HOVER)))
                })
                .child(i18n::t(label))
        };
        Some(
            div()
                .id("pending-open")
                .absolute()
                .top(px(56.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w(px(420.))
                        .max_w_full()
                        .p_3()
                        .rounded(px(10.))
                        .bg(c(PANEL))
                        .border_1()
                        .border_color(ca(ACCENT, 0x88))
                        .shadow_lg()
                        .occlude()
                        .track_focus(&self.open_focus)
                        .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                            match ev.keystroke.key.as_str() {
                                "enter" => this.accept_open(window, cx),
                                "escape" => this.cancel_open(window, cx),
                                _ => return,
                            }
                            cx.stop_propagation();
                        }))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(c(TEXT))
                                .child(i18n::tf(
                                    "Open {name}? The vault that is open now will lock first.",
                                    &[("name", &name)],
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(btn("pending-open-yes", "Lock and open", true).on_click(
                                    cx.listener(|this, _, window, cx| this.accept_open(window, cx)),
                                ))
                                .child(btn("pending-open-no", "Cancel", false).on_click(
                                    cx.listener(|this, _, window, cx| this.cancel_open(window, cx)),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_menu_notice() -> Option<gpui::AnyElement> {
        use theme::*;
        if !MENU_NOTICE.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let btn = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(7.))
                .cursor_pointer()
                .text_size(px(12.))
                .when(primary, |d| {
                    d.bg(c(ACCENT))
                        .text_color(c(ACCENT_INK))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                })
                .when(!primary, |d| {
                    d.border_1()
                        .border_color(c(BORDER))
                        .bg(c(RAISED))
                        .text_color(c(TEXT_SOFT))
                        .hover(|d| d.bg(c(HOVER)))
                })
                .child(i18n::t(label))
        };
        let hide = || MENU_NOTICE.store(false, std::sync::atomic::Ordering::Relaxed);
        Some(
            div()
                .id("menu-notice")
                .absolute()
                .bottom(px(44.))
                .left(px(16.))
                .w(px(340.))
                .max_w_full()
                .p_3()
                .rounded(px(10.))
                .bg(c(PANEL))
                .border_1()
                .border_color(ca(ACCENT, 0x88))
                .shadow_lg()
                .occlude()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_size(px(12.5)).text_color(c(TEXT)).child(i18n::t(
                    "Keywhere added itself to your app menu, so you can pin it to the task bar and open .psafe3 files with it.",
                )))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(btn("menu-notice-ok", "OK", true).on_click(move |_, window, _| {
                            hide();
                            window.refresh();
                        }))
                        .child(btn("menu-notice-undo", "Undo", false).on_click(
                            move |_, window, cx| {
                                hide();
                                #[cfg(target_os = "linux")]
                                cx.background_executor()
                                    .spawn(async { sys::integrate::remove() })
                                    .detach();
                                #[cfg(not(target_os = "linux"))]
                                let _ = cx;
                                window.refresh();
                            },
                        )),
                )
                .into_any_element(),
        )
    }

    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(b) = &self.screen else {
            return;
        };
        let path = b.update(cx, |b, cx| {
            b.clear_clipboard_if_ours(cx);
            b.path.clone()
        });
        let unlock = Self::unlock_view(Some(path), window, cx);
        // Locked (idle, screen lock, Ctrl+L) while "Open …?" was asking:
        // the vault is locked now, so open the file, as when it arrives
        // while locked.
        if let Some(p) = self.pending_open.take() {
            unlock.update(cx, |u, cx| u.set_path(p, window, cx));
        }
        self.screen = Screen::Unlock(unlock);
        set_tray(sys::TrayState::Locked, cx);
        cx.notify();
        window.text_system().clear_line_layout_cache();
        secret_text::sweep();
        Self::flush_frames(window, cx);
    }

    /// GPUI keeps the previous frame's text layouts (which may contain a
    /// revealed password) until two more frames are drawn, and an idle app
    /// draws none. Force them, so the old layouts are freed — and zeroed by
    /// the global allocator — then sweep the secret display strings.
    fn flush_frames(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |_, cx| {
            for _ in 0..3 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(60))
                    .await;
                if cx.update(|window, _| window.refresh()).is_err() {
                    return;
                }
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(60))
                .await;
            let _ = cx.update(|window, _| window.text_system().clear_line_layout_cache());
            secret_text::sweep();
        })
        .detach();
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .size_full()
            .on_action(cx.listener(|this, _: &chrome::CloseWindow, window, cx| {
                this.close_window(window, cx)
            }))
            .font_family(assets::UI_FONT)
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.last_input = Instant::now()))
            .on_mouse_move(cx.listener(|this, _, _, _| this.last_input = Instant::now()))
            .capture_key_down(cx.listener(|this, _, _, _| this.last_input = Instant::now()))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .relative()
            .child(match &self.screen {
                Screen::Unlock(v) => v.clone().into_any_element(),
                Screen::Vault(b) => b.clone().into_any_element(),
            })
            .children(update_view::toast(cx))
            .children(self.render_pending_open(cx))
            .children(Self::render_menu_notice())
            .when_some(self.settings.clone(), |d, v| {
                d.child(
                    div()
                        .id("settings-backdrop")
                        .absolute()
                        .inset_0()
                        .rounded(chrome::corner_radius(window))
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_4()
                        .bg(theme::ca(0x050B1A, 0xB0))
                        .occlude()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
                        )
                        .child(v),
                )
            });
        chrome::frame(window, content)
    }
}

pub fn harden_process() {
    // No core dumps of a process that holds decrypted secrets.
    #[cfg(unix)]
    unsafe {
        let zero = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
        #[cfg(target_os = "linux")]
        if !dev_memory_test() {
            libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        }
        // No debugger may attach (release builds; it would also stop
        // developers from debugging).
        #[cfg(all(target_os = "macos", not(debug_assertions)))]
        libc::ptrace(libc::PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0);
    }
    #[cfg(windows)]
    sys::windows::harden();
}

/// DEV ONLY: the RAM-residue test harness needs to read our memory, which
/// PR_SET_DUMPABLE=0 forbids. Exists only with the `devtools` feature; the
/// release binary always hardens.
#[cfg(target_os = "linux")]
fn dev_memory_test() -> bool {
    #[cfg(feature = "devtools")]
    {
        std::env::var_os("KEYWHERE_DEV_MEMORY_TEST").is_some()
    }
    #[cfg(not(feature = "devtools"))]
    {
        false
    }
}

/// Run the app.
/// `file`: a vault given on the command line (file manager, "Open with");
/// without one, the last vault is used.
pub fn run(file: Option<PathBuf>) {
    // Second start (desktop shortcut `keywhere --show`, or a vault opened
    // from the file manager): hand over to the running instance and exit.
    #[cfg(unix)]
    {
        let cmd = match &file {
            Some(p) => match sys::instance::open_command(p) {
                Some(c) => c,
                None => {
                    eprintln!(
                        "keywhere: can't hand {} to the running instance",
                        p.display()
                    );
                    sys::instance::show_command()
                }
            },
            None => sys::instance::show_command(),
        };
        if sys::instance::forward_to_running(&cmd) {
            return;
        }
    }
    run_inner(file.or_else(config::last_vault), None, None)
}

/// Start after an in-app update. Never hand off to a running instance: if
/// one still answers, it is the old version and this start must not silently
/// disappear into it.
pub fn run_after_update(path: Option<PathBuf>) {
    update::cleanup_leftovers();
    #[cfg(unix)]
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        while sys::instance::is_running() {
            if Instant::now() >= deadline {
                eprintln!("Keywhere was updated, but the old version is still running. Quit it and start Keywhere again.");
                std::process::exit(1);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    run_inner(path, None, None)
}

/// DEV ONLY: master password to "type" into the real unlock field (RAM test).
#[cfg(feature = "devtools")]
static DEV_TYPED: std::sync::Mutex<Option<zeroize::Zeroizing<String>>> =
    std::sync::Mutex::new(None);

/// DEV ONLY (feature `devtools`): start locked and unlock through the real
/// UI path (password field → Unlock), for the RAM-residue test.
#[cfg(feature = "devtools")]
pub fn run_unlock_via_ui(path: PathBuf, password: zeroize::Zeroizing<String>) {
    *DEV_TYPED.lock().unwrap_or_else(|e| e.into_inner()) = Some(password);
    #[cfg(unix)]
    sys::instance::use_dev_socket();
    run_inner(Some(path), None, None)
}

/// DEV ONLY (feature `devtools`): start with an already unlocked demo vault so
/// screenshots can be automated. Not compiled into release builds by default.
#[cfg(feature = "devtools")]
pub fn run_preunlocked(path: PathBuf, vault: Vault, select: Option<String>) {
    #[cfg(unix)]
    sys::instance::use_dev_socket();
    run_inner(Some(path), Some((vault, None)), select)
}

fn run_inner(path: Option<PathBuf>, preunlocked: Preunlocked, select: Option<String>) {
    harden_process();
    let app = Application::new().with_assets(assets::Assets);
    // macOS: clicking the Dock icon with no window open brings it back,
    // like the tray's "Open Keywhere".
    app.on_reopen(|cx| handle_sys_event(sys::SysEvent::Show(None), cx));
    app.run(move |cx: &mut App| {
        assets::load_fonts(cx);
        app_icon::install();
        i18n::apply_setting(settings::get().language);
        use browser::*;
        cx.bind_keys([
            KeyBinding::new("secondary-l", Lock, None),
            KeyBinding::new("secondary-f", FocusSearch, Some("Browser")),
            KeyBinding::new("secondary-k", FocusSearch, Some("Browser")),
            KeyBinding::new("down", SelectNext, Some("Browser")),
            KeyBinding::new("up", SelectPrev, Some("Browser")),
            KeyBinding::new("secondary-c", CopyPassword, Some("Browser && !TextField")),
            KeyBinding::new("secondary-b", CopyUsername, Some("Browser && !TextField")),
            KeyBinding::new("secondary-u", OpenUrl, Some("Browser")),
            KeyBinding::new("secondary-r", ToggleReveal, Some("Browser")),
            KeyBinding::new("secondary-n", NewEntry, Some("Browser")),
            KeyBinding::new("secondary-e", EditEntry, Some("Browser")),
            KeyBinding::new("secondary-d", DuplicateEntry, Some("Browser")),
            KeyBinding::new(
                "delete",
                DeleteEntry,
                Some("Browser && !TextField && !Editor"),
            ),
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-,", OpenSettings, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let (tx, mut rx) = sys::channel();
        // Dev "copy:" runs still listen so tests can send `lock`.
        #[cfg(unix)]
        if select.as_deref().is_none_or(|s| s.starts_with("copy:")) || cfg!(feature = "devtools") {
            sys::instance::listen(tx.clone());
        }
        sys::watch_session(tx.clone());
        #[cfg(windows)]
        sys::windows::refresh_installed_version();
        let tray = sys::Tray::start(tx);
        cx.set_global(AppState {
            path: path.clone(),
            tray,
            window: None,
            hidden: None,
        });
        cx.on_app_quit(|cx| {
            drop_hidden_session(cx);
            #[cfg(unix)]
            sys::instance::cleanup();
            async {}
        })
        .detach();

        // Dev/test runs (screenshots, RAM test) never touch the network.
        let auto_update_check = select.is_none()
            && preunlocked.is_none()
            && !cfg!(feature = "devtools")
            && settings::get().check_updates == Some(true);
        open_main_window(path, preunlocked, select, cx);
        if auto_update_check {
            update::start_check(false, cx);
        }

        // AppImage: menu entry for the icon, pinning and "Open with".
        #[cfg(target_os = "linux")]
        if !cfg!(feature = "devtools") {
            let task = cx
                .background_executor()
                .spawn(async { sys::integrate::integrate() });
            cx.spawn(async move |cx| {
                if task.await == sys::integrate::Outcome::Created {
                    MENU_NOTICE.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = cx.update(|cx| cx.refresh_windows());
                }
            })
            .detach();
        }

        // A hidden (unlocked) session still auto-locks after IDLE_LOCK.
        cx.spawn(async move |cx| loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let expired = cx.update(|cx| {
                let expired = cx
                    .global::<AppState>()
                    .hidden
                    .as_ref()
                    .is_some_and(|h| idle_lock().is_some_and(|d| h.since.elapsed() >= d));
                if expired {
                    drop_hidden_session(cx);
                }
            });
            if expired.is_err() {
                break;
            }
        })
        .detach();

        cx.spawn(async move |cx| {
            while let Some(ev) = rx.next().await {
                if cx.update(|cx| handle_sys_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}

fn open_main_window(
    path: Option<PathBuf>,
    preunlocked: Preunlocked,
    select: Option<String>,
    cx: &mut App,
) {
    let bounds = Bounds::centered(None, size(px(1120.), px(720.)), cx);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("keywhere".into()),
                // Our own title bar (see chrome.rs). macOS keeps native traffic lights.
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Keywhere".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(gpui::point(px(14.), px(14.))),
                }),
                window_decorations: Some(gpui::WindowDecorations::Client),
                // The resize/shadow margin around the rounded frame must be
                // see-through, not painted opaque by the compositor.
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                window_min_size: Some(size(px(820.), px(520.))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Shell::new(path, preunlocked, select, window, cx)),
        )
        .expect("open window");
    cx.global_mut::<AppState>().window = Some(handle);
    cx.activate(true);
}

fn handle_sys_event(ev: sys::SysEvent, cx: &mut App) {
    let window = cx.global::<AppState>().window;
    match ev {
        sys::SysEvent::Open(path, token) => {
            // Only regular files; anything else just shows the window.
            let file = std::fs::metadata(&path).is_ok_and(|m| m.is_file());
            handle_sys_event(sys::SysEvent::Show(token), cx);
            if file {
                if let Some(w) = cx.global::<AppState>().window {
                    let _ = w.update(cx, |shell, window, cx| shell.request_open(path, window, cx));
                }
            }
        }
        sys::SysEvent::Show(token) => {
            let shown = window.is_some_and(|w| {
                w.update(cx, |shell, window, cx| {
                    // Wayland: use the tray/launcher token so the compositor
                    // really raises us (GPUI's own request gets rejected).
                    #[cfg(target_os = "linux")]
                    let raised = token
                        .as_deref()
                        .is_some_and(|t| sys::wayland::activate_with_token(window, t));
                    #[cfg(not(target_os = "linux"))]
                    let raised = {
                        let _ = &token;
                        false
                    };
                    if !raised {
                        window.activate_window();
                    }
                    shell.focus_primary(window, cx);
                })
                .is_ok()
            });
            if !shown {
                // Window was hidden to the tray: reopen it, still unlocked if
                // the background session hasn't timed out, else locked.
                match cx.global_mut::<AppState>().hidden.take() {
                    Some(h) => open_main_window(Some(h.path), Some((h.vault, h.lock)), None, cx),
                    None => {
                        let path = cx
                            .global::<AppState>()
                            .path
                            .clone()
                            .or_else(config::last_vault);
                        open_main_window(path, None, None, cx);
                    }
                }
                // Raise the new window with the token once it is mapped.
                #[cfg(target_os = "linux")]
                if let (Some(t), Some(w)) = (token, cx.global::<AppState>().window) {
                    cx.spawn(async move |cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(150))
                            .await;
                        let _ = w.update(cx, |_, window, _| {
                            if !sys::wayland::activate_with_token(window, &t) {
                                window.activate_window();
                            }
                        });
                    })
                    .detach();
                }
            }
        }
        sys::SysEvent::Lock => {
            drop_hidden_session(cx);
            if let Some(w) = window {
                let _ = w.update(cx, |shell, window, cx| shell.lock(window, cx));
            }
        }
        sys::SysEvent::LockForSleep => {
            drop_hidden_session(cx);
            if let Some(w) = window {
                let _ = w.update(cx, |shell, window, cx| {
                    shell.lock(window, cx);
                    // Paint the locked screen now so no secret frame is in
                    // the GPU buffers when RAM is frozen.
                    window.refresh();
                });
            }
            secret_text::sweep();
            sys::ack_lock();
        }
        sys::SysEvent::Hide => {
            if let Some(w) = window {
                let _ = w.update(cx, |shell, window, cx| shell.close_window(window, cx));
            }
        }
        sys::SysEvent::Quit => cx.quit(),
    }
}

gpui::actions!(keywhere, [Quit, OpenSettings]);

/// The AppImage just added itself to the app menu: say so once, with Undo.
static MENU_NOTICE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// "Update and restart": download and verify the new version, then lock,
/// start the new version and quit.
pub(crate) fn install_update(cx: &mut App) {
    update::start_install(cx, |cx, target| {
        if let Some(handle) = cx.global::<AppState>().window {
            let _ = handle.update(cx, |shell, window, cx| shell.lock(window, cx));
        }
        drop_hidden_session(cx);
        // Let lock's frame flush (~250 ms) free and zero the old text
        // layouts before the process goes away, then sweep once more.
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = cx.update(|cx| {
                if let Some(handle) = cx.global::<AppState>().window {
                    let _ = handle.update(cx, |_, window, _| {
                        window.text_system().clear_line_layout_cache()
                    });
                }
                secret_text::sweep();
                match update::relaunch(&target) {
                    Ok(()) => cx.quit(),
                    Err(e) => {
                        update::set_failed(update::UpdateError::Install(e.to_string()));
                        cx.refresh_windows();
                    }
                }
            });
        })
        .detach();
    });
}

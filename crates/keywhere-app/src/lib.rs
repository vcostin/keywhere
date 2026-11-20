//! Keywhere desktop app (GPUI) as a library, so dev tools/examples can drive it.

pub mod assets;
pub mod browser;
pub mod chrome;
pub mod config;
pub mod editor;
pub mod field;
pub mod model;
pub mod secret_text;
pub mod sys;
pub mod theme;
pub mod unlock;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    div, prelude::*, px, size, App, Application, Bounds, Context, Entity, Focusable, Global,
    KeyBinding, Window, WindowBounds, WindowHandle, WindowOptions,
};
use keywhere_core::Vault;

use browser::{Browser, BrowserEvent};
use unlock::{UnlockEvent, UnlockView};

/// Lock after this much time without keyboard or mouse input.
const IDLE_LOCK: Duration = Duration::from_secs(5 * 60);

/// App-wide state that outlives any one window (the tray keeps the app
/// running after the window is closed).
struct AppState {
    path: Option<PathBuf>,
    tray: sys::Tray,
    window: Option<WindowHandle<Shell>>,
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
    last_input: Instant,
    _idle: gpui::Task<()>,
    _on_quit: gpui::Subscription,
}

impl Shell {
    fn new(
        path: Option<PathBuf>,
        preunlocked: Option<Vault>,
        select: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let idle = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            secret_text::sweep();
            let r = this.update_in(cx, |this, window, cx| {
                if matches!(this.screen, Screen::Vault(_)) && this.last_input.elapsed() >= IDLE_LOCK
                {
                    this.lock(window, cx);
                }
            });
            if r.is_err() {
                break;
            }
        });
        let mut this = Self {
            screen: Screen::Unlock(Self::unlock_view(path.clone(), window, cx)),
            last_input: Instant::now(),
            _idle: idle,
            // Best-effort final wipe of secret display strings on exit.
            _on_quit: cx.on_app_quit(|_, _| {
                secret_text::sweep();
                async {}
            }),
        };
        if let (Some(v), Some(p)) = (preunlocked, path) {
            this.show_vault(p, v, window, cx);
            // Dev screenshots: "Title", "edit:Title", "delete:Title", "copy:Title" or "new".
            if let (Some(t), Screen::Vault(b)) = (select, &this.screen) {
                b.update(cx, |b, cx| match t.split_once(':') {
                    Some(("delete", title)) => {
                        b.select_title(title, cx);
                        b.delete(window, cx);
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
                let (vault, path) = view.update(cx, |v, _| (v.result.take(), v.path.clone()));
                if let (Some(vault), Some(path)) = (vault, path) {
                    config::remember_vault(&path);
                    if cx.has_global::<AppState>() {
                        cx.global_mut::<AppState>().path = Some(path.clone());
                    }
                    this.show_vault(path, vault, window, cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let b = cx.new(|cx| Browser::new(path, vault, window, cx));
        cx.subscribe_in(&b, window, |this, _, ev, window, cx| match ev {
            BrowserEvent::LockRequested => this.lock(window, cx),
        })
        .detach();
        self.screen = Screen::Vault(b);
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

    /// Window close button: always locks. With a tray the window is removed
    /// entirely (no taskbar entry) and the app keeps running in the tray;
    /// tray → Open / `keywhere --show` creates a fresh, locked window.
    /// Without a tray: quit. (Relies on the vendored gpui keep-alive patch.)
    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tray = cx.has_global::<AppState>() && cx.global::<AppState>().tray.available();
        self.lock(window, cx);
        if tray {
            cx.global_mut::<AppState>().window = None;
            window.remove_window();
        } else {
            cx.quit();
        }
    }

    /// Drop the vault (zeroizing all decrypted fields) and return to unlock.
    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(b) = &self.screen else {
            return;
        };
        let path = b.update(cx, |b, cx| {
            b.clear_clipboard_if_ours(cx);
            b.path.clone()
        });
        self.screen = Screen::Unlock(Self::unlock_view(Some(path), window, cx));
        set_tray(sys::TrayState::Locked, cx);
        cx.notify();
        // Wipe what we can now; the rest goes on the next ticks once GPUI
        // drops the old frame's text.
        secret_text::sweep();
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
            .child(match &self.screen {
                Screen::Unlock(v) => v.clone().into_any_element(),
                Screen::Vault(b) => b.clone().into_any_element(),
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
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

/// Run the app.
pub fn run(path: Option<PathBuf>) {
    // Second start (e.g. a desktop shortcut running `keywhere --show`): hand
    // over to the running instance and exit.
    #[cfg(unix)]
    if sys::instance::forward_to_running(&sys::instance::show_command()) {
        return;
    }
    run_inner(path, None, None)
}

/// DEV ONLY (feature `devtools`): start with an already unlocked demo vault so
/// screenshots can be automated. Not compiled into release builds by default.
#[cfg(feature = "devtools")]
pub fn run_preunlocked(path: PathBuf, vault: Vault, select: Option<String>) {
    run_inner(Some(path), Some(vault), select)
}

fn run_inner(path: Option<PathBuf>, preunlocked: Option<Vault>, select: Option<String>) {
    harden_process();
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx);
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
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            let (tx, mut rx) = sys::channel();
            // Dev "copy:" runs still listen so tests can send `lock`.
            #[cfg(unix)]
            if select.as_deref().is_none_or(|s| s.starts_with("copy:")) {
                sys::instance::listen(tx.clone());
            }
            sys::watch_session(tx.clone());
            let tray = sys::Tray::start(tx);
            cx.set_global(AppState {
                path: path.clone(),
                tray,
                window: None,
            });
            cx.on_app_quit(|_| {
                #[cfg(unix)]
                sys::instance::cleanup();
                async {}
            })
            .detach();

            open_main_window(path, preunlocked, select, cx);

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
    preunlocked: Option<Vault>,
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
                // Window was closed to the tray: open a fresh (locked) one.
                let path = cx
                    .global::<AppState>()
                    .path
                    .clone()
                    .or_else(config::last_vault);
                open_main_window(path, None, None, cx);
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
            if let Some(w) = window {
                let _ = w.update(cx, |shell, window, cx| shell.lock(window, cx));
            }
        }
        sys::SysEvent::Quit => cx.quit(),
    }
}

gpui::actions!(keywhere, [Quit]);

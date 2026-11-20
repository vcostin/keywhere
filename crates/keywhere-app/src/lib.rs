//! Keywhere desktop app (GPUI) as a library, so dev tools/examples can drive it.

pub mod assets;
pub mod browser;
pub mod config;
pub mod field;
pub mod model;
pub mod secret_text;
pub mod theme;
pub mod unlock;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use keywhere_core::Vault;
use gpui::{
    div, prelude::*, px, size, App, Application, Bounds, Context, Entity, KeyBinding, Window,
    WindowBounds, WindowOptions,
};

use browser::{Browser, BrowserEvent};
use unlock::{UnlockEvent, UnlockView};

/// Lock after this much time without keyboard or mouse input.
const IDLE_LOCK: Duration = Duration::from_secs(5 * 60);

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
                if matches!(this.screen, Screen::Vault(_)) && this.last_input.elapsed() >= IDLE_LOCK {
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
            if let (Some(t), Screen::Vault(b)) = (select, &this.screen) {
                b.update(cx, |b, cx| b.select_title(&t, cx));
            }
        }
        this
    }

    fn unlock_view(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Entity<UnlockView> {
        let view = cx.new(|cx| UnlockView::new(path, window, cx));
        cx.subscribe_in(&view, window, |this, view, ev, window, cx| match ev {
            UnlockEvent::Unlocked => {
                let (vault, path) = view.update(cx, |v, _| (v.result.take(), v.path.clone()));
                if let (Some(vault), Some(path)) = (vault, path) {
                    config::remember_vault(&path);
                    this.show_vault(path, vault, window, cx);
                }
            }
        })
        .detach();
        view
    }

    fn show_vault(&mut self, path: PathBuf, vault: Vault, window: &mut Window, cx: &mut Context<Self>) {
        let b = cx.new(|cx| Browser::new(path, vault, window, cx));
        cx.subscribe_in(&b, window, |this, _, ev, window, cx| match ev {
            BrowserEvent::LockRequested => this.lock(window, cx),
        })
        .detach();
        self.screen = Screen::Vault(b);
        self.last_input = Instant::now();
        cx.notify();
    }

    /// Drop the vault (zeroizing all decrypted fields) and return to unlock.
    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(b) = &self.screen else { return };
        let path = b.update(cx, |b, cx| {
            b.clear_clipboard_if_ours(cx);
            b.path.clone()
        });
        self.screen = Screen::Unlock(Self::unlock_view(Some(path), window, cx));
        cx.notify();
        // Wipe what we can now; the rest goes on the next ticks once GPUI
        // drops the old frame's text.
        secret_text::sweep();
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family(assets::UI_FONT)
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.last_input = Instant::now()))
            .on_mouse_move(cx.listener(|this, _, _, _| this.last_input = Instant::now()))
            .capture_key_down(cx.listener(|this, _, _, _| this.last_input = Instant::now()))
            .child(match &self.screen {
                Screen::Unlock(v) => v.clone().into_any_element(),
                Screen::Vault(b) => b.clone().into_any_element(),
            })
    }
}

pub fn harden_process() {
    // No core dumps of a process that holds decrypted secrets.
    #[cfg(unix)]
    unsafe {
        let zero = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
        #[cfg(target_os = "linux")]
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

/// Run the app.
pub fn run(path: Option<PathBuf>) {
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
    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        assets::load_fonts(cx);
        use browser::*;
        cx.bind_keys([
            KeyBinding::new("secondary-l", Lock, None),
            KeyBinding::new("secondary-f", FocusSearch, Some("Browser")),
            KeyBinding::new("secondary-k", FocusSearch, Some("Browser")),
            KeyBinding::new("down", SelectNext, Some("Browser")),
            KeyBinding::new("up", SelectPrev, Some("Browser")),
            KeyBinding::new("secondary-c", CopyPassword, Some("Browser")),
            KeyBinding::new("secondary-b", CopyUsername, Some("Browser")),
            KeyBinding::new("secondary-u", OpenUrl, Some("Browser")),
            KeyBinding::new("secondary-r", ToggleReveal, Some("Browser")),
            KeyBinding::new("secondary-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(1120.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("keywhere".into()),
                titlebar: Some(gpui::TitlebarOptions { title: Some("Keywhere".into()), ..Default::default() }),
                window_min_size: Some(size(px(820.), px(520.))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Shell::new(path, preunlocked, select, window, cx)),
        )
        .expect("open window");
        cx.activate(true);
    });
}

gpui::actions!(keywhere, [Quit]);

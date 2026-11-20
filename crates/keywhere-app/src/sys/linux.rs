//! Linux: StatusNotifierItem tray (KDE, GNOME with AppIndicator, ...) and
//! session watchers over D-Bus (no GTK).

use std::thread;

use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};

use super::{EventTx, SysEvent, TrayState};

pub type TrayHandle = ksni::blocking::Handle<KwTray>;

pub struct KwTray {
    tx: EventTx,
    state: TrayState,
}

const ICONS: [(i32, &[u8]); 4] = [
    (
        22,
        include_bytes!("../../../../assets/tray/keywhere-22.argb"),
    ),
    (
        32,
        include_bytes!("../../../../assets/tray/keywhere-32.argb"),
    ),
    (
        48,
        include_bytes!("../../../../assets/tray/keywhere-48.argb"),
    ),
    (
        64,
        include_bytes!("../../../../assets/tray/keywhere-64.argb"),
    ),
];

impl KwTray {
    fn send(&self, ev: SysEvent) {
        let _ = self.tx.unbounded_send(ev);
    }
}

impl ksni::Tray for KwTray {
    fn id(&self) -> String {
        "keywhere".into()
    }

    fn title(&self) -> String {
        "Keywhere".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        ICONS
            .iter()
            .map(|(s, data)| ksni::Icon {
                width: *s,
                height: *s,
                data: data.to_vec(),
            })
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Keywhere".into(),
            description: match self.state {
                TrayState::Locked => "Vault locked".into(),
                TrayState::Unlocked => "Vault unlocked".into(),
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(SysEvent::Show(activation_token()));
    }

    /// Deliberately minimal: no entry titles or search in the tray, so a
    /// glance at the panel never reveals vault content.
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let unlocked = self.state == TrayState::Unlocked;
        vec![
            StandardItem {
                label: "Open Keywhere".into(),
                activate: Box::new(|t: &mut Self| t.send(SysEvent::Show(activation_token()))),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Lock".into(),
                enabled: unlocked,
                icon_name: "system-lock-screen".into(),
                activate: Box::new(|t: &mut Self| t.send(SysEvent::Lock)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| t.send(SysEvent::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The XDG activation token for the tray event being handled. KDE may
/// deliver it slightly after the click, so retry briefly.
fn activation_token() -> Option<String> {
    for _ in 0..50 {
        if let Some(t) = ksni::current_xdg_activation_token() {
            return Some(t);
        }
        thread::sleep(std::time::Duration::from_millis(2));
    }
    None
}

pub fn start_tray(tx: EventTx) -> Option<TrayHandle> {
    KwTray {
        tx,
        state: TrayState::Locked,
    }
    .spawn()
    .ok()
}

pub fn set_tray_state(h: &TrayHandle, state: TrayState) {
    h.update(|t| t.state = state);
}

/// Lock when the system is about to sleep, the logind session is locked, or
/// the desktop screen saver/locker activates.
pub fn watch_session(tx: EventTx) {
    let t = tx.clone();
    let _ = thread::Builder::new()
        .name("keywhere-logind".into())
        .spawn(move || {
            if let Err(e) = watch_logind(&t) {
                eprintln!("keywhere: logind watcher unavailable: {e}");
            }
        });
    let _ = thread::Builder::new()
        .name("keywhere-screensaver".into())
        .spawn(move || {
            if let Err(e) = watch_screensaver(&tx) {
                eprintln!("keywhere: screen-saver watcher unavailable: {e}");
            }
        });
}

fn signal_rule<'a>(interface: &'a str, member: &'a str) -> zbus::Result<zbus::MatchRule<'a>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(interface)?
        .member(member)?
        .build())
}

fn watch_logind(tx: &EventTx) -> zbus::Result<()> {
    use zbus::blocking::{Connection, MessageIterator};
    let conn = Connection::system()?;
    let sleep = MessageIterator::for_match_rule(
        signal_rule("org.freedesktop.login1.Manager", "PrepareForSleep")?,
        &conn,
        None,
    )?;
    // Our own session's object path, so another user's lock doesn't lock us.
    // (Not every process is in a logind session; sleep still applies.)
    let session: zbus::Result<zbus::zvariant::OwnedObjectPath> = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetSessionByPID",
            &(std::process::id()),
        )
        .and_then(|m| m.body().deserialize());
    match session {
        Ok(session) => {
            let lock = MessageIterator::for_match_rule(
                zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.freedesktop.login1.Session")?
                    .member("Lock")?
                    .path(session.as_ref())?
                    .build(),
                &conn,
                None,
            )?;
            let t = tx.clone();
            thread::spawn(move || {
                for _msg in lock.flatten() {
                    if t.unbounded_send(SysEvent::Lock).is_err() {
                        break;
                    }
                }
            });
        }
        Err(e) => eprintln!("keywhere: no logind session ({e}); watching sleep only"),
    }
    for msg in sleep.flatten() {
        if let Ok(true) = msg.body().deserialize::<bool>() {
            if tx.unbounded_send(SysEvent::Lock).is_err() {
                break;
            }
        }
    }
    Ok(())
}

fn watch_screensaver(tx: &EventTx) -> zbus::Result<()> {
    use zbus::blocking::{Connection, MessageIterator};
    let conn = Connection::session()?;
    let it = MessageIterator::for_match_rule(
        signal_rule("org.freedesktop.ScreenSaver", "ActiveChanged")?,
        &conn,
        None,
    )?;
    // GNOME uses its own interface name for the same signal.
    let gnome = MessageIterator::for_match_rule(
        signal_rule("org.gnome.ScreenSaver", "ActiveChanged")?,
        &conn,
        None,
    )?;
    let t = tx.clone();
    thread::spawn(move || {
        for msg in gnome.flatten() {
            if let Ok(true) = msg.body().deserialize::<bool>() {
                if t.unbounded_send(SysEvent::Lock).is_err() {
                    break;
                }
            }
        }
    });
    for msg in it.flatten() {
        if let Ok(true) = msg.body().deserialize::<bool>() {
            if tx.unbounded_send(SysEvent::Lock).is_err() {
                break;
            }
        }
    }
    Ok(())
}

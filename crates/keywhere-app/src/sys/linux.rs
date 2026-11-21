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
    let session = own_session(&conn);
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
        Err(e) => {
            eprintln!("keywhere: no logind session ({e}); watching sleep and the screen saver only")
        }
    }
    // Delay inhibitor: logind waits (up to InhibitDelayMaxSec, default 5 s)
    // until we release it, so the vault is locked *before* RAM is frozen or
    // written to a hibernation image. Re-armed after every resume.
    let mut inhibitor = take_sleep_inhibitor(&conn);
    for msg in sleep.flatten() {
        match msg.body().deserialize::<bool>() {
            Ok(true) => {
                let before = super::lock_ack();
                if tx.unbounded_send(SysEvent::LockForSleep).is_err() {
                    break;
                }
                // Wait for the app to confirm the lock (and draw one
                // non-secret frame), then let the system sleep.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while super::lock_ack() == before && std::time::Instant::now() < deadline {
                    thread::sleep(std::time::Duration::from_millis(20));
                }
                thread::sleep(std::time::Duration::from_millis(150));
                inhibitor = None; // closing the fd releases the inhibitor
            }
            Ok(false) => {
                if inhibitor.is_none() {
                    inhibitor = take_sleep_inhibitor(&conn);
                }
            }
            Err(_) => {}
        }
    }
    drop(inhibitor);
    Ok(())
}

/// The logind session we belong to. Desktops that start apps as their own
/// systemd user service (KDE does) leave the process outside any session, so
/// `GetSessionByPID` fails; then use `$XDG_SESSION_ID`, then the user's
/// graphical ("display") session. With several graphical sessions of the
/// same user (multi-seat), that may be a different one than ours; an extra
/// lock is harmless, and sleep and the screen saver are still watched.
fn own_session(conn: &zbus::blocking::Connection) -> zbus::Result<zbus::zvariant::OwnedObjectPath> {
    use zbus::zvariant::OwnedObjectPath;
    let by_pid = manager(conn, "GetSessionByPID", &std::process::id());
    if by_pid.is_ok() {
        return by_pid;
    }
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        if let Ok(p) = manager(conn, "GetSession", &id) {
            return Ok(p);
        }
    }
    // org.freedesktop.login1.User.Display: (session id, object path).
    let user: OwnedObjectPath = manager(conn, "GetUser", &unsafe { libc::getuid() })?;
    let display: zbus::zvariant::OwnedValue = conn
        .call_method(
            Some("org.freedesktop.login1"),
            user.as_ref(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.User", "Display"),
        )?
        .body()
        .deserialize()?;
    let (_, path): (String, OwnedObjectPath) = display.try_into()?;
    if path.as_str() == "/" {
        return by_pid; // no graphical session: report the original error
    }
    Ok(path)
}

/// Call a login1 Manager method that returns an object path.
fn manager<B>(
    conn: &zbus::blocking::Connection,
    method: &str,
    body: &B,
) -> zbus::Result<zbus::zvariant::OwnedObjectPath>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        method,
        body,
    )?
    .body()
    .deserialize()
}

/// logind `Inhibit("sleep", …, "delay")`; the returned fd holds the lock.
fn take_sleep_inhibitor(conn: &zbus::blocking::Connection) -> Option<zbus::zvariant::OwnedFd> {
    conn.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        "Inhibit",
        &(
            "sleep",
            "Keywhere",
            "Locking the password vault before sleep",
            "delay",
        ),
    )
    .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedFd>())
    .map_err(|e| eprintln!("keywhere: no sleep inhibitor ({e}); locking without delay"))
    .ok()
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

#[cfg(test)]
mod tests {
    /// Outside a logind session (as when KDE starts us as a user service),
    /// the user's graphical session is still found. Skipped where there is
    /// no system bus or no graphical session (CI).
    #[test]
    fn finds_session_outside_session_scope() {
        let Ok(conn) = zbus::blocking::Connection::system() else {
            return;
        };
        let pid = super::manager(&conn, "GetSessionByPID", &std::process::id());
        let user = super::manager(&conn, "GetUser", &unsafe { libc::getuid() });
        let found = super::own_session(&conn);
        eprintln!("by pid: {pid:?}\nuser: {user:?}\nfound: {found:?}");
        if pid.is_ok() {
            assert_eq!(found.ok(), pid.ok());
        } else if user.is_ok() && std::env::var_os("WAYLAND_DISPLAY").is_some() {
            assert!(found.is_ok(), "graphical session not found: {found:?}");
        }
    }
}

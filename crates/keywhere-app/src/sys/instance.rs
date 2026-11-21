//! Single instance + `keywhere --show` for desktop shortcuts.
//!
//! The first instance listens on `$XDG_RUNTIME_DIR/keywhere.sock`, or, if that
//! is unavailable or too long for a socket path, on `keywhere.sock` inside a
//! per-uid directory in the temp dir. A second start connects, sends one
//! command and exits. Only the fixed commands `show` / `lock` are understood.
//!
//! Hardening against other local users:
//! * the socket directory must be a real directory (not a symlink) owned by
//!   us with no group/other permissions, otherwise single-instance is off;
//! * before connecting, the socket file itself must be owned by us;
//! * every accepted connection's peer uid (SO_PEERCRED / getpeereid) must be
//!   ours, otherwise it is dropped without reading.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{EventTx, SysEvent};

fn valid_token(t: &str) -> bool {
    (1..=256).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_graphic())
}

/// `show` plus the launcher's activation token (KDE/GNOME set
/// `XDG_ACTIVATION_TOKEN` when running a shortcut command).
pub fn show_command() -> String {
    match std::env::var("XDG_ACTIVATION_TOKEN") {
        Ok(t) if valid_token(&t) => format!("show {t}"),
        _ => "show".into(),
    }
}

static DEV: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Dev/test instances (pre-unlocked screenshot harness) use their own socket
/// so they can never talk to — or be driven instead of — the real app.
pub fn use_dev_socket() {
    DEV.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn socket_name() -> &'static str {
    if DEV.load(std::sync::atomic::Ordering::Relaxed) {
        "keywhere-dev.sock"
    } else {
        "keywhere.sock"
    }
}

/// Unix socket paths are limited to ~108 bytes (sun_path).
const MAX_SOCKET_PATH: usize = 100;

fn my_uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

/// A directory we may put our socket in: real dir, ours, mode xx00.
fn trusted_dir(dir: &Path) -> bool {
    match std::fs::symlink_metadata(dir) {
        Ok(m) => m.file_type().is_dir() && m.uid() == my_uid() && m.mode() & 0o077 == 0,
        Err(_) => false,
    }
}

fn socket_path() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR") {
        let dir = PathBuf::from(d);
        let p = dir.join(socket_name());
        if p.as_os_str().len() <= MAX_SOCKET_PATH && trusted_dir(&dir) {
            return Some(p);
        }
    }
    let dir = std::env::temp_dir().join(format!("keywhere-{}", my_uid()));
    // Create with 0700 atomically; an existing dir must pass the same checks
    // (another user could have pre-created it).
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
    if !trusted_dir(&dir) {
        eprintln!(
            "keywhere: {} is not a private directory; single-instance disabled",
            dir.display()
        );
        return None;
    }
    let p = dir.join(socket_name());
    (p.as_os_str().len() <= MAX_SOCKET_PATH).then_some(p)
}

/// Is the peer on this connection running as our user?
fn peer_is_us(s: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let fd = s.as_raw_fd();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: u32::MAX,
            gid: u32::MAX,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: cred/len are valid for writes of the sizes given.
        let r = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        r == 0 && cred.uid == my_uid()
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let (mut uid, mut gid) = (u32::MAX, u32::MAX);
        // SAFETY: uid/gid are valid out-pointers.
        let r = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        r == 0 && uid == my_uid()
    }
}

/// If another instance of *ours* is running, hand it `cmd` and return true.
pub fn forward_to_running(cmd: &str) -> bool {
    let Some(path) = socket_path() else {
        return false;
    };
    // Don't talk to (or be stopped by) a socket someone else planted.
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.uid() == my_uid() => {}
        _ => return false,
    }
    let Ok(mut s) = UnixStream::connect(&path) else {
        return false;
    };
    if !peer_is_us(&s) {
        return false;
    }
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    s.write_all(format!("{cmd}\n").as_bytes()).is_ok()
}

/// Is another instance of ours answering on the socket? (Connects without
/// sending a command.)
pub fn is_running() -> bool {
    let Some(path) = socket_path() else {
        return false;
    };
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.uid() == my_uid() => {}
        _ => return false,
    }
    UnixStream::connect(&path).is_ok_and(|s| peer_is_us(&s))
}

/// Become the primary instance: listen for commands on a background thread.
pub fn listen(tx: EventTx) {
    let Some(path) = socket_path() else { return };
    // A socket file nobody answers on is stale (previous crash). The
    // directory is private to us, so removing it is safe.
    if UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(&path) else {
        return;
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    std::thread::Builder::new()
        .name("keywhere-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if !peer_is_us(&stream) {
                    continue;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(stream.take(300))
                    .read_line(&mut line)
                    .is_err()
                {
                    continue;
                }
                let mut parts = line.trim().splitn(2, ' ');
                let ev = match (parts.next(), parts.next()) {
                    (Some("show"), token) => {
                        SysEvent::Show(token.filter(|t| valid_token(t)).map(str::to_string))
                    }
                    (Some("lock"), None) => SysEvent::Lock,
                    (Some("hide"), None) => SysEvent::Hide,
                    (Some("quit"), None) => SysEvent::Quit,
                    _ => continue,
                };
                if tx.unbounded_send(ev).is_err() {
                    break;
                }
            }
        })
        .ok();
}

pub fn cleanup() {
    if let Some(p) = socket_path() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_untrusted_dirs() {
        let base = std::env::temp_dir().join(format!("kw-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&base)
            .unwrap();
        assert!(trusted_dir(&base));
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!trusted_dir(&base), "group/other access must be refused");
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = base.with_extension("link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&base, &link).unwrap();
        assert!(!trusted_dir(&link), "symlinks must be refused");
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn peer_check_accepts_own_uid() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert!(peer_is_us(&a));
    }
}

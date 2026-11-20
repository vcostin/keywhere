//! Single instance + `keywhere --show` for desktop shortcuts.
//!
//! The first instance listens on `$XDG_RUNTIME_DIR/keywhere.sock` (a
//! per-user 0700 directory; fallback: a 0700 dir in the temp dir). A second
//! start connects, sends one command and exits. Only the fixed commands
//! `show` / `lock` are understood; nothing else is read from the socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use super::{EventTx, SysEvent};

/// Unix socket paths are limited to ~108 bytes (sun_path).
const MAX_SOCKET_PATH: usize = 100;

fn socket_path() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(d).join("keywhere.sock");
        if p.as_os_str().len() <= MAX_SOCKET_PATH {
            return p;
        }
    }
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let dir = std::env::temp_dir().join(format!("keywhere-{uid}"));
    let _ = std::fs::create_dir(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir.join("keywhere.sock")
}

/// If another instance is running, hand it `cmd` and return true.
pub fn forward_to_running(cmd: &str) -> bool {
    let Ok(mut s) = UnixStream::connect(socket_path()) else {
        return false;
    };
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    s.write_all(format!("{cmd}\n").as_bytes()).is_ok()
}

/// Become the primary instance: listen for commands on a background thread.
pub fn listen(tx: EventTx) {
    let path = socket_path();
    // A socket file nobody answers on is stale (previous crash).
    if UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(&path) else {
        return;
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    std::thread::Builder::new()
        .name("keywhere-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(stream.take_limit())
                    .read_line(&mut line)
                    .is_err()
                {
                    continue;
                }
                let ev = match line.trim() {
                    "show" => SysEvent::Show,
                    "lock" => SysEvent::Lock,
                    _ => continue,
                };
                if tx.unbounded_send(ev).is_err() {
                    break;
                }
            }
        })
        .ok();
}

trait TakeLimit {
    fn take_limit(self) -> std::io::Take<UnixStream>;
}

impl TakeLimit for UnixStream {
    fn take_limit(self) -> std::io::Take<UnixStream> {
        use std::io::Read;
        self.take(64)
    }
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}

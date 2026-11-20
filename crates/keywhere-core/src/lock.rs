//! Password Safe compatible lock file (`vault.plk` next to `vault.psafe3`).
//!
//! Contents are `user@host:pid`. A live lock held by someone else means we
//! open read-only. We only ever delete a lock whose contents are ours.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn lock_path(vault: &Path) -> PathBuf {
    vault.with_extension("plk")
}

fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: buf is valid for buf.len() bytes; gethostname NUL-terminates on success.
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "unknown".into())
}

fn username() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "unknown".into())
}

/// Our identity string as written into the lock file.
pub fn identity() -> String {
    format!("{}@{}:{}", username(), hostname(), std::process::id())
}

#[derive(Debug, PartialEq, Eq)]
pub enum LockState {
    /// We hold it.
    Acquired,
    /// Someone else holds it: their `user@host:pid` text.
    HeldBy(String),
}

/// A lock we hold; released (if still ours) on drop.
#[derive(Debug)]
pub struct VaultLock {
    path: PathBuf,
    ident: String,
}

impl VaultLock {
    /// Try to take the lock. Never overwrites an existing lock file.
    pub fn acquire(vault: &Path) -> std::io::Result<(Option<VaultLock>, LockState)> {
        let path = lock_path(vault);
        let ident = identity();
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&path) {
            Ok(mut f) => {
                f.write_all(ident.as_bytes())?;
                f.sync_all()?;
                Ok((Some(VaultLock { path, ident }), LockState::Acquired))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let holder = fs::read_to_string(&path).unwrap_or_default().trim().to_string();
                if holder == ident {
                    return Ok((Some(VaultLock { path, ident }), LockState::Acquired));
                }
                Ok((None, LockState::HeldBy(holder)))
            }
            Err(e) => Err(e),
        }
    }

    /// Take over a lock the user confirmed is stale.
    pub fn take_over(vault: &Path) -> std::io::Result<VaultLock> {
        let path = lock_path(vault);
        let _ = fs::remove_file(&path);
        match Self::acquire(vault)? {
            (Some(l), _) => Ok(l),
            (None, LockState::HeldBy(h)) => {
                Err(std::io::Error::other(format!("lock was re-taken by {h}")))
            }
            (None, _) => Err(std::io::Error::other("could not take lock")),
        }
    }

    /// Is a foreign lock's holder a dead process on this machine?
    pub fn is_stale(holder: &str) -> bool {
        let Some((user_host, pid)) = holder.rsplit_once(':') else { return false };
        let Some((_, host)) = user_host.split_once('@') else { return false };
        if host != hostname() {
            return false; // can't tell for other machines
        }
        let Ok(pid) = pid.parse::<i32>() else { return false };
        #[cfg(unix)]
        {
            // SAFETY: kill(pid, 0) only checks for existence/permission.
            let alive = unsafe { libc::kill(pid, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
            !alive
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            false
        }
    }
}

impl Drop for VaultLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).map(|s| s.trim() == self.ident).unwrap_or(false) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_lifecycle() {
        let dir = std::env::temp_dir().join(format!("kw-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let vault = dir.join("v.psafe3");
        let (l, st) = VaultLock::acquire(&vault).unwrap();
        assert_eq!(st, LockState::Acquired);
        assert_eq!(fs::read_to_string(dir.join("v.plk")).unwrap(), identity());
        drop(l);
        assert!(!dir.join("v.plk").exists());

        fs::write(dir.join("v.plk"), "alice@elsewhere:42").unwrap();
        let (l, st) = VaultLock::acquire(&vault).unwrap();
        assert!(l.is_none());
        assert_eq!(st, LockState::HeldBy("alice@elsewhere:42".into()));
        assert!(!VaultLock::is_stale("alice@elsewhere:42"));
        let l = VaultLock::take_over(&vault).unwrap();
        drop(l);
        assert!(!dir.join("v.plk").exists());

        // We never delete a lock that isn't ours.
        let (l, _) = VaultLock::acquire(&vault).unwrap();
        fs::write(dir.join("v.plk"), "bob@x:1").unwrap();
        drop(l);
        assert!(dir.join("v.plk").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}

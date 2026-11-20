//! Password Safe compatible lock file (`vault.plk` next to `vault.psafe3`).
//!
//! Contents are `user@host:pid`, written the way Password Safe writes it:
//! `wchar_t` text, i.e. UTF-32LE on Linux/macOS and UTF-16LE on Windows. We
//! read UTF-8, UTF-16LE and UTF-32LE. A live lock held by someone else means
//! we open read-only.
//!
//! Safety rules:
//! * one `VaultLock` per path per process (a second acquire is refused);
//! * we only delete a lock file that is still *the same file we created*
//!   (same inode on Unix) and still carries our identity;
//! * if writing a new lock fails, the half-written file is removed.
//!
//! A `.plk` in a synced folder is advisory only (two devices can both create
//! one before sync); the content-hash conflict check on save is what prevents
//! lost updates across devices.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static HELD: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

fn held() -> std::sync::MutexGuard<'static, Option<HashSet<PathBuf>>> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

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
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

/// Our identity string as written into the lock file.
pub fn identity() -> String {
    format!("{}@{}:{}", username(), hostname(), std::process::id())
}

/// Encode like Password Safe's `wchar_t` output.
fn encode_wide(s: &str) -> Vec<u8> {
    if cfg!(windows) {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    } else {
        s.chars().flat_map(|c| (c as u32).to_le_bytes()).collect()
    }
}

/// Decode a lock file written by Keywhere, Password Safe (any OS) or others.
pub fn decode_holder(b: &[u8]) -> String {
    let trim = |s: String| {
        s.trim_matches(|c: char| c.is_whitespace() || c == '\0')
            .to_string()
    };
    if !b.contains(&0) {
        return trim(String::from_utf8_lossy(b).into_owned());
    }
    if b.len().is_multiple_of(4) {
        let (quads, _) = b.as_chunks::<4>();
        let utf32: Option<String> = quads
            .iter()
            .map(|c| char::from_u32(u32::from_le_bytes(*c)))
            .collect();
        if let Some(s) = utf32.filter(|s| s.chars().all(|c| c == '\0' || !c.is_control())) {
            return trim(s);
        }
    }
    let (pairs, _) = b.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
    trim(String::from_utf16_lossy(&units))
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
    #[cfg(unix)]
    ino: (u64, u64),
}

#[cfg(unix)]
fn file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

impl VaultLock {
    /// Try to take the lock. Never overwrites an existing lock file.
    pub fn acquire(vault: &Path) -> std::io::Result<(Option<VaultLock>, LockState)> {
        let path = lock_path(vault);
        let ident = identity();
        {
            let mut g = held();
            let set = g.get_or_insert_with(HashSet::new);
            if set.contains(&path) {
                return Ok((None, LockState::HeldBy(ident)));
            }
        }
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&path) {
            Ok(mut f) => {
                if let Err(e) = f
                    .write_all(&encode_wide(&ident))
                    .and_then(|()| f.sync_all())
                {
                    drop(f);
                    let _ = fs::remove_file(&path);
                    return Err(e);
                }
                #[cfg(unix)]
                let ino = {
                    use std::os::unix::fs::MetadataExt;
                    let m = f.metadata()?;
                    (m.dev(), m.ino())
                };
                held().get_or_insert_with(HashSet::new).insert(path.clone());
                Ok((
                    Some(VaultLock {
                        path,
                        ident,
                        #[cfg(unix)]
                        ino,
                    }),
                    LockState::Acquired,
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let holder = fs::read(&path)
                    .map(|b| decode_holder(&b))
                    .unwrap_or_default();
                Ok((None, LockState::HeldBy(holder)))
            }
            Err(e) => Err(e),
        }
    }

    /// Take over a lock the user confirmed (or we proved) is stale.
    ///
    /// `expected` is the holder string the decision was based on. The lock
    /// file is first moved aside atomically, so exactly one file is captured;
    /// if it no longer names `expected` (someone re-took the lock meanwhile),
    /// it is put back and nothing is taken over.
    pub fn take_over(vault: &Path, expected: &str) -> std::io::Result<VaultLock> {
        let path = lock_path(vault);
        if held().as_ref().is_some_and(|s| s.contains(&path)) {
            return Err(std::io::Error::other("this process already holds the lock"));
        }
        let aside = path.with_extension(format!("plk.stale.{:08x}", rand_u32()));
        match fs::rename(&path, &aside) {
            Ok(()) => {
                let holder = fs::read(&aside)
                    .map(|b| decode_holder(&b))
                    .unwrap_or_default();
                if holder != expected {
                    // Not the lock we judged: restore it (unless a new one
                    // appeared in the meantime, which then wins).
                    if fs::hard_link(&aside, &path).is_ok() || !path.exists() {
                        let _ = fs::rename(&aside, &path);
                    }
                    let _ = fs::remove_file(&aside);
                    return Err(std::io::Error::other(format!(
                        "lock changed hands (now {})",
                        sanitize_holder(&holder)
                    )));
                }
                let _ = fs::remove_file(&aside);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match Self::acquire(vault)? {
            (Some(l), _) => Ok(l),
            (None, LockState::HeldBy(h)) => Err(std::io::Error::other(format!(
                "lock was re-taken by {}",
                sanitize_holder(&h)
            ))),
            (None, _) => Err(std::io::Error::other("could not take lock")),
        }
    }

    /// Is a foreign lock's holder a dead process on this machine?
    pub fn is_stale(holder: &str) -> bool {
        let Some((user_host, pid)) = holder.rsplit_once(':') else {
            return false;
        };
        let Some((_, host)) = user_host.split_once('@') else {
            return false;
        };
        if host != hostname() {
            return false; // can't tell for other machines
        }
        let Ok(pid) = pid.trim().parse::<i32>() else {
            return false;
        };
        if pid <= 0 || pid as u32 == std::process::id() {
            return false;
        }
        #[cfg(unix)]
        {
            // SAFETY: kill(pid, 0) only checks for existence/permission.
            let alive = unsafe { libc::kill(pid, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
            !alive
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

impl Drop for VaultLock {
    fn drop(&mut self) {
        if let Some(set) = held().as_mut() {
            set.remove(&self.path);
        }
        #[cfg(unix)]
        if file_id(&self.path) != Some(self.ino) {
            return; // replaced by someone else (e.g. take_over): not ours
        }
        let ours = fs::read(&self.path)
            .map(|b| decode_holder(&b) == self.ident)
            .unwrap_or(false);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Holder text is attacker-controlled (anyone who can write the folder can
/// plant a `.plk`): keep it short and free of control / bidi characters
/// before it is shown.
pub fn sanitize_holder(h: &str) -> String {
    let clean = crate::display_safe(h);
    let mut out: String = clean.chars().take(64).collect();
    if clean.chars().count() > 64 {
        out.push('…');
    }
    out
}

fn rand_u32() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::getrandom(&mut b);
    u32::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_encodings() {
        let id = "alice@box:1234";
        assert_eq!(decode_holder(id.as_bytes()), id);
        let u16: Vec<u8> = id.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_holder(&u16), id);
        let u32: Vec<u8> = id.chars().flat_map(|c| (c as u32).to_le_bytes()).collect();
        assert_eq!(decode_holder(&u32), id);
        assert_eq!(decode_holder(&encode_wide(id)), id);
    }

    #[test]
    fn lock_lifecycle() {
        let dir = std::env::temp_dir().join(format!("kw-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let vault = dir.join("v.psafe3");
        let plk = dir.join("v.plk");

        let (l, st) = VaultLock::acquire(&vault).unwrap();
        assert_eq!(st, LockState::Acquired);
        assert_eq!(decode_holder(&fs::read(&plk).unwrap()), identity());
        // A second acquire in the same process must not create a second owner.
        let (l2, st2) = VaultLock::acquire(&vault).unwrap();
        assert!(l2.is_none() && matches!(st2, LockState::HeldBy(_)));
        drop(l);
        assert!(!plk.exists());

        fs::write(&plk, "alice@elsewhere:42").unwrap();
        let (l, st) = VaultLock::acquire(&vault).unwrap();
        assert!(l.is_none());
        assert_eq!(st, LockState::HeldBy("alice@elsewhere:42".into()));
        assert!(!VaultLock::is_stale("alice@elsewhere:42"));
        // A take-over based on a different holder must not steal the lock.
        assert!(VaultLock::take_over(&vault, "carol@elsewhere:7").is_err());
        assert_eq!(
            decode_holder(&fs::read(&plk).unwrap()),
            "alice@elsewhere:42"
        );
        let l = VaultLock::take_over(&vault, "alice@elsewhere:42").unwrap();
        drop(l);
        assert!(!plk.exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "no stale leftovers");
        assert_eq!(sanitize_holder("a\u{202e}b\u{7}c"), "abc");

        // We never delete a lock that isn't ours (contents replaced).
        let (l, _) = VaultLock::acquire(&vault).unwrap();
        fs::write(&plk, "bob@x:1").unwrap();
        drop(l);
        assert!(plk.exists());
        fs::remove_file(&plk).unwrap();

        // ...nor one that was replaced by a new file with our identity.
        // (Create the replacement before the original disappears, so the
        // filesystem cannot hand back the same inode number.)
        let (l, _) = VaultLock::acquire(&vault).unwrap();
        let repl = dir.join("v.plk.new");
        fs::write(&repl, encode_wide(&identity())).unwrap();
        fs::rename(&repl, &plk).unwrap();
        #[cfg(unix)]
        {
            drop(l);
            assert!(plk.exists(), "different inode: not ours to delete");
        }
        #[cfg(not(unix))]
        drop(l);
        fs::remove_dir_all(&dir).unwrap();
    }
}

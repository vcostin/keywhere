//! Keywhere vault engine: Password Safe v3 files with a typed record model.
//!
//! The UI never touches bytes on disk; it goes through [`Vault`].

pub mod generator;
pub mod lock;
pub mod psafe2;
pub mod psafe3;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use psafe3::{Field, KeyMaterial, RawDb};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a Password Safe v3 file")]
    NotPsafe3,
    #[error("wrong master password")]
    WrongPassword,
    #[error("file integrity check failed — the file is corrupt or was tampered with")]
    Integrity,
    #[error("file is larger than Keywhere will open")]
    TooLarge,
    #[error("corrupt file: {0}")]
    Corrupt(&'static str),
    #[error("unsupported key-stretch iteration count {0}")]
    BadIterations(u32),
    #[error("system random number generator failed")]
    Rng,
    #[error("the vault file was changed by another program or device since it was opened")]
    Conflict,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Header field types (subset Keywhere writes).
pub mod hdr {
    pub const VERSION: u8 = 0x00;
    pub const UUID: u8 = 0x01;
    pub const LAST_SAVE_TIME: u8 = 0x04;
    pub const LAST_SAVE_WHAT: u8 = 0x06;
    /// One field per empty group (full path, Password Safe 3.x).
    pub const EMPTY_GROUP: u8 = 0x11;
}

/// "Work.Dev\.ops" → ["Work", "Dev.ops"] (Password Safe's group encoding).
pub fn decode_group_path(g: &str) -> Vec<String> {
    if g.is_empty() {
        return vec![];
    }
    let mut out = vec![String::new()];
    let mut chars = g.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'.') => {
                out.last_mut().unwrap().push('.');
                chars.next();
            }
            '.' => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out
}

/// ["Work", "Dev.ops"] → "Work.Dev\.ops".
pub fn encode_group_path<S: AsRef<str>>(segments: &[S]) -> String {
    segments.iter().map(|s| s.as_ref().replace('.', "\\.")).collect::<Vec<_>>().join(".")
}

/// Record field types.
pub mod rec {
    pub const UUID: u8 = 0x01;
    pub const GROUP: u8 = 0x02;
    pub const TITLE: u8 = 0x03;
    pub const USERNAME: u8 = 0x04;
    pub const NOTES: u8 = 0x05;
    pub const PASSWORD: u8 = 0x06;
    pub const CTIME: u8 = 0x07;
    pub const PTIME: u8 = 0x08;
    pub const ATIME: u8 = 0x09;
    pub const EXPIRY: u8 = 0x0a;
    pub const MTIME: u8 = 0x0c;
    pub const URL: u8 = 0x0d;
    pub const AUTOTYPE: u8 = 0x0e;
    pub const PW_HISTORY: u8 = 0x0f;
    pub const EMAIL: u8 = 0x14;
}

/// One vault entry. Unknown fields are kept verbatim for round-trip safety.
#[derive(Clone, Default, Debug)]
pub struct Record {
    pub fields: Vec<Field>,
}

impl Record {
    pub fn new() -> Result<Self, Error> {
        let mut uuid = [0u8; 16];
        getrandom::getrandom(&mut uuid).map_err(|_| Error::Rng)?;
        // RFC 4122 v4 bits, as Password Safe generates.
        uuid[6] = (uuid[6] & 0x0f) | 0x40;
        uuid[8] = (uuid[8] & 0x3f) | 0x80;
        let now = now_secs();
        let mut r = Record::default();
        r.set(rec::UUID, uuid.to_vec());
        r.set_time(rec::CTIME, now);
        r.set_time(rec::MTIME, now);
        r.set_time(rec::PTIME, now);
        Ok(r)
    }

    pub fn get(&self, kind: u8) -> Option<&[u8]> {
        self.fields
            .iter()
            .find(|f| f.kind == kind)
            .map(|f| f.data.as_slice())
    }

    pub fn text(&self, kind: u8) -> Option<&str> {
        self.get(kind).and_then(|d| std::str::from_utf8(d).ok())
    }

    pub fn set(&mut self, kind: u8, data: Vec<u8>) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.kind == kind) {
            f.data.iter_mut().for_each(|b| *b = 0);
            f.data = data;
        } else {
            self.fields.push(Field { kind, data });
        }
    }

    pub fn set_text(&mut self, kind: u8, s: &str) {
        self.set(kind, s.as_bytes().to_vec());
    }

    pub fn set_time(&mut self, kind: u8, secs: u32) {
        self.set(kind, secs.to_le_bytes().to_vec());
    }

    pub fn time(&self, kind: u8) -> Option<u64> {
        let d = self.get(kind)?;
        match d.len() {
            4 => Some(u32::from_le_bytes(d.try_into().ok()?) as u64),
            // pre-3.09 files stored times as 8 ASCII hex digits
            8 => std::str::from_utf8(d)
                .ok()
                .and_then(|h| u64::from_str_radix(h, 16).ok()),
            _ => None,
        }
    }

    /// Set the group from path segments, escaping literal dots as "\.".
    pub fn set_group_path(&mut self, segments: &[&str]) {
        self.set_text(rec::GROUP, &encode_group_path(segments));
    }

    pub fn title(&self) -> &str {
        self.text(rec::TITLE).unwrap_or("")
    }

    pub fn uuid(&self) -> Option<&[u8]> {
        self.get(rec::UUID)
    }

    /// Set or clear (empty string) a text field.
    pub fn set_or_clear(&mut self, kind: u8, s: &str) {
        if s.is_empty() {
            self.remove(kind);
        } else {
            self.set_text(kind, s);
        }
    }

    pub fn remove(&mut self, kind: u8) {
        self.fields.retain(|f| f.kind != kind); // Field zeroizes on drop
    }

    /// Change the password: pushes the old one onto the password history
    /// (Password Safe format), updates PTIME and MTIME. No-op if unchanged.
    pub fn set_password(&mut self, new: &str, now: u32) {
        let old = Zeroizing::new(self.text(rec::PASSWORD).unwrap_or("").to_string());
        if old.as_str() == new {
            return;
        }
        // Keep history unless the user turned it off (status flag '0').
        // Skip passwords Windows Password Safe would mis-count (non-BMP chars
        // are 2 UTF-16 units there) or that don't fit the 4-hex-digit length.
        let enabled = self
            .text(rec::PW_HISTORY)
            .is_none_or(|h| h.starts_with('1'));
        let storable = old.chars().all(|c| (c as u32) <= 0xFFFF) && old.chars().count() <= 0xFFFF;
        if enabled && storable && !old.is_empty() {
            let mut hist = self.history();
            let max = self.history_max().unwrap_or(DEFAULT_HISTORY_MAX);
            let ptime = self.time(rec::PTIME).unwrap_or(now as u64);
            hist.push(HistoryEntry {
                changed: ptime,
                password: old.clone(),
            });
            while hist.len() > max as usize {
                hist.remove(0);
            }
            self.set_history(max, &hist);
        }
        self.set_text(rec::PASSWORD, new);
        self.set_time(rec::PTIME, now);
        self.set_time(rec::MTIME, now);
    }

    fn history_max(&self) -> Option<u8> {
        let h = self.text(rec::PW_HISTORY)?;
        u8::from_str_radix(h.get(1..3)?, 16).ok()
    }

    /// Parse the password history field:
    /// `"1" | max(2 hex) | count(2 hex) | (time(8 hex) len(4 hex) password)*`.
    /// Oldest first. Malformed tails are ignored.
    pub fn history(&self) -> Vec<HistoryEntry> {
        let mut out = Vec::new();
        let Some(h) = self.text(rec::PW_HISTORY) else {
            return out;
        };
        let Some(count) = h.get(3..5).and_then(|n| usize::from_str_radix(n, 16).ok()) else {
            return out;
        };
        let mut rest = h.get(5..).unwrap_or("");
        for _ in 0..count {
            let (Some(t), Some(l)) = (rest.get(0..8), rest.get(8..12)) else {
                break;
            };
            let (Ok(t), Ok(l)) = (u64::from_str_radix(t, 16), usize::from_str_radix(l, 16)) else {
                break;
            };
            // Length is in characters (Password Safe counts TCHARs).
            let body = &rest[12..];
            let end = body
                .char_indices()
                .nth(l)
                .map(|(i, _)| i)
                .unwrap_or(body.len());
            if body[..end].chars().count() != l {
                break;
            }
            out.push(HistoryEntry {
                changed: t,
                password: Zeroizing::new(body[..end].to_string()),
            });
            rest = &body[end..];
        }
        out
    }

    fn set_history(&mut self, max: u8, entries: &[HistoryEntry]) {
        let mut s = Zeroizing::new(format!("1{:02x}{:02x}", max, entries.len()));
        for e in entries {
            s.push_str(&format!(
                "{:08x}{:04x}",
                e.changed as u32,
                e.password.chars().count()
            ));
            s.push_str(&e.password);
        }
        self.set_text(rec::PW_HISTORY, &s);
    }
    /// Group path segments; '.' separates levels, "\." is a literal dot.
    pub fn group_path(&self) -> Vec<String> {
        self.text(rec::GROUP).map(decode_group_path).unwrap_or_default()
    }
}

/// Default number of previous passwords kept when Keywhere starts a history.
pub const DEFAULT_HISTORY_MAX: u8 = 5;

pub struct HistoryEntry {
    /// When this password was set (Unix seconds).
    pub changed: u64,
    pub password: Zeroizing<String>,
}

pub fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

/// On-disk format of the open vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Password Safe v3 (`.psafe3`): the default.
    V3,
    /// Password Safe v2 (Password Gorilla's legacy `.dat`): kept in place for
    /// compatibility; weak KDF and no integrity protection.
    V2,
}

enum VaultKey {
    V3(KeyMaterial),
    V2 {
        key: psafe2::V2Key,
        prefs: String,
        utf8: bool,
    },
}

/// An unlocked vault.
pub struct Vault {
    pub header: Vec<Field>,
    pub records: Vec<Record>,
    key: VaultKey,
    /// SHA-256 of the file bytes we last read or wrote; used to detect edits
    /// made elsewhere (another device via a sync folder, Password Safe, ...).
    disk_hash: Option<[u8; 32]>,
}

impl Vault {
    /// Decrypt bytes. If the file uses fewer than `MIN_ITERATIONS_ON_SAVE`
    /// rounds, a fresh salt + stronger key is derived now (we only have the
    /// password at this moment), so the next save is upgraded.
    ///
    /// Password Safe v2 files (Password Gorilla legacy) are detected and kept
    /// in v2 on save; see [`Vault::format`] and [`Vault::upgrade_to_v3`].
    pub fn decrypt(bytes: &[u8], password: &[u8]) -> Result<Self, Error> {
        let disk_hash = Some(Sha256::digest(bytes).into());
        if bytes.len() >= 4 && &bytes[..4] == b"PWS3" {
            let (raw, mut key) = psafe3::decode(bytes, password)?;
            if key.iterations < psafe3::MIN_ITERATIONS_ON_SAVE {
                key = KeyMaterial::derive(password, psafe3::MIN_ITERATIONS_ON_SAVE)?;
            }
            return Ok(Vault {
                header: raw.header.clone(),
                records: raw
                    .records
                    .iter()
                    .map(|r| Record { fields: r.clone() })
                    .collect(),
                key: VaultKey::V3(key),
                disk_hash,
            });
        }
        if psafe2::looks_like_v2(bytes) {
            let (raw, key) = psafe2::decode(bytes, password)?;
            return Ok(Vault {
                header: vec![],
                records: raw
                    .records
                    .into_iter()
                    .map(|fields| Record { fields })
                    .collect(),
                key: VaultKey::V2 {
                    key,
                    prefs: raw.prefs,
                    utf8: raw.utf8,
                },
                disk_hash,
            });
        }
        Err(Error::NotPsafe3)
    }

    pub fn format(&self) -> Format {
        match self.key {
            VaultKey::V3(_) => Format::V3,
            VaultKey::V2 { .. } => Format::V2,
        }
    }

    /// Upgrade a v2 vault and write it to `new_path` (which must not exist),
    /// all-or-nothing: on any failure the vault is restored to its v2 state
    /// so later saves still go to the v2 file in v2.
    pub fn upgrade_to_v3_at(&mut self, password: &[u8], new_path: &Path) -> Result<(), Error> {
        if new_path.exists() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "target file already exists",
            )));
        }
        let VaultKey::V2 { key, prefs, utf8 } = &self.key else {
            return Ok(());
        };
        if !key.verify(password) {
            return Err(Error::WrongPassword);
        }
        // Snapshot the v2 state (the key holds no password, only RND/SALT/key).
        let saved_key = VaultKey::V2 {
            key: key.clone_key(),
            prefs: prefs.clone(),
            utf8: *utf8,
        };
        let saved_header = self.header.clone();
        let saved_hash = self.disk_hash;
        let res = self
            .upgrade_to_v3(password)
            .and_then(|()| self.save(new_path));
        if res.is_err() {
            self.key = saved_key;
            self.header = saved_header;
            self.disk_hash = saved_hash;
        }
        res
    }

    /// Convert an open v2 vault to v3 in memory (the caller then saves it to
    /// a new `.psafe3` path). Needs the master password again, because v3
    /// derives a fresh stretched key; it is checked against the v2 verifier.
    pub fn upgrade_to_v3(&mut self, password: &[u8]) -> Result<(), Error> {
        let VaultKey::V2 { key, .. } = &self.key else {
            return Ok(());
        };
        if !key.verify(password) {
            return Err(Error::WrongPassword);
        }
        let mut uuid = [0u8; 16];
        getrandom::getrandom(&mut uuid).map_err(|_| Error::Rng)?;
        self.header = vec![
            Field {
                kind: hdr::VERSION,
                data: vec![0x0d, 0x03],
            },
            Field {
                kind: hdr::UUID,
                data: uuid.to_vec(),
            },
        ];
        self.key = VaultKey::V3(KeyMaterial::derive(
            password,
            psafe3::MIN_ITERATIONS_ON_SAVE,
        )?);
        self.disk_hash = None; // the v3 file is new
        Ok(())
    }

    /// A new, empty vault.
    pub fn create(password: &[u8]) -> Result<Self, Error> {
        let mut uuid = [0u8; 16];
        getrandom::getrandom(&mut uuid).map_err(|_| Error::Rng)?;
        let header = vec![
            Field {
                kind: hdr::VERSION,
                data: vec![0x0d, 0x03],
            },
            Field {
                kind: hdr::UUID,
                data: uuid.to_vec(),
            },
        ];
        let key = KeyMaterial::derive(password, psafe3::MIN_ITERATIONS_ON_SAVE)?;
        Ok(Vault {
            header,
            records: vec![],
            key: VaultKey::V3(key),
            disk_hash: None,
        })
    }

    pub fn open(path: &Path, password: &[u8]) -> Result<Self, Error> {
        let bytes = Zeroizing::new(fs::read(path)?);
        Self::decrypt(&bytes, password)
    }

    /// Key-stretch rounds (0 for v2, which has none).
    pub fn iterations(&self) -> u32 {
        match &self.key {
            VaultKey::V3(k) => k.iterations,
            VaultKey::V2 { .. } => 0,
        }
    }

    /// Re-key with a new master password (fresh salt; K/L/IV are fresh on
    /// every save anyway).
    pub fn change_password(&mut self, new_password: &[u8]) -> Result<(), Error> {
        match &mut self.key {
            VaultKey::V3(k) => *k = KeyMaterial::derive(new_password, k.iterations)?,
            VaultKey::V2 { key, .. } => *key = psafe2::V2Key::derive(new_password)?,
        }
        Ok(())
    }

    fn set_header(&mut self, kind: u8, data: Vec<u8>) {
        if let Some(f) = self.header.iter_mut().find(|f| f.kind == kind) {
            f.data.iter_mut().for_each(|b| *b = 0);
            f.data = data;
        } else {
            self.header.push(Field { kind, data });
        }
    }

    pub fn encrypt(&mut self) -> Result<Vec<u8>, Error> {
        if let VaultKey::V2 { key, prefs, utf8 } = &self.key {
            let raw = psafe2::RawDbV2 {
                prefs: prefs.clone(),
                utf8: *utf8,
                records: self.records.iter().map(|r| r.fields.clone()).collect(),
            };
            return psafe2::encode(&raw, key);
        }
        if !self.header.iter().any(|f| f.kind == hdr::VERSION) {
            self.header.insert(
                0,
                Field {
                    kind: hdr::VERSION,
                    data: vec![0x0d, 0x03],
                },
            );
        }
        self.set_header(hdr::LAST_SAVE_TIME, now_secs().to_le_bytes().to_vec());
        self.set_header(
            hdr::LAST_SAVE_WHAT,
            format!("Keywhere V{}", env!("CARGO_PKG_VERSION")).into_bytes(),
        );
        let VaultKey::V3(key) = &self.key else {
            unreachable!()
        };
        let raw = RawDb {
            iterations: key.iterations,
            header: self.header.clone(),
            records: self.records.iter().map(|r| r.fields.clone()).collect(),
        };
        psafe3::encode(&raw, key)
    }

    /// Has the file on disk changed since we opened or last saved it?
    pub fn changed_on_disk(&self, path: &Path) -> Result<bool, Error> {
        let Some(ours) = self.disk_hash else {
            return Ok(path.exists());
        };
        match fs::read(path) {
            Ok(b) => Ok(<[u8; 32]>::from(Sha256::digest(&b)) != ours),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(e.into()),
        }
    }

    /// Safe save. Refuses with [`Error::Conflict`] if the file changed on
    /// disk since we read it; use [`Vault::save_overwrite`] after the user
    /// confirms.
    pub fn save(&mut self, path: &Path) -> Result<(), Error> {
        self.save_inner(path, true)
    }

    /// Like [`Vault::save`] but replaces whatever is on disk (user confirmed).
    pub fn save_overwrite(&mut self, path: &Path) -> Result<(), Error> {
        self.save_inner(path, false)
    }

    /// Safe save for synced folders:
    /// 1. write `<file>.tmp.<random>` (mode 0600) in the same directory, fsync;
    /// 2. re-read the temp file from disk and decrypt-verify it;
    /// 3. read the live file once; if `check`, it must still match what we
    ///    opened/last saved, else `Conflict` (nothing touched);
    /// 4. write that exact content to `<stem>.ibak` (Password Safe's name);
    /// 5. if `check`, re-read the live file right before replacing it and
    ///    abort with `Conflict` if it moved under us;
    /// 6. atomically replace (`rename`; on Windows std uses MoveFileExW with
    ///    MOVEFILE_REPLACE_EXISTING), fsync the directory, and record the hash
    ///    of what is actually on disk now.
    ///
    /// The temp file is removed on any failure. A residual window remains
    /// between step 5 and the rename; the `.plk` lock covers local writers.
    fn save_inner(&mut self, path: &Path, check: bool) -> Result<(), Error> {
        let bytes = self.encrypt()?;
        let tmp = temp_path(path)?;
        let result = (|| {
            write_new_file(&tmp, &bytes)?;
            let on_disk = Zeroizing::new(fs::read(&tmp)?);
            match &self.key {
                VaultKey::V3(k) => drop(psafe3::decode_with_key(&on_disk, k)?),
                VaultKey::V2 { key, .. } => drop(psafe2::decode_with_key(&on_disk, key)?),
            }

            let prev = match fs::read(path) {
                Ok(b) => Some(Zeroizing::new(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            let prev_hash: Option<[u8; 32]> = prev.as_ref().map(|b| Sha256::digest(&b[..]).into());
            if check && prev_hash != self.disk_hash {
                return Err(Error::Conflict);
            }
            if let Some(prev) = &prev {
                let bak = path.with_extension("ibak");
                let bak_tmp = temp_path(&bak)?;
                let r =
                    write_new_file(&bak_tmp, prev).and_then(|()| Ok(fs::rename(&bak_tmp, &bak)?));
                if r.is_err() {
                    let _ = fs::remove_file(&bak_tmp);
                }
                r?;
            }
            if check {
                let now: Option<[u8; 32]> = match fs::read(path) {
                    Ok(b) => Some(Sha256::digest(&b).into()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.into()),
                };
                if now != prev_hash {
                    return Err(Error::Conflict);
                }
            }
            fs::rename(&tmp, path)?;
            if let Some(dir) = path.parent() {
                if let Ok(d) = fs::File::open(dir) {
                    let _ = d.sync_all();
                }
            }
            // Track what really landed (normally exactly `bytes`).
            self.disk_hash = Some(Sha256::digest(fs::read(path)?).into());
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    /// Groups that exist without entries (header field 0x11). v2 files have
    /// no such field: there an empty group lasts for the session only.
    pub fn empty_groups(&self) -> Vec<Vec<String>> {
        self.header
            .iter()
            .filter(|f| f.kind == hdr::EMPTY_GROUP)
            .filter_map(|f| std::str::from_utf8(&f.data).ok())
            .map(decode_group_path)
            .filter(|p| !p.is_empty())
            .collect()
    }

    pub fn add_empty_group(&mut self, path: &[String]) {
        if path.is_empty() || self.empty_groups().iter().any(|g| g == path) {
            return;
        }
        self.header.push(Field { kind: hdr::EMPTY_GROUP, data: encode_group_path(path).into_bytes() });
    }

    pub fn remove_empty_group(&mut self, path: &[String]) {
        let enc = encode_group_path(path);
        self.header.retain(|f| !(f.kind == hdr::EMPTY_GROUP && f.data == enc.as_bytes()));
    }

    /// Drop "empty group" markers for groups that now contain entries (or
    /// have subgroups with entries), as Password Safe does.
    pub fn prune_empty_groups(&mut self) {
        let used: Vec<Vec<String>> = self.records.iter().map(|r| r.group_path()).collect();
        for g in self.empty_groups() {
            if used.iter().any(|u| u.starts_with(&g)) {
                self.remove_empty_group(&g);
            }
        }
    }

    /// Index of the record with this UUID.
    pub fn find(&self, uuid: &[u8]) -> Option<usize> {
        self.records.iter().position(|r| r.uuid() == Some(uuid))
    }
}

fn temp_path(path: &Path) -> Result<PathBuf, Error> {
    let mut r = [0u8; 8];
    getrandom::getrandom(&mut r).map_err(|_| Error::Rng)?;
    let hex: String = r.iter().map(|b| format!("{b:02x}")).collect();
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".tmp.{hex}"));
    Ok(PathBuf::from(name))
}

/// Create a brand-new file (fails if it exists), owner-only on Unix, fsync'd.
fn write_new_file(path: &Path, data: &[u8]) -> Result<(), Error> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../../fixtures/original-empty.psafe3");

    #[test]
    fn opens_password_safe_file() {
        let v = Vault::decrypt(FIXTURE, b"test123").expect("decrypt");
        assert!(v.records.is_empty());
        assert!(v.header.iter().any(|f| f.kind == hdr::VERSION));
    }

    #[test]
    fn wrong_password_rejected() {
        assert!(matches!(
            Vault::decrypt(FIXTURE, b"nope"),
            Err(Error::WrongPassword)
        ));
    }

    #[test]
    fn tamper_detected() {
        let mut v = Vault::decrypt(FIXTURE, b"test123").unwrap();
        let mut r = Record::new().unwrap();
        r.set_text(rec::TITLE, "x");
        v.records.push(r);
        v.change_password(b"pw").unwrap();
        let mut bytes = v.encrypt().unwrap();
        // flip a ciphertext bit in the first record area
        bytes[152 + 16 * 3] ^= 1;
        assert!(Vault::decrypt(&bytes, b"pw").is_err());
    }

    #[test]
    fn hostile_iterations_rejected() {
        let mut b = FIXTURE.to_vec();
        b[36..40].copy_from_slice(&((1u32 << 24) + 1).to_le_bytes());
        assert!(matches!(
            Vault::decrypt(&b, b"test123"),
            Err(Error::BadIterations(_))
        ));
    }

    #[test]
    fn every_ciphertext_bitflip_is_uniform_integrity_error() {
        let bytes = sample_bytes();
        for i in (152..bytes.len() - 48).step_by(7) {
            let mut b = bytes.clone();
            b[i] ^= 0x40;
            assert!(
                matches!(Vault::decrypt(&b, b"pw"), Err(Error::Integrity)),
                "offset {i}"
            );
        }
    }

    fn sample_bytes() -> Vec<u8> {
        let mut v = Vault::decrypt(FIXTURE, b"test123").unwrap();
        for i in 0..3 {
            let mut r = Record::new().unwrap();
            r.set_text(rec::TITLE, &format!("entry {i}"));
            r.set_text(rec::PASSWORD, "hunter2hunter2");
            v.records.push(r);
        }
        v.change_password(b"pw").unwrap();
        v.encrypt().unwrap()
    }

    #[test]
    fn safe_save_roundtrip_and_backup() {
        let dir = std::env::temp_dir().join(format!("kw-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.psafe3");
        fs::write(&path, FIXTURE).unwrap();
        let mut v = Vault::open(&path, b"test123").unwrap();
        v.records.push(Record::new().unwrap());
        v.save(&path).unwrap();
        assert_eq!(Vault::open(&path, b"test123").unwrap().records.len(), 1);
        assert_eq!(fs::read(dir.join("vault.ibak")).unwrap(), FIXTURE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn group_path_escape_roundtrip() {
        let mut r = Record::new().unwrap();
        r.set_group_path(&["Work", "acme.example", "Dev"]);
        assert_eq!(r.text(rec::GROUP), Some("Work.acme\\.example.Dev"));
        assert_eq!(r.group_path(), vec!["Work", "acme.example", "Dev"]);
    }

    #[test]
    fn truncated_rejected() {
        for n in [0, 10, 151, 200, FIXTURE.len() - 1] {
            assert!(Vault::decrypt(&FIXTURE[..n], b"test123").is_err());
        }
    }

    #[test]
    fn roundtrip_preserves_unknown_fields() {
        let mut v = Vault::decrypt(FIXTURE, b"test123").unwrap();
        let mut r = Record::new().unwrap();
        r.set_text(rec::TITLE, "Ünïcødé 🔑");
        r.set_text(rec::GROUP, "Work.Dev\\.ops");
        r.set(0x7e, vec![1, 2, 3]); // unknown type
        v.records.push(r);
        v.change_password(b"secret").unwrap();
        let bytes = v.encrypt().unwrap();
        let v2 = Vault::decrypt(&bytes, b"secret").unwrap();
        assert_eq!(v2.records[0].title(), "Ünïcødé 🔑");
        assert_eq!(v2.records[0].group_path(), vec!["Work", "Dev.ops"]);
        assert_eq!(v2.records[0].get(0x7e), Some(&[1u8, 2, 3][..]));
        assert!(v2.iterations() >= psafe3::MIN_ITERATIONS_ON_SAVE);
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kw-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_detects_external_change() {
        let dir = tmpdir("conflict");
        let path = dir.join("v.psafe3");
        fs::write(&path, FIXTURE).unwrap();
        let mut a = Vault::open(&path, b"test123").unwrap();
        let mut b = Vault::open(&path, b"test123").unwrap();
        a.records.push(Record::new().unwrap());
        a.save(&path).unwrap(); // first writer wins
        b.records.push(Record::new().unwrap());
        assert!(matches!(b.save(&path), Err(Error::Conflict)));
        // a can keep saving: its hash tracks its own write
        a.save(&path).unwrap();
        b.save_overwrite(&path).unwrap();
        assert_eq!(Vault::open(&path, b"test123").unwrap().records.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_without_password_reuses_key() {
        let dir = tmpdir("rekey");
        let path = dir.join("v.psafe3");
        fs::write(&path, FIXTURE).unwrap();
        let mut v = Vault::open(&path, b"test123").unwrap();
        v.save(&path).unwrap();
        assert!(Vault::open(&path, b"test123").is_ok());
        v.change_password(b"n3w").unwrap();
        v.save(&path).unwrap();
        assert!(matches!(
            Vault::open(&path, b"test123"),
            Err(Error::WrongPassword)
        ));
        assert!(Vault::open(&path, b"n3w").is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn password_history_roundtrip() {
        let mut r = Record::new().unwrap();
        r.set_password("first", 1000);
        assert!(r.history().is_empty());
        r.set_password("sécond", 2000);
        r.set_password("third", 3000);
        r.set_password("third", 4000); // unchanged: no history entry
        let h = r.history();
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].changed, h[0].password.as_str()), (1000, "first"));
        assert_eq!((h[1].changed, h[1].password.as_str()), (2000, "sécond"));
        assert_eq!(r.text(rec::PASSWORD), Some("third"));
        for i in 0..10 {
            r.set_password(&format!("p{i}"), 5000 + i);
        }
        assert_eq!(r.history().len(), DEFAULT_HISTORY_MAX as usize);
    }

    #[test]
    fn history_respects_disabled_flag_and_skips_non_bmp() {
        let mut r = Record::new().unwrap();
        r.set_password("a", 1);
        r.set_text(rec::PW_HISTORY, "00500");
        r.set_password("b", 2);
        assert!(r.history().is_empty());
        assert!(r.text(rec::PW_HISTORY).unwrap().starts_with('0'));

        let mut r = Record::new().unwrap();
        r.set_password("key🔑", 1);
        r.set_password("plain", 2);
        assert!(r.history().is_empty());
    }

    #[test]
    fn conflict_detected_even_without_prior_check() {
        let dir = tmpdir("conflict2");
        let path = dir.join("v.psafe3");
        fs::write(&path, FIXTURE).unwrap();
        let mut a = Vault::open(&path, b"test123").unwrap();
        fs::write(&path, b"someone else wrote this").unwrap();
        assert!(matches!(a.save(&path), Err(Error::Conflict)));
        assert_eq!(fs::read(&path).unwrap(), b"someone else wrote this");
        assert!(
            !dir.join("v.ibak").exists(),
            "no backup written on conflict"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_groups_roundtrip_and_prune() {
        let mut v = Vault::decrypt(FIXTURE, b"test123").unwrap();
        let g = vec!["Work".to_string(), "acme.example".to_string()];
        // The Password Safe fixture already has an empty group "test".
        assert_eq!(v.empty_groups(), vec![vec!["test".to_string()]]);
        v.add_empty_group(&g);
        v.add_empty_group(&g); // no duplicates
        assert_eq!(v.empty_groups().iter().filter(|x| **x == g).count(), 1);
        v.change_password(b"pw").unwrap();
        let bytes = v.encrypt().unwrap();
        let mut v2 = Vault::decrypt(&bytes, b"pw").unwrap();
        assert!(v2.empty_groups().contains(&g));
        let mut r = Record::new().unwrap();
        r.set_group_path(&["Work", "acme.example", "Sub"]);
        v2.records.push(r);
        v2.prune_empty_groups();
        assert_eq!(v2.empty_groups(), vec![vec!["test".to_string()]]);
        assert_eq!(decode_group_path("Work.Dev\\.ops"), vec!["Work", "Dev.ops"]);
    }

    #[test]
    fn parses_password_safe_history_format() {
        let mut r = Record::default();
        r.set_text(rec::PW_HISTORY, "1050263b2a4000006hunter64b2a4110003abc");
        let h = r.history();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].password.as_str(), "hunter");
        assert_eq!(h[1].password.as_str(), "abc");
    }

    #[test]
    fn new_vault_roundtrip() {
        let mut v = Vault::create(b"pw").unwrap();
        let mut r = Record::new().unwrap();
        r.set_text(rec::TITLE, "x");
        v.records.push(r);
        let bytes = v.encrypt().unwrap();
        let v2 = Vault::decrypt(&bytes, b"pw").unwrap();
        assert_eq!(v2.records[0].title(), "x");
        assert_eq!(v2.iterations(), psafe3::MIN_ITERATIONS_ON_SAVE);
    }

    const GORILLA_V2: &[u8] = include_bytes!("../../../fixtures/gorilla-v2-testkey.dat");

    #[test]
    fn v2_vault_saves_in_v2_and_upgrades() {
        let dir = tmpdir("v2");
        let path = dir.join("legacy.dat");
        fs::write(&path, GORILLA_V2).unwrap();
        let mut v = Vault::open(&path, b"TESTKEY").unwrap();
        assert_eq!(v.format(), Format::V2);
        assert_eq!(v.records.len(), 4);
        let mut r = Record::new().unwrap();
        r.set_text(rec::TITLE, "Added by Keywhere");
        r.set_text(rec::URL, "https://example.com");
        r.set_password("s3cret", 1000);
        v.records.push(r);
        v.save(&path).unwrap();
        let v2 = Vault::open(&path, b"TESTKEY").unwrap();
        assert_eq!(v2.format(), Format::V2, "stays v2 on disk");
        assert_eq!(v2.records.len(), 5);
        let added = v2
            .records
            .iter()
            .find(|r| r.title() == "Added by Keywhere")
            .unwrap();
        assert_eq!(added.text(rec::URL), Some("https://example.com"));

        let mut v3 = Vault::open(&path, b"TESTKEY").unwrap();
        assert!(matches!(
            v3.upgrade_to_v3(b"wrong"),
            Err(Error::WrongPassword)
        ));
        v3.upgrade_to_v3(b"TESTKEY").unwrap();
        let new_path = dir.join("legacy.psafe3");
        v3.save(&new_path).unwrap();
        let back = Vault::open(&new_path, b"TESTKEY").unwrap();
        assert_eq!(back.format(), Format::V3);
        assert_eq!(back.records.len(), 5);
        assert!(
            !fs::read(&path).unwrap().is_empty(),
            "v2 original untouched"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

//! Keywhere vault engine: Password Safe v3 files with a typed record model.
//!
//! The UI never touches bytes on disk; it goes through [`Vault`].

pub mod psafe3;

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use psafe3::{Field, RawDb};

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
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Header field types (subset Keywhere writes).
pub mod hdr {
    pub const VERSION: u8 = 0x00;
    pub const UUID: u8 = 0x01;
    pub const LAST_SAVE_TIME: u8 = 0x04;
    pub const LAST_SAVE_WHAT: u8 = 0x06;
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
        self.fields.iter().find(|f| f.kind == kind).map(|f| f.data.as_slice())
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
            8 => std::str::from_utf8(d).ok().and_then(|h| u64::from_str_radix(h, 16).ok()),
            _ => None,
        }
    }

    /// Set the group from path segments, escaping literal dots as "\.".
    pub fn set_group_path(&mut self, segments: &[&str]) {
        let g: Vec<String> = segments.iter().map(|s| s.replace('.', "\\.")).collect();
        self.set_text(rec::GROUP, &g.join("."));
    }

    pub fn title(&self) -> &str {
        self.text(rec::TITLE).unwrap_or("")
    }
    /// Group path segments; '.' separates levels, "\." is a literal dot.
    pub fn group_path(&self) -> Vec<String> {
        let Some(g) = self.text(rec::GROUP) else { return vec![] };
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
}

pub fn now_secs() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

/// An unlocked vault.
pub struct Vault {
    pub iterations: u32,
    pub header: Vec<Field>,
    pub records: Vec<Record>,
}

impl Vault {
    pub fn decrypt(bytes: &[u8], password: &[u8]) -> Result<Self, Error> {
        let raw = psafe3::decode(bytes, password)?;
        Ok(Vault {
            iterations: raw.iterations,
            header: raw.header.clone(),
            records: raw.records.iter().map(|r| Record { fields: r.clone() }).collect(),
        })
    }

    pub fn open(path: &Path, password: &[u8]) -> Result<Self, Error> {
        let bytes = zeroize::Zeroizing::new(fs::read(path)?);
        Self::decrypt(&bytes, password)
    }

    fn set_header(&mut self, kind: u8, data: Vec<u8>) {
        if let Some(f) = self.header.iter_mut().find(|f| f.kind == kind) {
            f.data.iter_mut().for_each(|b| *b = 0);
            f.data = data;
        } else {
            self.header.push(Field { kind, data });
        }
    }

    pub fn encrypt(&mut self, password: &[u8]) -> Result<Vec<u8>, Error> {
        if !self.header.iter().any(|f| f.kind == hdr::VERSION) {
            self.header.insert(0, Field { kind: hdr::VERSION, data: vec![0x0d, 0x03] });
        }
        self.set_header(hdr::LAST_SAVE_TIME, now_secs().to_le_bytes().to_vec());
        self.set_header(
            hdr::LAST_SAVE_WHAT,
            format!("Keywhere V{}", env!("CARGO_PKG_VERSION")).into_bytes(),
        );
        let raw = RawDb {
            iterations: self.iterations,
            header: self.header.clone(),
            records: self.records.iter().map(|r| r.fields.clone()).collect(),
        };
        psafe3::encode(&raw, password)
    }

    /// Safe save for synced folders:
    /// 1. write `<file>.tmp.<random>` (mode 0600) in the same directory, fsync;
    /// 2. re-read the temp file from disk and decrypt-verify it;
    /// 3. copy the current file to `<stem>.ibak` (Password Safe's name), fsync;
    /// 4. atomically replace (`rename`; on Windows std uses MoveFileExW with
    ///    MOVEFILE_REPLACE_EXISTING), then fsync the directory.
    ///
    /// The temp file is removed on any failure.
    pub fn save(&mut self, path: &Path, password: &[u8]) -> Result<(), Error> {
        let bytes = self.encrypt(password)?;
        let tmp = temp_path(path)?;
        let result = (|| {
            write_new_file(&tmp, &bytes)?;
            let on_disk = zeroize::Zeroizing::new(fs::read(&tmp)?);
            Vault::decrypt(&on_disk, password)?;
            if path.exists() {
                let bak = path.with_extension("ibak");
                let prev = zeroize::Zeroizing::new(fs::read(path)?);
                let bak_tmp = temp_path(&bak)?;
                let r = write_new_file(&bak_tmp, &prev).and_then(|()| Ok(fs::rename(&bak_tmp, &bak)?));
                if r.is_err() {
                    let _ = fs::remove_file(&bak_tmp);
                }
                r?;
            }
            fs::rename(&tmp, path)?;
            if let Some(dir) = path.parent() {
                if let Ok(d) = fs::File::open(dir) {
                    let _ = d.sync_all();
                }
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
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
        assert!(matches!(Vault::decrypt(FIXTURE, b"nope"), Err(Error::WrongPassword)));
    }

    #[test]
    fn tamper_detected() {
        let mut v = Vault::decrypt(FIXTURE, b"test123").unwrap();
        let mut r = Record::new().unwrap();
        r.set_text(rec::TITLE, "x");
        v.records.push(r);
        let mut bytes = v.encrypt(b"pw").unwrap();
        // flip a ciphertext bit in the first record area
        bytes[152 + 16 * 3] ^= 1;
        assert!(Vault::decrypt(&bytes, b"pw").is_err());
    }

    #[test]
    fn hostile_iterations_rejected() {
        let mut b = FIXTURE.to_vec();
        b[36..40].copy_from_slice(&((1u32 << 24) + 1).to_le_bytes());
        assert!(matches!(Vault::decrypt(&b, b"test123"), Err(Error::BadIterations(_))));
    }

    #[test]
    fn every_ciphertext_bitflip_is_uniform_integrity_error() {
        let bytes = sample_bytes();
        for i in (152..bytes.len() - 48).step_by(7) {
            let mut b = bytes.clone();
            b[i] ^= 0x40;
            assert!(matches!(Vault::decrypt(&b, b"pw"), Err(Error::Integrity)), "offset {i}");
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
        v.encrypt(b"pw").unwrap()
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
        v.save(&path, b"test123").unwrap();
        assert_eq!(Vault::open(&path, b"test123").unwrap().records.len(), 1);
        assert_eq!(fs::read(dir.join("vault.ibak")).unwrap(), FIXTURE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap()
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
        let bytes = v.encrypt(b"secret").unwrap();
        let v2 = Vault::decrypt(&bytes, b"secret").unwrap();
        assert_eq!(v2.records[0].title(), "Ünïcødé 🔑");
        assert_eq!(v2.records[0].group_path(), vec!["Work", "Dev.ops"]);
        assert_eq!(v2.records[0].get(0x7e), Some(&[1u8, 2, 3][..]));
        assert!(v2.iterations >= psafe3::MIN_ITERATIONS_ON_SAVE);
    }
}

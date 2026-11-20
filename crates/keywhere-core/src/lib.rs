//! Keywhere vault engine: Password Safe v3 files with a typed record model.
//!
//! The UI never touches bytes on disk; it goes through [`Vault`].

pub mod psafe3;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use psafe3::{Field, RawDb};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a Password Safe v3 file")]
    NotPsafe3,
    #[error("wrong master password")]
    WrongPassword,
    #[error("file integrity check (HMAC) failed — file is corrupt or was tampered with")]
    HmacMismatch,
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
            8 => Some(u64::from_le_bytes(d.try_into().ok()?)),
            _ => None,
        }
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

    /// Safe save for synced folders: write `<file>.tmp`, fsync, verify it
    /// decrypts, keep `<file>~` as the previous version, then atomic rename.
    pub fn save(&mut self, path: &Path, password: &[u8]) -> Result<(), Error> {
        let bytes = self.encrypt(password)?;
        // Round-trip check before we touch the real file.
        Vault::decrypt(&bytes, password)?;
        let tmp = path.with_extension("psafe3.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        if path.exists() {
            let mut bak = path.as_os_str().to_owned();
            bak.push("~");
            fs::copy(path, &bak)?;
        }
        fs::rename(&tmp, path)?;
        if let Some(dir) = path.parent() {
            if let Ok(d) = fs::File::open(dir) {
                let _ = d.sync_all();
            }
        }
        Ok(())
    }
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

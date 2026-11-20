//! Password Safe v2 (`.dat`, Password Gorilla's legacy format) codec.
//!
//! Layout (see docs/formatV2.txt of Password Safe):
//!
//! ```text
//! RND[8] | H(RND)[20] | SALT[20] | IP[8] | Blowfish-CBC(fields...)
//! ```
//!
//! * H(RND) = SHA1_IV0( swap(E^1000(swap(RND))) | 00 00 ), where E is
//!   Blowfish-ECB keyed with SHA1(RND | 00 00 | password), and SHA1_IV0 is
//!   SHA-1 with an all-zero initial state (a Password Safe 1.x quirk).
//! * Data key = SHA1(password | SALT) (20-byte Blowfish key).
//! * Password Safe 1.x/2.x cast bytes to native little-endian longs before
//!   Blowfish ("genderbender"), so every 32-bit word is byte-swapped around
//!   the (big-endian) cipher, including the IV.
//! * Each field: one 8-byte block {len u32 LE, type u8, 3 pad}, then
//!   max(1, ceil(len/8)) data blocks. Records end with type 0xff.
//! * The first three fields are the "Format Description Block": name
//!   (" !!!Version 2 File Format!!! ..."), password "2.0", notes = prefs.
//!
//! v2 has NO integrity protection (no HMAC) and a weak, unstretched KDF.
//! Keywhere supports it for compatibility only and says so in the UI.

use blowfish::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use blowfish::Blowfish;
use sha1::{Digest, Sha1};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::psafe3::Field;
use crate::Error;

const HEADER_LEN: usize = 8 + 20 + 20 + 8;
const BLOCK: usize = 8;
const MAGIC: &str = "!!!Version 2 File Format!!!";
const FORMAT_NAME: &str =
    " !!!Version 2 File Format!!! Please upgrade to PasswordSafe 2.0 or later";
/// Same caps as v3 (and Gorilla refuses fields > 64 KiB).
const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_FIELD: usize = 16 * 1024 * 1024;
const MAX_FIELDS: usize = 1_000_000;

/// v2 text field types (Gorilla also treats URL, 13, as text).
const TEXT_TYPES: [u8; 6] = [0x02, 0x03, 0x04, 0x05, 0x06, 0x0d];
const NOTES: u8 = 0x05;
const UUID: u8 = 0x01;
const END: u8 = 0xff;

/// What we need to write the file again without the password: the original
/// RND/H(RND) pair (depends only on RND + password) and SALT + data key.
pub struct V2Key {
    rnd: [u8; 8],
    hrnd: [u8; 20],
    salt: [u8; 20],
    key: Zeroizing<[u8; 20]>,
}

impl std::fmt::Debug for V2Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("V2Key { .. }")
    }
}

impl V2Key {
    /// Fresh RND/SALT for a password (new file or password change).
    pub fn derive(password: &[u8]) -> Result<Self, Error> {
        let rnd: [u8; 8] = random()?;
        let salt: [u8; 20] = random()?;
        Ok(V2Key {
            hrnd: compute_hrnd(&rnd, password),
            key: data_key(password, &salt),
            rnd,
            salt,
        })
    }

    /// Copy (for rollback). Zeroizing on both copies.
    pub fn clone_key(&self) -> Self {
        V2Key {
            rnd: self.rnd,
            hrnd: self.hrnd,
            salt: self.salt,
            key: Zeroizing::new(*self.key),
        }
    }

    /// Does `password` match this key's verifier?
    pub fn verify(&self, password: &[u8]) -> bool {
        bool::from(compute_hrnd(&self.rnd, password).ct_eq(&self.hrnd))
    }
}

/// Decoded v2 database: preferences string + records (fields without the
/// 0xff terminator). Text fields are converted to UTF-8.
pub struct RawDbV2 {
    pub prefs: String,
    pub utf8: bool,
    pub records: Vec<Vec<Field>>,
}

fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|_| Error::Rng)?;
    Ok(b)
}

/// Reverse the bytes of every 32-bit word (Password Safe 1.x/2.x quirk).
fn swap_words(b: &mut [u8]) {
    let (words, rest) = b.as_chunks_mut::<4>();
    debug_assert!(rest.is_empty());
    for w in words {
        w.reverse();
    }
}

/// SHA-1 with an all-zero initial state.
fn sha1_iv0(msg: &[u8]) -> [u8; 20] {
    let mut padded = msg.to_vec();
    let bits = (msg.len() as u64) * 8;
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bits.to_be_bytes());
    let mut state = [0u32; 5];
    let blocks: Vec<GenericArray<u8, _>> = padded
        .as_chunks::<64>()
        .0
        .iter()
        .map(|c| GenericArray::clone_from_slice(c))
        .collect();
    sha1::compress(&mut state, &blocks);
    let mut out = [0u8; 20];
    for (i, w) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

fn compute_hrnd(rnd: &[u8; 8], password: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(rnd);
    h.update([0u8, 0u8]);
    h.update(password);
    let temp_salt = Zeroizing::new(<[u8; 20]>::from(h.finalize()));
    let bf = Blowfish::<byteorder::BE>::new_from_slice(&temp_salt[..]).expect("20-byte key");
    let mut block = *rnd;
    swap_words(&mut block);
    for _ in 0..1000 {
        bf.encrypt_block(GenericArray::from_mut_slice(&mut block));
    }
    swap_words(&mut block);
    let mut msg = block.to_vec();
    msg.extend_from_slice(&[0, 0]);
    sha1_iv0(&msg)
}

fn data_key(password: &[u8], salt: &[u8; 20]) -> Zeroizing<[u8; 20]> {
    let mut h = Sha1::new();
    h.update(password);
    h.update(salt);
    Zeroizing::new(h.finalize().into())
}

/// Is this plausibly a v2 file? (v3 starts with "PWS3"; v2 has no magic
/// before decryption, so this only checks shape.)
pub fn looks_like_v2(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER_LEN + 3 * 2 * BLOCK
        && (bytes.len() - HEADER_LEN).is_multiple_of(BLOCK)
        && &bytes[..4] != b"PWS3"
}

pub fn decode(bytes: &[u8], password: &[u8]) -> Result<(RawDbV2, V2Key), Error> {
    if bytes.len() > MAX_FILE {
        return Err(Error::TooLarge);
    }
    if !looks_like_v2(bytes) {
        return Err(Error::NotPsafe3);
    }
    let rnd: [u8; 8] = bytes[0..8].try_into().unwrap();
    let hrnd: [u8; 20] = bytes[8..28].try_into().unwrap();
    let salt: [u8; 20] = bytes[28..48].try_into().unwrap();
    if !bool::from(compute_hrnd(&rnd, password).ct_eq(&hrnd)) {
        return Err(Error::WrongPassword);
    }
    let key = V2Key {
        rnd,
        hrnd,
        salt,
        key: data_key(password, &salt),
    };
    let db = decode_with_key(bytes, &key)?;
    Ok((db, key))
}

/// Decrypt with key material we hold (verifying our own write).
pub fn decode_with_key(bytes: &[u8], key: &V2Key) -> Result<RawDbV2, Error> {
    if !looks_like_v2(bytes)
        || bytes[0..8] != key.rnd
        || bytes[8..28] != key.hrnd
        || bytes[28..48] != key.salt
    {
        return Err(Error::WrongPassword);
    }
    let ip: [u8; 8] = bytes[48..56].try_into().unwrap();
    let bf = Blowfish::<byteorder::BE>::new_from_slice(&key.key[..]).expect("20-byte key");

    // CBC in "swapped" space.
    let mut plain = Zeroizing::new(bytes[HEADER_LEN..].to_vec());
    let mut prev = ip;
    swap_words(&mut prev);
    for chunk in plain.as_chunks_mut::<BLOCK>().0 {
        swap_words(chunk);
        let c: [u8; 8] = *chunk;
        bf.decrypt_block(GenericArray::from_mut_slice(chunk));
        for (b, p) in chunk.iter_mut().zip(prev) {
            *b ^= p;
        }
        prev = c;
        swap_words(chunk);
    }

    // Fields. Without an HMAC, structure checks are all we have; any
    // malformation is reported uniformly.
    let mut fields: Vec<(u8, Zeroizing<Vec<u8>>)> = Vec::new();
    let mut pos = 0usize;
    while pos < plain.len() {
        if fields.len() >= MAX_FIELDS || plain.len() - pos < BLOCK {
            return Err(Error::Integrity);
        }
        let len = u32::from_le_bytes(plain[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = plain[pos + 4];
        if len > MAX_FIELD {
            return Err(Error::Integrity);
        }
        let start = pos + BLOCK;
        let blocks = len.div_ceil(BLOCK).max(1);
        let end = start + blocks * BLOCK;
        if end > plain.len() {
            return Err(Error::Integrity);
        }
        fields.push((kind, Zeroizing::new(plain[start..start + len].to_vec())));
        pos = end;
    }
    if fields.len() < 3 {
        return Err(Error::Integrity);
    }
    let name = String::from_utf8_lossy(&fields[0].1);
    if name.get(1..1 + MAGIC.len()) != Some(MAGIC) {
        // A v1 file (or wrong format). Not supported.
        return Err(Error::Corrupt("Password Safe v1 files are not supported"));
    }
    let prefs = String::from_utf8_lossy(&fields[2].1).into_owned();
    let utf8 = pref_is_utf8(&prefs);

    let mut records = Vec::new();
    let mut cur: Vec<Field> = Vec::new();
    for (kind, data) in fields.into_iter().skip(3) {
        if kind == END {
            if !cur.is_empty() {
                records.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let data = if TEXT_TYPES.contains(&kind) {
            let mut s = Zeroizing::new(decode_text(&data, utf8));
            if kind == NOTES {
                s = Zeroizing::new(s.replace("\r\n", "\n"));
            }
            s.as_bytes().to_vec()
        } else {
            data.to_vec()
        };
        cur.push(Field { kind, data });
    }
    if !cur.is_empty() {
        records.push(cur);
    }
    Ok(RawDbV2 {
        prefs,
        utf8,
        records,
    })
}

/// Encrypt. Reuses RND/H(RND)/SALT from `key`; fresh IV every save.
pub fn encode(db: &RawDbV2, key: &V2Key) -> Result<Vec<u8>, Error> {
    // Any non-ASCII text => write UTF-8 and set IsUTF8. Gorilla honours the
    // flag; Password Safe 3 decodes v2 text with the system locale, which is
    // UTF-8 on modern desktops. (8-bit codepages only survive on the machine
    // that wrote them, so we never write cp1252.)
    let needs_utf8 = !db.utf8
        && db
            .records
            .iter()
            .flatten()
            .any(|f| TEXT_TYPES.contains(&f.kind) && !f.data.is_ascii());
    let prefs = if needs_utf8 {
        set_pref_utf8(&db.prefs)
    } else {
        db.prefs.clone()
    };

    let mut plain = Zeroizing::new(Vec::new());
    push_field(&mut plain, 0, FORMAT_NAME.as_bytes());
    push_field(&mut plain, 0x06, b"2.0");
    push_field(&mut plain, NOTES, prefs.as_bytes());
    for rec in &db.records {
        // Password Safe's v2 reader needs the UUID as the record's first
        // field; add one if the record has none.
        let mut ordered: Vec<&Field> = Vec::with_capacity(rec.len() + 1);
        let generated;
        match rec.iter().find(|f| f.kind == UUID && f.data.len() == 16) {
            Some(u) => ordered.push(u),
            None => {
                let mut u: [u8; 16] = random()?;
                u[6] = (u[6] & 0x0f) | 0x40;
                u[8] = (u[8] & 0x3f) | 0x80;
                generated = Field {
                    kind: UUID,
                    data: u.to_vec(),
                };
                ordered.push(&generated);
            }
        }
        ordered.extend(rec.iter().filter(|f| f.kind != UUID));
        for f in ordered {
            if TEXT_TYPES.contains(&f.kind) {
                let s = std::str::from_utf8(&f.data)
                    .map_err(|_| Error::Corrupt("non-UTF-8 text field"))?;
                let s = if f.kind == NOTES {
                    Zeroizing::new(s.replace('\n', "\r\n"))
                } else {
                    Zeroizing::new(s.to_string())
                };
                // ASCII-only text is identical in every encoding.
                let bytes = Zeroizing::new(s.as_bytes().to_vec());
                push_field(&mut plain, f.kind, &bytes);
            } else {
                push_field(&mut plain, f.kind, &f.data);
            }
        }
        push_field(&mut plain, END, &[]);
    }

    let ip: [u8; 8] = random()?;
    let mut out = Vec::with_capacity(HEADER_LEN + plain.len());
    out.extend_from_slice(&key.rnd);
    out.extend_from_slice(&key.hrnd);
    out.extend_from_slice(&key.salt);
    out.extend_from_slice(&ip);

    let bf = Blowfish::<byteorder::BE>::new_from_slice(&key.key[..]).expect("20-byte key");
    let mut prev = ip;
    swap_words(&mut prev);
    for chunk in plain.as_chunks::<BLOCK>().0 {
        let mut b: [u8; 8] = *chunk;
        swap_words(&mut b);
        for (x, p) in b.iter_mut().zip(prev) {
            *x ^= p;
        }
        bf.encrypt_block(GenericArray::from_mut_slice(&mut b));
        prev = b;
        swap_words(&mut b);
        out.extend_from_slice(&b);
    }
    Ok(out)
}

fn push_field(out: &mut Vec<u8>, kind: u8, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&[0, 0, 0]);
    let blocks = data.len().div_ceil(BLOCK).max(1);
    out.extend_from_slice(data);
    out.resize(out.len() + blocks * BLOCK - data.len(), 0);
}

/// Preferences: "X nn vv X nn vv ..."; IsUTF8 is boolean #24.
fn pref_is_utf8(prefs: &str) -> bool {
    let t: Vec<&str> = prefs.split_whitespace().collect();
    t.chunks(3)
        .any(|c| c.len() == 3 && c[0] == "B" && c[1] == "24" && c[2] == "1")
}

fn set_pref_utf8(prefs: &str) -> String {
    let t: Vec<&str> = prefs.split_whitespace().collect();
    let mut out: Vec<String> = t
        .chunks(3)
        .filter(|c| !(c.len() == 3 && c[0] == "B" && c[1] == "24"))
        .map(|c| c.join(" "))
        .collect();
    out.push("B 24 1".into());
    out.join(" ")
}

/// Windows-1252 bytes 0x80..=0x9F (others map to the same code point).
const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

fn decode_text(b: &[u8], utf8: bool) -> String {
    if utf8 {
        if let Ok(s) = std::str::from_utf8(b) {
            return s.to_string();
        }
    }
    b.iter()
        .map(|&c| {
            if (0x80..0xa0).contains(&c) {
                CP1252_HIGH[(c - 0x80) as usize]
            } else {
                c as char
            }
        })
        .collect()
}

#[cfg(test)]
fn encode_cp1252(s: &str) -> Option<Vec<u8>> {
    s.chars()
        .map(|ch| {
            let cp = ch as u32;
            if cp < 0x80 || (0xa0..=0xff).contains(&cp) {
                Some(cp as u8)
            } else {
                CP1252_HIGH
                    .iter()
                    .position(|&c| c == ch)
                    .map(|i| 0x80 + i as u8)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The v2 test database embedded in Password Gorilla's pwtest.tcl
    /// (password "TESTKEY"); Password Safe 3's pwsafe-cli reads the same 4
    /// records from it.
    const GORILLA: &[u8] = include_bytes!("../../../fixtures/gorilla-v2-testkey.dat");

    fn text(r: &[Field], k: u8) -> String {
        r.iter()
            .find(|f| f.kind == k)
            .map(|f| String::from_utf8(f.data.clone()).unwrap())
            .unwrap_or_default()
    }

    #[test]
    fn reads_gorilla_v2_vector() {
        let (db, _) = decode(GORILLA, b"TESTKEY").unwrap();
        assert_eq!(db.records.len(), 4);
        let mut titles: Vec<String> = db.records.iter().map(|r| text(r, 0x03)).collect();
        titles.sort();
        assert!(titles.contains(&"Title 1 in Group 1".to_string()));
        let r = db
            .records
            .iter()
            .find(|r| text(r, 0x03) == "Title 2 in Group 1")
            .unwrap();
        assert_eq!(text(r, 0x04), "User 2 in Group 1");
        assert_eq!(text(r, 0x06), "Password 2 in Group 1");
        assert_eq!(text(r, 0x02), "Group 1");
    }

    #[test]
    fn wrong_password() {
        assert!(matches!(
            decode(GORILLA, b"nope"),
            Err(Error::WrongPassword)
        ));
    }

    #[test]
    fn roundtrip_keeps_extra_fields_and_unicode() {
        let (mut db, key) = decode(GORILLA, b"TESTKEY").unwrap();
        db.records[0].push(Field {
            kind: 0x0d,
            data: b"https://example.com".to_vec(),
        });
        db.records[0].push(Field {
            kind: 0x14,
            data: b"me@example.com".to_vec(),
        });
        db.records[0].push(Field {
            kind: 0x05,
            data: "line1\nline2 ünï €".as_bytes().to_vec(),
        });
        db.records[1].push(Field {
            kind: 0x03,
            data: "Ключ 🔑".as_bytes().to_vec(),
        });
        let bytes = encode(&db, &key).unwrap();
        let (db2, _) = decode(&bytes, b"TESTKEY").unwrap();
        assert_eq!(db2.records.len(), 4);
        assert_eq!(text(&db2.records[0], 0x0d), "https://example.com");
        assert_eq!(text(&db2.records[0], 0x14), "me@example.com");
        assert!(db2.utf8, "non-ASCII text switches the file to UTF-8");
        assert!(db2.records[1]
            .iter()
            .any(|f| f.kind == 0x03 && f.data == "Ключ 🔑".as_bytes()));
    }

    #[test]
    fn cp1252_roundtrip() {
        let s = "café € “quotes”";
        assert_eq!(decode_text(&encode_cp1252(s).unwrap(), false), s);
        assert!(encode_cp1252("🔑").is_none());
    }

    #[test]
    fn truncated_and_garbage_rejected() {
        for n in [0, 10, 56, 63, GORILLA.len() - 8] {
            assert!(decode(&GORILLA[..n], b"TESTKEY").is_err());
        }
        let mut b = GORILLA.to_vec();
        b[100] ^= 0xff;
        // No HMAC in v2: a flipped bit may or may not be detectable, but it
        // must never panic.
        let _ = decode(&b, b"TESTKEY");
    }

    #[test]
    fn uuid_written_first_and_added_if_missing() {
        let (mut db, key) = decode(GORILLA, b"TESTKEY").unwrap();
        db.records.push(vec![Field {
            kind: 0x03,
            data: b"no uuid".to_vec(),
        }]);
        let mut moved = db.records[0].clone();
        moved.rotate_left(1); // UUID no longer first
        db.records[0] = moved;
        let (db2, _) = decode(&encode(&db, &key).unwrap(), b"TESTKEY").unwrap();
        for r in &db2.records {
            assert_eq!(r[0].kind, UUID);
            assert_eq!(r[0].data.len(), 16);
        }
    }

    #[test]
    fn new_key_verifies() {
        let k = V2Key::derive(b"pw").unwrap();
        assert!(k.verify(b"pw"));
        assert!(!k.verify(b"px"));
    }
}

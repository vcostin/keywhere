//! Password Safe v3 (`.psafe3`) codec.
//!
//! Layout (all integers little-endian):
//!
//! ```text
//! "PWS3" | SALT[32] | ITER u32 | H(P')[32] | B1B2[32] | B3B4[32] | IV[16]
//! | Twofish-CBC(K, IV, header fields .. 0xff, record fields .. 0xff, ...)
//! | "PWS3-EOFPWS3-EOF" | HMAC-SHA256(L, plaintext field data)[32]
//! ```
//!
//! P' = SHA256^ITER(SHA256(password || SALT)); H(P') = SHA256(P').
//! K = Twofish-ECB-dec(P', B1B2), L = Twofish-ECB-dec(P', B3B4).
//!
//! Security notes:
//! * Password hash and HMAC are compared in constant time.
//! * No parsed data is returned unless the HMAC verifies.
//! * Every length read from the file is bounds-checked before use.
//! * Key material and decrypted field data are zeroized on drop.
//! * Unknown header / record field types are preserved for round-trip.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use twofish::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use twofish::Twofish;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::Error;

const TAG: &[u8; 4] = b"PWS3";
const EOF_MARKER: &[u8; 16] = b"PWS3-EOFPWS3-EOF";
const PREAMBLE_LEN: usize = 4 + 32 + 4 + 32 + 32 + 32 + 16; // 152
const BLOCK: usize = 16;
pub const END_OF_ENTRY: u8 = 0xff;

/// Password Safe itself refuses fewer than 2048 iterations.
pub const MIN_ITERATIONS_ON_OPEN: u32 = 2048;
/// Floor applied whenever Keywhere writes a file.
pub const MIN_ITERATIONS_ON_SAVE: u32 = 262_144;
/// Upper bound to avoid a hostile file pinning the CPU forever (~2^28).
pub const MAX_ITERATIONS: u32 = 1 << 28;

/// One TLV field: header or record. Data is zeroized when dropped.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Field {
    pub kind: u8,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print field contents.
        write!(f, "Field {{ kind: {:#04x}, len: {} }}", self.kind, self.data.len())
    }
}

/// Raw decoded database: header fields and records (each a list of fields,
/// without the terminating 0xff field).
pub struct RawDb {
    pub iterations: u32,
    pub header: Vec<Field>,
    pub records: Vec<Vec<Field>>,
}

fn stretch(password: &[u8], salt: &[u8], iterations: u32) -> Zeroizing<[u8; 32]> {
    let mut h = Sha256::new();
    h.update(password);
    h.update(salt);
    let mut x = Zeroizing::new([0u8; 32]);
    x.copy_from_slice(&h.finalize());
    for _ in 0..iterations {
        let d = Sha256::digest(&x[..]);
        x.copy_from_slice(&d);
    }
    x
}

fn twofish(key: &[u8]) -> Twofish {
    Twofish::new_from_slice(key).expect("Twofish accepts 32-byte keys")
}

fn ecb_decrypt_32(cipher: &Twofish, input: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&input[..32]);
    for chunk in out.chunks_mut(BLOCK) {
        cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
    }
    out
}

fn ecb_encrypt_32(cipher: &Twofish, input: &[u8; 32]) -> [u8; 32] {
    let mut out = *input;
    for chunk in out.chunks_mut(BLOCK) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    out
}

fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|_| Error::Rng)?;
    Ok(b)
}

/// Decrypt and parse a psafe3 file.
pub fn decode(bytes: &[u8], password: &[u8]) -> Result<RawDb, Error> {
    if bytes.len() < PREAMBLE_LEN + EOF_MARKER.len() + 32 || &bytes[..4] != TAG {
        return Err(Error::NotPsafe3);
    }
    let salt = &bytes[4..36];
    let iterations = u32::from_le_bytes(bytes[36..40].try_into().unwrap());
    if !(MIN_ITERATIONS_ON_OPEN..=MAX_ITERATIONS).contains(&iterations) {
        return Err(Error::BadIterations(iterations));
    }
    let stored_hp = &bytes[40..72];
    let b1b2 = &bytes[72..104];
    let b3b4 = &bytes[104..136];
    let iv: [u8; 16] = bytes[136..152].try_into().unwrap();

    let p_prime = stretch(password, salt, iterations);
    let hp = Sha256::digest(&p_prime[..]);
    if !bool::from(hp.as_slice().ct_eq(stored_hp)) {
        return Err(Error::WrongPassword);
    }

    let pc = twofish(&p_prime[..]);
    let k = ecb_decrypt_32(&pc, b1b2);
    let l = ecb_decrypt_32(&pc, b3b4);

    let tail = bytes.len() - 48;
    if &bytes[tail..tail + 16] != EOF_MARKER {
        return Err(Error::Corrupt("missing EOF marker / truncated file"));
    }
    let enc = &bytes[PREAMBLE_LEN..tail];
    if enc.len() % BLOCK != 0 {
        return Err(Error::Corrupt("ciphertext not block aligned"));
    }
    let stored_mac = &bytes[tail + 16..];

    // CBC decrypt into a zeroizing buffer.
    let kc = twofish(&k[..]);
    let mut plain = Zeroizing::new(enc.to_vec());
    let mut prev = iv;
    for chunk in plain.chunks_mut(BLOCK) {
        let mut c = [0u8; 16];
        c.copy_from_slice(chunk);
        kc.decrypt_block(GenericArray::from_mut_slice(chunk));
        for (b, p) in chunk.iter_mut().zip(prev.iter()) {
            *b ^= p;
        }
        prev = c;
    }

    // Parse TLV fields.
    let mut fields: Vec<Field> = Vec::new();
    let mut pos = 0usize;
    while pos < plain.len() {
        if plain.len() - pos < BLOCK {
            return Err(Error::Corrupt("short field block"));
        }
        let len = u32::from_le_bytes(plain[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = plain[pos + 4];
        let start = pos + 5;
        let end = start.checked_add(len).ok_or(Error::Corrupt("field length overflow"))?;
        if end > plain.len() {
            return Err(Error::Corrupt("field length past end of data"));
        }
        fields.push(Field { kind, data: plain[start..end].to_vec() });
        let used = (5 + len).max(BLOCK);
        pos += used.div_ceil(BLOCK) * BLOCK;
    }

    // Authenticate before trusting anything structural.
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&l[..]).expect("hmac key");
    for f in &fields {
        mac.update(&f.data);
    }
    let computed = mac.finalize().into_bytes();
    if !bool::from(computed.as_slice().ct_eq(stored_mac)) {
        return Err(Error::HmacMismatch);
    }

    let mut it = fields.into_iter();
    let mut header = Vec::new();
    let mut saw_header_end = false;
    for f in it.by_ref() {
        if f.kind == END_OF_ENTRY {
            saw_header_end = true;
            break;
        }
        header.push(f);
    }
    if !saw_header_end {
        return Err(Error::Corrupt("header not terminated"));
    }
    let mut records = Vec::new();
    let mut cur = Vec::new();
    for f in it {
        if f.kind == END_OF_ENTRY {
            records.push(std::mem::take(&mut cur));
        } else {
            cur.push(f);
        }
    }
    if !cur.is_empty() {
        return Err(Error::Corrupt("last record not terminated"));
    }
    Ok(RawDb { iterations, header, records })
}

fn push_field(out: &mut Vec<u8>, mac: &mut Hmac<Sha256>, kind: u8, data: &[u8]) -> Result<(), Error> {
    mac.update(data);
    let len = u32::try_from(data.len()).map_err(|_| Error::Corrupt("field too large"))?;
    let total = (5 + data.len()).max(BLOCK).div_ceil(BLOCK) * BLOCK;
    let start = out.len();
    out.extend_from_slice(&len.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(data);
    // Pad with random bytes, as Password Safe does.
    let pad = total - (5 + data.len());
    let mut padding = vec![0u8; pad];
    getrandom::getrandom(&mut padding).map_err(|_| Error::Rng)?;
    out.extend_from_slice(&padding);
    debug_assert_eq!(out.len() - start, total);
    Ok(())
}

/// Encrypt a database. Fresh salt, K, L and IV are generated on every save.
pub fn encode(db: &RawDb, password: &[u8]) -> Result<Vec<u8>, Error> {
    let iterations = db.iterations.clamp(MIN_ITERATIONS_ON_SAVE, MAX_ITERATIONS);
    let salt: [u8; 32] = random()?;
    let k = Zeroizing::new(random::<32>()?);
    let l = Zeroizing::new(random::<32>()?);
    let iv: [u8; 16] = random()?;

    let p_prime = stretch(password, &salt, iterations);
    let hp = Sha256::digest(&p_prime[..]);
    let pc = twofish(&p_prime[..]);

    let mut out = Vec::with_capacity(4096);
    out.extend_from_slice(TAG);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&iterations.to_le_bytes());
    out.extend_from_slice(&hp);
    out.extend_from_slice(&ecb_encrypt_32(&pc, &k));
    out.extend_from_slice(&ecb_encrypt_32(&pc, &l));
    out.extend_from_slice(&iv);

    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&l[..]).expect("hmac key");
    let mut plain = Zeroizing::new(Vec::new());
    for f in &db.header {
        push_field(&mut plain, &mut mac, f.kind, &f.data)?;
    }
    push_field(&mut plain, &mut mac, END_OF_ENTRY, &[])?;
    for rec in &db.records {
        for f in rec {
            push_field(&mut plain, &mut mac, f.kind, &f.data)?;
        }
        push_field(&mut plain, &mut mac, END_OF_ENTRY, &[])?;
    }

    let kc = twofish(&k[..]);
    let mut prev = iv;
    for chunk in plain.chunks(BLOCK) {
        let mut b = [0u8; 16];
        for i in 0..BLOCK {
            b[i] = chunk[i] ^ prev[i];
        }
        kc.encrypt_block(GenericArray::from_mut_slice(&mut b));
        out.extend_from_slice(&b);
        prev = b;
    }

    out.extend_from_slice(EOF_MARKER);
    out.extend_from_slice(&mac.finalize().into_bytes());
    Ok(out)
}

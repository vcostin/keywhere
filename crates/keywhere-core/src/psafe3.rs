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
//! * No field is materialised unless the HMAC verifies; after a correct
//!   passphrase every failure maps to one uniform `Error::Integrity`.
//! * Every length read from the file is bounds-checked and size-capped.
//! * Key material and decrypted field data are zeroized on drop.
//! * Unknown header / record field types are preserved for round-trip.

use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
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
/// Upper bound so a hostile ITER cannot pin the CPU for minutes (2^24).
pub const MAX_ITERATIONS: u32 = 1 << 24;
/// Hostile-input caps, checked before allocation.
pub const MAX_CIPHERTEXT: usize = 64 * 1024 * 1024;
pub const MAX_FIELD: usize = 32 * 1024 * 1024;
pub const MAX_FIELDS: usize = 1_000_000;

/// One TLV field: header or record. Data is zeroized when dropped.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Field {
    pub kind: u8,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print field contents.
        write!(
            f,
            "Field {{ kind: {:#04x}, len: {} }}",
            self.kind,
            self.data.len()
        )
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

/// The stretched key, pinned in RAM: heap-boxed so it never moves,
/// `mlock`ed (kept out of swap/zram) and excluded from core dumps where the
/// OS allows; wiped and unlocked on drop. Failure to lock (RLIMIT_MEMLOCK)
/// is not fatal: the key is still zeroized, and [`LockedKey::locked`] says
/// so, so the app can warn (audit N2). Windows doesn't lock it yet.
pub struct LockedKey {
    key: Box<Zeroizing<[u8; 32]>>,
    locked: bool,
}

impl LockedKey {
    fn new(key: Zeroizing<[u8; 32]>) -> Self {
        let key = Box::new(key);
        #[cfg(unix)]
        let locked = unsafe {
            let ptr = key.as_ptr() as *const libc::c_void;
            let locked = libc::mlock(ptr, 32) == 0;
            #[cfg(target_os = "linux")]
            {
                // madvise needs a page-aligned range: cover the key's page.
                let page = libc::sysconf(libc::_SC_PAGESIZE) as usize;
                let start = (ptr as usize) & !(page - 1);
                libc::madvise(start as *mut libc::c_void, page, libc::MADV_DONTDUMP);
            }
            locked
        };
        #[cfg(not(unix))]
        let locked = false;
        LockedKey { key, locked }
    }

    fn get(&self) -> &[u8; 32] {
        &self.key
    }

    /// Did `mlock` succeed (the key can't be paged out)?
    fn locked(&self) -> bool {
        self.locked
    }
}

impl Drop for LockedKey {
    fn drop(&mut self) {
        // Zeroizing wipes the bytes when the box drops; unlock afterwards is
        // fine (the page stays resident until then).
        use zeroize::Zeroize;
        self.key.zeroize();
        #[cfg(unix)]
        if self.locked {
            unsafe {
                libc::munlock(self.key.as_ptr() as *const libc::c_void, 32);
            }
        }
    }
}

/// The password-derived key for one vault: salt, iteration count and the
/// stretched key P'. Lets an unlocked vault be saved again without keeping the
/// master password in memory. Zeroized on drop; never written anywhere except
/// as the file's own salt/iteration header.
pub struct KeyMaterial {
    salt: [u8; 32],
    pub iterations: u32,
    p_prime: LockedKey,
}

impl KeyMaterial {
    /// Fresh salt + stretch (used for new vaults, password changes and
    /// iteration upgrades). Iterations are clamped to the save floor.
    pub fn derive(password: &[u8], iterations: u32) -> Result<Self, Error> {
        let iterations = iterations.clamp(MIN_ITERATIONS_ON_SAVE, MAX_ITERATIONS);
        let salt: [u8; 32] = random()?;
        let p_prime = stretch(password, &salt, iterations);
        Ok(Self {
            salt,
            iterations,
            p_prime: LockedKey::new(p_prime),
        })
    }
}

impl KeyMaterial {
    /// Is the stretched key locked in RAM (`mlock` succeeded)? False on
    /// Windows, and wherever the memory-lock limit is too low.
    pub fn in_locked_memory(&self) -> bool {
        self.p_prime.locked()
    }
}

impl std::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyMaterial {{ iterations: {}, .. }}", self.iterations)
    }
}

fn preamble(bytes: &[u8]) -> Result<(&[u8], u32), Error> {
    if bytes.len() < PREAMBLE_LEN + EOF_MARKER.len() + 32 || &bytes[..4] != TAG {
        return Err(Error::NotPsafe3);
    }
    let iterations = u32::from_le_bytes(bytes[36..40].try_into().unwrap());
    if !(MIN_ITERATIONS_ON_OPEN..=MAX_ITERATIONS).contains(&iterations) {
        return Err(Error::BadIterations(iterations));
    }
    Ok((&bytes[4..36], iterations))
}

/// Decrypt and parse a psafe3 file with the master password.
pub fn decode(bytes: &[u8], password: &[u8]) -> Result<(RawDb, KeyMaterial), Error> {
    let (salt, iterations) = preamble(bytes)?;
    let p_prime = stretch(password, salt, iterations);
    let db = decode_core(bytes, &p_prime, iterations)?;
    Ok((
        db,
        KeyMaterial {
            salt: salt.try_into().unwrap(),
            iterations,
            p_prime: LockedKey::new(p_prime),
        },
    ))
}

/// Decrypt with key material we already hold (e.g. verifying our own save).
/// Fails with `WrongPassword` if the file was re-keyed by someone else.
pub fn decode_with_key(bytes: &[u8], key: &KeyMaterial) -> Result<RawDb, Error> {
    let (salt, iterations) = preamble(bytes)?;
    if !bool::from(salt.ct_eq(&key.salt)) || iterations != key.iterations {
        return Err(Error::WrongPassword);
    }
    decode_core(bytes, key.p_prime.get(), iterations)
}

fn decode_core(bytes: &[u8], p_prime: &[u8; 32], iterations: u32) -> Result<RawDb, Error> {
    let stored_hp = &bytes[40..72];
    let b1b2 = &bytes[72..104];
    let b3b4 = &bytes[104..136];
    let iv: [u8; 16] = bytes[136..152].try_into().unwrap();

    let hp = Sha256::digest(&p_prime[..]);
    if !bool::from(hp.as_slice().ct_eq(stored_hp)) {
        return Err(Error::WrongPassword);
    }

    let pc = twofish(&p_prime[..]);
    let k = ecb_decrypt_32(&pc, b1b2);
    let l = ecb_decrypt_32(&pc, b3b4);

    let tail = bytes.len() - 48;
    let enc = &bytes[PREAMBLE_LEN..tail];
    if enc.len() > MAX_CIPHERTEXT {
        return Err(Error::TooLarge);
    }
    let stored_mac = &bytes[tail + 16..];
    // From here on the passphrase is known-good: every failure is reported as
    // a single, uniform integrity error so a tamperer learns nothing.
    parse_authenticated(&bytes[tail..tail + 16], enc, &iv, &k, &l, stored_mac)
        .map(|(header, records)| RawDb {
            iterations,
            header,
            records,
        })
        .map_err(|()| Error::Integrity)
}

type Parsed = (Vec<Field>, Vec<Vec<Field>>);

fn parse_authenticated(
    eof: &[u8],
    enc: &[u8],
    iv: &[u8; 16],
    k: &[u8; 32],
    l: &[u8; 32],
    stored_mac: &[u8],
) -> Result<Parsed, ()> {
    if eof != EOF_MARKER || enc.is_empty() || !enc.len().is_multiple_of(BLOCK) {
        return Err(());
    }
    let mut plain = Zeroizing::new(enc.to_vec());
    cbc::Decryptor::<Twofish>::new(k.into(), iv.into())
        .decrypt_padded_mut::<NoPadding>(&mut plain)
        .map_err(|_| ())?;

    let spans = field_spans(&plain)?;

    // Authenticate before materialising any field.
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(l).expect("hmac key");
    for &(_, a, b) in &spans {
        mac.update(&plain[a..b]);
    }
    let computed = mac.finalize().into_bytes();
    if !bool::from(computed.as_slice().ct_eq(stored_mac)) {
        return Err(());
    }
    assemble(&plain, spans)
}

/// A field as (type, payload start, payload end) inside the plaintext.
type Span = (u8, usize, usize);

/// Walk the decrypted TLV stream once, collecting spans only (no field is
/// built here). Every bound is checked before it is used.
fn field_spans(plain: &[u8]) -> Result<Vec<Span>, ()> {
    let mut spans: Vec<Span> = Vec::new();
    let mut pos = 0usize;
    while pos < plain.len() {
        if plain.len() - pos < BLOCK || spans.len() >= MAX_FIELDS {
            return Err(());
        }
        let len = u32::from_le_bytes(plain[pos..pos + 4].try_into().unwrap()) as usize;
        if len > MAX_FIELD {
            return Err(());
        }
        let start = pos + 5;
        let end = start + len; // len <= 32 MiB, cannot overflow
        if end > plain.len() {
            return Err(());
        }
        spans.push((plain[pos + 4], start, end));
        pos += (5 + len).max(BLOCK).div_ceil(BLOCK) * BLOCK;
    }
    Ok(spans)
}

/// Build the header and records from spans. Only called once the HMAC over
/// those spans has verified.
///
/// The HMAC covers payloads, not type bytes (format v3), and flipping IV
/// bits changes the first field's type without touching the HMAC. An END
/// field with a payload is therefore refused: otherwise turning the version
/// field into END would silently drop it and move the rest of the header
/// into a bogus record. Writers never put data in END.
fn assemble(plain: &[u8], spans: Vec<Span>) -> Result<Parsed, ()> {
    if spans
        .iter()
        .any(|&(kind, a, b)| kind == END_OF_ENTRY && a != b)
    {
        return Err(());
    }
    let mut it = spans.into_iter();
    let mut header = Vec::new();
    let mut saw_header_end = false;
    for (kind, a, b) in it.by_ref() {
        if kind == END_OF_ENTRY {
            saw_header_end = true;
            break;
        }
        header.push(Field {
            kind,
            data: plain[a..b].to_vec(),
        });
    }
    if !saw_header_end {
        return Err(());
    }
    let mut records = Vec::new();
    let mut cur = Vec::new();
    for (kind, a, b) in it {
        if kind == END_OF_ENTRY {
            records.push(std::mem::take(&mut cur));
        } else {
            cur.push(Field {
                kind,
                data: plain[a..b].to_vec(),
            });
        }
    }
    if !cur.is_empty() {
        return Err(());
    }
    Ok((header, records))
}

/// Fuzzing only (`cargo fuzz` builds with `--cfg fuzzing`): run the
/// plaintext parser on bytes that never went through decryption or the HMAC.
/// Not compiled into any normal build; the real path always authenticates
/// first (see `parse_authenticated`).
#[cfg(fuzzing)]
#[doc(hidden)]
pub fn fuzz_parse_plaintext(plain: &[u8]) -> Option<(Vec<Field>, Vec<Vec<Field>>)> {
    if !plain.len().is_multiple_of(BLOCK) {
        return None;
    }
    let spans = field_spans(plain).ok()?;
    assemble(plain, spans).ok()
}

fn push_field(
    out: &mut Vec<u8>,
    mac: &mut Hmac<Sha256>,
    kind: u8,
    data: &[u8],
) -> Result<(), Error> {
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

/// Encrypt a database under `key`. Fresh K, L and IV on every save; the salt
/// changes whenever the key is re-derived (password change, iteration upgrade).
pub fn encode(db: &RawDb, key: &KeyMaterial) -> Result<Vec<u8>, Error> {
    let iterations = key.iterations;
    if !(MIN_ITERATIONS_ON_SAVE..=MAX_ITERATIONS).contains(&iterations) {
        return Err(Error::BadIterations(iterations));
    }
    let salt = key.salt;
    let k = Zeroizing::new(random::<32>()?);
    let l = Zeroizing::new(random::<32>()?);
    let iv: [u8; 16] = random()?;

    let p_prime = key.p_prime.get();
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

    debug_assert!(plain.len().is_multiple_of(BLOCK));
    let ct_start = out.len();
    out.extend_from_slice(&plain);
    let n = plain.len();
    cbc::Encryptor::<Twofish>::new((&*k).into(), (&iv).into())
        .encrypt_padded_mut::<NoPadding>(&mut out[ct_start..], n)
        .map_err(|_| Error::Integrity)?;

    out.extend_from_slice(EOF_MARKER);
    out.extend_from_slice(&mac.finalize().into_bytes());
    Ok(out)
}

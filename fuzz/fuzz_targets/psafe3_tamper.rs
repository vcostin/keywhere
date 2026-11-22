//! Tampering with a valid vault: flip, truncate or append bytes of a real
//! encrypted file and decode it with the right key. This reaches what
//! whole-file fuzzing with random bytes can't: the verifier, the K/L unwrap,
//! CBC, the EOF marker and the HMAC.
//!
//! Oracle: a tampered file may only decode if its content is unchanged.
//! Two things are outside the v3 HMAC by design: CBC padding and field type
//! bytes. Through the IV an attacker can flip bits of the first plaintext
//! block, so the first header field's type may change (to anything but END,
//! which `assemble` refuses when it carries data). Every payload, every other
//! type and the record structure must survive or the file must be refused.
#![no_main]

use std::sync::OnceLock;

use arbitrary::Arbitrary;
use keywhere_core::psafe3::{self, Field, KeyMaterial, RawDb};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Tamper {
    flips: Vec<(u16, u8)>,
    truncate: Option<u16>,
    append: Vec<u8>,
}

fn field(kind: u8, data: &[u8]) -> Field {
    Field {
        kind,
        data: data.to_vec(),
    }
}

fn sample() -> RawDb {
    RawDb {
        iterations: 0,
        header: vec![field(0x00, &[0x0f, 0x03]), field(0x06, b"fuzz")],
        records: vec![
            vec![
                field(0x01, &[7; 16]),
                field(0x03, b"Example"),
                field(0x06, b"not-a-real-password"),
            ],
            vec![field(0x01, &[9; 16]), field(0xe5, b"unknown type")],
        ],
    }
}

fn same(a: &[Field], b: &[Field]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.kind == y.kind && x.data == y.data)
}

fuzz_target!(|t: Tamper| {
    // One key and one file per process: stretching runs once, and every
    // input tampers with the same bytes, so coverage stays deterministic.
    static VAULT: OnceLock<(KeyMaterial, RawDb, Vec<u8>)> = OnceLock::new();
    let (key, db, file) = VAULT.get_or_init(|| {
        let key = KeyMaterial::derive(b"fuzz", 0).unwrap();
        let mut db = sample();
        db.iterations = key.iterations;
        let file = psafe3::encode(&db, &key).unwrap();
        (key, db, file)
    });

    let mut bytes = file.clone();
    for &(at, x) in &t.flips {
        let i = at as usize % bytes.len();
        bytes[i] ^= x;
    }
    if let Some(n) = t.truncate {
        bytes.truncate(n as usize % (bytes.len() + 1));
    }
    bytes.extend_from_slice(&t.append);

    if let Ok(back) = psafe3::decode_with_key(&bytes, key) {
        assert_eq!(
            db.header.len(),
            back.header.len(),
            "tampered header accepted"
        );
        assert_eq!(
            db.header[0].data, back.header[0].data,
            "tampered header accepted"
        );
        assert!(
            same(&db.header[1..], &back.header[1..]),
            "tampered header accepted"
        );
        assert_eq!(
            db.records.len(),
            back.records.len(),
            "tampered records accepted"
        );
        for (a, b) in db.records.iter().zip(&back.records) {
            assert!(same(a, b), "tampered record accepted");
        }
    }
});

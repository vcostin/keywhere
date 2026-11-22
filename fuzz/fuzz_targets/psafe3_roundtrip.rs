//! encode → decode_with_key must give back exactly what was written: types,
//! payloads and order of header and record fields (unknown types included).
#![no_main]

use std::sync::OnceLock;

use arbitrary::Arbitrary;
use keywhere_core::psafe3::{self, Field, KeyMaterial, RawDb};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Db {
    header: Vec<(u8, Vec<u8>)>,
    records: Vec<Vec<(u8, Vec<u8>)>>,
}

/// 0xff ends a header or record, so it can't be a field type of its own.
fn fields(v: &[(u8, Vec<u8>)]) -> Vec<Field> {
    v.iter()
        .filter(|(k, _)| *k != 0xff)
        .map(|(kind, data)| Field {
            kind: *kind,
            data: data.clone(),
        })
        .collect()
}

fn same(a: &[Field], b: &[Field]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.kind == y.kind && x.data == y.data)
}

fuzz_target!(|db: Db| {
    // Stretching once (262 144 rounds) keeps each run cheap.
    static KEY: OnceLock<KeyMaterial> = OnceLock::new();
    let key = KEY.get_or_init(|| KeyMaterial::derive(b"fuzz", 0).unwrap());

    let raw = RawDb {
        iterations: key.iterations,
        header: fields(&db.header),
        records: db.records.iter().map(|r| fields(r)).collect(),
    };
    let bytes = psafe3::encode(&raw, key).expect("encode");
    let back = psafe3::decode_with_key(&bytes, key).expect("decode own file");
    assert!(same(&raw.header, &back.header));
    assert_eq!(raw.records.len(), back.records.len());
    for (a, b) in raw.records.iter().zip(&back.records) {
        assert!(same(a, b));
    }
});

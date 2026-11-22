//! The psafe3 plaintext parser (TLV walk + header/record assembly) on
//! hostile plaintext. In the app this code only runs after the HMAC has
//! verified, so whole-file fuzzing can't reach it; `fuzz_parse_plaintext`
//! exists only under `--cfg fuzzing`.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some((header, records)) = keywhere_core::psafe3::fuzz_parse_plaintext(data) {
        // Every field must lie inside the input.
        let total: usize = header
            .iter()
            .chain(records.iter().flatten())
            .map(|f| f.data.len())
            .sum();
        assert!(total <= data.len());
    }
});

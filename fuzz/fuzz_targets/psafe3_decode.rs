//! Whole-file `psafe3::decode` on hostile bytes: preamble, size caps,
//! verifier and HMAC checks must reject garbage without panicking.
//!
//! Key stretching makes every input with a valid tag cost ITER SHA-256
//! rounds, so inputs above 4096 rounds are skipped to keep the fuzzer fast
//! (the iteration bounds themselves are unit-tested).
#![no_main]

use keywhere_core::psafe3;
use libfuzzer_sys::fuzz_target;

const MAX_FUZZ_ITER: u32 = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() >= 40 {
        let iter = u32::from_le_bytes(data[36..40].try_into().unwrap());
        if iter > MAX_FUZZ_ITER {
            return;
        }
    }
    let _ = psafe3::decode(data, b"fuzz");
});

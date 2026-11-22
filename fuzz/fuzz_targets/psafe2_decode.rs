//! Password Safe v2 (Password Gorilla) `psafe2::decode`. v2 has no MAC, so
//! once the password check passes, hostile ciphertext decrypts to garbage
//! that goes straight into the field parser. The seed is Password Gorilla's
//! public test vector (password "TESTKEY"), so mutations past its 48-byte
//! header reach that parser.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = keywhere_core::psafe2::decode(data, b"TESTKEY");
});

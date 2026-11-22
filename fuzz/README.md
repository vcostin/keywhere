# Fuzzing the vault engine

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) targets for
`keywhere-core`. They need a nightly toolchain; the rest of the repo stays on
the pinned stable one.

| Target | What it feeds | Why |
| --- | --- | --- |
| `psafe3_decode` | whole hostile files to `psafe3::decode` | preamble, size caps and the password check (random bytes never pass the verifier, so this target stops there) |
| `psafe3_tamper` | flips, truncation and appended bytes on a real encrypted file, decoded with the right key | verifier, K/L unwrap, CBC, EOF marker and HMAC; a tampered file may only decode if its content is unchanged |
| `psafe3_parse` | decrypted plaintext to the TLV parser | the parser only runs after the HMAC verifies, so whole-file fuzzing can't reach it; `psafe3::fuzz_parse_plaintext` exists only under `--cfg fuzzing` |
| `psafe3_roundtrip` | arbitrary header/record fields | `encode` → `decode_with_key` returns the same types, payloads and order |
| `psafe2_decode` | whole files to `psafe2::decode` | v2 has no MAC: after the password check, garbage goes straight into the parser |

`psafe3_decode` skips files that ask for more than 4096 key-stretching rounds
(each run would otherwise cost up to 2²⁴ SHA-256 rounds). `psafe3_tamper`
stretches once per process and reuses that key and file.

The v3 format leaves two things outside the HMAC: CBC padding and field type
bytes. Through the IV, the first header field's type can be changed without
breaking the HMAC; `psafe3_tamper` allows exactly that and nothing else. It
found that turning that field into END (`0xff`) used to drop the version field
and turn the rest of the header into a bogus record; an END field with data is
now refused.

## Running

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd fuzz
cargo +nightly fuzz list
# corpus/<target> is the working corpus (git-ignored); seeds/<target> holds
# the committed starting inputs.
cargo +nightly fuzz run psafe2_decode corpus/psafe2_decode seeds/psafe2_decode -- -max_total_time=300
```

A crash is written to `artifacts/<target>/`. Reproduce it with
`cargo +nightly fuzz run <target> artifacts/<target>/crash-…`, then add a unit
test in `keywhere-core` with that input before fixing it.

## Seeds

No real secrets. `seeds/psafe2_decode` is Password Gorilla's public v2 test
vector (password `TESTKEY`, also `fixtures/gorilla-v2-testkey.dat`);
`seeds/psafe3_decode` is `fixtures/original-empty.psafe3` with its iteration
count lowered so the target doesn't skip it (it then fails the password check,
as any mutation of it will); `seeds/psafe3_parse` is a hand-made plaintext
with two records of dummy values. `psafe3_tamper` builds its own vault from
dummy values, so its seed is empty.

`fuzz_parse_plaintext` exists only when `--cfg fuzzing` is set, which
cargo-fuzz does; a normal or release build never contains it unless someone
passes that flag on purpose.

CI runs each target for 60 s on pull requests that touch
`crates/keywhere-core` or `fuzz/` ([fuzz.yml](../.github/workflows/fuzz.yml)).

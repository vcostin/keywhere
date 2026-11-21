# Fuzzing the vault engine

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) targets for
`keywhere-core`. They need a nightly toolchain; the rest of the repo stays on
the pinned stable one.

| Target | What it feeds | Why |
| --- | --- | --- |
| `psafe3_decode` | whole files to `psafe3::decode` | preamble, size caps, verifier and HMAC checks on hostile bytes |
| `psafe3_parse` | decrypted plaintext to the TLV parser | the parser only runs after the HMAC verifies, so whole-file fuzzing can't reach it; `psafe3::fuzz_parse_plaintext` exists only under `--cfg fuzzing` |
| `psafe3_roundtrip` | arbitrary header/record fields | `encode` → `decode_with_key` returns the same types, payloads and order |
| `psafe2_decode` | whole files to `psafe2::decode` | v2 has no MAC: after the password check, garbage goes straight into the parser |

`psafe3_decode` skips files that ask for more than 4096 key-stretching rounds
(each run would otherwise cost up to 2²⁴ SHA-256 rounds).

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
count lowered so the target doesn't skip it; `seeds/psafe3_parse` is a
hand-made plaintext with two records of dummy values.

CI runs each target for 60 s on pull requests that touch
`crates/keywhere-core` or `fuzz/` ([fuzz.yml](../.github/workflows/fuzz.yml)).

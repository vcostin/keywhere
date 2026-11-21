# RAM residue test

Measures whether vault secrets remain in Keywhere's process memory, using
the dev-only harnesses (feature `devtools`; never part of release builds).

* `hardened` – the real hardening (`PR_SET_DUMPABLE=0`): even the parent
  process must be unable to read `/proc/<pid>/maps` / `mem`.
* `ui` – unlock through the real password field (password from a file, not
  argv/env), scan while unlocked, send `lock`, scan again.
* `copy` – copy a password to the clipboard, lock, scan.
* `reveal` – reveal a password on screen, lock, scan (residues found after
  locking are printed with the bytes around them).
* `scan` – pre-unlocked demo (master password arrives via env, so it is
  expected in memory: harness artefact).

```sh
cargo build --release -p keywhere --features devtools --examples
# secrets.txt: one known secret per line (e.g. exported from a demo vault)
python3 -I tools/memtest/scan.py <workdir> target/release/examples/memtest_unlock ui
```

`<workdir>` must contain `demo.psafe3`, `secrets.txt`, and for `ui` a
`pw.txt` with the master password. Run it inside a (nested) Wayland session.
Results go to `<workdir>/result-<mode>.txt`. See docs/security/ram-test.md.

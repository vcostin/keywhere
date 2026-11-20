//! DEV ONLY: RAM-residue test. Unlocks through the real password field.
//! The password is read from a file into zeroizing memory (not argv/env).
//! cargo run --features devtools --example memtest_unlock -- vault.psafe3 pwfile
fn main() {
    let mut args = std::env::args_os().skip(1);
    let path = std::path::PathBuf::from(args.next().expect("vault"));
    let pw_file = args.next().expect("password file");
    let bytes = zeroize::Zeroizing::new(std::fs::read(pw_file).expect("read password file"));
    let text = std::str::from_utf8(&bytes)
        .expect("utf-8")
        .trim_end_matches('\n');
    let pw = zeroize::Zeroizing::new(text.to_string());
    drop(bytes);
    keywhere::run_unlock_via_ui(path, pw);
}

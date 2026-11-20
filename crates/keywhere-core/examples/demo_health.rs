//! DEV: make a scratch demo vault exercise the health views.
//! cargo run --example demo_health -- copy.psafe3   (password in $KEYWHERE_PASSWORD)
use keywhere_core::{now_secs, Vault};
fn main() {
    // Never on argv: visible in `ps` and shell history.
    let pw = std::env::var("KEYWHERE_PASSWORD").expect("set KEYWHERE_PASSWORD");
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("path"));
    let mut v = Vault::open(&path, pw.as_bytes()).expect("open");
    let now = now_secs();
    for r in v.records.iter_mut() {
        match r.title() {
            "Netflix" | "Amazon" => r.set_password("sunshine1", now),
            "Router admin" => r.set_password("admin123", now),
            "Gmail" | "GitHub" => r.set_favorite(true),
            _ => {}
        }
    }
    v.save_overwrite(&path).expect("save");
}

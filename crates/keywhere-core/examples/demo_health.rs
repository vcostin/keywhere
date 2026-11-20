//! DEV: make a scratch demo vault exercise the health views.
//! cargo run --example demo_health -- copy.psafe3 PASSWORD
use keywhere_core::{now_secs, Vault};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = std::path::PathBuf::from(&a[1]);
    let mut v = Vault::open(&path, a[2].as_bytes()).expect("open");
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

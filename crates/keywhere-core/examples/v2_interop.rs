//! Interop check: open a v2 file, add entries, save in place (stays v2).
//! cargo run --example v2_interop -- file.dat PASSWORD
use keywhere_core::{rec, Record, Vault};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = std::path::PathBuf::from(&a[1]);
    let mut v = Vault::open(&path, a[2].as_bytes()).expect("open");
    println!("format {:?}, {} records", v.format(), v.records.len());
    let mut r = Record::new().unwrap();
    r.set_group_path(&["Keywhere", "Tests"]);
    r.set_text(rec::TITLE, "Café ünïcode Ключ");
    r.set_text(rec::USERNAME, "kw-user");
    r.set_text(rec::URL, "https://example.com/login");
    r.set_text(rec::NOTES, "line one\nline two");
    r.set_password("p@ss-€-word", keywhere_core::now_secs());
    v.records.push(r);
    v.save(&path).expect("save");
    let back = Vault::open(&path, a[2].as_bytes()).expect("reopen");
    println!(
        "saved: format {:?}, {} records",
        back.format(),
        back.records.len()
    );
}

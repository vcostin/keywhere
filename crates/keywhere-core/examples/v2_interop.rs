//! Interop check: open a v2 file, add entries, save in place (stays v2).
//! cargo run --example v2_interop -- file.dat   (password in $KEYWHERE_PASSWORD)
use keywhere_core::{rec, Record, Vault};
fn main() {
    // Never on argv: visible in `ps` and shell history.
    let pw = std::env::var("KEYWHERE_PASSWORD").expect("set KEYWHERE_PASSWORD");
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("path"));
    let mut v = Vault::open(&path, pw.as_bytes()).expect("open");
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
    let back = Vault::open(&path, pw.as_bytes()).expect("reopen");
    println!(
        "saved: format {:?}, {} records",
        back.format(),
        back.records.len()
    );
}

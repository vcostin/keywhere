//! Test-vault helper for the end-to-end test (tools/e2e/run.sh).
//!
//! * `e2e_vault create <file>`: a new vault with one entry. Master password,
//!   entry title and entry password come from `KEYWHERE_PASSWORD`,
//!   `E2E_TITLE` and `E2E_ENTRY_PASSWORD` (env, not argv).
//! * `e2e_vault titles <file>`: decrypt with `KEYWHERE_PASSWORD` and print
//!   every entry's title, one per line.

use keywhere_core::{rec, Record, Vault};
use std::path::PathBuf;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().expect("command: create | titles");
    let path = PathBuf::from(args.next().expect("vault file"));
    let pw = env("KEYWHERE_PASSWORD");
    match cmd.as_str() {
        "create" => {
            let mut v = Vault::create(pw.as_bytes()).expect("create");
            let mut r = Record::new().expect("record");
            r.set_text(rec::TITLE, &env("E2E_TITLE"));
            r.set_text(rec::USERNAME, "e2e");
            r.set_text(rec::PASSWORD, &env("E2E_ENTRY_PASSWORD"));
            v.records.push(r);
            v.save(&path).expect("save");
        }
        "titles" => {
            let v = Vault::open(&path, pw.as_bytes()).expect("open");
            for r in &v.records {
                println!("{}", r.title());
            }
        }
        other => panic!("unknown command {other}"),
    }
}

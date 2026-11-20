//! Populate a psafe3 vault with sample records.
//! Usage: KEYWHERE_PASSWORD=... cargo run --example populate -- <file.psafe3>
//! (password from env, never argv, so it doesn't show up in `ps`).

use keywhere_core::{now_secs, rec, Record, Vault};
use std::path::PathBuf;

fn gen_password(len: usize) -> String {
    const SET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%^&*-_=+?";
    let mut out = String::new();
    while out.len() < len {
        let mut b = [0u8; 1];
        getrandom::getrandom(&mut b).unwrap();
        // rejection sampling: no modulo bias
        if (b[0] as usize) < 256 - 256 % SET.len() {
            out.push(SET[b[0] as usize % SET.len()] as char);
        }
    }
    out
}

struct S {
    group: &'static str,
    title: &'static str,
    user: &'static str,
    url: &'static str,
    email: &'static str,
    notes: &'static str,
    pw: Option<&'static str>,
}

const DAY: u32 = 86_400;

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("path"));
    let pw = std::env::var("KEYWHERE_PASSWORD").expect("KEYWHERE_PASSWORD");
    let mut v = Vault::open(&path, pw.as_bytes()).expect("open");

    let samples = [
        S { group: "Personal.Email", title: "Gmail", user: "jane.doe", url: "https://mail.google.com", email: "jane.doe@example.com", notes: "Recovery phone ends in 42.", pw: None },
        S { group: "Personal.Email", title: "Proton Mail", user: "janedoe", url: "https://account.proton.me", email: "janedoe@proton.example", notes: "", pw: None },
        S { group: "Personal.Social", title: "Mastodon", user: "@jane@fosstodon.example", url: "https://fosstodon.org", email: "jane.doe@example.com", notes: "", pw: None },
        S { group: "Personal.Social", title: "Reddit", user: "jane_reads", url: "https://www.reddit.com", email: "", notes: "", pw: None },
        S { group: "Personal.Shopping", title: "Amazon", user: "jane.doe@example.com", url: "https://www.amazon.com", email: "jane.doe@example.com", notes: "Prime renews in March.", pw: None },
        S { group: "Personal.Shopping", title: "eBay", user: "jdoe_deals", url: "https://www.ebay.com", email: "", notes: "", pw: None },
        S { group: "Finance.Banking", title: "Northwind Bank", user: "12345678", url: "https://bank.northwind.example", email: "", notes: "Card PIN is NOT stored here.\nSecurity Q: first pet → \"Pixel\"", pw: None },
        S { group: "Finance.Banking", title: "Credit Card Portal", user: "jane.doe", url: "https://cards.example.com", email: "", notes: "", pw: None },
        S { group: "Finance.Investing", title: "Brokerage", user: "JD-99812", url: "https://invest.example.com", email: "", notes: "2FA via authenticator app.", pw: None },
        S { group: "Work", title: "Company SSO", user: "jdoe", url: "https://sso.acme.example", email: "jane.doe@acme.example", notes: "Rotates every 90 days.", pw: None },
        S { group: "Work.Dev", title: "GitHub", user: "janedoe-dev", url: "https://github.com", email: "jane.doe@acme.example", notes: "Recovery codes in 'Work.Dev / GitHub recovery'.", pw: None },
        S { group: "Work.Dev", title: "GitHub recovery codes", user: "", url: "", email: "", notes: "a1b2c-3d4e5\nf6g7h-8i9j0\nk1l2m-3n4o5\np6q7r-8s9t0", pw: Some("") },
        S { group: "Work.Dev", title: "npm registry token", user: "janedoe", url: "https://www.npmjs.com", email: "", notes: "Automation token, read-only.", pw: None },
        S { group: "Work.Servers", title: "prod-db-01 (PostgreSQL)", user: "app_rw", url: "postgres://10.0.4.21:5432/app", email: "", notes: "Use bastion host.", pw: None },
        S { group: "Work.Servers", title: "bastion.acme.example SSH", user: "jdoe", url: "ssh://bastion.acme.example", email: "", notes: "Key-based; this is the sudo password.", pw: None },
        S { group: "Home", title: "Home Wi-Fi", user: "SSID: Nebula-5G", url: "", email: "", notes: "WPA3. Guest network: Nebula-Guest.", pw: None },
        S { group: "Home", title: "Router admin", user: "admin", url: "http://192.168.1.1", email: "", notes: "", pw: None },
        S { group: "Home", title: "NAS (Synology)", user: "jane", url: "https://nas.local:5001", email: "", notes: "", pw: None },
        S { group: "Travel", title: "Airline frequent flyer", user: "FF 7781 2290", url: "https://www.airline.example", email: "jane.doe@example.com", notes: "", pw: None },
        S { group: "Travel", title: "Passport", user: "", url: "", email: "", notes: "Number: X1234567\nExpires: 2031-05-14", pw: Some("") },
        S { group: "", title: "Netflix", user: "jane.doe@example.com", url: "https://www.netflix.com", email: "", notes: "", pw: None },
        S { group: "", title: "Café Wi-Fi « Ünïcødé » 🔑", user: "guest", url: "", email: "", notes: "Unicode round-trip test entry.", pw: None },
    ];

    let now = now_secs();
    for (i, s) in samples.iter().enumerate() {
        let mut r = Record::new().unwrap();
        // spread creation dates over the last ~2 years
        let age = (i as u32 * 37 + 5) * DAY;
        r.set_time(rec::CTIME, now - age);
        r.set_time(rec::MTIME, now - age / 3);
        r.set_time(rec::PTIME, now - age / 3);
        if !s.group.is_empty() {
            r.set_text(rec::GROUP, s.group);
        }
        r.set_text(rec::TITLE, s.title);
        if !s.user.is_empty() {
            r.set_text(rec::USERNAME, s.user);
        }
        let pw = match s.pw {
            Some(p) => p.to_string(),
            None => gen_password(14 + (i % 4) * 4),
        };
        r.set_text(rec::PASSWORD, &pw);
        if !s.url.is_empty() {
            r.set_text(rec::URL, s.url);
        }
        if !s.email.is_empty() {
            r.set_text(rec::EMAIL, s.email);
        }
        if !s.notes.is_empty() {
            r.set_text(rec::NOTES, s.notes);
        }
        if s.title == "Company SSO" {
            // expiring soon -> exercises expiry UI
            r.set_time(rec::EXPIRY, now + 5 * DAY);
            // password history: "1" active, max 05, count 01, then time(8 hex) len(4 hex) pw
            let old = gen_password(16);
            let hist = format!("10501{:08x}{:04x}{}", now - 90 * DAY, old.len(), old);
            r.set_text(rec::PW_HISTORY, &hist);
        }
        if s.title == "Router admin" {
            r.set_time(rec::EXPIRY, now - 10 * DAY); // already expired
        }
        v.records.push(r);
    }
    v.save(&path, pw.as_bytes()).expect("save");
    let check = Vault::open(&path, pw.as_bytes()).expect("reopen");
    println!("saved {} records, {} iterations", check.records.len(), check.iterations);
    for r in &check.records {
        println!("  [{}] {}", r.group_path().join(" / "), r.title());
    }
}

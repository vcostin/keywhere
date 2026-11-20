//! Non-secret view model derived from an unlocked vault.

use std::collections::BTreeMap;

use gpui::SharedString;
use keywhere_core::{rec, Record, Vault};

/// What the list and sidebar show. Never contains passwords or notes.
#[derive(Clone)]
pub struct EntryVm {
    pub index: usize, // into vault.records
    pub title: SharedString,
    pub username: SharedString,
    pub group: Vec<String>,
    pub group_label: SharedString,
    pub expiry: Option<u64>,
    haystack: String, // lowercase: title, user, url, email, group
}

#[derive(Clone, PartialEq, Eq)]
pub enum Filter {
    All,
    Expiring,
    Group(Vec<String>),
}

pub struct GroupNode {
    pub path: Vec<String>,
    pub count: usize,
}

pub fn build(vault: &Vault) -> Vec<EntryVm> {
    let mut v: Vec<EntryVm> = vault
        .records
        .iter()
        .enumerate()
        .map(|(index, r)| {
            let group = r.group_path();
            let title = r.title().to_string();
            let username = r.text(rec::USERNAME).unwrap_or("").to_string();
            let haystack = [
                title.as_str(),
                username.as_str(),
                r.text(rec::URL).unwrap_or(""),
                r.text(rec::EMAIL).unwrap_or(""),
                &group.join(" "),
            ]
            .join("\u{1}")
            .to_lowercase();
            EntryVm {
                index,
                group_label: group.join(" / ").into(),
                title: title.into(),
                username: username.into(),
                group,
                expiry: r.time(rec::EXPIRY).filter(|t| *t > 0),
                haystack,
            }
        })
        .collect();
    v.sort_by_key(|e| e.title.to_lowercase());
    v
}

/// All group paths (including intermediate ones) with recursive counts.
pub fn groups(entries: &[EntryVm]) -> Vec<GroupNode> {
    let mut map: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    for e in entries {
        for depth in 1..=e.group.len() {
            *map.entry(e.group[..depth].to_vec()).or_default() += 1;
        }
    }
    map.into_iter()
        .map(|(path, count)| GroupNode { path, count })
        .collect()
}

pub const EXPIRY_WINDOW_SECS: u64 = 14 * 86_400;

pub fn is_expiring(e: &EntryVm, now: u64) -> bool {
    matches!(e.expiry, Some(t) if t <= now + EXPIRY_WINDOW_SECS)
}

pub fn matches(e: &EntryVm, filter: &Filter, query: &str, now: u64) -> bool {
    let f = match filter {
        Filter::All => true,
        Filter::Expiring => is_expiring(e, now),
        Filter::Group(p) => e.group.starts_with(p),
    };
    f && (query.is_empty() || query.split_whitespace().all(|w| e.haystack.contains(w)))
}

/// Rough strength in bits from length and character classes (UI hint only).
pub fn strength_bits(pw: &str) -> f64 {
    let mut pool = 0u32;
    if pw.chars().any(|c| c.is_ascii_lowercase()) {
        pool += 26;
    }
    if pw.chars().any(|c| c.is_ascii_uppercase()) {
        pool += 26;
    }
    if pw.chars().any(|c| c.is_ascii_digit()) {
        pool += 10;
    }
    if pw.chars().any(|c| c.is_ascii_punctuation() || c == ' ') {
        pool += 33;
    }
    if !pw.is_ascii() {
        pool += 100;
    }
    if pool == 0 {
        return 0.0;
    }
    pw.chars().count() as f64 * (pool as f64).log2()
}

pub fn history_count(r: &Record) -> usize {
    // "1" active, max (2 hex), count (2 hex), ...
    r.text(rec::PW_HISTORY)
        .and_then(|h| h.get(3..5))
        .and_then(|n| usize::from_str_radix(n, 16).ok())
        .unwrap_or(0)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Unix seconds → (year, month 1-12, day), proleptic Gregorian, UTC.
fn civil(secs: u64) -> (i64, u32, u32) {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn fmt_date(secs: u64) -> String {
    let (y, m, d) = civil(secs);
    format!("{} {}, {}", MONTHS[(m - 1) as usize], d, y)
}

pub fn fmt_relative_expiry(exp: u64, now: u64) -> (String, bool) {
    if exp <= now {
        let days = (now - exp) / 86_400;
        (
            format!("Password expired {} ({}).", ago(days), fmt_date(exp)),
            true,
        )
    } else {
        let days = (exp - now).div_ceil(86_400);
        let when = if days == 1 {
            "tomorrow".to_string()
        } else {
            format!("in {days} days")
        };
        (
            format!("Password expires {when} ({}).", fmt_date(exp)),
            false,
        )
    }
}

fn ago(days: u64) -> String {
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        n => format!("{n} days ago"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(951_782_400), (2000, 2, 29));
        assert_eq!(fmt_date(1_795_132_800), "Nov 20, 2026");
    }

    #[test]
    fn strength_monotonic() {
        assert!(strength_bits("aaaa") < strength_bits("aA1!aA1!"));
        assert_eq!(strength_bits(""), 0.0);
    }
}

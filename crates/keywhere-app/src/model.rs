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
    /// Entries without any group.
    NoGroup,
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
/// `empty` = groups stored without entries (they and their parents appear
/// with the counts of whatever they contain, possibly 0).
pub fn groups(entries: &[EntryVm], empty: &[Vec<String>]) -> Vec<GroupNode> {
    let mut map: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    for g in empty {
        for depth in 1..=g.len() {
            map.entry(g[..depth].to_vec()).or_default();
        }
    }
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
        Filter::NoGroup => e.group.is_empty(),
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
    let month = crate::i18n::t(MONTHS[(m - 1) as usize]);
    match crate::i18n::current() {
        // "20 nov. 2026"
        crate::i18n::Lang::Ro => format!("{d} {month} {y}"),
        // "Nov 20, 2026"
        crate::i18n::Lang::En => format!("{month} {d}, {y}"),
    }
}

/// Expiry banner text and whether it is already past.
pub fn fmt_relative_expiry(exp: u64, now: u64) -> (String, bool) {
    use crate::i18n::tf;
    let date = fmt_date(exp);
    if exp <= now {
        let days = ((now - exp) / 86_400).to_string();
        let msg = match (now - exp) / 86_400 {
            0 => tf("Password expired today ({date}).", &[("date", &date)]),
            1 => tf("Password expired yesterday ({date}).", &[("date", &date)]),
            _ => tf(
                "Password expired {n} days ago ({date}).",
                &[("n", &days), ("date", &date)],
            ),
        };
        (msg, true)
    } else {
        let n = (exp - now).div_ceil(86_400);
        let msg = if n == 1 {
            tf("Password expires tomorrow ({date}).", &[("date", &date)])
        } else {
            tf(
                "Password expires in {n} days ({date}).",
                &[("n", &n.to_string()), ("date", &date)],
            )
        };
        (msg, false)
    }
}

/// (year, month, day) → Unix seconds at 00:00 UTC.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn fmt_iso_date(secs: u64) -> String {
    let (y, m, d) = civil(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Parse `YYYY-MM-DD` (UTC midnight) into Unix seconds that fit psafe3's u32.
pub fn parse_iso_date(s: &str) -> Option<u32> {
    let mut it = s.trim().split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some()
        || !(1970..=2105).contains(&y)
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
    {
        return None;
    }
    let secs = days_from_civil(y, m, d) * 86_400;
    // Reject e.g. Feb 31 by round-tripping.
    if civil(secs as u64) != (y, m, d) {
        return None;
    }
    u32::try_from(secs).ok()
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
    fn iso_dates() {
        assert_eq!(parse_iso_date("2026-11-20"), Some(1_795_132_800));
        assert_eq!(fmt_iso_date(1_795_132_800), "2026-11-20");
        assert_eq!(parse_iso_date("2026-02-31"), None);
        assert_eq!(parse_iso_date("nope"), None);
    }

    #[test]
    fn strength_monotonic() {
        assert!(strength_bits("aaaa") < strength_bits("aA1!aA1!"));
        assert_eq!(strength_bits(""), 0.0);
    }
}

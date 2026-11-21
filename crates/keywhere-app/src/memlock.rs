//! Whether the unlocked vault's key is pinned in RAM (audit N2). A failed
//! `mlock` never blocks unlocking: it is logged once per run and shown as a
//! note in Settings while that vault is open.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const NONE: u8 = 0;
const LOCKED: u8 = 1;
const NOT_LOCKED: u8 = 2;

static STATE: AtomicU8 = AtomicU8::new(NONE);
static LOGGED: AtomicBool = AtomicBool::new(false);

/// A vault was unlocked (or brought back from the tray).
pub fn record(locked: bool) {
    STATE.store(if locked { LOCKED } else { NOT_LOCKED }, Ordering::Relaxed);
    if !locked && !LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!("keywhere: warning: {}", message_en());
    }
}

/// The vault was locked or closed.
pub fn clear() {
    STATE.store(NONE, Ordering::Relaxed);
}

/// Is a vault open whose key is not locked in RAM?
pub fn not_locked() -> bool {
    STATE.load(Ordering::Relaxed) == NOT_LOCKED
}

/// The English note (also the translation key).
pub fn message_en() -> &'static str {
    if cfg!(windows) {
        "The vault key is not locked in RAM on Windows yet, so it can reach the page file. Use BitLocker or another full-disk encryption."
    } else {
        "The vault key could not be locked in RAM (memory lock limit too low, or a v2 vault), so it can reach swap. Use encrypted swap, or raise RLIMIT_MEMLOCK."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_follows_the_open_vault() {
        record(true);
        assert!(!not_locked());
        record(false);
        assert!(not_locked());
        clear();
        assert!(!not_locked());
    }
}

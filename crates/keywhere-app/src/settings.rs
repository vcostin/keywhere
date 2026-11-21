//! User settings (never secrets), stored as simple `key = value` lines in
//! `<config dir>/keywhere/settings`. Read from anywhere via [`get`].

use std::sync::RwLock;
use std::time::Duration;

use crate::i18n::Lang;
use crate::theme::ThemeChoice;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    /// Hide to the tray and keep the vault unlocked (auto-lock still applies).
    Hide,
    /// Lock the vault, then hide to the tray.
    Lock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Minutes without input before locking; 0 = never.
    pub idle_lock_minutes: u32,
    pub clipboard_clear_secs: u32,
    pub close_action: CloseAction,
    /// `None` = follow the system language.
    pub language: Option<Lang>,
    pub theme: ThemeChoice,
    /// Ask GitHub for a newer release at startup. `None` = not decided yet:
    /// no network until the user answers the first-run question.
    pub check_updates: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            idle_lock_minutes: 5,
            clipboard_clear_secs: 30,
            // Safe by default (audit KW-H1): closing locks. Keeping the vault
            // unlocked in the background is an explicit opt-in.
            close_action: CloseAction::Lock,
            language: None,
            // Keywhere's identity is the navy dark look; System/Light are opt-in.
            theme: ThemeChoice::Dark,
            check_updates: None,
        }
    }
}

pub const IDLE_CHOICES: [u32; 8] = [1, 2, 5, 10, 15, 30, 60, 0];
pub const CLIPBOARD_MIN: u32 = 5;
pub const CLIPBOARD_MAX: u32 = 120;

impl Settings {
    pub fn idle_lock(&self) -> Option<Duration> {
        (self.idle_lock_minutes > 0)
            .then(|| Duration::from_secs(self.idle_lock_minutes as u64 * 60))
    }

    pub fn clipboard_clear(&self) -> Duration {
        Duration::from_secs(
            self.clipboard_clear_secs
                .clamp(CLIPBOARD_MIN, CLIPBOARD_MAX) as u64,
        )
    }

    fn parse(text: &str) -> Self {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "idle_lock_minutes" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.idle_lock_minutes = n.min(24 * 60);
                    }
                }
                "clipboard_clear_secs" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.clipboard_clear_secs = n.clamp(CLIPBOARD_MIN, CLIPBOARD_MAX);
                    }
                }
                "close_action" => {
                    // Only an explicit "hide" keeps the vault unlocked.
                    s.close_action = if v == "hide" {
                        CloseAction::Hide
                    } else {
                        CloseAction::Lock
                    }
                }
                "language" => s.language = Lang::from_code(v),
                "theme" => s.theme = ThemeChoice::from_code(v),
                "check_updates" => {
                    s.check_updates = match v {
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
        s
    }

    fn serialize(&self) -> String {
        format!(
            "# Keywhere settings\nidle_lock_minutes = {}\nclipboard_clear_secs = {}\nclose_action = {}\nlanguage = {}\ntheme = {}\ncheck_updates = {}\n",
            self.idle_lock_minutes,
            self.clipboard_clear_secs,
            match self.close_action {
                CloseAction::Hide => "hide",
                CloseAction::Lock => "lock",
            },
            self.language.map(Lang::code).unwrap_or("system"),
            self.theme.code(),
            match self.check_updates {
                Some(true) => "true",
                Some(false) => "false",
                None => "ask",
            },
        )
    }
}

static SETTINGS: RwLock<Option<Settings>> = RwLock::new(None);

pub fn get() -> Settings {
    if let Some(s) = *SETTINGS.read().unwrap_or_else(|e| e.into_inner()) {
        return s;
    }
    let s = crate::config::read_file("settings")
        .map(|t| Settings::parse(&t))
        .unwrap_or_default();
    *SETTINGS.write().unwrap_or_else(|e| e.into_inner()) = Some(s);
    s
}

/// Change settings and persist them.
pub fn update(f: impl FnOnce(&mut Settings)) {
    let mut s = get();
    f(&mut s);
    *SETTINGS.write().unwrap_or_else(|e| e.into_inner()) = Some(s);
    crate::config::write_file("settings", &s.serialize());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_clamping() {
        let s = Settings {
            idle_lock_minutes: 15,
            clipboard_clear_secs: 45,
            close_action: CloseAction::Lock,
            language: Some(Lang::Ro),
            theme: ThemeChoice::Light,
            check_updates: Some(false),
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
        let bad = Settings::parse("clipboard_clear_secs = 9999\nlanguage = xx\nnonsense\n");
        assert_eq!(bad.clipboard_clear_secs, CLIPBOARD_MAX);
        assert_eq!(bad.language, None);
        assert_eq!(Settings::parse("idle_lock_minutes = 0").idle_lock(), None);
    }

    #[test]
    fn close_locks_unless_hide_is_chosen() {
        assert_eq!(Settings::default().close_action, CloseAction::Lock);
        assert_eq!(Settings::parse("").close_action, CloseAction::Lock);
        assert_eq!(
            Settings::parse("close_action = bogus").close_action,
            CloseAction::Lock
        );
        assert_eq!(
            Settings::parse("close_action = hide").close_action,
            CloseAction::Hide
        );
    }

    #[test]
    fn update_checks_are_off_until_chosen() {
        assert_eq!(Settings::default().check_updates, None);
        assert_eq!(Settings::parse("check_updates = maybe").check_updates, None);
        assert_eq!(
            Settings::parse("check_updates = true").check_updates,
            Some(true)
        );
        let s = Settings {
            check_updates: None,
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&s.serialize()).check_updates, None);
    }
}

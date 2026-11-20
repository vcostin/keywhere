//! Navy palette (see docs/mockups) with a light variant.
//!
//! Views name colors by their *dark* value (the constants below) and go
//! through [`c`] / [`ca`]; in light mode those map each dark color to its
//! light counterpart, so no view needs theme-specific code.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{rgb, rgba, Rgba, WindowAppearance};

pub const BG: u32 = 0x0A1631;
pub const SURFACE: u32 = 0x0F2042;
pub const PANEL: u32 = 0x132852;
pub const RAISED: u32 = 0x1A3464;
pub const HOVER: u32 = 0x1A3360;
pub const BORDER: u32 = 0x23427A;
pub const LINE: u32 = 0x1B3560;
pub const TEXT: u32 = 0xE7EDF9;
pub const TEXT_SOFT: u32 = 0xC9D5EE;
pub const MUTED: u32 = 0x8EA3CB;
pub const DIM: u32 = 0x6077A3;
pub const ACCENT: u32 = 0xF2B24C;
pub const ACCENT_INK: u32 = 0x2A1A00;
pub const FOCUS: u32 = 0x5B8CFF;
pub const SELECTION: u32 = 0x244A8F;
pub const AVATAR: u32 = 0x1E3970;
pub const AVATAR_INK: u32 = 0xAFC3EA;
pub const LINK: u32 = 0x8FB2FF;
pub const WARN: u32 = 0xF7C04A;
pub const DANGER: u32 = 0xFF7A7A;
pub const OK: u32 = 0x4FD69C;

static LIGHT: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];

    pub fn code(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
        }
    }

    pub fn from_code(c: &str) -> ThemeChoice {
        match c {
            "dark" => ThemeChoice::Dark,
            "light" => ThemeChoice::Light,
            "system" => ThemeChoice::System,
            _ => ThemeChoice::Dark,
        }
    }
}

/// Resolve the setting against the desktop's appearance.
pub fn apply(choice: ThemeChoice, system: WindowAppearance) {
    let light = match choice {
        ThemeChoice::Dark => false,
        ThemeChoice::Light => true,
        ThemeChoice::System => matches!(
            system,
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    };
    LIGHT.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Light counterpart of a dark palette color (unknown colors pass through).
fn light(hex: u32) -> u32 {
    match hex {
        BG => 0xF3F6FB,
        SURFACE => 0xE8EDF6,
        PANEL => 0xFFFFFF,
        RAISED => 0xE2E8F3,
        HOVER => 0xDAE3F1,
        BORDER => 0xC5D0E4,
        LINE => 0xD9E0EC,
        TEXT => 0x14213D,
        TEXT_SOFT => 0x2C3E63,
        MUTED => 0x58698F,
        DIM => 0x7B8BAA,
        ACCENT => 0xE39A1F,
        FOCUS => 0x2F6BEA,
        SELECTION => 0xD3E0F8,
        AVATAR => 0xDCE4F2,
        AVATAR_INK => 0x3A4E78,
        LINK => 0x2459C8,
        WARN => 0xA86A12,
        DANGER => 0xC23A3A,
        OK => 0x1E9A60,
        // literal accents used by cards / chips / the window frame
        0x2A4A85 => 0xC3CEE3,
        0x3D62AB => 0xAEBDD9,
        0x4E7BE0 => 0x2F6BEA,
        0xCFDCFF => 0x1D4FB8,
        // modal backdrop stays dark (with alpha)
        other => other,
    }
}

fn resolve(hex: u32) -> u32 {
    if is_light() {
        light(hex)
    } else {
        hex
    }
}

pub fn c(hex: u32) -> Rgba {
    rgb(resolve(hex))
}

/// `hex` with alpha 0..=255.
pub fn ca(hex: u32, alpha: u8) -> Rgba {
    // The dark modal backdrop is lighter and softer in light mode.
    let (hex, alpha) = match (is_light(), hex) {
        (true, 0x050B1A) => (0x0A1631, alpha / 2),
        _ => (resolve(hex), alpha),
    };
    rgba((hex << 8) | alpha as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_mapping_differs_for_every_palette_color() {
        for col in [
            BG, SURFACE, PANEL, RAISED, HOVER, BORDER, LINE, TEXT, TEXT_SOFT, MUTED, DIM, FOCUS,
            SELECTION, LINK,
        ] {
            assert_ne!(light(col), col, "{col:06x}");
        }
    }
}

//! Navy palette (see docs/mockups). One warm accent, used sparingly.

use gpui::{rgb, rgba, Rgba};

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

pub fn c(hex: u32) -> Rgba {
    rgb(hex)
}

/// `hex` with alpha 0..=255.
pub fn ca(hex: u32, alpha: u8) -> Rgba {
    rgba((hex << 8) | alpha as u32)
}

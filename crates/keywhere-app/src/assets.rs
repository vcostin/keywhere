//! Assets compiled into the binary (no runtime file lookups).

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString, Styled};

pub struct Assets;

macro_rules! asset_table {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../../assets/", $name)) as &[u8])),*]
    };
}

static TABLE: &[(&str, &[u8])] = asset_table![
    "keywhere-logo.svg",
    "icons/check.svg",
    "icons/chevron-down.svg",
    "icons/chevron-right.svg",
    "icons/clock.svg",
    "icons/copy.svg",
    "icons/external.svg",
    "icons/eye-off.svg",
    "icons/eye.svg",
    "icons/file.svg",
    "icons/flag.svg",
    "icons/folder.svg",
    "icons/gear.svg",
    "icons/list.svg",
    "icons/lock.svg",
    "icons/search.svg",
    "icons/shield.svg",
    "icons/star-filled.svg",
    "icons/star.svg",
    "icons/warning.svg",
    "icons/win-close.svg",
    "icons/win-maximize.svg",
    "icons/win-minimize.svg",
    "icons/win-restore.svg",
];

/// The app logo, loaded from the embedded asset table (a bare string would be
/// parsed as a URI by `img`).
pub fn logo(size: f32) -> gpui::Img {
    gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded(
        "keywhere-logo.svg".into(),
    )))
    .size(gpui::px(size))
}

/// Inter (SIL OFL 1.1, see assets/fonts/Inter-LICENSE.txt), bundled so the UI
/// looks the same on every OS.
pub const UI_FONT: &str = "Inter";

pub fn load_fonts(cx: &mut gpui::App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../../../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../../../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../../../assets/fonts/Inter-Bold.ttf")),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("keywhere: could not load bundled fonts: {e}");
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(TABLE
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(TABLE
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

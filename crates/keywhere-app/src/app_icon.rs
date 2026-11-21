//! The amber key as the window's task-bar icon (Wayland xdg-toplevel-icon,
//! X11 `_NET_WM_ICON`), rendered from the logo SVG at the usual icon sizes.

const LOGO: &[u8] = include_bytes!("../../../assets/keywhere-key.svg");
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// Hand the icon to GPUI before any window opens.
pub fn install() {
    gpui::set_window_icon(Some("keywhere"), images());
}

fn images() -> Vec<gpui::WindowIconImage> {
    use resvg::{tiny_skia, usvg};
    let Ok(tree) = usvg::Tree::from_data(LOGO, &usvg::Options::default()) else {
        return Vec::new();
    };
    let view = tree.size();
    SIZES
        .iter()
        .filter_map(|&size| {
            let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
            let scale = size as f32 / view.width().max(view.height());
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            // tiny-skia stores premultiplied RGBA; GPUI wants straight RGBA.
            let rgba = pixmap
                .pixels()
                .iter()
                .flat_map(|p| {
                    let c = p.demultiply();
                    [c.red(), c.green(), c.blue(), c.alpha()]
                })
                .collect();
            Some(gpui::WindowIconImage { size, rgba })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_every_size_with_visible_pixels() {
        let imgs = super::images();
        assert_eq!(imgs.len(), super::SIZES.len());
        for img in imgs {
            assert_eq!(img.rgba.len(), (img.size * img.size * 4) as usize);
            assert!(
                img.rgba.chunks_exact(4).any(|p| p[3] == 255),
                "{} px is empty",
                img.size
            );
        }
    }
}

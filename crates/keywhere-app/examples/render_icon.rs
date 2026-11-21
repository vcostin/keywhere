//! Render the amber-key logo to PNGs for platform packaging (macOS `.icns`).
//!
//!   cargo run --release -p keywhere --example render_icon -- <out_dir> [--macos-plate]
//!
//! `--macos-plate` draws the key on Keywhere's navy rounded-square plate,
//! sized to Apple's app-icon grid (824 px body on a 1024 px canvas). Since
//! Big Sur, macOS shows a bare shape small on a grey fallback plate.
//!
//! Writes `icon_<size>.png` for 16…1024 px. The PNGs are stored uncompressed
//! (zlib "stored" blocks), so no PNG encoder dependency is needed;
//! `iconutil` recompresses them anyway.

use resvg::{tiny_skia, usvg};
use std::path::PathBuf;

const LOGO: &[u8] = include_bytes!("../../../assets/keywhere-key.svg");
const SIZES: [u32; 7] = [16, 32, 64, 128, 256, 512, 1024];

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    let plate = std::env::args().any(|a| a == "--macos-plate");
    std::fs::create_dir_all(&out).expect("create output directory");
    let svg = if plate {
        macos_plate(std::str::from_utf8(LOGO).expect("UTF-8 logo"))
    } else {
        String::from_utf8(LOGO.to_vec()).expect("UTF-8 logo")
    };
    let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).expect("logo SVG");
    let view = tree.size();
    for size in SIZES {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("pixmap");
        let scale = size as f32 / view.width().max(view.height());
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let rgba: Vec<u8> = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let path = out.join(format!("icon_{size}.png"));
        std::fs::write(&path, png(size, &rgba)).expect("write PNG");
        println!("{}", path.display());
    }
}

/// The logo (viewBox 0 0 256 256; the key itself spans about x 49–200,
/// y 35–214, centred near 124,124) on a navy plate in the app's colours,
/// the key about 62 % of the plate's height.
fn macos_plate(logo: &str) -> String {
    let open_end = logo.find('>').expect("<svg ...>") + 1;
    let close = logo.rfind("</svg>").expect("</svg>");
    let inner = &logo[open_end..close];
    // Scale 2.85: key height 179 * 2.85 ≈ 510 px; centre 124 * 2.85 ≈ 353,
    // so the nested viewport starts at 512 - 353 = 159.
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="kw-plate" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#1A3464"/>
      <stop offset="1" stop-color="#0A1631"/>
    </linearGradient>
  </defs>
  <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#kw-plate)"/>
  <rect x="102" y="102" width="820" height="820" rx="183" fill="none" stroke="#2A4A85" stroke-width="4"/>
  <svg x="159" y="159" width="729" height="729" viewBox="0 0 256 256">{inner}</svg>
</svg>"##
    )
}

/// A minimal RGBA8 PNG with an uncompressed zlib stream.
fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    let row = size as usize * 4;
    let mut raw = Vec::with_capacity((row + 1) * size as usize);
    for line in rgba.chunks_exact(row) {
        raw.push(0); // filter: none
        raw.extend_from_slice(line);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(b) = blocks.next() {
        z.push(blocks.peek().is_none() as u8);
        let len = b.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(b"IHDR", &ihdr), (b"IDAT", &z), (b"IEND", &Vec::new())] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &x in data {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

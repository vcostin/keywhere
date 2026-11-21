//! Render the amber-key logo to PNGs for platform packaging (macOS `.icns`).
//!
//!   cargo run --release -p keywhere --example render_icon -- <out_dir>
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
    std::fs::create_dir_all(&out).expect("create output directory");
    let tree = usvg::Tree::from_data(LOGO, &usvg::Options::default()).expect("logo SVG");
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

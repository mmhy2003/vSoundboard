//! Regenerates `assets/vsoundboard.ico` from `icon::disc_rgba`.
//!
//! cargo run --example make_icon

use vsoundboard::icon::disc_rgba;

const SIZES: [u32; 4] = [16, 32, 48, 256];

fn png(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, size, size);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(&disc_rgba(size)).expect("png data");
    drop(w);
    out
}

fn main() {
    let images: Vec<Vec<u8>> = SIZES.iter().map(|&s| png(s)).collect();
    let mut ico = Vec::new();
    // ICONDIR: reserved, type 1 (icon), image count.
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (size, data) in SIZES.iter().zip(&images) {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        // ICONDIRENTRY: width, height, palette, reserved, planes, bpp, bytes, offset.
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in &images {
        ico.extend_from_slice(data);
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vsoundboard.ico");
    std::fs::write(path, &ico).expect("write icon");
    println!("wrote {path} ({} bytes)", ico.len());
}

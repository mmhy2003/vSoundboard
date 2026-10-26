//! The app's icon. The artwork is `assets/logo.png`; `examples/make_icon.rs` turns it into
//! `assets/vsoundboard.ico`, which is embedded in the exe (build.rs) and in this module, so the
//! tray and window use exactly the same images.

pub const ICO: &[u8] = include_bytes!("../assets/vsoundboard.ico");

/// Image sizes in the .ico, smallest first.
pub const SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];

/// RGBA8 pixels of the `size`×`size` image stored in the .ico, if it has that size.
pub fn icon_rgba(size: u32) -> Option<Vec<u8>> {
    let count = u16::from_le_bytes([ICO[4], ICO[5]]) as usize;
    let entry = (0..count)
        .map(|i| &ICO[6 + i * 16..6 + (i + 1) * 16])
        .find(|e| {
            let w = if e[0] == 0 { 256 } else { e[0] as u32 };
            w == size
        })?;
    let len = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
    let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
    // make_icon writes every entry as an 8-bit RGBA PNG.
    let mut reader = png::Decoder::new(std::io::Cursor::new(ICO.get(offset..offset + len)?))
        .read_info()
        .ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.width != size || info.height != size {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_ico_has_expected_sizes() {
        assert_eq!(&ICO[0..4], &[0, 0, 1, 0], "ICONDIR header");
        let count = u16::from_le_bytes([ICO[4], ICO[5]]) as usize;
        let sizes: Vec<u32> = (0..count)
            .map(|i| match ICO[6 + i * 16] {
                0 => 256,
                w => w as u32,
            })
            .collect();
        assert_eq!(sizes, SIZES.to_vec());
    }

    #[test]
    fn every_size_decodes_to_rgba_with_transparency() {
        for size in SIZES {
            let px = icon_rgba(size).unwrap_or_else(|| panic!("size {size} missing"));
            assert_eq!(px.len(), (size * size * 4) as usize, "size {size}");
            let alpha: Vec<u8> = px.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
            assert_eq!(alpha[0], 0, "size {size}: corner should be transparent");
            assert!(alpha.contains(&255), "size {size}: has opaque pixels");
        }
    }

    #[test]
    fn unknown_size_is_none() {
        assert_eq!(icon_rgba(100), None);
    }
}

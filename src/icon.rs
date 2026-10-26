//! The app's icon: a green disc, drawn in code. Used for the tray, the window, and (via
//! `examples/make_icon.rs`) the committed `assets/vsoundboard.ico`.

/// RGBA8 pixels of a `size`×`size` anti-aliased green disc on a transparent background.
pub fn disc_rgba(size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let alpha = ((c + 0.5 - d).clamp(0.0, 1.0) * 255.0) as u8;
            rgba.extend_from_slice(&[46, 160, 100, alpha]);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_has_opaque_centre_and_clear_corners() {
        let px = disc_rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        let alpha = |x: usize, y: usize| px[(y * 32 + x) * 4 + 3];
        assert_eq!(alpha(16, 16), 255);
        assert_eq!(alpha(0, 0), 0);
    }

    #[test]
    fn committed_ico_has_expected_sizes() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/vsoundboard.ico"
        ))
        .expect("assets/vsoundboard.ico exists");
        assert_eq!(&bytes[0..4], &[0, 0, 1, 0], "ICONDIR header");
        let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        let sizes: Vec<u32> = (0..count)
            .map(|i| match bytes[6 + i * 16] {
                0 => 256,
                w => w as u32,
            })
            .collect();
        assert_eq!(sizes, vec![16, 32, 48, 256]);
    }
}

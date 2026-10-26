use super::SharedParams;

pub type Frame = [f32; 2];

/// cable = mic (mono → both channels) * mic_gain + sound * pad_gain * sounds_gain.
/// In Go Live mode the sound is left out of the cable. Missing mic samples count as silence.
pub fn mix_cable(sound: &[Frame], mic: &[f32], p: &SharedParams, out: &mut [Frame]) {
    let sound_gain = if p.go_live() {
        0.0
    } else {
        p.pad_gain.load() * p.sounds_gain.load()
    };
    let mic_gain = p.mic_gain.load();
    for (i, o) in out.iter_mut().enumerate() {
        let s = sound.get(i).copied().unwrap_or([0.0; 2]);
        let m = mic.get(i).copied().unwrap_or(0.0) * mic_gain;
        *o = [
            (s[0] * sound_gain + m).clamp(-1.0, 1.0),
            (s[1] * sound_gain + m).clamp(-1.0, 1.0),
        ];
    }
}

/// monitor = sound * pad_gain * sounds_gain * monitor_gain.
pub fn mix_monitor(sound: &[Frame], p: &SharedParams, out: &mut [Frame]) {
    let gain = p.pad_gain.load() * p.sounds_gain.load() * p.monitor_gain.load();
    for (i, o) in out.iter_mut().enumerate() {
        let s = sound.get(i).copied().unwrap_or([0.0; 2]);
        *o = [
            (s[0] * gain).clamp(-1.0, 1.0),
            (s[1] * gain).clamp(-1.0, 1.0),
        ];
    }
}

/// Stereo frames → device layout: 1 channel = (L+R)/2; 2+ channels = L, R, then zeros.
pub fn write_interleaved(frames: &[Frame], channels: usize, out: &mut [f32]) {
    if channels == 0 {
        return;
    }
    for (f, chunk) in frames.iter().zip(out.chunks_mut(channels)) {
        if channels == 1 {
            chunk[0] = (f[0] + f[1]) * 0.5;
        } else {
            chunk[0] = f[0];
            chunk[1] = f[1];
            chunk[2..].fill(0.0);
        }
    }
}

/// Device layout → mono, averaging all channels. Reuses `out`'s capacity.
pub fn downmix_mono(input: &[f32], channels: usize, out: &mut Vec<f32>) {
    out.clear();
    if channels == 0 {
        return;
    }
    let scale = 1.0 / channels as f32;
    out.extend(
        input
            .chunks_exact(channels)
            .map(|c| c.iter().sum::<f32>() * scale),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SharedParams {
        SharedParams::default()
    }

    fn approx(a: &[Frame], b: &[Frame]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!(
                (x[0] - y[0]).abs() < 1e-6 && (x[1] - y[1]).abs() < 1e-6,
                "{a:?} != {b:?}"
            );
        }
    }

    #[test]
    fn go_live_removes_sound_from_cable_only() {
        let p = params();
        let mut out = [[0.0; 2]; 1];
        mix_cable(&[[0.5, 0.5]], &[0.2], &p, &mut out);
        approx(&out, &[[0.7, 0.7]]);

        p.go_live.store(true, std::sync::atomic::Ordering::Relaxed);
        mix_cable(&[[0.5, 0.5]], &[0.2], &p, &mut out);
        approx(&out, &[[0.2, 0.2]]);
        mix_monitor(&[[0.5, 0.5]], &p, &mut out);
        approx(&out, &[[0.5, 0.5]]);
    }

    #[test]
    fn gains_multiply() {
        let p = params();
        p.sounds_gain.store(0.5);
        p.pad_gain.store(0.5);
        p.monitor_gain.store(0.5);
        let mut out = [[0.0; 2]; 1];
        mix_monitor(&[[1.0, 1.0]], &p, &mut out);
        approx(&out, &[[0.125, 0.125]]);
        mix_cable(&[[1.0, 1.0]], &[0.0], &p, &mut out);
        approx(&out, &[[0.25, 0.25]]);
    }

    #[test]
    fn mic_gain_only_affects_mic() {
        let p = params();
        p.mic_gain.store(0.0);
        let mut out = [[0.0; 2]; 1];
        mix_cable(&[[0.3, -0.3]], &[0.9], &p, &mut out);
        approx(&out, &[[0.3, -0.3]]);
        mix_monitor(&[[0.3, -0.3]], &p, &mut out);
        approx(&out, &[[0.3, -0.3]]);
    }

    #[test]
    fn clamps() {
        let p = params();
        let mut out = [[0.0; 2]; 1];
        mix_cable(&[[0.9, -0.9]], &[0.9], &p, &mut out);
        assert_eq!(out[0][0], 1.0);
        mix_cable(&[[-0.9, -0.9]], &[-0.9], &p, &mut out);
        assert_eq!(out[0], [-1.0, -1.0]);
    }

    #[test]
    fn mic_shorter_than_sound_pads_with_silence() {
        let p = params();
        let mut out = [[0.0; 2]; 3];
        mix_cable(&[[0.1, 0.1], [0.2, 0.2], [0.3, 0.3]], &[0.5], &p, &mut out);
        approx(&out, &[[0.6, 0.6], [0.2, 0.2], [0.3, 0.3]]);
    }

    #[test]
    fn interleave_mono_and_multichannel() {
        let mut mono = [0.0; 1];
        write_interleaved(&[[0.2, 0.4]], 1, &mut mono);
        assert!((mono[0] - 0.3).abs() < 1e-6);

        let mut eight = [9.0; 8];
        write_interleaved(&[[0.2, 0.4]], 8, &mut eight);
        assert_eq!(eight, [0.2, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

        let mut stereo = [0.0; 4];
        write_interleaved(&[[0.1, 0.2], [0.3, 0.4]], 2, &mut stereo);
        assert_eq!(stereo, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn downmix() {
        let mut out = Vec::new();
        downmix_mono(&[0.2, 0.4, 1.0, 0.0], 2, &mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.3).abs() < 1e-6 && (out[1] - 0.5).abs() < 1e-6);

        downmix_mono(&[0.7, 0.1], 1, &mut out);
        assert_eq!(out, vec![0.7, 0.1]);
    }
}

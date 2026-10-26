use std::fs::File;
use std::path::Path;
use std::time::Duration;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::Frame;

/// Streams a sound file as stereo frames at the file's own sample rate.
pub struct SoundReader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    sample_rate: u32,
    num_frames: Option<u64>,
    scratch: Vec<f32>,
}

fn unreadable(reason: impl std::fmt::Display) -> String {
    format!("Can't read this file: {reason}")
}

impl SoundReader {
    /// Errors are user-facing: "File not found: …" or "Can't read this file: …".
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("File not found: {}", path.display()),
            _ => unreadable(e),
        })?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|_| unreadable("unsupported format"))?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| unreadable("no audio in it"))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| unreadable("unsupported format"))?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|_| unreadable("unsupported codec"))?;
        let sample_rate = params
            .sample_rate
            .filter(|r| (1_000..=768_000).contains(r))
            .ok_or_else(|| unreadable("unsupported sample rate"))?;
        let track_id = track.id;
        let num_frames = track.num_frames;
        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            num_frames,
            scratch: Vec::new(),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn total_duration(&self) -> Option<Duration> {
        self.num_frames
            .map(|n| Duration::from_secs_f64(n as f64 / self.sample_rate as f64))
    }

    /// Next decoded chunk as stereo frames; Ok(None) at end of file.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<Frame>>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(unreadable(e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let buf = match self.decoder.decode(&packet) {
                Ok(buf) => buf,
                // A corrupt frame: skip it and keep going.
                Err(SymError::DecodeError(_)) => continue,
                Err(e) => return Err(unreadable(e)),
            };
            let channels = buf.spec().channels().count().max(1);
            self.scratch.resize(buf.samples_interleaved(), 0.0);
            buf.copy_to_slice_interleaved(&mut self.scratch);
            let frames = self
                .scratch
                .chunks_exact(channels)
                .map(|c| {
                    if channels == 1 {
                        [c[0], c[0]]
                    } else {
                        [c[0], c[1]]
                    }
                })
                .collect();
            return Ok(Some(frames));
        }
    }
}

const RESAMPLE_CHUNK: usize = 1024;

/// Converts stereo frames between sample rates, buffering input internally so any chunk size works.
pub struct RateConverter {
    resampler: Option<Async<f32>>,
    ratio: f64,
    pending: Vec<f32>,
    out_buf: Vec<f32>,
    delay_left: usize,
    frames_in: u64,
    frames_out: u64,
}

impl RateConverter {
    pub fn new(from: u32, to: u32) -> Self {
        let ratio = to as f64 / from as f64;
        let resampler = (from != to).then(|| {
            let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
                .oversampling_factor(256)
                .interpolation(SincInterpolationType::Quadratic);
            Async::<f32>::new_sinc(ratio, 1.1, &params, RESAMPLE_CHUNK, 2, FixedAsync::Input)
                .expect("valid resampler parameters")
        });
        let (out_max, delay) = resampler
            .as_ref()
            .map(|r| (r.output_frames_max(), r.output_delay()))
            .unwrap_or((0, 0));
        Self {
            resampler,
            ratio,
            pending: Vec::new(),
            out_buf: vec![0.0; out_max * 2],
            delay_left: delay,
            frames_in: 0,
            frames_out: 0,
        }
    }

    pub fn process(&mut self, input: &[Frame]) -> Vec<Frame> {
        if self.resampler.is_none() {
            return input.to_vec();
        }
        self.frames_in += input.len() as u64;
        self.pending.extend(input.iter().flatten());
        let mut out = Vec::new();
        while self.pending.len() / 2 >= self.next_needed() {
            self.run_chunk(None, &mut out);
        }
        out
    }

    /// Drain the remaining tail at end of file.
    pub fn flush(&mut self) -> Vec<Frame> {
        if self.resampler.is_none() {
            return Vec::new();
        }
        let expected = (self.frames_in as f64 * self.ratio).round() as u64;
        let mut out = Vec::new();
        // The first partial call takes the leftover input; later ones push zeros through
        // to drain the resampler's delay line.
        let mut guard = 0;
        while self.frames_out < expected && guard < 16 {
            let partial = self.pending.len() / 2;
            self.run_chunk(Some(partial), &mut out);
            guard += 1;
        }
        let over = self.frames_out.saturating_sub(expected) as usize;
        out.truncate(out.len().saturating_sub(over));
        self.frames_out -= over as u64;
        out
    }

    fn next_needed(&self) -> usize {
        self.resampler
            .as_ref()
            .map(|r| r.input_frames_next())
            .unwrap_or(0)
    }

    fn run_chunk(&mut self, partial: Option<usize>, out: &mut Vec<Frame>) {
        let need = self.next_needed();
        if self.pending.len() < need * 2 {
            self.pending.resize(need * 2, 0.0);
        }
        let in_frames = self.pending.len() / 2;
        let out_frames = self.out_buf.len() / 2;
        let resampler = self
            .resampler
            .as_mut()
            .expect("only called when resampling");
        let input = InterleavedSlice::new(&self.pending[..], 2, in_frames).expect("sized input");
        let mut output =
            InterleavedSlice::new_mut(&mut self.out_buf[..], 2, out_frames).expect("sized output");
        let indexing = Indexing {
            partial_len: partial,
            ..Indexing::new()
        };
        let (n_in, n_out) = resampler
            .process_into_buffer(&input, &mut output, Some(&indexing))
            .expect("buffers sized from the resampler");
        match partial {
            // Anything beyond the valid frames was zero padding.
            Some(_) => self.pending.clear(),
            None => {
                self.pending.drain(..n_in * 2);
            }
        }
        let skip = self.delay_left.min(n_out);
        self.delay_left -= skip;
        let produced = &self.out_buf[skip * 2..n_out * 2];
        out.extend(produced.as_chunks::<2>().0.iter().copied());
        self.frames_out += (n_out - skip) as u64;
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::Path;

    /// Write a WAV where every frame is `frame` (one value per channel).
    pub fn write_wav(path: &Path, rate: u32, bits: u16, frame: &[f32], frames: usize) {
        let spec = hound::WavSpec {
            channels: frame.len() as u16,
            sample_rate: rate,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        let scale = (1i64 << (bits - 1)) as f32 - 1.0;
        for _ in 0..frames {
            for &v in frame {
                let s = (v * scale).round() as i32;
                match bits {
                    8 => w.write_sample(s as i8).unwrap(),
                    16 => w.write_sample(s as i16).unwrap(),
                    _ => w.write_sample(s).unwrap(),
                }
            }
        }
        w.finalize().unwrap();
    }

    /// 4 KB of deterministic noise, which no decoder accepts.
    pub fn write_garbage(path: &Path) {
        let mut x: u32 = 0x1234_5678;
        let bytes: Vec<u8> = (0..4096)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect();
        std::fs::write(path, bytes).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;

    fn read_all(r: &mut SoundReader) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(chunk) = r.next_chunk().unwrap() {
            out.extend(chunk);
        }
        out
    }

    #[test]
    fn reads_mono_22050() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("mono.wav");
        write_wav(&p, 22_050, 16, &[0.5], 22_050);
        let mut r = SoundReader::open(&p).unwrap();
        assert_eq!(r.sample_rate(), 22_050);
        let dur = r.total_duration().unwrap();
        assert!((dur.as_secs_f64() - 1.0).abs() < 0.01, "{dur:?}");
        let frames = read_all(&mut r);
        assert_eq!(frames.len(), 22_050);
        assert!(
            frames
                .iter()
                .all(|f| (f[0] - 0.5).abs() < 1e-3 && (f[1] - 0.5).abs() < 1e-3)
        );
    }

    #[test]
    fn reads_96k_24bit_stereo() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("hi.wav");
        write_wav(&p, 96_000, 24, &[0.25, -0.25], 48_000);
        let mut r = SoundReader::open(&p).unwrap();
        assert_eq!(r.sample_rate(), 96_000);
        let frames = read_all(&mut r);
        assert_eq!(frames.len(), 48_000);
        assert!(
            frames
                .iter()
                .all(|f| (f[0] - 0.25).abs() < 1e-3 && (f[1] + 0.25).abs() < 1e-3)
        );
    }

    #[test]
    fn takes_first_two_of_many_channels() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("quad.wav");
        write_wav(&p, 48_000, 16, &[0.1, 0.2, 0.3, 0.4], 100);
        let frames = read_all(&mut SoundReader::open(&p).unwrap());
        assert!(
            frames
                .iter()
                .all(|f| (f[0] - 0.1).abs() < 1e-3 && (f[1] - 0.2).abs() < 1e-3)
        );
    }

    #[test]
    fn zero_length_wav() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("empty.wav");
        write_wav(&p, 44_100, 16, &[0.0, 0.0], 0);
        let mut r = SoundReader::open(&p).unwrap();
        assert_eq!(read_all(&mut r).len(), 0);
    }

    #[test]
    fn garbage_is_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.mp3");
        write_garbage(&p);
        let err = SoundReader::open(&p)
            .err()
            .expect("garbage should not open");
        assert!(err.starts_with("Can't read this file"), "{err}");
    }

    #[test]
    fn missing_file_is_error() {
        let err = SoundReader::open(Path::new(r"C:\definitely\not\here.wav"))
            .err()
            .unwrap();
        assert!(err.starts_with("File not found"), "{err}");
    }

    #[test]
    fn converter_44100_to_48000_length() {
        let mut c = RateConverter::new(44_100, 48_000);
        let mut total = 0;
        let input = vec![[0.1, 0.1]; 1000];
        for _ in 0..44 {
            total += c.process(&input).len();
        }
        total += c.process(&input[..100]).len();
        total += c.flush().len();
        assert!((total as i64 - 48_000).abs() <= 512, "{total}");
    }

    #[test]
    fn converter_preserves_level() {
        let mut c = RateConverter::new(44_100, 48_000);
        let mut out = Vec::new();
        for _ in 0..20 {
            out.extend(c.process(&vec![[0.5, -0.5]; 1000]));
        }
        // Away from the edges a constant input stays constant.
        let mid = &out[out.len() / 4..out.len() / 2];
        assert!(
            mid.iter()
                .all(|f| (f[0] - 0.5).abs() < 0.01 && (f[1] + 0.5).abs() < 0.01)
        );
    }

    #[test]
    fn converter_passthrough_when_equal() {
        let mut c = RateConverter::new(48_000, 48_000);
        let input: Vec<Frame> = (0..777).map(|i| [i as f32, -(i as f32)]).collect();
        assert_eq!(c.process(&input), input);
        assert!(c.flush().is_empty());
    }

    #[test]
    #[ignore = "needs local mp3/ogg/flac files in target/fixtures"]
    fn decodes_each_format() {
        for name in ["tone.mp3", "tone.ogg", "tone.flac"] {
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/fixtures")
                .join(name);
            let frames = read_all(&mut SoundReader::open(&p).unwrap());
            assert!(!frames.is_empty(), "{name}");
        }
    }

    #[test]
    fn zero_sample_rate_is_error() {
        // A hand-built WAV whose fmt chunk claims 0 Hz.
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36u32 + 8).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&2u16.to_le_bytes()); // channels
        b.extend_from_slice(&0u32.to_le_bytes()); // sample rate
        b.extend_from_slice(&0u32.to_le_bytes()); // byte rate
        b.extend_from_slice(&4u16.to_le_bytes()); // block align
        b.extend_from_slice(&16u16.to_le_bytes()); // bits
        b.extend_from_slice(b"data");
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("zero.wav");
        std::fs::write(&p, b).unwrap();
        let err = SoundReader::open(&p).err().expect("0 Hz must not open");
        assert!(err.starts_with("Can't read this file"), "{err}");
    }
}

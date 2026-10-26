//! Manual smoke test for the audio engine against real devices.
//!
//! cargo run --example smoke -- [--mic NAME] [--cable NAME] [--monitor NAME] [--secs N] [--file PATH]
//!
//! Plays a sound (a generated 440 Hz tone unless --file is given) through the engine and, when the
//! cable is VB-Cable, records "CABLE Output" to confirm the sound and mic actually arrive there.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vsoundboard::audio::SharedParams;
use vsoundboard::audio::engine::{self, EngineCommand, EngineEvent};
use vsoundboard::config::DeviceNames;

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let names = DeviceNames {
        mic: arg(&args, "--mic"),
        cable: arg(&args, "--cable")
            .or_else(|| Some("CABLE Input (VB-Audio Virtual Cable)".into())),
        monitor: arg(&args, "--monitor"),
    };
    let secs: u64 = arg(&args, "--secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let file = arg(&args, "--file").map(Into::into).unwrap_or_else(|| {
        let p = std::env::temp_dir().join("vsoundboard-smoke-tone.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for n in 0..44_100 * 2 {
            let v = ((n as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.3 * 32_767.0)
                as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        p
    });
    println!(
        "devices: {names:?}\nfile: {}",
        std::path::Path::new(&file).display()
    );

    // Record what Discord would hear.
    let rms = Arc::new(Mutex::new((0.0f64, 0u64)));
    let host = cpal::default_host();
    let capture = host
        .input_devices()
        .unwrap()
        .find(|d| d.to_string().starts_with("CABLE Output"))
        .map(|dev| {
            let cfg = dev.default_input_config().unwrap();
            let r = rms.clone();
            let s = dev
                .build_input_stream::<f32, _, _>(
                    cfg.config(),
                    move |data, _| {
                        let mut g = r.lock().unwrap();
                        g.0 += data.iter().map(|&x| (x * x) as f64).sum::<f64>();
                        g.1 += data.len() as u64;
                    },
                    |e| eprintln!("capture error: {e}"),
                    None,
                )
                .unwrap();
            s.play().unwrap();
            s
        });
    let take_rms = || {
        let mut g = rms.lock().unwrap();
        let v = if g.1 == 0 {
            0.0
        } else {
            (g.0 / g.1 as f64).sqrt()
        };
        *g = (0.0, 0);
        v
    };

    let repaints = Arc::new(AtomicU64::new(0));
    let rp = repaints.clone();
    let mut eng = engine::start(
        names,
        vec![(1, file, 1.0)],
        Arc::new(SharedParams::default()),
        Arc::new(move || {
            rp.fetch_add(1, Ordering::Relaxed);
        }),
    );

    std::thread::sleep(Duration::from_millis(1500));
    println!("status: {:?}", *eng.status.lock().unwrap());
    println!("cable RMS before sound (mic only): {:.6}", take_rms());

    eng.send(EngineCommand::PlayPad(1));
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(1000));
        while let Ok(ev) = eng.events.try_recv() {
            if !matches!(ev, EngineEvent::Devices(_)) {
                println!("event: {ev:?}");
            }
        }
        println!(
            "t={:>2}s cable RMS {:.6}  position {:?}  mic latency {:?}  mic underruns {}",
            start.elapsed().as_secs(),
            take_rms(),
            *eng.position.lock().unwrap(),
            *eng.mic_latency.lock().unwrap(),
            eng.mic_underruns.load(Ordering::Relaxed),
        );
    }
    eng.shutdown();
    drop(capture);
    println!("repaint callbacks: {}", repaints.load(Ordering::Relaxed));
}

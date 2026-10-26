//! Moves mic samples from the mic's clock to the cable's clock.
//!
//! The two devices run on independent crystals, so over hours the mic delivers slightly more or
//! fewer samples than the cable consumes. We resample with linear interpolation and nudge the
//! ratio (±0.5% max) to keep the ring's fill level near a small target, which keeps the added
//! delay constant without periodic dropouts.

use ringbuf::traits::Consumer;

const FILL_SMOOTHING: f32 = 0.01;
const GAIN_PER_SAMPLE: f64 = 0.000_05;
const MAX_CORRECTION: f64 = 0.005;
const OVERFLOW_FACTOR: usize = 4;

pub struct DriftResampler {
    nominal_ratio: f64,
    ratio: f64,
    pos: f64,
    s0: f32,
    s1: f32,
    target_fill: usize,
    smoothed_fill: f32,
    priming: bool,
    underruns: u64,
}

impl DriftResampler {
    pub fn new(in_rate: u32, out_rate: u32, target_fill: usize) -> Self {
        let nominal_ratio = in_rate as f64 / out_rate as f64;
        Self {
            nominal_ratio,
            ratio: nominal_ratio,
            pos: 0.0,
            s0: 0.0,
            s1: 0.0,
            target_fill: target_fill.max(1),
            smoothed_fill: target_fill as f32,
            priming: true,
            underruns: 0,
        }
    }

    /// Pull mono mic samples from `input`, write exactly `out.len()` samples at the output rate.
    pub fn process<C: Consumer<Item = f32>>(&mut self, input: &mut C, out: &mut [f32]) {
        let mut fill = input.occupied_len();
        if fill > OVERFLOW_FACTOR * self.target_fill {
            input.skip(fill - self.target_fill);
            fill = self.target_fill;
        }

        if self.priming {
            if fill < self.target_fill {
                out.fill(0.0);
                return;
            }
            self.priming = false;
            self.smoothed_fill = fill as f32;
            let first = input.try_pop().unwrap_or(0.0);
            self.s0 = first;
            self.s1 = first;
            self.pos = 0.0;
        }

        self.smoothed_fill += FILL_SMOOTHING * (fill as f32 - self.smoothed_fill);
        let error = (self.smoothed_fill - self.target_fill as f32) as f64;
        let correction = (GAIN_PER_SAMPLE * error).clamp(-MAX_CORRECTION, MAX_CORRECTION);
        self.ratio = self.nominal_ratio * (1.0 + correction);

        for i in 0..out.len() {
            while self.pos >= 1.0 {
                match input.try_pop() {
                    Some(s) => {
                        self.s0 = self.s1;
                        self.s1 = s;
                        self.pos -= 1.0;
                    }
                    None => {
                        self.underruns += 1;
                        self.priming = true;
                        out[i..].fill(0.0);
                        return;
                    }
                }
            }
            out[i] = self.s0 + (self.s1 - self.s0) * self.pos as f32;
            self.pos += self.ratio;
        }
    }

    pub fn underruns(&self) -> u64 {
        self.underruns
    }

    pub fn smoothed_fill(&self) -> f32 {
        self.smoothed_fill
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::{HeapRb, traits::*};

    /// Event-driven simulation of two independent device clocks: the mic pushes 10 ms bursts
    /// every `10 ms / (1 + skew)` and the cable pulls 10 ms blocks every 10 ms. Because the clocks
    /// differ, the phase between the two callbacks rotates, which is the case that causes dropouts.
    struct Sim {
        prod: ringbuf::HeapProd<f32>,
        cons: ringbuf::HeapCons<f32>,
        rs: DriftResampler,
        in_burst: usize,
        out_block: usize,
        t_push: f64,
        t_pull: f64,
        n_in: u64,
        out: Vec<f32>,
    }

    /// Target used by the engine: one mic period + one cable period + 2 ms of margin.
    /// Both periods are 10 ms here, expressed in input samples.
    fn target_for(in_rate: u32, _out_rate: u32) -> usize {
        let period = in_rate as usize / 100;
        period + period + in_rate as usize / 500
    }

    impl Sim {
        fn new(in_rate: u32, out_rate: u32) -> Self {
            let (prod, cons) = HeapRb::<f32>::new(in_rate as usize / 5).split(); // 200 ms
            Self {
                prod,
                cons,
                rs: DriftResampler::new(in_rate, out_rate, target_for(in_rate, out_rate)),
                in_burst: in_rate as usize / 100,
                out_block: out_rate as usize / 100,
                // Start the mic a third of a period ahead so the phase isn't trivially aligned.
                t_push: 0.0,
                t_pull: 0.0033,
                n_in: 0,
                out: vec![0.0; out_rate as usize / 100],
            }
        }

        /// Advance to the next cable callback, delivering any mic bursts due before it.
        fn tick(&mut self, skew: f64, signal: impl Fn(u64) -> f32, produce: bool) -> &[f32] {
            while self.t_push <= self.t_pull {
                if produce {
                    for _ in 0..self.in_burst {
                        let _ = self.prod.try_push(signal(self.n_in));
                        self.n_in += 1;
                    }
                }
                self.t_push += 0.01 / (1.0 + skew);
            }
            self.t_pull += 0.01;
            self.out.resize(self.out_block, 0.0);
            self.rs.process(&mut self.cons, &mut self.out);
            &self.out
        }
    }

    #[test]
    fn same_rate_no_skew_preserves_ramp_slope() {
        let mut sim = Sim::new(48_000, 48_000);
        // Steep enough that f32 rounding is small next to the per-sample step, short enough to stay below 1.0.
        let slope = 1e-5;
        let mut all = Vec::new();
        for _ in 0..200 {
            all.extend_from_slice(sim.tick(0.0, |n| n as f32 * slope, true));
        }
        // Skip priming (first half second).
        let tail = &all[24_000..];
        assert!(tail.iter().all(|&v| v > 0.0), "silence after priming");
        for w in tail.windows(2) {
            let d = (w[1] - w[0]) as f64;
            assert!(
                (d / slope as f64 - 1.0).abs() < 0.02,
                "step {d} vs slope {slope}"
            );
        }
        assert_eq!(sim.rs.underruns(), 0);
    }

    fn holds_fill(skew: f64) {
        let mut sim = Sim::new(48_000, 48_000);
        let target = target_for(48_000, 48_000) as f32;
        for t in 0..360_000 {
            sim.tick(skew, |_| 0.1, true);
            if t > 1000 {
                let f = sim.rs.smoothed_fill();
                assert!(
                    f >= 0.5 * target && f <= 2.0 * target,
                    "tick {t}: fill {f} out of range (skew {skew})"
                );
            }
        }
        assert_eq!(sim.rs.underruns(), 0, "skew {skew}");
    }

    #[test]
    fn holds_fill_under_clock_mismatch_for_an_hour() {
        holds_fill(0.0005);
        holds_fill(-0.0005);
    }

    #[test]
    fn converts_44100_to_48000() {
        let mut sim = Sim::new(44_100, 48_000);
        let sine =
            |n: u64| (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 44_100.0).sin() as f32;
        let mut all = Vec::new();
        for _ in 0..300 {
            all.extend_from_slice(sim.tick(0.0, sine, true));
        }
        // Last second of output.
        let last = &all[all.len() - 48_000..];
        let crossings = last
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        let freq = crossings as f64 / 2.0;
        assert!((freq - 1000.0).abs() <= 2.0, "measured {freq} Hz");
    }

    #[test]
    fn underrun_recovers() {
        let mut sim = Sim::new(48_000, 48_000);
        for _ in 0..200 {
            sim.tick(0.0, |_| 0.5, true);
        }
        let mut silent_seen = false;
        for _ in 0..50 {
            let out = sim.tick(0.0, |_| 0.5, false);
            if out.iter().all(|&v| v == 0.0) {
                silent_seen = true;
            }
        }
        assert!(sim.rs.underruns() >= 1);
        assert!(silent_seen);
        let mut back = false;
        for _ in 0..20 {
            let out = sim.tick(0.0, |_| 0.5, true);
            if out.iter().all(|&v| (v - 0.5).abs() < 1e-6) {
                back = true;
            }
        }
        assert!(back, "signal did not return after re-priming");
    }

    #[test]
    fn overflow_trims_latency() {
        let (mut prod, mut cons) = HeapRb::<f32>::new(48_000).split();
        let target = 1056;
        let mut rs = DriftResampler::new(48_000, 48_000, target);
        for _ in 0..10 * target {
            prod.try_push(0.2).unwrap();
        }
        let mut out = vec![0.0; 480];
        rs.process(&mut cons, &mut out);
        assert!(
            cons.occupied_len() <= target + 480,
            "fill {}",
            cons.occupied_len()
        );
    }
}

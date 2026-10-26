//! The player thread: decodes the current sound and feeds it to each output's ring.
//!
//! Switching sounds uses a flush handshake so no tail of the previous sound leaks out: the player
//! bumps `flush_gen` on every feed, waits until each attached feed's audio callback has emptied
//! its ring and acknowledged, and only then starts pushing the new sound.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use super::decode::{RateConverter, SoundReader};
use super::{Frame, SharedParams};

pub enum PlayerCommand {
    Play { pad: u64, path: PathBuf, gain: f32 },
    Stop,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    Started { pad: u64, total: Option<Duration> },
    Finished { pad: u64 },
    Stopped { pad: u64 },
    FileError { pad: u64, message: String },
}

#[derive(Debug, Default)]
pub struct FeedShared {
    pub attached: AtomicBool,
    pub flush_gen: AtomicU64,
    pub acked_gen: AtomicU64,
    pub played: AtomicU64,
}

/// Consumer side of one output's sound ring, read from that output's audio callback.
pub struct SoundFeed {
    cons: HeapCons<Frame>,
    shared: Arc<FeedShared>,
}

impl SoundFeed {
    /// Fill `out` with sound frames (zeros when empty). Never blocks. Honors flush requests.
    pub fn read(&mut self, out: &mut [Frame]) {
        let wanted = self.shared.flush_gen.load(Ordering::Acquire);
        if wanted != self.shared.acked_gen.load(Ordering::Relaxed) {
            self.cons.clear();
            self.shared.played.store(0, Ordering::Relaxed);
            self.shared.acked_gen.store(wanted, Ordering::Release);
            out.fill([0.0; 2]);
            return;
        }
        let n = self.cons.pop_slice(out);
        out[n..].fill([0.0; 2]);
        self.shared.played.fetch_add(n as u64, Ordering::Relaxed);
    }
}

struct Playing {
    pad: u64,
    reader: SoundReader,
    /// One converter per feed that was attached when the sound started.
    convs: [Option<RateConverter>; 2],
    pending: [Vec<Frame>; 2],
    eof: bool,
}

struct Player {
    rates: [u32; 2],
    prods: [HeapProd<Frame>; 2],
    shared: [Arc<FeedShared>; 2],
    params: Arc<SharedParams>,
    events: Sender<PlayerEvent>,
    current: Option<Playing>,
}

impl Player {
    fn run(mut self, rx: Receiver<PlayerCommand>) {
        loop {
            let cmd = if self.current.is_some() {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            match cmd {
                Some(PlayerCommand::Play { pad, path, gain }) => self.start(pad, &path, gain),
                Some(PlayerCommand::Stop) => self.stop(),
                Some(PlayerCommand::Shutdown) => return,
                None => {}
            }
            if self.current.is_some() && !self.pump() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    fn attached(&self, i: usize) -> bool {
        self.shared[i].attached.load(Ordering::Relaxed)
    }

    /// Ask every feed to empty its ring and wait (up to 100 ms) for attached ones to confirm.
    fn flush(&self) {
        let gens: Vec<u64> = self
            .shared
            .iter()
            .map(|s| s.flush_gen.fetch_add(1, Ordering::AcqRel) + 1)
            .collect();
        let deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < deadline {
            let done = (0..2).all(|i| {
                !self.attached(i) || self.shared[i].acked_gen.load(Ordering::Acquire) == gens[i]
            });
            if done {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn stop(&mut self) {
        if let Some(p) = self.current.take() {
            self.flush();
            let _ = self.events.send(PlayerEvent::Stopped { pad: p.pad });
        }
    }

    fn start(&mut self, pad: u64, path: &Path, gain: f32) {
        self.stop();
        let reader = match SoundReader::open(path) {
            Ok(r) => r,
            Err(message) => {
                let _ = self.events.send(PlayerEvent::FileError { pad, message });
                return;
            }
        };
        // Nothing of the old sound is left in the rings, so the new gain can't touch its tail.
        self.params.pad_gain.store(gain);
        let rate = reader.sample_rate();
        let convs = [0, 1].map(|i| {
            self.attached(i)
                .then(|| RateConverter::new(rate, self.rates[i]))
        });
        let total = reader.total_duration();
        self.current = Some(Playing {
            pad,
            reader,
            convs,
            pending: [Vec::new(), Vec::new()],
            eof: false,
        });
        let _ = self.events.send(PlayerEvent::Started { pad, total });
    }

    /// Decode and push as much as fits. Returns false when nothing could move (rings full).
    fn pump(&mut self) -> bool {
        let Some(p) = self.current.as_mut() else {
            return false;
        };

        if p.pending.iter().all(|v| v.is_empty()) && !p.eof {
            match p.reader.next_chunk() {
                Ok(Some(frames)) => {
                    for i in 0..2 {
                        if let Some(c) = p.convs[i].as_mut() {
                            p.pending[i] = c.process(&frames);
                        }
                    }
                }
                Ok(None) => {
                    p.eof = true;
                    for i in 0..2 {
                        if let Some(c) = p.convs[i].as_mut() {
                            p.pending[i] = c.flush();
                        }
                    }
                }
                Err(message) => {
                    let pad = p.pad;
                    self.stop();
                    let _ = self.events.send(PlayerEvent::FileError { pad, message });
                    return true;
                }
            }
        }

        let mut moved = false;
        for i in 0..2 {
            if !self.shared[i].attached.load(Ordering::Relaxed) {
                p.pending[i].clear();
                continue;
            }
            let n = self.prods[i].push_slice(&p.pending[i]);
            if n > 0 {
                p.pending[i].drain(..n);
                moved = true;
            }
        }

        if p.eof && p.pending.iter().all(|v| v.is_empty()) {
            let drained = (0..2).all(|i| {
                !self.shared[i].attached.load(Ordering::Relaxed) || self.prods[i].is_empty()
            });
            if drained {
                let pad = p.pad;
                self.current = None;
                let _ = self.events.send(PlayerEvent::Finished { pad });
            }
            return false;
        }
        moved
    }
}

pub struct PlayerHandle {
    pub tx: Sender<PlayerCommand>,
    pub feeds_shared: [Arc<FeedShared>; 2],
    pub rates: [u32; 2],
    pub thread: Option<JoinHandle<()>>,
}

impl PlayerHandle {
    /// Time played so far, from the first attached feed.
    pub fn elapsed(&self) -> Option<Duration> {
        (0..2)
            .find(|&i| self.feeds_shared[i].attached.load(Ordering::Relaxed))
            .map(|i| {
                let played = self.feeds_shared[i].played.load(Ordering::Relaxed);
                Duration::from_secs_f64(played as f64 / self.rates[i] as f64)
            })
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(PlayerCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Feeds: index 0 = cable, 1 = monitor. Each ring holds 200 ms at its own rate.
pub fn spawn_player(
    rates: [u32; 2],
    params: Arc<SharedParams>,
    events: Sender<PlayerEvent>,
) -> (PlayerHandle, [SoundFeed; 2]) {
    let shared = [
        Arc::new(FeedShared::default()),
        Arc::new(FeedShared::default()),
    ];
    let [(p0, c0), (p1, c1)] = rates.map(|r| HeapRb::<Frame>::new((r as usize / 5).max(1)).split());
    let feeds = [
        SoundFeed {
            cons: c0,
            shared: shared[0].clone(),
        },
        SoundFeed {
            cons: c1,
            shared: shared[1].clone(),
        },
    ];
    let (tx, rx) = crossbeam_channel::unbounded();
    let player = Player {
        rates,
        prods: [p0, p1],
        shared: shared.clone(),
        params,
        events,
        current: None,
    };
    let thread = std::thread::Builder::new()
        .name("player".into())
        .spawn(move || player.run(rx))
        .expect("spawn player thread");
    let handle = PlayerHandle {
        tx,
        feeds_shared: shared,
        rates,
        thread: Some(thread),
    };
    (handle, feeds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decode::test_util::{write_garbage, write_wav};
    use crossbeam_channel::Receiver;
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    /// Reads both feeds in 10 ms blocks on its own thread, like two audio callbacks.
    struct Pump {
        got: Arc<Mutex<[Vec<Frame>; 2]>>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Pump {
        fn start(feeds: [SoundFeed; 2], rates: [u32; 2], read_feed: [bool; 2]) -> Self {
            let got = Arc::new(Mutex::new([Vec::new(), Vec::new()]));
            let stop = Arc::new(AtomicBool::new(false));
            let (g, s) = (got.clone(), stop.clone());
            let thread = std::thread::spawn(move || {
                let mut feeds = feeds;
                let mut bufs = [
                    vec![[0.0; 2]; rates[0] as usize / 100],
                    vec![[0.0; 2]; rates[1] as usize / 100],
                ];
                while !s.load(Ordering::Relaxed) {
                    for i in 0..2 {
                        if read_feed[i] {
                            feeds[i].read(&mut bufs[i]);
                            g.lock().unwrap()[i].extend_from_slice(&bufs[i]);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            Self {
                got,
                stop,
                thread: Some(thread),
            }
        }

        fn len(&self, i: usize) -> usize {
            self.got.lock().unwrap()[i].len()
        }

        fn since(&self, i: usize, from: usize) -> Vec<Frame> {
            self.got.lock().unwrap()[i][from..].to_vec()
        }
    }

    impl Drop for Pump {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    struct Rig {
        _dir: tempfile::TempDir,
        dir: PathBuf,
        player: PlayerHandle,
        events: Receiver<PlayerEvent>,
        pump: Pump,
    }

    fn rig(rates: [u32; 2], attached: [bool; 2]) -> Rig {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        let (tx, events) = crossbeam_channel::unbounded();
        let (player, feeds) = spawn_player(rates, Arc::new(SharedParams::default()), tx);
        for (shared, on) in player.feeds_shared.iter().zip(attached) {
            shared.attached.store(on, Ordering::Relaxed);
        }
        let pump = Pump::start(feeds, rates, attached);
        Rig {
            _dir: d,
            dir,
            player,
            events,
            pump,
        }
    }

    impl Rig {
        fn wav(&self, name: &str, level: f32, secs: f32) -> PathBuf {
            let p = self.dir.join(name);
            write_wav(&p, 48_000, 16, &[level, level], (48_000.0 * secs) as usize);
            p
        }

        fn play(&self, pad: u64, path: &Path) {
            self.player
                .tx
                .send(PlayerCommand::Play {
                    pad,
                    path: path.to_path_buf(),
                    gain: 1.0,
                })
                .unwrap();
        }

        fn wait_for(
            &self,
            want: impl Fn(&PlayerEvent) -> bool,
            timeout_ms: u64,
        ) -> Vec<PlayerEvent> {
            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            let mut seen = Vec::new();
            while Instant::now() < deadline {
                if let Ok(e) = self.events.recv_timeout(Duration::from_millis(10)) {
                    let done = want(&e);
                    seen.push(e);
                    if done {
                        return seen;
                    }
                }
            }
            panic!("timed out; saw {seen:?}");
        }
    }

    fn nonzero(frames: &[Frame]) -> Vec<Frame> {
        frames
            .iter()
            .copied()
            .filter(|f| f[0].abs() > 0.01 || f[1].abs() > 0.01)
            .collect()
    }

    #[test]
    fn plays_to_both_feeds() {
        let r = rig([48_000, 48_000], [true, true]);
        let a = r.wav("a.wav", 0.5, 0.3);
        r.play(1, &a);
        let seen = r.wait_for(|e| matches!(e, PlayerEvent::Finished { .. }), 3000);
        assert!(matches!(seen[0], PlayerEvent::Started { pad: 1, .. }));
        assert_eq!(seen.last(), Some(&PlayerEvent::Finished { pad: 1 }));
        for i in 0..2 {
            let got = nonzero(&r.pump.since(i, 0));
            assert!(
                (got.len() as i64 - 14_400).abs() <= 480,
                "feed {i}: {}",
                got.len()
            );
            assert!(got.iter().all(|f| (f[0] - 0.5).abs() < 1e-3));
        }
    }

    #[test]
    fn rapid_switches_never_leak_previous_sound() {
        let r = rig([48_000, 48_000], [true, true]);
        let a = r.wav("a.wav", 0.25, 5.0);
        let b = r.wav("b.wav", 0.75, 5.0);
        r.play(1, &a);
        for _ in 0..20 {
            r.play(2, &b);
            r.play(1, &a);
        }
        r.play(2, &b);
        // 42 plays in total; wait for the last one to start.
        let started = std::cell::Cell::new(0);
        let seen = r.wait_for(
            |e| {
                if matches!(e, PlayerEvent::Started { .. }) {
                    started.set(started.get() + 1);
                }
                started.get() == 42
            },
            5000,
        );
        assert_eq!(
            seen.last(),
            Some(&PlayerEvent::Started {
                pad: 2,
                total: seen_total(&seen)
            })
        );
        let from = r.pump.len(0);
        let from_mon = r.pump.len(1);
        std::thread::sleep(Duration::from_millis(400));
        for (i, start) in [(0, from), (1, from_mon)] {
            let got = nonzero(&r.pump.since(i, start));
            assert!(!got.is_empty(), "feed {i} got no sound");
            assert!(
                got.iter().all(|f| (f[0] - 0.75).abs() < 1e-3),
                "feed {i} leaked the previous sound"
            );
        }
        assert!(
            !seen
                .iter()
                .any(|e| matches!(e, PlayerEvent::Finished { .. }))
        );
        assert!(
            r.events
                .try_iter()
                .all(|e| !matches!(e, PlayerEvent::Finished { .. }))
        );
    }

    fn seen_total(seen: &[PlayerEvent]) -> Option<Duration> {
        match seen.last() {
            Some(PlayerEvent::Started { total, .. }) => *total,
            _ => None,
        }
    }

    #[test]
    fn stop_silences_and_reports() {
        let r = rig([48_000, 48_000], [true, true]);
        let a = r.wav("a.wav", 0.5, 5.0);
        r.play(1, &a);
        r.wait_for(|e| matches!(e, PlayerEvent::Started { .. }), 1000);
        std::thread::sleep(Duration::from_millis(100));
        r.player.tx.send(PlayerCommand::Stop).unwrap();
        r.wait_for(|e| *e == PlayerEvent::Stopped { pad: 1 }, 1000);
        // Two blocks of grace for a read already in flight.
        std::thread::sleep(Duration::from_millis(25));
        let from = r.pump.len(0);
        std::thread::sleep(Duration::from_millis(200));
        assert!(nonzero(&r.pump.since(0, from)).is_empty());
        assert!(nonzero(&r.pump.since(1, r.pump.len(1).min(from))).is_empty());
        assert!(
            r.events
                .try_iter()
                .all(|e| !matches!(e, PlayerEvent::Finished { .. }))
        );
    }

    #[test]
    fn retrigger_restarts() {
        let r = rig([48_000, 48_000], [true, true]);
        let a = r.wav("a.wav", 0.5, 5.0);
        r.play(1, &a);
        r.wait_for(|e| matches!(e, PlayerEvent::Started { .. }), 1000);
        std::thread::sleep(Duration::from_millis(300));
        let before = r.player.elapsed().unwrap();
        assert!(before >= Duration::from_millis(150), "{before:?}");
        r.play(1, &a);
        r.wait_for(|e| *e == PlayerEvent::Stopped { pad: 1 }, 1000);
        r.wait_for(|e| matches!(e, PlayerEvent::Started { pad: 1, .. }), 1000);
        let after = r.player.elapsed().unwrap();
        assert!(after < Duration::from_millis(100), "{after:?}");
    }

    #[test]
    fn garbage_file_reports_error() {
        let r = rig([48_000, 48_000], [true, true]);
        let g = r.dir.join("x.mp3");
        write_garbage(&g);
        r.play(7, &g);
        let seen = r.wait_for(|e| matches!(e, PlayerEvent::FileError { .. }), 1000);
        match seen.last().unwrap() {
            PlayerEvent::FileError { pad, message } => {
                assert_eq!(*pad, 7);
                assert!(message.starts_with("Can't read this file"), "{message}");
            }
            _ => unreachable!(),
        }
        // Still alive afterwards.
        let a = r.wav("a.wav", 0.5, 0.1);
        r.play(1, &a);
        r.wait_for(|e| *e == PlayerEvent::Finished { pad: 1 }, 2000);
    }

    #[test]
    fn detached_monitor_does_not_stall() {
        let r = rig([48_000, 48_000], [true, false]);
        let a = r.wav("a.wav", 0.5, 0.3);
        r.play(1, &a);
        r.wait_for(|e| *e == PlayerEvent::Finished { pad: 1 }, 3000);
    }

    #[test]
    fn rates_differ_per_feed() {
        let r = rig([48_000, 44_100], [true, true]);
        let a = r.wav("a.wav", 0.5, 0.3);
        r.play(1, &a);
        r.wait_for(|e| *e == PlayerEvent::Finished { pad: 1 }, 3000);
        let n0 = nonzero(&r.pump.since(0, 0)).len() as i64;
        let n1 = nonzero(&r.pump.since(1, 0)).len() as i64;
        assert!((n0 - 14_400).abs() <= 480, "cable {n0}");
        assert!((n1 - 13_230).abs() <= 441, "monitor {n1}");
    }
}

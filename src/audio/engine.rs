//! The engine thread owns every audio stream (cpal streams can't move between threads), the pad
//! table and the player. The UI, hotkeys and tray all talk to it through `EngineCommand`s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use ringbuf::HeapRb;
use ringbuf::traits::{Producer, Split};

use super::devices::{
    DeviceState, DeviceStatus, ROLES, Role, input_rate, list_inputs, list_outputs, open_input,
    open_output, output_rate, plan_reconnect, selected,
};
use super::mix::{downmix_mono, mix_cable, mix_monitor, write_interleaved};
use super::player::{PlayerCommand, PlayerEvent, PlayerHandle, spawn_player};
use super::{AtomicF32, DriftResampler, SharedParams};
use crate::config::{DeviceNames, RoutingMode};

pub enum EngineCommand {
    PlayPad(u64),
    Stop,
    ToggleMode,
    SetPads(Vec<(u64, PathBuf, f32)>),
    SetDevices(DeviceNames),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    Player(PlayerEvent),
    ModeChanged(RoutingMode),
    Devices(DeviceStatus),
}

/// (pad, elapsed, total)
pub type Position = Option<(u64, Duration, Option<Duration>)>;

pub struct EngineHandle {
    pub tx: Sender<EngineCommand>,
    pub events: Receiver<EngineEvent>,
    pub params: Arc<SharedParams>,
    pub status: Arc<Mutex<DeviceStatus>>,
    pub position: Arc<Mutex<Position>>,
    pub mic_latency: Arc<Mutex<Option<Duration>>>,
    /// Mic dropouts since the devices were last (re)opened; should stay 0.
    pub mic_underruns: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn send(&self, cmd: EngineCommand) {
        let _ = self.tx.send(cmd);
    }

    /// Stop all audio and join the engine thread.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(EngineCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn start(
    devices: DeviceNames,
    pads: Vec<(u64, PathBuf, f32)>,
    params: Arc<SharedParams>,
    on_event: Arc<dyn Fn() + Send + Sync>,
) -> EngineHandle {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (ev_tx, events) = crossbeam_channel::unbounded();
    let status = Arc::new(Mutex::new(DeviceStatus::default()));
    let position = Arc::new(Mutex::new(None));
    let mic_latency = Arc::new(Mutex::new(None));
    let mic_underruns = Arc::new(AtomicU64::new(0));
    let engine = Engine {
        names: devices,
        pads: pads.into_iter().map(|(id, p, g)| (id, (p, g))).collect(),
        params: params.clone(),
        events: ev_tx,
        on_event,
        status: status.clone(),
        position: position.clone(),
        mic_latency: mic_latency.clone(),
        mic_underruns: mic_underruns.clone(),
        rig: None,
        current: None,
        retry: RetryTimer::new(),
        last_lists: None,
    };
    let thread = std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || engine.run(rx))
        .expect("spawn engine thread");
    EngineHandle {
        tx,
        events,
        params,
        status,
        position,
        mic_latency,
        mic_underruns,
        thread: Some(thread),
    }
}

/// Names of the devices to open for [mic, cable, monitor]: selected and not known to be missing.
fn devices_to_build(names: &DeviceNames, planned: &DeviceStatus) -> [Option<String>; 3] {
    ROLES.map(|role| match planned.get(role) {
        DeviceState::Missing => None,
        _ => selected(names, role).map(str::to_string),
    })
}

/// When to retry a device that failed to open: right away if the device list changed,
/// otherwise after 5 s, then 10 s, 20 s, … up to a minute.
#[derive(Debug)]
struct RetryTimer {
    next: Option<Instant>,
    delay: Duration,
}

impl RetryTimer {
    fn new() -> Self {
        Self {
            next: None,
            delay: Duration::from_secs(5),
        }
    }

    fn due(&self, now: Instant, list_changed: bool) -> bool {
        list_changed || self.next.is_none_or(|t| now >= t)
    }

    fn failed(&mut self, now: Instant) {
        self.next = Some(now + self.delay);
        self.delay = (self.delay * 2).min(Duration::from_secs(60));
    }

    fn succeeded(&mut self) {
        *self = Self::new();
    }
}

/// How often device health is checked and the latency estimate refreshed.
const SUPERVISE_EVERY: Duration = Duration::from_secs(2);
/// How often the playback position is refreshed for the UI.
const TICK: Duration = Duration::from_millis(100);
/// Extra mic buffering beyond one mic period + one cable period, so callback jitter can't underrun.
const MIC_MARGIN: Duration = Duration::from_millis(2);
/// Callback buffers start this large and only grow if a device asks for more.
const SCRATCH_FRAMES: usize = 8192;
/// WASAPI shared mode's usual period; used for the mic target before real periods are known.
const ASSUMED_PERIOD: Duration = Duration::from_millis(10);

/// Live measurements written by the callbacks for the latency estimate.
#[derive(Default)]
struct Meters {
    mic_period: AtomicU32,
    cable_period: AtomicU32,
    mic_fill: AtomicF32Cell,
    mic_underruns: AtomicU64,
}

/// `AtomicF32` without a Default impl requirement on the shared type.
struct AtomicF32Cell(AtomicF32);

impl Default for AtomicF32Cell {
    fn default() -> Self {
        Self(AtomicF32::new(0.0))
    }
}

/// Everything that exists while devices are open. Dropped and rebuilt as a whole on any
/// device change, which keeps the wiring between streams, rings and the player simple.
struct Rig {
    player: Option<PlayerHandle>,
    /// This rig's own player events, so a torn-down player's late events are dropped with it.
    player_events: Receiver<PlayerEvent>,
    _streams: Vec<cpal::Stream>,
    lost: Vec<(Role, Arc<AtomicBool>)>,
    meters: Arc<Meters>,
    mic_rate: Option<u32>,
    cable_rate: Option<u32>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(p) = self.player.take() {
            p.shutdown();
        }
    }
}

struct Engine {
    names: DeviceNames,
    pads: HashMap<u64, (PathBuf, f32)>,
    params: Arc<SharedParams>,
    events: Sender<EngineEvent>,
    on_event: Arc<dyn Fn() + Send + Sync>,
    status: Arc<Mutex<DeviceStatus>>,
    position: Arc<Mutex<Position>>,
    mic_latency: Arc<Mutex<Option<Duration>>>,
    mic_underruns: Arc<AtomicU64>,
    rig: Option<Rig>,
    /// (pad, total) of what's playing.
    current: Option<(u64, Option<Duration>)>,
    retry: RetryTimer,
    /// Device lists seen at the last check, to notice plug/unplug.
    last_lists: Option<(Vec<String>, Vec<String>)>,
}

impl Engine {
    fn run(mut self, rx: Receiver<EngineCommand>) {
        self.supervise(true);
        let mut last_supervise = Instant::now();
        loop {
            let player_events = self
                .rig
                .as_ref()
                .map_or_else(crossbeam_channel::never, |r| r.player_events.clone());
            crossbeam_channel::select! {
                recv(rx) -> cmd => match cmd {
                    Ok(EngineCommand::Shutdown) | Err(_) => break,
                    Ok(cmd) => self.handle(cmd),
                },
                recv(player_events) -> ev => if let Ok(ev) = ev {
                    self.on_player_event(ev);
                },
                default(TICK) => {},
            }
            self.update_position();
            if last_supervise.elapsed() >= SUPERVISE_EVERY {
                self.supervise(false);
                self.update_latency();
                last_supervise = Instant::now();
            }
        }
        self.teardown();
    }

    /// Drop every stream and the player, telling the UI if a sound was cut off.
    fn teardown(&mut self) {
        self.rig = None;
        if let Some((pad, _)) = self.current.take() {
            self.update_position();
            self.emit(EngineEvent::Player(PlayerEvent::Stopped { pad }));
        }
    }

    fn emit(&self, ev: EngineEvent) {
        let _ = self.events.send(ev);
        (self.on_event)();
    }

    fn player(&self) -> Option<&PlayerHandle> {
        self.rig.as_ref().and_then(|r| r.player.as_ref())
    }

    fn handle(&mut self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::PlayPad(id) => {
                if let (Some((path, gain)), Some(player)) = (self.pads.get(&id), self.player()) {
                    let _ = player.tx.send(PlayerCommand::Play {
                        pad: id,
                        path: path.clone(),
                        gain: *gain,
                    });
                }
            }
            EngineCommand::Stop => {
                if let Some(player) = self.player() {
                    let _ = player.tx.send(PlayerCommand::Stop);
                }
            }
            EngineCommand::ToggleMode => {
                let live = !self.params.go_live();
                self.params.go_live.store(live, Ordering::Relaxed);
                let mode = if live {
                    RoutingMode::GoLive
                } else {
                    RoutingMode::Normal
                };
                self.emit(EngineEvent::ModeChanged(mode));
            }
            EngineCommand::SetPads(pads) => {
                self.pads = pads.into_iter().map(|(id, p, g)| (id, (p, g))).collect();
            }
            EngineCommand::SetDevices(names) => {
                self.names = names;
                self.teardown();
                self.retry.succeeded();
                self.set_status(DeviceStatus::default());
                self.supervise(true);
            }
            EngineCommand::Shutdown => {}
        }
    }

    fn on_player_event(&mut self, ev: PlayerEvent) {
        match &ev {
            PlayerEvent::Started { pad, total } => self.current = Some((*pad, *total)),
            PlayerEvent::Finished { pad } | PlayerEvent::Stopped { pad } => {
                if self.current.map(|c| c.0) == Some(*pad) {
                    self.current = None;
                }
            }
            PlayerEvent::FileError { .. } => {}
        }
        self.update_position();
        self.emit(EngineEvent::Player(ev));
    }

    fn update_position(&self) {
        let pos = self.current.and_then(|(pad, total)| {
            let elapsed = self.player()?.elapsed()?;
            Some((pad, elapsed, total))
        });
        *self.position.lock().unwrap() = pos;
    }

    fn update_latency(&self) {
        let est = self.rig.as_ref().and_then(|rig| {
            let (mic_rate, cable_rate) = (rig.mic_rate? as f64, rig.cable_rate? as f64);
            let m = &rig.meters;
            let mic_period = m.mic_period.load(Ordering::Relaxed) as f64;
            let cable_period = m.cable_period.load(Ordering::Relaxed) as f64;
            if mic_period == 0.0 || cable_period == 0.0 {
                return None;
            }
            // A sample waits on average for the ring's fill minus half the block the cable takes
            // per callback, plus one period of device buffering on each side.
            let ring =
                (m.mic_fill.0.load() as f64 - cable_period * mic_rate / cable_rate / 2.0).max(0.0);
            let secs = mic_period / mic_rate + ring / mic_rate + cable_period / cable_rate;
            Some(Duration::from_secs_f64(secs))
        });
        *self.mic_latency.lock().unwrap() = est;
        let underruns = self
            .rig
            .as_ref()
            .map_or(0, |r| r.meters.mic_underruns.load(Ordering::Relaxed));
        self.mic_underruns.store(underruns, Ordering::Relaxed);
    }

    fn set_status(&self, st: DeviceStatus) {
        let changed = {
            let mut cur = self.status.lock().unwrap();
            let changed = *cur != st;
            *cur = st.clone();
            changed
        };
        if changed {
            self.emit(EngineEvent::Devices(st));
        }
    }

    /// Check device health; rebuild everything if anything needs (re)opening or went missing.
    fn supervise(&mut self, force: bool) {
        let lost: Vec<Role> = self
            .rig
            .as_ref()
            .map(|r| {
                r.lost
                    .iter()
                    .filter(|(_, f)| f.load(Ordering::Relaxed))
                    .map(|(r, _)| *r)
                    .collect()
            })
            .unwrap_or_default();
        let inputs = list_inputs();
        let outputs = list_outputs();
        let current = self.status.lock().unwrap().clone();
        let now = Instant::now();
        let lists = Some((inputs.clone(), outputs.clone()));
        let list_changed = self.last_lists != lists;
        self.last_lists = lists;
        let retry_failed = force || self.retry.due(now, list_changed);
        let (to_open, planned) = plan_reconnect(
            &self.names,
            &inputs,
            &outputs,
            &current,
            &lost,
            retry_failed,
        );
        let went_away = ROLES
            .iter()
            .any(|&r| *current.get(r) == DeviceState::Open && *planned.get(r) != DeviceState::Open);
        if !force && to_open.is_empty() && !went_away {
            return;
        }
        // Rebuild every present device, not just the ones that changed.
        self.teardown();
        let (rig, status) = self.build(&planned);
        self.rig = Some(rig);
        if ROLES
            .iter()
            .any(|&r| matches!(status.get(r), DeviceState::Failed(_)))
        {
            self.retry.failed(now);
        } else {
            self.retry.succeeded();
        }
        self.set_status(status);
    }

    fn build(&self, planned: &DeviceStatus) -> (Rig, DeviceStatus) {
        let (player_tx, player_events) = crossbeam_channel::unbounded();
        let mut status = planned.clone();
        let [mic, cable, monitor] = devices_to_build(&self.names, planned);

        let fail = |status: &mut DeviceStatus, role: Role, e: String| {
            status.set(role, DeviceState::Failed(e))
        };
        let mic_rate = mic.as_deref().and_then(|n| {
            input_rate(n)
                .map_err(|e| fail(&mut status, Role::Mic, e))
                .ok()
        });
        let cable_rate = cable.as_deref().and_then(|n| {
            output_rate(n)
                .map_err(|e| fail(&mut status, Role::Cable, e))
                .ok()
        });
        let monitor_rate = monitor.as_deref().and_then(|n| {
            output_rate(n)
                .map_err(|e| fail(&mut status, Role::Monitor, e))
                .ok()
        });

        let meters = Arc::new(Meters::default());
        let mut streams = Vec::new();
        let mut lost = Vec::new();

        let player = (cable_rate.is_some() || monitor_rate.is_some()).then(|| {
            spawn_player(
                [cable_rate.unwrap_or(48_000), monitor_rate.unwrap_or(48_000)],
                self.params.clone(),
                player_tx,
            )
        });
        let (player, feeds) = match player {
            Some((h, [f0, f1])) => (Some(h), [Some(f0), Some(f1)]),
            None => (None, [None, None]),
        };
        let [cable_feed, monitor_feed] = feeds;

        // Mic → ring → cable callback.
        let mut mic_cons = None;
        if let (Some(name), Some(rate)) = (mic.as_deref(), mic_rate) {
            let (mut prod, cons) = HeapRb::<f32>::new(rate as usize / 5).split(); // 200 ms
            let flag = Arc::new(AtomicBool::new(false));
            let m = meters.clone();
            let mut mono = Vec::with_capacity(SCRATCH_FRAMES);
            let opened = open_input(
                name,
                move |data, channels| {
                    downmix_mono(data, channels, &mut mono);
                    m.mic_period.store(mono.len() as u32, Ordering::Relaxed);
                    prod.push_slice(&mono);
                },
                flag.clone(),
            );
            match opened {
                Ok((s, _)) => {
                    streams.push(s);
                    lost.push((Role::Mic, flag));
                    status.set(Role::Mic, DeviceState::Open);
                    mic_cons = Some(cons);
                }
                Err(e) => status.set(Role::Mic, DeviceState::Failed(e)),
            }
        }

        if let (Some(name), Some(rate), Some(mut feed)) = (cable.as_deref(), cable_rate, cable_feed)
        {
            let flag = Arc::new(AtomicBool::new(false));
            let params = self.params.clone();
            let m = meters.clone();
            let mut mic = mic_cons.map(|cons| {
                let in_rate = mic_rate.expect("mic open implies rate");
                let target = (ASSUMED_PERIOD * 2 + MIC_MARGIN).as_secs_f64() * in_rate as f64;
                (cons, DriftResampler::new(in_rate, rate, target as usize))
            });
            let mut sound = vec![[0.0f32; 2]; SCRATCH_FRAMES];
            let mut mic_buf = vec![0.0f32; SCRATCH_FRAMES];
            let mut mixed = vec![[0.0f32; 2]; SCRATCH_FRAMES];
            let opened = open_output(
                name,
                move |data, channels| {
                    let frames = data.len() / channels.max(1);
                    if sound.len() < frames {
                        sound.resize(frames, [0.0; 2]);
                        mic_buf.resize(frames, 0.0);
                        mixed.resize(frames, [0.0; 2]);
                    }
                    m.cable_period.store(frames as u32, Ordering::Relaxed);
                    feed.read(&mut sound[..frames]);
                    match mic.as_mut() {
                        Some((cons, rs)) => {
                            rs.process(cons, &mut mic_buf[..frames]);
                            m.mic_fill.0.store(rs.smoothed_fill());
                            m.mic_underruns.store(rs.underruns(), Ordering::Relaxed);
                        }
                        None => mic_buf[..frames].fill(0.0),
                    }
                    mix_cable(
                        &sound[..frames],
                        &mic_buf[..frames],
                        &params,
                        &mut mixed[..frames],
                    );
                    write_interleaved(&mixed[..frames], channels, data);
                },
                flag.clone(),
            );
            match opened {
                Ok((s, _)) => {
                    streams.push(s);
                    lost.push((Role::Cable, flag));
                    status.set(Role::Cable, DeviceState::Open);
                    if let Some(p) = &player {
                        p.feeds_shared[0].attached.store(true, Ordering::Relaxed);
                    }
                }
                Err(e) => status.set(Role::Cable, DeviceState::Failed(e)),
            }
        }

        if let (Some(name), Some(_), Some(mut feed)) =
            (monitor.as_deref(), monitor_rate, monitor_feed)
        {
            let flag = Arc::new(AtomicBool::new(false));
            let params = self.params.clone();
            let mut sound = vec![[0.0f32; 2]; SCRATCH_FRAMES];
            let mut mixed = vec![[0.0f32; 2]; SCRATCH_FRAMES];
            let opened = open_output(
                name,
                move |data, channels| {
                    let frames = data.len() / channels.max(1);
                    if sound.len() < frames {
                        sound.resize(frames, [0.0; 2]);
                        mixed.resize(frames, [0.0; 2]);
                    }
                    feed.read(&mut sound[..frames]);
                    mix_monitor(&sound[..frames], &params, &mut mixed[..frames]);
                    write_interleaved(&mixed[..frames], channels, data);
                },
                flag.clone(),
            );
            match opened {
                Ok((s, _)) => {
                    streams.push(s);
                    lost.push((Role::Monitor, flag));
                    status.set(Role::Monitor, DeviceState::Open);
                    if let Some(p) = &player {
                        p.feeds_shared[1].attached.store(true, Ordering::Relaxed);
                    }
                }
                Err(e) => status.set(Role::Monitor, DeviceState::Failed(e)),
            }
        }

        let rig = Rig {
            player,
            player_events,
            _streams: streams,
            lost,
            meters,
            mic_rate,
            cable_rate,
        };
        (rig, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn retry_backs_off_and_resets() {
        let t0 = Instant::now();
        let mut r = RetryTimer::new();
        assert!(r.due(t0, false));
        r.failed(t0);
        assert!(!r.due(t0 + Duration::from_secs(1), false));
        assert!(
            r.due(t0 + Duration::from_secs(1), true),
            "a changed device list retries at once"
        );
        assert!(r.due(t0 + Duration::from_secs(5), false));
        r.failed(t0 + Duration::from_secs(5));
        assert!(!r.due(t0 + Duration::from_secs(14), false));
        assert!(r.due(t0 + Duration::from_secs(15), false));
        for _ in 0..10 {
            r.failed(t0);
        }
        assert!(
            r.due(t0 + Duration::from_secs(60), false),
            "capped at a minute"
        );
        r.succeeded();
        assert!(r.due(t0, false));
    }

    fn bare_engine() -> (Engine, Receiver<EngineEvent>) {
        let (events, rx) = crossbeam_channel::unbounded();
        let engine = Engine {
            names: DeviceNames::default(),
            pads: HashMap::new(),
            params: Arc::new(SharedParams::default()),
            events,
            on_event: Arc::new(|| {}),
            status: Arc::default(),
            position: Arc::default(),
            mic_latency: Arc::default(),
            mic_underruns: Arc::default(),
            rig: None,
            current: None,
            retry: RetryTimer::new(),
            last_lists: None,
        };
        (engine, rx)
    }

    #[test]
    fn teardown_reports_the_interrupted_sound() {
        let (mut engine, rx) = bare_engine();
        engine.current = Some((5, None));
        engine.teardown();
        assert_eq!(
            rx.try_recv().ok(),
            Some(EngineEvent::Player(PlayerEvent::Stopped { pad: 5 }))
        );
        assert!(engine.current.is_none());
        engine.teardown();
        assert!(rx.try_recv().is_err(), "nothing to report when idle");
    }

    #[test]
    fn first_open_uses_selected_devices() {
        // At startup every role is NotSelected in the status, but the names are set.
        let names = DeviceNames {
            mic: Some("Mic".into()),
            cable: Some("CABLE Input".into()),
            monitor: None,
        };
        let planned = DeviceStatus {
            mic: DeviceState::NotSelected,
            cable: DeviceState::NotSelected,
            monitor: DeviceState::NotSelected,
        };
        assert_eq!(
            devices_to_build(&names, &planned),
            [
                Some("Mic".to_string()),
                Some("CABLE Input".to_string()),
                None
            ]
        );
        let missing = DeviceStatus {
            mic: DeviceState::Missing,
            ..planned
        };
        assert_eq!(devices_to_build(&names, &missing)[0], None);
    }

    #[test]
    fn engine_ignores_unknown_pad_and_survives_no_devices() {
        let params = Arc::new(SharedParams::default());
        let mut engine = start(
            DeviceNames::default(),
            Vec::new(),
            params.clone(),
            Arc::new(|| {}),
        );
        engine.send(EngineCommand::PlayPad(99));
        engine.send(EngineCommand::ToggleMode);
        engine.send(EngineCommand::Stop);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut got_mode = false;
        while Instant::now() < deadline && !got_mode {
            if let Ok(e) = engine.events.recv_timeout(Duration::from_millis(20)) {
                got_mode = e == EngineEvent::ModeChanged(RoutingMode::GoLive);
            }
        }
        assert!(got_mode);
        assert!(params.go_live());

        let t = Instant::now();
        engine.shutdown();
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}

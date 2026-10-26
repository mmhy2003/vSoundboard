use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::config::{Config, RoutingMode};

/// f32 stored as bits so audio callbacks can read it without locking.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

/// Live mix settings shared between the UI, the player and the audio callbacks.
#[derive(Debug)]
pub struct SharedParams {
    pub sounds_gain: AtomicF32,
    pub mic_gain: AtomicF32,
    pub monitor_gain: AtomicF32,
    pub pad_gain: AtomicF32,
    pub go_live: AtomicBool,
}

impl Default for SharedParams {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

impl SharedParams {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            sounds_gain: AtomicF32::new(cfg.volumes.sounds),
            mic_gain: AtomicF32::new(cfg.volumes.mic),
            monitor_gain: AtomicF32::new(cfg.volumes.monitor),
            pad_gain: AtomicF32::new(1.0),
            go_live: AtomicBool::new(cfg.mode == RoutingMode::GoLive),
        }
    }

    pub fn go_live(&self) -> bool {
        self.go_live.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_f32_round_trips() {
        let a = AtomicF32::new(0.25);
        assert_eq!(a.load(), 0.25);
        a.store(-1.5);
        assert_eq!(a.load(), -1.5);
    }

    #[test]
    fn from_config_copies_volumes_and_mode() {
        let mut cfg = Config::default();
        cfg.volumes.sounds = 0.5;
        cfg.volumes.mic = 0.25;
        cfg.volumes.monitor = 0.75;
        cfg.mode = RoutingMode::GoLive;
        let p = SharedParams::from_config(&cfg);
        assert_eq!(p.sounds_gain.load(), 0.5);
        assert_eq!(p.mic_gain.load(), 0.25);
        assert_eq!(p.monitor_gain.load(), 0.75);
        assert_eq!(p.pad_gain.load(), 1.0);
        assert!(p.go_live());
    }
}

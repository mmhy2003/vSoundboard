use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub pads: Vec<Pad>,
    pub stop_hotkey: Option<String>,
    pub mode_hotkey: Option<String>,
    pub devices: DeviceNames,
    pub volumes: Volumes,
    pub mode: RoutingMode,
    pub next_pad_id: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            pads: Vec::new(),
            stop_hotkey: None,
            mode_hotkey: None,
            devices: DeviceNames::default(),
            volumes: Volumes::default(),
            mode: RoutingMode::default(),
            next_pad_id: 1,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Pad {
    pub id: u64,
    pub name: String,
    pub path: PathBuf,
    pub hotkey: Option<String>,
    pub volume: f32,
}

impl Default for Pad {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            path: PathBuf::new(),
            hotkey: None,
            volume: 1.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct DeviceNames {
    pub mic: Option<String>,
    pub cable: Option<String>,
    pub monitor: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Volumes {
    pub sounds: f32,
    pub mic: f32,
    pub monitor: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Self {
            sounds: 1.0,
            mic: 1.0,
            monitor: 1.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RoutingMode {
    #[default]
    Normal,
    GoLive,
}

#[derive(Debug, PartialEq)]
pub enum LoadOutcome {
    Fresh(Config),
    Loaded(Config),
    RecoveredFromCorrupt { config: Config, backup: PathBuf },
}

impl LoadOutcome {
    pub fn into_config(self) -> Config {
        match self {
            LoadOutcome::Fresh(c) | LoadOutcome::Loaded(c) => c,
            LoadOutcome::RecoveredFromCorrupt { config, .. } => config,
        }
    }
}

/// `%APPDATA%\vSoundboard\config.json`
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("vSoundboard").join("config.json")
}

pub fn load(path: &Path) -> LoadOutcome {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return LoadOutcome::Fresh(Config::default()),
    };
    match serde_json::from_slice::<Config>(&bytes) {
        Ok(mut cfg) => {
            normalize(&mut cfg);
            LoadOutcome::Loaded(cfg)
        }
        Err(_) => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".bad-{secs}"));
            let backup = path.with_file_name(name);
            if std::fs::rename(path, &backup).is_err() {
                // Couldn't move it aside; keep a copy rather than lose it.
                let _ = std::fs::write(&backup, &bytes);
            }
            LoadOutcome::RecoveredFromCorrupt {
                config: Config::default(),
                backup,
            }
        }
    }
}

pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let json = serde_json::to_vec_pretty(cfg).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

fn clamp_volume(v: f32) -> f32 {
    if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) }
}

fn normalize(cfg: &mut Config) {
    cfg.volumes.sounds = clamp_volume(cfg.volumes.sounds);
    cfg.volumes.mic = clamp_volume(cfg.volumes.mic);
    cfg.volumes.monitor = clamp_volume(cfg.volumes.monitor);
    for pad in &mut cfg.pads {
        pad.volume = clamp_volume(pad.volume);
    }
    let max_id = cfg.pads.iter().map(|p| p.id).max().unwrap_or(0);
    cfg.next_pad_id = cfg.next_pad_id.max(max_id + 1);
}

impl Config {
    pub fn add_pad(&mut self, path: PathBuf) -> u64 {
        let id = self.next_pad_id;
        self.next_pad_id += 1;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.pads.push(Pad {
            id,
            name,
            path,
            hotkey: None,
            volume: 1.0,
        });
        id
    }

    pub fn remove_pad(&mut self, id: u64) {
        self.pads.retain(|p| p.id != id);
    }

    pub fn pad(&self, id: u64) -> Option<&Pad> {
        self.pads.iter().find(|p| p.id == id)
    }

    pub fn pad_mut(&mut self, id: u64) -> Option<&mut Pad> {
        self.pads.iter_mut().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        (dir, path)
    }

    #[test]
    fn missing_file_is_fresh() {
        let (_d, path) = tmp();
        assert_eq!(load(&path), LoadOutcome::Fresh(Config::default()));
    }

    #[test]
    fn round_trips_unicode_paths() {
        let (_d, path) = tmp();
        let mut cfg = Config::default();
        cfg.add_pad(PathBuf::from(r"C:\Users\محمد\🎵 sounds\airhorn.mp3"));
        cfg.add_pad(PathBuf::from(r"C:\Users\محمد\🎵 sounds\بروه.wav"));
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path), LoadOutcome::Loaded(cfg.clone()));
        assert_eq!(cfg.pads[0].name, "airhorn");
        assert_eq!(cfg.pads[1].name, "بروه");
    }

    #[test]
    fn corrupt_json_is_backed_up() {
        let (_d, path) = tmp();
        std::fs::write(&path, r#"{"pads": ["#).unwrap();
        match load(&path) {
            LoadOutcome::RecoveredFromCorrupt { config, backup } => {
                assert_eq!(config, Config::default());
                assert_eq!(std::fs::read_to_string(&backup).unwrap(), r#"{"pads": ["#);
                assert!(
                    backup
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("config.json.bad-")
                );
                assert!(!path.exists());
            }
            other => panic!("expected RecoveredFromCorrupt, got {other:?}"),
        }
    }

    #[test]
    fn unknown_and_missing_fields_tolerated() {
        let (_d, path) = tmp();
        std::fs::write(
            &path,
            r#"{"version":1,"future_field":3,"pads":[{"id":4,"name":"x","path":"C:\\x.wav"}]}"#,
        )
        .unwrap();
        let cfg = match load(&path) {
            LoadOutcome::Loaded(c) => c,
            other => panic!("expected Loaded, got {other:?}"),
        };
        assert_eq!(cfg.pads.len(), 1);
        assert_eq!(cfg.pads[0].volume, 1.0);
        assert!(cfg.next_pad_id >= 5);
    }

    #[test]
    fn volumes_clamped_on_load() {
        let (_d, path) = tmp();
        std::fs::write(
            &path,
            r#"{"volumes":{"sounds":7.5,"mic":0.5,"monitor":0.25},"pads":[{"id":1,"name":"a","path":"a.wav","volume":-2},{"id":2,"name":"b","path":"b.wav","volume":0.5}]}"#,
        )
        .unwrap();
        let cfg = load(&path).into_config();
        assert_eq!(cfg.volumes.sounds, 1.0);
        assert_eq!(cfg.volumes.mic, 0.5);
        assert_eq!(cfg.pads[0].volume, 0.0);
        assert_eq!(cfg.pads[1].volume, 0.5);

        // NaN can't be written as JSON, so check normalization directly.
        let mut c = Config::default();
        c.volumes.monitor = f32::NAN;
        normalize(&mut c);
        assert_eq!(c.volumes.monitor, 1.0);
    }

    #[test]
    fn ids_not_reused_after_remove() {
        let mut cfg = Config::default();
        let a = cfg.add_pad("a.wav".into());
        cfg.add_pad("b.wav".into());
        cfg.remove_pad(a);
        cfg.add_pad("c.wav".into());
        let ids: Vec<u64> = cfg.pads.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![2, 3]);
    }

    #[test]
    fn save_is_atomic() {
        let (d, path) = tmp();
        save(&path, &Config::default()).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        assert!(path.exists());
    }

    #[test]
    fn save_creates_parent_dir() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("vSoundboard").join("config.json");
        save(&path, &Config::default()).unwrap();
        assert!(path.exists());
    }
}

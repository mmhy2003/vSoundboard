use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    /// Separate boards; only the selected one's pads are shown and have live shortcuts.
    pub groups: Vec<Group>,
    pub selected_group: usize,
    /// The flat pad list of files written before groups existed; read once, never written.
    #[serde(rename = "pads", skip_serializing)]
    pub legacy_pads: Option<Vec<Pad>>,
    pub stop_hotkey: Option<String>,
    pub mode_hotkey: Option<String>,
    pub devices: DeviceNames,
    pub volumes: Volumes,
    pub mode: RoutingMode,
    pub next_pad_id: u64,
    pub updates: UpdateSettings,
    /// The version that last ran, to show "Updated to …" once after an update.
    pub last_run_version: Option<String>,
    /// Keep the window above other windows.
    pub always_on_top: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct UpdateSettings {
    pub auto_check: bool,
    /// Unix seconds; "Later" hides the update dialog until then.
    pub snoozed_until: Option<u64>,
    /// Set just before handing over to the installer, to detect an update that didn't land.
    pub attempted_version: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            snoozed_until: None,
            attempted_version: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 2,
            groups: vec![Group {
                name: FIRST_GROUP.into(),
                pads: Vec::new(),
            }],
            selected_group: 0,
            legacy_pads: None,
            stop_hotkey: None,
            mode_hotkey: None,
            devices: DeviceNames::default(),
            volumes: Volumes::default(),
            mode: RoutingMode::default(),
            next_pad_id: 1,
            updates: UpdateSettings::default(),
            last_run_version: None,
            always_on_top: false,
        }
    }
}

/// The name of the group a fresh install starts with, and that old pads move into.
pub const FIRST_GROUP: &str = "Main";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Group {
    pub name: String,
    pub pads: Vec<Pad>,
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
    RecoveredFromCorrupt {
        config: Config,
        backup: PathBuf,
    },
    /// The file exists but couldn't be read (locked, no permission). Never save over it.
    Unreadable {
        config: Config,
        error: String,
    },
}

impl LoadOutcome {
    pub fn into_config(self) -> Config {
        match self {
            LoadOutcome::Fresh(c) | LoadOutcome::Loaded(c) => c,
            LoadOutcome::RecoveredFromCorrupt { config, .. } => config,
            LoadOutcome::Unreadable { config, .. } => config,
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return LoadOutcome::Fresh(Config::default());
        }
        Err(e) => {
            return LoadOutcome::Unreadable {
                config: Config::default(),
                error: e.to_string(),
            };
        }
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
    for pad in cfg.groups.iter_mut().flat_map(|g| &mut g.pads) {
        pad.volume = clamp_volume(pad.volume);
    }
    let max_id = cfg.all_pads().map(|p| p.id).max().unwrap_or(0);
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
        let Some(group) = self.groups.get_mut(self.selected_group) else {
            return id;
        };
        group.pads.push(Pad {
            id,
            name,
            path,
            hotkey: None,
            volume: 1.0,
        });
        id
    }

    pub fn remove_pad(&mut self, id: u64) {
        for group in &mut self.groups {
            group.pads.retain(|p| p.id != id);
        }
    }

    /// Move pad `id` so it ends up at index `to` of its group (clamped to the end); the pads in
    /// between shift along by one. Returns whether anything moved.
    pub fn move_pad(&mut self, id: u64, to: usize) -> bool {
        let Some(pads) = self.group_of(id).map(|g| &mut self.groups[g].pads) else {
            return false;
        };
        let Some(from) = pads.iter().position(|p| p.id == id) else {
            return false;
        };
        let to = to.min(pads.len() - 1);
        if from == to {
            return false;
        }
        let pad = pads.remove(from);
        pads.insert(to, pad);
        true
    }

    /// The selected group's pads, in board order.
    pub fn pads(&self) -> &[Pad] {
        self.groups
            .get(self.selected_group)
            .map(|g| g.pads.as_slice())
            .unwrap_or_default()
    }

    /// Every pad in every group.
    pub fn all_pads(&self) -> impl Iterator<Item = &Pad> {
        self.groups.iter().flat_map(|g| &g.pads)
    }

    /// Index of the group holding pad `id`.
    pub fn group_of(&self, id: u64) -> Option<usize> {
        self.groups
            .iter()
            .position(|g| g.pads.iter().any(|p| p.id == id))
    }

    pub fn pad(&self, id: u64) -> Option<&Pad> {
        self.all_pads().find(|p| p.id == id)
    }

    pub fn pad_mut(&mut self, id: u64) -> Option<&mut Pad> {
        self.groups
            .iter_mut()
            .flat_map(|g| &mut g.pads)
            .find(|p| p.id == id)
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
        assert_eq!(cfg.pads()[0].name, "airhorn");
        assert_eq!(cfg.pads()[1].name, "بروه");
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
            r#"{"version":2,"future_field":3,"groups":[{"name":"Main","pads":[{"id":4,"name":"x","path":"C:\\x.wav"}]}]}"#,
        )
        .unwrap();
        let cfg = match load(&path) {
            LoadOutcome::Loaded(c) => c,
            other => panic!("expected Loaded, got {other:?}"),
        };
        assert_eq!(cfg.pads().len(), 1);
        assert_eq!(cfg.pads()[0].volume, 1.0);
        assert!(cfg.next_pad_id >= 5);
    }

    #[test]
    fn volumes_clamped_on_load() {
        let (_d, path) = tmp();
        std::fs::write(
            &path,
            r#"{"volumes":{"sounds":7.5,"mic":0.5,"monitor":0.25},"groups":[{"name":"Main","pads":[{"id":1,"name":"a","path":"a.wav","volume":-2},{"id":2,"name":"b","path":"b.wav","volume":0.5}]}]}"#,
        )
        .unwrap();
        let cfg = load(&path).into_config();
        assert_eq!(cfg.volumes.sounds, 1.0);
        assert_eq!(cfg.volumes.mic, 0.5);
        assert_eq!(cfg.pads()[0].volume, 0.0);
        assert_eq!(cfg.pads()[1].volume, 0.5);

        // NaN can't be written as JSON, so check normalization directly.
        let mut c = Config::default();
        c.volumes.monitor = f32::NAN;
        normalize(&mut c);
        assert_eq!(c.volumes.monitor, 1.0);
    }

    fn order(cfg: &Config) -> Vec<&str> {
        cfg.pads().iter().map(|p| p.name.as_str()).collect()
    }

    fn abcd() -> Config {
        let mut cfg = Config::default();
        for n in ["a", "b", "c", "d"] {
            cfg.add_pad(format!("{n}.wav").into());
        }
        cfg
    }

    #[test]
    fn move_pad_forward_pushes_others_back() {
        let mut cfg = abcd();
        // Drop "a" (id 1) on "c" (index 2): it takes that spot.
        assert!(cfg.move_pad(1, 2));
        assert_eq!(order(&cfg), ["b", "c", "a", "d"]);
    }

    #[test]
    fn move_pad_backward_pushes_others_along() {
        let mut cfg = abcd();
        // Drop "d" (id 4) on "b" (index 1).
        assert!(cfg.move_pad(4, 1));
        assert_eq!(order(&cfg), ["a", "d", "b", "c"]);
    }

    #[test]
    fn move_pad_to_end_and_no_ops() {
        let mut cfg = abcd();
        assert!(cfg.move_pad(2, usize::MAX), "past the end means last");
        assert_eq!(order(&cfg), ["a", "c", "d", "b"]);
        assert!(!cfg.move_pad(1, 0), "already there");
        assert!(!cfg.move_pad(99, 0), "unknown pad");
        assert_eq!(order(&cfg), ["a", "c", "d", "b"]);
        // Ids, names and hotkeys travel with the pad.
        assert_eq!(cfg.pads()[3].id, 2);
    }

    #[test]
    fn ids_not_reused_after_remove() {
        let mut cfg = Config::default();
        let a = cfg.add_pad("a.wav".into());
        cfg.add_pad("b.wav".into());
        cfg.remove_pad(a);
        cfg.add_pad("c.wav".into());
        let ids: Vec<u64> = cfg.pads().iter().map(|p| p.id).collect();
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
    fn always_on_top_defaults_off_and_round_trips() {
        let (_d, path) = tmp();
        std::fs::write(&path, r#"{"version":1,"pads":[]}"#).unwrap();
        assert!(!load(&path).into_config().always_on_top);
        let cfg = Config {
            always_on_top: true,
            ..Config::default()
        };
        save(&path, &cfg).unwrap();
        assert!(load(&path).into_config().always_on_top);
    }

    #[test]
    fn old_config_gets_update_defaults() {
        let (_d, path) = tmp();
        std::fs::write(&path, r#"{"version":1,"pads":[]}"#).unwrap();
        let cfg = load(&path).into_config();
        assert!(cfg.updates.auto_check);
        assert_eq!(cfg.updates.snoozed_until, None);
        assert_eq!(cfg.last_run_version, None);
    }

    #[test]
    fn save_creates_parent_dir() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("vSoundboard").join("config.json");
        save(&path, &Config::default()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn unreadable_file_is_not_fresh() {
        // A directory where the file should be: reading fails, but not with NotFound.
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("config.json");
        std::fs::create_dir(&path).unwrap();
        match load(&path) {
            LoadOutcome::Unreadable { config, .. } => assert_eq!(config, Config::default()),
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn default_has_one_empty_main_group() {
        let c = Config::default();
        assert_eq!(c.groups.len(), 1);
        assert_eq!(c.groups[0].name, "Main");
        assert!(c.pads().is_empty());
        assert_eq!(c.selected_group, 0);
    }

    #[test]
    fn pads_live_in_the_selected_group_but_ids_are_global() {
        let mut c = Config::default();
        let a = c.add_pad("a.wav".into());
        c.groups.push(Group {
            name: "Gaming".into(),
            pads: vec![],
        });
        c.selected_group = 1;
        let b = c.add_pad("b.wav".into());
        assert_ne!(a, b);
        assert_eq!(c.pads().iter().map(|p| p.id).collect::<Vec<_>>(), vec![b]);
        assert_eq!(c.all_pads().count(), 2);
        // Found even in a group that isn't selected.
        assert_eq!(c.pad(a).unwrap().name, "a");
        assert_eq!(c.group_of(a), Some(0));
        c.remove_pad(a);
        assert_eq!(c.groups[0].pads.len(), 0);
    }

    #[test]
    fn move_pad_stays_within_its_group() {
        let mut c = abcd();
        c.groups.push(Group {
            name: "G".into(),
            pads: vec![],
        });
        assert!(c.move_pad(1, usize::MAX));
        assert_eq!(order(&c), ["b", "c", "d", "a"]);
        assert!(c.groups[1].pads.is_empty());
    }

    #[test]
    fn groups_round_trip_with_selection() {
        let (_d, path) = tmp();
        let mut c = abcd();
        c.groups.push(Group {
            name: "Gaming".into(),
            pads: vec![],
        });
        c.selected_group = 1;
        save(&path, &c).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("pads"),
            "flat pads never written"
        );
        assert_eq!(load(&path).into_config(), c);
    }
}

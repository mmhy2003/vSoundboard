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
            if let Some(pads) = cfg.legacy_pads.take() {
                // Settings from before groups: keep a copy for older versions (only the first
                // time, so a later re-migration can't replace it), then move the pads into Main.
                let backup = pre_groups_backup_path(path);
                if !backup.exists() {
                    let _ = std::fs::write(&backup, &bytes);
                }
                cfg.groups = vec![Group {
                    name: FIRST_GROUP.into(),
                    pads,
                }];
                cfg.selected_group = 0;
                cfg.version = 2;
            }
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

/// Where the settings from before groups existed are kept, next to `config`.
pub fn pre_groups_backup_path(config: &Path) -> PathBuf {
    let mut name = config.file_name().unwrap_or_default().to_os_string();
    name.push(".pre-groups");
    config.with_file_name(name)
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
    if cfg.groups.is_empty() {
        cfg.groups.push(Group {
            name: FIRST_GROUP.into(),
            pads: Vec::new(),
        });
    }
    if cfg.selected_group >= cfg.groups.len() {
        cfg.selected_group = 0;
    }
    let max_id = cfg.all_pads().map(|p| p.id).max().unwrap_or(0);
    cfg.next_pad_id = cfg.next_pad_id.max(max_id + 1);
    // Pad ids must be unique across groups; later duplicates get fresh ones.
    let mut seen = std::collections::HashSet::new();
    for pad in cfg.groups.iter_mut().flat_map(|g| &mut g.pads) {
        if !seen.insert(pad.id) {
            pad.id = cfg.next_pad_id;
            cfg.next_pad_id += 1;
            seen.insert(pad.id);
        }
    }
    // Names: trimmed, never empty, unique ignoring case.
    let mut taken: Vec<String> = Vec::new();
    for (i, group) in cfg.groups.iter_mut().enumerate() {
        let mut name = group.name.trim().to_string();
        if name.is_empty() {
            name = format!("Group {}", i + 1);
        }
        let base = name.clone();
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            name = format!("{base} {n}");
            n += 1;
        }
        taken.push(name.to_lowercase());
        group.name = name;
    }
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

    /// The trimmed name, or why it can't be used. `except` is the group being renamed.
    pub fn check_group_name(&self, name: &str, except: Option<usize>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a name.".into());
        }
        let clash = self
            .groups
            .iter()
            .enumerate()
            .find(|(i, g)| Some(*i) != except && g.name.to_lowercase() == name.to_lowercase());
        match clash {
            Some((_, g)) => Err(format!("There's already a group called “{}”.", g.name)),
            None => Ok(name.to_string()),
        }
    }

    /// Add an empty group at the end and select it; returns its index.
    pub fn add_group(&mut self, name: String) -> usize {
        self.groups.push(Group {
            name,
            pads: Vec::new(),
        });
        self.selected_group = self.groups.len() - 1;
        self.selected_group
    }

    pub fn rename_group(&mut self, index: usize, name: String) {
        if let Some(g) = self.groups.get_mut(index) {
            g.name = name;
        }
    }

    /// Remove a group and its pads, keeping the selection on the same group when it survives,
    /// or moving it to the tab that took the deleted one's place. The last group stays.
    pub fn delete_group(&mut self, index: usize) -> Option<Group> {
        if self.groups.len() <= 1 || index >= self.groups.len() {
            return None;
        }
        let group = self.groups.remove(index);
        if self.selected_group > index {
            self.selected_group -= 1;
        }
        self.selected_group = self.selected_group.min(self.groups.len() - 1);
        Some(group)
    }

    /// Returns whether the selection changed.
    pub fn select_group(&mut self, index: usize) -> bool {
        if index >= self.groups.len() || index == self.selected_group {
            return false;
        }
        self.selected_group = index;
        true
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

    const V1: &str = r#"{"version":1,"stop_hotkey":"control+Digit0","pads":[
  {"id":1,"name":"Airhorn","path":"a.wav","hotkey":"control+Digit1","volume":0.5},
  {"id":2,"name":"Bruh","path":"b.wav"}]}"#;

    #[test]
    fn old_flat_file_migrates_into_main() {
        let (_d, path) = tmp();
        std::fs::write(&path, V1).unwrap();
        let c = load(&path).into_config();
        assert_eq!(c.groups.len(), 1);
        assert_eq!(c.groups[0].name, "Main");
        assert_eq!(order(&c), ["Airhorn", "Bruh"]);
        assert_eq!(c.pad(1).unwrap().hotkey.as_deref(), Some("control+Digit1"));
        assert_eq!(c.pad(1).unwrap().volume, 0.5);
        assert_eq!(c.stop_hotkey.as_deref(), Some("control+Digit0"));
        assert_eq!(c.legacy_pads, None);
        assert_eq!(
            std::fs::read_to_string(pre_groups_backup_path(&path)).unwrap(),
            V1
        );
    }

    #[test]
    fn migration_keeps_existing_backup() {
        let (_d, path) = tmp();
        std::fs::write(pre_groups_backup_path(&path), "original").unwrap();
        std::fs::write(&path, V1).unwrap();
        load(&path);
        assert_eq!(
            std::fs::read_to_string(pre_groups_backup_path(&path)).unwrap(),
            "original"
        );
    }

    #[test]
    fn new_format_file_writes_no_backup() {
        let (_d, path) = tmp();
        save(&path, &abcd()).unwrap();
        load(&path);
        assert!(!pre_groups_backup_path(&path).exists());
    }

    #[test]
    fn broken_groups_are_repaired() {
        let (_d, path) = tmp();
        std::fs::write(
            &path,
            r#"{"version":2,"selected_group":9,"groups":[
              {"name":"  ","pads":[{"id":1,"name":"a","path":"a.wav"}]},
              {"name":"Gaming","pads":[{"id":1,"name":"b","path":"b.wav"}]},
              {"name":"gaming","pads":[]}]}"#,
        )
        .unwrap();
        let c = load(&path).into_config();
        let names: Vec<_> = c.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Group 1", "Gaming", "gaming 2"]);
        assert_eq!(c.selected_group, 0);
        let ids: Vec<u64> = c.all_pads().map(|p| p.id).collect();
        assert_eq!(ids, [1, 2]);
        assert!(c.next_pad_id >= 3);

        std::fs::write(&path, r#"{"version":2,"groups":[]}"#).unwrap();
        assert_eq!(load(&path).into_config().groups[0].name, "Main");
    }

    fn three_groups() -> Config {
        let mut c = Config::default();
        c.add_group("Gaming".into());
        c.add_group("Misc".into());
        c
    }

    #[test]
    fn add_group_selects_it() {
        let c = three_groups();
        assert_eq!(c.selected_group, 2);
        assert!(c.pads().is_empty());
    }

    #[test]
    fn group_names_are_checked() {
        let c = three_groups();
        assert_eq!(c.check_group_name("  Live ", None), Ok("Live".into()));
        assert_eq!(c.check_group_name("   ", None), Err("Enter a name.".into()));
        assert_eq!(
            c.check_group_name("gaming", None),
            Err("There's already a group called “Gaming”.".into())
        );
    }

    #[test]
    fn rename_to_own_name_in_other_case_is_allowed() {
        let mut c = three_groups();
        assert_eq!(c.check_group_name("GAMING", Some(1)), Ok("GAMING".into()));
        c.rename_group(1, "GAMING".into());
        assert_eq!(c.groups[1].name, "GAMING");
    }

    #[test]
    fn last_group_cannot_be_deleted() {
        let mut c = Config::default();
        assert!(c.delete_group(0).is_none());
        assert_eq!(c.groups.len(), 1);
    }

    #[test]
    fn delete_before_selected_keeps_selection() {
        let mut c = three_groups();
        assert_eq!(c.delete_group(0).unwrap().name, "Main");
        assert_eq!(c.groups[c.selected_group].name, "Misc");
    }

    #[test]
    fn delete_selected_moves_to_neighbour() {
        let mut c = three_groups();
        c.select_group(1);
        c.delete_group(1);
        assert_eq!(c.groups[c.selected_group].name, "Misc");
        // Deleting the last tab falls back to the one on its left.
        c.delete_group(c.selected_group);
        assert_eq!(c.groups[c.selected_group].name, "Main");
    }

    #[test]
    fn select_group_bounds() {
        let mut c = three_groups();
        assert!(c.select_group(0));
        assert!(!c.select_group(0));
        assert!(!c.select_group(7));
        assert_eq!(c.selected_group, 0);
    }
}

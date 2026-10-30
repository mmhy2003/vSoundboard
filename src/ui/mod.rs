mod board;
mod bottom_bar;
mod capture;
mod groups;
mod settings;
mod update_dialog;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use eframe::egui;

use crate::audio::engine::{EngineCommand, EngineEvent, EngineHandle};
use crate::audio::player::PlayerEvent;
use crate::config::{self, Config, RoutingMode};
use crate::hotkeys::{HotkeyAction, HotkeyRegistry};
use crate::tray::{self, TrayRequests};

pub use capture::Capture;
pub use groups::GroupDialog;
pub use settings::first_run_defaults;
pub use update_dialog::{current_version, now_secs};

pub const SUPPORTED_EXTENSIONS: [&str; 4] = ["mp3", "wav", "ogg", "flac"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Board,
    Settings,
}

pub struct SoundboardApp {
    pub config: Config,
    pub config_path: PathBuf,
    pub engine: EngineHandle,
    pub screen: Screen,
    pub playing: Option<u64>,
    pub pad_errors: HashMap<u64, String>,
    pub notice: Option<String>,
    pub capture: Option<Capture>,
    /// Pad being renamed and the text typed so far.
    pub renaming: Option<(u64, String)>,
    /// The open New / Rename / Delete group dialog.
    pub group_dialog: Option<GroupDialog>,
    /// The shortcut warning last put in `notice`, so switching groups can take it down.
    pub hotkey_notice: Option<String>,
    pub hotkeys: Option<HotkeyRegistry>,
    /// (inputs, outputs) shown on the settings screen; refreshed each time it opens.
    pub device_lists: Option<(Vec<String>, Vec<String>)>,
    /// Whether the Start with Windows entry points at this exe; read when settings open.
    pub autostart_on: Option<bool>,
    /// Keeps the tray icon alive; while it exists, closing the window hides it instead.
    pub tray: Option<tray_icon::TrayIcon>,
    pub tray_requests: Arc<TrayRequests>,
    quitting: bool,
    /// Frames left in which to hide the window (start hidden in the tray).
    pub hide_frames: u8,
    /// The window was started off-screen (hidden start) and needs a real position when shown.
    pub parked_offscreen: bool,
    /// Set when the settings file couldn't be read; nothing is saved so it isn't overwritten.
    pub read_only: bool,
    /// Background update checker (None in tests).
    pub updater: Option<crate::update::worker::UpdateWorker>,
    /// True when this is the installed copy, so "Update now" can run the installer.
    pub can_install: bool,
    /// The installer handoff after "Update now", if one is under way or failed.
    pub handoff: Option<update_dialog::Handoff>,
    /// A link shown next to the notice (e.g. the manual download page).
    pub notice_link: Option<String>,
    /// Last frame the download progress was on screen (decides whether to install right away).
    pub download_watched: Option<std::time::Instant>,
}

/// The pad table the engine needs: (id, file, volume).
pub fn engine_pads(cfg: &Config) -> Vec<(u64, PathBuf, f32)> {
    cfg.all_pads()
        .map(|p| (p.id, p.path.clone(), p.volume))
        .collect()
}

pub fn window_level(always_on_top: bool) -> egui::WindowLevel {
    if always_on_top {
        egui::WindowLevel::AlwaysOnTop
    } else {
        egui::WindowLevel::Normal
    }
}

/// Pads are at least 140 px wide with 8 px gaps; always at least one column.
pub fn grid_columns(available_width: f32) -> usize {
    const PAD: f32 = 140.0;
    const GAP: f32 = 8.0;
    (((available_width + GAP) / (PAD + GAP)).floor() as usize).max(1)
}

/// "1:05", "12:00", "1:02:03"
pub fn format_time(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl SoundboardApp {
    pub fn new(config: Config, config_path: PathBuf, engine: EngineHandle) -> Self {
        Self {
            config,
            config_path,
            engine,
            screen: Screen::Board,
            playing: None,
            pad_errors: HashMap::new(),
            notice: None,
            capture: None,
            renaming: None,
            group_dialog: None,
            hotkey_notice: None,
            hotkeys: None,
            device_lists: None,
            autostart_on: None,
            tray: None,
            tray_requests: Arc::default(),
            quitting: false,
            hide_frames: 0,
            parked_offscreen: false,
            read_only: false,
            updater: None,
            can_install: false,
            handoff: None,
            notice_link: None,
            download_watched: None,
        }
    }

    /// Install the tray icon; call on the UI thread.
    pub fn enable_tray(&mut self, ctx: &egui::Context) {
        match tray::install(
            ctx.clone(),
            self.engine.tx.clone(),
            self.tray_requests.clone(),
        ) {
            Ok(icon) => self.tray = Some(icon),
            Err(e) => self.notice = Some(format!("Couldn't add the tray icon: {e}")),
        }
    }

    /// Install global shortcuts; call on the UI thread.
    pub fn enable_hotkeys(&mut self) {
        match HotkeyRegistry::new(self.engine.tx.clone()) {
            Ok(reg) => {
                self.hotkeys = Some(reg);
                self.refresh_hotkeys();
            }
            Err(e) => self.notice = Some(format!("Keyboard shortcuts are unavailable: {e}")),
        }
    }

    /// Re-register the live shortcuts, saying which ones Windows refused. A warning left over
    /// from another group's shortcuts goes away; other notices stay.
    pub fn refresh_hotkeys(&mut self) {
        let failures = self.sync_hotkeys();
        let showing_ours = self.hotkey_notice.is_some() && self.notice == self.hotkey_notice;
        if failures.is_empty() {
            if showing_ours {
                self.notice = None;
            }
            self.hotkey_notice = None;
        } else {
            let msgs: Vec<String> = failures.into_iter().map(|(_, m)| m).collect();
            let msg = msgs.join(" ");
            self.notice = Some(msg.clone());
            self.notice_link = None;
            self.hotkey_notice = Some(msg);
        }
    }

    pub fn sync_hotkeys(&mut self) -> Vec<(HotkeyAction, String)> {
        match self.hotkeys.as_mut() {
            Some(reg) => reg.sync(&self.config),
            None => Vec::new(),
        }
    }

    pub fn suspend_hotkeys(&mut self) {
        if let Some(reg) = self.hotkeys.as_mut() {
            reg.suspend();
        }
    }

    /// The window lost focus or is hiding: an open capture would leave every shortcut off.
    pub fn window_focus_lost(&mut self) {
        if self.capture.take().is_some() {
            self.sync_hotkeys();
        }
    }

    pub fn start_capture(&mut self, target: HotkeyAction) {
        self.suspend_hotkeys();
        self.capture = Some(Capture::new(target));
    }

    /// Drag-and-drop: move pad `id` to index `to`, pushing the others along; saved right away.
    pub fn move_pad(&mut self, id: u64, to: usize) {
        if self.config.move_pad(id, to) {
            self.persist();
        }
    }

    /// Pin the window above other windows (or unpin it); remembered between restarts.
    pub fn set_always_on_top(&mut self, ctx: &egui::Context, on: bool) {
        self.config.always_on_top = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(window_level(on)));
        self.persist();
    }

    /// Show another group's board; its shortcuts become the live ones. A sound that's playing
    /// keeps playing.
    pub fn select_group(&mut self, index: usize) {
        if self.config.select_group(index) {
            self.persist();
            self.refresh_hotkeys();
        }
    }

    /// Add an empty group (the name is already checked) and switch to it.
    pub fn create_group(&mut self, name: String) {
        self.config.add_group(name);
        self.persist();
        self.refresh_hotkeys();
    }

    pub fn rename_group(&mut self, index: usize, name: String) {
        self.config.rename_group(index, name);
        self.persist();
    }

    /// Remove a group and its pads (never the audio files), stopping its sound if one plays.
    pub fn delete_group(&mut self, index: usize) {
        let Some(group) = self.config.groups.get(index) else {
            return;
        };
        let ids: Vec<u64> = group.pads.iter().map(|p| p.id).collect();
        if self.config.delete_group(index).is_none() {
            return;
        }
        if self.playing.is_some_and(|p| ids.contains(&p)) {
            self.engine.send(EngineCommand::Stop);
        }
        for id in &ids {
            self.pad_errors.remove(id);
        }
        self.persist();
        self.refresh_hotkeys();
    }

    pub fn remove_pad(&mut self, id: u64) {
        if self.playing == Some(id) {
            self.engine.send(EngineCommand::Stop);
        }
        self.config.remove_pad(id);
        self.pad_errors.remove(&id);
        self.persist();
        self.sync_hotkeys();
    }

    /// Save the config and give the engine the current pad table.
    pub fn persist(&mut self) {
        if self.read_only {
            // Leave the unreadable file alone; the engine still gets the pads.
        } else if let Err(e) = config::save(&self.config_path, &self.config) {
            self.notice = Some(format!("Couldn't save settings: {e}"));
        }
        self.engine
            .send(EngineCommand::SetPads(engine_pads(&self.config)));
    }

    pub fn play(&mut self, id: u64) {
        let Some(pad) = self.config.pad(id) else {
            return;
        };
        if !pad.path.exists() {
            let msg = format!("File not found: {}", pad.path.display());
            self.pad_errors.insert(id, msg);
            return;
        }
        self.pad_errors.remove(&id);
        self.engine.send(EngineCommand::PlayPad(id));
    }

    /// Add pads for supported files; anything else is listed in a notice.
    pub fn add_files(&mut self, paths: Vec<PathBuf>) {
        let mut skipped = Vec::new();
        for path in paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if ext.is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e.as_str())) {
                self.config.add_pad(path);
            } else {
                skipped.push(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
        }
        if !skipped.is_empty() {
            self.notice = Some(format!(
                "Skipped {} (only MP3, WAV, OGG and FLAC are supported).",
                skipped.join(", ")
            ));
        }
        self.persist();
    }

    /// Re-check which pad files exist (on startup and after adding).
    pub fn check_files(&mut self) {
        for pad in self.config.all_pads() {
            if !pad.path.exists() {
                self.pad_errors
                    .insert(pad.id, format!("File not found: {}", pad.path.display()));
            }
        }
    }

    pub fn set_mode(&mut self, mode: RoutingMode) {
        self.engine.params.go_live.store(
            mode == RoutingMode::GoLive,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.config.mode = mode;
        self.persist();
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.engine.events.try_recv() {
            match ev {
                EngineEvent::Player(PlayerEvent::Started { pad, .. }) => self.playing = Some(pad),
                EngineEvent::Player(
                    PlayerEvent::Finished { pad } | PlayerEvent::Stopped { pad },
                ) => {
                    if self.playing == Some(pad) {
                        self.playing = None;
                    }
                }
                EngineEvent::Player(PlayerEvent::FileError { pad, message }) => {
                    self.pad_errors.insert(pad, message);
                    if self.playing == Some(pad) {
                        self.playing = None;
                    }
                }
                EngineEvent::ModeChanged(mode) => {
                    self.config.mode = mode;
                    self.persist();
                }
                EngineEvent::Devices(_) => {}
            }
        }
    }
}

impl eframe::App for SoundboardApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();

        if self.hide_frames > 0 {
            self.hide_frames -= 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.request_repaint();
        }

        if self.tray_requests.show.swap(false, Ordering::Relaxed) {
            if std::mem::take(&mut self.parked_offscreen) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    160.0, 120.0,
                )));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.tray_requests.quit.swap(false, Ordering::Relaxed) {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let (close, focused) =
            ctx.input(|i| (i.viewport().close_requested(), i.viewport().focused));
        if self.capture.is_some() && (close || focused == Some(false)) {
            self.window_focus_lost();
        }
        // With a tray icon, the close button hides the window and shortcuts keep working.
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_files(dropped);
        }
        match self.screen {
            Screen::Board => {
                bottom_bar::show(self, ui);
                board::show(self, ui);
            }
            Screen::Settings => settings::show(self, ui),
        }
        capture::show(self, ui.ctx());
        update_dialog::show(self, ui.ctx());
        board::rename_dialog(self, ui.ctx());
        groups::dialog(self, ui.ctx());
        if self.playing.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.config.mode = if self.engine.params.go_live() {
            RoutingMode::GoLive
        } else {
            RoutingMode::Normal
        };
        self.persist();
        self.engine.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{SharedParams, engine};
    use crate::config::DeviceNames;
    use std::sync::Arc;

    fn app(dir: &std::path::Path) -> SoundboardApp {
        let eng = engine::start(
            DeviceNames::default(),
            Vec::new(),
            Arc::new(SharedParams::default()),
            Arc::new(|| {}),
        );
        SoundboardApp::new(Config::default(), dir.join("config.json"), eng)
    }

    #[test]
    fn grid_columns_fit_width() {
        assert_eq!(grid_columns(100.0), 1);
        assert_eq!(grid_columns(0.0), 1);
        assert_eq!(grid_columns(148.0 * 4.0), 4);
        assert_eq!(grid_columns(600.0), 4);
    }

    #[test]
    fn format_time_cases() {
        assert_eq!(format_time(Duration::from_secs(65)), "1:05");
        assert_eq!(format_time(Duration::from_secs(720)), "12:00");
        assert_eq!(format_time(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(format_time(Duration::from_millis(999)), "0:00");
    }

    #[test]
    fn add_files_filters_extensions() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.add_files(vec![
            d.path().join("a.MP3"),
            d.path().join("b.txt"),
            d.path().join("c.flac"),
        ]);
        let names: Vec<&str> = app.config.pads().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
        let notice = app.notice.clone().unwrap();
        assert!(notice.contains("b.txt"), "{notice}");
        // Saved straight away.
        assert_eq!(config::load(&app.config_path).into_config().pads().len(), 2);
    }

    #[test]
    fn play_marks_missing_files() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.add_files(vec![d.path().join("gone.wav")]);
        let id = app.config.pads()[0].id;
        app.play(id);
        let err = app.pad_errors.get(&id).unwrap();
        assert!(err.starts_with("File not found"), "{err}");
    }

    #[test]
    fn focus_loss_cancels_capture() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.start_capture(HotkeyAction::Stop);
        app.window_focus_lost();
        assert!(app.capture.is_none());
    }

    #[test]
    fn read_only_app_never_saves() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.read_only = true;
        app.add_files(vec![d.path().join("a.wav")]);
        assert!(!app.config_path.exists());
    }

    /// Swap the engine's command channel for one the test can read.
    fn tap_engine(app: &mut SoundboardApp) -> crossbeam_channel::Receiver<EngineCommand> {
        let (tx, rx) = crossbeam_channel::unbounded();
        app.engine.tx = tx;
        rx
    }

    fn sent_stop(rx: &crossbeam_channel::Receiver<EngineCommand>) -> bool {
        rx.try_iter().any(|c| matches!(c, EngineCommand::Stop))
    }

    #[test]
    fn switching_groups_saves_and_keeps_pads_apart() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.add_files(vec![d.path().join("a.wav")]);
        app.create_group("Gaming".into());
        app.add_files(vec![d.path().join("b.wav")]);
        app.select_group(0);
        let saved = config::load(&app.config_path).into_config();
        assert_eq!(saved.selected_group, 0);
        assert_eq!(saved.groups[1].name, "Gaming");
        let names: Vec<_> = saved.pads().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a"]);
        app.rename_group(1, "Games".into());
        assert_eq!(
            config::load(&app.config_path).into_config().groups[1].name,
            "Games"
        );
    }

    #[test]
    fn delete_group_stops_its_playing_pad() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.create_group("Gaming".into());
        app.add_files(vec![d.path().join("gone.wav")]);
        let id = app.config.pads()[0].id;
        app.playing = Some(id);
        app.pad_errors.insert(id, "x".into());
        let rx = tap_engine(&mut app);
        app.delete_group(1);
        assert!(sent_stop(&rx));
        assert!(app.pad_errors.is_empty());
        assert_eq!(app.config.groups.len(), 1);
        assert_eq!(config::load(&app.config_path).into_config().groups.len(), 1);
    }

    #[test]
    fn deleting_another_group_leaves_playback_alone() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.add_files(vec![d.path().join("a.wav")]);
        app.playing = Some(app.config.pads()[0].id);
        app.create_group("Gaming".into());
        let rx = tap_engine(&mut app);
        app.delete_group(1);
        assert!(!sent_stop(&rx));
    }

    #[test]
    fn switching_does_not_stop_playback() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.add_files(vec![d.path().join("a.wav")]);
        let id = app.config.pads()[0].id;
        app.playing = Some(id);
        let rx = tap_engine(&mut app);
        app.create_group("Gaming".into());
        app.select_group(0);
        app.select_group(1);
        assert!(!sent_stop(&rx));
        assert_eq!(app.playing, Some(id));
        assert!(engine_pads(&app.config).iter().any(|(p, _, _)| *p == id));
    }

    #[test]
    fn hotkey_warning_follows_the_group() {
        // Ctrl+Alt+Shift+F22: nothing real uses it. `holder` stands in for another app.
        const COMBO: &str = "shift+control+alt+F22";
        let mut held = Config::default();
        let x = held.add_pad("x.wav".into());
        held.pad_mut(x).unwrap().hotkey = Some(COMBO.into());
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut holder = crate::hotkeys::HotkeyRegistry::new(tx).unwrap();
        assert!(holder.sync(&held).is_empty());

        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.add_files(vec![d.path().join("a.wav")]);
        let a = app.config.pads()[0].id;
        app.config.pad_mut(a).unwrap().hotkey = Some(COMBO.into());
        app.create_group("Livestream".into());
        app.enable_hotkeys();
        app.notice = None;

        app.notice_link = Some("https://example.invalid".into());
        app.select_group(0);
        let warning = app.notice.clone().unwrap();
        assert!(warning.contains("already used by another app"), "{warning}");
        assert_eq!(
            app.notice_link, None,
            "no unrelated link on a shortcut warning"
        );

        app.select_group(1);
        assert_eq!(app.notice, None, "the warning was about the other group");

        // A notice that isn't about shortcuts stays put.
        app.notice = Some("Couldn't save settings: disk full".into());
        app.select_group(0);
        app.select_group(1);
        assert_eq!(app.notice, None);
        app.notice = Some("Couldn't save settings: disk full".into());
        app.create_group("Misc".into());
        assert_eq!(
            app.notice.as_deref(),
            Some("Couldn't save settings: disk full")
        );
    }
}

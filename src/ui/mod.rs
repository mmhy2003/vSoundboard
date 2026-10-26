mod board;
mod bottom_bar;
mod capture;
mod settings;

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
pub use settings::first_run_defaults;

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
    pub hotkeys: Option<HotkeyRegistry>,
    /// (inputs, outputs) shown on the settings screen; refreshed each time it opens.
    pub device_lists: Option<(Vec<String>, Vec<String>)>,
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
}

/// The pad table the engine needs: (id, file, volume).
pub fn engine_pads(cfg: &Config) -> Vec<(u64, PathBuf, f32)> {
    cfg.pads
        .iter()
        .map(|p| (p.id, p.path.clone(), p.volume))
        .collect()
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
            hotkeys: None,
            device_lists: None,
            tray: None,
            tray_requests: Arc::default(),
            quitting: false,
            hide_frames: 0,
            parked_offscreen: false,
            read_only: false,
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
                let failures = self.sync_hotkeys();
                if !failures.is_empty() {
                    let msgs: Vec<String> = failures.into_iter().map(|(_, m)| m).collect();
                    self.notice = Some(msgs.join(" "));
                }
            }
            Err(e) => self.notice = Some(format!("Keyboard shortcuts are unavailable: {e}")),
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
        for pad in &self.config.pads {
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
        board::rename_dialog(self, ui.ctx());
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
        let names: Vec<&str> = app.config.pads.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
        let notice = app.notice.clone().unwrap();
        assert!(notice.contains("b.txt"), "{notice}");
        // Saved straight away.
        assert_eq!(config::load(&app.config_path).into_config().pads.len(), 2);
    }

    #[test]
    fn play_marks_missing_files() {
        let d = tempfile::tempdir().unwrap();
        let mut app = app(d.path());
        app.add_files(vec![d.path().join("gone.wav")]);
        let id = app.config.pads[0].id;
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
}

use eframe::egui;

use super::{Screen, SoundboardApp};
use crate::audio::devices::{self, DeviceState, find_cable};
use crate::audio::engine::EngineCommand;
use crate::config::{Config, DeviceNames};
use crate::hotkeys::{self, HotkeyAction};

pub const NO_CABLE: &str = "VB-Cable wasn't found. vSoundboard needs it to act as a microphone in Discord. Install it free from https://vb-audio.com/Cable/ (restart your PC afterwards), then choose “CABLE Input” above.";
pub const DISCORD_HELP: &str = "In Discord → User Settings → Voice & Video, set Input Device to “CABLE Output (VB-Audio Virtual Cable)”. Turn off Noise Suppression and Automatic Gain Control, and set Input Sensitivity low (or manual) so music isn't cut off.";
pub const PTT_HELP: &str =
    "Using Push to Talk? Sounds only reach the call while you hold your Push to Talk key.";
pub const GO_LIVE_HELP: &str = "Sharing your whole screen with sound? Switch to Go Live mode so viewers hear each sound once. In Go Live mode, friends who aren't watching your stream won't hear sounds.";
pub const VB_CABLE_URL: &str = "https://vb-audio.com/Cable/";

pub fn status_text(s: &DeviceState) -> String {
    match s {
        DeviceState::Open => "Working".into(),
        DeviceState::Missing => "Not found — plugged in?".into(),
        DeviceState::NotSelected => "Not selected".into(),
        DeviceState::Failed(e) => format!("Error: {e}"),
    }
}

/// On a fresh config, pick VB-Cable for the virtual mic and Windows' default mic and speakers
/// for the rest (never the cable itself). Returns true if anything was chosen.
pub fn first_run_defaults(
    cfg: &mut Config,
    outputs: &[String],
    default_in: Option<String>,
    default_out: Option<String>,
) -> bool {
    if cfg.devices != DeviceNames::default() {
        return false;
    }
    let is_cable = |n: &String| n.to_lowercase().contains("vb-audio");
    cfg.devices = DeviceNames {
        mic: default_in.filter(|n| !is_cable(n)),
        cable: find_cable(outputs),
        monitor: default_out.filter(|n| !is_cable(n)),
    };
    cfg.devices != DeviceNames::default()
}

fn device_combo(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<String>,
    options: &[String],
) -> bool {
    let mut changed = false;
    ui.label(label);
    let shown = value.clone().unwrap_or_else(|| "(none)".into());
    egui::ComboBox::from_id_salt(label)
        .width(320.0)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            changed |= ui.selectable_value(value, None, "(none)").changed();
            for name in options {
                changed |= ui
                    .selectable_value(value, Some(name.clone()), name)
                    .changed();
            }
        });
    changed
}

fn shortcut_row(
    app: &mut SoundboardApp,
    ui: &mut egui::Ui,
    label: &str,
    current: Option<String>,
    target: HotkeyAction,
) {
    ui.label(label);
    let text = current
        .as_deref()
        .and_then(hotkeys::parse)
        .map(hotkeys::label)
        .unwrap_or_else(|| "—".into());
    if ui.button(format!("{text}  (change…)")).clicked() {
        app.start_capture(target);
    }
}

pub fn show(app: &mut SoundboardApp, ui: &mut egui::Ui) {
    if app.device_lists.is_none() {
        app.device_lists = Some((devices::list_inputs(), devices::list_outputs()));
    }
    let (inputs, outputs) = app.device_lists.clone().unwrap_or_default();
    let status = app.engine.status.lock().unwrap().clone();

    egui::Panel::top("settings_top").show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                app.screen = Screen::Board;
                app.device_lists = None;
            }
            ui.heading("Devices");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⟳ Refresh").clicked() {
                    app.device_lists = None;
                }
            });
        });
    });

    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut names = app.config.devices.clone();
            let mut changed = false;
            egui::Grid::new("devices")
                .num_columns(3)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    changed |= device_combo(ui, "Microphone", &mut names.mic, &inputs);
                    ui.label(status_text(&status.mic));
                    ui.end_row();
                    changed |=
                        device_combo(ui, "Virtual mic (to Discord)", &mut names.cable, &outputs);
                    ui.label(status_text(&status.cable));
                    ui.end_row();
                    changed |= device_combo(ui, "Headphones", &mut names.monitor, &outputs);
                    ui.label(status_text(&status.monitor));
                    ui.end_row();
                });
            if changed {
                app.config.devices = names.clone();
                app.persist();
                app.engine.send(EngineCommand::SetDevices(names));
            }

            if find_cable(&outputs).is_none() {
                ui.add_space(8.0);
                ui.colored_label(ui.visuals().warn_fg_color, NO_CABLE);
                ui.hyperlink(VB_CABLE_URL);
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label("Headphones volume");
                let mut v = app.config.volumes.monitor;
                let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
                if r.changed() {
                    app.config.volumes.monitor = v;
                    app.engine.params.monitor_gain.store(v);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    app.persist();
                }
            });
            if let Some(lat) = *app.engine.mic_latency.lock().unwrap() {
                ui.label(format!("Mic delay added: ~{} ms", lat.as_millis()));
            }

            ui.add_space(12.0);
            ui.heading("Shortcuts");
            egui::Grid::new("shortcuts")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    let stop = app.config.stop_hotkey.clone();
                    shortcut_row(app, ui, "Stop", stop, HotkeyAction::Stop);
                    ui.end_row();
                    let mode = app.config.mode_hotkey.clone();
                    shortcut_row(app, ui, "Go Live switch", mode, HotkeyAction::ToggleMode);
                    ui.end_row();
                });

            ui.add_space(12.0);
            ui.heading("Setting up Discord");
            for text in [DISCORD_HELP, PTT_HELP, GO_LIVE_HELP] {
                ui.add_space(4.0);
                ui.label(text);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn first_run_picks_cable_and_windows_defaults() {
        let mut cfg = Config::default();
        let outs = s(&["Speakers (Realtek)", "CABLE Input (VB-Audio Virtual Cable)"]);
        assert!(first_run_defaults(
            &mut cfg,
            &outs,
            Some("USB Mic".into()),
            Some("Speakers (Realtek)".into())
        ));
        assert_eq!(cfg.devices.mic.as_deref(), Some("USB Mic"));
        assert_eq!(
            cfg.devices.cable.as_deref(),
            Some("CABLE Input (VB-Audio Virtual Cable)")
        );
        assert_eq!(cfg.devices.monitor.as_deref(), Some("Speakers (Realtek)"));
    }

    #[test]
    fn first_run_never_picks_the_cable_as_mic_or_headphones() {
        let mut cfg = Config::default();
        let outs = s(&["CABLE Input (VB-Audio Virtual Cable)"]);
        first_run_defaults(
            &mut cfg,
            &outs,
            Some("CABLE Output (VB-Audio Virtual Cable)".into()),
            Some("CABLE Input (VB-Audio Virtual Cable)".into()),
        );
        assert_eq!(cfg.devices.mic, None);
        assert_eq!(cfg.devices.monitor, None);
    }

    #[test]
    fn first_run_leaves_existing_choices_alone() {
        let mut cfg = Config::default();
        cfg.devices.monitor = Some("Headset".into());
        assert!(!first_run_defaults(
            &mut cfg,
            &s(&["CABLE Input"]),
            Some("Mic".into()),
            None
        ));
        assert_eq!(cfg.devices.cable, None);
    }

    #[test]
    fn status_texts() {
        assert_eq!(status_text(&DeviceState::Open), "Working");
        assert_eq!(
            status_text(&DeviceState::Missing),
            "Not found — plugged in?"
        );
        assert_eq!(status_text(&DeviceState::NotSelected), "Not selected");
        assert_eq!(
            status_text(&DeviceState::Failed("busy".into())),
            "Error: busy"
        );
    }
}

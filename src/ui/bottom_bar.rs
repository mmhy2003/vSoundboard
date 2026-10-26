use eframe::egui;

use super::{SoundboardApp, format_time};
use crate::audio::engine::EngineCommand;
use crate::config::RoutingMode;
use crate::hotkeys;

pub fn show(app: &mut SoundboardApp, ui: &mut egui::Ui) {
    egui::Panel::bottom("bottom_bar").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            now_playing(app, ui);
            ui.separator();

            let stop_label = match app.config.stop_hotkey.as_deref().and_then(hotkeys::parse) {
                Some(hk) => format!("■ Stop ({})", hotkeys::label(hk)),
                None => "■ Stop".to_string(),
            };
            if ui.button(stop_label).clicked() {
                app.engine.send(EngineCommand::Stop);
            }
            ui.separator();

            let mut sounds = app.config.volumes.sounds;
            let (changed, done) = volume_slider(ui, "Sounds", &mut sounds);
            if changed {
                app.config.volumes.sounds = sounds;
                app.engine.params.sounds_gain.store(sounds);
            }
            if done {
                app.persist();
            }
            let mut mic = app.config.volumes.mic;
            let (changed, done) = volume_slider(ui, "Mic", &mut mic);
            if changed {
                app.config.volumes.mic = mic;
                app.engine.params.mic_gain.store(mic);
            }
            if done {
                app.persist();
            }
            ui.separator();

            let mut mode = app.config.mode;
            ui.selectable_value(&mut mode, RoutingMode::Normal, "Normal");
            ui.selectable_value(&mut mode, RoutingMode::GoLive, "Go Live")
                .on_hover_text("Sharing your whole screen with sound? Go Live sends sounds only to your headphones so viewers hear each sound once.");
            if mode != app.config.mode {
                app.set_mode(mode);
            }
        });
        ui.add_space(4.0);
    });
}

fn now_playing(app: &SoundboardApp, ui: &mut egui::Ui) {
    let pos = *app.engine.position.lock().unwrap();
    let text = match (app.playing, pos) {
        (Some(id), pos) => {
            let name = app.config.pad(id).map(|p| p.name.as_str()).unwrap_or("?");
            match pos {
                Some((_, elapsed, Some(total))) => {
                    format!(
                        "▶ {name}  {} / {}",
                        format_time(elapsed),
                        format_time(total)
                    )
                }
                Some((_, elapsed, None)) => format!("▶ {name}  {}", format_time(elapsed)),
                None => format!("▶ {name}"),
            }
        }
        (None, _) => "Nothing playing".to_string(),
    };
    ui.label(text);
}

/// Returns (changed, finished): finished is true when a drag ends or a click/keyboard change lands.
fn volume_slider(ui: &mut egui::Ui, label: &str, value: &mut f32) -> (bool, bool) {
    ui.label(label);
    let r = ui.add(egui::Slider::new(value, 0.0..=1.0).show_value(false));
    (
        r.changed(),
        r.drag_stopped() || (r.changed() && !r.dragged()),
    )
}

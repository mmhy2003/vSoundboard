use eframe::egui::{self, Color32, RichText, Vec2};

use super::{SUPPORTED_EXTENSIONS, Screen, SoundboardApp, grid_columns};
use crate::hotkeys;

const PAD_SIZE: Vec2 = Vec2::new(140.0, 64.0);

pub fn show(app: &mut SoundboardApp, ui: &mut egui::Ui) {
    egui::Panel::top("top_bar").show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button("+ Add sound").clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .add_filter("Audio", &SUPPORTED_EXTENSIONS)
                    .pick_files()
            {
                app.add_files(files);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙ Devices").clicked() {
                    app.screen = Screen::Settings;
                }
            });
        });
        if let Some(notice) = app.notice.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, notice);
                if ui.small_button("✕").clicked() {
                    app.notice = None;
                }
            });
        }
    });

    egui::CentralPanel::default().show(ui, |ui| {
        if app.config.pads.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Drop MP3, WAV, OGG or FLAC files here, or click “+ Add sound”.");
            });
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            let cols = grid_columns(ui.available_width());
            let pads: Vec<_> = app
                .config
                .pads
                .iter()
                .map(|p| (p.id, p.name.clone(), p.hotkey.clone()))
                .collect();
            for row in pads.chunks(cols) {
                ui.horizontal(|ui| {
                    for (id, name, hotkey) in row {
                        pad_button(app, ui, *id, name, hotkey.as_deref());
                    }
                });
                ui.add_space(4.0);
            }
        });
    });
}

fn pad_button(
    app: &mut SoundboardApp,
    ui: &mut egui::Ui,
    id: u64,
    name: &str,
    hotkey: Option<&str>,
) {
    let shortcut = hotkey
        .and_then(hotkeys::parse)
        .map(hotkeys::label)
        .unwrap_or_else(|| "—".into());
    let error = app.pad_errors.get(&id).cloned();
    let playing = app.playing == Some(id);

    let mut text = RichText::new(format!("{name}\n{shortcut}"));
    if error.is_some() {
        text = text.weak();
    }
    let mut button = egui::Button::new(text).min_size(PAD_SIZE).wrap();
    if playing {
        button = button.fill(Color32::from_rgb(46, 125, 88));
    }
    let mut response = ui.add_sized(PAD_SIZE, button);
    if let Some(err) = &error {
        response = response.on_hover_text(err);
    }
    if response.clicked() {
        app.play(id);
    }
}

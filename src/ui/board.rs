use eframe::egui::{self, Color32, DragAndDrop, RichText, Vec2};

use super::{SUPPORTED_EXTENSIONS, Screen, SoundboardApp, grid_columns};
use crate::hotkeys::{self, HotkeyAction};

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
                let pinned = app.config.always_on_top;
                if ui
                    .selectable_label(pinned, "📌 Always on top")
                    .on_hover_text("Keep vSoundboard above other windows")
                    .clicked()
                {
                    app.set_always_on_top(ui.ctx(), !pinned);
                }
            });
        });
        if let Some(notice) = app.notice.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, notice);
                if let Some(link) = app.notice_link.clone() {
                    ui.hyperlink_to("Open", link);
                }
                if ui.small_button("✕").clicked() {
                    app.notice = None;
                    app.notice_link = None;
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
        // Esc cancels a drag in progress.
        if DragAndDrop::has_any_payload(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            DragAndDrop::clear_payload(ui.ctx());
            ui.ctx().stop_dragging();
        }
        let dragged = DragAndDrop::payload::<u64>(ui.ctx()).map(|p| *p);
        let dragged_from = dragged.and_then(|d| app.config.pads.iter().position(|p| p.id == d));

        egui::ScrollArea::vertical().show(ui, |ui| {
            let cols = grid_columns(ui.available_width());
            let pads: Vec<_> = app
                .config
                .pads
                .iter()
                .map(|p| (p.id, p.name.clone(), p.hotkey.clone()))
                .collect();
            for (r, row) in pads.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    for (c, (id, name, hotkey)) in row.iter().enumerate() {
                        let index = r * cols + c;
                        pad_button(app, ui, index, *id, name, hotkey.as_deref(), dragged_from);
                    }
                });
                ui.add_space(4.0);
            }
            // Dropping in the empty space after the last pad moves it to the end.
            let rest = ui.allocate_response(
                Vec2::new(ui.available_width(), ui.available_height().max(PAD_SIZE.y)),
                egui::Sense::hover(),
            );
            if dragged.is_some() && rest.contains_pointer() {
                let stroke = egui::Stroke::new(3.0, ui.visuals().selection.bg_fill);
                ui.painter().rect_stroke(
                    rest.rect.shrink(2.0),
                    6.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(id) = rest.dnd_release_payload::<u64>() {
                app.move_pad(*id, usize::MAX);
            }
        });

        // The pad being dragged follows the pointer.
        if let (Some(id), Some(pos)) = (dragged, ui.ctx().pointer_hover_pos())
            && let Some(pad) = app.config.pad(id)
        {
            let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pad-drag"));
            let painter = ui.ctx().layer_painter(layer);
            let rect = egui::Rect::from_center_size(pos, PAD_SIZE);
            let fill = ui.visuals().selection.bg_fill;
            painter.rect_filled(rect, 6.0, fill);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &pad.name,
                egui::TextStyle::Button.resolve(ui.style()),
                ui.visuals().strong_text_color(),
            );
        }
    });
}

/// Where the drop marker goes on the pad under the pointer: moving a pad forward lands it
/// after the target (the others shift back), moving it backward lands it before.
pub fn marker_after(from: usize, to: usize) -> bool {
    from < to
}

fn pad_button(
    app: &mut SoundboardApp,
    ui: &mut egui::Ui,
    index: usize,
    id: u64,
    name: &str,
    hotkey: Option<&str>,
    dragged_from: Option<usize>,
) {
    let shortcut = hotkey
        .and_then(hotkeys::parse)
        .map(hotkeys::label)
        .unwrap_or_else(|| "—".into());
    let error = app.pad_errors.get(&id).cloned();
    let playing = app.playing == Some(id);

    let being_dragged = dragged_from == Some(index);
    let mut text = RichText::new(format!("{name}\n{shortcut}"));
    if error.is_some() || being_dragged {
        text = text.weak();
    }
    let mut button = egui::Button::new(text).min_size(PAD_SIZE).wrap();
    if playing {
        button = button.fill(Color32::from_rgb(46, 125, 88));
    }
    // Click plays; press-and-move starts a drag (egui tells the two apart).
    let mut response = ui.add_sized(PAD_SIZE, button).interact(egui::Sense::drag());
    response.dnd_set_drag_payload(id);
    if let Some(err) = &error {
        response = response.on_hover_text(err);
    }
    if let Some(from) = dragged_from
        && from != index
        && response.contains_pointer()
    {
        let r = response.rect;
        let x = if marker_after(from, index) {
            r.right() + 3.0
        } else {
            r.left() - 3.0
        };
        let stroke = egui::Stroke::new(3.0, ui.visuals().selection.bg_fill);
        ui.painter()
            .line_segment([egui::pos2(x, r.top()), egui::pos2(x, r.bottom())], stroke);
    }
    if let Some(dropped) = response.dnd_release_payload::<u64>()
        && *dropped != id
    {
        app.move_pad(*dropped, index);
    }
    if response.clicked() {
        app.play(id);
    }
    response.context_menu(|ui| pad_menu(app, ui, id));
}

fn pad_menu(app: &mut SoundboardApp, ui: &mut egui::Ui, id: u64) {
    let Some(pad) = app.config.pad(id) else {
        return;
    };
    let name = pad.name.clone();
    if ui.button("Rename").clicked() {
        app.renaming = Some((id, name));
        ui.close();
    }
    if ui.button("Set shortcut…").clicked() {
        app.start_capture(HotkeyAction::PlayPad(id));
        ui.close();
    }
    ui.horizontal(|ui| {
        ui.label("Volume");
        let mut vol = app.config.pad(id).map(|p| p.volume).unwrap_or(1.0);
        let r = ui.add(
            egui::Slider::new(&mut vol, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        );
        if r.changed() {
            if let Some(p) = app.config.pad_mut(id) {
                p.volume = vol;
            }
            if app.playing == Some(id) {
                app.engine.params.pad_gain.store(vol);
            }
        }
        if r.drag_stopped() || (r.changed() && !r.dragged()) {
            app.persist();
        }
    });
    ui.separator();
    if ui.button("Remove").clicked() {
        app.remove_pad(id);
        ui.close();
    }
}

/// Small dialog for renaming a pad; an empty name keeps the old one.
pub fn rename_dialog(app: &mut SoundboardApp, ctx: &egui::Context) {
    let Some((id, mut text)) = app.renaming.clone() else {
        return;
    };
    let mut done = None;
    let response = egui::Modal::new(egui::Id::unique("rename")).show(ctx, |ui| {
        ui.heading("Rename sound");
        let edit = ui.text_edit_singleline(&mut text);
        edit.request_focus();
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            done = Some(true);
        }
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() {
                done = Some(false);
            }
        });
    });
    if response.should_close() && done.is_none() {
        done = Some(false);
    }
    match done {
        None => app.renaming = Some((id, text)),
        Some(save) => {
            app.renaming = None;
            let trimmed = text.trim();
            if save && !trimmed.is_empty() {
                if let Some(p) = app.config.pad_mut(id) {
                    p.name = trimmed.to_string();
                }
                app.persist();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_marker_side_matches_where_the_pad_lands() {
        // [a,b,c,d]: a (0) onto c (2) lands after c; d (3) onto b (1) lands before b.
        assert!(marker_after(0, 2));
        assert!(!marker_after(3, 1));
    }
}

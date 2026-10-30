use eframe::egui;

use super::SoundboardApp;
use crate::config::Group;

/// An open New / Rename / Delete dialog for a group.
#[derive(Clone, Debug, PartialEq)]
pub enum GroupDialog {
    New {
        text: String,
        error: Option<String>,
    },
    Rename {
        index: usize,
        text: String,
        error: Option<String>,
    },
    Delete {
        index: usize,
    },
}

/// What to ask before deleting `group`; `None` when it's empty and can go without asking.
pub fn delete_prompt(group: &Group) -> Option<String> {
    let n = group.pads.len();
    if n == 0 {
        return None;
    }
    let sounds = if n == 1 { "sound" } else { "sounds" };
    Some(format!(
        "Delete “{}” and its {n} {sounds}? The audio files stay on your PC.",
        group.name
    ))
}

/// The row of group tabs under the top bar, with "+" for a new group.
pub fn tabs(app: &mut SoundboardApp, ui: &mut egui::Ui) {
    egui::ScrollArea::horizontal()
        .id_salt("group-tabs")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let names: Vec<String> = app.config.groups.iter().map(|g| g.name.clone()).collect();
                let deletable = names.len() > 1;
                for (i, name) in names.into_iter().enumerate() {
                    let tab = ui.selectable_label(i == app.config.selected_group, &name);
                    if tab.clicked() {
                        app.select_group(i);
                    }
                    tab.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            app.group_dialog = Some(GroupDialog::Rename {
                                index: i,
                                text: name.clone(),
                                error: None,
                            });
                            ui.close();
                        }
                        if ui
                            .add_enabled(deletable, egui::Button::new("Delete"))
                            .clicked()
                        {
                            match delete_prompt(&app.config.groups[i]) {
                                Some(_) => {
                                    app.group_dialog = Some(GroupDialog::Delete { index: i })
                                }
                                None => app.delete_group(i),
                            }
                            ui.close();
                        }
                    });
                }
                if ui.button("+").on_hover_text("New group").clicked() {
                    app.group_dialog = Some(GroupDialog::New {
                        text: String::new(),
                        error: None,
                    });
                }
            });
        });
}

/// The open group dialog, if any.
pub fn dialog(app: &mut SoundboardApp, ctx: &egui::Context) {
    let Some(state) = app.group_dialog.clone() else {
        return;
    };
    match state {
        GroupDialog::New { text, error } => name_dialog(app, ctx, None, text, error),
        GroupDialog::Rename { index, text, error } => {
            name_dialog(app, ctx, Some(index), text, error)
        }
        GroupDialog::Delete { index } => delete_dialog(app, ctx, index),
    }
}

fn name_dialog(
    app: &mut SoundboardApp,
    ctx: &egui::Context,
    index: Option<usize>,
    mut text: String,
    mut error: Option<String>,
) {
    let mut done = None;
    let response = egui::Modal::new(egui::Id::unique("group-name")).show(ctx, |ui| {
        ui.heading(if index.is_some() {
            "Rename group"
        } else {
            "New group"
        });
        let edit = ui.text_edit_singleline(&mut text);
        edit.request_focus();
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            done = Some(true);
        }
        if let Some(err) = &error {
            ui.colored_label(ui.visuals().error_fg_color, err);
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
    let keep_open = |text, error| match index {
        Some(index) => GroupDialog::Rename { index, text, error },
        None => GroupDialog::New { text, error },
    };
    match done {
        None => app.group_dialog = Some(keep_open(text, error)),
        Some(false) => app.group_dialog = None,
        Some(true) => match app.config.check_group_name(&text, index) {
            Err(e) => {
                error = Some(e);
                app.group_dialog = Some(keep_open(text, error));
            }
            Ok(name) => {
                app.group_dialog = None;
                match index {
                    Some(i) => app.rename_group(i, name),
                    None => app.create_group(name),
                }
            }
        },
    }
}

fn delete_dialog(app: &mut SoundboardApp, ctx: &egui::Context, index: usize) {
    let Some(prompt) = app.config.groups.get(index).and_then(delete_prompt) else {
        app.group_dialog = None;
        return;
    };
    let mut done = None;
    let response = egui::Modal::new(egui::Id::unique("group-delete")).show(ctx, |ui| {
        ui.heading("Delete group");
        ui.label(prompt);
        ui.horizontal(|ui| {
            if ui.button("Delete").clicked() {
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
        None => {}
        Some(delete) => {
            app.group_dialog = None;
            if delete {
                app.delete_group(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Pad;

    #[test]
    fn delete_prompt_copy() {
        let mut g = Group {
            name: "Gaming".into(),
            pads: vec![],
        };
        assert_eq!(delete_prompt(&g), None);
        g.pads.push(Pad::default());
        assert_eq!(
            delete_prompt(&g).unwrap(),
            "Delete “Gaming” and its 1 sound? The audio files stay on your PC."
        );
        g.pads.push(Pad::default());
        assert_eq!(
            delete_prompt(&g).unwrap(),
            "Delete “Gaming” and its 2 sounds? The audio files stay on your PC."
        );
    }
}

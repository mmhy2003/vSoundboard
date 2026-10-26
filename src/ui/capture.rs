use eframe::egui;
use global_hotkey::hotkey::HotKey;

use super::SoundboardApp;
use crate::config::Config;
use crate::hotkeys::{self, HotkeyAction};

/// An open "press the keys" dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub target: HotkeyAction,
    /// The combo to save; `None` means clear the shortcut.
    pub pending: Option<HotKey>,
    pub error: Option<String>,
}

impl Capture {
    pub fn new(target: HotkeyAction) -> Self {
        Self {
            target,
            pending: None,
            error: None,
        }
    }
}

fn slot(cfg: &mut Config, target: HotkeyAction) -> Option<&mut Option<String>> {
    match target {
        HotkeyAction::PlayPad(id) => cfg.pad_mut(id).map(|p| &mut p.hotkey),
        HotkeyAction::Stop => Some(&mut cfg.stop_hotkey),
        HotkeyAction::ToggleMode => Some(&mut cfg.mode_hotkey),
    }
}

/// Store `cap.pending` as the target's shortcut, or explain why it can't be used.
pub fn apply_capture(cfg: &mut Config, cap: &Capture) -> Result<(), String> {
    if let Some(hk) = cap.pending
        && let Some(other) = hotkeys::find_conflict(cfg, hk, cap.target)
    {
        return Err(hotkeys::conflict_message(cfg, hk, other));
    }
    let value = cap.pending.map(hotkeys::to_config_string);
    match slot(cfg, cap.target) {
        Some(s) => {
            *s = value;
            Ok(())
        }
        None => Err("That sound no longer exists.".into()),
    }
}

/// The target's current shortcut string, for undoing a save Windows rejects.
fn current(cfg: &mut Config, target: HotkeyAction) -> Option<String> {
    slot(cfg, target).and_then(|s| s.clone())
}

enum Outcome {
    Open,
    Cancel,
    Save(Option<HotKey>),
}

/// Who a shortcut belongs to, for dialog titles.
pub fn target_name(cfg: &Config, target: HotkeyAction) -> String {
    match target {
        HotkeyAction::PlayPad(id) => cfg.pad(id).map(|p| p.name.clone()).unwrap_or_default(),
        HotkeyAction::Stop => "Stop".into(),
        HotkeyAction::ToggleMode => "Go Live switch".into(),
    }
}

pub fn show(app: &mut SoundboardApp, ctx: &egui::Context) {
    let Some(cap) = app.capture.clone() else {
        return;
    };
    let title = format!(
        "Press the keys for “{}”",
        target_name(&app.config, cap.target)
    );

    // The first non-modifier key press, with whatever modifiers are held, is the combo.
    let mut outcome = Outcome::Open;
    ctx.input(|i| {
        for ev in &i.events {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } = ev
            {
                if *key == egui::Key::Escape {
                    outcome = Outcome::Cancel;
                } else if let Some(hk) =
                    hotkeys::from_egui(physical_key.unwrap_or(*key), *modifiers)
                {
                    outcome = Outcome::Save(Some(hk));
                }
            }
        }
    });

    let response = egui::Modal::new(egui::Id::unique("capture")).show(ctx, |ui| {
        ui.set_min_width(300.0);
        ui.heading(&title);
        ui.add_space(6.0);
        ui.label("Hold Ctrl, Shift or Alt if you like, then press a key. Esc cancels.");
        if let Some(err) = &cap.error {
            ui.add_space(6.0);
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                outcome = Outcome::Save(None);
            }
            if ui.button("Cancel").clicked() {
                outcome = Outcome::Cancel;
            }
        });
    });
    if response.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Cancel;
    }

    match outcome {
        Outcome::Open => {}
        Outcome::Cancel => {
            app.capture = None;
            app.sync_hotkeys();
        }
        Outcome::Save(pending) => {
            let attempt = Capture {
                pending,
                error: None,
                ..cap.clone()
            };
            let previous = current(&mut app.config, cap.target);
            match apply_capture(&mut app.config, &attempt) {
                Err(msg) => {
                    app.capture = Some(Capture {
                        error: Some(msg),
                        ..cap
                    })
                }
                Ok(()) => {
                    app.persist();
                    let failures = app.sync_hotkeys();
                    if let Some((_, msg)) = failures.into_iter().find(|(a, _)| *a == cap.target) {
                        // Windows refused it (another app holds it): put the old one back.
                        if let Some(s) = slot(&mut app.config, cap.target) {
                            *s = previous;
                        }
                        app.persist();
                        app.suspend_hotkeys();
                        app.capture = Some(Capture {
                            error: Some(msg),
                            ..cap
                        });
                    } else {
                        app.capture = None;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    fn ctrl(code: Code) -> HotKey {
        HotKey::new(Some(Modifiers::CONTROL), code)
    }

    fn cfg() -> Config {
        let mut c = Config {
            stop_hotkey: Some("control+Digit0".into()),
            ..Config::default()
        };
        c.add_pad("airhorn.wav".into());
        c.add_pad("bruh.wav".into());
        c
    }

    #[test]
    fn saves_to_pad() {
        let mut c = cfg();
        let cap = Capture {
            pending: Some(ctrl(Code::Digit1)),
            ..Capture::new(HotkeyAction::PlayPad(2))
        };
        apply_capture(&mut c, &cap).unwrap();
        assert_eq!(c.pad(2).unwrap().hotkey.as_deref(), Some("control+Digit1"));
    }

    #[test]
    fn saves_to_stop_and_mode() {
        let mut c = cfg();
        let cap = Capture {
            pending: Some(ctrl(Code::KeyS)),
            ..Capture::new(HotkeyAction::Stop)
        };
        apply_capture(&mut c, &cap).unwrap();
        assert_eq!(c.stop_hotkey.as_deref(), Some("control+KeyS"));
        let cap = Capture {
            pending: Some(HotKey::new(None, Code::F9)),
            ..Capture::new(HotkeyAction::ToggleMode)
        };
        apply_capture(&mut c, &cap).unwrap();
        assert_eq!(c.mode_hotkey.as_deref(), Some("F9"));
    }

    #[test]
    fn rejects_combo_used_by_stop() {
        let mut c = cfg();
        let cap = Capture {
            pending: Some(ctrl(Code::Digit0)),
            ..Capture::new(HotkeyAction::PlayPad(1))
        };
        assert_eq!(
            apply_capture(&mut c, &cap),
            Err("Ctrl+0 is already used by Stop.".to_string())
        );
        assert_eq!(c.pad(1).unwrap().hotkey, None);
    }

    #[test]
    fn clear_sets_none() {
        let mut c = cfg();
        c.pad_mut(1).unwrap().hotkey = Some("control+Digit1".into());
        apply_capture(&mut c, &Capture::new(HotkeyAction::PlayPad(1))).unwrap();
        assert_eq!(c.pad(1).unwrap().hotkey, None);
    }
}

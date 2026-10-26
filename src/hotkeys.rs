use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use crossbeam_channel::Sender;
use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::audio::engine::EngineCommand;
use crate::config::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    PlayPad(u64),
    Stop,
    ToggleMode,
}

pub fn parse(s: &str) -> Option<HotKey> {
    HotKey::from_str(s).ok()
}

/// Canonical form stored in config, e.g. "control+Digit1".
pub fn to_config_string(hk: HotKey) -> String {
    hk.into_string()
}

/// Human-readable form, e.g. "Ctrl+1", "Ctrl+Shift+F9", "Num 5".
pub fn label(hk: HotKey) -> String {
    let mut out = String::new();
    for (m, name) in [
        (Modifiers::CONTROL, "Ctrl+"),
        (Modifiers::SHIFT, "Shift+"),
        (Modifiers::ALT, "Alt+"),
        (Modifiers::META, "Win+"),
    ] {
        if hk.mods.contains(m) {
            out.push_str(name);
        }
    }
    out.push_str(&key_label(hk.key));
    out
}

fn key_label(code: Code) -> String {
    let name = code.to_string();
    let special = match code {
        Code::NumpadAdd => Some("Num +"),
        Code::NumpadSubtract => Some("Num -"),
        Code::NumpadMultiply => Some("Num *"),
        Code::NumpadDivide => Some("Num /"),
        Code::NumpadDecimal => Some("Num ."),
        Code::NumpadEnter => Some("Num Enter"),
        Code::ArrowUp => Some("Up"),
        Code::ArrowDown => Some("Down"),
        Code::ArrowLeft => Some("Left"),
        Code::ArrowRight => Some("Right"),
        Code::Minus => Some("-"),
        Code::Equal => Some("="),
        Code::Comma => Some(","),
        Code::Period => Some("."),
        Code::Slash => Some("/"),
        Code::Backslash => Some("\\"),
        Code::Semicolon => Some(";"),
        Code::Quote => Some("'"),
        Code::Backquote => Some("`"),
        Code::BracketLeft => Some("["),
        Code::BracketRight => Some("]"),
        _ => None,
    };
    if let Some(s) = special {
        return s.to_string();
    }
    if let Some(rest) = name.strip_prefix("Numpad") {
        return format!("Num {rest}");
    }
    name.strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
        .unwrap_or(&name)
        .to_string()
}

/// All parsable bindings in the config; unset or unparsable entries are skipped.
pub fn bindings(cfg: &Config) -> Vec<(HotKey, HotkeyAction)> {
    let pads = cfg
        .pads
        .iter()
        .map(|p| (p.hotkey.as_deref(), HotkeyAction::PlayPad(p.id)));
    let extra = [
        (cfg.stop_hotkey.as_deref(), HotkeyAction::Stop),
        (cfg.mode_hotkey.as_deref(), HotkeyAction::ToggleMode),
    ];
    pads.chain(extra)
        .filter_map(|(s, action)| Some((parse(s?)?, action)))
        .collect()
}

/// The action other than `for_action` that already uses `candidate`, if any.
pub fn find_conflict(
    cfg: &Config,
    candidate: HotKey,
    for_action: HotkeyAction,
) -> Option<HotkeyAction> {
    bindings(cfg)
        .into_iter()
        .find(|(hk, action)| {
            *action != for_action && hk.mods == candidate.mods && hk.key == candidate.key
        })
        .map(|(_, action)| action)
}

pub fn conflict_message(cfg: &Config, hk: HotKey, other: HotkeyAction) -> String {
    let owner = match other {
        HotkeyAction::PlayPad(id) => {
            let name = cfg
                .pad(id)
                .map(|p| p.name.as_str())
                .unwrap_or("another sound");
            format!("“{name}”")
        }
        HotkeyAction::Stop => "Stop".to_string(),
        HotkeyAction::ToggleMode => "the Go Live switch".to_string(),
    };
    format!("{} is already used by {owner}.", label(hk))
}

/// Map a key press captured by egui to a global hotkey; None for keys Windows hotkeys can't use.
pub fn from_egui(key: egui::Key, mods: egui::Modifiers) -> Option<HotKey> {
    use egui::Key as K;
    let code = match key {
        K::A => Code::KeyA,
        K::B => Code::KeyB,
        K::C => Code::KeyC,
        K::D => Code::KeyD,
        K::E => Code::KeyE,
        K::F => Code::KeyF,
        K::G => Code::KeyG,
        K::H => Code::KeyH,
        K::I => Code::KeyI,
        K::J => Code::KeyJ,
        K::K => Code::KeyK,
        K::L => Code::KeyL,
        K::M => Code::KeyM,
        K::N => Code::KeyN,
        K::O => Code::KeyO,
        K::P => Code::KeyP,
        K::Q => Code::KeyQ,
        K::R => Code::KeyR,
        K::S => Code::KeyS,
        K::T => Code::KeyT,
        K::U => Code::KeyU,
        K::V => Code::KeyV,
        K::W => Code::KeyW,
        K::X => Code::KeyX,
        K::Y => Code::KeyY,
        K::Z => Code::KeyZ,
        // egui doesn't tell numpad digits apart from the top row.
        K::Num0 => Code::Digit0,
        K::Num1 => Code::Digit1,
        K::Num2 => Code::Digit2,
        K::Num3 => Code::Digit3,
        K::Num4 => Code::Digit4,
        K::Num5 => Code::Digit5,
        K::Num6 => Code::Digit6,
        K::Num7 => Code::Digit7,
        K::Num8 => Code::Digit8,
        K::Num9 => Code::Digit9,
        K::F1 => Code::F1,
        K::F2 => Code::F2,
        K::F3 => Code::F3,
        K::F4 => Code::F4,
        K::F5 => Code::F5,
        K::F6 => Code::F6,
        K::F7 => Code::F7,
        K::F8 => Code::F8,
        K::F9 => Code::F9,
        K::F10 => Code::F10,
        K::F11 => Code::F11,
        K::F12 => Code::F12,
        K::F13 => Code::F13,
        K::F14 => Code::F14,
        K::F15 => Code::F15,
        K::F16 => Code::F16,
        K::F17 => Code::F17,
        K::F18 => Code::F18,
        K::F19 => Code::F19,
        K::F20 => Code::F20,
        K::F21 => Code::F21,
        K::F22 => Code::F22,
        K::F23 => Code::F23,
        K::F24 => Code::F24,
        K::ArrowUp => Code::ArrowUp,
        K::ArrowDown => Code::ArrowDown,
        K::ArrowLeft => Code::ArrowLeft,
        K::ArrowRight => Code::ArrowRight,
        K::Space => Code::Space,
        K::Insert => Code::Insert,
        K::Delete => Code::Delete,
        K::Home => Code::Home,
        K::End => Code::End,
        K::PageUp => Code::PageUp,
        K::PageDown => Code::PageDown,
        K::Enter => Code::Enter,
        K::Backspace => Code::Backspace,
        K::Tab => Code::Tab,
        K::Minus => Code::Minus,
        K::Equals => Code::Equal,
        K::Comma => Code::Comma,
        K::Period => Code::Period,
        K::Slash => Code::Slash,
        K::Backslash => Code::Backslash,
        K::Semicolon => Code::Semicolon,
        K::Quote => Code::Quote,
        K::Backtick => Code::Backquote,
        K::OpenBracket => Code::BracketLeft,
        K::CloseBracket => Code::BracketRight,
        K::Pause => Code::Pause,
        K::ScrollLock => Code::ScrollLock,
        K::PrintScreen => Code::PrintScreen,
        K::MediaPlayPause => Code::MediaPlayPause,
        K::MediaStop => Code::MediaStop,
        K::MediaTrackNext => Code::MediaTrackNext,
        K::MediaTrackPrevious => Code::MediaTrackPrevious,
        _ => return None,
    };
    let mut m = Modifiers::empty();
    if mods.ctrl {
        m |= Modifiers::CONTROL;
    }
    if mods.shift {
        m |= Modifiers::SHIFT;
    }
    if mods.alt {
        m |= Modifiers::ALT;
    }
    Some(HotKey::new(Some(m), code))
}

/// Registers shortcuts with Windows and sends their actions straight to the engine, so they
/// work while the window is hidden. Must be created on the thread running the window's event loop.
pub struct HotkeyRegistry {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    actions: Arc<RwLock<HashMap<u32, HotkeyAction>>>,
}

impl HotkeyRegistry {
    pub fn new(engine_tx: Sender<EngineCommand>) -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        let actions: Arc<RwLock<HashMap<u32, HotkeyAction>>> = Arc::default();
        let lookup = actions.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state != HotKeyState::Pressed {
                return;
            }
            let action = lookup.read().ok().and_then(|m| m.get(&e.id).copied());
            let cmd = match action {
                Some(HotkeyAction::PlayPad(id)) => EngineCommand::PlayPad(id),
                Some(HotkeyAction::Stop) => EngineCommand::Stop,
                Some(HotkeyAction::ToggleMode) => EngineCommand::ToggleMode,
                None => return,
            };
            let _ = engine_tx.send(cmd);
        }));
        Ok(Self {
            manager,
            registered: Vec::new(),
            actions,
        })
    }

    /// Unregister all, then register `bindings(cfg)`. Returns failures as (action, user message).
    pub fn sync(&mut self, cfg: &Config) -> Vec<(HotkeyAction, String)> {
        self.suspend();
        let mut failures = Vec::new();
        let mut actions = HashMap::new();
        for (hk, action) in bindings(cfg) {
            match self.manager.register(hk) {
                Ok(()) => {
                    self.registered.push(hk);
                    actions.insert(hk.id(), action);
                }
                Err(global_hotkey::Error::AlreadyRegistered(_)) => {
                    failures.push((
                        action,
                        format!("{} is already used by another app.", label(hk)),
                    ));
                }
                Err(e) => failures.push((action, format!("Couldn't set {}: {e}", label(hk)))),
            }
        }
        if let Ok(mut m) = self.actions.write() {
            *m = actions;
        }
        failures
    }

    /// Unregister everything (while capturing a new shortcut, so existing ones reach the window).
    pub fn suspend(&mut self) {
        let _ = self.manager.unregister_all(&self.registered);
        self.registered.clear();
        if let Ok(mut m) = self.actions.write() {
            m.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn hk(mods: Option<Modifiers>, key: Code) -> HotKey {
        HotKey::new(mods, key)
    }

    fn cfg_with_pad(name: &str, hotkey: &str) -> Config {
        let mut cfg = Config::default();
        let id = cfg.add_pad(PathBuf::from(format!("{name}.wav")));
        cfg.pad_mut(id).unwrap().hotkey = Some(hotkey.to_string());
        cfg.pad_mut(id).unwrap().name = name.to_string();
        cfg
    }

    #[test]
    fn config_string_round_trip() {
        for k in [
            hk(Some(Modifiers::CONTROL), Code::Digit1),
            hk(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::F9),
            hk(Some(Modifiers::ALT), Code::NumpadAdd),
            hk(None, Code::F13),
        ] {
            assert_eq!(parse(&to_config_string(k)), Some(k));
        }
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse("ctrl++"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn labels() {
        assert_eq!(label(hk(Some(Modifiers::CONTROL), Code::Digit1)), "Ctrl+1");
        assert_eq!(
            label(hk(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::F9)),
            "Ctrl+Shift+F9"
        );
        assert_eq!(label(hk(None, Code::Numpad5)), "Num 5");
        assert_eq!(label(hk(Some(Modifiers::ALT), Code::KeyA)), "Alt+A");
    }

    #[test]
    fn conflict_with_other_pad() {
        let cfg = cfg_with_pad("Airhorn", "control+Digit1");
        let ctrl1 = hk(Some(Modifiers::CONTROL), Code::Digit1);
        assert_eq!(
            find_conflict(&cfg, ctrl1, HotkeyAction::PlayPad(2)),
            Some(HotkeyAction::PlayPad(1))
        );
    }

    #[test]
    fn no_conflict_with_self() {
        let cfg = cfg_with_pad("Airhorn", "control+Digit1");
        let ctrl1 = hk(Some(Modifiers::CONTROL), Code::Digit1);
        assert_eq!(find_conflict(&cfg, ctrl1, HotkeyAction::PlayPad(1)), None);
    }

    #[test]
    fn conflict_with_stop_and_mode() {
        let mut cfg = Config {
            stop_hotkey: Some("control+Digit0".into()),
            mode_hotkey: Some("F9".into()),
            ..Config::default()
        };
        cfg.add_pad("a.wav".into());
        assert_eq!(
            find_conflict(
                &cfg,
                hk(Some(Modifiers::CONTROL), Code::Digit0),
                HotkeyAction::PlayPad(1)
            ),
            Some(HotkeyAction::Stop)
        );
        assert_eq!(
            find_conflict(&cfg, hk(None, Code::F9), HotkeyAction::PlayPad(1)),
            Some(HotkeyAction::ToggleMode)
        );
        assert_eq!(
            find_conflict(&cfg, hk(None, Code::F9), HotkeyAction::ToggleMode),
            None
        );
    }

    #[test]
    fn conflict_message_text() {
        let cfg = cfg_with_pad("Airhorn", "control+Digit1");
        assert_eq!(
            conflict_message(
                &cfg,
                hk(Some(Modifiers::CONTROL), Code::Digit1),
                HotkeyAction::PlayPad(1)
            ),
            "Ctrl+1 is already used by “Airhorn”."
        );
        assert_eq!(
            conflict_message(
                &cfg,
                hk(Some(Modifiers::CONTROL), Code::Digit0),
                HotkeyAction::Stop
            ),
            "Ctrl+0 is already used by Stop."
        );
        assert_eq!(
            conflict_message(&cfg, hk(None, Code::F9), HotkeyAction::ToggleMode),
            "F9 is already used by the Go Live switch."
        );
    }

    #[test]
    fn from_egui_maps() {
        assert_eq!(
            from_egui(egui::Key::Num1, egui::Modifiers::CTRL),
            Some(hk(Some(Modifiers::CONTROL), Code::Digit1))
        );
        assert_eq!(
            from_egui(egui::Key::F9, egui::Modifiers::NONE),
            Some(hk(None, Code::F9))
        );
        assert_eq!(
            from_egui(egui::Key::A, egui::Modifiers::SHIFT | egui::Modifiers::ALT),
            Some(hk(Some(Modifiers::SHIFT | Modifiers::ALT), Code::KeyA))
        );
        assert_eq!(from_egui(egui::Key::Copy, egui::Modifiers::NONE), None);
        assert_eq!(
            from_egui(egui::Key::ControlLeft, egui::Modifiers::CTRL),
            None
        );
    }

    #[test]
    fn bindings_skip_garbage() {
        let mut cfg = cfg_with_pad("Airhorn", "control+Digit1");
        let bad = cfg.add_pad("bad.wav".into());
        cfg.pad_mut(bad).unwrap().hotkey = Some("ctrl++".into());
        cfg.stop_hotkey = Some("control+Digit0".into());
        let b = bindings(&cfg);
        assert_eq!(b.len(), 2);
        assert!(b.contains(&(
            hk(Some(Modifiers::CONTROL), Code::Digit1),
            HotkeyAction::PlayPad(1)
        )));
        assert!(b.contains(&(
            hk(Some(Modifiers::CONTROL), Code::Digit0),
            HotkeyAction::Stop
        )));
    }

    #[test]
    fn registry_reports_combo_held_elsewhere() {
        // Ctrl+Alt+Shift+F23: nothing real uses it. The first registry stands in for "another app".
        let cfg = cfg_with_pad("Airhorn", "shift+control+alt+F23");
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut first = HotkeyRegistry::new(tx.clone()).unwrap();
        assert!(first.sync(&cfg).is_empty());

        let mut second = HotkeyRegistry::new(tx).unwrap();
        let failures = second.sync(&cfg);
        assert_eq!(
            failures,
            vec![(
                HotkeyAction::PlayPad(1),
                "Ctrl+Shift+Alt+F23 is already used by another app.".to_string()
            )]
        );

        // Once released, it can be taken.
        first.suspend();
        assert!(second.sync(&cfg).is_empty());
        second.suspend();
    }
}

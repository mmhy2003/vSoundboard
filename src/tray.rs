//! System tray icon. Its handlers run outside the UI loop (the window may be hidden), so they only
//! set flags and wake the app; `SoundboardApp::logic` acts on them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;
use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::audio::engine::EngineCommand;

#[derive(Default)]
pub struct TrayRequests {
    pub show: AtomicBool,
    pub quit: AtomicBool,
}

fn icon() -> Icon {
    Icon::from_rgba(crate::icon::disc_rgba(32), 32, 32).expect("valid icon size")
}

/// Create the tray icon. Must run on the UI thread; keep the returned icon alive.
pub fn install(
    ctx: egui::Context,
    engine_tx: Sender<EngineCommand>,
    requests: Arc<TrayRequests>,
) -> Result<TrayIcon, String> {
    let show = MenuItem::new("Show", true, None);
    let stop = MenuItem::new("Stop sound", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&show, &stop, &quit])
        .map_err(|e| e.to_string())?;

    let tray = TrayIconBuilder::new()
        .with_tooltip("vSoundboard")
        .with_icon(icon())
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())?;

    let (show_id, stop_id, quit_id) = (show.id().clone(), stop.id().clone(), quit.id().clone());
    let (req, wake) = (requests.clone(), ctx.clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == show_id {
            req.show.store(true, Ordering::Relaxed);
        } else if e.id == stop_id {
            let _ = engine_tx.send(EngineCommand::Stop);
        } else if e.id == quit_id {
            req.quit.store(true, Ordering::Relaxed);
        }
        wake.request_repaint();
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            requests.show.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        }
    }));
    Ok(tray)
}

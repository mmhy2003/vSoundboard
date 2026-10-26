#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use eframe::egui;
use vsoundboard::audio::{SharedParams, devices, engine};
use vsoundboard::cli;
use vsoundboard::config::{self, LoadOutcome};
use vsoundboard::single_instance;
use vsoundboard::ui::{self, SoundboardApp};

/// egui's built-in fonts lack Arabic and many symbols; fall back to Windows' own fonts.
fn add_system_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, file) in [("segoe", "segoeui.ttf"), ("segoe-symbol", "seguisym.ttf")] {
        let path = std::path::Path::new(r"C:\Windows\Fonts").join(file);
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.into());
            }
        }
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result {
    let args = cli::parse(std::env::args());
    if args.check_vbcable {
        let found = devices::find_cable(&devices::list_outputs()).is_some();
        std::process::exit(if found { 0 } else { 1 });
    }
    let Some(_instance) = single_instance::acquire("vSoundboard") else {
        // A second launch brings the running copy forward, unless it's the hidden autostart one.
        if !args.minimized {
            single_instance::show_existing("vSoundboard");
        }
        return Ok(());
    };
    let path = config::config_path();
    let outcome = config::load(&path);
    let notice = match &outcome {
        LoadOutcome::RecoveredFromCorrupt { backup, .. } => Some(format!(
            "Your settings file was damaged, so vSoundboard started fresh. The old file was kept as {}.",
            backup.display()
        )),
        LoadOutcome::Unreadable { error, .. } => Some(format!(
            "Couldn't read your settings ({error}). Changes won't be saved this time; restart vSoundboard to try again."
        )),
        _ => None,
    };
    let read_only = matches!(outcome, LoadOutcome::Unreadable { .. });
    let mut cfg = outcome.into_config();
    if !read_only
        && ui::first_run_defaults(
            &mut cfg,
            &devices::list_outputs(),
            devices::default_input_name(),
            devices::default_output_name(),
        )
    {
        let _ = config::save(&path, &cfg);
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("vSoundboard")
        .with_inner_size([680.0, 480.0])
        .with_min_inner_size([360.0, 300.0])
        .with_drag_and_drop(true);
    if args.minimized {
        // eframe always shows the window after its first frame, so a hidden start puts that
        // frame off-screen without focus, and the app hides the window right after.
        viewport = viewport
            .with_position([-32_000.0, -32_000.0])
            .with_active(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "vSoundboard",
        options,
        Box::new(move |cc| {
            add_system_fonts(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let params = Arc::new(SharedParams::from_config(&cfg));
            let eng = engine::start(
                cfg.devices.clone(),
                ui::engine_pads(&cfg),
                params,
                Arc::new(move || ctx.request_repaint()),
            );
            let mut app = SoundboardApp::new(cfg, path, eng);
            app.notice = notice;
            app.read_only = read_only;
            if args.minimized {
                app.hide_frames = 3;
                app.parked_offscreen = true;
            }
            app.check_files();
            app.enable_hotkeys();
            app.enable_tray(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

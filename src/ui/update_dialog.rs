//! The update dialog and its rules: it only appears while vSoundboard's window is focused,
//! except right after the friend clicked "Check for updates".

use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;
use semver::Version;

use super::SoundboardApp;
use crate::update::handoff::{SETUP_MUTEX, launch_installer, wait_for_setup_mutex};
use crate::update::worker::UpdateState;
use crate::update::{RELEASES_PAGE, SNOOZE_SECS, Update, should_prompt};

pub const RESTART_WARNING: &str =
    "Updating restarts vSoundboard, so your voice in Discord drops for a few seconds.";
pub const ONLY_INSTALLED: &str = "Updates are only available in the installed app";
const SETUP_WAIT: Duration = Duration::from_secs(15);

/// Whether the dialog may be on screen now.
pub fn dialog_should_show(
    state: &UpdateState,
    focused: bool,
    visible: bool,
    now: u64,
    snoozed_until: Option<u64>,
) -> bool {
    if !visible {
        return false;
    }
    match state {
        UpdateState::Available { manual: true, .. } => true,
        UpdateState::Available { manual: false, .. } => {
            focused && should_prompt(now, snoozed_until, false)
        }
        UpdateState::Downloading { .. }
        | UpdateState::Ready { .. }
        | UpdateState::DownloadFailed { .. } => true,
        _ => false,
    }
}

/// "vSoundboard X.Y.Z is available (you have A.B.C)."
pub fn available_text(new: &Version, current: &Version) -> String {
    format!("vSoundboard {new} is available (you have {current}).")
}

/// The settings screen's one-line update status.
pub fn status_text(state: &UpdateState, current: &Version, can_install: bool) -> String {
    match state {
        UpdateState::Checking => "Checking…".into(),
        UpdateState::UpToDate => format!("You're up to date (vSoundboard {current})"),
        UpdateState::Available { update, .. }
        | UpdateState::Downloading { update, .. }
        | UpdateState::Ready { update, .. }
        | UpdateState::DownloadFailed { update, .. } => {
            format!("vSoundboard {} is available", update.version)
        }
        UpdateState::CheckFailed(e) => format!("Couldn't check for updates: {e}"),
        UpdateState::Idle if !can_install => ONLY_INSTALLED.into(),
        UpdateState::Idle => String::new(),
    }
}

/// Where the handoff to the installer stands.
pub enum Handoff {
    /// Waiting for the installer's SetupMutex (result arrives on the channel).
    Waiting(mpsc::Receiver<bool>),
    /// The installer didn't start; shown until the friend closes it.
    Failed { update: Update, reason: String },
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn snooze(app: &mut SoundboardApp) {
    app.config.updates.snoozed_until = Some(now_secs() + SNOOZE_SECS);
    app.persist();
    if let Some(w) = &app.updater {
        w.dismiss();
    }
}

/// Hand over to the downloaded installer; the app quits once the installer is really running.
fn start_handoff(app: &mut SoundboardApp, update: &Update, path: &std::path::Path) {
    app.config.updates.attempted_version = Some(update.version.to_string());
    app.persist();
    match launch_installer(path) {
        Ok(()) => {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(wait_for_setup_mutex(SETUP_MUTEX, SETUP_WAIT));
            });
            app.handoff = Some(Handoff::Waiting(rx));
        }
        Err(e) => handoff_failed(app, update.clone(), e),
    }
}

fn handoff_failed(app: &mut SoundboardApp, update: Update, reason: String) {
    app.config.updates.attempted_version = None;
    app.persist();
    app.handoff = Some(Handoff::Failed { update, reason });
}

pub fn show(app: &mut SoundboardApp, ctx: &egui::Context) {
    let Some(worker) = &app.updater else { return };
    let state = worker.state();
    let current = current_version();

    // Installer handoff in progress or failed: that takes over the dialog.
    if let Some(Handoff::Waiting(rx)) = &app.handoff {
        match rx.try_recv() {
            Ok(true) => {
                app.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Ok(false) | Err(mpsc::TryRecvError::Disconnected) => {
                if let UpdateState::Ready { update, .. } = &state {
                    let reason = "The installer didn't start in time.".to_string();
                    handoff_failed(app, update.clone(), reason);
                } else {
                    app.handoff = None;
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        egui::Modal::new(egui::Id::unique("update")).show(ctx, |ui| {
            ui.label("Starting the installer…");
        });
        ctx.request_repaint_after(Duration::from_millis(100));
        return;
    }
    if let Some(Handoff::Failed { update, reason }) = &app.handoff {
        let (update, reason) = (update.clone(), reason.clone());
        let mut close = false;
        egui::Modal::new(egui::Id::unique("update")).show(ctx, |ui| {
            ui.label(format!("The update couldn't be started. {reason}"));
            ui.horizontal(|ui| {
                if ui.button("Download manually").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(&update.page));
                }
                close = ui.button("Close").clicked();
            });
        });
        if close {
            app.handoff = None;
            if let Some(w) = &app.updater {
                w.dismiss();
            }
        }
        return;
    }

    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
    if !dialog_should_show(
        &state,
        focused,
        true,
        now_secs(),
        app.config.updates.snoozed_until,
    ) {
        return;
    }

    enum Action {
        None,
        Later,
        UpdateNow(Update),
        Cancel,
        Retry(Update),
        Close,
    }
    let mut action = Action::None;
    let can_install = app.can_install;
    let response = egui::Modal::new(egui::Id::unique("update")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        match &state {
            UpdateState::Available { update, .. } => {
                ui.heading("Update available");
                ui.label(available_text(&update.version, &current));
                ui.label(RESTART_WARNING);
                let page = if update.page.is_empty() {
                    RELEASES_PAGE
                } else {
                    &update.page
                };
                ui.hyperlink_to("What's new", page);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_install, egui::Button::new("Update now"))
                        .on_disabled_hover_text(ONLY_INSTALLED)
                        .clicked()
                    {
                        action = Action::UpdateNow(update.clone());
                    }
                    if ui.button("Later").clicked() {
                        action = Action::Later;
                    }
                });
            }
            UpdateState::Downloading { update, done } => {
                let pct = (*done * 100).checked_div(update.size).unwrap_or(0).min(100);
                ui.label(format!("Downloading… {pct}%"));
                ui.add(egui::ProgressBar::new(pct as f32 / 100.0));
                if ui.button("Cancel").clicked() {
                    action = Action::Cancel;
                }
            }
            UpdateState::Ready { .. } => {
                ui.label("Starting the installer…");
            }
            UpdateState::DownloadFailed { update, reason } => {
                ui.label(format!("The update couldn't be downloaded. {reason}"));
                ui.horizontal(|ui| {
                    if ui.button("Try again").clicked() {
                        action = Action::Retry(update.clone());
                    }
                    if ui.button("Download manually").clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab(&update.page));
                    }
                    if ui.button("Close").clicked() {
                        action = Action::Close;
                    }
                });
            }
            _ => {}
        }
    });
    // Esc or clicking outside counts as Later while it's only an offer.
    if response.should_close()
        && matches!(state, UpdateState::Available { .. })
        && matches!(action, Action::None)
    {
        action = Action::Later;
    }

    match action {
        Action::None => {}
        Action::Later => snooze(app),
        Action::UpdateNow(u) | Action::Retry(u) => {
            if let Some(w) = &app.updater {
                w.download(u);
            }
        }
        Action::Cancel => {
            if let Some(w) = &app.updater {
                w.cancel();
            }
        }
        Action::Close => {
            if let Some(w) = &app.updater {
                w.dismiss();
            }
        }
    }
    if let UpdateState::Ready { update, path } = &state
        && app.handoff.is_none()
    {
        start_handoff(app, update, path);
    }
    if matches!(state, UpdateState::Downloading { .. }) {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available(manual: bool) -> UpdateState {
        UpdateState::Available {
            update: Update {
                version: Version::new(0, 3, 0),
                url: String::new(),
                size: 1,
                sha256: String::new(),
                page: String::new(),
            },
            manual,
        }
    }

    #[test]
    fn dialog_waits_for_focus() {
        let s = available(false);
        assert!(!dialog_should_show(&s, true, false, 100, None), "hidden");
        assert!(
            !dialog_should_show(&s, false, true, 100, None),
            "not focused"
        );
        assert!(dialog_should_show(&s, true, true, 100, None));
        assert!(
            !dialog_should_show(&s, true, true, 100, Some(200)),
            "snoozed"
        );
        assert!(!dialog_should_show(
            &UpdateState::UpToDate,
            true,
            true,
            100,
            None
        ));
    }

    #[test]
    fn manual_check_shows_without_focus() {
        assert!(dialog_should_show(
            &available(true),
            false,
            true,
            100,
            Some(200)
        ));
    }

    #[test]
    fn available_copy() {
        assert_eq!(
            available_text(&Version::new(0, 3, 0), &Version::new(0, 2, 0)),
            "vSoundboard 0.3.0 is available (you have 0.2.0)."
        );
    }

    #[test]
    fn status_lines() {
        let v = Version::new(0, 2, 0);
        assert_eq!(
            status_text(&UpdateState::UpToDate, &v, true),
            "You're up to date (vSoundboard 0.2.0)"
        );
        assert_eq!(status_text(&UpdateState::Idle, &v, false), ONLY_INSTALLED);
        assert_eq!(
            status_text(&available(false), &v, true),
            "vSoundboard 0.3.0 is available"
        );
        assert_eq!(
            status_text(&UpdateState::CheckFailed("HTTP 500".into()), &v, true),
            "Couldn't check for updates: HTTP 500"
        );
    }
}

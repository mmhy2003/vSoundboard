//! The update worker thread: periodic and manual checks, and downloads. The UI polls `state()`
//! each frame (like it does the engine); nothing here touches the UI or audio threads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, Sender};
use semver::Version;

use super::{Update, decide, net};

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available { update: Update, manual: bool },
    CheckFailed(String),
    Downloading { update: Update, done: u64 },
    Ready { update: Update, path: PathBuf },
    DownloadFailed { update: Update, reason: String },
}

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Delay before the first automatic check.
    pub first: Duration,
    /// Time between automatic checks.
    pub interval: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(10),
            interval: Duration::from_secs(6 * 3600),
        }
    }
}

enum Cmd {
    Check { manual: bool },
    Download(Update),
    Dismiss,
    Shutdown,
}

pub struct UpdateWorker {
    tx: Sender<Cmd>,
    state: Arc<Mutex<UpdateState>>,
    auto: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
}

/// Where downloaded installers go; emptied on startup.
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("vSoundboard-update")
}

impl UpdateWorker {
    pub fn start(
        api_url: String,
        current: Version,
        auto_check: bool,
        on_change: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self::start_with_timing(api_url, current, auto_check, on_change, Timing::default())
    }

    pub fn start_with_timing(
        api_url: String,
        current: Version,
        auto_check: bool,
        on_change: Arc<dyn Fn() + Send + Sync>,
        timing: Timing,
    ) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let state = Arc::new(Mutex::new(UpdateState::Idle));
        let auto = Arc::new(AtomicBool::new(auto_check));
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicU64::new(0));
        let (st, au, ca, dn) = (state.clone(), auto.clone(), cancel.clone(), done.clone());
        std::thread::Builder::new()
            .name("updater".into())
            .spawn(move || {
                let set = |s: UpdateState| {
                    *st.lock().unwrap() = s;
                    on_change();
                };
                let check = |manual: bool| {
                    if manual {
                        set(UpdateState::Checking);
                    }
                    let result =
                        net::fetch_latest(&api_url).and_then(|body| decide(&body, &current));
                    match result {
                        Ok(Some(update)) => set(UpdateState::Available { update, manual }),
                        Ok(None) => set(UpdateState::UpToDate),
                        Err(e) if manual => set(UpdateState::CheckFailed(e)),
                        Err(_) => set(UpdateState::Idle),
                    }
                };
                let mut next_auto = Instant::now() + timing.first;
                loop {
                    let wait = next_auto.saturating_duration_since(Instant::now());
                    match rx.recv_timeout(wait) {
                        Ok(Cmd::Check { manual }) => check(manual),
                        Ok(Cmd::Download(update)) => {
                            ca.store(false, Ordering::Relaxed);
                            set(UpdateState::Downloading {
                                update: update.clone(),
                                done: 0,
                            });
                            let path = download_dir()
                                .join(format!("vSoundboard-Setup-{}.exe", update.version));
                            match net::download(&update, &path, &dn, &ca) {
                                Ok(()) => set(UpdateState::Ready { update, path }),
                                // Cancelling goes back to the offer, so the friend can pick Later.
                                Err(e) if e == "Cancelled" => set(UpdateState::Available {
                                    update,
                                    manual: true,
                                }),
                                Err(reason) => set(UpdateState::DownloadFailed { update, reason }),
                            }
                        }
                        Ok(Cmd::Dismiss) => set(UpdateState::Idle),
                        Ok(Cmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => {
                            next_auto = Instant::now() + timing.interval;
                            // Don't replace an offer or a download in progress.
                            let busy = matches!(
                                *st.lock().unwrap(),
                                UpdateState::Available { .. }
                                    | UpdateState::Downloading { .. }
                                    | UpdateState::Ready { .. }
                                    | UpdateState::DownloadFailed { .. }
                                    | UpdateState::Checking
                            );
                            if au.load(Ordering::Relaxed) && !busy {
                                check(false);
                            }
                        }
                    }
                }
            })
            .expect("spawn updater thread");
        Self {
            tx,
            state,
            auto,
            cancel,
            done,
        }
    }

    pub fn state(&self) -> UpdateState {
        let mut s = self.state.lock().unwrap().clone();
        if let UpdateState::Downloading { done, .. } = &mut s {
            *done = self.done.load(Ordering::Relaxed);
        }
        s
    }

    pub fn check_now(&self) {
        let _ = self.tx.send(Cmd::Check { manual: true });
    }

    pub fn download(&self, update: Update) {
        let _ = self.tx.send(Cmd::Download(update));
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn set_auto_check(&self, on: bool) {
        self.auto.store(on, Ordering::Relaxed);
    }

    /// Back to Idle (after Later / Close).
    pub fn dismiss(&self) {
        let _ = self.tx.send(Cmd::Dismiss);
    }
}

impl Drop for UpdateWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::net::test_server::{Reply, serve};

    const FIXTURE: &str = include_str!("../../tests/fixtures/release-v0.2.0.json");

    fn wait_for(w: &UpdateWorker, pred: impl Fn(&UpdateState) -> bool, ms: u64) -> UpdateState {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let s = w.state();
            if pred(&s) || Instant::now() > deadline {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn slow_timing() -> Timing {
        Timing {
            first: Duration::from_secs(3600),
            interval: Duration::from_secs(3600),
        }
    }

    #[test]
    fn check_reports_available_from_local_feed() {
        let base = serve(vec![Reply::ok(FIXTURE)]);
        let w = UpdateWorker::start_with_timing(
            format!("{base}/latest"),
            Version::new(0, 1, 0),
            true,
            Arc::new(|| {}),
            slow_timing(),
        );
        w.check_now();
        let s = wait_for(&w, |s| matches!(s, UpdateState::Available { .. }), 2000);
        match s {
            UpdateState::Available { update, manual } => {
                assert!(manual);
                assert_eq!(update.version, Version::new(0, 2, 0));
            }
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn up_to_date_when_not_newer() {
        let base = serve(vec![Reply::ok(FIXTURE)]);
        let w = UpdateWorker::start_with_timing(
            format!("{base}/latest"),
            Version::new(0, 2, 0),
            true,
            Arc::new(|| {}),
            slow_timing(),
        );
        w.check_now();
        assert_eq!(
            wait_for(&w, |s| *s == UpdateState::UpToDate, 2000),
            UpdateState::UpToDate
        );
    }

    #[test]
    fn check_against_failing_server_reports_error() {
        let failing = || Reply {
            status: 500,
            ..Reply::ok("")
        };
        let base = serve(vec![failing(), failing()]);
        let timing = Timing {
            first: Duration::from_millis(50),
            interval: Duration::from_secs(3600),
        };
        let w = UpdateWorker::start_with_timing(
            format!("{base}/latest"),
            Version::new(0, 1, 0),
            true,
            Arc::new(|| {}),
            timing,
        );
        // The automatic check fails silently.
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(w.state(), UpdateState::Idle);
        // A manual one says why.
        w.check_now();
        let s = wait_for(&w, |s| matches!(s, UpdateState::CheckFailed(_)), 2000);
        assert_eq!(s, UpdateState::CheckFailed("HTTP 500".to_string()));
    }

    #[test]
    fn auto_check_off_never_checks() {
        let base = serve(vec![Reply::ok(FIXTURE)]);
        let timing = Timing {
            first: Duration::from_millis(20),
            interval: Duration::from_millis(20),
        };
        let w = UpdateWorker::start_with_timing(
            format!("{base}/latest"),
            Version::new(0, 1, 0),
            false,
            Arc::new(|| {}),
            timing,
        );
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(w.state(), UpdateState::Idle);
    }

    #[test]
    fn worker_download_reaches_ready_and_dismiss_resets() {
        let data: Vec<u8> = (0..20_000).map(|i| (i % 7) as u8).collect();
        let base = serve(vec![Reply::ok(data.clone())]);
        let w = UpdateWorker::start_with_timing(
            "http://127.0.0.1:9/unused".into(),
            Version::new(0, 1, 0),
            false,
            Arc::new(|| {}),
            slow_timing(),
        );
        use sha2::{Digest, Sha256};
        let u = Update {
            version: Version::new(0, 9, 9),
            url: format!("{base}/s"),
            size: data.len() as u64,
            sha256: Sha256::digest(&data)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            page: String::new(),
        };
        w.download(u.clone());
        let s = wait_for(&w, |s| matches!(s, UpdateState::Ready { .. }), 3000);
        match s {
            UpdateState::Ready { update, path } => {
                assert_eq!(update, u);
                assert_eq!(std::fs::read(&path).unwrap(), data);
                let _ = std::fs::remove_file(path);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        w.dismiss();
        assert_eq!(
            wait_for(&w, |s| *s == UpdateState::Idle, 1000),
            UpdateState::Idle
        );
    }
}

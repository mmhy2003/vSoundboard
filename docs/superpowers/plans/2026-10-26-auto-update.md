# Auto-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** vSoundboard finds newer GitHub Releases, asks (only while its window is focused), and on "Update now" downloads, verifies and runs the new installer, which restarts the new version. Failed updates never leave the friend without the app.

**Architecture:**
- **Pure decision logic** (`src/update/mod.rs`): release JSON → update decision, snooze rules, startup notices, installed-copy rule. Unit-tested against the real v0.2.0 API response.
- **Worker thread** (`src/update/worker.rs`): checks and downloads (`src/update/net.rs`) and publishes its state for the UI to poll each frame. This follows the engine pattern; the audio threads are untouched.
- **Handoff** (`src/update/handoff.rs`): starts the installer, waits for its `SetupMutex`, and only then quits the app.
- **Installer:** `SetupMutex` plus a relaunch on `/UPDATE=1`, on success or failure.
- **UI:** a dialog and settings additions.

**Tech Stack:** Rust (existing app), `ureq` (HTTPS, rustls), `semver`, `sha2`, `serde_json` (existing), `winreg`/`windows-sys` (existing), Inno Setup 7.1.

**Spec:** `docs/superpowers/specs/2026-10-26-auto-update-design.md`. Read it first. Branch rules are in `CLAUDE.md`. All work goes on `develop`, and the plan ends there; the owner releases v0.3.0 separately.

## Global Constraints

- **Endpoints and naming:**
  - API: `https://api.github.com/repos/mmhy2003/vSoundboard/releases/latest`. Send `User-Agent: vSoundboard/<version>` and `Accept: application/vnd.github+json`.
  - Asset name: exactly `vSoundboard-Setup-<X.Y.Z>.exe`. The digest must be `sha256:<64 hex>`. If either is missing, there is no update.
- **Timing:** the first automatic check is 10 s after startup, then every 6 h. "Later" snoozes for 24 h (86 400 s).
- **Where files and settings go:**
  - Download folder: `%TEMP%\vSoundboard-update\`. Delete leftovers on startup.
  - Installer arguments: `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE=1`.
  - The setup mutex name is `vSoundboardSetup`. Wait up to 15 s for it.
  - The installed-copy rule uses the HKCU uninstall key `Software\Microsoft\Windows\CurrentVersion\Uninstall\{16385C1B-9E26-44CD-BD5A-3022320628A0}_is1`, value `InstallLocation`.
  - Config, all `#[serde(default)]`:
    - `updates.auto_check: bool = true`;
    - `updates.snoozed_until: Option<u64>`;
    - `updates.attempted_version: Option<String>`;
    - `last_run_version: Option<String>`.
- **Copy (exact):**
  - Dialog:
    - "vSoundboard {new} is available (you have {current})."
    - "Updating restarts vSoundboard, so your voice in Discord drops for a few seconds."
    - Buttons: "Update now", "Later". Link: "What's new".
  - Progress: "Downloading… {n}%", with a "Cancel" button.
  - Failure:
    - "The update couldn't be downloaded. {reason}"
    - "The update couldn't be started. {reason}"
    - Buttons: "Try again", "Download manually", "Close".
  - Notices:
    - "Updated to vSoundboard {v}."
    - "The update to vSoundboard {v} didn't install. You can download it manually from GitHub."
  - Settings:
    - Checkbox: "Automatically check for updates". Button: "Check for updates".
    - Status lines:
      - "Checking…"
      - "You're up to date (vSoundboard {v})"
      - "vSoundboard {v} is available"
      - "Couldn't check for updates: {reason}"
      - "Updates are only available in the installed app"
- **Safety:**
  - Automatic checks never show errors.
  - Nothing is installed without the friend clicking "Update now".
  - The dialog appears only while the window is visible and focused, except straight after a manual check.
  - Network I/O never runs on the UI or audio threads.

## Review Focus

1. **An update is found while the window is hidden or a game has focus:** no dialog until vSoundboard is focused. (Task 4: `dialog_waits_for_focus`.)
2. **Offline, GitHub 5xx, a rate limit or garbage JSON on an automatic check:** silence, then a retry at the next interval. A manual check shows the reason. (Task 2: `check_against_failing_server_reports_error`.)
3. **A truncated or tampered download:** nothing is run, the failure state shows, and the app keeps running. (Task 2: `download_rejects_wrong_hash`, `download_rejects_short_file`.)
4. **The installer is blocked and never really starts:** the app stays open and shows "couldn't be started". (Task 3: `wait_for_setup_mutex_times_out`.)
5. **The PC clock moves backwards, leaving `snoozed_until` far in the future:** the snooze still ends within 24 h. (Task 1: `snooze_far_in_future_is_capped`.)

---

## File Structure

```
src/update/mod.rs        Update, Decision, decide(), should_prompt(), startup_notice(), is_installed_copy(), install_location()
src/update/net.rs        fetch_latest(), download()
src/update/worker.rs     UpdateWorker thread + UpdateState
src/update/handoff.rs    launch_installer(), wait_for_setup_mutex()
src/ui/update_dialog.rs  dialog + startup notices
src/ui/settings.rs       (modify) checkbox, button, status line
src/ui/mod.rs            (modify) worker ownership, focus gating, notices
src/config.rs            (modify) UpdateSettings, last_run_version
src/cli.rs               (modify) --update-feed=<url> (test switch for the rehearsal)
src/main.rs              (modify) start worker, startup notice, temp cleanup
installer/vsoundboard.iss (modify) SetupMutex, /UPDATE=1 relaunch
tests/fixtures/release-v0.2.0.json   real API response (already saved)
README.md                (modify) "Updates" section
```

---

### Task 1: Config fields and pure update logic

**Files:** Create `src/update/mod.rs`. Modify `src/lib.rs` and `src/config.rs`. Run `cargo add semver`.

**Interfaces:**
- Produces:
  ```rust
  // config.rs
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(default)]
  pub struct UpdateSettings { pub auto_check: bool /*true*/, pub snoozed_until: Option<u64>, pub attempted_version: Option<String> }
  // Config gains: pub updates: UpdateSettings, pub last_run_version: Option<String>
  // update/mod.rs
  pub const API_URL: &str; pub const RELEASES_PAGE: &str = "https://github.com/mmhy2003/vSoundboard/releases";
  pub const SNOOZE_SECS: u64 = 86_400;
  #[derive(Clone, Debug, PartialEq)]
  pub struct Update { pub version: semver::Version, pub url: String, pub size: u64, pub sha256: String, pub page: String }
  pub fn decide(release_json: &str, current: &semver::Version) -> Result<Option<Update>, String>;
  pub fn should_prompt(now: u64, snoozed_until: Option<u64>, manual: bool) -> bool;
  #[derive(Debug, PartialEq)] pub enum StartupNotice { None, Updated(String), Failed(String) }
  pub fn startup_notice(running: &semver::Version, attempted: Option<&str>, last_run: Option<&str>) -> StartupNotice;
  pub fn is_installed_copy(exe: &Path, install_location: Option<&Path>) -> bool;   // same folder, case-insensitive
  pub fn install_location() -> Option<PathBuf>;                                     // reads the uninstall key
  ```
- [ ] **Step 1: Write the failing tests.**
  - **`decide`**, using the fixture `include_str!("../../tests/fixtures/release-v0.2.0.json")`:
    - `fixture_is_newer_than_0_1_0`: from 0.1.0 it returns `Some(Update{ version: 0.2.0, size: 5947717, sha256: "aa2953fe…90b2", url ends with "/vSoundboard-Setup-0.2.0.exe", page: "https://github.com/mmhy2003/vSoundboard/releases/tag/v0.2.0" })`.
    - `same_or_older_is_none`: 0.2.0 and 0.3.0 give `None`.
    - `missing_asset_or_digest_is_none`: edit the fixture JSON to rename the asset, or drop or change the `digest` prefix to `md5:`. Each gives `None`.
    - `bad_tag_or_json_is_err_or_none`: tag `"latest"` gives `None`; `"not json"` gives `Err`.
  - **Snooze:**
    - `snooze_rules`: `should_prompt(100, None, false)` is true; `(100, Some(200), false)` is false; `(200, Some(200), false)` is true; `(100, Some(200), true)` is true (manual ignores the snooze).
    - `snooze_far_in_future_is_capped`: `should_prompt(1000, Some(1000 + 3*86_400), false)` is true. Treat a snooze more than 24 h ahead as expired.
  - **Startup notices** (`startup_notices`):
    - running 0.3.0, attempted `Some("0.3.0")` → `Updated("0.3.0")`;
    - running 0.2.0, attempted `Some("0.3.0")` → `Failed("0.3.0")`;
    - running 0.3.0, attempted `None`, last_run `Some("0.2.0")` → `Updated("0.3.0")`;
    - running 0.3.0, last_run `Some("0.3.0")` → `None`;
    - first run (both `None`) → `None`.
  - **Installed copy** (`installed_copy_rule`): `C:\Users\x\AppData\Local\Programs\vSoundboard\vsoundboard.exe` with location `c:\users\X\appdata\local\programs\vsoundboard\` is true; with `C:\Projects\vSoundboard\target\release\vsoundboard.exe` it's false; with `None` it's false.
  - **Config** (`old_config_gets_update_defaults`): a config JSON without `updates` loads with `auto_check == true` and `snoozed_until == None`.
- [ ] **Step 2:** `cargo test --lib update config` fails.
- [ ] **Step 3: Implement.** `decide` parses with `serde_json::Value`. The tag's leading `v` is stripped before semver parsing.
- [ ] **Step 4:** The tests pass, and clippy is clean.
- [ ] **Step 5: Commit** with "feat: update decision logic and settings".

---

### Task 2: Network, verification and the worker

**Files:** Create `src/update/net.rs` and `src/update/worker.rs`. Run `cargo add ureq sha2` (rustls default; no native-tls).

**Interfaces:**
- Consumes: `decide`, `Update`, `API_URL` (Task 1).
- Produces:
  ```rust
  pub fn fetch_latest(api_url: &str) -> Result<String, String>;            // body text; 10 s timeout; non-2xx → Err("HTTP <code>")
  pub fn download(u: &Update, dest: &Path, done: &AtomicU64, cancel: &AtomicBool) -> Result<(), String>;
      // streams to dest, updates `done`; checks size then SHA-256; on any failure deletes dest; "Cancelled" on cancel
  #[derive(Clone, Debug, PartialEq)]
  pub enum UpdateState { Idle, Checking, UpToDate, Available { update: Update, manual: bool }, CheckFailed(String),
                         Downloading { update: Update, done: u64 }, Ready { update: Update, path: PathBuf }, DownloadFailed { update: Update, reason: String } }
  pub struct UpdateWorker { /* thread, tx, Arc<Mutex<UpdateState>>, Arc<AtomicBool> auto */ }
  impl UpdateWorker {
      pub fn start(api_url: String, current: semver::Version, auto_check: bool, on_change: Arc<dyn Fn() + Send + Sync>) -> Self;
      pub fn state(&self) -> UpdateState;
      pub fn check_now(&self);              // manual
      pub fn download(&self, u: Update);
      pub fn cancel(&self);
      pub fn set_auto_check(&self, on: bool);
      pub fn dismiss(&self);                // back to Idle (after Later / Close)
  }
  ```
  **Schedule:** the first automatic check runs 10 s after `start`, then every 6 h while `auto_check` is on. A failed automatic check sets `Idle`; a failed manual check sets `CheckFailed`. Use the constants `FIRST_CHECK = 10 s`, `INTERVAL = 6 h`, and make them overridable in tests through a `start_with_timing(...)` constructor.
- [ ] **Step 1: Write the failing tests.** Use a tiny in-test HTTP server: `std::net::TcpListener` on `127.0.0.1:0`, answering N requests with canned status and body.
  - `check_reports_available_from_local_feed`: serve the fixture JSON with the asset URL rewritten to the local server. `check_now()` leads to `Available{ version 0.2.0, manual: true }` within 2 s (current = 0.1.0).
  - `check_against_failing_server_reports_error`: a 500 response with a manual check leads to `CheckFailed("HTTP 500")`. With an automatic check (first check after 50 ms in test timing), the state ends `Idle` and never `CheckFailed`.
  - `download_verifies_and_reports_progress`: serve 100 000 bytes whose SHA-256 is in the `Update`. The result is `Ok`, the file is equal, and `done == 100_000`.
  - `download_rejects_wrong_hash`: a wrong digest gives `Err` containing "verification", and the file is deleted.
  - `download_rejects_short_file`: the server sends 50 000 bytes but `size` says 100 000. The result is `Err` and the file is deleted.
  - `cancel_stops_download`: a slow server (1 KB per 10 ms) and `cancel` set after 50 ms give `Err("Cancelled")`.
- [ ] **Step 2:** Fail. **Step 3:** Implement. **Step 4:** Pass. Also run three times in a row to catch timing flakiness.
- [ ] **Step 5: Commit** with "feat: update checker and verified downloader".

---

### Task 3: Handoff and installer changes

**Files:** Create `src/update/handoff.rs`. Modify `installer/vsoundboard.iss`.

**Interfaces:**
- Produces:
  ```rust
  pub const SETUP_MUTEX: &str = "vSoundboardSetup";
  pub fn launch_installer(path: &Path) -> Result<(), String>;   // spawn detached with the Global Constraints args
  pub fn wait_for_setup_mutex(name: &str, timeout: Duration) -> bool;  // poll OpenMutexW(SYNCHRONIZE) every 100 ms
  ```
- **Script changes:**
  - `SetupMutex=vSoundboardSetup` in `[Setup]`.
  - `function IsUpdate: Boolean` returns `ExpandConstant('{param:UPDATE|0}') = '1'`.
  - In `DeinitializeSetup`: if `IsUpdate`, `not CheckForMutexes('Local\vSoundboard')`, and `{app}\vsoundboard.exe` exists, launch it with `ShellExecAsOriginalUser('open', …, '', '', SW_SHOWNORMAL, ewNoWait, rc)`. Wrap it in try/except, since `{app}` may not be set if setup aborted very early.
  - This single place covers both success and failure ("never strand"). The "couldn't be closed" abort is excluded naturally, because the app's mutex is still held.
  - There is no separate `[Run]` entry.
- [ ] **Step 1: Write the failing tests.**
  - `wait_for_setup_mutex_finds_held_mutex`: the test creates a mutex named `vSoundboardSetup-test-<pid>` with `CreateMutexW`. The wait returns true within 200 ms.
  - `wait_for_setup_mutex_times_out`: a name nobody holds, with a 300 ms timeout, returns false after at least 300 ms.
- [ ] **Step 2:** Fail. **Step 3:** Implement and edit the script. `make installer` compiles with 0 warnings.
- [ ] **Step 4: Installer checks** (PowerShell, silent; leave the PC clean afterwards):
  1. Install `dist\vSoundboard-Setup-<v>.exe` silently.
  2. Re-run it with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE=1` while the installed app runs `--minimized`.
     - The old process exits.
     - A new `vsoundboard.exe` process is running within 10 s, and it is the installed path.
     - While setup ran, `vSoundboardSetup` existed: poll `wait_for_setup_mutex` from a debug helper, or check the setup log for "SetupMutex".
  3. Re-run **without** `/UPDATE=1` while the app runs. The app is closed and **not** relaunched (unchanged behaviour).
  4. Uninstall.
- [ ] **Step 5: Commit** with "feat: installer handoff for updates".

---

### Task 4: UI — dialog, notices and settings

**Files:**
- Create `src/ui/update_dialog.rs`.
- Modify `src/ui/mod.rs`, `src/ui/settings.rs`, `src/main.rs` and `src/cli.rs`.

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  ```rust
  // cli.rs: Args gains `pub update_feed: Option<String>` from `--update-feed=<url>` (rehearsal only; documented as a test switch)
  // update_dialog.rs
  pub fn dialog_should_show(state: &UpdateState, focused: bool, visible: bool, now: u64, snoozed_until: Option<u64>) -> bool;
  pub fn available_text(new: &semver::Version, current: &semver::Version) -> String;  // exact copy
  pub fn show(app: &mut SoundboardApp, ctx: &egui::Context);
  // SoundboardApp gains: pub updater: Option<UpdateWorker>, pub update_status: String
  ```
- **Behaviour:**
  - **Visibility** (`dialog_should_show`):
    - `Available{manual: true}` shows when visible, even if not focused, because the friend just clicked "Check for updates".
    - `Available{manual: false}` needs `focused && visible && should_prompt(now, snoozed_until, false)`.
    - `Downloading`, `Ready` and `DownloadFailed` always show while visible.
  - **"Later"**, Esc or ✕: set `updates.snoozed_until = now + SNOOZE_SECS`, persist, and call `dismiss()`.
  - **"Update now":** `download(update)`. When the state reaches `Ready`:
    1. set `updates.attempted_version`, then persist;
    2. `launch_installer`;
    3. `wait_for_setup_mutex(SETUP_MUTEX, 15 s)` on a short-lived thread (not the UI thread). The UI shows "Starting the installer…" meanwhile.
    4. If true, quit the same way as tray → Quit (set `quitting`, `ViewportCommand::Close`).
    5. If false, clear `attempted_version`, persist, and show "The update couldn't be started. The installer didn't start in time."
  - **"Download manually"** opens `update.page` (or `RELEASES_PAGE`) with the `open` crate or `ShellExecuteW`.
  - **"Try again"** downloads again.
  - **Startup:**
    1. Delete `%TEMP%\vSoundboard-update\`.
    2. Compute `startup_notice(...)` and put its text in `app.notice`. On `Failed`, also set `snoozed_until = now + SNOOZE_SECS`.
    3. Clear `attempted_version`, set `last_run_version`, and persist.
  - **Starting the worker:** start it only if `is_installed_copy(current_exe, install_location())` **or** `update_feed` is set. Otherwise the settings status reads "Updates are only available in the installed app", and "Check for updates" still runs a manual check (Available shows, "Update now" is disabled with that same text).
  - **Feed URL:** `update_feed` replaces `API_URL`.
  - **Settings:** the checkbox calls `set_auto_check` and persists. The button calls `check_now`. The status line maps `UpdateState` to the exact copy.
- [ ] **Step 1: Write the failing tests.**
  - `dialog_waits_for_focus`: automatic Available, `visible=false` → false; `visible=true, focused=false` → false; `focused && visible` → true; snoozed → false.
  - `manual_check_shows_without_focus`: `Available{manual:true}`, `visible=true, focused=false` → true.
  - `available_copy`: `available_text(0.3.0, 0.2.0) == "vSoundboard 0.3.0 is available (you have 0.2.0)."`
  - `cli_parses_update_feed`: `["x","--update-feed=http://127.0.0.1:8765/latest.json"]` gives `Some("http://127.0.0.1:8765/latest.json")`.
- [ ] **Step 2:** Fail. **Step 3:** Implement. **Step 4:** Pass. Clippy and fmt are clean.
- [ ] **Step 5: Commit** with "feat: update dialog, notices and settings".

---

### Task 5: End-to-end rehearsal, docs, push

**Files:** Modify `README.md` (an "Updates" section: what the dialog does, Later = 24 h, the settings checkbox, and that updates don't touch VB-Cable or the friend's choices).

- [ ] **Step 1: Build the "new" installer.**
  1. Temporarily set `Cargo.toml` to `0.2.99` and run `make installer`.
  2. Copy `dist\vSoundboard-Setup-0.2.99.exe` to a scratch feed folder.
  3. Revert `Cargo.toml` (`git checkout Cargo.toml Cargo.lock`) and run `make installer` again, giving `vSoundboard-Setup-0.2.0.exe` that contains the updater.
- [ ] **Step 2: Build the fake feed.**
  1. Write `latest.json` into the feed folder, based on the fixture:
     - `tag_name` = `v0.2.99`;
     - asset name `vSoundboard-Setup-0.2.99.exe`;
     - `browser_download_url` = `http://127.0.0.1:8765/vSoundboard-Setup-0.2.99.exe`;
     - the real `size` and `"sha256:<hex>"`.
  2. Serve it with `python -m http.server 8765 --bind 127.0.0.1 --directory <feed folder>`, as a background process.
- [ ] **Step 3: Rehearse the happy path.**
  1. Silently install 0.2.0.
  2. Start `vsoundboard.exe --update-feed=http://127.0.0.1:8765/latest.json`.
  3. Focus the window. The dialog shows the exact copy.
  4. Click "Update now". Drive it through UI Automation via AccessKit (find the button by name and Invoke it). If UIA can't reach it, the owner clicks.
  5. **Expected:**
     - a progress bar, then the app exits;
     - the installed exe's ProductVersion becomes 0.2.99, and a new process runs;
     - the notice "Updated to vSoundboard 0.2.99.";
     - no SmartScreen window (also check the downloaded file has no `Zone.Identifier` stream: `Get-Item -Stream *`);
     - `%TEMP%\vSoundboard-update\` is empty after the restart.
- [ ] **Step 4: Rehearse the tamper path.**
  1. Edit `latest.json` to a wrong digest, restart the 0.2.99 app with a 0.3.99 tag pointing at the same file.
  2. Update now → "couldn't be downloaded… verification", nothing installed, and the app still running.
- [ ] **Step 5: Clean up.** Uninstall, stop the server and delete the feed folder.
- [ ] **Step 6:** Run `make check` and commit the README as "docs: auto-update".
- [ ] **Step 7:** Push `develop` and wait for CI to go green. **No tag, no release:** the owner releases v0.3.0.
- [ ] **Step 8: Owner checklist** (in the final message):
  - (a) The dialog doesn't appear while gaming or hidden in the tray, and appears on focusing the window.
  - (b) Later → the dialog is gone for 24 h, also after a restart.
  - (c) The first real update after v0.3.0: install v0.3.0 on your own PC first.

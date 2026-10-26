# Installer & Release Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship vSoundboard as a single `vSoundboard-Setup-X.Y.Z.exe`. The installer optionally downloads and runs VB-Audio's VB-Cable setup and can start the app with Windows. It is built by `make installer` locally and by GitHub Actions.

**Architecture:**
- **App changes.** The app gets:
  - two CLI switches: `--minimized` (start hidden in the tray) and `--check-vbcable` (exit code only);
  - an autostart module that reads and writes the HKCU Run value;
  - an embedded icon and version resource.
- **Installer.** One Inno Setup 7 script reads the version from the built exe's resources, so the version is never typed twice. It reuses the app's own `--check-vbcable` to decide whether to offer VB-Cable.
- **CI.** A reusable GitHub workflow builds the installer. `ci.yml` runs it on pushes to `develop` and on pull requests, uploading the installer as an artifact. `release.yml` runs it on `v*` tags and publishes a draft-then-public GitHub Release.

**Tech Stack:** Rust (existing app), `winreg` crate, `winresource` crate (build.rs), Inno Setup 7.1 (ISPP + Pascal `[Code]`), GNU Make, GitHub Actions (`windows-latest`, `gh` CLI).

**Spec:** `docs/superpowers/specs/2026-10-26-installer-design.md`. Read it first. The app spec is `docs/superpowers/specs/2026-10-26-vsoundboard-design.md`, and the branch rules are in `CLAUDE.md`.

## Global Constraints

- All work happens on `develop`; never commit to `main`.
- Installer per user: `PrivilegesRequired=lowest`, `DefaultDirName={localappdata}\Programs\vSoundboard`, `ArchitecturesAllowed=x64compatible`, `ArchitecturesInstallIn64BitMode=x64compatible`.
- `AppId={{16385C1B-9E26-44CD-BD5A-3022320628A0}` must never change. `AppPublisher=Mohamed H. Youssef`. `AppURL=https://github.com/mmhy2003/vSoundboard`.
- Output: `dist\vSoundboard-Setup-<version>.exe`. `dist/` is git-ignored.
- The version comes from `Cargo.toml` only and reaches the installer through the exe's ProductVersion resource.
- VB-Cable:
  - **URL:** `https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip`.
  - **SHA-256:** `b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb` (computed 2026-10-26).
  - **Setup program:** `VBCABLE_Setup_x64.exe`. The pack's readme says this one covers x64, and the pack's ARM64 driver means it covers ARM64 too.
  - **Run it** elevated and visible, waiting for it to finish.
  - **Copy for the VB-Cable page**, exactly as written: "VB-CABLE is made by VB-Audio (https://vb-cable.com). It is donationware: free to use, and you can support VB-Audio at vb-cable.com."
- Autostart:
  - The Run value is `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value name `vSoundboard`.
  - Its data is `"<exe path>" --minimized`.
- Single-instance mutex name: `Local\vSoundboard` (already exists in `src/single_instance.rs`).
- Unsigned. Nothing is bundled from VB-Audio.

## Review Focus

1. **vSoundboard running, even hidden in the tray, during an upgrade or uninstall:** it is closed, the files are replaced or removed, and no "file in use" error appears. (Task 4: silent upgrade test with the app running.)
2. **The VB-Cable step fails:** the friend is offline, the download hash doesn't match, they decline the admin prompt, or they close VB-Audio's window. vSoundboard still installs, the fallback message explains what to do, and nothing claims VB-Cable is installed. (Task 5: download-failure test via a bad-URL define, plus the owner checklist.)
3. **Upgrade after turning autostart off in the app:** it stays off. (Task 4: silent upgrade test with the Run value absent.)
4. **Install path or exe path with spaces or non-ASCII characters (an Arabic user name):** the Run value is quoted and still launches. (Task 2: `command_quotes_unicode_path_with_spaces` test.)
5. **A tag whose version doesn't match `Cargo.toml`:** the release workflow fails before anything is published. (Task 6: version-check script test.)

---

## File Structure

```
build.rs                         embed icon + version resource (winresource)
assets/vsoundboard.ico           16/32/48/256 icon, generated, committed
examples/make_icon.rs            regenerates assets/vsoundboard.ico
src/cli.rs                       Args { minimized, check_vbcable } + parse
src/autostart.rs                 Run-value read/write
src/icon.rs                      disc_rgba(size) shared by tray, window icon, make_icon
src/tray.rs                      (modify) use icon::disc_rgba
src/main.rs                      (modify) flags, hidden start, window icon
src/ui/settings.rs               (modify) Start-with-Windows checkbox, version label
installer/vsoundboard.iss        the Inno Setup script
scripts/check-tag-version.sh     tag vs Cargo.toml check, used by CI
Makefile                         (modify) installer target
.github/workflows/build.yml      reusable: test, clippy, build, installer, upload artifact
.github/workflows/ci.yml         push develop / PRs → build.yml
.github/workflows/release.yml    tag v* → version check → build.yml → draft release → publish
.gitignore                       (modify) add /dist
README.md                        (modify) install instructions
```

---

### Task 1: CLI switches, hidden start, `--check-vbcable`

**Files:** Create `src/cli.rs`. Modify `src/lib.rs` and `src/main.rs`.

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Default, PartialEq)]
  pub struct Args { pub minimized: bool, pub check_vbcable: bool }
  pub fn parse(args: impl IntoIterator<Item = String>) -> Args   // skips argv[0]; unknown args ignored
  ```
- [ ] **Step 1: Write the failing tests** (`src/cli.rs`):
  - `no_args_is_default`: `parse(["vsoundboard.exe"])` returns `Args::default()`.
  - `flags_are_recognised`: `["x", "--minimized", "--check-vbcable"]` sets both to true.
  - `unknown_args_ignored`: `["x", "--bogus", "--minimized"]` sets only `minimized`.
  - `program_name_never_matches`: `["--minimized"]` (argv[0] only) returns the default.
- [ ] **Step 2:** `cargo test --lib cli` fails.
- [ ] **Step 3: Implement `parse`.** In `main.rs`:
  1. `let args = cli::parse(std::env::args());`
  2. If `args.check_vbcable`, run `std::process::exit(if devices::find_cable(&devices::list_outputs()).is_some() { 0 } else { 1 })`. This happens before the single-instance guard, the config and any window.
  3. If `args.minimized`, use `ViewportBuilder::with_visible(false)`. There must be no window flash. Tray → Show opens it (existing `TrayRequests::show` path).
- [ ] **Step 4:** `cargo test --lib cli` passes.
- [ ] **Step 5: Manual checks.**
  - `cargo build`, then `target\debug\vsoundboard.exe --check-vbcable; echo $?` exits 0 on this PC (VB-Cable is installed), with no window.
  - `target\debug\vsoundboard.exe --minimized` gives a tray icon and no window, and the process stays alive (`tasklist`).
- [ ] **Step 6: Commit** with "feat: --minimized and --check-vbcable switches".

---

### Task 2: Autostart + settings checkbox + version label

**Files:** Create `src/autostart.rs`. Modify `src/lib.rs` and `src/ui/settings.rs`. Run `cargo add winreg`.

**Interfaces:**
- Consumes: nothing new.
- Produces:
  ```rust
  pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
  pub const VALUE_NAME: &str = "vSoundboard";
  pub fn command_for(exe: &Path) -> String;                         // "\"<exe>\" --minimized"
  pub fn is_enabled_in(key: &str, exe: &Path) -> bool;              // value exists AND equals command_for(exe) (case-insensitive)
  pub fn set_enabled_in(key: &str, exe: &Path, on: bool) -> std::io::Result<()>;  // write or delete (deleting a missing value is Ok)
  pub fn is_enabled(exe: &Path) -> bool;                           // = is_enabled_in(RUN_KEY, ..)
  pub fn set_enabled(exe: &Path, on: bool) -> std::io::Result<()>;
  // settings.rs
  pub fn version_label() -> String;                                // "vSoundboard 0.1.0" from CARGO_PKG_VERSION
  ```
- [ ] **Step 1: Write the failing tests.** Use a throwaway key `Software\vSoundboard-test-<pid>-<test name>` under HKCU, deleted at the end of each test.
  - `command_quotes_unicode_path_with_spaces`: `command_for(Path::new(r"C:\Users\محمد علي\AppData\Local\Programs\vSoundboard\vsoundboard.exe"))` equals `"\"C:\\Users\\محمد علي\\AppData\\Local\\Programs\\vSoundboard\\vsoundboard.exe\" --minimized"`.
  - `round_trip_on_off`:
    - `set_enabled_in(k, exe, true)`, then `is_enabled_in(k, exe)` is true;
    - `set_enabled_in(k, exe, false)`, then it is false;
    - `set_enabled_in(k, exe, false)` again is Ok.
  - `other_exe_is_not_enabled`: enabled for `C:\a\vsoundboard.exe` means `is_enabled_in(k, C:\b\vsoundboard.exe)` is false (a stale entry from another install).
  - `version_label_uses_cargo_version`: `version_label() == format!("vSoundboard {}", env!("CARGO_PKG_VERSION"))`.
- [ ] **Step 2:** `cargo test --lib autostart settings` fails.
- [ ] **Step 3: Implement.** In the settings screen, below the shortcuts and above the Discord help:
  - A "Start with Windows" checkbox. Its state is `autostart::is_enabled(&current_exe)`, read once when the screen opens and cached in `app.device_lists`' refresh cycle.
  - Toggling it calls `set_enabled`. On error, show `notice = "Couldn't change Start with Windows: <err>"`.
  - A small weak label with `version_label()`.
- [ ] **Step 4:** Tests pass; `cargo clippy --all-targets -- -D warnings` is clean.
- [ ] **Step 5: Commit** with "feat: Start with Windows toggle and version label".

---

### Task 3: App icon and version resource

**Files:**
- Create `src/icon.rs`, `examples/make_icon.rs`, `assets/vsoundboard.ico` and `build.rs`.
- Modify `src/tray.rs` (use `icon::disc_rgba`) and `src/main.rs` (window icon via `ViewportBuilder::with_icon`).
- Run `cargo add --build winresource`.

**Interfaces:**
- Produces: `pub fn disc_rgba(size: u32) -> Vec<u8>`, which is the existing tray disc drawing, generalised to any size, RGBA8, `size*size*4` bytes.
- [ ] **Step 1: Write the failing tests** (`src/icon.rs`):
  - `disc_has_opaque_centre_and_clear_corners`: for size 32, the centre pixel's alpha is 255, pixel (0,0) has alpha 0, and the length is 4096.
  - `committed_ico_has_expected_sizes`: parse `assets/vsoundboard.ico`'s ICONDIR (6-byte header, then 16-byte entries; width byte 0 means 256). The sizes are exactly `[16, 32, 48, 256]`.
- [ ] **Step 2:** It fails (the module and file are missing).
- [ ] **Step 3: Implement.**
  - `disc_rgba`.
  - `examples/make_icon.rs` writes the ICO with PNG-compressed entries (use the `png` crate as a dev-dependency) for 16/32/48/256. Run it once and commit the `.ico`.
  - `build.rs`: on the `windows` target, `winresource::WindowsResource::new().set_icon("assets/vsoundboard.ico").compile()`. This also writes FileVersion/ProductVersion from `CARGO_PKG_VERSION`.
  - The tray and window use `disc_rgba`.
- [ ] **Step 4:** Tests pass. `cargo build --release`. Then `(Get-Item target\release\vsoundboard.exe).VersionInfo.ProductVersion` prints the `Cargo.toml` version, and Explorer shows the green disc on the exe.
- [ ] **Step 5: Commit** with "feat: app icon and version resource".

---

### Task 4: Inno Setup script (app install, shortcuts, autostart, upgrade, uninstall)

**Files:**
- Create `installer/vsoundboard.iss`.
- Modify `Makefile`, `.gitignore` (`/dist`) and `README.md`.

**Interfaces:**
- Consumes: `target\release\vsoundboard.exe` (with ProductVersion and icon from Task 3), the `--minimized` switch (Task 1), and the Run value format from Task 2's `command_for`.
- **Script decisions:**
  - **Version:** `#define AppVersion GetStringFileInfo(AddBackslash(SourcePath) + "..\target\release\vsoundboard.exe", "ProductVersion")`. Fail the compile with `#error` if it's empty.
  - **[Setup] directives:** `OutputBaseFilename=vSoundboard-Setup-{#AppVersion}`, `OutputDir=..\dist`, `SetupIconFile=..\assets\vsoundboard.ico`, `UninstallDisplayIcon={app}\vsoundboard.exe`, `WizardStyle=modern`, `DisableProgramGroupPage=yes`, `CloseApplications=no` (we close it ourselves). Plus the Global Constraints values.
  - **[Tasks]:**
    - `desktopicon`: "Create a desktop shortcut", `Flags: unchecked`.
    - `autostart`: "Start vSoundboard when Windows starts (in the tray)", checked.
    - **Upgrade rule:** when a previous install exists, set `autostart` to match whether the Run value currently exists, using `WizardSelectTasks` in `CurPageChanged(wpSelectTasks)`, first visit only. This overrides Inno's remembered choice.
  - **[Icons]:** Start Menu `{userprograms}\vSoundboard`, plus the desktop shortcut (task `desktopicon`).
  - **[Registry]:** HKCU Run value `vSoundboard` = `"""{app}\vsoundboard.exe"" --minimized"`, `Tasks: autostart`, `Flags: uninsdeletevalue`. In `CurStepChanged(ssPostInstall)`, if `autostart` is not selected, `RegDeleteValue` it, so unticking on upgrade removes it.
  - **[Run]:** `{app}\vsoundboard.exe`, description "Launch vSoundboard", `Flags: postinstall nowait skipifsilent`.
  - **Closing a running app:**
    - In `PrepareToInstall` and `InitializeUninstall`: if `CheckForMutexes('Local\vSoundboard')`, show `MsgBox('vSoundboard is running and will be closed.', mbInformation, MB_OK)`. Skip the box when `WizardSilent` / `UninstallSilent`.
    - Then run `Exec('taskkill.exe', '/F /IM vsoundboard.exe', '', SW_HIDE, ewWaitUntilTerminated, rc)`.
    - Then wait up to 5 s for the mutex to disappear.
  - Settings in `%APPDATA%\vSoundboard` are never touched.
- [ ] **Step 1: Makefile.** `installer: release` → `iscc installer/vsoundboard.iss`. Add it to `.PHONY` and `help` ("Build dist/vSoundboard-Setup-<version>.exe"). Add `/dist` to `.gitignore`.
- [ ] **Step 2:** `make installer` writes `dist\vSoundboard-Setup-0.1.0.exe`. `iscc` must show no warnings.
- [ ] **Step 3: Silent round-trip test** (pins Review Focus 1 and 3). Use the real per-user location; this PC's owner approved local testing. The test must leave the PC as it was, with no Run value or uninstall entry left over. Run it with PowerShell:
  1. `dist\vSoundboard-Setup-0.1.0.exe /VERYSILENT /SUPPRESSMSGBOXES /TASKS="autostart" /LOG=dist\install1.log`.
     - The exe exists in `%LOCALAPPDATA%\Programs\vSoundboard`.
     - The Run value equals `"<that dir>\vsoundboard.exe" --minimized`.
     - Apps & Features (HKCU Uninstall key) shows vSoundboard 0.1.0.
  2. Start the installed exe with `--minimized` and confirm it's running. Delete the Run value (as the in-app toggle would).
  3. Re-run the installer silently with no `/TASKS`, which is an upgrade.
     - The old process is gone, and the install log has no "file in use"/DeleteFile retry errors.
     - The Run value is still absent.
  4. Run `unins000.exe /VERYSILENT`.
     - The install dir and the Run value are gone.
     - `%APPDATA%\vSoundboard\config.json` still exists if it existed before.
- [ ] **Step 4: README.** Add an "Install" section:
  - download `vSoundboard-Setup-X.Y.Z.exe` from GitHub Releases;
  - SmartScreen: "More info → Run anyway";
  - the VB-Cable option and the restart;
  - the "Start with Windows" option.
  - Add `make installer` under Building.
- [ ] **Step 5: Commit** with "feat: Inno Setup installer".

---

### Task 5: VB-Cable page, download and install

**Files:** Modify `installer/vsoundboard.iss`.

**Interfaces:**
- Consumes: `--check-vbcable` (Task 1), and the URL, SHA-256 and copy in Global Constraints.
- **Script decisions:**
  - `ArchiveExtraction=auto`.
  - `#define VbCableUrl`, `#define VbCableSha256`.
  - `#ifdef VbCableUrlOverride` lets the test build swap in a bad URL.
- **Detection:**
  - In `InitializeWizard`: `ExtractTemporaryFile('vsoundboard.exe')`, then `Exec(ExpandConstant('{tmp}\vsoundboard.exe'), '--check-vbcable', '', SW_HIDE, ewWaitUntilTerminated, rc)`.
  - `VbCableMissing := (rc <> 0)`. If `Exec` itself fails, treat VB-Cable as missing.
- **Page:**
  - `CreateInputOptionPage(wpSelectTasks, 'VB-Cable', 'vSoundboard needs VB-Cable to act as a microphone in Discord', <Global Constraints copy>, False, False)`.
  - One checkbox, ticked: "Download and install VB-Cable now (free, from vb-audio.com; requires administrator permission)".
  - `ShouldSkipPage` returns true when `not VbCableMissing`.
- **Download:**
  - In `NextButtonClick(wpReady)`, if ticked, `DownloadPage.Add(VbCableUrl, 'VBCABLE_Driver_Pack45.zip', VbCableSha256)` (`CreateDownloadPage` as in Inno's `CodeDownloadFiles.iss` example), then `Download`.
  - On any exception (offline, hash mismatch, user abort), set `VbFallback := True`, log it, and return True. The vSoundboard install continues.
- **Extraction:** a `[Files]` entry `Source: "{tmp}\VBCABLE_Driver_Pack45.zip"; DestDir: "{tmp}\vbcable"; Flags: external extractarchive recursesubdirs ignoreversion deleteafterinstall; Check: VbDownloaded`.
- **Running VB-Audio's setup:**
  - In `CurStepChanged(ssPostInstall)`, if downloaded: `ShellExec('runas', ExpandConstant('{tmp}\vbcable\VBCABLE_Setup_x64.exe'), '', ExpandConstant('{tmp}\vbcable'), SW_SHOW, ewWaitUntilTerminated, rc)`.
  - If `ShellExec` returns True, set `VbLaunched := True`. If it returns False (declined UAC or failed), set `VbFallback := True`.
- **Outcome:**
  - `NeedRestart` returns `VbLaunched`.
  - The finished-page text, when `VbLaunched`, is "If you installed VB-Cable, restart your PC before using vSoundboard in Discord." via `UpdateReadyMemo`/`CurPageChanged(wpFinished)` label text. The "Launch vSoundboard" `[Run]` entry gets `Check: not VbLaunched`.
  - When `VbFallback`, a `MsgBox` (suppressible) says: "VB-Cable wasn't installed. You can install it later from https://vb-audio.com/Cable/. vSoundboard's Devices screen explains the setup. Open the download page now?" On Yes, `ShellExec('open', 'https://vb-audio.com/Cable/', …)`. No restart prompt.
- [ ] **Step 1:** `make installer` compiles cleanly.
- [ ] **Step 2: Fallback test** (pins Review Focus 2).
  1. Build a test installer:
     - `iscc /DVbCableUrlOverride=https://download.vb-audio.com/Download_CABLE/does-not-exist.zip /DForceVbCablePage installer\vsoundboard.iss`.
     - `ForceVbCablePage` makes `VbCableMissing := True` regardless.
     - Use `/F` to give it a different output name.
  2. Run it with `/SILENT /LOG=dist\fallback.log` (visible progress, no prompts).
     - The log shows the download failed and the install continued.
     - vSoundboard is installed, and no `runas` was attempted.
  3. Uninstall silently afterwards.
- [ ] **Step 3: Hash test.** Confirm the pinned hash matches the live file: `curl -sL <url> | sha256sum`. If it differs, stop and report. Don't silently update it.
- [ ] **Step 4: Owner checklist.** Append this to the plan's ledger and the final message. These are interactive and can't be automated here:
  - (a) On a PC without VB-Cable, tick the box. Download progress shows, the UAC prompt appears, VB-Audio's window appears, and after install there's a restart prompt with Launch unticked.
  - (b) Decline UAC. The fallback message appears and the page opens on Yes.
  - (c) Close VB-Audio's window without installing. The restart wording is still "If you installed…".
  - (d) Untick the box. No download happens, and the app's Devices screen shows the VB-Cable help.
- [ ] **Step 5: Commit** with "feat: optional VB-Cable download and install".

---

### Task 6: GitHub Actions — CI build, release, and the GitHub test

**Files:**
- Create `.github/workflows/build.yml`, `.github/workflows/ci.yml`, `.github/workflows/release.yml` and `scripts/check-tag-version.sh`.
- Modify `CLAUDE.md` (link the workflows).

**Interfaces:**
- `scripts/check-tag-version.sh <tag>`:
  - exits 0 if `<tag>` minus a leading `v` equals the first `version = "…"` in `Cargo.toml`;
  - otherwise prints `Tag v1.2.3 does not match Cargo.toml version 0.1.0` and exits 1.
- `build.yml` (`on: workflow_call`, output artifact `installer`), on `windows-latest`:
  1. checkout;
  2. `dtolnay/rust-toolchain@stable` with clippy and rustfmt;
  3. `Swatinem/rust-cache@v2`;
  4. `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --lib --bins`;
  5. install Inno Setup **7.1.0** silently from jrsoftware's versioned download (`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-`), pinned to 7.1.0 rather than "latest";
  6. `cargo build --release`;
  7. `iscc installer/vsoundboard.iss`, by full path;
  8. `actions/upload-artifact@v4`, name `installer`, path `dist/vSoundboard-Setup-*.exe`.
- `ci.yml`: `on: push: branches: [develop]` and `pull_request`, then `uses: ./.github/workflows/build.yml`.
- `release.yml`: `on: push: tags: ['v*']`, `permissions: contents: write`. It has three jobs:
  - **check:** runs `bash scripts/check-tag-version.sh "$GITHUB_REF_NAME"`.
  - **build:** needs check, `uses: ./.github/workflows/build.yml`.
  - **publish:** needs build. It downloads the artifact, then:
    1. `gh release create "$TAG" --draft --title "vSoundboard ${TAG#v}" --generate-notes dist/*.exe`
    2. `gh release edit "$TAG" --draft=false`
- [ ] **Step 1: Write the failing test for the version check** (pins Review Focus 5). `scripts/check-tag-version.sh` doesn't exist yet. Run:
  - `bash scripts/check-tag-version.sh v0.1.0`, expecting exit 0;
  - `bash scripts/check-tag-version.sh v9.9.9`, expecting exit 1 and the message.

  Expected now: "No such file".
- [ ] **Step 2:** Implement the script. Both checks behave as expected.
- [ ] **Step 3:** Write the three workflows.
- [ ] **Step 4: Commit** with "ci: build installer on develop and publish releases from tags".
- [ ] **Step 5: GitHub test** (the owner's "test building with github"):
  1. `git push -u origin develop`.
  2. `gh run watch` on the ci run, then `gh run download <id> -n installer -D dist/ci`.
  - Success: the run is green and `dist/ci/vSoundboard-Setup-0.1.0.exe` exists.
  - Install that downloaded installer silently, check that it runs, then uninstall (same as Task 4 Step 3, items 1 and 4).
  - No tag is pushed and no release is created.
- [ ] **Step 6:** In `CLAUDE.md`'s Releasing section, add "The `v*` tag triggers `.github/workflows/release.yml`; pushes to `develop` build a test installer artifact via `ci.yml`." **Commit and push** with "docs: link release workflows".

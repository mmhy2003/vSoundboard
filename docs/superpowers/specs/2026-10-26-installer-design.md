# vSoundboard Installer & Release Pipeline — Design

Date: 2026-10-26
Status: Draft, awaiting approval
Builds on: `docs/superpowers/specs/2026-10-26-vsoundboard-design.md` (the app)

## Intent

The owner wants friends to download vSoundboard from GitHub and get a working setup on a PC that has never seen the project. A normal Windows installer should:

- put the app in place;
- optionally get VB-Cable, the virtual mic the app depends on;
- optionally start the app with Windows, sitting in the tray.

The owner wants to build the same installer two ways: on their own PC, and automatically on GitHub when they release. Releases follow the branch rules in `CLAUDE.md`.

Success: a friend downloads `vSoundboard-Setup-X.Y.Z.exe` from a GitHub Release, clicks through the wizard, ticks "install VB-Cable", restarts, and has a working soundboard in Discord.

## Goals

- One `vSoundboard-Setup-X.Y.Z.exe` built with **Inno Setup 7** (7.1.0 is installed on the owner's PC; `iscc` is on PATH).
- Wizard options: desktop shortcut, start with Windows, and download-and-install VB-Cable when it's missing.
- VB-Cable is **downloaded from VB-Audio's site at install time** (owner's decision), never shipped inside our installer.
- Upgrades install in place. Uninstall is clean.
- The app gains "start hidden in the tray" for autostart, and a "Start with Windows" toggle.
- `make installer` builds the installer locally. Pushing a `vX.Y.Z` tag builds it on GitHub Actions and publishes a GitHub Release with the installer attached.

## Non-goals

- Code signing. Friends will see Windows SmartScreen's "Windows protected your PC" and must click "More info → Run anyway". The owner accepted this.
- Bundling VB-Cable files in our installer, or installing VB-Cable A+B / C+D (those can't be redistributed).
- Auto-update inside the app.
- MSI packages, winget/Chocolatey/Scoop listings, portable zip.
- Installing for all users / into Program Files (see Constraints).
- Uninstalling VB-Cable when vSoundboard is uninstalled.

## Anti-goals

- A friend ends up with vSoundboard installed but no clue why Discord can't hear sounds. The VB-Cable situation must always be explained.
- The installer silently installs a driver. VB-Cable must be the friend's visible, informed choice, credited to VB-Audio.
- Uninstall or upgrade deletes the friend's pads, shortcuts and settings.
- A published release whose installer version doesn't match the tag.

## VB-Cable licensing (researched 2026-10-26)

VB-Audio's licensing page (https://vb-audio.com/Services/licensing.htm) allows distributing the standard VB-CABLE package with another application, including silent installation, under the donationware model, on three conditions:

- the end user can identify VB-CABLE as a VB-Audio application;
- the end user is able to donate;
- the distributor credits www.vb-cable.com and states that VB-CABLE is donationware.

Downloading from VB-Audio at install time is the more conservative form of this. The VB-Cable page in our installer must therefore:

- name VB-Audio as the maker;
- link https://vb-cable.com;
- say "VB-CABLE is donationware: free to use, and you can support VB-Audio at vb-cable.com."

## Branches, versions and releases (owner's rules)

These rules are recorded in `CLAUDE.md`:

- All work happens on `develop`. `main` only receives merges from `develop`.
- Every merge into `main` is a release:
  1. bump `version` in `Cargo.toml` on `develop` (commit `release: vX.Y.Z`);
  2. merge into `main`;
  3. push an annotated tag `vX.Y.Z` on the merge commit;
  4. the tag triggers the GitHub release workflow.
- *(Claude's call, accepted with "ok")* Claude proposes the bump from what changed since the last release, and the owner can override:
  - while on 0.x: new features → minor (`0.2.0`), fixes only → patch (`0.1.1`);
  - after 1.0: semver.
- The repo `mmhy2003/vSoundboard` is **public** (owner made it public on 2026-10-26), so friends can download Releases directly and the default `GITHUB_TOKEN` can publish them.
- **Scope of this work (owner's decision: "test building with github"):** the plan ends with:
  - the pipeline committed on `develop`;
  - `develop` pushed to GitHub;
  - a successful **build-only** run of the release workflow on GitHub, triggered by hand, whose installer is downloadable as a workflow artifact.

  Cutting the first real release (version bump, merge to `main`, tag) is a separate step the owner will ask for.
- *(Claude's call)* The first release made with this pipeline is **v0.2.0**. It is a minor bump from the current `0.1.0`, because it adds the installer and autostart features. `0.1.0` was never tagged.

## Installer behaviour

### Install scope
- *(Claude's call)* **Per-user install, no admin prompt for our part**:
  - `PrivilegesRequired=lowest`;
  - default folder `{localappdata}\Programs\vSoundboard`;
  - Start Menu shortcut under the user's Programs.
- Only VB-Audio's setup asks for admin, and only if the friend opts into it.
- The user may change the folder on the "Select Destination" page.
- A fixed `AppId` GUID, so a newer installer upgrades an existing install in place. The GUID is generated once and never changed.
- Publisher shown in Apps & Features: *(Claude's call)* "Mohamed H. Youssef". App URL: the GitHub repo, https://github.com/mmhy2003/vSoundboard.

### Wizard pages, in order
1. **Welcome.**
2. **Destination folder** (default as above).
3. **Additional tasks** (checkboxes):
   - "Create a desktop shortcut". *(Claude's call)* Unticked by default.
   - "Start vSoundboard when Windows starts (in the tray)". *(Claude's call)* Ticked by default.
4. **VB-Cable**, a custom page shown only when VB-Cable isn't detected:
   - **Explains:** vSoundboard needs VB-Cable to act as a microphone in Discord.
   - **Credit:** the VB-Audio credit and donationware text from the licensing section above.
   - **Checkbox, ticked by default:** "Download and install VB-Cable now (free, from vb-audio.com; requires administrator permission)".
5. **Ready / installing.**
   - If the friend ticked VB-Cable, a download step with a progress bar. *(Claude's call)* It uses Inno Setup's built-in download support, so no plugins are needed. The builder should verify the 7.1 API.
6. **Finish**, with "Launch vSoundboard" ticked by default.
   - If VB-Cable was installed in this run, Finish asks to restart Windows, with "Yes, restart now" / "No, I'll restart later" (Inno's standard restart prompt). VB-Cable typically appears only after a reboot.
   - In that case, "Launch vSoundboard" is not offered, or is unticked. The friend should restart first.

### VB-Cable detection
- **What counts as "detected":** the same rule the app uses, `find_cable` in `src/audio/devices.rs`: some playback device's name contains "cable input" (case-insensitive). The installer must not disagree with the app.
- *(Claude's call, not prescribing)* The simplest reliable way is to reuse the app's own logic: the newly installed `vsoundboard.exe` gets a hidden `--check-vbcable` switch.
  - It exits with 0 if VB-Cable is present, 1 if not, and opens no window.
  - The installer runs it after the files are copied.
  - If the builder finds a registry check that matches `find_cable` reliably, that's acceptable too.

### VB-Cable download and install
- **Download:**
  - From VB-Audio's official URL. Currently https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip (1.29 MB, Oct 2024). Keep it as one constant in the script.
  - *(Claude's call)* Verify the download's SHA-256 against a pinned value. If the hash doesn't match (VB-Audio replaced the file) or the download fails, use the fallback below. This ensures we never run an unexpected binary.
- **Install:**
  - Extract the zip to a temp folder.
  - Run VB-Audio's setup elevated: `VBCABLE_Setup_x64.exe` on x64, the ARM64 setup on ARM64 (32-bit Windows is refused, see Constraints).
  - Wait for it to finish.
  - *(Claude's call)* Run it **visibly, not silently**, so the friend sees VB-Audio's own window and clicks its "Install Driver" button. This is the clearest way to satisfy "the user can identify VB-CABLE as a VB-Audio application".
- **Outcome after VB-Audio's setup closes:** its window can be closed without clicking "Install Driver", and the device appears only after a reboot, so the installer can't know whether the driver went in. *(Claude's call)* When VB-Audio's setup was launched (elevation accepted) and has exited:
  - the Finish page says: "If you installed VB-Cable, restart your PC before using vSoundboard in Discord."
  - It offers Inno's Restart now / later choice.
  - "Launch vSoundboard" is unticked.
  - The installer never claims VB-Cable is installed.
- **Fallback:** if the download, hash check, extraction or launching setup fails, or the elevation prompt is declined:
  - the installer finishes installing vSoundboard anyway;
  - it shows a message: "VB-Cable wasn't installed. You can install it later from https://vb-audio.com/Cable/. vSoundboard's Devices screen explains the setup.";
  - it offers to open that page.
  - No restart prompt is shown in this case.

### Running app, upgrade, uninstall
- Before installing or uninstalling, any running vSoundboard must be closed reliably, including when it's hidden in the tray, where closing the window only hides it.
  - Config is saved on every change, so ending the process is safe.
  - *(Claude's call)* Detect it via the app's existing single-instance mutex (`Local\vSoundboard`, `src/single_instance.rs`). Tell the user "vSoundboard is running and will be closed", then end it.
- **Upgrade** keeps the install folder and the user's config.
  - *(Claude's call)* On upgrade, the "Start with Windows" task starts ticked **only if the autostart Run value currently exists**. That way an upgrade never undoes what the friend set with the in-app toggle.
  - Other task choices (desktop shortcut) follow Inno's default of remembering the previous install.
- **Uninstall:**
  - removes the program files, the Start Menu and desktop shortcuts, and the autostart entry;
  - **keeps** `%APPDATA%\vSoundboard\` (pads, shortcuts, device choices);
  - leaves VB-Cable installed.
  - *(Claude's call)* The uninstaller doesn't ask about deleting settings.

## App changes

1. **Start hidden.** `vsoundboard.exe --minimized` starts with the window hidden and the tray icon present. Global shortcuts and the engine run normally. Tray → Show opens the window. Without the flag, behaviour is unchanged.
2. **Autostart entry.** One shared definition:
   - *(Claude's call)* a value named `vSoundboard` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`;
   - its data is `"<install dir>\vsoundboard.exe" --minimized`.
   - The installer writes it when the task is ticked, and removes it when unticked on upgrade and on uninstall.
3. **"Start with Windows" checkbox** on the Devices (settings) screen. It reflects whether the Run value exists and points at the running exe, and writes or removes the same value. This lets friends change their mind without reinstalling.
4. **`--check-vbcable`** as described above, if the builder takes that route.
5. **Version shown in the app.** *(Claude's call)* The settings screen shows "vSoundboard X.Y.Z" from `CARGO_PKG_VERSION`, so friends can tell you which version they run.
6. **Icon.** *(Claude's call)*
   - There's currently no `.ico`, so the exe, shortcuts and installer would all show the generic icon.
   - Add one `assets/vsoundboard.ico` (multi-size: 16/32/48/256), matching the green-disc tray icon.
   - Embed it in the exe as a Windows resource, and use it as the installer's `SetupIconFile`.

## Building

### Locally
- GNU Make 4.4.1 is installed on the owner's PC (via winget), alongside the existing Makefile. `make installer`:
  1. runs `cargo build --release`;
  2. reads the version from `Cargo.toml`;
  3. runs `iscc` with that version passed in (for example `/DAppVersion=0.2.0`);
  4. writes `dist\vSoundboard-Setup-<version>.exe`.
- `dist/` is git-ignored. The version is never hand-typed in the `.iss` file.
- The `.iss` script lives in the repo at `installer/vsoundboard.iss` *(Claude's call)*.

### On GitHub
- `.github/workflows/release.yml` runs on a pushed tag `v*`, and by hand (`workflow_dispatch`) in **build-only** mode. Build-only mode:
  - runs the same build;
  - uploads the installer as a workflow artifact;
  - creates no release.

  On a Windows runner, the workflow does the following:
  1. Fail if the tag (minus `v`) ≠ the `Cargo.toml` version.
  2. Run `cargo test --lib --bins` and `cargo clippy --all-targets -- -D warnings`. Fail the release on errors.
  3. `cargo build --release`.
  4. Install **Inno Setup 7** on the runner, matching the owner's local version, so the same script compiles identically. Don't rely on whatever version the runner image ships.
  5. Build the installer the same way as `make installer`.
  6. Tag runs only. Publish safely: create the release as a **draft** first, attach the installer, then publish it. If any step fails, at worst a hidden draft remains, never a broken public release. Create a GitHub Release for the tag:
     - title `vSoundboard X.Y.Z`;
     - attach `vSoundboard-Setup-X.Y.Z.exe`;
     - *(Claude's call)* GitHub's auto-generated release notes.
  - Not a pre-release.
- *(Claude's call)* Also run the tests and clippy on pushes to `develop` and on pull requests (`ci.yml`), so a broken `develop` is noticed before release day. This is cheap and keeps "every merge to main is a release" safe.

## Constraints

- Windows 10/11, x64 primary. *(Claude's call)* The installer refuses 32-bit Windows (`ArchitecturesAllowed=x64compatible`), since the app is x64 only. On ARM64, the x64 app runs under emulation. ARM64 only matters for which VB-Audio setup to launch; our app is built for x64 only (non-goal: an ARM64 build).
- No admin rights for our own install. VB-Cable's driver install is the only elevated step.
- The installer is unsigned.
- The VB-Cable credit and donationware text above must appear wherever we offer VB-Cable.

## Left to the builder

- Exact Inno Setup 7 Pascal-script APIs for download, archive extraction, elevated execution, and closing the running app.
- The SHA-256 of the current VB-Cable pack. Compute it from the official download when implementing.
- How to embed the icon resource (e.g. the `winresource`/`embed-resource` crate) and how to draw the `.ico`.
- How `--minimized` hides the window at startup in eframe 0.37 (e.g. `ViewportBuilder::with_visible(false)`). It must not flash a window.
- CI details: the Rust toolchain setup action, caching, and how Inno Setup 7 is installed on the runner.
- Tests. Pure logic gets unit tests: version parsing, Run-value read/write against a test key, CLI flag parsing. The installer itself gets a manual checklist (fresh install with and without VB-Cable, upgrade, uninstall), recorded in the plan.

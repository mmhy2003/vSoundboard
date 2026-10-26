# vSoundboard Auto-Update: Design

Date: 2026-10-26
Status: Draft, awaiting approval
Builds on:
- `docs/superpowers/specs/2026-10-26-installer-design.md` (installer and release pipeline);
- `docs/superpowers/specs/2026-10-26-vsoundboard-design.md` (the app).

## Intent

Friends installed vSoundboard from a GitHub Release (v0.2.0 is the first). The owner wants them to get new versions without going back to GitHub by hand. The app notices a new release, asks, and on "Update now" it downloads and installs the new version, then starts it again.

The guiding constraint is that friends use vSoundboard **during Discord calls and games**. The mic passes through the app, so restarting it briefly cuts the friend's voice. An update must therefore never happen, or pop up, unless the friend is looking at vSoundboard and chooses it.

## Goals

- Detect a newer GitHub Release automatically: on startup and every 6 hours while running.
- Tell the friend with a dialog offering **Update now** / **Later**.
- **Update now:**
  1. download the new installer with a progress bar;
  2. verify it;
  3. run it silently;
  4. the installer closes every running vSoundboard, installs the new version, and starts it again.
- **Later:** don't ask again for 24 hours, even across restarts.
- A manual "Check for updates" button and an "Automatically check for updates" setting.

## Non-goals

- Silent/automatic installation without asking, or background updates while the app is hidden.
- Delta or partial updates, rollback to older versions, update channels (beta/stable), "skip this version".
- Code signing (still unsigned; see the installer design).
- Updating VB-Cable.
- Any server other than GitHub Releases for `mmhy2003/vSoundboard`.

## Anti-goals

- **Interrupting a friend:** the dialog appears while the app is hidden in the tray or another window (a game) has focus, or the app restarts mid-call without an explicit click.
- **Running an unverified file:** the app runs a downloaded file that wasn't verified against what GitHub says it published.
- **Ending up with nothing:** a failed update leaves the friend with no running vSoundboard and no idea why.
- **Bothering developers:** dev builds (run from `cargo run` / `target\`) try to update themselves.
- **Noise:** the friend is nagged by error dialogs when offline or when GitHub is unreachable.

## Behaviour

### Checking (decided with the owner; details marked)
- **When:**
  - *(Claude's call)* about 10 s after startup, so launch isn't slowed;
  - then every 6 hours while running;
  - and when the friend clicks **Check for updates**.
  - Automatic checks run only when "Automatically check for updates" is on *(Claude's call: on by default)*.
- **Source:** `GET https://api.github.com/repos/mmhy2003/vSoundboard/releases/latest`.
  - Unauthenticated; the rate limit (60/hour/IP) is far above our use. Send a `User-Agent`.
  - "latest" already excludes drafts and pre-releases.
- **What counts as newer:** the tag `vX.Y.Z` parsed as semver is greater than the running version (`CARGO_PKG_VERSION`). Unparseable tags are ignored.
- **The file to install:** the release asset named exactly `vSoundboard-Setup-X.Y.Z.exe`, matching the tag. From the API we use:
  - `browser_download_url`;
  - `size`;
  - `digest`. GitHub provides `"digest": "sha256:<hex>"`; this was verified on v0.2.0, where it matches the file's SHA-256.
  - A release without this asset, or without a `sha256:` digest, is treated as "no update available".
- **Failures** (offline, HTTP error, bad JSON) are silent for automatic checks; they retry at the next scheduled check. A manual check shows the error in the settings status line.
- **Dev builds don't auto-update.** *(Claude's call)* Automatic checks and "Update now" only run when the running exe is the installed copy, meaning its folder equals the `InstallLocation` of the installer's HKCU uninstall entry (AppId `{16385C1B-9E26-44CD-BD5A-3022320628A0}`).
  - Otherwise the settings line says "Updates are only available in the installed app".
  - The manual check still works and can report that an update exists.

### The dialog (owner's rules)
- **Shown only when the vSoundboard window is visible and focused.**
  - If an update is found while the app is hidden or not focused, nothing pops up. The dialog appears the next time the window is focused.
  - No tray balloon or notification *(Claude's call, consistent with "don't interrupt")*.
- **Content:**
  - "vSoundboard X.Y.Z is available (you have A.B.C)."
  - "Updating restarts vSoundboard, so your voice in Discord drops for a few seconds."
  - "What's new" link to the release's `html_url`.
  - Buttons: **Update now**, **Later**.
- **Later:**
  - sets "don't ask before now + 24 h", saved in the config so it survives restarts;
  - after that, the next check that still finds an update shows the dialog at the next focused moment.
  - *(Claude's call)* If a **newer** version than the snoozed one appears, the snooze still applies until it expires. This keeps it simple.
  - *(Claude's call)* Closing the dialog with Esc or ✕ counts as Later.
- **A manual "Check for updates"** that finds an update shows the dialog immediately, ignoring the snooze, because the friend asked.

### Updating
1. **Update now:** the dialog switches to "Downloading… N%" with a Cancel button.
2. **Download:** to `%TEMP%\vSoundboard-update\vSoundboard-Setup-X.Y.Z.exe`.
   - Any older file there is replaced.
   - *(Claude's call)* Downloading happens only after the click, not ahead of time.
3. **Verify:** the file size equals `size` and its SHA-256 equals `digest`. On mismatch, delete the file and show the failure state.
4. **Hand off:**
   1. Save the config, including `updates.attempted_version = "X.Y.Z"`.
   2. Start the installer detached: `vSoundboard-Setup-X.Y.Z.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE=1`.
   3. *(Claude's call: start-up handshake.)* Wait until the installer is really running before quitting.
      - The installer declares a setup mutex, `SetupMutex=vSoundboardSetup`, which Inno holds for its whole run.
      - The app waits up to about 15 s for that mutex to exist.
      - If it never appears (antivirus blocked the file, Inno failed to unpack), the app does **not** quit. It clears `attempted_version` and shows the failure state "The update couldn't be started".
   4. Once the mutex is seen, exit the app normally (the same path as tray → Quit), so the engine shuts down cleanly.
5. **The installer** (changes to `installer/vsoundboard.iss`):
   - **Closing the app:** as today, it closes any still-running vSoundboard (mutex check, taskkill, wait). Because the app quits itself first, this is normally instant.
   - **Upgrading:** as today, in silent upgrade mode:
     - keeps the autostart state;
     - keeps the previous desktop-shortcut choice;
     - never offers or installs VB-Cable (no `/VBCABLE=1`).
   - **New: `SetupMutex=vSoundboardSetup`**, the handshake the app waits for.
   - **New: relaunch on `/UPDATE=1`.** When the install succeeds, the installer starts the new version normally (visible, not `--minimized`), because the friend was looking at the window when they clicked.
   - **New: never strand the friend.** If an `/UPDATE=1` install fails or is aborted *(Claude's call)*, the installer still starts whatever vSoundboard is installed. Inno rolls back to the previous version on failure. "Couldn't close the app" is the exception: it implies a copy is still running.
6. **After the restart:**
   - *(Claude's call)* The app shows a one-time notice line: "Updated to vSoundboard X.Y.Z". It detects this by remembering the last version that ran in the config.
   - *(Claude's call: failed-update detection.)* If `updates.attempted_version` is set and the running version is older than it, the update didn't land. The installer failed and its fallback restarted the old version. The app then:
     - shows a notice: "The update to vSoundboard X.Y.Z didn't install. You can download it manually from GitHub.", with a link to the release page;
     - snoozes the update dialog for 24 hours, so a repeating failure can't loop "Update now → voice drops → asked again";
     - clears `attempted_version`.

     If the running version is equal or newer, it simply clears `attempted_version`.
   - It deletes the downloaded installer from `%TEMP%\vSoundboard-update\` on startup.

### Failure states
In the dialog:
- **Download failed** or **verification failed:** "The update couldn't be downloaded. <reason>". Buttons: **Try again**, **Download manually** (opens the release page), **Close**. The app keeps running.
- **The installer couldn't be started** (`CreateProcess` failure): same treatment. The app does not exit.

### Settings screen additions
- A checkbox: "Automatically check for updates", saved in config, default on.
- A **Check for updates** button.
- A status line, one of:
  - "Checking…"
  - "You're up to date (vSoundboard A.B.C)"
  - "vSoundboard X.Y.Z is available" (the dialog opens)
  - "Couldn't check for updates: <reason>"
  - "Updates are only available in the installed app"
- The existing version label stays.

### Config additions
Use `#[serde(default)]` fields, so old configs load:
- `updates.auto_check: bool`, default true;
- `updates.snoozed_until: Option<u64>` (Unix seconds);
- `last_run_version: Option<String>`;
- `updates.attempted_version: Option<String>` (set just before handing off to the installer).

## Constraints

- Windows 10/11 x64, per-user install (no UAC for our installer; `PrivilegesRequired=lowest`).
- **No SmartScreen during updates:**
  - Files the app downloads itself don't get a Mark-of-the-Web, so running the unsigned installer this way doesn't trigger SmartScreen.
  - The builder must not add a Zone.Identifier stream.
  - Verify on the owner's PC that an update run shows no SmartScreen prompt.
- **Threads:** network I/O runs off the UI and audio threads. The UI polls state each frame; the audio engine is never blocked.
- **Updating twice in a row:** the update must work when the friend updates again later from the version it installed. The relaunched app is a normal installed copy.
- **Repo slug and AppId:** use the repo slug `mmhy2003/vSoundboard` and the AppId above as constants.

## Left to the builder

- **HTTP/TLS client crate.** For example `ureq` with rustls: it's small and blocking, which is fine on a worker thread. Also the JSON parsing (`serde_json` already exists).
- How the update worker thread is structured, and how the UI observes progress (atomics or channel). Keep the existing pattern of the UI polling each frame.
- How `/UPDATE=1` relaunch and the "launch even on failure" rule are expressed in Inno Setup 7 (e.g. a `[Run]` entry with a `Check:`, and a `DeinitializeSetup` fallback).
- **Testing:**
  - Unit-test the pure parts: version comparison, release-JSON → update decision (fixture JSON from the real v0.2.0 response), asset/digest validation, snooze timing, and the dev-build/installed-copy rule.
  - For an end-to-end check, the builder may add a test-only override of the releases URL. It's honoured only in debug builds or behind a clearly test-only switch, never settable by a release config.
  - The real round trip (0.2.0 → a newer release) is verified once the next release exists. The plan should say how, for example by installing 0.2.x and then publishing the next version.

## Testing and releases (owner's decision)

Friends on v0.2.0 have no updater, so the first version with this feature has to be installed by hand. The first real automatic update is from that version to the one after it.

- **Before any release, the plan rehearses the whole update on the owner's PC:**
  1. install the new build;
  2. point the test-only releases-URL override at a fake "newer release" (a local copy of the API response naming a CI-built or locally built installer with a higher version and its real size and SHA-256);
  3. click Update now;
  4. check the download, verification, handshake, silent install, restart and "Updated to…" notice;
  5. check that no SmartScreen prompt appears.

  The rehearsal also covers:
  - a forced verification failure, where nothing installs and the old version keeps running;
  - the failed-install path, with the "didn't install" notice.
- **The plan stops on `develop`**, as with the installer work. The owner decides when to release this as **v0.3.0** (a minor bump: a new feature).
- **The release after v0.3.0** is the first real automatic update. The owner installs v0.3.0 on their own PC beforehand, so they see that update at the same time as friends.

## Release-pipeline impact

None required: releases already publish `vSoundboard-Setup-X.Y.Z.exe`, and GitHub supplies the digest. The release workflow must keep that exact asset name.

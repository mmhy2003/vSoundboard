# vSoundboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Windows soundboard app that mixes the owner's mic with sound files into VB-Cable (Discord's mic) and plays the sounds in their headphones, triggered by global shortcuts from a pad-grid window.

**Architecture:** One Rust binary. An **engine thread** owns all cpal streams (cpal `Stream` is `!Send`) and handles commands. A **player thread** decodes files (symphonia), resamples them to each output's rate (rubato), and pushes frames into one lock-free ring per output. The **audio callbacks** pull from those rings, mix in the mic, apply gains read from atomics, and never lock or block. The **egui UI** (eframe) owns the `Config` and talks to the engine through a channel. Global hotkeys and the tray go straight to the engine through OS event handlers, so they work while the window is hidden.

**Tech Stack:** Rust stable (MSVC toolchain), eframe/egui, cpal (WASAPI), symphonia (+`mp3` feature), rubato, ringbuf, crossbeam-channel, serde/serde_json, global-hotkey, tray-icon, rfd. Dev: hound, tempfile.

**Spec:** `docs/superpowers/specs/2026-10-26-vsoundboard-design.md`. Read it before starting any task.

## Global Constraints

- Windows 10/11 only. The app never requests elevation.
- Formats: MP3, WAV, OGG (Vorbis), FLAC.
- Only one sound plays at a time. A new trigger cuts off the current sound. Re-triggering the playing pad restarts it.
- Mic passthrough adds under ~30 ms (mic capture → our mix → write to CABLE Input). VB-Cable and Discord are outside that budget.
- Audio callbacks: no locks, no blocking, no allocation in the steady state.
- Config: `%APPDATA%\vSoundboard\config.json`. Saved on every change, written atomically (temp file + rename).
- A corrupt config is moved to `config.json.bad-<unix-seconds>` and the app starts empty. It is never silently overwritten.
- Hotkeys use `RegisterHotKey` via the `global-hotkey` crate. A fired shortcut is consumed. A low-level keyboard hook is out of scope.
- Files are referenced in place, never copied.
- VB-Cable is linked to (https://vb-audio.com/Cable/), never bundled.
- Use `cargo add <crate>` for the latest versions and commit `Cargo.lock`. Where this plan names an API, check it against the docs for the version you actually got.
- `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` in `main.rs`.

## Review Focus

1. **Hours-long sessions:** mic and cable clocks drift. Delay must not grow and there must be no periodic dropouts. Test in Task 4.
2. **Mashing shortcuts:** many triggers in quick succession. You hear only the latest sound, with no tail of the earlier one and no stale "Finished" events. Test in Task 6.
3. **Odd or broken files:** mono, 22.05 kHz, 96 kHz/24-bit, zero-length, or garbage renamed to `.mp3`. The good ones play correctly. The broken ones show an error on the pad and never crash or hang. Tests in Tasks 5 and 6.
4. **Devices that are missing at startup, unplugged mid-session, or come back:** the status is shown and the app recovers without a restart. Test in Task 7.
5. **Config from the real world:** non-ASCII paths (Arabic, emoji), unknown or missing fields, out-of-range volumes, and corrupt JSON all load sensibly. Test in Task 1.

---

## File Structure

```
Cargo.toml
src/main.rs              entry: load config, start engine, run eframe
src/config.rs            Config types, load/save, pad CRUD
src/hotkeys.rs           actions, parse/format/labels, conflicts, egui→Code, OS registry
src/audio/mod.rs         re-exports
src/audio/shared.rs      AtomicF32, SharedParams
src/audio/mix.rs         pure mixing/channel mapping
src/audio/drift.rs       DriftResampler (mic → cable, clock drift)
src/audio/decode.rs      SoundReader (symphonia), RateConverter (rubato)
src/audio/player.rs      player thread, SoundFeed, flush handshake
src/audio/devices.rs     enumerate devices, open streams, reconnect planning
src/audio/engine.rs      engine thread, EngineHandle, EngineCommand, EngineEvent
src/ui/mod.rs            SoundboardApp (eframe::App), Screen
src/ui/board.rs          pad grid, add/drag-drop, pad context menu
src/ui/bottom_bar.rs     now playing, Stop, sliders, mode switch
src/ui/settings.rs       devices, shortcuts for Stop/mode, help text, latency readout
src/ui/capture.rs        shortcut capture modal
src/tray.rs              tray icon + menu
tests/                   integration tests where noted; most are #[cfg(test)] unit tests
```

---

### Task 1: Toolchain, scaffold, and config

**Files:** Create `Cargo.toml`, `src/main.rs`, `src/config.rs`. Modify `.gitignore` (add `/target`).

**Interfaces:**
- Produces:
  ```rust
  pub struct Config { pub version: u32 /*=1*/, pub pads: Vec<Pad>, pub stop_hotkey: Option<String>,
      pub mode_hotkey: Option<String>, pub devices: DeviceNames, pub volumes: Volumes, pub mode: RoutingMode,
      pub next_pad_id: u64 }
  pub struct Pad { pub id: u64, pub name: String, pub path: PathBuf, pub hotkey: Option<String>, pub volume: f32 }
  pub struct DeviceNames { pub mic: Option<String>, pub cable: Option<String>, pub monitor: Option<String> }
  pub struct Volumes { pub sounds: f32, pub mic: f32, pub monitor: f32 }   // defaults 1.0
  pub enum RoutingMode { Normal, GoLive }                                   // default Normal
  pub enum LoadOutcome { Fresh(Config), Loaded(Config), RecoveredFromCorrupt { config: Config, backup: PathBuf } }
  pub fn config_path() -> PathBuf                  // %APPDATA%\vSoundboard\config.json
  pub fn load(path: &Path) -> LoadOutcome
  pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()>
  impl Config { pub fn add_pad(&mut self, path: PathBuf) -> u64; pub fn remove_pad(&mut self, id: u64);
                pub fn pad(&self, id: u64) -> Option<&Pad>; pub fn pad_mut(&mut self, id: u64) -> Option<&mut Pad> }
  ```
  All types derive `Serialize, Deserialize, Clone, Debug, PartialEq`, with `#[serde(default)]` at struct level. `add_pad` names the pad after the file stem, sets volume 1.0, and takes the id from `next_pad_id++`. Ids are never reused.

- [ ] **Step 1: Install the toolchain.** Ask the owner before installing. Run `winget install -e --id Rustlang.Rustup`, then `winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`. Open a new shell and run `rustup default stable-msvc`. Pass when `cargo --version` prints a version.
- [ ] **Step 2: Scaffold.** Run `cargo init --name vsoundboard`, then `cargo add serde --features derive serde_json` and `cargo add --dev tempfile`. `main.rs` just calls `config::load(&config::config_path())` and prints the outcome for now.
- [ ] **Step 3: Write the failing tests** in `src/config.rs`:
  - `missing_file_is_fresh`: `load(tmp/"config.json")` returns `Fresh(Config::default())`.
  - `round_trips_unicode_paths`: add pads for `C:\Users\محمد\🎵 sounds\airhorn.mp3` and `…\بروه.wav`, then save and load. The result is `Loaded(cfg)` with `cfg ==` the original, and pad names are `"airhorn"` and `"بروه"`.
  - `corrupt_json_is_backed_up`: write `{"pads": [` to the file. Loading returns `RecoveredFromCorrupt` with an empty config, the backup exists with the original bytes, and `config.json` no longer exists.
  - `unknown_and_missing_fields_tolerated`: `{"version":1,"future_field":3,"pads":[{"id":4,"name":"x","path":"C:\\x.wav"}]}` loads. The pad volume is 1.0 and `next_pad_id >= 5`.
  - `volumes_clamped_on_load`: `volumes.sounds = 7.5` → 1.0, pad volume `-2` → 0.0, NaN → 1.0.
  - `ids_not_reused_after_remove`: add, add, remove the first, add. The ids are `[2, 3]`, not `[2, 1]`.
  - `save_is_atomic`: after `save`, no `*.tmp` file is left in the directory.
- [ ] **Step 4:** `cargo test config` fails to compile or fails.
- [ ] **Step 5: Implement.** `load` normalizes after parsing: clamp volumes to 0..=1 (NaN → 1.0), and raise `next_pad_id` above the maximum pad id. `save` creates the parent directory, writes `config.json.tmp`, then calls `std::fs::rename`.
- [ ] **Step 6:** `cargo test config` passes. `cargo clippy -- -D warnings` is clean.
- [ ] **Step 7: Commit** with "feat: scaffold project and config persistence".

---

### Task 2: Hotkey model

**Files:** Create `src/hotkeys.rs`. Run `cargo add global-hotkey eframe`. Use `eframe::egui` everywhere; never add `egui` as a separate dependency, so the versions can't drift apart.

**Interfaces:**
- Consumes: `Config`, `Pad` (Task 1).
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum HotkeyAction { PlayPad(u64), Stop, ToggleMode }
  pub fn parse(s: &str) -> Option<HotKey>                 // via HotKey::from_str
  pub fn to_config_string(hk: HotKey) -> String           // HotKey::into_string, e.g. "control+Digit1"
  pub fn label(hk: HotKey) -> String                      // human: "Ctrl+1", "Ctrl+Shift+F9", "Num 5", "Alt+A"
  pub fn bindings(cfg: &Config) -> Vec<(HotKey, HotkeyAction)>   // skips None and unparsable
  pub fn find_conflict(cfg: &Config, candidate: HotKey, for_action: HotkeyAction) -> Option<HotkeyAction>
  pub fn conflict_message(cfg: &Config, hk: HotKey, other: HotkeyAction) -> String
  pub fn from_egui(key: egui::Key, mods: egui::Modifiers) -> Option<HotKey>  // None for unsupported keys
  ```
- [ ] **Step 1: Write the failing tests.**
  - `config_string_round_trip`: for Ctrl+1, Ctrl+Shift+F9, Alt+NumpadAdd, and F13, `parse(&to_config_string(hk)) == Some(hk)`.
  - `labels`: Ctrl+Digit1 → `"Ctrl+1"`, Ctrl+Shift+F9 → `"Ctrl+Shift+F9"`, Numpad5 → `"Num 5"`, Alt+KeyA → `"Alt+A"`.
  - `conflict_with_other_pad`: pad 1 has `Ctrl+1`. `find_conflict(cfg, Ctrl+1, PlayPad(2)) == Some(PlayPad(1))`.
  - `no_conflict_with_self`: `find_conflict(cfg, Ctrl+1, PlayPad(1)) == None`.
  - `conflict_with_stop_and_mode`: `stop_hotkey = "control+Digit0"`. Assigning it to a pad conflicts with `Stop`. The same works for `ToggleMode`.
  - `conflict_message_text`: `"Ctrl+1 is already used by “Airhorn”."`, `"Ctrl+0 is already used by Stop."`, `"F9 is already used by the Go Live switch."`
  - `from_egui_maps`: (`Key::Num1`, ctrl) → Ctrl+Digit1. `F9` → F9. Keys with no `Code` mapping → None.
  - `bindings_skip_garbage`: a pad with hotkey `"ctrl++"` is skipped and the other bindings remain.
- [ ] **Step 2:** `cargo test hotkeys` fails.
- [ ] **Step 3: Implement.** `from_egui` covers A–Z, 0–9, F1–F24, the numpad keys egui exposes, arrows, Space, Insert/Delete/Home/End/PageUp/PageDown, and punctuation egui exposes. egui doesn't distinguish numpad digits, so map `Num0..9` → `Digit0..9`. Modifier mapping: ctrl → CONTROL, shift → SHIFT, alt → ALT. `command` is ignored on Windows.
- [ ] **Step 4:** `cargo test hotkeys` passes.
- [ ] **Step 5: Commit** with "feat: hotkey model, labels and conflict detection".

---

### Task 3: Shared params and mixing math

**Files:** Create `src/audio/mod.rs`, `src/audio/shared.rs`, `src/audio/mix.rs`.

**Interfaces:**
- Produces:
  ```rust
  pub struct AtomicF32(AtomicU32); impl AtomicF32 { pub fn new(v: f32) -> Self; pub fn load(&self) -> f32; pub fn store(&self, v: f32) }
  pub struct SharedParams { pub sounds_gain: AtomicF32, pub mic_gain: AtomicF32, pub monitor_gain: AtomicF32,
                            pub pad_gain: AtomicF32, pub go_live: AtomicBool }
  impl SharedParams { pub fn from_config(cfg: &Config) -> Self }
  pub type Frame = [f32; 2];
  /// cable = mic (mono→both) * mic_gain + sound * pad_gain * sounds_gain (sound omitted if go_live); clamp to [-1,1]
  pub fn mix_cable(sound: &[Frame], mic: &[f32], p: &SharedParams, out: &mut [Frame]);
  /// monitor = sound * pad_gain * sounds_gain * monitor_gain; clamp
  pub fn mix_monitor(sound: &[Frame], p: &SharedParams, out: &mut [Frame]);
  /// stereo → device layout: 1 ch = (L+R)/2; ≥2 ch = L, R, then zeros
  pub fn write_interleaved(frames: &[Frame], channels: usize, out: &mut [f32]);
  /// device layout → mono: average of all channels
  pub fn downmix_mono(input: &[f32], channels: usize, out: &mut Vec<f32>);
  ```
- [ ] **Step 1: Write the failing tests.**
  - `go_live_removes_sound_from_cable_only`: sound=[0.5,0.5], mic=[0.2]. Normal: cable=[0.7,0.7]. GoLive: cable=[0.2,0.2], and `mix_monitor` is still [0.5,0.5].
  - `gains_multiply`: sounds_gain 0.5, pad_gain 0.5, monitor_gain 0.5, sound=1.0 → monitor=0.125, cable sound part=0.25.
  - `mic_gain_only_affects_mic`: mic_gain=0 → cable equals the sound part. The monitor is unchanged.
  - `clamps`: a 0.9 sound + 0.9 mic gives exactly 1.0.
  - `interleave_mono_and_multichannel`: [0.2,0.4] with 1 ch gives [0.3]. With 8 ch it gives [0.2,0.4,0,0,0,0,0,0].
  - `downmix`: 2 ch [0.2,0.4,1.0,0.0] gives [0.3,0.5].
  - `mic_shorter_than_sound_pads_with_silence`: mic.len() < sound.len() is treated as silence for the missing samples.
- [ ] **Step 2:** `cargo test audio::mix` fails.
- [ ] **Step 3: Implement.** Write into caller-provided buffers only, with no allocation (`downmix_mono` reuses `out`'s capacity).
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5: Commit** with "feat: mixing math and shared params".

---

### Task 4: Drift-correcting mic resampler

**Files:** Create `src/audio/drift.rs`. Run `cargo add ringbuf`.

**Interfaces:**
- Produces:
  ```rust
  pub struct DriftResampler { /* nominal_ratio, ratio, frac pos, last two samples, target_fill, smoothed_fill, priming: bool, underruns: u64 */ }
  impl DriftResampler {
      pub fn new(in_rate: u32, out_rate: u32, target_fill: usize) -> Self;
      /// Pull mono mic samples from `input`, write exactly out.len() samples at out_rate.
      pub fn process<C: ringbuf::traits::Consumer<Item = f32>>(&mut self, input: &mut C, out: &mut [f32]);
      pub fn underruns(&self) -> u64;
      pub fn smoothed_fill(&self) -> f32;
  }
  ```
- Algorithm (the tests alone don't determine it):
  - Linear interpolation, stepping the input by `ratio = in_rate/out_rate * correction` per output sample.
  - Each call, `smoothed_fill += 0.01 * (fill - smoothed_fill)`. Then `correction = 1 + clamp(0.000_05 * (smoothed_fill - target_fill), -0.005, 0.005)`.
  - **Priming:** while `fill < target_fill`, output silence and consume nothing. Leave priming once fill reaches target.
  - **Underrun:** if the ring empties mid-call, count one underrun, output silence for the rest, and re-enter priming.
  - **Overflow guard:** if `fill > 4 * target_fill`, discard the oldest samples down to `target_fill`.
  - The engine uses `target_fill` = 10 ms of mic samples.
- [ ] **Step 1: Write the failing tests.** Drive the resampler with a simulated producer/consumer loop (no threads): each tick, push `round(in_rate*dt*(1+skew))` samples (keeping the fractional remainder) and pull `out_rate*dt`, with dt = 10 ms.
  - `same_rate_no_skew_is_transparent`: 48k → 48k with a ramp signal. After priming, the output equals the input delayed, within 1e-4.
  - `holds_fill_under_clock_mismatch_for_an_hour`: 48k → 48k with skew +0.0005, then −0.0005, each for 360 000 ticks (1 simulated hour). After the first 10 s: `underruns() == 0`, and `smoothed_fill` stays within `[0.5*target, 2*target]` for the whole run.
  - `converts_44100_to_48000`: a 1 kHz sine in. The measured output frequency (zero crossings over 1 s, after warmup) is 1000 ± 2 Hz.
  - `underrun_recovers`: stop the producer for 50 ticks, then resume. `underruns() >= 1`, output is silence during the gap, and the signal returns after re-priming.
  - `overflow_trims_latency`: push 10 × target at once. After one `process`, fill ≤ target + one call's worth.
- [ ] **Step 2:** `cargo test audio::drift` fails.
- [ ] **Step 3: Implement** as described above.
- [ ] **Step 4:** Tests pass. Use `cargo test --release audio::drift` if the hour-long test is slow in debug.
- [ ] **Step 5: Commit** with "feat: drift-correcting mic resampler".

---

### Task 5: Decoding and rate conversion

**Files:** Create `src/audio/decode.rs`. Run `cargo add symphonia --features mp3 rubato` and `cargo add --dev hound`.

**Interfaces:**
- Produces:
  ```rust
  pub struct SoundReader { /* symphonia FormatReader + Decoder, track id, sample_rate, channels */ }
  impl SoundReader {
      pub fn open(path: &Path) -> Result<Self, String>;        // message is user-facing, e.g. "Can't read this file: unsupported format"
      pub fn sample_rate(&self) -> u32;
      pub fn total_duration(&self) -> Option<Duration>;        // from n_frames when known
      /// Next decoded chunk as stereo frames at the file's own rate; Ok(None) at end of file.
      pub fn next_chunk(&mut self) -> Result<Option<Vec<Frame>>, String>;
  }
  pub struct RateConverter { /* rubato resampler, or passthrough when rates are equal */ }
  impl RateConverter {
      pub fn new(from: u32, to: u32) -> Self;
      pub fn process(&mut self, input: &[Frame]) -> Vec<Frame>;  // buffers internally; any input length
      pub fn flush(&mut self) -> Vec<Frame>;                      // drain the tail at end of file
  }
  ```
  Mono files are duplicated to L=R. More than 2 channels: take the first two. Skip packets that fail to decode with `DecodeError` (corrupt frame) and continue. IO errors other than end of stream are returned as `Err`.
- [ ] **Step 1: Write the failing tests.** Generate fixtures into a tempdir with hound.
  - `reads_mono_22050`: 1 s mono at 22 050 Hz, constant 0.5. Sample rate 22050, about 22050 frames total, every frame [0.5,0.5] (±1e-3), and `total_duration ≈ 1 s`.
  - `reads_96k_24bit_stereo`: 0.5 s with L=0.25, R=−0.25. The values come through per channel.
  - `zero_length_wav`: `open` is Ok and the first `next_chunk` is `Ok(None)`.
  - `garbage_is_error`: 4 KB of random bytes saved as `x.mp3` makes `open` return `Err` whose message starts with `"Can't read this file"`.
  - `missing_file_is_error`: the `Err` message starts with `"File not found"`.
  - `converter_44100_to_48000_length`: converting 44 100 frames (in chunks of 1000) plus `flush` yields 48 000 ± 512 frames.
  - `converter_passthrough_when_equal`: input == output exactly.
  - `decodes_each_format` (`#[ignore]`, run manually): one small MP3, OGG and FLAC that the implementer creates locally (with ffmpeg if available). Each decodes to more than 0 frames. Don't commit copyrighted audio.
- [ ] **Step 2:** `cargo test audio::decode` fails.
- [ ] **Step 3: Implement** with symphonia's probe → default track → decoder, converting into `Vec<Frame>` with `SampleBuffer<f32>`. Use the current rubato's fixed-input-size resampler (sinc or FFT) and handle partial chunks with internal buffering.
- [ ] **Step 4:** Tests pass. Also run `cargo test -- --ignored decodes_each_format`.
- [ ] **Step 5: Commit** with "feat: file decoding and rate conversion".

---

### Task 6: Player thread and sound feeds

**Files:** Create `src/audio/player.rs`. Run `cargo add crossbeam-channel`.

**Interfaces:**
- Consumes: `SoundReader`, `RateConverter` (Task 5), `Frame`, `SharedParams` (Task 3).
- Produces:
  ```rust
  pub enum PlayerCommand { Play { pad: u64, path: PathBuf, gain: f32 }, Stop, Shutdown }
  pub enum PlayerEvent { Started { pad: u64, total: Option<Duration> }, Finished { pad: u64 }, Stopped { pad: u64 }, FileError { pad: u64, message: String } }
  pub struct SoundFeed { /* HeapCons<Frame>, Arc<FeedShared> */ }
  impl SoundFeed {
      /// Fill `out` with sound frames (zeros when empty). Never blocks. Honors flush requests.
      pub fn read(&mut self, out: &mut [Frame]);
  }
  pub struct FeedShared { pub attached: AtomicBool, pub flush_gen: AtomicU64, pub acked_gen: AtomicU64, pub played: AtomicU64 }
  pub struct PlayerHandle { pub tx: Sender<PlayerCommand>, pub feeds_shared: [Arc<FeedShared>; 2], pub rates: [u32; 2] }
  impl PlayerHandle { pub fn elapsed(&self) -> Option<Duration> }  // from the first attached feed's `played` / rate
  /// index 0 = cable, 1 = monitor; ring capacity = 200 ms at each rate
  pub fn spawn_player(rates: [u32; 2], params: Arc<SharedParams>, events: Sender<PlayerEvent>) -> (PlayerHandle, [SoundFeed; 2]);
  ```
- Switch protocol (the tests alone don't determine it):
  1. On `Play` or `Stop`, the player increments `flush_gen` on every feed.
  2. It waits until each **attached** feed has `acked_gen == flush_gen`, polling every 1 ms with a 100 ms timeout.
  3. Only then does it set `params.pad_gain` and push the new sound.
  4. `SoundFeed::read` checks `flush_gen != acked_gen` first. If they differ, it drains the ring, sets `acked_gen`, resets `played`, and outputs zeros for this call.
  5. The player pushes only to attached feeds. When a ring is full, it sleeps 2 ms and retries, still checking for commands.
  6. `Finished` is sent after end of file and once every attached ring is empty. An interrupted sound gets `Stopped`, never `Finished`.
  7. `Play` with the same pad while it's playing restarts from 0.
  8. A file error sends `FileError`. The previous sound is already stopped by then.
- [ ] **Step 1: Write the failing tests.** Run the player with real `SoundFeed`s pumped by a test thread that calls `read` in 10 ms blocks, with both feeds attached. Use DC-level WAV fixtures from Task 5.
  - `plays_to_both_feeds`: a 0.3 s file at 0.5 DC → both feeds see about 0.3 s of 0.5, followed by `Started` then `Finished`.
  - `rapid_switches_never_leak_previous_sound`: Play A (DC 0.25, 5 s), then 20 times in a row Play B (DC 0.75, 5 s) and Play A, ending on B. After the final `Started{B}`, every nonzero frame read is 0.75. Zero `Finished` events arrive before B completes.
  - `stop_silences_and_reports`: Play A, wait 100 ms, Stop. You get `Stopped{A}`, feeds are all zeros within 2 read blocks, and no `Finished{A}` arrives.
  - `retrigger_restarts`: Play A, wait 200 ms, Play A again. `elapsed()` drops back near 0.
  - `garbage_file_reports_error`: Play the garbage `.mp3`. `FileError` arrives with a message starting with `"Can't read this file"`, and there's no panic.
  - `detached_monitor_does_not_stall`: with `attached=false` on feed 1, the 0.3 s file still finishes.
  - `rates_differ_per_feed`: rates [48000, 44100]. Each feed receives about 0.3 s worth at its own rate (±10 ms).
- [ ] **Step 2:** `cargo test audio::player` fails.
- [ ] **Step 3: Implement** the thread loop: `recv_timeout` for commands while idle. While playing, `try_recv` between chunks. One `RateConverter` per attached feed.
- [ ] **Step 4:** Tests pass three runs in a row (`for i in 1 2 3; do cargo test audio::player || break; done`), so flaky timing shows up.
- [ ] **Step 5: Commit** with "feat: player thread with cut-off switching".

---

### Task 7: Devices and reconnect planning

**Files:** Create `src/audio/devices.rs`. Run `cargo add cpal`.

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)] pub enum Role { Mic, Cable, Monitor }
  #[derive(Clone, PartialEq, Debug, Default)] pub enum DeviceState { #[default] NotSelected, Open, Missing, Failed(String) }
  #[derive(Clone, PartialEq, Debug, Default)] pub struct DeviceStatus { pub mic: DeviceState, pub cable: DeviceState, pub monitor: DeviceState }
  pub fn list_inputs() -> Vec<String>;
  pub fn list_outputs() -> Vec<String>;
  pub fn find_cable(outputs: &[String]) -> Option<String>;   // case-insensitive contains "cable input"
  /// Which roles need (re)opening now.
  pub fn plan_reconnect(selected: &DeviceNames, inputs: &[String], outputs: &[String],
                        status: &DeviceStatus, lost: &[Role]) -> (Vec<Role>, DeviceStatus);
  pub fn open_input(name: &str, on_samples: impl FnMut(&[f32], usize /*channels*/) + Send + 'static,
                    lost: Arc<AtomicBool>) -> Result<(cpal::Stream, u32 /*rate*/), String>;
  pub fn open_output(name: &str, fill: impl FnMut(&mut [f32], usize /*channels*/) + Send + 'static,
                     lost: Arc<AtomicBool>) -> Result<(cpal::Stream, u32), String>;
  ```
  `open_*` use the device's default config and handle `F32` and `I16` sample formats, converting to and from f32 at the boundary. The error callback sets `lost`. Use whichever device-name method cpal's current version provides.
  A remembered device that isn't present is `Missing`, not cleared. The name stays in config so the device reconnects when it's plugged back in. This deliberately reads the spec's "fall back to not selected" as "not active, shown in the UI, and recovered automatically", which is what its device-recovery requirement needs.
- [ ] **Step 1: Write the failing tests** (pure ones only):
  - `find_cable_matches_vb_name`: `["Speakers (Realtek)", "CABLE Input (VB-Audio Virtual Cable)"]` → the second entry. `["Speakers"]` → None.
  - `missing_at_startup`: selected mic "USB Mic" isn't in the inputs. Status mic = `Missing`, and Mic isn't in the to-open list.
  - `reappears_reopens`: status mic `Missing`, now listed → Mic is in the to-open list.
  - `lost_flag_reopens_or_marks_missing`: lost=[Cable] and the cable is still listed → reopen. Not listed → `Missing`.
  - `not_selected_stays_quiet`: `monitor: None` → `NotSelected` and never opened.
  - `open_and_healthy_untouched`: an `Open` role that isn't lost is not reopened.
- [ ] **Step 2:** `cargo test audio::devices` fails.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Tests pass. Add an `#[ignore]` test `lists_real_devices` that prints `list_inputs()`/`list_outputs()`. Run it once and check your real devices appear.
- [ ] **Step 5: Commit** with "feat: device enumeration and reconnect planning".

---

### Task 8: Engine assembly (first audible milestone)

**Files:** Create `src/audio/engine.rs`. Modify `src/main.rs` (temporary CLI smoke mode, below).

**Interfaces:**
- Consumes: Tasks 3–7.
- Produces:
  ```rust
  pub enum EngineCommand { PlayPad(u64), Stop, ToggleMode, SetPads(Vec<(u64, PathBuf, f32)>),
                           SetDevices(DeviceNames), Shutdown }
  pub enum EngineEvent { Player(PlayerEvent), ModeChanged(RoutingMode), Devices(DeviceStatus) }
  pub struct EngineHandle { pub tx: Sender<EngineCommand>, pub events: Receiver<EngineEvent>,
                            pub params: Arc<SharedParams>, pub status: Arc<Mutex<DeviceStatus>>,
                            pub position: Arc<Mutex<Option<(u64, Duration, Option<Duration>)>>>,
                            pub mic_latency: Arc<Mutex<Option<Duration>>> }
  pub fn start(devices: DeviceNames, pads: Vec<(u64, PathBuf, f32)>, params: Arc<SharedParams>,
               on_event: Arc<dyn Fn() + Send + Sync>) -> EngineHandle;   // on_event = request_repaint
  ```
- How it works:
  - The engine thread owns the streams, the pad table and the `PlayerHandle`.
  - It loops on `recv_timeout(250 ms)`. Every 2 s it runs `plan_reconnect` with fresh device lists and the `lost` flags.
  - Whenever the set of open output rates changes, it shuts down and respawns the player. This stops the current sound, which is acceptable.
  - Mic callback: `downmix_mono` → push into a `HeapRb<f32>` (capacity 200 ms).
  - Cable callback: `DriftResampler::process` (target 10 ms) → `SoundFeed[0].read` → `mix_cable` → `write_interleaved`.
  - Monitor callback: `SoundFeed[1].read` → `mix_monitor` → `write_interleaved`.
  - Preallocate scratch buffers in each closure at 8192 frames. Grow only if a callback asks for more.
  - `ToggleMode` flips `params.go_live` and emits `ModeChanged`.
  - `PlayPad` for an unknown id is ignored.
  - `mic_latency` = mic callback period + `smoothed_fill` + cable callback period, each converted to time. Update it every 2 s.
  - Forward `PlayerEvent`s and update `position`. Call `on_event()` after each event.
- [ ] **Step 1: Write the failing test** `engine_ignores_unknown_pad_and_survives_no_devices`: start with all devices `None` and send `PlayPad(99)`, `ToggleMode`, `Stop`, `Shutdown`. You get `ModeChanged(GoLive)`, the thread joins within 1 s, and nothing panics.
- [ ] **Step 2:** It fails. **Step 3:** Implement. **Step 4:** It passes.
- [ ] **Step 5: Manual smoke test.** Add a temporary `--smoke <mic> <cable> <monitor> <file>` mode to `main.rs` that starts the engine and plays the file. With VB-Cable installed:
  - In Windows Sound settings → Recording → CABLE Output → Listen, confirm the mix is audible.
  - Talk while the file plays. Your voice and the sound are both present, and the sound also plays on the monitor device.
  - Log `mic_latency` and confirm it's under 30 ms.
  - Let it run for 15 minutes with mic passthrough on. There are no clicks, and the latency doesn't grow.
- [ ] **Step 6: Go Live check (the spec says to do this early).** In Discord, start a whole-screen share with sound, with a second account or a friend watching. Run the smoke test in Normal mode and note whether the viewer hears the sound twice. Toggle to GoLive and confirm they hear it once. Record the result in the commit message. If Discord does **not** double the sound, skip the Go Live parts of Tasks 9 and 11 and the mode hotkey, and note that in the commit.
- [ ] **Step 7:** Remove the smoke mode, or keep it behind `#[cfg(debug_assertions)]`. **Commit** with "feat: audio engine (mic passthrough, cable + monitor outputs)".

---

### Task 9: Main window — pad grid and bottom bar

**Files:** Create `src/ui/mod.rs`, `src/ui/board.rs`, `src/ui/bottom_bar.rs`. Modify `src/main.rs`. Run `cargo add rfd`.

**Interfaces:**
- Consumes: `Config`, `save`, `EngineHandle`, `EngineCommand`, `hotkeys::label`.
- Produces:
  ```rust
  pub enum Screen { Board, Settings }
  pub struct SoundboardApp { pub config: Config, pub config_path: PathBuf, pub engine: EngineHandle,
                             pub screen: Screen, pub playing: Option<u64>, pub pad_errors: HashMap<u64, String>,
                             pub notice: Option<String> }
  impl SoundboardApp {
      pub fn persist(&mut self);              // save + push SetPads; on save error set notice
      pub fn play(&mut self, id: u64);
      pub fn add_files(&mut self, paths: Vec<PathBuf>);   // filters to mp3/wav/ogg/flac (case-insensitive); others → notice
  }
  pub fn grid_columns(available_width: f32) -> usize;     // pad min width 140 px + 8 px gap, at least 1
  pub fn format_time(d: Duration) -> String;              // "1:05", "12:00", "1:02:03"
  ```
- Layout, from the spec and mockup A:
  - **Top bar:** `+ Add sound` (rfd multi-file picker) and `⚙ Devices`.
  - **Grid:** each pad shows its name and its `label(hotkey)` or `—`. The playing pad is highlighted.
  - **Missing file or decode error:** the pad is dimmed, and hovering shows `File not found: <path>` or the error. Check `path.exists()` at startup and before each play.
  - **Drag-and-drop:** onto the window via `ctx.input(|i| i.raw.dropped_files)`.
  - **Bottom bar:** `▶ <name>  1:12 / 3:40` (or `Nothing playing`), `■ Stop (<label>)`, `Sounds` slider, `Mic` slider, and a `Normal | Go Live` toggle. Sliders and the toggle write to `engine.params` immediately and persist when released.
  - **Pad volume:** if the playing pad's volume changes, also store it to `params.pad_gain`.
  - **Events:** drain `engine.events` every frame. `ModeChanged` → update `config.mode` and persist. `FileError` → add to `pad_errors`.
- [ ] **Step 1: Write the failing tests.** `grid_columns(100.0) == 1`, `grid_columns(600.0) == 4`. `format_time` cases as above. In `add_files_filters_extensions`, `[a.MP3, b.txt, c.flac]` adds 2 pads and sets a notice mentioning `b.txt` (construct the app with a test engine started with no devices).
- [ ] **Step 2:** Fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5: Manual check.** `cargo run`. Add 3 files by picker and 2 by drag-drop. Click pads and confirm the cut-off behavior. Restart the app and check the pads persist. Rename a file on disk and restart: that pad is dimmed with a hover message, and clicking it doesn't crash.
- [ ] **Step 6: Commit** with "feat: pad grid and now-playing bar".

---

### Task 10: Pad menu, shortcut capture, and global hotkeys

**Files:** Create `src/ui/capture.rs`. Modify `src/hotkeys.rs` (add `HotkeyRegistry`), `src/ui/board.rs`, and `src/ui/mod.rs`.

**Interfaces:**
- Consumes: Task 2 functions, `EngineCommand`.
- Produces:
  ```rust
  pub struct HotkeyRegistry { /* GlobalHotKeyManager, registered: Vec<HotKey>, Arc<RwLock<HashMap<u32, HotkeyAction>>> */ }
  impl HotkeyRegistry {
      pub fn new(engine_tx: Sender<EngineCommand>) -> Result<Self, String>;  // installs GlobalHotKeyEvent::set_event_handler
      /// Unregister all, then register bindings(cfg). Returns failures as (action, user message);
      /// Error::AlreadyRegistered → "<label> is already used by another app.", other errors → "Couldn't set <label>: <err>".
      pub fn sync(&mut self, cfg: &Config) -> Vec<(HotkeyAction, String)>;
      pub fn suspend(&mut self);   // unregister all (used while capturing)
  }
  pub struct Capture { pub target: HotkeyAction, pub pending: Option<HotKey>, pub error: Option<String> }
  ```
- Event handler: on `HotKeyState::Pressed` only, look up the action and send `PlayPad`/`Stop`/`ToggleMode` to the engine. This works with the window hidden.
- `HotkeyRegistry::new` must run on the eframe main thread (the global-hotkey requirement on Windows), so do it inside the eframe app-creator closure.
- Right-click menu on a pad: `Rename` (inline text field; empty → keep old name), `Set shortcut…`, `Volume` (slider 0–100%), `Remove`.
- Capture modal:
  - Title `Press the keys for “<name>”`. Buttons `Clear` and `Cancel`. Esc cancels.
  - On open, call `suspend()` so existing combos reach egui.
  - The first non-modifier key press (with modifiers) becomes `pending`.
  - If `find_conflict` finds something, show `conflict_message` and don't save.
  - Otherwise save to config, `persist`, and `sync`. If the OS rejects the combo (`AlreadyRegistered`), show `"<label> is already used by another app."`, revert that pad's hotkey, and `sync` again.
  - Closing the modal always ends with `sync`.
- [ ] **Step 1: Write the failing test.** Make a pure helper `fn apply_capture(cfg: &mut Config, cap: &Capture) -> Result<(), String>` that returns the conflict message, or sets the hotkey string on the right target (pad/stop/mode). Tests: it saves to a pad, rejects a combo used by Stop with the exact message, and `Clear` sets `None`.
- [ ] **Step 2:** Fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5: Manual check.** Assign Ctrl+1 and F9. Focus Notepad: the shortcuts play sounds and the keystroke doesn't reach Notepad. Assign a combo another app holds (start a second instance of the app with the same shortcut) and see the "another app" message. Try a combo already on another pad and see the pad-conflict message.
- [ ] **Step 6: Commit** with "feat: shortcut capture and global hotkeys".

---

### Task 11: Settings screen

**Files:** Create `src/ui/settings.rs`. Modify `src/ui/mod.rs`.

**Interfaces:**
- Consumes: `list_inputs`, `list_outputs`, `find_cable`, `DeviceStatus`, `EngineCommand::SetDevices`, `Capture` (Task 10), `engine.mic_latency`.
- Content:
  - **Three dropdowns:** `Microphone`, `Virtual mic (to Discord)`, `Headphones`. Each has a status line: `Working` / `Not found — plugged in?` / `Not selected` / `Error: <msg>`. Refresh the lists when the screen opens.
  - **Default on first run:** if `devices.cable` is None and `find_cable` finds one, preselect it.
  - **Headphones volume** slider. This writes `monitor_gain`.
  - **Shortcut buttons** for `Stop` and `Go Live switch`, using the capture modal.
  - **Latency readout:** `Mic delay added: ~N ms`.
  - **← Back** button.
  - **Help copy, exactly:**
    - When no cable is found: `VB-Cable wasn't found. vSoundboard needs it to act as a microphone in Discord. Install it free from https://vb-audio.com/Cable/ (restart your PC afterwards), then choose “CABLE Input” above.` Render the URL as a hyperlink.
    - Always: `In Discord → User Settings → Voice & Video, set Input Device to “CABLE Output (VB-Audio Virtual Cable)”. Turn off Noise Suppression and Automatic Gain Control, and set Input Sensitivity low (or manual) so music isn't cut off.`
    - Always: `Using Push to Talk? Sounds only reach the call while you hold your Push to Talk key.`
    - Always: `Sharing your whole screen with sound? Switch to Go Live mode so viewers hear each sound once. In Go Live mode, friends who aren't watching your stream won't hear sounds.`
  - Changing a device persists and sends `SetDevices`.
- [ ] **Step 1: Write the failing test** for a pure helper `fn status_text(s: &DeviceState) -> String` covering all four states with the exact strings above.
- [ ] **Step 2:** Fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5: Manual check.**
  - Unplug a USB mic or headset while running: the status changes to `Not found — plugged in?` within about 2 s. Plug it back in: it changes to `Working` and passthrough resumes without a restart.
  - Uninstalling VB-Cable isn't needed. Instead, temporarily make `find_cable` return None to see the help text.
- [ ] **Step 6: Commit** with "feat: settings screen with device status and help".

---

### Task 12: Tray and close-to-tray

**Files:** Create `src/tray.rs`. Modify `src/ui/mod.rs` and `src/main.rs`. Run `cargo add tray-icon`.

**Interfaces:**
- Produces: `pub fn install(ctx: egui::Context, engine_tx: Sender<EngineCommand>, quit: Arc<AtomicBool>) -> Result<tray_icon::TrayIcon, String>`. Keep the returned `TrayIcon` alive in the app.
- Behavior:
  - **Window close:** in `update`, if `close_requested()` and `!quit`, send `ViewportCommand::CancelClose` and `Visible(false)`.
  - **Tray menu:** `Show`, `Stop sound`, `Quit`, handled via `MenuEvent::set_event_handler`, so it works while hidden.
    - `Show` → `Visible(true)` + `Focus` + `request_repaint`.
    - `Stop sound` → `EngineCommand::Stop`.
    - `Quit` → `quit=true`, `Visible(true)`, `Close`.
  - **Left-click on the icon:** same as `Show`.
  - **Icon:** generated in code (32×32 RGBA). No asset file.
  - **Exit:** `App::on_exit` copies `params.go_live` into `config.mode`, persists, and sends `Shutdown`.
  - **Known risk:** if `Visible(true)` from the handler doesn't wake a hidden eframe window on this eframe version, fall back to `Minimized(true)` on close and `Minimized(false)` on Show. Note which one you used in the commit.
- [ ] **Step 1: Manual test only** (OS integration). Close the window: the app stays in the tray, and shortcuts still play into the cable. Toggle Go Live by shortcut while hidden, then Tray → Quit and restart: the mode was remembered. Tray → Show restores the window.
- [ ] **Step 2: Commit** with "feat: system tray and close-to-tray".

---

### Task 13: End-to-end check and README

**Files:** Modify `README.md`.

- [ ] **Step 1: README.** Cover what it is, VB-Cable install with the link, Discord settings (same copy as the Settings screen), how to add sounds and set shortcuts, Go Live mode, and building with `cargo build --release`.
- [ ] **Step 2: End-to-end in Discord** with a second account or a friend:
  - Voice and sounds are heard in Normal mode.
  - Go Live mode works during a whole-screen share.
  - Shortcuts work while a game is focused. Try a game running as admin if available.
  - Mashing shortcuts plays only the latest sound.
  - Re-triggering the playing pad restarts it.
- [ ] **Step 3: Long run.** 2 hours in a call with mic passthrough on. Friends report no echo, robotic voice, or growing delay. `Mic delay added` stays under 30 ms.
- [ ] **Step 4:** `cargo clippy -- -D warnings`, `cargo test`, and `cargo build --release` all pass. The release exe opens with no console window.
- [ ] **Step 5: Commit** with "docs: README and setup guide".

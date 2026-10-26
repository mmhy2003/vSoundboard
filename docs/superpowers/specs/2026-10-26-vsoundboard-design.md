# vSoundboard — Design

Date: 2026-10-26
Status: Draft, awaiting approval

## Intent

A free, software-only soundboard for Windows. The owner talks with friends on Discord and streams there, and wants to play sound effects and music that the people on the call hear — without buying a hardware soundboard. Sounds are triggered by keyboard shortcuts that work while Discord or a game has focus.

Success: with Discord's input device set to the virtual cable, friends hear the owner's voice as normal, plus any sound the owner fires, and the owner hears the sounds in their own headphones too.

## Goals

- Play local audio files (MP3, WAV, OGG, FLAC) into a virtual microphone that Discord uses as its input.
- Mix the owner's real microphone into that same virtual microphone, so voice and sounds both reach the call.
- Play the sounds (not the owner's voice) to the owner's headphones at the same time.
- Global keyboard shortcuts per sound, plus a global Stop shortcut.
- A simple desktop window laid out as a pad grid (chosen by the owner).
- Remember everything between runs: pads, shortcuts, per-sound volume, devices, master volumes.

## Non-goals (v1)

- Streaming from URLs/YouTube — sounds are files on disk.
- Layering/overlapping sounds — only one sound plays at a time.
- macOS / Linux.
- Writing or shipping our own virtual audio driver.
- Bundling or auto-installing VB-Cable.
- Voice effects / voice changer, EQ, ducking.
- Push-to-talk support (holding Discord's PTT key while a sound plays). The owner rarely uses PTT; with PTT on, sounds only reach the call while the key is held, and the help text says so.
- Pages, folders, categories or tags for pads.
- Cloud sync, sharing boards, plugin systems.

## Anti-goals (would make it a failure even if it "works")

- The owner's voice sounds worse or noticeably delayed to friends because it's routed through the app.
- A shortcut silently fails to fire mid-game.
- Setup has to be redone each time the app starts.

## Constraints and decisions

- **Platform:** Windows 10/11 only.
- **Language:** Rust. *(Claude's call; the owner delegated it.)* Reason: mature Windows audio crates, no GC pauses in the audio path, single small .exe.
- **UI toolkit:** egui via eframe. *(Claude's call.)* Native window, no webview.
- **Virtual mic:** VB-Cable (free, installed by the owner). The app writes its mix to the "CABLE Input" playback device; Discord selects "CABLE Output" as its microphone. Windows does not let a user-mode app create a capture device, so a third-party virtual cable is required.

## How it behaves

### Audio routing

```
Real mic (capture) ──┐
                     ├─► mix ──► CABLE Input (playback) ──► Discord mic = CABLE Output
Current sound ───────┤
                     └─► Headphones / monitor device (sound only, no mic)
```

- Mic passthrough is always on while the app is running and devices are configured. It must add as little latency as practical — target under ~30 ms added by the app itself (mic capture → our mix → write to CABLE Input), measured with ordinary Windows shared-mode audio. VB-Cable's own buffering and Discord's capture are outside that budget. *(Claude's call.)* Must never be perceptibly laggy in conversation.
- The mic and the cable are different devices with independent clocks. The passthrough must handle clock drift over long sessions (hours) without growing delay or periodic clicks/dropouts.
- Device sample rates/formats may differ from the files and from each other; resample as needed.
- **Sounds volume** master slider scales the sound in both the cable and the headphones. **Mic volume** slider scales only the mic going into the cable. Each pad also has its own volume (multiplies with the master).
- *(Claude's call)* The monitor (headphones) output has its own volume so the owner can hear sounds quieter than friends do. If this is cluttering the UI, it can live in the Devices screen rather than the bottom bar.

### Go Live mode (whole-screen sharing)

The owner shares their whole screen with sound when going live. Discord then captures system audio, which includes our headphone copy of the sound — so in normal routing, viewers would hear each sound twice, slightly out of sync (once through the mic, once through the stream).

*(Claude's call)* The app has two routing modes, switchable from the bottom bar and by an optional global shortcut, remembered between runs:

- **Normal** (default): sound → CABLE Input (mixed with mic) **and** → headphones.
- **Go Live**: sound → headphones only. The screen share carries it to viewers once; the owner still hears it. Mic passthrough continues unchanged. Trade-off: people in the voice call who aren't watching the stream won't hear sounds in this mode.

The current mode is always visible in the bottom bar. The builder should verify early, against a real Discord whole-screen share, that (a) Normal mode does produce the double sound and (b) Go Live mode produces exactly one copy; if Discord turns out to exclude our output from capture, drop Go Live mode.

### Playback rules

- Only one sound plays at a time. Triggering any pad stops the current sound and starts the new one immediately (owner's decision).
- *(Claude's call)* Triggering the pad that is currently playing restarts it from the beginning.
- Stop (button or global shortcut) stops whatever is playing.
- Long files (full songs) must start quickly; don't require decoding a whole song into memory before playback starts.

### Main window (pad grid — owner's choice, mockup "A")

- Top: "+ Add sound" button and a "Devices" button.
- Middle: grid of pads. Each pad shows the sound's name and its shortcut (or "—" if none). Clicking a pad plays it. The playing pad is visibly highlighted. Grid reflows with window width.
- Bottom bar: currently playing name with elapsed / total time, a Stop button showing its shortcut, Sounds volume slider, Mic volume slider, Normal / Go Live mode switch.
- A pad whose file is missing shows as unavailable and explains why on hover; clicking it doesn't crash.

### Managing sounds

- Add via the "+ Add sound" button (file picker, multi-select) or by dragging files onto the window.
- The app references files where they are; it does not copy them. *(Claude's call.)*
- Default pad name = file name without extension.
- Right-click a pad: Rename, Set shortcut, Volume, Remove. *(Claude's call)* Reordering pads by drag is nice-to-have, not required for v1.
- Setting a shortcut: the app listens for the next key combination the owner presses and shows it; Esc cancels; a "Clear" option removes it. If the combo is already used by another pad, or can't be registered with Windows (taken by another app), say so plainly and don't assign it.

### Devices screen

- Three dropdowns: Microphone (real mic), Virtual mic output (defaults to "CABLE Input" if present), Headphones / monitor.
- If no VB-Cable device is found, show a clear message with a link to https://vb-audio.com/Cable/ and a one-line explanation of what to set in Discord.
- *(Claude's call)* The help text recommends Discord settings that let sounds through cleanly: input sensitivity / voice activity set low or manual, and noise suppression (Krisp) and automatic gain off, since those can cut or mangle music.
- If a selected device disappears (unplugged, driver restart), show it in the UI and recover automatically when it comes back, without restarting the app.

### Shortcuts

- Global: they must fire while Discord, a browser, or a game has focus.
- Any key combo with at least one non-modifier key; function keys and numpad keys allowed.
- One global Stop shortcut, configurable like the others. One optional global shortcut to toggle Normal / Go Live mode.
- *(Claude's call)* Use Windows' standard hotkey registration (`RegisterHotKey`). A fired shortcut is consumed and does not reach the focused app — the owner should pick combos they don't need in-game. This lets the app detect combos already taken by other apps, keeps working when an elevated game has focus, and avoids needing admin rights (which would break drag-and-drop from Explorer). The app does not run elevated. A low-level keyboard hook is out of scope for v1.

### Window and tray

- *(Claude's call)* Closing the window hides it to the system tray; shortcuts keep working. Tray menu: Show, Stop sound, Quit.
- Starting with Windows is not required for v1.

### Persistence

- *(Claude's call)* Config stored as a single human-readable file under `%APPDATA%\vSoundboard\`. Save automatically on every change. Survive a missing/corrupt config by starting empty and keeping a backup of the bad file rather than overwriting it silently.
- Remembered devices are matched by name on startup; if not found, fall back to "not selected" and surface it in the UI.

## Left to the builder

- Crate choices for audio I/O, decoding, resampling, global hotkeys, tray, file dialogs (e.g. cpal/WASAPI, symphonia, rubato, global-hotkey or a low-level keyboard hook, tray-icon, rfd). Pick what meets the latency and reliability requirements above; if `RegisterHotKey`-style hotkeys prove unreliable in games, a low-level keyboard hook is acceptable.
- Threading model and how the UI talks to the audio engine (it must never block the audio callback).
- Exact clock-drift strategy for mic passthrough.
- Config file format and schema.
- Visual styling of the egui UI, beyond the layout above.
- Test strategy; the mixing/decoding/config logic should be testable without real devices.

## Open items

None blocking. VB-Cable licensing means we only link to it, not redistribute it.

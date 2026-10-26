# vSoundboard

A free software soundboard for Windows. You trigger sound effects and music with keyboard shortcuts, and the people in your Discord call hear them mixed with your voice. You hear them in your headphones too.

## How it works

```
Your mic ──────┐
               ├─► mix ──► CABLE Input ──► Discord's mic = "CABLE Output"
Current sound ─┤
               └─► your headphones (sound only)
```

Windows doesn't let an ordinary app create a microphone, so vSoundboard uses the free **VB-Cable** virtual audio cable as Discord's mic.

## Install

1. Download `vSoundboard-Setup-X.Y.Z.exe` from the [Releases page](https://github.com/mmhy2003/vSoundboard/releases).
2. Run it. The installer isn't code-signed, so Windows may show "Windows protected your PC". Click **More info → Run anyway**.
3. Choose your options:
   - **Desktop shortcut.**
   - **Start vSoundboard when Windows starts.** It starts hidden in the tray; you can change this later under ⚙ Devices.
   - **Download and install VB-Cable**, offered only if it's missing. The installer fetches VB-Audio's official package and runs their setup, which asks for administrator permission. VB-CABLE is made by VB-Audio (https://vb-cable.com) and is donationware: free to use, and you can support VB-Audio at vb-cable.com.
4. If you installed VB-Cable, **restart your PC** before using vSoundboard in Discord.

It installs just for your user (no administrator rights needed for vSoundboard itself). Uninstalling from Windows' Apps settings keeps your sounds and shortcuts, and leaves VB-Cable installed.

## Setup

1. **VB-Cable:** if you didn't install it with the installer, get it free from https://vb-audio.com/Cable/ and restart your PC.
2. **Start vSoundboard** and click **⚙ Devices**:
   - **Microphone:** your real mic.
   - **Virtual mic (to Discord):** "CABLE Input (VB-Audio Virtual Cable)". This is picked automatically when VB-Cable is installed.
   - **Headphones:** where you want to hear the sounds yourself.
3. **In Discord**, go to User Settings → Voice & Video:
   - Set **Input Device** to "CABLE Output (VB-Audio Virtual Cable)".
   - Turn off **Noise Suppression** and **Automatic Gain Control**.
   - Set **Input Sensitivity** low (or manual), so music isn't cut off.

If you use Push to Talk, sounds only reach the call while you're holding your Push to Talk key.

## Using it

- **Add sounds:** drag MP3, WAV, OGG or FLAC files onto the window, or click **+ Add sound**. Files stay where they are on your PC; vSoundboard just remembers where they are.
- **Play:** click a pad or press its shortcut. Shortcuts work while Discord or a game has focus. Starting a new sound cuts off the one that's playing. Pressing the playing pad again restarts it.
- **Right-click a pad** to rename it, set its shortcut, change its volume, or remove it.
- **Stop:** use the ■ Stop button, or set a Stop shortcut under ⚙ Devices.
- **Shortcuts are taken over:** the key press goes to vSoundboard and doesn't reach the game, so pick combinations you don't use in-game.
- **Closing the window** keeps vSoundboard running in the system tray, so shortcuts keep working. Use the tray icon's menu to show the window again or quit.

### Go Live mode

If you share your **whole screen with sound**, Discord also records the copy of each sound playing in your headphones, so viewers would hear it twice. Switch the bottom bar to **Go Live**. Sounds then play only to your headphones, and your screen share carries them to viewers once. In Go Live mode, friends in the call who aren't watching your stream won't hear sounds. You can give the switch a shortcut under ⚙ Devices.

## Settings file

Everything is saved automatically to `%APPDATA%\vSoundboard\config.json`. If that file gets damaged, vSoundboard starts fresh and keeps the old file next to it as `config.json.bad-<number>`.

## Building

Requires Rust (MSVC toolchain) and the Visual Studio C++ Build Tools. The installer also needs [Inno Setup 7](https://jrsoftware.org/isinfo.php), with `iscc` on PATH, and GNU Make.

```
make installer             # dist\vSoundboard-Setup-<version>.exe
cargo build --release      # target\release\vsoundboard.exe
cargo test
cargo run --example smoke  # plays a test tone through VB-Cable and reports what arrived
```

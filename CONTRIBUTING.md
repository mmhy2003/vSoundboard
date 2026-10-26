# Contributing to vSoundboard

Thanks for wanting to help! Bug reports, ideas, documentation fixes and code are all welcome.

## Reporting bugs and suggesting ideas

[Open an issue](https://github.com/mmhy2003/vSoundboard/issues). For bugs, please include:

- your Windows version;
- your vSoundboard version (shown under ⚙ Devices);
- what you did, what you expected, and what happened instead;
- for audio problems, which microphone and output devices you selected.

## Getting set up

You need:

- **Rust**, stable, with the MSVC toolchain (`rustup default stable-msvc`);
- the **Visual Studio C++ Build Tools**, with the "Desktop development with C++" workload;
- **GNU Make**, used for the shortcuts below;
- [**Inno Setup 7**](https://jrsoftware.org/isinfo.php), with `iscc` on PATH, if you want to build the installer;
- [**VB-Cable**](https://vb-audio.com/Cable/), to try the audio path for real.

Then clone the repo and run the app:

```
git clone https://github.com/mmhy2003/vSoundboard.git
cd vSoundboard
git checkout develop
cargo run
```

## Branches

- **`develop`** is where all work happens. Branch from it and open pull requests against it.
- **`main`** only receives merges from `develop` when a release is made. Don't open pull requests against `main`.

## Making a change

1. **Branch** from `develop`, for example `fix/hotkey-numpad` or `feature/sound-folders`.
2. **Write a test first** where you can. Most logic lives in plain functions with unit tests next to them (`#[cfg(test)] mod tests`). Audio, decoding, config, updates and shortcuts all have examples to follow.
3. **Keep the audio callbacks real-time safe.** Code that runs inside a cpal callback must not lock, allocate or block. See `src/audio/engine.rs`.
4. **Run the checks** before pushing:

   ```
   make check    # cargo fmt --check, clippy with warnings as errors, and the tests
   ```

   Some tests are `#[ignore]`d because they need real devices, local audio files or the internet. Run them with `make test-all` if your change touches those areas.
5. **Try it for real** if you changed anything you can hear or see. `cargo run --example smoke` plays a test tone through VB-Cable and reports what arrived.
6. **Open a pull request** against `develop`. Describe what changed, why, and how you tested it. CI builds and tests every pull request and produces a test installer you can download from the run's Artifacts.

## Project layout

| Path | What's there |
|------|--------------|
| `src/audio/` | Audio engine: devices, mic passthrough with clock-drift correction, mixing, decoding, the player |
| `src/ui/` | The egui window: pad grid, bottom bar, Devices screen, dialogs |
| `src/update/` | Update checks, verified downloads, and the handoff to the installer |
| `src/config.rs` | Settings file (`%APPDATA%\vSoundboard\config.json`) |
| `src/hotkeys.rs` | Global shortcuts |
| `installer/vsoundboard.iss` | Inno Setup script |
| `.github/workflows/` | CI and release pipelines |
| `docs/superpowers/` | Design documents and implementation plans for each feature |

## Releases

Maintainers make releases by bumping the version in `Cargo.toml` on `develop`, merging into `main`, and tagging `vX.Y.Z`. The tag builds the installer and publishes the GitHub Release; see `CLAUDE.md` for the exact steps. You don't need to bump versions in pull requests.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE), the same as the rest of the project.

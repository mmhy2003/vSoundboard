# vSoundboard: working rules

## Branches
- `develop` is where all work happens: new features, fixes and docs. Commit there, or on short-lived branches merged into `develop`.
- `main` only receives merges from `develop`, and only when the owner decides to release. Never commit directly to `main`.

## Releasing (every merge into `main`)
1. Bump `version` in `Cargo.toml` (and `Cargo.lock`) on `develop`, then commit as `release: vX.Y.Z`.
2. Merge `develop` into `main`.
3. Tag the merge commit `vX.Y.Z` (annotated) and push `main` and the tag.
4. The `v*` tag triggers `.github/workflows/release.yml`, which checks the tag against `Cargo.toml`, builds the installer and publishes a GitHub Release.

Pushes to `develop` (and pull requests) run `.github/workflows/ci.yml`, which builds a test installer you can download from the run's Artifacts.

#!/usr/bin/env bash
# Usage: check-tag-version.sh <tag>
# Fails unless the tag (minus a leading "v") equals the package version in Cargo.toml,
# so a release can never be published under the wrong version.
set -euo pipefail
tag="${1:?usage: check-tag-version.sh <tag>}"
cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$(dirname "$0")/../Cargo.toml" | head -n 1)
if [ "${tag#v}" != "$cargo_version" ]; then
  echo "Tag $tag does not match Cargo.toml version $cargo_version" >&2
  exit 1
fi
echo "Tag $tag matches Cargo.toml version $cargo_version"

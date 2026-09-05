#!/usr/bin/env bash
# Build the same verified macOS asset locally and in GitHub Actions.
set -euo pipefail
cd "$(dirname "$0")/.."
tag=${1:-v$(tr -d '[:space:]' < VERSION)}
python3 scripts/version.py check --tag "$tag"
[[ $(uname -s) == Darwin ]] || { echo "macOS is required for a universal release" >&2; exit 1; }

for target in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked --target "$target"
done

mkdir -p target/release-assets
asset=target/release-assets/stt-cli-macos-universal
lipo -create -output "$asset" \
  target/aarch64-apple-darwin/release/stt-cli \
  target/x86_64-apple-darwin/release/stt-cli
chmod +x "$asset"
codesign --force --sign - "$asset"
codesign --verify --strict "$asset"
lipo "$asset" -verify_arch arm64 x86_64
python3 scripts/version.py check --tag "$tag" --binary "$asset"
(cd target/release-assets && shasum -a 256 stt-cli-macos-universal > SHA256SUMS)
echo "Release assets: target/release-assets"

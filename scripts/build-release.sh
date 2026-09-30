#!/usr/bin/env bash
# Build and verify the macOS release asset locally.
set -euo pipefail
cd "$(dirname "$0")/.."
tag=${1:-v$(tr -d '[:space:]' < VERSION)}
python3 scripts/version.py check --tag "$tag"
[[ $(uname -s) == Darwin ]] || { echo "macOS is required for a universal release" >&2; exit 1; }

# Panic locations and file!() can retain private paths even in stripped binaries.
# Encoded flags preserve paths containing spaces and existing compiler options.
CARGO_ENCODED_RUSTFLAGS=$(python3 - <<'PY'
import os
from pathlib import Path

encoded = os.environ.get("CARGO_ENCODED_RUSTFLAGS")
if encoded is None:
    flags = os.environ.get("RUSTFLAGS", "").split()
else:
    flags = encoded.split("\x1f") if encoded else []
for source, target in (
    (Path.home(), "/build/home"),
    (Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")), "/build/cargo"),
    (Path.cwd(), "/build/stt-cli"),
):
    for prefix in dict.fromkeys((str(source.absolute()), str(source.resolve()))):
        flags.append(f"--remap-path-prefix={prefix}={target}")
print("\x1f".join(flags), end="")
PY
)
export CARGO_ENCODED_RUSTFLAGS

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
for architecture in arm64 x86_64; do
  lipo "$asset" -verify_arch "$architecture"
done
python3 scripts/version.py check --tag "$tag" --binary "$asset"
cp LICENSE target/release-assets/LICENSE
(cd target/release-assets && shasum -a 256 stt-cli-macos-universal LICENSE > SHA256SUMS)
echo "Release assets: target/release-assets"

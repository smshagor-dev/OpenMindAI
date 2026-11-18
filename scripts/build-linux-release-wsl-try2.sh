#!/usr/bin/env bash
set -euo pipefail

export PATH="/home/smshagor/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export CARGO_HOME="/home/smshagor/openmindai-cargo-home-4.0.0-try2"
export CARGO_TARGET_DIR="/home/smshagor/openmindai-cargo-target-4.0.0-try2"
export TMPDIR="/home/smshagor/openmindai-tmp-4.0.0-try2"
export NPM_CONFIG_CACHE="/home/smshagor/openmindai-npm-cache-4.0.0-try2"
export CARGO_PROFILE_RELEASE_LTO=false
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=32
export CARGO_BUILD_JOBS=1
export RUST_BACKTRACE=1

src_tar="/mnt/c/Users/smsha/AppData/Local/Temp/openmindai-src-4.0.0.tar"
workdir="/home/smshagor/openmindai-wsl-build-4.0.0-try2"
outdir="/mnt/c/Users/smsha/AppData/Local/Temp/openmindai-linux-artifacts-4.0.0"

rm -rf "$workdir" "$outdir" "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR" "$NPM_CONFIG_CACHE"
mkdir -p "$workdir" "$outdir" "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR" "$NPM_CONFIG_CACHE"
tar -xf "$src_tar" -C "$workdir"
cd "$workdir"

rustup component add rustfmt
npm ci

cat > src-tauri/tauri.linux-unsigned.conf.json <<'JSON'
{
  "$schema": "https://schema.tauri.app/config/2",
  "bundle": {
    "targets": ["deb", "appimage"],
    "createUpdaterArtifacts": false
  }
}
JSON

npm run tauri -- build --config src-tauri/tauri.linux-unsigned.conf.json

find "$CARGO_TARGET_DIR/release/bundle" -type f \( -name "*.deb" -o -name "*.AppImage" \) -print -exec cp {} "$outdir/" \;

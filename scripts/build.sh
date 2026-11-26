#!/bin/sh
# Builds the mom CLI and momputer.app on a Mac. The twin of build.ps1.
#   ./scripts/build.sh              -> target/release/bundle/macos/momputer.app and a .dmg
#   ./scripts/build.sh --app        -> just momputer.app (no disk image)
#   ./scripts/build.sh --no-bundle  -> just target/release/momputer and target/release/mom
# You need Rust 1.95+ and Xcode's command line tools (`xcode-select --install`).
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p mom-cli

# Tauri bundles mom next to the app; it wants the target triple in the file name.
triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p app/src-tauri/binaries
cp target/release/mom "app/src-tauri/binaries/mom-$triple"
chmod +x "app/src-tauri/binaries/mom-$triple"

if [ "${1:-}" = "--no-bundle" ]; then
    cargo build --release -p momputer
    echo "Built target/release/momputer and target/release/mom"
    exit 0
fi

if ! cargo tauri --version >/dev/null 2>&1; then
    echo "Installing the Tauri CLI (one time)..."
    cargo install tauri-cli --version '^2.12' --locked
fi
cd app/src-tauri
if [ "${1:-}" = "--app" ]; then
    cargo tauri build --bundles app
else
    cargo tauri build
fi
echo "momputer.app is in target/release/bundle/macos/"

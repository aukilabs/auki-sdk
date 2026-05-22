#!/usr/bin/env bash
set -euo pipefail

CROSS_VERSION="0.2.5"
WASM_PACK_VERSION="0.13.1"
WASM_BINDGEN_CLI_VERSION="0.2.121"

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required; install it from https://rustup.rs/" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required; install Rust with rustup first" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required for Python binding generation; install Python 3 first" >&2
  exit 1
fi
python3 --version

if ! command -v node >/dev/null 2>&1; then
  echo "node is required for JavaScript binding smoke tests; install Node.js first" >&2
  exit 1
fi
node --version

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required for JavaScript package checks; install Node.js/npm first" >&2
  exit 1
fi
npm --version

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for Linux cross builds; install Docker Desktop or Docker Engine first" >&2
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  echo "docker is required for Linux cross builds; start Docker and verify 'docker info' succeeds" >&2
  exit 1
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  if ! xcodebuild -version >/dev/null 2>&1; then
    echo "xcodebuild is required for Apple platform builds; install Xcode or Xcode Command Line Tools first" >&2
    exit 1
  fi

  if ! xcrun --version >/dev/null 2>&1; then
    echo "xcrun is required for Apple platform builds; run 'xcode-select --install' or select a valid Xcode path" >&2
    exit 1
  fi

  if ! command -v lipo >/dev/null 2>&1; then
    echo "lipo is required for Apple platform builds; install Xcode Command Line Tools first" >&2
    exit 1
  fi

  xcodebuild -version
  xcrun --version
fi

rust_targets=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  aarch64-apple-ios
  aarch64-apple-ios-sim
  x86_64-apple-ios
  wasm32-unknown-unknown
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
)

for target in "${rust_targets[@]}"; do
  rustup target add "$target"
done

cargo install --force --locked cross --version "$CROSS_VERSION"
cargo install --force --locked wasm-pack --version "$WASM_PACK_VERSION"
cargo install --force --locked wasm-bindgen-cli --version "$WASM_BINDGEN_CLI_VERSION"

cross --version
wasm-pack --version
wasm-bindgen --version

echo "Auki SDK toolchain is installed."

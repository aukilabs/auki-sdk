#!/usr/bin/env bash
# Build auki-sdk-web into src/web/generated for the Expo web backend.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_CRATE="$(cd "$ROOT/../web/auki-sdk-web" && pwd)"
OUT="$ROOT/src/web/generated"

command -v wasm-pack >/dev/null || {
  echo "wasm-pack is required (pin 0.13.1). e.g. cargo install wasm-pack --version 0.13.1 --locked" >&2
  exit 1
}

rustup target list --installed | grep -q '^wasm32-unknown-unknown$' || {
  echo "missing rustup target wasm32-unknown-unknown" >&2
  exit 1
}

rm -rf "$OUT"
mkdir -p "$OUT"

# getrandom 0.3 on wasm32-unknown-unknown may need:
#   export RUSTFLAGS="${RUSTFLAGS:-} --cfg getrandom_backend=\"wasm_js\""
# (left off for now — file for Matt if noise/getrandom panics under release)

# Temporary: --dev like standard-protocols/web. --release + wasm-opt hung under
# Metro after DMS Ready (no dial / no WSS). Report upstream; switch back when fixed.
# finite-protocols = info + catalog + registry + blob (dig path needs registry/blob).
wasm-pack build "$WEB_CRATE" \
  --target web \
  --out-dir "$OUT" \
  --dev \
  --features finite-protocols,message,stream \
  --locked

# Drop npm packaging noise from wasm-pack; Metro imports the JS/WASM directly.
rm -f "$OUT/package.json" "$OUT/.gitignore" "$OUT/README.md"

echo "wrote wasm package to $OUT"

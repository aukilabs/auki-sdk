#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pkg="bindings/javascript/auki-proto"
src="$pkg/src"

rm -rf "$pkg"
mkdir -p "$src"

cat > "$pkg/package.json" <<'JSON'
{
  "name": "@aukilabs/auki-proto",
  "type": "module",
  "version": "0.0.0",
  "license": "MIT",
  "description": "Generated JavaScript/TypeScript protobuf bindings for Auki schemas.",
  "private": true,
  "dependencies": {
    "@bufbuild/protobuf": "^2.0.0"
  },
  "devDependencies": {
    "@bufbuild/protoc-gen-es": "^2.0.0"
  }
}
JSON

cat > "$pkg/README.md" <<'MD'
# @aukilabs/auki-proto

Generated JavaScript/TypeScript protobuf bindings from root `proto/auki` schemas.

This directory is generated locally by `scripts/generate-javascript-proto.sh` and is ignored by git. Do not edit or commit generated files from this directory.
MD

(
  cd "$pkg"
  npm install --silent
)

PATH="$PWD/$pkg/node_modules/.bin:$PATH" \
protoc \
  -I proto \
  --es_out "$src" \
  --es_opt target=ts,import_extension=js \
  proto/auki/*.proto

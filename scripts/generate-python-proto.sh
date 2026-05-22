#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pkg="bindings/python/auki-proto"

rm -rf "$pkg"
mkdir -p "$pkg"

cat > "$pkg/pyproject.toml" <<'TOML'
[project]
name = "auki-proto"
version = "0.0.0"
description = "Generated Python protobuf bindings for Auki schemas."
requires-python = ">=3.10"
dependencies = ["protobuf>=5"]
TOML

cat > "$pkg/README.md" <<'MD'
# auki-proto

Generated Python protobuf bindings from root `proto/auki` schemas.

This directory is generated locally by `scripts/generate-python-proto.sh` and is ignored by git. Do not edit or commit generated files from this directory.
MD

protoc \
  -I proto \
  --python_out "$pkg" \
  proto/auki/*.proto

find "$pkg" -type d -exec sh -c 'touch "$0/__init__.py"' {} \;

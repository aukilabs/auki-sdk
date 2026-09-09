#!/usr/bin/env bash
set -euo pipefail

SWIFT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SWIFT_DIR"

if ! command -v mint >/dev/null 2>&1; then
  echo "Mint is required: brew install mint" >&2
  exit 1
fi

mint run xcodegen generate --spec project.yml

#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

just generate-rust-proto
bash scripts/generate-javascript-proto.sh
bash scripts/generate-swift-proto.sh
bash scripts/generate-python-proto.sh

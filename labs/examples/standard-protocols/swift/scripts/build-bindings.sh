#!/usr/bin/env bash
set -euo pipefail

SWIFT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_ROOT="$(cd "$SWIFT_DIR/../../../.." && pwd)"

"$WORKSPACE_ROOT/core/bindings/swift/auki-sdk-swift/build-xcframework.sh"

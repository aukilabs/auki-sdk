#!/usr/bin/env bash
# Regenerate the betterproto-generated Python code from `auki-datatypes/proto/*.proto`.
#
# Run this any time a `.proto` file changes upstream. The output overwrites
# `auki_datatypes/auki/*.py`; commit the diff alongside the `.proto` change.
#
# Cross-language byte equality is enforced by the locked-vector tests in
# `tests/test_locked_vectors.py` — those tests will trip on regen if the
# `.proto` change accidentally breaks wire compat with the Rust prost
# encoder. Run `pytest` after regen to confirm.
#
# Requires:
#   - `protoc` on PATH (homebrew: `brew install protobuf`)
#   - The `[regen]` extras: `pip install -e .[regen]`
set -euo pipefail

cd "$(dirname "$0")"

PROTO_DIR="../../../crates/auki-datatypes/proto"
OUT_DIR="auki_datatypes"

if ! command -v protoc >/dev/null 2>&1; then
    echo "error: protoc not found on PATH" >&2
    echo "       install via: brew install protobuf  (or your distro equivalent)" >&2
    exit 1
fi

if ! command -v protoc-gen-python_betterproto >/dev/null 2>&1; then
    echo "error: protoc-gen-python_betterproto not found on PATH" >&2
    echo "       install via: pip install -e .[regen]" >&2
    exit 1
fi

# Find every .proto in the source dir; pass them all to a single protoc
# invocation so betterproto can resolve cross-file references.
protoc \
    -I "$PROTO_DIR" \
    --python_betterproto_out="$OUT_DIR" \
    "$PROTO_DIR"/*.proto

# protoc with --python_betterproto_out drops a top-level
# `__init__.py` next to the `auki/` package; we already maintain a
# richer `__init__.py` (with re-exports), so revert any change protoc
# made to it.
git checkout -- "$OUT_DIR/__init__.py" 2>/dev/null || true

echo ""
echo "Regenerated $(ls "$OUT_DIR"/auki/*.py | wc -l | tr -d ' ') Python module(s) under $OUT_DIR/auki/."
echo "Run 'pytest' to verify cross-language byte equality with the locked Rust vectors."

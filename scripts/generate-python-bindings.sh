#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <crate>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

crate="$1"
lib_name="${crate//-/_}"
package_dir="bindings/python/${crate}"
generated_dir="${package_dir}/generated"

case "$(uname -s)" in
  Darwin) lib_ext="dylib" ;;
  Linux) lib_ext="so" ;;
  MINGW*|MSYS*|CYGWIN*) lib_ext="dll" ;;
  *) echo "unsupported host OS for Python binding generation: $(uname -s)" >&2; exit 1 ;;
esac

cargo build -p "$crate"

library="target/debug/lib${lib_name}.${lib_ext}"
if [[ ! -f "$library" ]]; then
  echo "expected UniFFI library not found: $library" >&2
  echo "crate packages with custom [lib].name are not supported by this script yet" >&2
  exit 1
fi

mkdir -p "$generated_dir"
rm -f \
  "$generated_dir/${lib_name}.py" \
  "$generated_dir/${lib_name}.pyi" \
  "$generated_dir/lib${lib_name}.${lib_ext}"

cargo run -p "$crate" --features cli --bin uniffi-bindgen -- generate \
  --library "$library" \
  --language python \
  --out-dir "$generated_dir"

cp "$library" "$generated_dir/"

echo "Generated Python bindings in $generated_dir"

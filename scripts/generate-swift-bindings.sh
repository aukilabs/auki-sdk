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
package_dir="bindings/swift/${crate}"
generated_dir="${package_dir}/generated"

case "$(uname -s)" in
  Darwin) lib_ext="dylib" ;;
  Linux) lib_ext="so" ;;
  MINGW*|MSYS*|CYGWIN*) lib_ext="dll" ;;
  *) echo "unsupported host OS for Swift binding generation: $(uname -s)" >&2; exit 1 ;;
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
  "$package_dir/${lib_name}.swift" \
  "$package_dir/${lib_name}FFI.h" \
  "$package_dir/${lib_name}FFI.modulemap" \
  "$generated_dir/${lib_name}.swift" \
  "$generated_dir/${lib_name}FFI.h" \
  "$generated_dir/${lib_name}FFI.modulemap" \
  "$generated_dir/lib${lib_name}.${lib_ext}"

cargo run -p "$crate" --features cli --bin uniffi-bindgen -- generate \
  --library "$library" \
  --language swift \
  --out-dir "$generated_dir"

cp "$library" "$generated_dir/"

echo "Generated Swift package sources in $generated_dir"

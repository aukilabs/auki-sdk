#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <crate> [target ...]" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

crate="$1"
shift

lib_name="${crate//-/_}"
package_dir="bindings/python/${crate}"
module_dir="${package_dir}/${lib_name}"
host_target="$(rustc -vV | sed -n 's/^host: //p')"

if [[ ! -f "$module_dir/__init__.py" ]]; then
  echo "missing generated Python package: $module_dir/__init__.py" >&2
  echo "run scripts/generate-python-bindings.sh first" >&2
  exit 1
fi

if [[ $# -eq 0 ]]; then
  if [[ -n "${AUKI_PYTHON_NATIVE_TARGETS:-}" ]]; then
    read -r -a targets <<< "$AUKI_PYTHON_NATIVE_TARGETS"
    set -- "${targets[@]}"
  elif [[ "$(uname -s)" == "Darwin" ]]; then
    set -- "$host_target" aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
  else
    set -- "$host_target" x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
  fi
fi

seen_targets=" "

for target in "$@"; do
  if [[ "$seen_targets" == *" $target "* ]]; then
    continue
  fi
  seen_targets="${seen_targets}${target} "

  case "$target" in
    *-linux-*) lib_file="lib${lib_name}.so"; builder="${CROSS:-cross}" ;;
    *-apple-darwin) lib_file="lib${lib_name}.dylib"; builder="cargo" ;;
    *-pc-windows-*) lib_file="${lib_name}.dll"; builder="${CROSS:-cross}" ;;
    *) echo "unsupported Python native-library target: $target" >&2; exit 1 ;;
  esac

  if ! command -v "$builder" >/dev/null 2>&1; then
    echo "required build tool not found for $target: $builder" >&2
    exit 1
  fi

  if [[ "$builder" == "${CROSS:-cross}" && "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]]; then
    export CROSS_CONTAINER_OPTS="${CROSS_CONTAINER_OPTS:---platform linux/amd64}"
  fi

  "$builder" build --release -p "$crate" --target "$target"

  library="target/${target}/release/${lib_file}"
  if [[ ! -f "$library" ]]; then
    echo "expected UniFFI library not found: $library" >&2
    exit 1
  fi

  target_dir="${module_dir}/native/${target}"
  mkdir -p "$target_dir"
  cp "$library" "$target_dir/"
  echo "Copied $library to $target_dir/"
done

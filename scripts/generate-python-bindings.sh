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
module_dir="${package_dir}/${lib_name}"
host_target="$(rustc -vV | sed -n 's/^host: //p')"
generated_dir="$(mktemp -d "${TMPDIR:-/tmp}/${crate}.python-bindings.XXXXXX")"
trap 'rm -rf "$generated_dir"' EXIT

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

mkdir -p "$module_dir/native/${host_target}"
rm -f \
  "$module_dir/__init__.py" \
  "$module_dir/native/${host_target}/lib${lib_name}.${lib_ext}"

cargo run -p "$crate" --features cli --bin uniffi-bindgen -- generate \
  --library "$library" \
  --language python \
  --out-dir "$generated_dir"

python3 scripts/patch-uniffi-python-loader.py "$generated_dir/${lib_name}.py" "$lib_name"

cp "$generated_dir/${lib_name}.py" "$module_dir/__init__.py"
cp "$library" "$module_dir/native/${host_target}/"

if [[ ! -f "$package_dir/pyproject.toml" ]]; then
  cat > "$package_dir/pyproject.toml" <<EOF
[build-system]
requires = ["setuptools>=69", "wheel"]
build-backend = "setuptools.build_meta"

[project]
name = "${crate}"
version = "0.0.0"
description = "UniFFI-generated Python bindings for ${crate}."
readme = "README.md"
license = { text = "MIT" }
authors = [{ name = "Auki Labs Limited" }]
requires-python = ">=3.8"
classifiers = [
    "Programming Language :: Python :: Implementation :: CPython",
    "Operating System :: POSIX :: Linux",
    "Operating System :: MacOS",
    "License :: OSI Approved :: MIT License",
]

[project.urls]
Repository = "https://github.com/aukilabs/auki-sdk"

[tool.setuptools]
packages = ["${lib_name}"]

[tool.setuptools.package-data]
${lib_name} = ["native/*/*"]
EOF
fi

if [[ ! -f "$package_dir/setup.py" ]]; then
  cat > "$package_dir/setup.py" <<EOF
from setuptools import setup
from setuptools.dist import Distribution


class BinaryDistribution(Distribution):
    def has_ext_modules(self):
        return True


setup(distclass=BinaryDistribution, zip_safe=False)
EOF
fi

echo "Generated Python package in $package_dir"
echo "Included host library for $host_target"

bash scripts/build-python-native-libs.sh "$crate"

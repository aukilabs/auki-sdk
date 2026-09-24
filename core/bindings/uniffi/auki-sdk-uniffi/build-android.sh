#!/usr/bin/env bash
# Build libauki_sdk_uniffi.so for Android ABIs and generate UniFFI Kotlin.
#
# Prerequisites:
#   ANDROID_NDK_HOME or ANDROID_NDK_ROOT (NDK r27+ recommended)
#   rustup targets are installed by this script when missing:
#     aarch64-linux-android armv7-linux-androideabi
#     x86_64-linux-android i686-linux-android
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../../../.." && pwd)"
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="$WORKSPACE_ROOT/target"
fi
LIB_NAME="auki_sdk_uniffi"
OUT="$CRATE_DIR/target-android"
BINDINGS="$OUT/bindings"
API="${ANDROID_API:-24}"

NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [[ -z "$NDK" || ! -d "$NDK" ]]; then
  echo "Set ANDROID_NDK_HOME or ANDROID_NDK_ROOT to an installed NDK (r27+)." >&2
  exit 1
fi

case "$(uname -s)" in
  Darwin)
    if [[ -d "$NDK/toolchains/llvm/prebuilt/darwin-arm64" ]]; then
      HOST_TAG="darwin-arm64"
    elif [[ -d "$NDK/toolchains/llvm/prebuilt/darwin-x86_64" ]]; then
      HOST_TAG="darwin-x86_64"
    else
      echo "NDK has no Darwin LLVM prebuilt under $NDK/toolchains/llvm/prebuilt" >&2
      exit 1
    fi
    HOST_LIB_EXT="dylib"
    ;;
  Linux)
    HOST_TAG="linux-x86_64"
    HOST_LIB_EXT="so"
    ;;
  *)
    echo "Unsupported host $(uname -s)" >&2
    exit 1
    ;;
esac

TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
if [[ ! -x "$TOOLCHAIN/bin/llvm-ar" ]]; then
  echo "NDK toolchain is incomplete: $TOOLCHAIN" >&2
  exit 1
fi

configure_target() {
  local rust_target="$1"
  local clang_triple="$2"
  local clang="$TOOLCHAIN/bin/${clang_triple}${API}-clang"
  local key
  key="$(printf '%s' "$rust_target" | tr '[:lower:]-' '[:upper:]_')"
  if [[ ! -x "$clang" ]]; then
    echo "Missing NDK compiler: $clang" >&2
    exit 1
  fi
  export "CC_${rust_target//-/_}=$clang"
  export "CXX_${rust_target//-/_}=${clang}++"
  export "AR_${rust_target//-/_}=$TOOLCHAIN/bin/llvm-ar"
  export "CARGO_TARGET_${key}_LINKER=$clang"
  export "CARGO_TARGET_${key}_AR=$TOOLCHAIN/bin/llvm-ar"
}

# 64-bit Android devices and emulators may use 16 KB pages.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 ${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-}"
export CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 ${CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS:-}"

configure_target aarch64-linux-android aarch64-linux-android
configure_target armv7-linux-androideabi armv7a-linux-androideabi
configure_target x86_64-linux-android x86_64-linux-android
configure_target i686-linux-android i686-linux-android

for rust_target in \
  aarch64-linux-android \
  armv7-linux-androideabi \
  x86_64-linux-android \
  i686-linux-android
do
  if ! rustup target list --installed | grep -qx "$rust_target"; then
    rustup target add "$rust_target"
  fi
done

rm -rf "$OUT"
mkdir -p "$BINDINGS"
cd "$WORKSPACE_ROOT"

cargo build --locked --release -p auki-sdk-uniffi --features cli,standard-protocols

HOST_LIB="$CARGO_TARGET_DIR/release/lib${LIB_NAME}.${HOST_LIB_EXT}"
if [[ ! -f "$HOST_LIB" ]]; then
  echo "Host library missing: $HOST_LIB" >&2
  exit 1
fi

cargo run --locked --release --features cli,standard-protocols \
  -p auki-sdk-uniffi --bin uniffi-bindgen -- generate \
  --library "$HOST_LIB" \
  --language kotlin \
  --out-dir "$BINDINGS" \
  --no-format

copy_abi() {
  local rust_target="$1"
  local abi="$2"
  local built="$CARGO_TARGET_DIR/$rust_target/release/lib${LIB_NAME}.so"
  cargo build --locked --release -p auki-sdk-uniffi \
    --features standard-protocols \
    --target "$rust_target"
  if [[ ! -f "$built" ]]; then
    echo "Android library missing: $built" >&2
    exit 1
  fi
  mkdir -p "$OUT/jniLibs/$abi"
  cp "$built" "$OUT/jniLibs/$abi/lib${LIB_NAME}.so"
}

copy_abi aarch64-linux-android arm64-v8a
copy_abi armv7-linux-androideabi armeabi-v7a
copy_abi x86_64-linux-android x86_64
copy_abi i686-linux-android x86

echo "Android libraries: $OUT/jniLibs"
echo "Kotlin bindings: $BINDINGS"

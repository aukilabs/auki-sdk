#!/usr/bin/env bash
# Build auki-sdk-uniffi for Android and copy UniFFI Kotlin plus libauki_sdk_uniffi.so
# into the Expo module. Generated outputs are gitignored, same as the iOS XCFramework.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UNIFFI_CRATE="$(cd "$ROOT/../uniffi/auki-sdk-uniffi" && pwd)"
ANDROID="$ROOT/android/src/main"
JNI="$ANDROID/jniLibs"
KOTLIN="$ANDROID/java/uniffi/auki_sdk_uniffi"

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="$(cd "$ROOT/../../.." && pwd)/target"
fi

bash "$UNIFFI_CRATE/build-android.sh"

rm -rf "$JNI" "$KOTLIN"
mkdir -p "$KOTLIN"
cp -R "$UNIFFI_CRATE/target-android/jniLibs/." "$JNI/"
cp "$UNIFFI_CRATE/target-android/bindings/uniffi/auki_sdk_uniffi/"*.kt "$KOTLIN/"
python3 "$ROOT/scripts/patch-uniffi-kotlin.py" "$KOTLIN/auki_sdk_uniffi.kt"

for abi in arm64-v8a armeabi-v7a x86_64 x86; do
  test -f "$JNI/$abi/libauki_sdk_uniffi.so"
done
test -f "$KOTLIN/auki_sdk_uniffi.kt"
grep -q 'auki_sdk_uniffi' "$KOTLIN/auki_sdk_uniffi.kt"

echo "synced libauki_sdk_uniffi.so → $JNI"
echo "synced UniFFI Kotlin → $KOTLIN (CARGO_TARGET_DIR=$CARGO_TARGET_DIR)"

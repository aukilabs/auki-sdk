#!/usr/bin/env bash
# Generate the ignored Expo Android app and build a debug APK.
set -euo pipefail
domain_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
domain_expo="$domain_root/core/bindings/expo"
domain_example="$domain_expo/example"
domain_output="$domain_root/target/domain-data-expo-android-app"
export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"

java_major() {
  "$1" -version 2>&1 | sed -n 's/.* version "\([0-9][0-9]*\).*/\1/p' | head -1
}
java_is_gradle8() {
  local major
  major="$(java_major "$1")"
  [[ -n "$major" && "$major" -ge 17 && "$major" -le 23 ]]
}
if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/java" ]] && java_is_gradle8 "$JAVA_HOME/bin/java"; then
  :
elif home="$(/usr/libexec/java_home -v 21 2>/dev/null)" || home="$(/usr/libexec/java_home -v 17 2>/dev/null)"; then
  export JAVA_HOME="$home"
elif [[ -x "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java" ]] &&
  java_is_gradle8 "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java"; then
  export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
else
  printf 'Install JDK 17 or 21 and set JAVA_HOME. Expo Gradle 8.14 cannot run on JDK 25.\n' >&2
  exit 1
fi

mkdir -p "$domain_output"
domain_package_backup="$domain_output/example-package.json.backup"
cp "$domain_example/package.json" "$domain_package_backup"
restore_package() { cp "$domain_package_backup" "$domain_example/package.json"; }
trap restore_package EXIT INT TERM

cd "$domain_expo"
npm run build
npm run typecheck
bash scripts/sync-android-jni.sh
cd "$domain_example"
npm ci --ignore-scripts
EXPO_PUBLIC_AUKI_DOMAIN_DATA_TEST=1 CI=1 EXPO_NO_TELEMETRY=1 \
  npx expo prebuild --platform android --no-install --clean
restore_package
cd android
EXPO_PUBLIC_AUKI_DOMAIN_DATA_TEST=1 CI=1 EXPO_NO_TELEMETRY=1 \
  ./gradlew :app:assembleDebug
domain_apk="$(find app/build/outputs/apk/debug -name '*.apk' -print -quit)"
test -n "$domain_apk"
cp "$domain_apk" "$domain_output/app-debug.apk"
printf 'Expo Android Domain data app: %s\n' "$domain_output/app-debug.apk"

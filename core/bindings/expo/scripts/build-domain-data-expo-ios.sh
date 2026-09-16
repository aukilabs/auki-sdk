#!/usr/bin/env bash
# Generate the ignored Expo iOS app and build a Release simulator artifact.
set -euo pipefail
domain_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
domain_expo="$domain_root/core/bindings/expo"
domain_example="$domain_expo/example"
domain_output="$domain_root/target/domain-data-expo-ios-app"
mkdir -p "$domain_output"
domain_package_backup="$domain_output/example-package.json.backup"
cp "$domain_example/package.json" "$domain_package_backup"
restore_package() { cp "$domain_package_backup" "$domain_example/package.json"; }
trap restore_package EXIT INT TERM

cd "$domain_expo"
npm run build
npm run typecheck
bash scripts/sync-ios-xcframework.sh
cd "$domain_example"
npm ci --ignore-scripts
EXPO_PUBLIC_AUKI_DOMAIN_DATA_TEST=1 CI=1 EXPO_NO_TELEMETRY=1 \
  npx expo prebuild --platform ios --no-install --clean
restore_package
cd ios
pod install
EXPO_PUBLIC_AUKI_DOMAIN_DATA_TEST=1 CI=1 EXPO_NO_TELEMETRY=1 \
  xcodebuild -quiet -workspace ZitadelHandoffTest.xcworkspace \
    -scheme ZitadelHandoffTest -configuration Release -sdk iphonesimulator \
    -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$domain_output/DerivedData" CODE_SIGNING_ALLOWED=NO build
printf 'Expo iOS Domain data app: %s\n' \
  "$domain_output/DerivedData/Build/Products/Release-iphonesimulator/ZitadelHandoffTest.app"

# ZITADEL Expo handoff test app

This deliberately small Web/iOS app exercises the **actual Expo module**, not a
mock native module. It uses synthetic loopback IdP/API/DDS responses from
`test-support/zitadel-bindings-fixture.mjs`; it is not Z10's real-service chain or
a live ZITADEL login UI. Product PKCE/storage integration is illustrated in the
[package README](../README.md).

The app pins Expo 54.0.37, React 19.1.0 and React Native 0.81.5 (Expo 54's compatible
versions). Existing package peer dependencies are not narrowed. Native test
storage uses Expo SecureStore's real simulator Keychain. Browser test storage
uses one IndexedDB transaction with strict durability. Both store the full JSON
payload as one record/item and await completion. These are **synthetic fixtures**;
choose your real app's browser secret-storage/XSS policy separately. The test
bundle allows cleartext HTTP solely for loopback fixtures; do not copy that ATS
setting to a production app.

## Setup

From the SDK root:

```sh
cd bindings/expo
npm ci --ignore-scripts
npm run build
npm run typecheck
bash scripts/sync-ios-xcframework.sh
cd example
npm ci --ignore-scripts
CI=1 npx expo prebuild --platform ios --no-install
cd ios
pod install
```

For a clean SDK checkout, `npm run build` creates generated Wasm declarations
before the first typecheck. Native sources, Pods, XCFrameworks, app projects, and
build products are generated/ignored; no private credentials or signing account
is required. `npm ci` reports existing development-tree advisories; these tests
do not perform an unrelated dependency upgrade or audit fix.

## Web runtime

From the SDK root:

```sh
node --test bindings/expo/scripts/prepare-metro-wasm.test.cjs
bash test-support/run-zitadel-expo-web.sh
```

Requires Chrome and free loopback ports 18111/18113. The script builds the module,
starts Metro and the synthetic fixture, runs seven host cases through actual
Chrome/Playwright CLI 0.1.19, freezes the real page for five seconds during a
pending save, resumes, and verifies the final results. It owns/cleans only its
named browser and server processes. Artifacts: `output/playwright/zitadel-z09-web-*`.

## iOS runtime

Requires Xcode, CocoaPods, and an installed iOS Simulator runtime. Build from
`bindings/expo/example` (the paths below refer to the SDK's ignored target folder):

```sh
CI=1 EXPO_NO_TELEMETRY=1 xcodebuild \
  -workspace ios/ZitadelHandoffTest.xcworkspace -scheme ZitadelHandoffTest \
  -configuration Release -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath ../../../target/zitadel-z09-ios/DerivedData \
  -jobs 2 CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES build
```

The simulator uses local ad-hoc signing (`-`), not a developer certificate or
provisioning profile. Do not disable signing: the test uses real Keychain storage,
which needs an application identifier in the simulator's generated entitlements.
See Apple's [Keychain entitlement documentation](https://developer.apple.com/documentation/Security/sharing-access-to-keychain-items-among-a-collection-of-apps).

Then, from the SDK root:

```sh
bash test-support/run-zitadel-expo-ios.sh
```

This creates a separately named simulator (default: iPhone 17/iOS 26.2), installs
the Release/Hermes app with its embedded JS bundle, and runs the same seven cases.
It backgrounds the app by opening Safari on **that simulator only**, waits five
seconds and foregrounds it; the app asserts real background/active AppState
transitions before completing its pending save. Caller deadlines may elapse
during suspension: retry still observes the same session-owned generation.
The script terminates its test app and shuts down a simulator it created, but
retains the device for inspection. Override `ZITADEL_SIMULATOR_RUNTIME` for another
installed runtime, or set `ZITADEL_SIMULATOR_UDID` for an already booted, task-owned
device whose name starts with `Zitadel Z09`. Supplied devices are not shut down
automatically. Results/screenshots: `target/zitadel-z09-ios/run-*`.

Run Web/iOS sequentially because they share fixture port 18111. Cases cover
zero-auth-I/O import, real acknowledged storage, concurrent callers, rotation,
rejection/retry of the same generation, stale ACK rejection, restart from storage,
startup failure, Domain denial, missing Promise, typed terminal/configuration
errors, logout with a pending write, and suspension/resume. The test credential
item/record is cleared after completed session close.

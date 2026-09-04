# `@aukilabs/auki-sdk-expo`

Expo module for authenticated Auki peers.

| Platform | Backend |
|----------|---------|
| **Web** | [`auki-sdk-web`](../web/auki-sdk-web) via `wasm-pack` |
| **iOS** | [`auki-sdk-swift`](../swift/auki-sdk-swift) XCFramework |
| **Android** | no-op (all methods throw) |

Peyote (and other Expo apps) should depend on this package and call the JS API only — never import raw `pkg/` or UniFFI paths.

## Build

Prerequisites: Rust, `wasm-pack` 0.13.1, `wasm32-unknown-unknown`, Node 20.19+.

```sh
cd bindings/expo
chmod +x scripts/*.sh
npm install
npm run build:wasm   # → src/web/generated
npx tsc && node scripts/copy-web-artifacts.cjs   # → build/ (+ build/web/generated)
# or: npm run build
```

iOS XCFramework (optional until you link the pod into an app):

```sh
./scripts/sync-ios-xcframework.sh   # builds auki-sdk-swift and copies into ios/Frameworks/
```

## JS API (handle-based)

Sessions and peers are opaque string handles so web Wasm objects and iOS UniFFI objects share one surface:

```ts
import AukiSdkExpo from "@aukilabs/auki-sdk-expo";

// Temporary until upstream Zitadel→DDS auth lands — then swap to product login.
const session = await AukiSdkExpo.loginDev(email, password);
const domains = await AukiSdkExpo.accessibleDomains(session);
const peer = await AukiSdkExpo.startPeerWithDiscovery(
  session,
  domains[0].id,
  "DiscoverOnly",
);
const candidates = await AukiSdkExpo.discoverProtocol(peer, "/auki/info/1.0.0");
await AukiSdkExpo.shutdown(peer);
```

| Method | Web | iOS | Android |
|--------|-----|-----|---------|
| `loginDev` | yes | yes | throws |
| `accessibleDomains` / `startPeer*` / `discover*` | yes | yes | throws |
| `infoFetchExact` / catalog / registry / blob | yes | yes | throws |
| `stream*` | yes | yes | throws |

## Metro (consumer)

- `assetExts` must include `wasm` (Expo / peyote already do).
- Add this package path to `watchFolders` when using a `file:` / path dependency.
- `unstable_enableSymlinks: true` if the SDK is symlinked into the monorepo.

Example dep:

```json
"@aukilabs/auki-sdk-expo": "file:../../auki-sdk/bindings/expo"
```

## Auth note

Product auth (Zitadel → DDS) is expected upstream. Until that lands, consumers use `loginDev` for local/dev fleet smoke only.

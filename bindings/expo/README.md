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
bash scripts/sync-ios-xcframework.sh   # builds and copies into ios/AukiSDK.xcframework
```

## JS API (handle-based)

Sessions and peers are opaque string handles so web Wasm objects and iOS UniFFI objects share one surface:

```ts
import AukiSdkExpo from "@aukilabs/auki-sdk-expo";

// Existing password login remains available.
const session = await AukiSdkExpo.loginDev(email, password);
const domains = await AukiSdkExpo.accessibleDomains(session);
const peer = await AukiSdkExpo.startPeerWithDiscovery(
  session,
  selectedDomainId, // explicit selection from domains
  "DiscoverOnly",
);
const candidates = await AukiSdkExpo.discoverProtocol(peer, "/auki/info/1.0.0");
await AukiSdkExpo.shutdown(peer);
// Also await closeSession(session) before clearing any session storage.
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

The build adapts wasm-bindgen's unused default `import.meta.url` fallback for
Metro's classic client bundle. The SDK supplies the real Wasm asset URL
explicitly and lazily initializes it; generated output must not be hand-edited.
The [local example's Metro configuration](example/metro.config.js) also ensures
one React/Expo runtime across a local SDK link without disabling nested dependencies.

Example dep:

```json
"@aukilabs/auki-sdk-expo": "file:../../auki-sdk/bindings/expo"
```

## ZITADEL login handoff

The host owns PKCE login and secure persistence. After login, stop competing
refresh in your OAuth library, other tabs, and other processes. Import only once
per grant. `issuer` and `clientId` come from trusted app configuration. The SDK
owns refresh after handoff; its five-field payload also accepts opaque access
tokens and an unknown initial expiry.

```ts
import AukiSdkExpo, { importZitadelSession, closeSession } from '@aukilabs/auki-sdk-expo';

const credentials = await yourHostPkceLogin(); // accessToken, refreshToken,
// clientId, issuer, accessTokenExpiresAt (RFC 3339 text or null/undefined).
await yourSecureStore.saveAtomically(credentials);
const session = await importZitadelSession(credentials, async replacement => {
  await yourSecureStore.saveAtomically({
    accessToken: replacement.exposeAccessToken(),
    refreshToken: replacement.exposeRefreshToken(),
    clientId: replacement.clientId,
    issuer: replacement.issuer,
    accessTokenExpiresAt: replacement.accessTokenExpiresAt,
  });
});
// Retain session in app state BEFORE the first auth operation.
// The application supplies selectedDomainId; no ZITADEL Domain listing in v1.
const peer = await AukiSdkExpo.startPeer(session, selectedDomainId);
// Observe AukiSdkExpo.waitStopped(peer) for terminal peer failures.
await AukiSdkExpo.shutdown(peer);
await closeSession(session);
await yourSecureStore.clear();
```

`importZitadelSession(credentials, store, {apiBaseUrl, ddsBaseUrl, dmsBaseUrl})`
accepts explicit service bases; omitting the last argument uses SDK development
defaults. DDS must run and be configured for direct ZITADEL P2P admission; the
new SDK does not use API's legacy token exchange. Building this package does not
deploy or upgrade shared services. `accessibleDomains()` remains available for
password sessions, but returns `configuration` for ZITADEL sessions.
Import may await Web Wasm loading, but returns a retained session ID before any
auth request/rotation. Startup failures do not discard that ID or its replacement
credentials. `closeSession` also works for password sessions.

The storage callback must return a Promise. Resolve only after the complete
replacement is durably, atomically persisted. On success **or rejection**, all
writes started by that call must have settled: no detached writes, no reentry
into the same session, and no event-only persistence. The internal Expo bridge
sends token-free request IDs, explicitly retrieves a snapshot, then sends a
success/failure ACK **after** the host Promise settles. An unrelated/stale ACK
cannot release the pending save. Do not call underscore-prefixed bridge methods
or subscribe to internal storage events yourself.

Thrown errors, rejected Promises and missing Promises become redacted
`persistence` failures. Retry using the **same session ID** to save the retained
generation without refreshing again. `closeSession` drains any outstanding write
and its ACK before returning; only then remove durable credentials. Keep the JS
runtime and storage listener alive until it completes. If the bridge disappears
or a host write never settles, a safe completed logout cannot be promised. Do
not add a timeout that treats an outstanding write as completed. Stop owned peers
separately; closing is not immediate remote-token revocation.

Errors expose the same `code` on Web and iOS: `authentication_required`,
`configuration`, `authorization_denied`, `persistence`, `transient`, `cancelled`,
or `closed`. Sign in again for authentication-required, fix configuration errors,
choose a readable Domain for denial, and retry persistence/transient failures on
the retained handle. `waitStopped` preserves terminal auth classifications too.
Other protocol/transport failures retain their existing errors.

Snapshots redact logging/JSON; token extraction is explicit. The original host
payload and extracted strings remain secrets and must not be logged. Keep RFC
3339 expiry strings intact to preserve nanoseconds. Human subject strings remain
exact, case-sensitive UTF-8, never trimmed/normalized or coerced to UUIDs.

Web and iOS are implemented; Android remains unsupported. See the
[minimal runtime example](example/README.md) for reproducible Web/iOS host tests.

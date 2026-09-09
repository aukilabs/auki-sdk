# Auki networking for Expo

`@aukilabs/auki-sdk-expo` exposes session and peer handles on Web and iOS.
Android is currently unimplemented.

From the SDK repository root, with the
[platform toolchains](../../../docs/reference/networking.md#platforms-and-installation)
installed:

~~~sh
cd core/bindings/expo
npm ci --ignore-scripts
npm run build
~~~

For iOS, also run `bash scripts/sync-ios-xcframework.sh` from this directory.
Add this package to your app as a local dependency. For Metro Web, include
`wasm` in `assetExts` and the package in `watchFolders`; see the
[example Metro configuration](example/metro.config.js).

## Import an existing login

Your host supplies credentials from PKCE login and an atomic secure store.
Stop competing token refresh before handing the session to the SDK.

~~~ts
import AukiSdkExpo, {
  importZitadelSession, closeSession,
} from "@aukilabs/auki-sdk-expo";

const session = await importZitadelSession(credentials, async replacement => {
  await secureStore.saveAtomically({
    accessToken: replacement.exposeAccessToken(),
    refreshToken: replacement.exposeRefreshToken(),
    clientId: replacement.clientId,
    issuer: replacement.issuer,
    accessTokenExpiresAt: replacement.accessTokenExpiresAt,
  });
});
// Retain session in app state before starting any authentication operation.
const peer = await AukiSdkExpo.startPeer(session, selectedDomainId);

// On logout, after closing any application endpoints:
await AukiSdkExpo.shutdown(peer);
await closeSession(session);
await secureStore.clear();
~~~

`credentials`, `secureStore`, and `selectedDomainId` belong to your app.
Imported sessions require a known Domain ID. See
[authentication](../../../docs/how-to/authenticate.md) for persistence failures,
session ownership, and service requirements.

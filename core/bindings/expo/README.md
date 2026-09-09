# Auki networking for Expo

Use `@aukilabs/auki-sdk-expo` to connect an Expo app on Web or iOS.
Android is not implemented.

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

Your app supplies `credentials` from PKCE login and a known `selectedDomainId`.
In this example, `secureStore` is your storage code: `saveAtomically` saves all
credentials together, and `clear` deletes them. Both methods must finish their
writes before returning.

Stop your app's token refresh loop before importing; the SDK will refresh the
tokens. Keep the returned session in app state, including when peer startup
fails, so you can retry with it.

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
// Save session in app state before starting the peer.
const peer = await AukiSdkExpo.startPeer(session, selectedDomainId);

// On logout, after closing your message handlers:
await AukiSdkExpo.shutdown(peer);
await closeSession(session);
await secureStore.clear();
~~~

See [authentication](../../../docs/how-to/authenticate.md#reuse-a-zitadel-login)
for storage failures and DDS requirements.

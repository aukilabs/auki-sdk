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

## Sign in and select a Domain

Password login returns a session handle shared by Domain discovery, Domain data,
and any peers the app starts. Persist one random client ID for the app
installation and reuse it across logins.

~~~ts
import AukiSdkExpo, { closeSession, data, domains } from "@aukilabs/auki-sdk-expo";

const session = await AukiSdkExpo.loginDev(email, password, installationId);
try {
  const firstPage = await domains(session).list({ limit: 50, offset: 0 });
  if (!firstPage.domains.length) throw new Error("No accessible Domains");
  const selectedDomainId = firstPage.domains[0].id;
  const client = await data(session, selectedDomainId);
  try {
    const records = await client.list({ dataType: "my-app.report.v1" });
    if (records.length) showReport(await client.read(records[0].id));
  } finally {
    await client.close();
  }
} finally {
  await closeSession(session);
}
~~~

For another environment, use
`AukiSdkExpo.loginWithEnvironment(apiBase, ddsBase, dmsBase, email, password,
installationId)`. Keep all three endpoints aligned to the same deployment.

Domain listing is server-paged; totals may change between pages. Visibility in
the list does not imply read, write, or delete permission. Portal and pose reads
have their own permission checks. Imported ZITADEL sessions can use a known
Domain ID for data access, but cannot list Domains with the current provider
contract.

## Stream files with bounded callbacks

`readTo` awaits the destination before requesting the next chunk. `writeStream`
requests at most the supplied maximum from the source. Expo additionally caps
each native bridge message at 256 KiB; the SDK assembles those chunks into the
server's multipart part size. A callback that does not settle is cancelled after
the SDK's 30-second callback deadline; cancellation and client close also release
pending callbacks.

~~~ts
const client = await data(session, selectedDomainId);
const controller = new AbortController();
try {
  let offset = 0;
  const saved = await client.writeStream(
    { name: `report-${Date.now()}`, dataType: "my-app.report.v1" },
    source.size,
    async maximum => {
      const end = Math.min(source.size, offset + maximum);
      const chunk = new Uint8Array(await source.slice(offset, end).arrayBuffer());
      offset = end;
      return chunk;
    },
    { maxBytes: source.size },
    controller.signal,
  );

  await client.readTo(
    saved.id,
    async chunk => destination.write(chunk),
    { maxBytes: source.size },
    controller.signal,
  );
} finally {
  await client.close();
}
~~~

Uploads require a known nonzero size. A named multipart completion can replace
an existing name. Cancellation aborts and drains a known multipart session;
await the operation and `client.close()`. The application owns partial download
cleanup. See the [Domain data guide](../../../docs/how-to/domain-data.md) for
permission, conflict, timeout, and retry behavior.

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

## Checks

`npm run build` compiles the Web WASM backend and TypeScript package.
`npm run test:domain-data` runs the offline bridge lifecycle, backpressure,
cancellation, and awaited-close tests. The following local host tests use only
the loopback fixture in `test-support/domain-data-local-fixture.mjs`:

~~~sh
# Requires the example dependencies: cd example && npm ci --ignore-scripts
bash scripts/run-domain-data-expo-web.sh

# Generates ignored native files, builds a Release simulator app, then runs it.
bash scripts/build-domain-data-expo-ios.sh
bash scripts/run-domain-data-expo-ios.sh
~~~

The iOS build is written below the repository's
`target/domain-data-expo-ios-app/` directory. Both host runs cover password and
imported sessions, renewal and storage failure, paged listing, portal/pose
metadata, CRUD, bounded transfers, permission errors, cancellation, multipart
abort, close, and fixture cleanup.

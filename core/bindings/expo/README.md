# Auki networking for Expo

Use `@aukilabs/auki-sdk-expo` to connect an Expo app on Web, iOS, or Android.

From the SDK repository root, with the
[platform toolchains](../../../docs/reference/networking.md#platforms-and-installation)
installed:

~~~sh
cd core/bindings/expo
npm ci --ignore-scripts
npm run build
~~~

For iOS, also run `bash scripts/sync-ios-xcframework.sh` from this directory.
For Android, set `ANDROID_NDK_HOME` and run `bash scripts/sync-android-jni.sh`.
Both scripts build [`auki-sdk-uniffi`](../uniffi/auki-sdk-uniffi/README.md), the
shared native library `libauki_sdk_uniffi`. iOS gets Swift; Android gets Kotlin.
Web does not use that library. `npm run build` compiles the separate Wasm crate
[`auki-sdk-web`](../web/auki-sdk-web/README.md). Native outputs are gitignored, but
`.npmignore` still packs them for a local `file:` install and for `npm pack`.
After syncing, reinstall the package in the app
(`npm i @aukilabs/auki-sdk-expo`) so `node_modules` is not a stale copy. For
Metro Web, include `wasm` in `assetExts` and the package in `watchFolders`; see
the [example Metro configuration](example/metro.config.js).

## npm package

Pack from this directory. `npm pack` and `npm publish` run `prepare`, which
runs `npm run build` again. They do not rebuild the native libraries, so sync
those first when the tarball should include iOS and Android:

~~~sh
npm ci --ignore-scripts
npm run build
bash scripts/sync-ios-xcframework.sh
bash scripts/sync-android-jni.sh
npm pack --dry-run
~~~

The dry run should list `build/index.js`, `build/web/generated/`,
`ios/AukiSDK.xcframework/`, `android/src/main/jniLibs/`, and
`android/src/main/java/uniffi/`. It should not list `android/build/`,
`example/`, or `src/web/generated/`. `publishConfig.access` is `public`.
After `npm login` as an `@aukilabs` publisher, `npm publish` uploads
`@aukilabs/auki-sdk-expo`. Bump `version` for each upload.

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
have their own permission checks. Imported ZITADEL sessions support the default
own-Domain listing for a picker and can use a known Domain ID for data access.
The SDK strictly validates the ordinary service-token profile: human `user-access`
uses the deployed User Domain route, while App-shaped viewer grants are never
sent to that broader route and require the permission-scoped `purpose=p2p`
exchange. Organization and Domain Server filters, and portal-to-Domain
association queries, remain unsupported for imported sessions.
The [example picker helper](example/domain-data.js) requests one explicit page;
the UI can request another offset when the user asks for more.

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

## Submit and inspect DMS jobs

Use the existing session and an explicit Domain ID. This creates no peer and
does not start a polling loop:

~~~ts
import { jobs } from "@aukilabs/auki-sdk-expo";

const client = await jobs(session, selectedDomainId);
try {
  const spec = {
    label: "map update",
    tasks: [{
      label: "reconstruct",
      stage: "reconstruct",
      capability: "com.example.private/reconstruct/v1",
      mode: "dedicated" as const,
      inputsCids: [inputId],
    }],
  };
  const estimate = await client.estimate(spec);
  showEstimatedCredits(estimate.total); // Decimal text, not a JS number.
  const jobId = await client.submit(spec);
  const details = await client.get(jobId);
} finally {
  await client.close();
}
~~~

Edges refer to stage names. Custom capability strings pass through unchanged;
the SDK has no capability catalogue whitelist. `list` returns one DMS page and
an opaque `next_cursor`. Every operation accepts an optional `AbortSignal`.

If `submit` rejects with `kind === "submission_uncertain"`, DMS may have
accepted the job. Inspect existing jobs before choosing a recovery action;
automatic resubmission can duplicate work and charges. Authentication errors
retain codes such as `persistence`, and HTTP errors retain `status`.

For a deployment verified to support the idempotency contract, use
`await client.submitWithKey(spec, idempotencyKey, signal)`. Persist the key and immutable
specification before the first send and reuse both for recovery; no automatic
retry is added. An in-progress submission exposes the retry delay through `retryAfterSeconds`;
a plain HTTP 409 is a conflict. Older servers ignore keys, so verify the backend
rollout before enabling this method. Existing unkeyed `submit` behavior is unchanged.

See the [jobs reference](../../../docs/reference/jobs.md) for required write
authority, worker availability and provider limitations.

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

## Send a typed message

Open one receiver-owned Catalog `message_channel` through an exact advertised
route, send, and close. `messageSend` waits for the ACK. This surface does not
mount inbound Message or add `claim` / `connectRobot`.

~~~ts
const sender = await AukiSdkExpo.messageOpenExact(
  peer,
  { peerId, route },
  channelJson,
);
await AukiSdkExpo.messageSend(sender, "example.event", timestampNs, payloadBase64);
await AukiSdkExpo.messageClose(sender);
~~~

`channelJson` is a Catalog v3 `message_channel` row: `variant`, `owner_peer_id`,
`resource_id`, and `clock` (`peer_id`, `id`, `hash`). `timestampNs` is a decimal
integer string. `payloadBase64` may be empty.

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

# Same loopback cases on an Android emulator. Requires ANDROID_NDK_HOME, the Android SDK, and JDK 17 or 21.
bash scripts/build-domain-data-expo-android.sh
bash scripts/run-domain-data-expo-android.sh
~~~

The iOS build is written below the repository's
`target/domain-data-expo-ios-app/` directory. The Android APK is written to
`target/domain-data-expo-android-app/`. These host runs cover password and
imported sessions, renewal and storage failure, paged listing, portal/pose
metadata, CRUD, bounded transfers, permission errors, cancellation, multipart
abort, close, and fixture cleanup. The Android emulator reaches the host fixture through `adb reverse` on ports 18114, 18115, and 18111.
Plain HTTP is accepted only for a loopback host, so the example keeps `127.0.0.1`. The Domain Server
uses 18115 so that port can be reversed; other hosts leave it ephemeral.

`npm run test:jobs` runs the offline jobs bridge tests for custom capability
pass-through, paging, cancellation, ambiguous submission handling, persistence
codes, and awaited close.

## Fleet inventory and activity

```ts
import { fleet } from "@aukilabs/auki-sdk-expo";

const client = await fleet(sessionId, selectedDomainId);
try {
  const inventory = await client.list();
  const candidates = await client.computePool({ mode: "dedicated", capabilities: ["vendor.example/inspect/v7"] });
  console.log(inventory.machines, inventory.sources, candidates.complete);
} finally {
  await client.close();
}
```

Web, iOS, and Android share typed queries and snake_case snapshot results. Every
operation accepts an optional `AbortSignal`. `FleetError` retains `kind`, `code`, and
available `status`; credential persistence and cancellation stay actionable.
Rebuild/sync the XCFramework with its generated Swift wrappers for iOS, and run
`scripts/sync-android-jni.sh` before an Android build. Run `npm run test:fleet` for the bridge fixture and see the
[fleet reference](../../../docs/reference/fleet.md) for permissions, status and
provider limits. Closing a fleet client leaves the session usable.

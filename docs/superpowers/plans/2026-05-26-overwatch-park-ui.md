# Overwatch Park UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `examples/overwatch` use Park's operator UI from `../park/src/ui` while replacing Park's Rust HTTP/WebSocket backend with the SDK's generated browser JavaScript/WASM bindings.

**Architecture:** Copy Park's browser frontend as the UI source of truth, then replace only the backend-facing data modules with a browser-local SDK runtime. The runtime owns the generated `@aukilabs/auki-network`, `@aukilabs/auki-domain`, and `@aukilabs/auki-geometry` bindings, exposes Park-shaped cluster/daemon/catalog/stream state, and preserves Overwatch's current acceptance rule: Vite may serve static assets and proxy Discovery, but the app must not call app `/api/*` routes. Stream tiles consume SDK stream handles directly instead of polling Park's `/api/streams/*` cache endpoints.

**Tech Stack:** Vite, vanilla TypeScript, Tailwind CSS v4, Motion One, Three.js, generated SDK JavaScript/WASM packages, Discovery HTTP API, browser WebRTC data channels through generated SDK transport, Vitest, Playwright smoke tests.

---

## Assumptions

- "Park" means the primary Park UI under `../park/src/ui`, not the smaller `../park/apps/park-web-peer` demo.
- The first implementation targets visual parity and live browser-peer camera/point-cloud-style stream surfaces. Park backend-only controls remain present but are either SDK-backed or explicitly disabled in the browser runtime.
- `examples/overwatch` remains a web-only SDK example. It must not grow an app backend, HTTP control server, or app-specific signaling service.
- Discovery is still allowed as SDK infrastructure. Same-origin `/discovery` proxying is allowed for HTTPS/ngrok runs because it is not an app backend.
- Source copying is acceptable for the first pass. A shared package between Park and Overwatch can follow only after this example proves the backend replacement boundary.

## Non-Goals

- Do not modify the Park repo as part of this SDK repo change.
- Do not introduce a compatibility server that re-creates Park's `/api/*` surface.
- Do not reimplement Park's Rust K1 forward-kinematics backend in this pass.
- Do not make the browser peer a historical recording browser. Overwatch stays live-only.

## Recommended Approach

Use a source-level copy of `../park/src/ui` inside `examples/overwatch`, then replace the data modules that currently call `/api/*`.

Alternatives considered:

- Import Park UI directly from `../park/src/ui` with Vite aliases. This keeps fewer copied files, but it couples the SDK example to a sibling checkout and makes CI/reproducibility brittle.
- Build an in-browser `fetch` interceptor that answers Park's `/api/*` routes from SDK state. This preserves more source unchanged, but it hides the real boundary and risks shipping an accidental backend-shaped contract.
- Start from `../park/apps/park-web-peer`. This already has a browser peer contract, but it is not visually identical to Park's operations hub.

The copy-plus-data-adapter approach keeps the UI visually identical while making every backend dependency explicit in TypeScript.

## File Structure

Primary files copied from Park:

- Copy: `../park/src/ui/src/**` -> `examples/overwatch/src/**`
- Copy/adapt: `../park/src/ui/index.html` -> `examples/overwatch/index.html`
- Copy/adapt: `../park/src/ui/vite.config.ts` -> `examples/overwatch/vite.config.ts`
- Copy/adapt: `../park/src/ui/vitest.config.ts` -> `examples/overwatch/vitest.config.ts`
- Copy/adapt: `../park/src/ui/tsconfig.json` -> `examples/overwatch/tsconfig.json`

Overwatch SDK/runtime files:

- Modify: `examples/overwatch/scripts/stage-sdk.mjs`
- Modify: `examples/overwatch/package.json`
- Create: `examples/overwatch/src/sdk/runtime.ts`
- Create: `examples/overwatch/src/sdk/runtime.test.ts`
- Create: `examples/overwatch/src/sdk/demoSensors.ts`
- Create: `examples/overwatch/src/sdk/streamHub.ts`
- Create: `examples/overwatch/src/sdk/streamHub.test.ts`
- Keep/adapt: `examples/overwatch/src/sdk/createOverwatchPeer.ts`
- Keep/adapt: `examples/overwatch/src/sdk/contract.ts`

Park data modules to replace or adapt:

- Modify: `examples/overwatch/src/data/cluster.ts`
- Modify: `examples/overwatch/src/data/daemons.ts`
- Modify: `examples/overwatch/src/data/discovery.ts`
- Modify: `examples/overwatch/src/data/info.ts`
- Modify: `examples/overwatch/src/data/sensorLogs.ts`
- Modify: `examples/overwatch/src/data/registry.ts`
- Modify: `examples/overwatch/src/data/preview.ts`
- Modify: `examples/overwatch/src/data/pointcloudPreview.ts`
- Modify: `examples/overwatch/src/data/inspect.ts`
- Modify: `examples/overwatch/src/data/mic.ts`
- Modify: `examples/overwatch/src/data/recordings.ts`
- Modify: `examples/overwatch/src/data/settings.ts`

Views that need small browser-runtime guards:

- Modify: `examples/overwatch/src/views/robot/tiles/k1Pose.ts`
- Modify: `examples/overwatch/src/views/robot/tiles/world.ts`
- Modify only if imports break: `examples/overwatch/src/views/robot/index.ts`
- Modify only if browser-only copy text is needed: `examples/overwatch/src/views/directory/parkSelfCard.ts`

Tests and smoke:

- Modify: `examples/overwatch/scripts/smoke-two-browser.mjs`
- Create/keep copied Park tests under `examples/overwatch/src/**/*.test.ts`
- Remove or replace React-only tests from the old Overwatch shell.

Docs and propagation:

- Modify: `examples/overwatch/README.md`
- Modify: `examples/overwatch/changelog.md`
- Modify: `examples/changelog.md`
- Modify: `changelog.md`

---

## Task 1: Replace The React Shell With Park's UI Source

**Files:**

- Modify: `examples/overwatch/package.json`
- Modify: `examples/overwatch/package-lock.json`
- Modify: `examples/overwatch/index.html`
- Modify: `examples/overwatch/vite.config.ts`
- Modify: `examples/overwatch/vitest.config.ts`
- Modify: `examples/overwatch/tsconfig.json`
- Delete: `examples/overwatch/tailwind.config.ts`
- Delete: `examples/overwatch/postcss.config.js`
- Delete: `examples/overwatch/src/App.tsx`
- Delete: `examples/overwatch/src/App.test.tsx`
- Delete: `examples/overwatch/src/components/**`
- Delete: `examples/overwatch/src/state/appState.ts`
- Delete: `examples/overwatch/src/state/appState.test.ts`
- Replace: `examples/overwatch/src/main.tsx` with `examples/overwatch/src/main.ts`
- Add/replace: `examples/overwatch/src/**` copied from `../park/src/ui/src/**`

- [ ] **Step 1: Snapshot current boundaries**

Run:

```bash
git status --short
find examples/overwatch/src -maxdepth 3 -type f | sort
find ../park/src/ui/src -maxdepth 3 -type f | sort
```

Expected: note any existing untracked files before touching the tree. Do not revert unrelated user changes.

- [ ] **Step 2: Copy Park UI source**

Copy `../park/src/ui/src/**` into `examples/overwatch/src/**` and copy the Park `index.html` shape. The Overwatch `index.html` should use:

```html
<html lang="en" class="h-full">
  <body class="h-full m-0">
    <div id="app" class="h-full"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

Keep Park's Google font links and title for visual parity.

- [ ] **Step 3: Convert package dependencies**

Update `examples/overwatch/package.json` to remove React-specific dependencies and add Park UI dependencies plus the generated SDK packages:

```json
{
  "dependencies": {
    "@aukilabs/auki-domain": "file:./sdk-generated/auki-domain",
    "@aukilabs/auki-geometry": "file:./sdk-generated/auki-geometry",
    "@aukilabs/auki-network": "file:./sdk-generated/auki-network",
    "motion": "^12.38.0",
    "three": "^0.169.0"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.0.0",
    "@types/three": "^0.169.0",
    "happy-dom": "^20.9.0",
    "playwright": "^1.53.0",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.6.0",
    "vite": "^5.4.0",
    "vitest": "^2.1.0"
  }
}
```

Keep scripts:

```json
{
  "dev": "vite --host 0.0.0.0 --port 7880",
  "build": "vite build",
  "test": "node scripts/stage-sdk.mjs && vitest run",
  "smoke": "node scripts/smoke-two-browser.mjs",
  "typecheck": "tsc --noEmit"
}
```

- [ ] **Step 4: Convert Vite/Tailwind configuration**

Use Park's Tailwind v4 plugin and preserve Overwatch's port/proxy:

```ts
import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [tailwindcss()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
  server: {
    allowedHosts: ["taina-proclergy-chang.ngrok-free.dev"],
    host: "0.0.0.0",
    port: 7880,
    proxy: {
      "/discovery": {
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/discovery/, ""),
        target: "http://127.0.0.1:8091",
      },
    },
  },
});
```

- [ ] **Step 5: Run the expected failing build**

Run:

```bash
npm --prefix examples/overwatch install
npm --prefix examples/overwatch run typecheck
```

Expected: FAIL on data modules that still reference Park backend assumptions or copied tests that expect `/api/*`. Do not fix by adding an HTTP shim.

- [ ] **Step 6: Commit**

After the later tasks make typecheck pass, commit this task with the data-adapter changes it depends on:

```bash
git add examples/overwatch
git commit -m "feat: port Park UI shell into Overwatch"
```

---

## Task 2: Stage All Browser SDK Packages

**Files:**

- Modify: `examples/overwatch/scripts/stage-sdk.mjs`
- Modify: `examples/overwatch/package.json`

- [ ] **Step 1: Add `auki-geometry` to the staged SDK packages**

`stage-sdk.mjs` must copy:

```js
const packages = [
  { name: "auki-network", source: path.join(repoRoot, "bindings/javascript/auki-network"), required: "index.js" },
  { name: "auki-domain", source: path.join(repoRoot, "bindings/javascript/auki-domain"), required: "index.js" },
  { name: "auki-geometry", source: path.join(repoRoot, "bindings/javascript/auki-geometry"), required: "auki_geometry.js" },
];
```

- [ ] **Step 2: Verify missing package failure text**

Temporarily move `bindings/javascript/auki-geometry/auki_geometry.js` aside or mock the existence check in a focused unit test if the script is tested. The error must tell the worker to run:

```bash
just generate-javascript-bindings auki-geometry
```

- [ ] **Step 3: Run staging**

Run:

```bash
node examples/overwatch/scripts/stage-sdk.mjs
ls examples/overwatch/sdk-generated
```

Expected: `auki-network`, `auki-domain`, and `auki-geometry` are present.

---

## Task 3: Add The Browser SDK Runtime

**Files:**

- Create: `examples/overwatch/src/sdk/runtime.ts`
- Create: `examples/overwatch/src/sdk/runtime.test.ts`
- Create: `examples/overwatch/src/sdk/demoSensors.ts`
- Modify: `examples/overwatch/src/sdk/createOverwatchPeer.ts`
- Modify: `examples/overwatch/src/sdk/contract.ts`

- [ ] **Step 1: Write runtime contract tests**

Create tests around a fake `OverwatchPeer` that prove:

```ts
it("maps SDK participant snapshots to Park-style cluster state", async () => {
  const runtime = createSdkRuntime({ peerFactory: async () => fakePeer });
  const seen: unknown[] = [];
  runtime.subscribeCluster((snap) => seen.push(snap));
  fakePeer.emit(fakeSnapshotWithRemoteCamera());
  expect(seen.at(-1)).toMatchObject({
    status: {
      source: {
        kind: "in_cluster",
        cluster_name: "overwatch",
      },
    },
    self: {
      kind: "self",
      peer_id: "self-peer",
    },
    peers: [
      {
        kind: "peer",
        peer_id: "remote-peer",
      },
    ],
  });
});
```

Also prove no `fetch` call is made during runtime initialization by stubbing `globalThis.fetch` to throw.

- [ ] **Step 2: Implement the runtime singleton**

`runtime.ts` should export:

```ts
export type RuntimeClusterMode = "create" | "join";

export type SdkRuntime = {
  ensureStarted(): Promise<void>;
  listClusters(discoveryUrl: string): Promise<DiscoveryClusterEntry[]>;
  enterCluster(input: { discoveryUrl: string; clusterName: string; mode: RuntimeClusterMode }): Promise<ClusterStatus>;
  leaveCluster(): Promise<ClusterStatus>;
  subscribeCluster(cb: (snap: ClusterSnapshot) => void): () => void;
  getCluster(): ClusterSnapshot;
  getParticipant(peerId: string): RuntimeParticipant | null;
  getParticipantInfo(peerId: string): ParticipantInfo | null;
  getParticipantSensors(peerId: string): RuntimeSensor[];
  getStream(peerId: string, sensorId: string): Promise<StreamHandle>;
  debugState(): Record<string, unknown>;
};
```

The runtime owns one `OverwatchPeer`. It calls `observeParticipants` once and derives Park-shaped state for all data modules.

- [ ] **Step 3: Preserve browser identity and app metadata**

Update `createOverwatchPeer.ts` so the generated domain peer identifies as Park for visual parity:

```ts
return new AukiBrowserDomainPeer({
  networkPeer,
  discoveryFactory: (url: string) => new DiscoveryDirectoryClient(url),
  appId: "park",
  displayName: `Park Browser ${networkPeer.peerId.slice(-6)}`,
}) as OverwatchPeer;
```

Keep the existing wallet seed localStorage key or migrate it deliberately to `auki:overwatch:wallet-seed:v1`. Do not use Park's production key because this is an SDK example.

- [ ] **Step 4: Add deterministic demo sensors**

Add `demoSensors.ts` with one camera sensor and one detection/generic sensor:

```ts
export const demoCameraSensor = {
  sensor_id: "overwatch/browser/demo-camera",
  sensor_hash: "overwatch-demo-camera-v1",
  kind: "camera" as const,
  label: "Browser preview",
};

export const generatedBytesSensor = {
  sensor_id: "overwatch/browser/generated-bytes",
  sensor_hash: "overwatch-generated-bytes-v1",
  kind: "detection" as const,
  label: "Generated bytes",
};
```

Publish a tiny valid JPEG loop for `demoCameraSensor` so Playwright smoke can open a remote video tile without camera permission.

- [ ] **Step 5: Verify**

Run:

```bash
npm --prefix examples/overwatch run test -- src/sdk/runtime.test.ts
```

Expected: PASS. Runtime tests use fake peers and make no app `/api/*` calls.

---

## Task 4: Replace Domain, Cluster, And Daemon Data Modules

**Files:**

- Modify: `examples/overwatch/src/data/cluster.ts`
- Modify: `examples/overwatch/src/data/daemons.ts`
- Modify: `examples/overwatch/src/data/discovery.ts`
- Modify: `examples/overwatch/src/views/onboarding/domainPromptModal.ts`
- Keep: `examples/overwatch/src/data/domain.ts`
- Keep: `examples/overwatch/src/data/domainName.ts`

- [ ] **Step 1: Convert `data/cluster.ts` to runtime-backed state**

Replace the polling loop over:

```ts
fetch("/api/info")
fetch("/api/cluster/peers")
fetch("/api/cluster/status")
```

with `sdkRuntime.subscribeCluster`. Preserve Park's exported types and helper names so views stay unchanged:

```ts
export function subscribeCluster(cb: Listener): () => void {
  return sdkRuntime.subscribeCluster(cb);
}

export function getCluster(): ClusterSnapshot {
  return sdkRuntime.getCluster();
}
```

- [ ] **Step 2: Convert `data/daemons.ts` to peer-derived rows**

`Daemon.url` becomes the SDK peer id. `source` stays `"cluster"` so directory cards and routes keep working:

```ts
const daemons = snapshot.peers.map((p) => ({
  url: p.peer_id,
  name: p.info.name || p.peer_id,
  app: p.info.app || "unknown",
  source: "cluster" as const,
}));
```

Self must not appear in `getDaemons()`; Park's own card already renders separately.

- [ ] **Step 3: Convert Discovery cluster listing**

`data/discovery.ts` should call `sdkRuntime.listClusters(discoveryUrl)` instead of `/api/discovery/snapshot`.

The returned `DiscoverySnapshot.raw_json` can be the original JSON returned by `DiscoveryDirectoryClient.discoverPeersJson`.

- [ ] **Step 4: Convert the domain prompt**

Replace:

```ts
fetch(`/api/cluster/list?...`)
fetch(`/api/cluster/create`, ...)
fetch(`/api/cluster/join`, ...)
fetch("/api/cluster/leave", ...)
```

with:

```ts
await sdkRuntime.listClusters(url);
await sdkRuntime.enterCluster({ discoveryUrl: url, clusterName, mode: "create" });
await sdkRuntime.enterCluster({ discoveryUrl: url, clusterName, mode: "join" });
await sdkRuntime.leaveCluster();
```

`mode: "create"` should list clusters first and fail if the named cluster already exists. `mode: "join"` should list clusters first and fail if the named cluster does not exist. Both successful paths may call the generated SDK's current `createOrJoin` method after that preflight.

- [ ] **Step 5: Add fetch-ban tests**

Add or update tests so these modules fail if they call `/api`:

```ts
vi.stubGlobal("fetch", vi.fn((url: string) => {
  if (String(url).includes("/api/")) throw new Error(`unexpected API call ${url}`);
  return Promise.reject(new Error("network disabled in test"));
}));
```

- [ ] **Step 6: Verify**

Run:

```bash
npm --prefix examples/overwatch run test -- src/data/cluster.test.ts src/data/router.test.ts src/views/onboarding/domainPromptModal.test.ts
```

Expected: PASS. No app `/api/*` calls are made.

---

## Task 5: Replace Info, Sensor Log, And Health Data

**Files:**

- Modify: `examples/overwatch/src/data/info.ts`
- Modify: `examples/overwatch/src/data/sensorLogs.ts`
- Modify: `examples/overwatch/src/data/health.ts`
- Keep/adapt: `examples/overwatch/src/data/participantInfo.ts`

- [ ] **Step 1: Runtime-backed participant info**

`subscribeInfo(peerId, cb)` should subscribe to SDK runtime state and emit `InfoSnapshot` for the participant whose `peer_id` equals the supplied `url` argument.

Return status:

- `"ok"` when the peer exists in the SDK snapshot.
- `"pending"` before the first SDK snapshot.
- `"unreachable"` when the peer disappears.
- Never emit `"no_info"` for SDK browser peers because participant info is synthesized from SDK state.

- [ ] **Step 2: Runtime-backed sensor logs**

`subscribeSensorLogs(peerId, cb)` should synthesize current-session log rows from SDK participant sensors:

```ts
{
  sensor_log_id: `${peerId}/${sensor.sensor_id}`,
  session_id: `${peerId}/browser-session`,
  sensor_id: sensor.sensor_id,
  sensor_hash: sensor.sensor_hash,
  clock_id: `${sensor.sensor_id}/clock`,
  clock_hash: `${sensor.sensor_hash}:clock`,
  retention_ns: 0,
  duration_ns: 0,
  started_at_ns: 1,
  stopped_at_ns: null,
}
```

This gives Park's sidebar, strip, and tile routing the same live-row shape they expect from Control API v1 without pretending historical logs exist.

- [ ] **Step 3: Runtime-backed health**

`subscribeHealth(peerId, cb)` should derive health from `subscribeInfo`:

- `ok` for present connected peers.
- `unreachable` for disappeared peers.
- `unknown` before the first snapshot.

- [ ] **Step 4: Verify**

Run:

```bash
npm --prefix examples/overwatch run test -- src/data/info.test.ts src/data/sensorLogs.test.ts
```

Expected: PASS. Tests cover peer disappearance and sensor-list changes.

---

## Task 6: Replace Catalog, Registry, And Geometry Fetches

**Files:**

- Modify: `examples/overwatch/src/data/registry.ts`
- Create: `examples/overwatch/src/data/registry.test.ts`
- Modify: `examples/overwatch/src/sdk/runtime.ts`
- Modify: `examples/overwatch/src/sdk/contract.ts`

- [ ] **Step 1: Catalog from SDK participant snapshots**

`fetchCatalog(peerId)` should return:

```ts
{
  sensors: runtime.getParticipantSensors(peerId).map((sensor) => ({
    sensor_id: sensor.sensor_id,
    sensor_hash: sensor.sensor_hash,
    kind: sensor.kind,
    sensor_entry_json: sensor.sensor_entry_json ?? null,
    frame_entry_json: sensor.frame_entry_json ?? null,
  })),
}
```

If the SDK snapshot only has `sensor_id`, `sensor_hash`, and `kind`, keep using Park's existing `synthEntryFromCatalogKind` fallback.

- [ ] **Step 2: Registry entries from embedded catalog JSON or synthetic fallbacks**

`fetchSensorEntry(peerId, sensorId, sensorHash)` should:

1. Look up the runtime catalog row.
2. Parse `sensor_entry_json` when present.
3. Fall back to `synthEntryFromCatalogKind(sensorId, kind)` when the catalog is kind-only.
4. Return `null` only when the peer or sensor is unknown.

- [ ] **Step 3: Frame entries from embedded catalog JSON**

`fetchFrameEntry(peerId, frameId, frameHash)` should find a matching `frame_entry_json` among that peer's catalog rows and parse it. Return `null` when no frame entry is available.

- [ ] **Step 4: Geometry matrices through `@aukilabs/auki-geometry`**

`fetchFrameConventionMatrix(peerId, frameId, frameHash)` should:

1. Initialize `@aukilabs/auki-geometry` once.
2. Fetch the source frame entry through `fetchFrameEntry`.
3. Use a Three/OpenGL target frame:

```ts
const threeFrame = {
  frame_id: "three/opengl",
  handedness: "right",
  axes: { x: "right", y: "up", z: "backward" },
  units: "meters",
};
```

4. Return `JSON.parse(conventionMatrixJson(JSON.stringify(source), JSON.stringify(threeFrame)))`.
5. Return identity matrix when no frame entry is available, matching Park's graceful fallback behavior.

- [ ] **Step 5: Stream descriptors from SDK stream accept messages**

`fetchStreamDescriptor(peerId, sensorId)` should read descriptor metadata from `streamHub` after the first `accept` message:

```ts
{
  sensor_id,
  sensor_hash,
  clock_id,
  clock_hash,
  frame_id,
  frame_hash,
}
```

If no stream has accepted yet, return `null`; existing tile code already retries.

- [ ] **Step 6: Verify**

Run:

```bash
npm --prefix examples/overwatch run test -- src/data/registry.test.ts
```

Expected: PASS for embedded entries, synthetic entries, frame matrices, and missing peer fallback.

---

## Task 7: Replace HTTP Stream Polling With SDK Stream Subscriptions

**Files:**

- Create: `examples/overwatch/src/sdk/streamHub.ts`
- Create: `examples/overwatch/src/sdk/streamHub.test.ts`
- Modify: `examples/overwatch/src/data/preview.ts`
- Modify: `examples/overwatch/src/data/pointcloudPreview.ts`
- Keep/adapt: `examples/overwatch/src/data/cdrPointCloud.ts`
- Keep/adapt: `examples/overwatch/src/data/peerSync.ts`

- [ ] **Step 1: Build one shared stream hub per `(peer_id, sensor_id)`**

`streamHub` should expose:

```ts
export type RuntimeStreamFrame = {
  descriptor: StreamDescriptor | null;
  payload: Uint8Array;
  seq: number;
  timestamp_ns: number;
  receivedAt: number;
  receivedAtWallMs: number;
};

export function subscribeRuntimeStream(
  spec: { peer_id: string; sensor_id: string },
  cb: (frame: RuntimeStreamFrame | null) => void,
): () => void;

export function getRuntimeStreamDescriptor(
  spec: { peer_id: string; sensor_id: string },
): StreamDescriptor | null;
```

Internally, open the SDK stream once per key via `sdkRuntime.getStream(peer_id, sensor_id)`, read `nextMessage()` in a loop, store the `accept` descriptor, and broadcast every `entry`.

- [ ] **Step 2: Convert camera preview**

Replace `fetch("/api/streams/.../latest.jpg")` in `data/preview.ts` with `subscribeRuntimeStream`.

When a frame arrives:

```ts
const blob = new Blob([frame.payload], { type: "image/jpeg" });
const url = URL.createObjectURL(blob);
```

Preserve Park's recent ObjectURL sliding window and `PreviewFrame` fields. Use `frame.timestamp_ns`, `frame.seq`, `descriptor.sensor_hash`, and `descriptor.clock_id`.

- [ ] **Step 3: Convert point cloud preview**

Replace `fetch("/api/streams/.../latest.cdr")` in `data/pointcloudPreview.ts` with `subscribeRuntimeStream`.

Use the existing `decodePointCloud2(frame.payload.buffer.slice(...))` path and preserve seq de-duplication.

- [ ] **Step 4: Preserve stream state**

Map stream lifecycle into Park's existing states:

- Before SDK stream opens: `"connecting"`
- After first `accept`: `"live"`
- SDK stream open failure: `"rejected"`
- SDK `decline`: `"declined"`
- SDK read error after accept: `"reconnecting"` if the hub retries, `"rejected"` if it gives up

- [ ] **Step 5: Add tests**

Use fake streams that emit:

```ts
{ accept: { sensor_id, sensor_hash, clock_id, clock_hash, frame_id, frame_hash } }
{ entry: { seq: 1, timestamp_ns: 10, payload: [255, 216, 255, 217] } }
```

Assert:

- `subscribePreview` emits an ObjectURL-backed frame.
- `fetch` is never called.
- `fetchStreamDescriptor` returns the accept descriptor after the first stream message.
- Unsubscribing the last listener closes the SDK stream.

- [ ] **Step 6: Verify**

Run:

```bash
npm --prefix examples/overwatch run test -- src/sdk/streamHub.test.ts src/data/preview.test.ts src/data/pointcloudPreview.test.ts
```

Expected: PASS. Existing copied Park tests should be updated to expect SDK stream URLs/state instead of `/api/streams/*`.

---

## Task 8: Make Backend-Only Park Features Browser-Safe

**Files:**

- Modify: `examples/overwatch/src/data/recordings.ts`
- Modify: `examples/overwatch/src/data/settings.ts`
- Modify: `examples/overwatch/src/data/inspect.ts`
- Modify: `examples/overwatch/src/data/mic.ts`
- Modify: `examples/overwatch/src/views/robot/tiles/k1Pose.ts`
- Modify: `examples/overwatch/src/views/robot/tiles/world.ts`

- [ ] **Step 1: Recordings become empty live-only state**

`recordings.ts` should return an empty recording list and make start/stop calls reject with a typed browser-runtime error:

```ts
throw new Error("Browser Overwatch is live-only; SDK recording control is not available in this example.");
```

The UI should remain mounted. Controls that call these functions can show Park's existing toast/error pattern.

- [ ] **Step 2: Settings become local browser settings**

`settings.ts` should use localStorage for any setting the UI still reads. `saveSdkContentsRoot` should reject with:

```ts
throw new Error("Browser Overwatch does not have a filesystem SDK contents root.");
```

- [ ] **Step 3: Inspect focus is local-only**

`inspect.ts` should keep route/focus state in memory and stop posting to `/api/inspect/focus`.

- [ ] **Step 4: Audio listen controls are disabled unless SDK audio is wired**

For this pass, `mic.ts` should report:

```ts
{
  available: false,
  enabled: false,
  level: null,
  reason: "Browser Overwatch audio is not wired in this pass."
}
```

If the copied Park UI expects different field names, preserve the exact exported type and map to its disabled state.

- [ ] **Step 5: K1 pose and world tiles must not open `/api/k1/pose` WebSockets**

Replace the WebSocket construction in `k1Pose.ts` and `world.ts` with an overlay state when SDK joint pose support is unavailable:

```ts
setOverlay("unsupported", "K1 pose requires an SDK browser FK source; no Park backend is running.", true);
```

Do not leave a dormant `new WebSocket("/api/k1/pose/...")` path in browser Overwatch.

- [ ] **Step 6: Verify app-wide API ban**

Run:

```bash
rg -n '"/api/|`/api/|/api/' examples/overwatch/src
```

Expected: no live code paths in `examples/overwatch/src` call `/api/*`. Comments may remain only if they explicitly describe the Park source behavior and the Overwatch replacement.

---

## Task 9: Update Smoke Testing For Park UI

**Files:**

- Modify: `examples/overwatch/scripts/smoke-two-browser.mjs`
- Modify: `examples/overwatch/src/main.ts`
- Modify: `examples/overwatch/src/sdk/runtime.ts`

- [ ] **Step 1: Expose a test-only global**

Expose enough runtime state for smoke tests without coupling to DOM class names:

```ts
globalThis.__overwatchPark = {
  snapshot: () => sdkRuntime.getCluster(),
  peerId: () => sdkRuntime.getCluster().self?.peer_id ?? null,
  openStream: (peerId: string, sensorId: string) => sdkRuntime.getStream(peerId, sensorId),
};
```

- [ ] **Step 2: Update join flow selectors**

The smoke should:

1. Start Discovery.
2. Start Vite on `127.0.0.1:7880`.
3. Open two browser contexts.
4. Fill Park's mandatory domain modal with Discovery URL and smoke domain name.
5. Click `Create new` in the first context.
6. Click `Join existing` in the second context.
7. Wait for `__overwatchPark.snapshot().peers.length >= 1` in both contexts.

- [ ] **Step 3: Verify remote video tile**

After both peers join:

1. On page B, click the remote Browser/Park card.
2. Toggle the remote `Browser preview` camera sensor in the bottom strip.
3. Wait for an `<img>` tile to receive a `blob:` URL.
4. Assert `globalThis.__overwatchPark.openStream(peerA, demoCameraSensor)` can read one SDK entry.

- [ ] **Step 4: Preserve the no-API assertion**

Keep and strengthen:

```js
const apiRequests = requestUrls.filter((url) => new URL(url).pathname.includes("/api/"));
if (apiRequests.length > 0) {
  throw new Error(`Overwatch smoke made app backend requests: ${apiRequests.join(", ")}`);
}
```

Allow `/discovery` and direct Discovery URLs.

- [ ] **Step 5: Verify**

Run:

```bash
just overwatch-smoke
```

Expected: PASS. Two browser peers join one Domain, the Park UI renders, a remote camera tile receives frames, and no app `/api/*` request occurs.

---

## Task 10: Documentation And Changelog Propagation

**Files:**

- Modify: `examples/overwatch/README.md`
- Modify: `examples/overwatch/changelog.md`
- Modify: `examples/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Update README**

Document:

- Overwatch now uses Park's frontend source.
- The backend replacement is the generated SDK browser runtime, not a local HTTP server.
- Browser-only limitations: no historical recordings, no Park filesystem settings, no K1 FK tile without a browser SDK FK source, audio disabled unless wired in the implementation.
- `just overwatch` and `just overwatch-smoke` commands.
- The acceptance invariant: no app `/api/*` calls.

- [ ] **Step 2: Add leaf changelog**

Add an `examples/overwatch/changelog.md` entry:

```md
### Nils's codex · May 26, HKT, 2026

Ported Overwatch to Park's operator UI while replacing Park's backend data modules with generated SDK browser/WASM bindings. The example remains backend-free: Discovery is the only network bootstrap service, sensor streams are consumed through SDK stream handles, and smoke coverage asserts no app `/api/*` calls.
```

- [ ] **Step 3: Propagate to parent changelogs**

Add one-liners to:

- `examples/changelog.md`
- `changelog.md`

- [ ] **Step 4: Final verification**

Run:

```bash
npm --prefix examples/overwatch run typecheck
npm --prefix examples/overwatch run test
npm --prefix examples/overwatch run build
just overwatch-smoke
```

Expected: all PASS.

---

## Risks And Decisions To Surface

- Park UI changes upstream after the copy will not automatically flow into Overwatch. If this becomes painful, make a shared UI package in Park or this repo after the browser SDK boundary is proven.
- Generated `AukiBrowserDomainPeer` currently exposes `createOrJoin`, not strict create-only and join-only methods. The plan uses Discovery preflight to keep Park's two-button modal behavior honest.
- Full Park feature parity requires SDK/browser equivalents for backend services Park currently provides: recording control, audio listen routing, K1 forward kinematics, and possibly richer registry/resource exchange.
- Point-cloud tiles can work if producers publish CDR `PointCloud2` payloads and catalog/frame metadata. If browser SDK streams carry a different point-cloud payload, `data/cdrPointCloud.ts` must be replaced with the SDK-owned decoder for that payload.
- The visual parity target should be checked with screenshots after Task 1 and after Task 9. The DOM should be Park's DOM; differences should come only from live data availability.

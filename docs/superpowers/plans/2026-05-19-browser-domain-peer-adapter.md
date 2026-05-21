# Browser Domain Peer Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first SDK-owned browser Domain peer package boundary that Park can load, while failing closed for real peer transport until browser-dialable SDK networking exists.

**Architecture:** Add `crates/auki-domain-browser` as a TypeScript package following the SDK's per-component binding pattern. The first tranche implements package scaffolding, Park-compatible contract types, global installer, browser identity persistence, Discovery HTTP listing, participant snapshot state, and explicit `transport_unavailable` results for join/create/stream operations. It does not implement browser libp2p/WebSocket/WebTransport/WebRTC transport or audio yet.

**Tech Stack:** TypeScript, Vitest, Vite-compatible ESM package, browser IndexedDB/localStorage APIs through injectable storage adapters, SDK conformance vectors from Rust docs/tests.

---

## File Structure

- `crates/auki-domain-browser/README.md` — public package spec and Park compatibility notes.
- `crates/auki-domain-browser/parking_lot.md` — browser transport and browser-Manager open questions.
- `crates/auki-domain-browser/changelog.md` — leaf changelog.
- `crates/auki-domain-browser/package.json` — npm package scripts and dev dependencies.
- `crates/auki-domain-browser/tsconfig.json` — TS config.
- `crates/auki-domain-browser/vitest.config.ts` — browser-package test config.
- `crates/auki-domain-browser/src/README.md` — actual implementation status.
- `crates/auki-domain-browser/src/sprint.md` — next implementation steps.
- `crates/auki-domain-browser/src/contract.ts` — Park-compatible public TypeScript contract.
- `crates/auki-domain-browser/src/errors.ts` — structured result/error helpers.
- `crates/auki-domain-browser/src/identity.ts` — persistent browser identity seam.
- `crates/auki-domain-browser/src/discovery.ts` — Discovery HTTP list/create shape and error mapping.
- `crates/auki-domain-browser/src/peer.ts` — `BrowserDomainPeer` first-tranche implementation.
- `crates/auki-domain-browser/src/installGlobal.ts` — `window.aukiBrowserPeer.createPeer()` installer.
- `crates/auki-domain-browser/src/index.ts` — package exports.
- `crates/auki-domain-browser/src/*.test.ts` — focused Vitest tests.

Propagate docs to `crates/changelog.md`, root `changelog.md`, and relevant parking lots immediately after each committed task.

---

### Task 1: Package Scaffold

**Files:**
- Create: `crates/auki-domain-browser/README.md`
- Create: `crates/auki-domain-browser/parking_lot.md`
- Create: `crates/auki-domain-browser/changelog.md`
- Create: `crates/auki-domain-browser/package.json`
- Create: `crates/auki-domain-browser/tsconfig.json`
- Create: `crates/auki-domain-browser/vitest.config.ts`
- Create: `crates/auki-domain-browser/src/README.md`
- Create: `crates/auki-domain-browser/src/sprint.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Create package metadata**

Create `crates/auki-domain-browser/package.json`:

```json
{
  "name": "@aukilabs/auki-domain-browser",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "scripts": {
    "build": "tsc -p tsconfig.json",
    "test": "vitest run",
    "test:watch": "vitest"
  },
  "devDependencies": {
    "typescript": "^5.6.0",
    "vitest": "^2.1.0",
    "happy-dom": "^20.9.0"
  }
}
```

- [ ] **Step 2: Create TypeScript config**

Create `crates/auki-domain-browser/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "lib": ["ES2022", "DOM"],
    "strict": true,
    "declaration": true,
    "outDir": "dist",
    "rootDir": "src",
    "skipLibCheck": true
  },
  "include": ["src"]
}
```

- [ ] **Step 3: Create Vitest config**

Create `crates/auki-domain-browser/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "happy-dom",
  },
});
```

- [ ] **Step 4: Create component docs**

Create `crates/auki-domain-browser/README.md`:

```markdown
# auki-domain-browser

Browser Domain peer adapter for the Auki SDK.

This package is the browser sibling of `auki-domain-py`: a consumer-facing Domain peer handle for web apps. Park is the first consumer. The package owns browser peer identity, Discovery HTTP calls, participant snapshots, and the SDK boundary for future browser transport and sensor streams.

The first tranche intentionally fails closed for real peer transport. It can install `window.aukiBrowserPeer.createPeer()` and expose the Park-compatible contract, but `joinDomain`, `createDomain`, and stream methods return structured `transport_unavailable` errors until an SDK-owned browser transport is implemented.
```

Create `crates/auki-domain-browser/parking_lot.md`:

```markdown
# Parking Lot — auki-domain-browser

Open questions for the browser Domain peer adapter.

## Items

- **2026-05-19 — Browser transport.** Native SDK peers advertise TCP/QUIC multiaddrs that browsers cannot dial directly. Decide the first SDK-owned browser transport: WebSocket multiaddrs, WebTransport, WebRTC-as-transport, or SDK relay.
- **2026-05-19 — Browser Manager scope.** Decide whether browser `createDomain` makes the browser a Manager in v1, provisions/depends on a native Manager, or lands after leaf-peer join support.
```

Create `crates/auki-domain-browser/changelog.md`:

```markdown
# Changelog — auki-domain-browser

Append-only timeline of changes for the browser Domain peer adapter. Latest entry on top.

---

### Nils's codex · May 19, HKT, 2026

Created the `auki-domain-browser` package scaffold for Park's browser-peer Milestone 0 handoff. The first tranche is explicitly an SDK boundary and identity/Discovery shell; real browser transport and audio remain follow-up work.
```

Create `crates/auki-domain-browser/src/README.md`:

```markdown
# auki-domain-browser/src

Implementation status for the browser Domain peer adapter.

Currently implemented:

- package scaffold
- Park-compatible contract types
- global installer
- browser identity seam
- Discovery HTTP list mapping
- explicit transport-unavailable behavior for real peer operations

Not yet implemented:

- browser-dialable SDK transport
- `/auki/join/0.0.1`
- `/auki/info/0.0.1`
- sensor catalogs
- audio streams
```

Create `crates/auki-domain-browser/src/sprint.md`:

```markdown
# auki-domain-browser/src — sprint

## Now

Make the first package tranche importable by Park:

- contract types
- `createPeer`
- `installAukiBrowserPeer`
- stable browser identity storage seam
- Discovery list mapping
- participant snapshots
- structured `transport_unavailable` for unsupported peer operations

## Next

Choose and implement SDK-owned browser transport so a browser leaf peer can join an existing native Manager and fetch `/auki/info/0.0.1`.
```

- [ ] **Step 5: Verify scaffold does not build yet**

Run:

```bash
cd crates/auki-domain-browser
npm install
npm run build
```

Expected: build fails because `src/index.ts` does not exist yet.

- [ ] **Step 6: Propagate changelogs**

Prepend to `crates/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

`auki-domain-browser`: created the browser Domain peer adapter package scaffold for Park's Milestone 0 SDK handoff. See `auki-domain-browser/changelog.md` for detail.
```

Prepend to root `changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**Browser Domain peer adapter package scaffolded.** Added the `auki-domain-browser` package shell for Park's browser-peer Milestone 0, with browser transport and browser-Manager scope tracked as explicit open SDK questions. See [`crates/changelog.md`](crates/changelog.md) for crate-level propagation.
```

- [ ] **Step 7: Commit**

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "chore: scaffold browser domain peer package"
```

---

### Task 2: Public Contract And Global Installer

**Files:**
- Create: `crates/auki-domain-browser/src/contract.ts`
- Create: `crates/auki-domain-browser/src/errors.ts`
- Create: `crates/auki-domain-browser/src/installGlobal.ts`
- Create: `crates/auki-domain-browser/src/index.ts`
- Create: `crates/auki-domain-browser/src/installGlobal.test.ts`
- Modify: `crates/auki-domain-browser/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write installer tests**

Create `crates/auki-domain-browser/src/installGlobal.test.ts`:

```ts
import { describe, expect, it, beforeEach } from "vitest";
import { installAukiBrowserPeer } from "./installGlobal";
import type { BrowserDomainPeer } from "./contract";

declare global {
  interface Window {
    aukiBrowserPeer?: { createPeer(): Promise<BrowserDomainPeer> };
  }
}

describe("installAukiBrowserPeer", () => {
  beforeEach(() => {
    delete window.aukiBrowserPeer;
  });

  it("installs a Park-compatible global factory", async () => {
    const peer = {} as BrowserDomainPeer;
    installAukiBrowserPeer(() => Promise.resolve(peer));

    await expect(window.aukiBrowserPeer?.createPeer()).resolves.toBe(peer);
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/installGlobal.test.ts
```

Expected: fail because `installGlobal.ts` and `contract.ts` do not exist.

- [ ] **Step 3: Implement contract and installer**

Create `crates/auki-domain-browser/src/contract.ts` using Park's current contract:

```ts
export type PeerId = string;
export type SensorId = string;
export type DomainName = string;

export type Result<T> =
  | { ok: true; value: T }
  | { ok: false; error: PeerError };

export type PeerError = {
  code:
    | "transport_unavailable"
    | "discovery_unreachable"
    | "domain_list_failed"
    | "domain_create_failed"
    | "domain_join_failed"
    | "domain_leave_failed"
    | "sensor_publish_failed"
    | "sensor_subscribe_failed"
    | "sensor_unsubscribe_failed"
    | "unsupported"
    | "unknown";
  message: string;
};

export type DomainSummary = {
  name: DomainName;
  managerPeerId?: PeerId;
  peerCount?: number;
};

export type SensorKind =
  | "camera"
  | "point_cloud"
  | "joint_encoders"
  | "audio"
  | "detection"
  | "unknown";

export const SDK_SENSOR_KINDS = [
  "camera",
  "point_cloud",
  "joint_encoders",
  "audio",
  "detection",
  "unknown",
] as const satisfies readonly SensorKind[];

export type SensorSummary = {
  id: SensorId;
  kind: SensorKind;
  label: string;
  publishable: boolean;
  subscribable: boolean;
};

export type StreamState =
  | "off"
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "declined"
  | "error";

export const SDK_STREAM_STATES = [
  "off",
  "idle",
  "connecting",
  "connected",
  "reconnecting",
  "declined",
  "error",
] as const satisfies readonly StreamState[];

export type MediaPresence = {
  micAvailable: boolean;
  micPublicationEnabled: boolean;
  micCaptureHealthy: boolean;
  listeningToPeerId: PeerId | null;
  listeningToSensorId: SensorId | null;
  playbackHealthy: boolean;
  selectedRemoteStreamState: StreamState;
  lastFrameUnixMs: number | null;
  inputLevel: number | null;
  outputLevel: number | null;
};

export type Participant = {
  peerId: PeerId;
  appId: string;
  displayName: string;
  isSelf: boolean;
  connected: boolean;
  sensors: SensorSummary[];
  mediaPresence: MediaPresence;
};

export type PeerSnapshot = {
  selfPeerId: PeerId;
  domainName: DomainName | null;
  participants: Participant[];
  managerPeerId: PeerId | null;
  electionState: "unknown" | "stable" | "degraded";
};

export type Unsubscribe = () => void;

export type BrowserDomainPeer = {
  getSelfPeerId(): Promise<PeerId>;
  listDomains(discoveryUrl: string): Promise<Result<DomainSummary[]>>;
  createDomain(discoveryUrl: string, domainName: DomainName): Promise<Result<void>>;
  joinDomain(discoveryUrl: string, domainName: DomainName): Promise<Result<void>>;
  leaveDomain(): Promise<Result<void>>;
  observeParticipants(onSnapshot: (snapshot: PeerSnapshot) => void): Unsubscribe;
  setParticipantMetadata(metadata: { appId: string; displayName: string }): Promise<Result<void>>;
  declareLocalSensors(sensors: SensorSummary[]): Promise<Result<void>>;
  setSensorPublication(sensorId: SensorId, enabled: boolean): Promise<Result<void>>;
  subscribeToSensor(peerId: PeerId, sensorId: SensorId): Promise<Result<void>>;
  unsubscribeFromSensor(peerId: PeerId, sensorId: SensorId): Promise<Result<void>>;
};

export type BrowserDomainPeerFactory = {
  createPeer(): Promise<BrowserDomainPeer>;
};
```

Create `crates/auki-domain-browser/src/errors.ts`:

```ts
import type { PeerError, Result } from "./contract";

export function ok<T>(value: T): Result<T> {
  return { ok: true, value };
}

export function fail<T>(code: PeerError["code"], message: string): Result<T> {
  return { ok: false, error: { code, message } };
}

export function transportUnavailable<T>(): Result<T> {
  return fail("transport_unavailable", "Browser SDK transport is not implemented yet.");
}
```

Create `crates/auki-domain-browser/src/installGlobal.ts`:

```ts
import type { BrowserDomainPeer, BrowserDomainPeerFactory } from "./contract";

declare global {
  interface Window {
    aukiBrowserPeer?: BrowserDomainPeerFactory;
  }
}

export function installAukiBrowserPeer(createPeer: () => Promise<BrowserDomainPeer>): void {
  window.aukiBrowserPeer = { createPeer };
}
```

Create `crates/auki-domain-browser/src/index.ts`:

```ts
export * from "./contract";
export * from "./errors";
export * from "./installGlobal";
```

- [ ] **Step 4: Verify test and build**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/installGlobal.test.ts
npm run build
```

Expected: test passes; build emits `dist/`.

- [ ] **Step 5: Changelog and commit**

Prepend leaf and parent changelog entries, then:

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: add browser peer contract installer"
```

---

### Task 3: Identity Storage Seam

**Files:**
- Create: `crates/auki-domain-browser/src/identity.ts`
- Create: `crates/auki-domain-browser/src/identity.test.ts`
- Modify: `crates/auki-domain-browser/src/index.ts`
- Modify: changelogs

- [ ] **Step 1: Write identity tests**

Create `crates/auki-domain-browser/src/identity.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { loadOrCreateSeed, memorySeedStore, shortPeerId } from "./identity";

describe("browser identity helpers", () => {
  it("persists generated seed through the provided store", async () => {
    const store = memorySeedStore();
    const first = await loadOrCreateSeed(store, () => new Uint8Array(32).fill(7));
    const second = await loadOrCreateSeed(store, () => new Uint8Array(32).fill(9));

    expect(Array.from(first)).toEqual(new Array(32).fill(7));
    expect(Array.from(second)).toEqual(new Array(32).fill(7));
  });

  it("rejects stored seeds that are not 32 bytes", async () => {
    const store = memorySeedStore(new Uint8Array([1, 2, 3]));

    await expect(loadOrCreateSeed(store, () => new Uint8Array(32))).rejects.toThrow(
      "Stored browser peer seed must be 32 bytes",
    );
  });

  it("formats short peer ids from the last six characters", () => {
    expect(shortPeerId("12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar")).toBe(
      "AiVKcar",
    );
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/identity.test.ts
```

Expected: fail because `identity.ts` does not exist.

- [ ] **Step 3: Implement identity seam**

Create `crates/auki-domain-browser/src/identity.ts`:

```ts
export type SeedStore = {
  load(): Promise<Uint8Array | null>;
  save(seed: Uint8Array): Promise<void>;
};

export async function loadOrCreateSeed(
  store: SeedStore,
  randomSeed: () => Uint8Array = cryptoRandomSeed,
): Promise<Uint8Array> {
  const existing = await store.load();
  if (existing) {
    if (existing.byteLength !== 32) {
      throw new Error("Stored browser peer seed must be 32 bytes");
    }
    return existing;
  }

  const seed = randomSeed();
  if (seed.byteLength !== 32) {
    throw new Error("Generated browser peer seed must be 32 bytes");
  }
  await store.save(seed);
  return seed;
}

export function memorySeedStore(initial: Uint8Array | null = null): SeedStore {
  let seed = initial;
  return {
    async load() {
      return seed ? new Uint8Array(seed) : null;
    },
    async save(next) {
      seed = new Uint8Array(next);
    },
  };
}

export function shortPeerId(peerId: string): string {
  return peerId.slice(-6);
}

function cryptoRandomSeed(): Uint8Array {
  const seed = new Uint8Array(32);
  crypto.getRandomValues(seed);
  return seed;
}
```

Update `src/index.ts`:

```ts
export * from "./contract";
export * from "./errors";
export * from "./identity";
export * from "./installGlobal";
```

- [ ] **Step 4: Verify**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/identity.test.ts
npm run build
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: add browser identity seed seam"
```

---

### Task 4: Discovery Listing

**Files:**
- Create: `crates/auki-domain-browser/src/discovery.ts`
- Create: `crates/auki-domain-browser/src/discovery.test.ts`
- Modify: `crates/auki-domain-browser/src/index.ts`
- Modify: changelogs

- [ ] **Step 1: Write Discovery tests**

Create `crates/auki-domain-browser/src/discovery.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { listDomains } from "./discovery";

describe("listDomains", () => {
  it("maps Discovery clusters into DomainSummary rows", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          clusters: [
            {
              name: "retail-lab",
              manager_peer_id: "peer-manager",
              peer_count: 2,
            },
          ],
        }),
        { status: 200 },
      ),
    );

    const result = await listDomains("http://discovery.example", fetcher);

    expect(fetcher).toHaveBeenCalledWith("http://discovery.example/clusters");
    expect(result).toEqual({
      ok: true,
      value: [{ name: "retail-lab", managerPeerId: "peer-manager", peerCount: 2 }],
    });
  });

  it("returns discovery_unreachable when fetch throws", async () => {
    const fetcher = vi.fn().mockRejectedValue(new Error("network down"));

    const result = await listDomains("http://discovery.example", fetcher);

    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.code).toBe("discovery_unreachable");
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/discovery.test.ts
```

Expected: fail because `discovery.ts` does not exist.

- [ ] **Step 3: Implement Discovery mapping**

Create `crates/auki-domain-browser/src/discovery.ts`:

```ts
import type { DomainSummary, Result } from "./contract";
import { fail, ok } from "./errors";

type Fetcher = (url: string) => Promise<Response>;

type DiscoveryCluster = {
  name: string;
  manager_peer_id?: string;
  peer_count?: number;
};

export async function listDomains(
  discoveryUrl: string,
  fetcher: Fetcher = fetch,
): Promise<Result<DomainSummary[]>> {
  const base = discoveryUrl.replace(/\/+$/, "");
  try {
    const response = await fetcher(`${base}/clusters`);
    if (!response.ok) {
      return fail("domain_list_failed", `Discovery returned HTTP ${response.status}`);
    }
    const body = (await response.json()) as { clusters?: DiscoveryCluster[] };
    const clusters = Array.isArray(body.clusters) ? body.clusters : [];
    return ok(
      clusters.map((cluster) => ({
        name: cluster.name,
        managerPeerId: cluster.manager_peer_id,
        peerCount: cluster.peer_count,
      })),
    );
  } catch (error) {
    const detail = error instanceof Error ? error.message : "unknown fetch error";
    return fail("discovery_unreachable", `Discovery unreachable: ${detail}`);
  }
}
```

Update `src/index.ts`:

```ts
export * from "./contract";
export * from "./discovery";
export * from "./errors";
export * from "./identity";
export * from "./installGlobal";
```

- [ ] **Step 4: Verify**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/discovery.test.ts
npm run build
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: map browser discovery domains"
```

---

### Task 5: First-Tranche Browser Peer

**Files:**
- Create: `crates/auki-domain-browser/src/peer.ts`
- Create: `crates/auki-domain-browser/src/peer.test.ts`
- Modify: `crates/auki-domain-browser/src/index.ts`
- Modify: changelogs

- [ ] **Step 1: Write peer tests**

Create `crates/auki-domain-browser/src/peer.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { createBrowserDomainPeer } from "./peer";

describe("createBrowserDomainPeer", () => {
  it("emits an idle unjoined snapshot immediately", async () => {
    const peer = await createBrowserDomainPeer({ peerId: "self-peer" });
    const snapshots: unknown[] = [];

    peer.observeParticipants((snapshot) => snapshots.push(snapshot));

    expect(snapshots).toEqual([
      {
        selfPeerId: "self-peer",
        domainName: null,
        participants: [],
        managerPeerId: null,
        electionState: "unknown",
      },
    ]);
  });

  it("delegates listDomains to Discovery mapping", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ clusters: [{ name: "demo" }] }), { status: 200 }),
    );
    const peer = await createBrowserDomainPeer({ peerId: "self-peer", fetcher });

    const result = await peer.listDomains("http://discovery.example");

    expect(result).toEqual({ ok: true, value: [{ name: "demo" }] });
  });

  it("fails closed for join until browser transport exists", async () => {
    const peer = await createBrowserDomainPeer({ peerId: "self-peer" });

    const result = await peer.joinDomain("http://discovery.example", "demo");

    expect(result).toEqual({
      ok: false,
      error: {
        code: "transport_unavailable",
        message: "Browser SDK transport is not implemented yet.",
      },
    });
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/peer.test.ts
```

Expected: fail because `peer.ts` does not exist.

- [ ] **Step 3: Implement first-tranche peer**

Create `crates/auki-domain-browser/src/peer.ts`:

```ts
import type {
  BrowserDomainPeer,
  PeerId,
  PeerSnapshot,
  Result,
  SensorSummary,
} from "./contract";
import { listDomains as listDiscoveryDomains } from "./discovery";
import { ok, transportUnavailable } from "./errors";

type Fetcher = (url: string) => Promise<Response>;

export type CreateBrowserDomainPeerOptions = {
  peerId: PeerId;
  fetcher?: Fetcher;
};

export async function createBrowserDomainPeer(
  options: CreateBrowserDomainPeerOptions,
): Promise<BrowserDomainPeer> {
  let snapshot: PeerSnapshot = {
    selfPeerId: options.peerId,
    domainName: null,
    participants: [],
    managerPeerId: null,
    electionState: "unknown",
  };
  const observers = new Set<(snapshot: PeerSnapshot) => void>();
  const fetcher = options.fetcher;

  const emit = () => {
    for (const observer of observers) observer(snapshot);
  };

  return {
    async getSelfPeerId() {
      return options.peerId;
    },
    listDomains(discoveryUrl) {
      return listDiscoveryDomains(discoveryUrl, fetcher);
    },
    async createDomain() {
      return transportUnavailable();
    },
    async joinDomain() {
      return transportUnavailable();
    },
    async leaveDomain() {
      snapshot = {
        selfPeerId: options.peerId,
        domainName: null,
        participants: [],
        managerPeerId: null,
        electionState: "unknown",
      };
      emit();
      return ok(undefined);
    },
    observeParticipants(onSnapshot) {
      observers.add(onSnapshot);
      onSnapshot(snapshot);
      return () => observers.delete(onSnapshot);
    },
    async setParticipantMetadata() {
      return ok(undefined);
    },
    async declareLocalSensors(_sensors: SensorSummary[]): Promise<Result<void>> {
      return ok(undefined);
    },
    async setSensorPublication() {
      return transportUnavailable();
    },
    async subscribeToSensor() {
      return transportUnavailable();
    },
    async unsubscribeFromSensor() {
      return transportUnavailable();
    },
  };
}
```

Update `src/index.ts`:

```ts
export * from "./contract";
export * from "./discovery";
export * from "./errors";
export * from "./identity";
export * from "./installGlobal";
export * from "./peer";
```

- [ ] **Step 4: Verify**

Run:

```bash
cd crates/auki-domain-browser
npm run test -- src/peer.test.ts
npm run test
npm run build
```

Expected: all package tests and build pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: add first browser domain peer shell"
```

---

## Follow-Up Plan Required

After this first tranche lands, write a separate plan for SDK-owned browser transport. That plan must decide one of:

- browser-dialable WebSocket multiaddrs served by native SDK peers
- WebTransport
- WebRTC as SDK transport
- SDK relay

Do not implement Park audio until a browser peer can join a Domain and fetch participant info through SDK-owned peer protocols.

## Self-Review Notes

- Spec coverage: This plan covers package scaffold, Park-compatible contract, global installer, identity storage seam, Discovery listing, idle snapshots, and fail-closed transport behavior. It intentionally does not claim to complete join/audio.
- Placeholder scan: No task contains TBD/TODO/fill-in placeholders. Future transport/audio work is explicitly excluded into a follow-up plan.
- Type consistency: `BrowserDomainPeer`, `PeerSnapshot`, `Result`, and `PeerError` match the first consumer contract, with SDK-specific `transport_unavailable` added.

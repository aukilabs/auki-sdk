import { describe, expect, it, vi } from "vitest";

import {
  createSdkRuntime,
  type OverwatchPeer,
  type PeerSnapshot,
} from "./runtime";

describe("createSdkRuntime", () => {
  it("maps SDK participant snapshots to Park-style cluster state", async () => {
    const peer = fakePeer();
    const runtime = createSdkRuntime({ peerFactory: async () => peer });
    const seen: unknown[] = [];

    await runtime.ensureStarted();
    runtime.subscribeCluster((snap) => seen.push(snap));
    peer.emit(fakeSnapshotWithRemoteCamera());

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
          info: {
            app: "park",
            name: "Remote Browser",
          },
        },
      ],
    });
  });

  it("initializes the SDK peer without calling app API routes", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockImplementation((input) => {
      throw new Error(`unexpected fetch: ${String(input)}`);
    });
    const runtime = createSdkRuntime({ peerFactory: async () => fakePeer() });

    await runtime.ensureStarted();

    expect(fetchSpy).not.toHaveBeenCalled();
    fetchSpy.mockRestore();
  });
});

type FakePeer = OverwatchPeer & {
  emit(snapshot: PeerSnapshot): void;
};

function fakePeer(): FakePeer {
  const observers = new Set<(snapshot: PeerSnapshot) => void>();
  return {
    peerId: "self-peer",
    async createOrJoin() {},
    observeParticipants(cb) {
      observers.add(cb);
      return () => observers.delete(cb);
    },
    async declareSensors() {},
    async publishSensor() {},
    async subscribeToSensor() {
      return {
        async nextMessage() {
          return null;
        },
      };
    },
    debugState() {
      return {};
    },
    emit(snapshot) {
      observers.forEach((cb) => cb(snapshot));
    },
  };
}

function fakeSnapshotWithRemoteCamera(): PeerSnapshot {
  return {
    selfPeerId: "self-peer",
    domainName: "overwatch",
    managerPeerId: "self-peer",
    role: "manager",
    participants: [
      {
        peer_id: "self-peer",
        name: "Park Browser self",
        app: "park",
        is_self: true,
        is_manager: true,
        connected: true,
        sensors: [],
      },
      {
        peer_id: "remote-peer",
        name: "Remote Browser",
        app: "park",
        connected: true,
        sensors: [
          {
            sensor_id: "remote-camera",
            sensor_hash: "remote-camera-hash",
            kind: "camera",
            label: "Browser preview",
          },
        ],
      },
    ],
  };
}

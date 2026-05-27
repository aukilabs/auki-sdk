import { DiscoveryDirectoryClient } from "@aukilabs/auki-network";

import type { OverwatchPeer, PeerSnapshot, SensorSummary, StreamHandle } from "./contract";
import { createOverwatchPeer, ensureOverwatchSdkInitialized } from "./createOverwatchPeer";
import {
  createDemoCameraSource,
  generatedBytesSource,
  localDemoSensors,
} from "./demoSensors";

export type {
  OverwatchPeer,
  PeerSnapshot,
  PeerDebugState,
  SensorKind,
  SensorSource,
  SensorStreamEntry,
  SensorSummary,
  StreamHandle,
} from "./contract";

export type RuntimeClusterMode = "create" | "join";

export type ParticipantInfo = {
  app: string;
  name: string;
  session_id: string;
  session_clock_id: string;
  session_clock_hash: string;
  session_now_ns: number;
  cluster_joined_at_ns: number | null;
  peer_id: string;
  app_instance: string;
  is_manager: boolean;
  manager_peer_id: string;
};

export type ClusterPeerWire = {
  peer_id: string;
  multiaddrs: string[];
  join_ts_ns: number;
};

export type ClusterStatus = {
  source:
    | { kind: "not_in_cluster" }
    | {
        kind: "in_cluster";
        url: string;
        cluster_name: string;
        is_manager: boolean;
        manager_peer_id: string;
      };
  discovery_url: string | null;
};

export type RuntimeParticipant = {
  kind: "self" | "peer";
  peer_id: string;
  info: ParticipantInfo;
  receivedAtMs: number;
  sessionStartedEstMs: number;
  clusterJoinedEstMs: number | null;
};

export type RuntimeSensor = SensorSummary;

export type ClusterSnapshot = {
  self: RuntimeParticipant | null;
  selfWarnings: string[];
  peers: RuntimeParticipant[];
  status: ClusterStatus | null;
  refreshedAtMs: number;
};

export type DiscoveryClusterEntry = {
  name: string;
  manager_peer_id: string;
  manager_multiaddrs: string[];
  relay_multiaddrs?: string[];
  peer_count: number;
  created_ns: number;
  last_liveness_check_ns: number;
};

export type SdkRuntimeOptions = {
  peerFactory?: () => Promise<OverwatchPeer>;
  discoveryFactory?: (url: string) => DiscoveryClientLike;
  now?: () => number;
};

export type SdkRuntime = {
  ensureStarted(): Promise<void>;
  listClusters(discoveryUrl: string): Promise<DiscoveryClusterEntry[]>;
  enterCluster(input: {
    discoveryUrl: string;
    clusterName: string;
    mode: RuntimeClusterMode;
  }): Promise<ClusterStatus>;
  leaveCluster(): Promise<ClusterStatus>;
  subscribeCluster(cb: (snap: ClusterSnapshot) => void): () => void;
  getCluster(): ClusterSnapshot;
  getParticipant(peerId: string): RuntimeParticipant | null;
  getParticipantInfo(peerId: string): ParticipantInfo | null;
  getParticipantSensors(peerId: string): RuntimeSensor[];
  getStream(peerId: string, sensorId: string): Promise<StreamHandle>;
  debugState(): Record<string, unknown>;
};

type DiscoveryClientLike = {
  discoverPeersJson(queryJson: string): Promise<string>;
};

const defaultNow = () => Date.now();

export const sdkRuntime = createSdkRuntime();

export function createSdkRuntime(options: SdkRuntimeOptions = {}): SdkRuntime {
  const peerFactory = options.peerFactory ?? createOverwatchPeer;
  const discoveryFactory =
    options.discoveryFactory ??
    ((url: string): DiscoveryClientLike => new DiscoveryDirectoryClient(url));
  const now = options.now ?? defaultNow;

  let peer: OverwatchPeer | null = null;
  let started: Promise<void> | null = null;
  let rawSnapshot: PeerSnapshot | null = null;
  let joinedDiscoveryUrl: string | null = null;
  let localSensorsPublished = false;
  let leftLocally = false;
  const listeners = new Set<(snap: ClusterSnapshot) => void>();

  const notify = () => {
    const snap = currentCluster();
    listeners.forEach((cb) => cb(snap));
  };

  const ensureStarted = async () => {
    if (started) {
      return started;
    }
    started = (async () => {
      peer = await peerFactory();
      peer.observeParticipants((snapshot) => {
        rawSnapshot = snapshot;
        if (snapshot.domainName) {
          leftLocally = false;
        }
        notify();
      });
    })();
    return started;
  };

  const currentCluster = (): ClusterSnapshot => {
    const refreshedAtMs = now();
    if (!rawSnapshot || leftLocally) {
      return {
        self: null,
        selfWarnings: [],
        peers: [],
        status: {
          source: { kind: "not_in_cluster" },
          discovery_url: joinedDiscoveryUrl,
        },
        refreshedAtMs,
      };
    }

    const participants = rawSnapshot.participants;
    const selfRaw =
      participants.find((p) => p.is_self) ??
      participants.find((p) => p.peer_id === rawSnapshot?.selfPeerId) ??
      null;
    const self = selfRaw ? toRuntimeParticipant("self", selfRaw, rawSnapshot, refreshedAtMs) : null;
    const peers = participants
      .filter((p) => p.peer_id !== rawSnapshot?.selfPeerId && !p.is_self)
      .map((p) => toRuntimeParticipant("peer", p, rawSnapshot!, refreshedAtMs));
    const inCluster = Boolean(rawSnapshot.domainName);

    return {
      self,
      selfWarnings: [],
      peers,
      status: {
        source: inCluster
          ? {
              kind: "in_cluster",
              url: rawSnapshot.domainName ?? "",
              cluster_name: rawSnapshot.domainName ?? "",
              is_manager: rawSnapshot.role === "manager",
              manager_peer_id: rawSnapshot.managerPeerId ?? "",
            }
          : { kind: "not_in_cluster" },
        discovery_url: joinedDiscoveryUrl,
      },
      refreshedAtMs,
    };
  };

  const ensureLocalSensors = async () => {
    if (!peer || localSensorsPublished) {
      return;
    }
    await peer.declareSensors(localDemoSensors);
    await peer.publishSensor(localDemoSensors[0]!.sensor_id, createDemoCameraSource());
    await peer.publishSensor(localDemoSensors[1]!.sensor_id, generatedBytesSource);
    localSensorsPublished = true;
  };

  return {
    ensureStarted,
    async listClusters(discoveryUrl) {
      await ensureStarted();
      await ensureOverwatchSdkInitialized();
      const discovery = discoveryFactory(discoveryUrl);
      const body = JSON.parse(await discovery.discoverPeersJson("{}")) as {
        clusters?: DiscoveryClusterEntry[];
      };
      return body.clusters ?? [];
    },
    async enterCluster({ discoveryUrl, clusterName, mode }) {
      await ensureStarted();
      const clusters = await this.listClusters(discoveryUrl);
      const exists = clusters.some((cluster) => cluster.name === clusterName);
      if (mode === "create" && exists) {
        throw new Error(`Domain ${clusterName} already exists on this Discovery URL.`);
      }
      if (mode === "join" && !exists) {
        throw new Error(`Domain ${clusterName} does not exist on this Discovery URL.`);
      }
      if (!peer) {
        throw new Error("SDK peer did not initialize");
      }
      joinedDiscoveryUrl = discoveryUrl;
      await ensureLocalSensors();
      await peer.createOrJoin({ discoveryUrl, clusterName });
      notify();
      return currentCluster().status!;
    },
    async leaveCluster() {
      leftLocally = true;
      notify();
      return currentCluster().status!;
    },
    subscribeCluster(cb) {
      listeners.add(cb);
      cb(currentCluster());
      return () => listeners.delete(cb);
    },
    getCluster() {
      return currentCluster();
    },
    getParticipant(peerId) {
      const snap = currentCluster();
      if (snap.self?.peer_id === peerId) {
        return snap.self;
      }
      return snap.peers.find((p) => p.peer_id === peerId) ?? null;
    },
    getParticipantInfo(peerId) {
      return this.getParticipant(peerId)?.info ?? null;
    },
    getParticipantSensors(peerId) {
      const participant = rawSnapshot?.participants.find((p) => p.peer_id === peerId);
      return participant?.sensors ?? [];
    },
    async getStream(peerId, sensorId) {
      await ensureStarted();
      if (!peer) {
        throw new Error("SDK peer did not initialize");
      }
      return peer.subscribeToSensor(peerId, sensorId);
    },
    debugState() {
      return peer?.debugState() ?? {};
    },
  };
}

function toRuntimeParticipant(
  kind: "self" | "peer",
  participant: NonNullable<PeerSnapshot["participants"][number]>,
  snapshot: PeerSnapshot,
  receivedAtMs: number,
): RuntimeParticipant {
  const sessionStartedEstMs = receivedAtMs - 1;
  const inCluster = Boolean(snapshot.domainName);
  const clusterJoinedEstMs = inCluster ? receivedAtMs - 1 : null;
  return {
    kind,
    peer_id: participant.peer_id,
    info: {
      app: participant.app ?? "park",
      name: participant.name ?? shortPeer(participant.peer_id),
      session_id: `${participant.peer_id}/browser-session`,
      session_clock_id: `${participant.peer_id}/browser-session/monotonic`,
      session_clock_hash: "browser-session-clock",
      session_now_ns: 1_000_000,
      cluster_joined_at_ns: inCluster ? 1 : null,
      peer_id: participant.peer_id,
      app_instance: `${participant.peer_id}/browser`,
      is_manager: participant.is_manager ?? snapshot.managerPeerId === participant.peer_id,
      manager_peer_id: snapshot.managerPeerId ?? "",
    },
    receivedAtMs,
    sessionStartedEstMs,
    clusterJoinedEstMs,
  };
}

function shortPeer(peerId: string): string {
  return peerId.length <= 14 ? peerId : `${peerId.slice(0, 4)}...${peerId.slice(-10)}`;
}

import type {
  BrowserDomainPeer,
  DomainName,
  MediaPresence,
  Participant,
  PeerId,
  PeerSnapshot,
  Result,
  SensorSummary,
} from "./contract.js";
import { listDomains as listDiscoveryDomains } from "./discovery.js";
import { ok, transportUnavailable } from "./errors.js";
import {
  createJsLibp2pBrowserPeer,
  type BrowserPeerTransport,
  type JoinTarget,
} from "./jsLibp2pPeer.js";

type Fetcher = (url: string) => Promise<Response>;

type BrowserDomainJoinValue = {
  domainName?: DomainName;
  domain_name?: DomainName;
  managerPeerId?: PeerId;
  manager_peer_id?: PeerId;
  membershipJson?: string;
  membership_json?: string;
};

type BrowserDomainJoinResult =
  | Result<void>
  | { ok: true; value?: BrowserDomainJoinValue };

type BrowserDomainSession = Omit<Partial<BrowserDomainPeer>, "joinDomain"> & {
  peerId?: () => PeerId;
  joinDomain?: (
    discoveryUrl: string,
    domainName: DomainName,
  ) => BrowserDomainJoinResult | Promise<BrowserDomainJoinResult>;
};

type MembershipPeer = {
  peer_id?: PeerId;
  peerId?: PeerId;
};

type MembershipDocument = {
  peers?: MembershipPeer[];
};

type BrowserPeerFactory = {
  createPeer: () => Promise<BrowserDomainPeer>;
};

declare global {
  interface Window {
    aukiBrowserPeer?: BrowserPeerFactory;
  }
}

export type CreateBrowserDomainPeerOptions = {
  peerId: PeerId;
  seed?: Uint8Array;
  fetcher?: Fetcher;
  sdkSession?: BrowserDomainSession;
  transport?: BrowserPeerTransport;
  resolveJoinTarget?: (discoveryUrl: string, domainName: DomainName) => Promise<JoinTarget>;
};

export async function createBrowserDomainPeer(
  options: CreateBrowserDomainPeerOptions,
): Promise<BrowserDomainPeer> {
  if (options.seed || options.transport) {
    return createJsLibp2pBrowserPeer({
      seed: options.seed,
      peerId: options.peerId,
      fetcher: options.fetcher,
      transport: options.transport,
      resolveJoinTarget: options.resolveJoinTarget,
    });
  }
  if (!options.sdkSession) {
    const globalSession = await createPeerFromGlobal();
    if (globalSession) {
      return globalSession;
    }
    return failClosedPeer(options.peerId, options.fetcher);
  }
  return wrapSdkSession(options.peerId, options.sdkSession, options.fetcher);
}

async function createPeerFromGlobal(): Promise<BrowserDomainPeer | null> {
  const globalFactory = typeof window !== "undefined" ? window.aukiBrowserPeer : undefined;
  if (!globalFactory?.createPeer) {
    return null;
  }

  try {
    return await globalFactory.createPeer();
  } catch (_error) {
    return null;
  }
}

function failClosedPeer(selfPeerId: PeerId, fetcher?: Fetcher): BrowserDomainPeer {
  let snapshot: PeerSnapshot = {
    selfPeerId,
    domainName: null,
    participants: [],
    managerPeerId: null,
    electionState: "unknown",
  };
  const observers = new Set<(snapshot: PeerSnapshot) => void>();
  const effectiveFetcher = fetcher;

  const emit = () => {
    for (const observer of observers) observer(snapshot);
  };

  return {
    async getSelfPeerId() {
      return selfPeerId;
    },
    listDomains(discoveryUrl) {
      return listDiscoveryDomains(discoveryUrl, effectiveFetcher);
    },
    async createDomain() {
      return transportUnavailable();
    },
    async joinDomain() {
      return transportUnavailable();
    },
    async leaveDomain() {
      snapshot = {
        selfPeerId,
        domainName: null,
        participants: [],
        managerPeerId: null,
        electionState: "unknown",
      };
      emit();
      return ok<void>(undefined);
    },
    observeParticipants(onSnapshot) {
      observers.add(onSnapshot);
      onSnapshot(snapshot);
      return () => {
        observers.delete(onSnapshot);
      };
    },
    async setParticipantMetadata() {
      return ok<void>(undefined);
    },
    async declareLocalSensors(_sensors: SensorSummary[]): Promise<Result<void>> {
      return ok<void>(undefined);
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

function wrapSdkSession(
  fallbackPeerId: PeerId,
  sdkSession: BrowserDomainSession,
  fetcher?: Fetcher,
): BrowserDomainPeer {
  const selfPeerId = sdkSession.peerId?.() ?? fallbackPeerId;
  const shell = failClosedPeer(selfPeerId, fetcher);
  let localMetadata = { appId: "park", displayName: selfPeerId };
  let localSensors: SensorSummary[] = [];
  let localMediaPresence: MediaPresence = emptyMediaPresence();
  let snapshot: PeerSnapshot = emptySnapshot(selfPeerId);
  const observers = new Set<(snapshot: PeerSnapshot) => void>();

  const emit = () => {
    for (const observer of observers) observer(snapshot);
  };

  const refreshSelfParticipant = () => {
    if (!snapshot.domainName) return;
    snapshot = {
      ...snapshot,
      participants: snapshot.participants.map((participant) =>
        participant.peerId === selfPeerId
          ? localParticipant(selfPeerId, localMetadata, localSensors, localMediaPresence)
          : participant,
      ),
    };
    emit();
  };

  return {
    getSelfPeerId() {
      return sdkSession.getSelfPeerId?.() ?? Promise.resolve(sdkSession.peerId?.() ?? selfPeerId);
    },
    listDomains(discoveryUrl) {
      return sdkSession.listDomains?.(discoveryUrl) ?? shell.listDomains(discoveryUrl);
    },
    createDomain(discoveryUrl, domainName) {
      return callResult(() => sdkSession.createDomain?.(discoveryUrl, domainName), () =>
        shell.createDomain(discoveryUrl, domainName),
      );
    },
    async joinDomain(discoveryUrl, domainName) {
      const result =
        (await sdkSession.joinDomain?.(discoveryUrl, domainName)) ??
        (await shell.joinDomain(discoveryUrl, domainName));
      if (!result.ok) return result;

      const joined = joinedSnapshotFromResult(
        selfPeerId,
        domainName,
        localMetadata,
        localSensors,
        localMediaPresence,
        result,
      );
      if (joined) {
        snapshot = joined;
        emit();
      }
      return ok<void>(undefined);
    },
    async leaveDomain() {
      const result = await callResult(() => sdkSession.leaveDomain?.(), () => shell.leaveDomain());
      if (result.ok) {
        snapshot = emptySnapshot(selfPeerId);
        emit();
      }
      return result;
    },
    observeParticipants(onSnapshot) {
      if (sdkSession.observeParticipants) {
        return sdkSession.observeParticipants(onSnapshot);
      }
      observers.add(onSnapshot);
      onSnapshot(snapshot);
      return () => {
        observers.delete(onSnapshot);
      };
    },
    async setParticipantMetadata(metadata) {
      const result = await callResult(() => sdkSession.setParticipantMetadata?.(metadata), () =>
        shell.setParticipantMetadata(metadata),
      );
      if (result.ok) {
        localMetadata = metadata;
        refreshSelfParticipant();
      }
      return result;
    },
    async declareLocalSensors(sensors) {
      const result = await callResult(() => sdkSession.declareLocalSensors?.(sensors), () =>
        shell.declareLocalSensors(sensors),
      );
      if (result.ok) {
        localSensors = sensors;
        refreshSelfParticipant();
      }
      return result;
    },
    setSensorPublication(sensorId, enabled) {
      return callResult(() => sdkSession.setSensorPublication?.(sensorId, enabled), () =>
        shell.setSensorPublication(sensorId, enabled),
      ).then((result) => {
        if (result.ok && sensorId === "audio") {
          localMediaPresence = {
            ...localMediaPresence,
            micAvailable: true,
            micPublicationEnabled: enabled,
            micCaptureHealthy: enabled,
          };
          refreshSelfParticipant();
        }
        return result;
      });
    },
    subscribeToSensor(peerId, sensorId) {
      return callResult(() => sdkSession.subscribeToSensor?.(peerId, sensorId), () =>
        shell.subscribeToSensor(peerId, sensorId),
      ).then((result) => {
        if (result.ok) {
          localMediaPresence = {
            ...localMediaPresence,
            listeningToPeerId: peerId,
            listeningToSensorId: sensorId,
            selectedRemoteStreamState: "connecting",
          };
          refreshSelfParticipant();
        }
        return result;
      });
    },
    unsubscribeFromSensor(peerId, sensorId) {
      return callResult(() => sdkSession.unsubscribeFromSensor?.(peerId, sensorId), () =>
        shell.unsubscribeFromSensor(peerId, sensorId),
      ).then((result) => {
        if (
          result.ok &&
          localMediaPresence.listeningToPeerId === peerId &&
          localMediaPresence.listeningToSensorId === sensorId
        ) {
          localMediaPresence = {
            ...localMediaPresence,
            listeningToPeerId: null,
            listeningToSensorId: null,
            selectedRemoteStreamState: "off",
          };
          refreshSelfParticipant();
        }
        return result;
      });
    },
  };
}

function emptySnapshot(selfPeerId: PeerId): PeerSnapshot {
  return {
    selfPeerId,
    domainName: null,
    participants: [],
    managerPeerId: null,
    electionState: "unknown",
  };
}

function joinedSnapshotFromResult(
  selfPeerId: PeerId,
  requestedDomainName: DomainName,
  localMetadata: { appId: string; displayName: string },
  localSensors: SensorSummary[],
  localMediaPresence: MediaPresence,
  result: BrowserDomainJoinResult,
): PeerSnapshot | null {
  if (!result.ok) return null;
  const value = joinValue(result.value);
  const domainName = value?.domainName ?? requestedDomainName;
  const managerPeerId = value?.managerPeerId ?? null;
  const peerIds = membershipPeerIds(value?.membershipJson);
  const orderedPeerIds = [
    selfPeerId,
    ...peerIds.filter((peerId) => peerId !== selfPeerId && peerId !== managerPeerId),
    ...(managerPeerId && managerPeerId !== selfPeerId && !peerIds.includes(managerPeerId)
      ? [managerPeerId]
      : []),
  ];

  return {
    selfPeerId,
    domainName,
    participants: orderedPeerIds.map((peerId) =>
      peerId === selfPeerId
        ? localParticipant(selfPeerId, localMetadata, localSensors, localMediaPresence)
        : remoteParticipant(peerId),
    ),
    managerPeerId,
    electionState: "stable",
  };
}

function joinValue(value: unknown): { domainName?: DomainName; managerPeerId?: PeerId; membershipJson?: string } | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as BrowserDomainJoinValue;
  return {
    domainName: raw.domainName ?? raw.domain_name,
    managerPeerId: raw.managerPeerId ?? raw.manager_peer_id,
    membershipJson: raw.membershipJson ?? raw.membership_json,
  };
}

function membershipPeerIds(membershipJson: string | undefined): PeerId[] {
  if (!membershipJson) return [];
  try {
    const membership = JSON.parse(membershipJson) as MembershipDocument;
    if (!Array.isArray(membership.peers)) return [];
    return membership.peers
      .map((peer) => peer.peer_id ?? peer.peerId)
      .filter((peerId): peerId is PeerId => typeof peerId === "string" && peerId.length > 0);
  } catch (_error) {
    return [];
  }
}

function localParticipant(
  selfPeerId: PeerId,
  metadata: { appId: string; displayName: string },
  sensors: SensorSummary[],
  mediaPresence: MediaPresence = emptyMediaPresence(),
): Participant {
  return {
    peerId: selfPeerId,
    appId: metadata.appId,
    displayName: metadata.displayName,
    isSelf: true,
    connected: true,
    sensors,
    mediaPresence: withSensorAvailability(mediaPresence, sensors),
  };
}

function remoteParticipant(peerId: PeerId): Participant {
  return {
    peerId,
    appId: "unknown",
    displayName: shortPeerId(peerId),
    isSelf: false,
    connected: true,
    sensors: [],
    mediaPresence: emptyMediaPresence(),
  };
}

function emptyMediaPresence(): MediaPresence {
  return {
    micAvailable: false,
    micPublicationEnabled: false,
    micCaptureHealthy: false,
    listeningToPeerId: null,
    listeningToSensorId: null,
    playbackHealthy: false,
    selectedRemoteStreamState: "off" as const,
    lastFrameUnixMs: null,
    inputLevel: null,
    outputLevel: null,
  };
}

function withSensorAvailability(
  mediaPresence: MediaPresence,
  sensors: SensorSummary[],
): MediaPresence {
  if (!sensors.some((sensor) => sensor.kind === "audio")) return mediaPresence;
  return {
    ...mediaPresence,
    micAvailable: true,
  };
}

function shortPeerId(peerId: PeerId): string {
  return peerId.length <= 12 ? peerId : `${peerId.slice(0, 8)}...${peerId.slice(-4)}`;
}

async function callResult<T>(
  call: () => Result<T> | Promise<Result<T>> | undefined,
  fallback: () => Result<T> | Promise<Result<T>>,
): Promise<Result<T>> {
  const result = call();
  return result === undefined ? fallback() : result;
}

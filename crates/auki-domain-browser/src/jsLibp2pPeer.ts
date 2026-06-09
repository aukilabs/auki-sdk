import { circuitRelayTransport } from "@libp2p/circuit-relay-v2";
import { generateKeyPairFromSeed } from "@libp2p/crypto/keys";
import { identify } from "@libp2p/identify";
import { noise } from "@libp2p/noise";
import { webRTC, webRTCDirect } from "@libp2p/webrtc";
import { webSockets } from "@libp2p/websockets";
import { yamux } from "@chainsafe/libp2p-yamux";
import type { Stream } from "@libp2p/interface";
import { multiaddr } from "@multiformats/multiaddr";
import { createLibp2p } from "libp2p";
import type {
  BrowserDomainPeer,
  DomainName,
  MediaPresence,
  Participant,
  PeerId,
  PeerSnapshot,
  Result,
  SensorKind,
  SensorSummary,
} from "./contract.js";
import { fail, ok } from "./errors.js";
import { derivePeerSeed } from "./identity.js";
import {
  INFO_PROTOCOL,
  InfoRequest,
  InfoResponse,
  JOIN_PROTOCOL,
  JoinRequest,
  JoinResponse,
  STREAM_PROTOCOL,
  AudioData,
  StreamMessage,
  type ProtocolStream,
  readFrame,
  writeFrame,
} from "./protocol/control.js";

export type { ProtocolStream };

const AUDIO_STREAM_OPEN_ATTEMPTS = 3;
const AUDIO_STREAM_RETRY_DELAY_MS = 100;

type Fetcher = (url: string) => Promise<Response>;

export type JoinTarget = {
  domainName: DomainName;
  managerPeerId: PeerId;
  managerMultiaddrs: string[];
  relayMultiaddrs?: string[];
};

export type BrowserPeerTransport = {
  peerId: PeerId;
  setRelayMultiaddrs?(multiaddrs: string[]): void;
  start(): Promise<void>;
  stop(): Promise<void>;
  advertisedMultiaddrs(): string[];
  handleProtocol(
    protocol: string,
    handler: (stream: ProtocolStream, remotePeerId: PeerId) => Promise<void>,
  ): Promise<void>;
  dialProtocol(peerId: PeerId, multiaddrs: string[], protocol: string): Promise<ProtocolStream>;
};

export type CreateJsLibp2pBrowserPeerOptions = {
  seed?: Uint8Array;
  peerId?: PeerId;
  fetcher?: Fetcher;
  transport?: BrowserPeerTransport;
  resolveJoinTarget?: (discoveryUrl: string, domainName: DomainName) => Promise<JoinTarget>;
};

type MembershipPeer = {
  peer_id?: PeerId;
  peerId?: PeerId;
  multiaddrs?: string[];
};

type MembershipDocument = {
  cluster_name?: string;
  clusterName?: string;
  peers?: MembershipPeer[];
};

type ParticipantInfo = {
  app?: string;
  name?: string;
  peer_id?: string;
  peerId?: string;
};

export async function createJsLibp2pBrowserPeer(
  options: CreateJsLibp2pBrowserPeerOptions,
): Promise<BrowserDomainPeer> {
  const seed = options.transport ? options.seed : requiredSeed(options.seed);
  const peerId = options.peerId ?? options.transport?.peerId;
  if (!peerId) {
    throw new Error("A browser peer id is required when transport construction is deferred.");
  }
  return new JsLibp2pBrowserPeer(
    peerId,
    options.transport ?? null,
    seed,
    options.fetcher ?? fetch,
    options.resolveJoinTarget,
  );
}

export async function createBrowserLibp2pTransport(options: {
  seed: Uint8Array;
  relayMultiaddrs?: string[];
}): Promise<BrowserPeerTransport> {
  const privateKey = await generateKeyPairFromSeed("Ed25519", await derivePeerSeed(options.seed));
  const relayListenAddrs = (options.relayMultiaddrs ?? []).map((addr) => `${addr}/p2p-circuit`);
  const node = await createLibp2p({
    privateKey,
    connectionGater: {
      denyDialMultiaddr: () => false,
    },
    addresses: {
      listen: relayListenAddrs.length > 0 ? relayListenAddrs : ["/p2p-circuit"],
    },
    transports: [webSockets(), webRTC(), webRTCDirect(), circuitRelayTransport()],
    connectionEncrypters: [noise()],
    streamMuxers: [yamux()],
    services: {
      identify: identify({
        protocolPrefix: "auki",
      }),
    },
  });
  return new Libp2pBrowserPeerTransport(node, options.relayMultiaddrs ?? []);
}

class JsLibp2pBrowserPeer implements BrowserDomainPeer {
  private metadata: { appId: "park"; displayName: string };
  private sensors: SensorSummary[] = [];
  private mediaPresence = emptyMediaPresence();
  private snapshot: PeerSnapshot;
  private membership: MembershipDocument = { peers: [] };
  private readonly observers = new Set<(snapshot: PeerSnapshot) => void>();
  private joinedTarget: JoinTarget | null = null;
  private membershipRefreshTimer: ReturnType<typeof setTimeout> | null = null;
  private started = false;

  constructor(
    private readonly peerId: PeerId,
    private transport: BrowserPeerTransport | null,
    private readonly seed: Uint8Array | undefined,
    private readonly fetcher: Fetcher,
    private readonly resolveJoinTargetOverride:
      | ((discoveryUrl: string, domainName: DomainName) => Promise<JoinTarget>)
      | undefined,
  ) {
    this.metadata = { appId: "park", displayName: peerId };
    this.snapshot = emptySnapshot(peerId);
  }

  async getSelfPeerId(): Promise<PeerId> {
    return this.peerId;
  }

  async listDomains(discoveryUrl: string): Promise<Result<Array<{ name: DomainName; managerPeerId?: PeerId; peerCount?: number }>>> {
    try {
      const clusters = await fetchDiscoveryClusters(discoveryUrl, this.fetcher);
      return ok(
        clusters.map((cluster) => ({
          name: cluster.name,
          managerPeerId: cluster.manager_peer_id,
          peerCount: cluster.peer_count,
        })),
      );
    } catch (error) {
      return fail("domain_list_failed", errorMessage(error));
    }
  }

  async createDomain(): Promise<Result<void>> {
    return fail("unsupported", "Browser js-libp2p peers join existing Discovery Domains.");
  }

  async joinDomain(discoveryUrl: string, domainName: DomainName): Promise<Result<void>> {
    try {
      const target = this.resolveJoinTargetOverride
        ? await this.resolveJoinTargetOverride(discoveryUrl, domainName)
        : await resolveJoinTarget(discoveryUrl, domainName, this.fetcher);
      await this.ensureStarted(target.relayMultiaddrs);
      const advertised = this.requireTransport().advertisedMultiaddrs();
      if (advertised.length === 0) {
        return fail("transport_unavailable", "Browser js-libp2p peer has no advertised multiaddrs.");
      }

      const stream = await this.requireTransport().dialProtocol(
        target.managerPeerId,
        target.managerMultiaddrs,
        JOIN_PROTOCOL,
      );
      await writeFrame(stream, JoinRequest, { multiaddrs: advertised });
      const response = await readFrame(stream, JoinResponse);
      await stream.close();
      if (response.kind?.case === "reject") {
        return fail("domain_join_failed", response.kind.value.reason);
      }
      if (response.kind?.case !== "accept") {
        return fail("domain_join_failed", "Join response did not include accept or reject.");
      }

      const membership = parseMembership(response.kind.value.membershipJson);
      this.applyMembership(membership, target.domainName, target.managerPeerId);
      await this.fetchRemoteInfoAndSensors(target.managerPeerId);
      this.joinedTarget = target;
      this.scheduleMembershipRefresh();
      this.emit();
      return ok(undefined);
    } catch (error) {
      return fail("domain_join_failed", errorMessage(error));
    }
  }

  async leaveDomain(): Promise<Result<void>> {
    this.clearMembershipRefresh();
    this.joinedTarget = null;
    this.snapshot = emptySnapshot(this.peerId);
    this.emit();
    return ok(undefined);
  }

  observeParticipants(onSnapshot: (snapshot: PeerSnapshot) => void): () => void {
    this.observers.add(onSnapshot);
    onSnapshot(this.snapshot);
    return () => {
      this.observers.delete(onSnapshot);
    };
  }

  async setParticipantMetadata(metadata: { appId: "park"; displayName: string }): Promise<Result<void>> {
    this.metadata = metadata;
    this.refreshSelfParticipant();
    return ok(undefined);
  }

  async declareLocalSensors(sensors: SensorSummary[]): Promise<Result<void>> {
    const unsupported = sensors.find((sensor) => !isSensorKind(sensor.kind));
    if (unsupported) {
      return fail("sensor_publish_failed", `unsupported sensor kind ${JSON.stringify(unsupported.kind)}`);
    }
    this.sensors = sensors;
    this.refreshSelfParticipant();
    return ok(undefined);
  }

  async setSensorPublication(sensorId: string, enabled: boolean): Promise<Result<void>> {
    if (sensorId === "audio") {
      this.mediaPresence = {
        ...this.mediaPresence,
        micAvailable: true,
        micPublicationEnabled: enabled,
        micCaptureHealthy: enabled,
      };
      this.refreshSelfParticipant();
    }
    return ok(undefined);
  }

  async subscribeToSensor(peerId: PeerId, sensorId: string): Promise<Result<void>> {
    try {
      this.mediaPresence = {
        ...this.mediaPresence,
        listeningToPeerId: peerId,
        listeningToSensorId: sensorId,
        selectedRemoteStreamState: "connecting",
      };
      this.refreshSelfParticipant();

      const multiaddrs = this.multiaddrsForPeer(peerId);
      const stream = await this.openAudioStream(peerId, multiaddrs);
      await writeFrame(stream, StreamMessage, {
        variant: { case: "request", value: { sensorId } },
      });
      const accept = await readFrame(stream, StreamMessage);
      if (accept.variant?.case !== "accept") {
        this.mediaPresence = {
          ...this.mediaPresence,
          selectedRemoteStreamState: accept.variant?.case === "decline" ? "declined" : "error",
          playbackHealthy: false,
        };
        this.refreshSelfParticipant();
        return ok(undefined);
      }
      const entry = await readFrame(stream, StreamMessage);
      if (entry.variant?.case !== "entry") {
        throw new Error("audio stream ended before first entry");
      }
      const audio = AudioData.decode(entry.variant.value.payload);
      await closeStreamQuietly(stream);
      this.mediaPresence = {
        ...this.mediaPresence,
        selectedRemoteStreamState: "connected",
        playbackHealthy: true,
        lastFrameUnixMs: Date.now(),
        outputLevel: pcmS16leLevel(audio.data),
      };
      this.refreshSelfParticipant();
      return ok(undefined);
    } catch (error) {
      this.mediaPresence = {
        ...this.mediaPresence,
        selectedRemoteStreamState: "error",
        playbackHealthy: false,
      };
      this.refreshSelfParticipant();
      return fail("sensor_subscribe_failed", errorMessage(error));
    }
  }

  private async openAudioStream(peerId: PeerId, multiaddrs: string[]): Promise<ProtocolStream> {
    let lastError: unknown;
    for (let attempt = 1; attempt <= AUDIO_STREAM_OPEN_ATTEMPTS; attempt += 1) {
      try {
        return await this.requireTransport().dialProtocol(peerId, multiaddrs, STREAM_PROTOCOL);
      } catch (error) {
        lastError = error;
        if (attempt < AUDIO_STREAM_OPEN_ATTEMPTS) {
          await delay(AUDIO_STREAM_RETRY_DELAY_MS);
        }
      }
    }
    throw lastError;
  }

  async unsubscribeFromSensor(peerId: PeerId, sensorId: string): Promise<Result<void>> {
    if (
      this.mediaPresence.listeningToPeerId === peerId &&
      this.mediaPresence.listeningToSensorId === sensorId
    ) {
      this.mediaPresence = {
        ...this.mediaPresence,
        listeningToPeerId: null,
        listeningToSensorId: null,
        selectedRemoteStreamState: "off",
      };
      this.refreshSelfParticipant();
    }
    return ok(undefined);
  }

  private async ensureStarted(relayMultiaddrs: string[] = []): Promise<void> {
    if (this.started) return;
    if (!this.transport) {
      this.transport = await createBrowserLibp2pTransport({
        seed: requiredSeed(this.seed),
        relayMultiaddrs,
      });
      if (this.transport.peerId !== this.peerId) {
        throw new Error(`Derived browser peer id changed from ${this.peerId} to ${this.transport.peerId}`);
      }
    }
    const transport = this.requireTransport();
    transport.setRelayMultiaddrs?.(relayMultiaddrs);
    await transport.start();
    await transport.handleProtocol(INFO_PROTOCOL, async (stream) => {
      await readFrame(stream, InfoRequest);
      await writeFrame(stream, InfoResponse, {
        participantInfoJson: JSON.stringify(this.localParticipantInfo()),
      });
      await stream.close();
    });
    await transport.handleProtocol(STREAM_PROTOCOL, async (stream) => {
      const request = await readFrame(stream, StreamMessage);
      const sensorId = request.variant?.case === "request" ? request.variant.value.sensorId : "";
      if (sensorId !== "audio" || !this.mediaPresence.micPublicationEnabled) {
        await writeFrame(stream, StreamMessage, {
          variant: { case: "decline", value: { reason: "sensor unavailable" } },
        });
        await stream.close();
        return;
      }
      await writeFrame(stream, StreamMessage, {
        variant: {
          case: "accept",
          value: {
            sensorId: "audio",
            sensorHash: "",
            clockId: "",
            clockHash: "",
            frameId: "",
            frameHash: "",
          },
        },
      });
      await writeFrame(stream, StreamMessage, {
        variant: {
          case: "entry",
          value: {
            timestampNs: 0,
            seq: 1,
            payload: AudioData.encode({ data: generatedAudioFrame() }),
          },
        },
      });
      await stream.close();
    });
    this.started = true;
  }

  private async refreshMembership(): Promise<void> {
    if (!this.joinedTarget || !this.started) return;
    const target = this.joinedTarget;
    const advertised = this.requireTransport().advertisedMultiaddrs();
    if (advertised.length === 0) return;
    const stream = await this.requireTransport().dialProtocol(
      target.managerPeerId,
      target.managerMultiaddrs,
      JOIN_PROTOCOL,
    );
    await writeFrame(stream, JoinRequest, { multiaddrs: advertised });
    const response = await readFrame(stream, JoinResponse);
    await stream.close();
    if (response.kind?.case !== "accept") return;
    this.applyMembership(parseMembership(response.kind.value.membershipJson), target.domainName, target.managerPeerId);
    await this.fetchRemoteInfoAndSensors(target.managerPeerId);
    this.emit();
  }

  private scheduleMembershipRefresh(delayMs = 250): void {
    this.clearMembershipRefresh();
    this.membershipRefreshTimer = setTimeout(() => {
      void this.refreshMembership()
        .catch(() => {
          // Membership refresh is opportunistic; the next tick will retry.
        })
        .finally(() => {
          if (this.joinedTarget) this.scheduleMembershipRefresh(1000);
        });
    }, delayMs);
    maybeUnref(this.membershipRefreshTimer);
  }

  private clearMembershipRefresh(): void {
    if (this.membershipRefreshTimer) {
      clearTimeout(this.membershipRefreshTimer);
      this.membershipRefreshTimer = null;
    }
  }

  private requireTransport(): BrowserPeerTransport {
    if (!this.transport) throw new Error("Browser js-libp2p transport has not been started.");
    return this.transport;
  }

  private applyMembership(
    membership: MembershipDocument,
    domainName: DomainName,
    managerPeerId: PeerId,
  ): void {
    this.membership = membership;
    const participants = new Map<PeerId, Participant>();
    participants.set(this.peerId, this.localParticipant());
    for (const peer of membership.peers ?? []) {
      const peerId = peer.peer_id ?? peer.peerId;
      if (!peerId || participants.has(peerId)) continue;
      participants.set(peerId, remoteParticipant(peerId, peerId === managerPeerId));
    }
    if (!participants.has(managerPeerId)) {
      participants.set(managerPeerId, remoteParticipant(managerPeerId, true));
    }
    this.snapshot = {
      selfPeerId: this.peerId,
      domainName: membership.cluster_name ?? membership.clusterName ?? domainName,
      participants: Array.from(participants.values()),
      managerPeerId,
      electionState: "stable",
    };
  }

  private async fetchRemoteInfoAndSensors(managerPeerId: PeerId): Promise<void> {
    const membership = this.snapshot.participants
      .filter((participant) => participant.peerId !== this.peerId && participant.peerId !== managerPeerId)
      .map((participant) => ({
        peerId: participant.peerId,
        multiaddrs:
          this.membership.peers
            ?.find((peer) => (peer.peer_id ?? peer.peerId) === participant.peerId)
            ?.multiaddrs?.slice() ?? [],
      }));

    for (const target of membership) {
      if (target.multiaddrs.length === 0) continue;
      try {
        const info = await this.requestInfo(target.peerId, target.multiaddrs);
        this.applyRemoteCatalog(target.peerId, info);
      } catch (_error) {
        // A freshly joined peer may see a member before that browser has
        // installed its handlers. The next membership refresh will fill it in.
      }
    }
  }

  private async requestInfo(peerId: PeerId, multiaddrs: string[]): Promise<ParticipantInfo> {
    const stream = await this.requireTransport().dialProtocol(peerId, multiaddrs, INFO_PROTOCOL);
    await writeFrame(stream, InfoRequest, {});
    const response = await readFrame(stream, InfoResponse);
    await stream.close();
    return JSON.parse(response.participantInfoJson) as ParticipantInfo;
  }

  private applyRemoteCatalog(peerId: PeerId, info: ParticipantInfo): void {
    this.snapshot = {
      ...this.snapshot,
      participants: this.snapshot.participants.map((participant) => {
        if (participant.peerId !== peerId) return participant;
        return {
          ...participant,
          appId: info.app ?? participant.appId,
          displayName: info.name ?? participant.displayName,
        };
      }),
    };
  }

  private localParticipantInfo(): ParticipantInfo {
    return {
      app: this.metadata.appId,
      name: this.metadata.displayName,
      peer_id: this.peerId,
    };
  }

  private localParticipant(): Participant {
    return {
      peerId: this.peerId,
      appId: this.metadata.appId,
      displayName: this.metadata.displayName,
      isSelf: true,
      connected: true,
      sensors: this.sensors,
      mediaPresence: withSensorAvailability(this.mediaPresence, this.sensors),
    };
  }

  private multiaddrsForPeer(peerId: PeerId): string[] {
    return (
      this.membership.peers
        ?.find((peer) => (peer.peer_id ?? peer.peerId) === peerId)
        ?.multiaddrs?.slice() ?? []
    );
  }

  private refreshSelfParticipant(): void {
    this.snapshot = {
      ...this.snapshot,
      participants: this.snapshot.participants.map((participant) =>
        participant.peerId === this.peerId ? this.localParticipant() : participant,
      ),
    };
    this.emit();
  }

  private emit(): void {
    for (const observer of this.observers) observer(this.snapshot);
  }
}

class Libp2pBrowserPeerTransport implements BrowserPeerTransport {
  readonly peerId: PeerId;
  private started = false;
  private relayMultiaddrs: string[];

  constructor(
    private readonly node: Awaited<ReturnType<typeof createLibp2p>>,
    relayMultiaddrs: string[],
  ) {
    this.peerId = node.peerId.toString();
    this.relayMultiaddrs = relayMultiaddrs;
  }

  setRelayMultiaddrs(multiaddrs: string[]): void {
    if (!this.started) this.relayMultiaddrs = multiaddrs;
  }

  async start(): Promise<void> {
    if (this.started) return;
    await this.node.start();
    for (const addr of this.relayMultiaddrs) {
      await this.node.dial(multiaddr(addr));
    }
    this.started = true;
  }

  async stop(): Promise<void> {
    await this.node.stop();
    this.started = false;
  }

  advertisedMultiaddrs(): string[] {
    const addrs = this.node.getMultiaddrs().map((addr) => addr.toString());
    if (addrs.some((addr) => addr.includes("/p2p-circuit"))) {
      return addrs;
    }
    return [
      ...addrs,
      ...this.relayMultiaddrs.map((addr) => `${addr}/p2p-circuit/p2p/${this.peerId}`),
    ];
  }

  async handleProtocol(
    protocol: string,
    handler: (stream: ProtocolStream, remotePeerId: PeerId) => Promise<void>,
  ): Promise<void> {
    await this.node.handle(protocol, async (stream, connection) => {
      await handler(streamFromLibp2p(stream), connection.remotePeer.toString());
    }, { force: true, runOnLimitedConnection: true });
  }

  async dialProtocol(peerId: PeerId, multiaddrs: string[], protocol: string): Promise<ProtocolStream> {
    if (multiaddrs.length === 0) {
      throw new Error(`No multiaddrs available for ${peerId}`);
    }
    const targets = multiaddrs.map((addr) => multiaddr(addr));
    const stream = await this.node.dialProtocol(targets, protocol, {
      runOnLimitedConnection: true,
    });
    return streamFromLibp2p(stream);
  }
}

function streamFromLibp2p(stream: Stream): ProtocolStream {
  return stream as unknown as ProtocolStream;
}

async function resolveJoinTarget(
  discoveryUrl: string,
  domainName: DomainName,
  fetcher: Fetcher,
): Promise<JoinTarget> {
  if (discoveryUrl.startsWith("inline-manager://")) {
    const encoded = discoveryUrl.slice("inline-manager://".length);
    const [manager, relay] = decodeURIComponent(encoded).split("|").filter(Boolean);
    const managerPeerId = peerIdFromMultiaddr(manager);
    return {
      domainName,
      managerPeerId,
      managerMultiaddrs: [manager],
      relayMultiaddrs: relay ? [relay] : [],
    };
  }

  const clusters = await fetchDiscoveryClusters(discoveryUrl, fetcher);
  const cluster = clusters.find((entry) => entry.name === domainName);
  if (!cluster) throw new Error(`Discovery Domain not found: ${domainName}`);
  const managerPeerId = cluster.manager_peer_id ?? peerIdFromMultiaddr(cluster.manager_multiaddrs?.[0]);
  const managerMultiaddrs = cluster.manager_multiaddrs ?? [];
  if (!managerPeerId || managerMultiaddrs.length === 0) {
    throw new Error(`Discovery Domain ${domainName} is missing manager peer/address metadata`);
  }
  return {
    domainName,
    managerPeerId,
    managerMultiaddrs,
    relayMultiaddrs: cluster.relay_multiaddrs ?? [],
  };
}

async function fetchDiscoveryClusters(discoveryUrl: string, fetcher: Fetcher): Promise<Array<{
  name: string;
  manager_peer_id?: PeerId;
  manager_multiaddrs?: string[];
  relay_multiaddrs?: string[];
  peer_count?: number;
}>> {
  const base = discoveryUrl.replace(/\/+$/, "");
  const response = await fetcher(`${base}/clusters`);
  if (!response.ok) throw new Error(`Discovery returned HTTP ${response.status}`);
  const body = (await response.json()) as { clusters?: unknown[] };
  if (!Array.isArray(body.clusters)) return [];
  return body.clusters.map((cluster) => {
    if (!cluster || typeof cluster !== "object") throw new Error("Discovery cluster row is invalid");
    const raw = cluster as Record<string, unknown>;
    if (typeof raw.name !== "string" || raw.name.length === 0) {
      throw new Error("Discovery cluster row is missing a cluster name");
    }
    return {
      name: raw.name,
      manager_peer_id: typeof raw.manager_peer_id === "string" ? raw.manager_peer_id : undefined,
      manager_multiaddrs: Array.isArray(raw.manager_multiaddrs)
        ? raw.manager_multiaddrs.filter((addr): addr is string => typeof addr === "string")
        : undefined,
      relay_multiaddrs: Array.isArray(raw.relay_multiaddrs)
        ? raw.relay_multiaddrs.filter((addr): addr is string => typeof addr === "string")
        : undefined,
      peer_count: typeof raw.peer_count === "number" ? raw.peer_count : undefined,
    };
  });
}

function parseMembership(membershipJson: string): MembershipDocument {
  const membership = JSON.parse(membershipJson) as MembershipDocument;
  if (!Array.isArray(membership.peers)) {
    return { ...membership, peers: [] };
  }
  return membership;
}

function isSensorKind(value: string): value is SensorKind {
  return (
    value === "camera" ||
    value === "point_cloud" ||
    value === "joint_encoders" ||
    value === "audio"
  );
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

function remoteParticipant(peerId: PeerId, isManager = false): Participant {
  return {
    peerId,
    appId: isManager ? "auki-network" : "auki-browser-peer",
    displayName: peerId,
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
    selectedRemoteStreamState: "off",
    lastFrameUnixMs: null,
    inputLevel: null,
    outputLevel: null,
  };
}

function withSensorAvailability(mediaPresence: MediaPresence, sensors: SensorSummary[]): MediaPresence {
  if (!sensors.some((sensor) => sensor.kind === "audio")) return mediaPresence;
  return {
    ...mediaPresence,
    micAvailable: true,
  };
}

function peerIdFromMultiaddr(addr: string | undefined): PeerId {
  const match = addr?.match(/\/p2p\/([^/]+)(?:\/|$)/);
  if (!match) throw new Error(`multiaddr is missing /p2p/<peer-id>: ${addr ?? ""}`);
  return match[1];
}

function generatedAudioFrame(): Uint8Array {
  const sampleCount = 320;
  const bytes = new Uint8Array(sampleCount * 2);
  const view = new DataView(bytes.buffer);
  for (let i = 0; i < sampleCount; i += 1) {
    const phase = (i % 80) / 80;
    const sample = Math.round(Math.sin(phase * Math.PI * 2) * 0.25 * 32767);
    view.setInt16(i * 2, sample, true);
  }
  return bytes;
}

function pcmS16leLevel(bytes: Uint8Array): number {
  if (bytes.byteLength < 2) return 0;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let sum = 0;
  let count = 0;
  for (let offset = 0; offset + 1 < bytes.byteLength; offset += 2) {
    const sample = view.getInt16(offset, true) / 32768;
    sum += sample * sample;
    count += 1;
  }
  return Math.sqrt(sum / Math.max(count, 1));
}

function maybeUnref(timer: ReturnType<typeof setTimeout>): void {
  if (typeof (timer as { unref?: unknown }).unref === "function") {
    (timer as { unref(): void }).unref();
  }
}

async function closeStreamQuietly(stream: ProtocolStream): Promise<void> {
  try {
    await stream.close();
  } catch (_error) {
    // The remote may already have closed after sending the test frame.
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    maybeUnref(timer);
  });
}

function requiredSeed(seed: Uint8Array | undefined): Uint8Array {
  if (!seed) throw new Error("A 32-byte browser peer seed is required for js-libp2p transport.");
  if (seed.byteLength !== 32) throw new Error("Browser peer seed must be 32 bytes.");
  return seed;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

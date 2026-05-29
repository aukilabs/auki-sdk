import { circuitRelayTransport } from "@libp2p/circuit-relay-v2";
import { generateKeyPairFromSeed } from "@libp2p/crypto/keys";
import { identify } from "@libp2p/identify";
import { noise } from "@libp2p/noise";
import { peerIdFromString } from "@libp2p/peer-id";
import { webSockets } from "@libp2p/websockets";
import { yamux } from "@chainsafe/libp2p-yamux";
import { multiaddr } from "@multiformats/multiaddr";
import { createLibp2p } from "libp2p";
import { derivePeerSeed } from "./identity.js";

export type BrowserTransportName =
  | "websocket"
  | "webrtc"
  | "webrtc_direct"
  | "circuit_relay_v2";

export type BrowserConnectionDirection = "dialer" | "listener";

export type BrowserConnectionTransport =
  | "websocket"
  | "webrtc"
  | "webrtc_direct"
  | "webtransport"
  | "tcp"
  | "quic"
  | "unknown";

export type BrowserConnectionPath = {
  connectionId: string;
  direction: BrowserConnectionDirection;
  transport: BrowserConnectionTransport;
  relayInvolved: boolean;
  remoteAddress: string;
  status: string;
  direct: boolean;
  rttMs?: number;
};

export type BrowserTransport = {
  peerId: string;
  start(): Promise<void>;
  stop(): Promise<void>;
  multiaddrs(): string[];
  addRelayServerAddresses?(addresses: string[]): Promise<void> | void;
  dial(addresses: string[], options?: { force?: boolean }): Promise<void>;
  closePeerConnections?(peerId: string, keepAddresses?: string[]): Promise<void>;
  connectionPaths?(peerId: string): BrowserConnectionPath[];
  registerProtocolHandler(
    protocol: string,
    handler: BrowserProtocolHandler,
    options?: BrowserProtocolHandlerOptions,
  ): Promise<void>;
  unregisterProtocolHandler(protocol: string): Promise<void>;
  dialProtocol(
    peerId: string,
    addresses: string[],
    protocol: string,
  ): Promise<BrowserProtocolStream>;
};

export type BrowserProtocolStream = AsyncIterable<Uint8Array | { subarray(): Uint8Array }> & {
  send(data: Uint8Array): boolean;
  close(): Promise<void>;
  closeRead?(): Promise<void>;
  abort?(error: Error): void;
  onDrain?(): Promise<void>;
};

export type BrowserProtocolHandler = (
  stream: BrowserProtocolStream,
  remotePeerId: string,
) => Promise<void> | void;

export type BrowserProtocolHandlerOptions = {
  maxInboundStreams?: number;
  maxOutboundStreams?: number;
};

export type CreateBrowserLibp2pTransportOptions = {
  seed: Uint8Array;
  relayServerAddresses?: string[];
};

type BrowserConnectionCandidate = {
  id: string;
  remoteAddr: { toString(): string };
  status: string;
  newStream(
    protocol: string,
    options: { runOnLimitedConnection: true },
  ): Promise<BrowserProtocolStream>;
  close(): Promise<void>;
  abort?(error: Error): void;
};

export type BrowserConnectionCleanupCandidate = Pick<
  BrowserConnectionCandidate,
  "id" | "remoteAddr" | "status"
>;

const PING_PROTOCOL_ID = "/ipfs/ping/1.0.0";
const PING_MESSAGE_BYTES = 32;
const CONNECTION_CLEANUP_TIMEOUT_MS = 3_000;
const CONNECTION_CLEANUP_POLL_MS = 50;
const CONNECTION_CLOSE_ATTEMPT_MS = 250;
const CONNECTION_CLOSE_REASON = "Closing non-selected Auki peer connection";

export function supportedBrowserTransports(): BrowserTransportName[] {
  return ["websocket", "webrtc", "webrtc_direct", "circuit_relay_v2"];
}

export async function createBrowserLibp2pTransport(
  options: CreateBrowserLibp2pTransportOptions,
): Promise<BrowserTransport> {
  const { webRTC, webRTCDirect } = await import("@libp2p/webrtc");
  const privateKey = await generateKeyPairFromSeed("Ed25519", await derivePeerSeed(options.seed));
  const relayListenAddrs = relayListenAddresses(options.relayServerAddresses ?? []);
  const node = await createLibp2p({
    privateKey,
    connectionMonitor: {
      abortConnectionOnPingFailure: false,
      pingInterval: 10_000,
      pingTimeout: {
        minTimeout: 10_000,
        maxTimeout: 30_000,
      },
    },
    connectionGater: {
      denyDialMultiaddr: () => false,
    },
    addresses: {
      listen: uniqueStrings([...relayListenAddrs, "/p2p-circuit", "/webrtc"]),
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
  await node.handle(PING_PROTOCOL_ID, (stream) => echoPingStream(stream), {
    runOnLimitedConnection: true,
    maxInboundStreams: 32,
    force: true,
  });
  return new Libp2pBrowserTransport(node, options.relayServerAddresses ?? []);
}

export async function echoPingStream(stream: BrowserProtocolStream): Promise<void> {
  let buffer = new Uint8Array();

  try {
    for await (const value of stream) {
      const chunk = normalizeStreamChunk(value);
      if (chunk.byteLength === 0) {
        continue;
      }

      const merged = new Uint8Array(buffer.byteLength + chunk.byteLength);
      merged.set(buffer);
      merged.set(chunk, buffer.byteLength);
      buffer = merged;

      while (buffer.byteLength >= PING_MESSAGE_BYTES) {
        const pong = buffer.slice(0, PING_MESSAGE_BYTES);
        buffer = buffer.slice(PING_MESSAGE_BYTES);
        if (!stream.send(pong) && stream.onDrain) {
          await stream.onDrain();
        }
      }
    }
  } finally {
    await stream.close().catch(() => undefined);
  }
}

function normalizeStreamChunk(value: Uint8Array | { subarray(): Uint8Array }): Uint8Array {
  return value instanceof Uint8Array ? value : value.subarray();
}

class Libp2pBrowserTransport implements BrowserTransport {
  readonly peerId: string;
  private started = false;

  constructor(
    private readonly node: Awaited<ReturnType<typeof createLibp2p>>,
    private relayServerAddresses: string[],
  ) {
    this.peerId = node.peerId.toString();
    this.relayServerAddresses = uniqueStrings(relayServerAddresses);
  }

  async start(): Promise<void> {
    if (this.started) return;
    await this.node.start();
    for (const address of this.relayServerAddresses) {
      await this.node.dial(multiaddr(address));
    }
    this.started = true;
  }

  async stop(): Promise<void> {
    await this.node.stop();
    this.started = false;
  }

  multiaddrs(): string[] {
    return browserReachableMultiaddrs(
      this.peerId,
      this.node.getMultiaddrs().map((address) => address.toString()),
    );
  }

  async addRelayServerAddresses(addresses: string[]): Promise<void> {
    const added = addresses.filter((address) => !this.relayServerAddresses.includes(address));
    this.relayServerAddresses = uniqueStrings([...this.relayServerAddresses, ...addresses]);
    if (!this.started || added.length === 0) {
      return;
    }
    await this.listenOnRelayServers(added);
  }

  async dial(addresses: string[], options: { force?: boolean } = {}): Promise<void> {
    if (addresses.length === 0) {
      throw new Error("No bootstrap addresses available to dial");
    }
    await this.node.dial(addresses.map((address) => multiaddr(address)), {
      force: options.force ?? false,
    });
  }

  private async listenOnRelayServers(addresses: string[]): Promise<void> {
    const listenAddresses = relayListenAddresses(addresses).map((address) => multiaddr(address));
    if (listenAddresses.length === 0) {
      return;
    }
    const internals = this.node as unknown as {
      components?: {
        transportManager?: {
          listen(addrs: ReturnType<typeof multiaddr>[]): Promise<void>;
        };
      };
    };
    const transportManager = internals.components?.transportManager;
    const listen = transportManager?.listen;
    if (!listen) {
      throw new Error("Browser transport cannot reserve relay addresses on this libp2p node");
    }
    await listen.call(transportManager, listenAddresses);
  }

  async closePeerConnections(peerId: string, keepAddresses: string[] = []): Promise<void> {
    const peer = peerIdFromString(peerId);
    const keep = new Set(keepAddresses);
    const startedAt = Date.now();

    while (Date.now() - startedAt < CONNECTION_CLEANUP_TIMEOUT_MS) {
      const connections = this.node.getConnections(peer);
      if (browserPeerConnectionCleanupComplete(connections, keepAddresses)) {
        return;
      }

      const targets = connectionsToClose(connections, keep);
      if (targets.length > 0) {
        await Promise.all(targets.map((connection) => closeConnectionAttempt(connection)));
      }

      await sleep(CONNECTION_CLEANUP_POLL_MS);
    }

    const remaining = this.node
      .getConnections(peer)
      .filter((connection) => connection.status === "open")
      .map(connectionSummary);
    throw new Error(
      `Timed out closing non-selected peer connections for ${peerId}: ${remaining.join(", ")}`,
    );
  }

  connectionPaths(peerId: string): BrowserConnectionPath[] {
    const peer = peerIdFromString(peerId);
    return this.node
      .getConnections(peer)
      .filter((connection) => connection.status === "open")
      .map((connection) => {
        const remoteAddress = connection.remoteAddr.toString();
        const rtt = connection.rtt;
        return {
          connectionId: connection.id,
          direction: connection.direction === "outbound" ? "dialer" : "listener",
          transport: classifyTransport(remoteAddress),
          relayInvolved: remoteAddress.includes("/p2p-circuit"),
          remoteAddress,
          status: connection.status,
          direct: connection.direct,
          rttMs: typeof rtt === "number" ? rtt : undefined,
        };
      });
  }

  async registerProtocolHandler(
    protocol: string,
    handler: BrowserProtocolHandler,
    options: BrowserProtocolHandlerOptions = {},
  ): Promise<void> {
    await this.node.handle(
      protocol,
      (stream, connection) => handler(stream, connection.remotePeer.toString()),
      {
        ...options,
        runOnLimitedConnection: true,
        force: true,
      },
    );
  }

  async unregisterProtocolHandler(protocol: string): Promise<void> {
    await this.node.unhandle(protocol);
  }

  async dialProtocol(
    peerId: string,
    addresses: string[],
    protocol: string,
  ): Promise<BrowserProtocolStream> {
    const peer = peerIdFromString(peerId);
    return openBrowserProtocolStream(
      this.node.getConnections(peer),
      addresses,
      protocol,
      (dialAddresses, options) =>
        this.node.dial(dialAddresses.map((address) => multiaddr(address)), options),
    );
  }
}

export function browserReachableMultiaddrs(
  peerId: string,
  observedAddresses: string[],
): string[] {
  return uniqueStrings(observedAddresses).filter((address) => addressTargetsPeer(address, peerId));
}

export async function openBrowserProtocolStream(
  connections: BrowserConnectionCandidate[],
  addresses: string[],
  protocol: string,
  dial: (
    addresses: string[],
    options: { force?: boolean },
  ) => Promise<BrowserConnectionCandidate>,
): Promise<BrowserProtocolStream> {
  for (const connection of connectionCandidates(connections, addresses)) {
    try {
      return await connection.newStream(protocol, { runOnLimitedConnection: true });
    } catch (error) {
      if (!isRetriableConnectionStateError(error)) {
        throw error;
      }
      await connection.close().catch(() => undefined);
    }
  }

  if (addresses.length === 0) {
    throw new Error(`No active connection or bootstrap addresses available for ${protocol}`);
  }

  const connection = await dial(addresses, { force: false });
  try {
    return await connection.newStream(protocol, { runOnLimitedConnection: true });
  } catch (error) {
    if (!isRetriableConnectionStateError(error)) {
      throw error;
    }
    await connection.close().catch(() => undefined);
  }

  const freshConnection = await dial(addresses, { force: true });
  return freshConnection.newStream(protocol, { runOnLimitedConnection: true });
}

function connectionCandidates<T extends BrowserConnectionCandidate>(
  connections: T[],
  addresses: string[],
): T[] {
  const ordered: T[] = [];
  const used = new Set<T>();
  const openConnections = connections.filter((connection) => connection.status === "open");

  for (const address of addresses) {
    for (const connection of openConnections) {
      if (used.has(connection)) {
        continue;
      }
      if (connection.remoteAddr.toString() === address) {
        ordered.push(connection);
        used.add(connection);
      }
    }
  }

  for (const connection of openConnections) {
    if (!used.has(connection)) {
      ordered.push(connection);
    }
  }

  return ordered;
}

function connectionsToClose<T extends BrowserConnectionCandidate>(
  connections: T[],
  keep: Set<string>,
): T[] {
  const retained = retainedConnectionIds(connections, keep);
  return connections.filter(
    (connection) => connection.status !== "closed" && !retained.has(connection.id),
  );
}

export function browserPeerConnectionCleanupComplete(
  connections: BrowserConnectionCleanupCandidate[],
  keepAddresses: string[],
): boolean {
  const keep = new Set(keepAddresses);
  const retained = retainedConnectionIds(connections, keep);
  if (
    connections.some(
      (connection) => connection.status !== "closed" && !retained.has(connection.id),
    )
  ) {
    return false;
  }

  const open = connections.filter((connection) => connection.status === "open");
  if (keep.size === 0) {
    return open.length === 0;
  }

  const seen = new Set<string>();
  for (const connection of open) {
    const address = connection.remoteAddr.toString();
    if (!keep.has(address) || seen.has(address)) {
      return false;
    }
    seen.add(address);
  }
  return seen.size > 0;
}

function retainedConnectionIds<T extends BrowserConnectionCleanupCandidate>(
  connections: T[],
  keep: Set<string>,
): Set<string> {
  const retained = new Set<string>();
  const retainedAddresses = new Set<string>();
  for (const connection of connections) {
    if (connection.status !== "open") {
      continue;
    }
    const address = connection.remoteAddr.toString();
    if (!keep.has(address) || retainedAddresses.has(address)) {
      continue;
    }
    retained.add(connection.id);
    retainedAddresses.add(address);
  }
  return retained;
}

async function closeConnectionAttempt(connection: BrowserConnectionCandidate): Promise<void> {
  if (connection.abort) {
    connection.abort(new Error(CONNECTION_CLOSE_REASON));
    return;
  }

  await Promise.race([
    connection.close().catch(() => undefined),
    sleep(CONNECTION_CLOSE_ATTEMPT_MS),
  ]);
}

function connectionSummary(connection: BrowserConnectionCandidate): string {
  return `${connection.id}:${connection.status}:${connection.remoteAddr.toString()}`;
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

function isRetriableConnectionStateError(error: unknown): boolean {
  if (!(error instanceof Error)) {
    return false;
  }
  const message = error.message.toLowerCase();
  return (
    message.includes("muxer is \"closing\"") ||
    message.includes("muxer is \"closed\"") ||
    message.includes("connection is \"closing\"") ||
    message.includes("connection is \"closed\"") ||
    message.includes("connection is closing") ||
    message.includes("connection is closed")
  );
}

function classifyTransport(address: string): BrowserConnectionTransport {
  if (address.includes("/webrtc-direct")) {
    return "webrtc_direct";
  }
  if (address.includes("/webrtc")) {
    return "webrtc";
  }
  if (address.includes("/webtransport")) {
    return "webtransport";
  }
  if (address.includes("/ws") || address.includes("/wss")) {
    return "websocket";
  }
  if (address.includes("/quic")) {
    return "quic";
  }
  if (address.includes("/tcp/")) {
    return "tcp";
  }
  return "unknown";
}

function relayListenAddresses(addresses: string[]): string[] {
  return uniqueStrings(addresses).map((address) => `${address}/p2p-circuit`);
}

function addressTargetsPeer(address: string, peerId: string): boolean {
  return address.endsWith(`/p2p/${peerId}`) || address.includes(`/p2p/${peerId}/`);
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    if (seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}

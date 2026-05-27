import { circuitRelayTransport } from "@libp2p/circuit-relay-v2";
import { generateKeyPairFromSeed } from "@libp2p/crypto/keys";
import { identify } from "@libp2p/identify";
import { noise } from "@libp2p/noise";
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

export type BrowserTransport = {
  peerId: string;
  start(): Promise<void>;
  stop(): Promise<void>;
  multiaddrs(): string[];
  dial(addresses: string[]): Promise<void>;
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

export function supportedBrowserTransports(): BrowserTransportName[] {
  return ["websocket", "webrtc", "webrtc_direct", "circuit_relay_v2"];
}

export async function createBrowserLibp2pTransport(
  options: CreateBrowserLibp2pTransportOptions,
): Promise<BrowserTransport> {
  const { webRTC, webRTCDirect } = await import("@libp2p/webrtc");
  const privateKey = await generateKeyPairFromSeed("Ed25519", await derivePeerSeed(options.seed));
  const relayListenAddrs = (options.relayServerAddresses ?? []).map(
    (address) => `${address}/p2p-circuit`,
  );
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
  return new Libp2pBrowserTransport(node, options.relayServerAddresses ?? []);
}

class Libp2pBrowserTransport implements BrowserTransport {
  readonly peerId: string;
  private started = false;

  constructor(
    private readonly node: Awaited<ReturnType<typeof createLibp2p>>,
    private readonly relayServerAddresses: string[],
  ) {
    this.peerId = node.peerId.toString();
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
    const addresses = this.node.getMultiaddrs().map((address) => address.toString());
    if (addresses.some((address) => address.includes("/p2p-circuit"))) {
      return addresses;
    }
    return [
      ...addresses,
      ...this.relayServerAddresses.map(
        (address) => `${address}/p2p-circuit/p2p/${this.peerId}`,
      ),
    ];
  }

  async dial(addresses: string[]): Promise<void> {
    if (addresses.length === 0) {
      throw new Error("No bootstrap addresses available to dial");
    }
    await this.node.dial(addresses.map((address) => multiaddr(address)));
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
    _peerId: string,
    addresses: string[],
    protocol: string,
  ): Promise<BrowserProtocolStream> {
    if (addresses.length === 0) {
      throw new Error(`No bootstrap addresses available for ${protocol}`);
    }
    return this.node.dialProtocol(
      addresses.map((address) => multiaddr(address)),
      protocol,
      { runOnLimitedConnection: true },
    );
  }
}

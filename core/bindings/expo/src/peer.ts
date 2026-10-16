import module from "./AukiSdkExpoModule";
import type {
  AukiDiscoveryCandidateInfo,
  AukiDiscoveryModeName,
  AukiPeerReachabilityModeName,
  PeerRoute,
} from "./AukiSdkExpo.types";
import { aukiError, normalizeAukiError } from "./errors";

export type { PeerRoute };

export type DiscoveryMode = "discoverOnly" | "discoverAndAdvertise";
export type Reachability = "outboundOnly" | "relay";

export type PeerStartOptions = {
  discovery?: DiscoveryMode;
  reachability?: Reachability;
};

export type MessageChannel = {
  variant: string;
  owner_peer_id: string;
  resource_id: string;
  clock: {
    peer_id: string;
    id: string;
    hash: string;
  };
};

export type StreamEntry = {
  timestampNs: bigint;
  sequence: bigint;
  payload: Uint8Array;
  jpeg: Uint8Array | null;
};

export type BlobReceipt = {
  bytes: Uint8Array;
  sha256: string;
  relayed: boolean;
  peerId: string;
};

const discoveryNames: Record<DiscoveryMode, AukiDiscoveryModeName> = {
  discoverOnly: "DiscoverOnly",
  discoverAndAdvertise: "DiscoverAndAdvertise",
};

const reachabilityNames: Record<Reachability, AukiPeerReachabilityModeName> = {
  outboundOnly: "OutboundOnly",
  relay: "RelayBacked",
};

let openChain: Promise<unknown> = Promise.resolve();

function enqueueOpen<T>(op: () => Promise<T>): Promise<T> {
  const run = openChain.then(op, op);
  openChain = run.then(() => undefined, () => undefined);
  return run;
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = "";
  const step = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += step) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + step));
  }
  return btoa(binary);
}

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function bytesFromBase64(value: unknown): Uint8Array | null {
  if (typeof value !== "string" || value.length === 0) return null;
  return decodeBase64(value);
}

function parseJson(value: string): unknown {
  return JSON.parse(value);
}

export class MessageSender {
  /** @internal */
  constructor(private readonly handle: string) {}

  send(message: { type: string; timestampNs: bigint; payload: Uint8Array }): Promise<void> {
    return module.messageSend(
      this.handle,
      message.type,
      message.timestampNs.toString(),
      encodeBase64(message.payload),
    ).catch(error => {
      throw normalizeAukiError(error, "Message send failed");
    });
  }

  async close(): Promise<void> {
    await module.messageClose(this.handle);
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}

export class StreamSubscription implements AsyncIterable<StreamEntry> {
  private cancelled = false;

  /** @internal */
  constructor(private readonly id: string) {}

  async next(): Promise<StreamEntry | null> {
    while (!this.cancelled) {
      const raw = await module.streamNext(this.id);
      if (raw == null) return null;
      const parsed = parseJson(raw) as {
        kind?: string;
        entry?: {
          timestampNs?: string;
          sequence?: string;
          payloadBase64?: string | null;
          jpegBase64?: string | null;
        } | null;
      };
      if (parsed.kind !== "entry" || !parsed.entry) continue;
      return {
        timestampNs: BigInt(parsed.entry.timestampNs ?? "0"),
        sequence: BigInt(parsed.entry.sequence ?? "0"),
        payload: bytesFromBase64(parsed.entry.payloadBase64) ?? new Uint8Array(),
        jpeg: bytesFromBase64(parsed.entry.jpegBase64),
      };
    }
    return null;
  }

  async cancel(): Promise<void> {
    if (this.cancelled) return;
    this.cancelled = true;
    await module.streamCancel(this.id).catch(() => undefined);
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.cancel();
  }

  async *[Symbol.asyncIterator](): AsyncGenerator<StreamEntry> {
    try {
      while (!this.cancelled) {
        const entry = await this.next();
        if (entry == null) return;
        yield entry;
      }
    } finally {
      await this.cancel();
    }
  }
}

export class Peer {
  readonly info: PeerInfo;
  readonly catalog: PeerCatalog;
  readonly registry: PeerRegistry;
  readonly blobs: PeerBlobs;
  readonly streams: PeerStreams;
  readonly messages: PeerMessages;

  /** @internal */
  private constructor(
    private readonly handle: string,
    readonly peerId: string,
    readonly domainId: string,
  ) {
    this.info = new PeerInfo(handle);
    this.catalog = new PeerCatalog(handle);
    this.registry = new PeerRegistry(handle);
    this.blobs = new PeerBlobs(handle);
    this.streams = new PeerStreams(handle);
    this.messages = new PeerMessages(handle);
  }

  /** @internal */
  static async start(sessionId: string, domainId: string, options: PeerStartOptions = {}): Promise<Peer> {
    const reachability = options.reachability ? reachabilityNames[options.reachability] : null;
    if (options.reachability && !reachability) {
      throw aukiError("input", "Unknown peer reachability");
    }
    let handle: string;
    try {
      handle = options.discovery
        ? await module.startPeerWithDiscovery(
          sessionId,
          domainId,
          discoveryNames[options.discovery],
          reachability,
        )
        : await module.startPeer(sessionId, domainId, reachability);
    } catch (error) {
      throw normalizeAukiError(error, "Peer failed to start");
    }
    try {
      const [peerId, resolvedDomain] = await Promise.all([
        module.peerId(handle),
        module.domainId(handle),
      ]);
      return new Peer(handle, peerId, resolvedDomain);
    } catch (error) {
      await module.shutdown(handle).catch(() => undefined);
      throw normalizeAukiError(error, "Peer failed to start");
    }
  }

  discover(protocolId?: string): Promise<AukiDiscoveryCandidateInfo[]> {
    const pending = protocolId
      ? module.discoverProtocol(this.handle, protocolId)
      : module.discover(this.handle);
    return pending.catch(error => {
      throw normalizeAukiError(error, "Discovery failed");
    });
  }

  async shutdown(): Promise<void> {
    await module.shutdown(this.handle);
  }

  waitUntilStopped(): Promise<void> {
    return module.waitUntilStopped(this.handle).catch(error => {
      throw normalizeAukiError(error, "Peer stopped unexpectedly");
    });
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.shutdown();
  }
}

class PeerInfo {
  constructor(private readonly handle: string) {}

  fetch(route: PeerRoute): Promise<unknown> {
    return enqueueOpen(async () => parseJson(await module.infoFetch(this.handle, route)));
  }
}

class PeerCatalog {
  constructor(private readonly handle: string) {}

  fetchResources(route: PeerRoute, variants: string[]): Promise<unknown> {
    return enqueueOpen(async () => parseJson(
      await module.catalogFetchResources(this.handle, route, variants),
    ));
  }
}

class PeerRegistry {
  constructor(private readonly handle: string) {}

  list(route: PeerRoute, kind: string): Promise<unknown> {
    return enqueueOpen(async () => parseJson(
      await module.registryList(this.handle, route, kind),
    ));
  }

  fetch(route: PeerRoute, kind: string, id: string, hash: string): Promise<unknown> {
    return enqueueOpen(async () => parseJson(
      await module.registryFetch(this.handle, route, kind, id, hash),
    ));
  }
}

class PeerBlobs {
  constructor(private readonly handle: string) {}

  fetch(route: PeerRoute, sha256: string): Promise<BlobReceipt> {
    return enqueueOpen(async () => {
      const receipt = parseJson(await module.blobFetch(this.handle, route, sha256)) as {
        bytesBase64?: string;
        sha256?: string;
        relayed?: boolean;
        peerId?: string;
      };
      if (!receipt.bytesBase64) throw aukiError("response", `blob ${sha256}: missing bytes`);
      return {
        bytes: decodeBase64(receipt.bytesBase64),
        sha256: receipt.sha256 ?? sha256,
        relayed: Boolean(receipt.relayed),
        peerId: receipt.peerId ?? route.peerId,
      };
    });
  }
}

class PeerStreams {
  constructor(private readonly handle: string) {}

  async subscribe(
    route: PeerRoute,
    payloadKind: string,
    request: unknown,
    options: { signal?: AbortSignal } = {},
  ): Promise<StreamSubscription> {
    if (options.signal?.aborted) throw aukiError("cancelled", "Stream request cancelled");
    const id = await module.streamSubscribe(
      this.handle,
      route,
      payloadKind,
      JSON.stringify(request),
    );
    const subscription = new StreamSubscription(id);
    if (options.signal?.aborted) {
      await subscription.cancel();
      throw aukiError("cancelled", "Stream request cancelled");
    }
    options.signal?.addEventListener("abort", () => {
      void subscription.cancel();
    }, { once: true });
    return subscription;
  }
}

class PeerMessages {
  constructor(private readonly handle: string) {}

  async open(route: PeerRoute, channel: MessageChannel): Promise<MessageSender> {
    const id = await module.messageOpen(this.handle, route, JSON.stringify(channel));
    return new MessageSender(id);
  }
}

import { NativeModule, registerWebModule } from "expo";

import type {
  AukiDiscoveryCandidateInfo,
  AukiDiscoveryModeName,
  AukiDomainInfo,
  AukiExactTarget,
  AukiSdkExpoModuleEvents,
} from "./AukiSdkExpo.types";
import { loadAukiSdkWasm, type AukiSdkWasm } from "./web/loadAukiSdkWasm";

type Session = Awaited<ReturnType<AukiSdkWasm["AukiUserSession"]["loginDev"]>>;
type Peer = Awaited<ReturnType<Session["startPeer"]>>;
type StreamClient = import("./web/generated/auki_sdk_web.js").AukiStreamClient;
type CatalogClient = import("./web/generated/auki_sdk_web.js").AukiCatalogClient;
type StreamSub = Awaited<ReturnType<StreamClient["subscribeExact"]>>;

function newId(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;
}

function mapCandidate(candidate: {
  peerId: string;
  routes: string[];
  servedProtocols: string[];
  expiresAt: string;
  source: string;
}): AukiDiscoveryCandidateInfo {
  return {
    peerId: candidate.peerId,
    routes: [...candidate.routes],
    servedProtocols: [...candidate.servedProtocols],
    expiresAt: candidate.expiresAt,
    source: candidate.source,
  };
}

class AukiSdkExpoModule extends NativeModule<AukiSdkExpoModuleEvents> {
  private wasm: AukiSdkWasm | null = null;
  private sessions = new Map<string, Session>();
  private peers = new Map<string, Peer>();
  private streams = new Map<string, StreamSub>();

  private async sdk(): Promise<AukiSdkWasm> {
    if (!this.wasm) {
      this.wasm = await loadAukiSdkWasm();
    }
    return this.wasm;
  }

  private session(sessionId: string): Session {
    const session = this.sessions.get(sessionId);
    if (!session) {
      throw new Error(`unknown session: ${sessionId}`);
    }
    return session;
  }

  private peer(peerHandle: string): Peer {
    const peer = this.peers.get(peerHandle);
    if (!peer) {
      throw new Error(`unknown peer: ${peerHandle}`);
    }
    return peer;
  }

  async loginDev(email: string, password: string): Promise<string> {
    const sdk = await this.sdk();
    const session = await sdk.AukiUserSession.loginDev(email, password);
    const id = newId("session");
    this.sessions.set(id, session);
    return id;
  }

  async loginWithDomainAccessToken(
    apiBaseUrl: string,
    ddsBaseUrl: string,
    dmsBaseUrl: string,
    domainAccessToken: string,
  ): Promise<string> {
    const sdk = await this.sdk();
    const session = await sdk.AukiUserSession.loginWithDomainAccessToken(
      apiBaseUrl,
      ddsBaseUrl,
      dmsBaseUrl,
      domainAccessToken,
    );
    const id = newId("session");
    this.sessions.set(id, session);
    return id;
  }

  async accessibleDomains(sessionId: string): Promise<AukiDomainInfo[]> {
    const domains = await this.session(sessionId).accessibleDomains();
    return domains.map((domain) => ({
      id: domain.id,
      name: domain.name ?? null,
      description: domain.description ?? null,
      organizationId: domain.organizationId ?? null,
    }));
  }

  async startPeer(sessionId: string, domainId: string): Promise<string> {
    const peer = await this.session(sessionId).startPeer(domainId);
    const id = newId("peer");
    this.peers.set(id, peer);
    return id;
  }

  async startPeerWithDiscovery(
    sessionId: string,
    domainId: string,
    mode: AukiDiscoveryModeName,
  ): Promise<string> {
    const sdk = await this.sdk();
    const discoveryMode =
      mode === "DiscoverAndAdvertise"
        ? sdk.AukiDiscoveryMode.DiscoverAndAdvertise
        : sdk.AukiDiscoveryMode.DiscoverOnly;
    const peer = await this.session(sessionId).startPeerWithDiscovery(
      domainId,
      discoveryMode,
    );
    const id = newId("peer");
    this.peers.set(id, peer);
    return id;
  }

  async peerId(peerHandle: string): Promise<string> {
    return this.peer(peerHandle).peerId;
  }

  async domainId(peerHandle: string): Promise<string> {
    return this.peer(peerHandle).domainId;
  }

  async discover(peerHandle: string): Promise<AukiDiscoveryCandidateInfo[]> {
    const candidates = await this.peer(peerHandle).discover();
    // wasm-bindgen objects must be freed (standard-protocols does this per candidate).
    try {
      return candidates.map(mapCandidate);
    } finally {
      for (const candidate of candidates) {
        candidate.free();
      }
    }
  }

  async discoverProtocol(
    peerHandle: string,
    protocolId: string,
  ): Promise<AukiDiscoveryCandidateInfo[]> {
    const candidates = await this.peer(peerHandle).discoverProtocol(protocolId);
    try {
      return candidates.map(mapCandidate);
    } finally {
      for (const candidate of candidates) {
        candidate.free();
      }
    }
  }

  async infoFetchExact(
    peerHandle: string,
    target: AukiExactTarget,
  ): Promise<string> {
    const sdk = await this.sdk();
    const info = await new sdk.AukiInfoClient(this.peer(peerHandle)).fetchExact(
      target,
    );
    // AukiParticipantInfo.sessionNowNs is bigint — JSON.stringify rejects it.
    return JSON.stringify(info, (_key, value) =>
      typeof value === "bigint" ? value.toString() : value,
    );
  }

  async catalogFetchResourcesExact(
    peerHandle: string,
    target: AukiExactTarget,
    variants: string[],
  ): Promise<string> {
    const sdk = await this.sdk();
    const resources = await new sdk.AukiCatalogClient(
      this.peer(peerHandle),
    ).fetchResourcesExact(
      target,
      variants as Parameters<CatalogClient["fetchResourcesExact"]>[1],
    );
    return JSON.stringify(resources);
  }

  async streamSubscribeExact(
    peerHandle: string,
    target: AukiExactTarget,
    payloadKind: string,
    requestJson: string,
  ): Promise<string> {
    const sdk = await this.sdk();
    const request = JSON.parse(requestJson);
    const subscription = await new sdk.AukiStreamClient(
      this.peer(peerHandle),
    ).subscribeExact(
      target,
      payloadKind as Parameters<StreamClient["subscribeExact"]>[1],
      request,
    );
    const id = newId("stream");
    this.streams.set(id, subscription);
    return id;
  }

  async streamNext(subscriptionId: string): Promise<string | null> {
    const subscription = this.streams.get(subscriptionId);
    if (!subscription) {
      throw new Error(`unknown stream subscription: ${subscriptionId}`);
    }
    const next = await subscription.next();
    if (next == null) {
      return null;
    }
    return JSON.stringify(next);
  }

  async streamCancel(subscriptionId: string): Promise<void> {
    const subscription = this.streams.get(subscriptionId);
    if (!subscription) {
      return;
    }
    await subscription.cancel();
    this.streams.delete(subscriptionId);
  }

  async shutdown(peerHandle: string): Promise<void> {
    const peer = this.peers.get(peerHandle);
    if (!peer) {
      return;
    }
    await peer.shutdown();
    this.peers.delete(peerHandle);
  }

  async waitStopped(peerHandle: string): Promise<void> {
    await this.peer(peerHandle).waitStopped();
  }
}

export default registerWebModule(AukiSdkExpoModule, "AukiSdkExpo");

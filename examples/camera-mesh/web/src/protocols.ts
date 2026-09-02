import {
  AukiBlobClient,
  AukiBlobEndpoint,
  AukiCatalogClient,
  AukiCatalogEndpoint,
  AukiInfoClient,
  AukiInfoEndpoint,
  AukiMessageClient,
  AukiMessageEndpoint,
  AukiPeer,
  AukiRegistryClient,
  AukiRegistryEndpoint,
  prepareCatalogResources,
  prepareRegistryEntry,
  type AukiAuthenticatedPeer,
  type AukiBlobProviderRequest,
  type AukiCatalogResource,
  type AukiCatalogResourcesResponse,
  type AukiClockRegistryEntry,
  type AukiExactTarget,
  type AukiFrameRegistryEntry,
  type AukiMessageChannelResource,
  type AukiMessageEvent,
  type AukiParticipantInfo,
  type AukiRegistryEntry,
  type AukiRegistryEntryEnvelope,
  type AukiRegistryKind,
  type AukiRegistryProviderRequest,
  type AukiRegistryRef,
  type AukiSensorRegistryEntry,
} from "../pkg-web/auki_sdk_web.js";

export const CAMERA_RESOURCE_ID = "camera/main";
export const CAMERA_CONTROL_RESOURCE_ID = "camera/control";
export const CAMERA_REPLY_RESOURCE_ID = "camera/replies";
export const CAMERA_WIDTH = 480;
export const CAMERA_HEIGHT = 270;
export const CAMERA_RATE_HZ = 5;

const APP = "auki-camera-mesh";
const APP_VERSION = "0.1.0";
const CLOCK_ID = "camera/utc";
const FRAME_ID = "camera/optical";
const MAX_STAGED_BLOBS = 8;
const MAX_PENDING_SNAPSHOTS = 16;
const MAX_BLOB_BYTES = 20 * 1024 * 1024;
const SNAPSHOT_TIMEOUT_MS = 45_000;
const EMPTY_BYTES = new Uint8Array();

type Awaitable<T> = T | Promise<T>;
type Endpoint = { close(): Promise<void>; free(): void };
type Client = { free(): void };

export type CameraProtocolRole = "publisher" | "viewer";
export type CameraControlType =
  | "camera.pause"
  | "camera.resume"
  | "camera.request_snapshot";

/** The application owns the session allowlist and its pending-approval state. */
export interface AccessPolicy {
  isAllowed(requester: AukiAuthenticatedPeer): boolean;
  requestApproval(requester: AukiAuthenticatedPeer): void;
}

export interface SnapshotReplyAddress {
  readonly target: {
    readonly peerId: string;
    readonly routes: readonly string[];
  };
  readonly channel: AukiMessageChannelResource;
}

export type CameraControl =
  | { readonly type: "camera.pause" }
  | { readonly type: "camera.resume" }
  | {
      readonly type: "camera.request_snapshot";
      readonly requestId: string;
    };

export interface SnapshotRequest {
  readonly event: AukiMessageEvent;
  readonly requester: AukiAuthenticatedPeer;
  readonly requestId: string;
  /** Authenticated-requester-bound address supplied by the viewer. */
  readonly reply: SnapshotReplyAddress;
}

export interface StagedBlob {
  readonly sha256: string;
  readonly size: number;
}

export interface SnapshotReady extends StagedBlob {
  readonly requester: AukiAuthenticatedPeer;
  readonly requestId: string;
  readonly reply: SnapshotReplyAddress;
}

export interface SnapshotAvailable extends StagedBlob {
  readonly event: AukiMessageEvent;
  readonly publisher: AukiAuthenticatedPeer;
  readonly requestId: string;
}

export interface CameraControlHandlers {
  pause(event: AukiMessageEvent): Awaitable<void>;
  resume(event: AukiMessageEvent): Awaitable<void>;
  /** Return immutable snapshot bytes to make them available through Blob v1. */
  requestSnapshot(request: SnapshotRequest): Awaitable<Uint8Array | undefined>;
  /** Announce the hash through the requester's reverse Message channel here. */
  snapshotReady?(snapshot: SnapshotReady): Awaitable<void>;
  /** Called only for a pending request bound to this authenticated publisher. */
  snapshotAvailable?(snapshot: SnapshotAvailable): Awaitable<void>;
  /** Called when a sent snapshot request receives no announcement in time. */
  snapshotExpired?(requestId: string): Awaitable<void>;
  ignored?(event: AukiMessageEvent, reason: string): void;
}

export interface CameraProtocolOptions {
  readonly role: CameraProtocolRole;
  readonly displayName: string;
  readonly access: AccessPolicy;
  readonly controls: CameraControlHandlers;
  readonly sessionId?: string;
  readonly event?: (message: string) => void;
}

export interface VerifiedRegistryEntry<T extends AukiRegistryEntry> {
  readonly kind: AukiRegistryKind;
  readonly id: string;
  readonly hash: string;
  readonly canonicalJson: string;
  readonly entry: T;
  readonly ref: AukiRegistryRef;
}

export interface CameraRegistryMetadata {
  readonly sensor: VerifiedRegistryEntry<AukiSensorRegistryEntry>;
  readonly clock: VerifiedRegistryEntry<AukiClockRegistryEntry>;
  readonly frame: VerifiedRegistryEntry<AukiFrameRegistryEntry>;
}

export interface RemoteCameraMetadata extends CameraRegistryMetadata {
  readonly info: AukiParticipantInfo;
  readonly catalog: AukiCatalogResource;
  readonly controlChannel: AukiMessageChannelResource;
}

export interface VerifiedBlob {
  readonly peerId: string;
  readonly sha256: string;
  readonly bytes: Uint8Array;
  readonly relayed: boolean;
  readonly verified: true;
}

export interface CameraProtocolIds {
  readonly info: string;
  readonly catalogResources: string;
  readonly catalogMaps: string;
  readonly registry: string;
  readonly blob: string;
  readonly message: string;
}

type CameraCatalogDescription = {
  readonly row: AukiCatalogResource;
  readonly sensor: AukiRegistryRef;
  readonly clock: AukiRegistryRef;
  readonly frame: AukiRegistryRef;
};

type PendingSnapshot = {
  readonly peerId: string;
  readonly timeout?: number;
};

/** Info/Catalog/Registry/Blob/Message composition for one camera-mesh peer. */
export class CameraProtocols {
  readonly metadata: CameraRegistryMetadata;
  readonly replyChannel?: AukiMessageChannelResource;

  private readonly endpoints: Endpoint[] = [];
  private readonly clients: Client[] = [];
  private readonly blobs = new Map<string, Uint8Array>();
  private readonly operations = new Set<Promise<unknown>>();
  private readonly pendingSnapshots = new Map<string, PendingSnapshot>();
  private readonly controlChannel: AukiMessageChannelResource;
  private readonly sessionId: string;
  private readonly publisherCatalog?: AukiCatalogResourcesResponse;
  private infoClient?: AukiInfoClient;
  private catalogClient?: AukiCatalogClient;
  private registryClient?: AukiRegistryClient;
  private blobClient?: AukiBlobClient;
  private messageClient?: AukiMessageClient;
  private receiver?: ReturnType<AukiMessageEndpoint["declare"]>;
  private receiverTask?: Promise<void>;
  private cameraAvailable = false;
  private closing = false;
  private closePromise?: Promise<void>;

  private constructor(
    private readonly peer: AukiPeer,
    private readonly options: CameraProtocolOptions,
  ) {
    this.sessionId = options.sessionId ?? globalThis.crypto.randomUUID();
    this.metadata = prepareCameraMetadata(peer.peerId, this.sessionId);
    this.controlChannel = {
      variant: "message_channel",
      owner_peer_id: peer.peerId,
      resource_id: CAMERA_CONTROL_RESOURCE_ID,
      clock: this.metadata.clock.ref,
    };
    if (options.role === "publisher") {
      this.publisherCatalog = prepareCatalogResources({
        resources: [
          this.cameraCatalog(),
          this.controlChannel as unknown as AukiCatalogResource,
        ],
      });
    }
    if (options.role === "viewer") {
      this.replyChannel = {
        variant: "message_channel",
        owner_peer_id: peer.peerId,
        resource_id: CAMERA_REPLY_RESOURCE_ID,
        clock: this.metadata.clock.ref,
      };
    }
  }

  static async mount(peer: AukiPeer, options: CameraProtocolOptions): Promise<CameraProtocols> {
    const protocols = new CameraProtocols(peer, options);
    try {
      protocols.mountEndpoints();
      return protocols;
    } catch (error) {
      await protocols.close().catch(() => undefined);
      throw error;
    }
  }

  get protocolIds(): CameraProtocolIds {
    const info = required(this.infoClient, "Info client");
    const catalog = required(this.catalogClient, "Catalog client");
    const registry = required(this.registryClient, "Registry client");
    const blob = required(this.blobClient, "Blob client");
    const message = required(this.messageClient, "Message client");
    return {
      info: info.protocol,
      catalogResources: catalog.resourceProtocol,
      catalogMaps: catalog.mapsProtocol,
      registry: registry.protocol,
      blob: blob.protocol,
      message: message.protocol,
    };
  }

  get servedProtocols(): string[] {
    return Object.values(this.protocolIds);
  }

  setCameraAvailable(available: boolean): void {
    this.assertOpen();
    if (this.options.role !== "publisher" && available) {
      throw new Error("a viewer cannot advertise a camera");
    }
    this.cameraAvailable = available;
  }

  fetchInfo(target: AukiExactTarget): Promise<AukiParticipantInfo> {
    return this.track(() => this.fetchInfoImpl(target));
  }

  resolveRemoteMetadata(target: AukiExactTarget): Promise<RemoteCameraMetadata> {
    return this.track(async () => {
      const [info, catalog] = await Promise.all([
        this.fetchInfoImpl(target),
        required(this.catalogClient, "Catalog client").fetchResourcesExact(
          target,
          ["sensor_log", "message_channel"],
        ),
      ]);
      assert(info.app === APP, `Info app must be ${APP}`);
      assert(info.appVersion === APP_VERSION, `Info app version must be ${APP_VERSION}`);
      this.emit(
        `Catalog fetched ${catalog.resources.length} row(s): ${catalog.resources
          .map((resource) => `${String(resource.variant)}:${String(resource["resource_id"])}`)
          .join(", ") || "none"}`,
      );
      const camera = parseCameraCatalog(catalog.resources, target.peerId);
      assert(info.sessionClockId === camera.clock.id, "Info and Catalog use different clocks");
      assert(info.sessionClockHash === camera.clock.hash, "Info and Catalog clock hashes differ");
      const controlChannel = parseControlChannel(catalog.resources, target.peerId, camera.clock);
      const registry = required(this.registryClient, "Registry client");
      // Resolve these small immutable entries in order. Exact relay routes may
      // reuse one authenticated connection, so racing three new request streams
      // here can make one runtime close a connection another request still uses.
      const sensorEntry = await registry.fetchExact(
        target,
        "sensor",
        camera.sensor.id,
        camera.sensor.hash,
      ) as AukiSensorRegistryEntry;
      const clockEntry = await registry.fetchExact(
        target,
        "clock",
        camera.clock.id,
        camera.clock.hash,
      ) as AukiClockRegistryEntry;
      const frameEntry = await registry.fetchExact(
        target,
        "frame",
        camera.frame.id,
        camera.frame.hash,
      ) as AukiFrameRegistryEntry;
      assert(clockEntry.session_id === info.sessionId,
        "Info and Clock Registry entry use different sessions");
      const sensor = verifyRegistryEntry("sensor", camera.sensor, sensorEntry);
      const clock = verifyRegistryEntry("clock", camera.clock, clockEntry);
      const frame = verifyRegistryEntry("frame", camera.frame, frameEntry);
      validateCameraEntries(sensor.entry, clock.entry, frame.entry, camera);
      return { info, catalog: camera.row, controlChannel, sensor, clock, frame };
    });
  }

  sendControl(
    target: AukiExactTarget,
    channel: AukiMessageChannelResource,
    control: CameraControl,
  ): Promise<void> {
    return this.track(async () => {
      assert(this.options.role === "viewer", "only a viewer sends camera controls");
      validateControlChannel(channel, target.peerId);
      let requestId: string | undefined;
      let reply: SnapshotReplyAddress | undefined;
      if (control.type === "camera.request_snapshot") {
        requestId = control.requestId;
        validateRequestId(requestId);
        assert(
          this.pendingSnapshots.size < MAX_PENDING_SNAPSHOTS,
          "too many snapshot requests are awaiting replies",
        );
        assert(!this.pendingSnapshots.has(requestId), "snapshot requestId is already pending");
        reply = {
          target: {
            peerId: this.peer.peerId,
            routes: [this.peer.tcpRoute, this.peer.wssRoute],
          },
          channel: required(this.replyChannel, "snapshot reply channel"),
        };
        validateReplyAddress(reply, this.peer.peerId);
        this.pendingSnapshots.set(requestId, { peerId: target.peerId });
      }
      try {
        const sender = await required(this.messageClient, "Message client").openExact(target, channel);
        try {
          assert(sender.remotePeer.peerId === target.peerId, "Message authenticated the wrong peer");
          assert(this.sameDomain(sender.remotePeer), "Message peer does not share the selected Domain");
          await sender.send(control.type, utcNowNs(), encodeControl(control, reply));
          if (requestId !== undefined) this.armPendingSnapshot(requestId, target.peerId);
        } finally {
          try {
            await sender.close();
          } finally {
            sender.free();
          }
        }
      } catch (error) {
        if (requestId !== undefined) this.removePendingSnapshot(requestId);
        throw error;
      }
    });
  }

  stageBlob(bytes: Uint8Array): Promise<StagedBlob> {
    return this.track(() => this.stageBlobImpl(bytes));
  }

  forgetBlob(sha256: string): boolean {
    return this.blobs.delete(sha256);
  }

  fetchVerifiedBlob(target: AukiExactTarget, sha256: string): Promise<VerifiedBlob> {
    return this.track(async () => {
      assert(isSha256(sha256), "Blob hash must be 64 lowercase hexadecimal characters");
      const receipt = await required(this.blobClient, "Blob client").fetchExact(target, sha256);
      assert(receipt.peerId === target.peerId, "Blob authenticated the wrong peer");
      assert(receipt.sha256 === sha256, "Blob receipt returned the wrong hash");
      assert(await sha256Hex(receipt.bytes) === sha256, "Blob failed local SHA-256 verification");
      return { ...receipt, verified: true };
    });
  }

  close(): Promise<void> {
    this.closePromise ??= this.closeOwned();
    return this.closePromise;
  }

  private mountEndpoints(): void {
    this.infoClient = new AukiInfoClient(this.peer);
    this.clients.push(this.infoClient);
    this.catalogClient = new AukiCatalogClient(this.peer);
    this.clients.push(this.catalogClient);
    this.registryClient = new AukiRegistryClient(this.peer);
    this.clients.push(this.registryClient);
    this.blobClient = new AukiBlobClient(this.peer);
    this.clients.push(this.blobClient);
    this.messageClient = new AukiMessageClient(this.peer);
    this.clients.push(this.messageClient);

    const info = AukiInfoEndpoint.mount(this.peer, (requester) => {
      if (!this.sameDomain(requester)) return null;
      return {
        app: APP,
        appVersion: APP_VERSION,
        name: this.options.displayName,
        sessionId: this.sessionId,
        sessionClockId: this.metadata.clock.id,
        sessionClockHash: this.metadata.clock.hash,
        sessionNowNs: utcNowNs(),
        peerId: this.peer.peerId,
        appInstance: `browser/${this.options.role}`,
      };
    });
    this.endpoints.push(info);

    const message = AukiMessageEndpoint.mount(this.peer);
    this.endpoints.push(message);
    const receivedChannel = this.options.role === "publisher"
      ? this.controlChannel
      : required(this.replyChannel, "snapshot reply channel");
    this.receiver = message.declare(receivedChannel, 16);
    this.receiverTask = this.drainMessages(this.receiver);

    const catalog = AukiCatalogEndpoint.mount(
      this.peer,
      (requester, request) => {
        if (!this.sameDomain(requester)) return { resources: [] };
        if (this.options.role !== "publisher") return { resources: [] };
        if (!this.allowed(requester)) {
          this.requestApproval(requester);
          return { resources: [] };
        }
        if (!this.cameraAvailable) {
          this.emit("Catalog request allowed while the camera is unavailable");
          return { resources: [] };
        }
        const snapshot = required(this.publisherCatalog, "publisher Catalog");
        this.emit(
          `Catalog exposed ${snapshot.resources
            .map((resource) => `${String(resource.variant)}:${String(resource["resource_id"])}`)
            .join(", ")} to ${requester.peerId} for ${request.variants.join(", ") || "all variants"}`,
        );
        return snapshot;
      },
      (requester) => {
        if (!this.sameDomain(requester) || !this.allowed(requester)) return { resources: [] };
        return { resources: [] };
      },
    );
    this.endpoints.push(catalog);

    const registry = AukiRegistryEndpoint.mount(this.peer, (requester, request) =>
      this.provideRegistry(requester, request));
    this.endpoints.push(registry);

    const blob = AukiBlobEndpoint.mount(this.peer, (requester, request) =>
      this.provideBlob(requester, request));
    this.endpoints.push(blob);

  }

  private cameraCatalog(): AukiCatalogResource {
    return {
      variant: "sensor_log",
      source_peer_id: this.peer.peerId,
      writer_peer_id: this.peer.peerId,
      resource_id: CAMERA_RESOURCE_ID,
      state: "live",
      head: { kind: "rolling", retention_ns: 1_000_000_000n / BigInt(CAMERA_RATE_HZ) },
      available: { bytes: 0n, entries: 0n, duration_ns: 0n },
      sensor: {
        kind: "camera",
        type: "rgb",
        sensor_id: this.metadata.sensor.id,
        sensor_hash: this.metadata.sensor.hash,
      },
      manifest: { clock: this.metadata.clock.ref, frame: this.metadata.frame.ref },
    };
  }

  private provideRegistry(
    requester: AukiAuthenticatedPeer,
    request: AukiRegistryProviderRequest,
  ) {
    if (!this.sameDomain(requester) || !this.allowed(requester)) {
      return { op: "error" as const, reason: "access_denied" };
    }
    const entries = [this.metadata.sensor, this.metadata.clock, this.metadata.frame];
    if (request.op === "list") {
      return {
        op: "list" as const,
        entries: entries
          .filter((entry) => entry.kind === request.kind)
          .map(({ id, hash }) => ({ id, hash })),
      };
    }
    const found = entries.find((entry) =>
      entry.kind === request.kind && entry.id === request.id && entry.hash === request.hash);
    return { op: "get" as const, entry: found ? toEnvelope(found) : null };
  }

  private async provideBlob(
    requester: AukiAuthenticatedPeer,
    request: AukiBlobProviderRequest,
  ) {
    if (!this.sameDomain(requester) || !this.allowed(requester)) return null;
    const bytes = this.blobs.get(request.sha256);
    if (!bytes || request.offset > BigInt(bytes.byteLength)) return null;
    const start = Number(request.offset);
    const end = Math.min(bytes.byteLength, start + request.maxLen);
    return { totalSize: BigInt(bytes.byteLength), bytes: bytes.slice(start, end) };
  }

  private async drainMessages(receiver: NonNullable<CameraProtocols["receiver"]>): Promise<void> {
    try {
      while (!this.closing) {
        const event = await receiver.next();
        if (!event) return;
        try {
          if (this.options.role === "publisher") await this.handleControl(event);
          else await this.handleSnapshotReply(event);
        } catch (error) {
          this.options.controls.ignored?.(event, errorMessage(error));
          this.emit(`Camera Message ${event.type} failed: ${errorMessage(error)}`);
        }
      }
    } catch (error) {
      if (!this.closing) this.emit(`Message receiver failed: ${errorMessage(error)}`);
    }
  }

  private async handleControl(event: AukiMessageEvent): Promise<void> {
    if (!this.sameDomain(event.sender) || !this.allowed(event.sender)) {
      this.options.controls.ignored?.(event, "sender is not on the camera session allowlist");
      this.emit(`Ignored ${event.type} from unauthorized peer ${event.sender.peerId}`);
      return;
    }
    if (event.type === "camera.pause") {
      assert(event.payload.byteLength === 0, "pause payload must be empty");
      await this.options.controls.pause(event);
    } else if (event.type === "camera.resume") {
      assert(event.payload.byteLength === 0, "resume payload must be empty");
      await this.options.controls.resume(event);
    } else if (event.type === "camera.request_snapshot") {
      await this.handleSnapshot(event);
    } else {
      this.options.controls.ignored?.(event, "unsupported camera control type");
    }
  }

  private async handleSnapshotReply(event: AukiMessageEvent): Promise<void> {
    if (!this.sameDomain(event.sender)) {
      this.options.controls.ignored?.(event, "snapshot sender belongs to another Domain");
      return;
    }
    if (event.type !== "camera.snapshot_ready") {
      this.options.controls.ignored?.(event, "unsupported camera reply type");
      return;
    }
    const snapshot = decodeSnapshotReady(event);
    const pending = this.pendingSnapshots.get(snapshot.requestId);
    assert(pending !== undefined, "snapshot reply has no pending request");
    assert(pending.peerId === event.sender.peerId, "snapshot reply came from the wrong peer");
    this.removePendingSnapshot(snapshot.requestId);
    await this.options.controls.snapshotAvailable?.(snapshot);
  }

  private async handleSnapshot(event: AukiMessageEvent): Promise<void> {
    const request = decodeSnapshotRequest(event);
    const bytes = await this.options.controls.requestSnapshot(request);
    if (bytes === undefined) return;
    assert(bytes instanceof Uint8Array, "snapshot handler must return a Uint8Array");
    const staged = await this.stageBlobImpl(bytes);
    const ready: SnapshotReady = {
      requester: event.sender,
      requestId: request.requestId,
      reply: request.reply,
      ...staged,
    };
    await this.sendSnapshotReady(request.reply, ready);
    await this.options.controls.snapshotReady?.(ready);
  }

  private async sendSnapshotReady(reply: SnapshotReplyAddress, snapshot: SnapshotReady): Promise<void> {
    validateReplyAddress(reply, snapshot.requester.peerId);
    const sender = await this.openSnapshotReply(reply);
    try {
      assert(sender.remotePeer.peerId === snapshot.requester.peerId,
        "snapshot reply authenticated the wrong peer");
      assert(this.sameDomain(sender.remotePeer), "snapshot requester no longer shares the Domain");
      await sender.send("camera.snapshot_ready", utcNowNs(), encodeSnapshotReady(snapshot));
    } finally {
      try {
        await sender.close();
      } finally {
        sender.free();
      }
    }
  }

  private async openSnapshotReply(reply: SnapshotReplyAddress) {
    const routes = browserRoutes(reply.target.routes);
    assert(routes.length > 0, "snapshot requester supplied no browser-compatible WSS route");
    const failures: string[] = [];
    for (const route of routes) {
      try {
        return await required(this.messageClient, "Message client").openExact(
          { peerId: reply.target.peerId, route },
          reply.channel,
        );
      } catch (error) {
        failures.push(errorMessage(error));
      }
    }
    throw new Error(`snapshot reply routes failed: ${failures.join("; ")}`);
  }

  private async stageBlobImpl(bytes: Uint8Array): Promise<StagedBlob> {
    assert(bytes.byteLength > 0, "cannot stage an empty Blob");
    assert(bytes.byteLength <= MAX_BLOB_BYTES, `Blob exceeds the ${MAX_BLOB_BYTES}-byte demo limit`);
    const owned = bytes.slice();
    const sha256 = await sha256Hex(owned);
    if (!this.blobs.has(sha256) && this.blobs.size >= MAX_STAGED_BLOBS) {
      const oldest = this.blobs.keys().next().value as string | undefined;
      if (oldest !== undefined) this.blobs.delete(oldest);
    }
    this.blobs.set(sha256, owned);
    return { sha256, size: owned.byteLength };
  }

  private async fetchInfoImpl(target: AukiExactTarget): Promise<AukiParticipantInfo> {
    const info = await required(this.infoClient, "Info client").fetchExact(target);
    assert(info.peerId === target.peerId, "Info authenticated the wrong peer");
    return info;
  }

  private sameDomain(requester: AukiAuthenticatedPeer): boolean {
    return requester.domainIds.includes(this.peer.domainId);
  }

  private allowed(requester: AukiAuthenticatedPeer): boolean {
    try {
      return this.options.access.isAllowed(requester);
    } catch (error) {
      this.emit(`Access policy failed closed: ${errorMessage(error)}`);
      return false;
    }
  }

  private requestApproval(requester: AukiAuthenticatedPeer): void {
    try {
      this.options.access.requestApproval(requester);
    } catch (error) {
      this.emit(`Approval callback failed: ${errorMessage(error)}`);
    }
  }

  private track<T>(operation: () => Promise<T>): Promise<T> {
    this.assertOpen();
    const promise = operation();
    this.operations.add(promise);
    void promise.then(
      () => this.operations.delete(promise),
      () => this.operations.delete(promise),
    );
    return promise;
  }

  private async closeOwned(): Promise<void> {
    this.closing = true;
    this.cameraAvailable = false;
    const errors: string[] = [];
    const receiver = this.receiver;
    this.receiver = undefined;
    if (receiver) {
      try {
        await receiver.close();
      } catch (error) {
        errors.push(`Message receiver: ${errorMessage(error)}`);
      } finally {
        receiver.free();
      }
    }
    if (this.receiverTask) {
      const result = await Promise.allSettled([this.receiverTask]);
      if (result[0]?.status === "rejected") {
        errors.push(`Message task: ${errorMessage(result[0].reason)}`);
      }
      this.receiverTask = undefined;
    }
    for (const endpoint of this.endpoints.reverse()) {
      try {
        await endpoint.close();
      } catch (error) {
        errors.push(`endpoint: ${errorMessage(error)}`);
      } finally {
        endpoint.free();
      }
    }
    this.endpoints.length = 0;
    const operations = await Promise.allSettled([...this.operations]);
    for (const result of operations) {
      if (result.status === "rejected") errors.push(`client operation: ${errorMessage(result.reason)}`);
    }
    for (const client of this.clients.reverse()) client.free();
    this.clients.length = 0;
    this.blobs.clear();
    for (const requestId of this.pendingSnapshots.keys()) {
      this.removePendingSnapshot(requestId);
    }
    if (errors.length) throw new Error(`Camera protocol shutdown failed: ${errors.join("; ")}`);
  }

  private assertOpen(): void {
    if (this.closing) throw new Error("camera protocols are closing");
  }

  private emit(message: string): void {
    this.options.event?.(message);
  }

  private removePendingSnapshot(requestId: string): void {
    const pending = this.pendingSnapshots.get(requestId);
    if (!pending) return;
    if (pending.timeout !== undefined) globalThis.clearTimeout(pending.timeout);
    this.pendingSnapshots.delete(requestId);
  }

  private armPendingSnapshot(requestId: string, peerId: string): void {
    const pending = this.pendingSnapshots.get(requestId);
    if (!pending || pending.peerId !== peerId) return;
    const timeout = globalThis.setTimeout(() => {
      const current = this.pendingSnapshots.get(requestId);
      if (!current || current.peerId !== peerId) return;
      this.pendingSnapshots.delete(requestId);
      this.emit(`Snapshot request ${requestId} timed out`);
      void Promise.resolve(this.options.controls.snapshotExpired?.(requestId))
        .catch((error) => this.emit(
          `Snapshot timeout handler failed: ${errorMessage(error)}`,
        ));
    }, SNAPSHOT_TIMEOUT_MS);
    this.pendingSnapshots.set(requestId, { peerId, timeout });
  }
}

function prepareCameraMetadata(peerId: string, sessionId: string): CameraRegistryMetadata {
  const frameEntry: AukiFrameRegistryEntry = {
    peer_id: peerId,
    frame_id: FRAME_ID,
    handedness: "right",
    axes: { x: "right", y: "down", z: "forward" },
    units: "meters",
  };
  const frame = verifyRegistryEntry(
    "frame",
    envelopeRef(peerId, prepareRegistryEntry("frame", frameEntry)),
    frameEntry,
  );
  const clockEntry: AukiClockRegistryEntry = {
    peer_id: peerId,
    session_id: sessionId,
    clock_id: CLOCK_ID,
    type: "utc_clock",
    unit: "ns",
    monotonic: false,
    epoch: "1970-01-01T00:00:00Z",
    scope: "global",
  };
  const clock = verifyRegistryEntry(
    "clock",
    envelopeRef(peerId, prepareRegistryEntry("clock", clockEntry)),
    clockEntry,
  );
  const sensorEntry: AukiSensorRegistryEntry = {
    peer_id: peerId,
    sensor_id: CAMERA_RESOURCE_ID,
    kind: "camera",
    type: "rgb",
    width: CAMERA_WIDTH,
    height: CAMERA_HEIGHT,
    frame_rate_hz: CAMERA_RATE_HZ,
    image_encoding: "jpeg",
    pixel_format: "rgb8",
    row_stride_bytes: 0,
    color_space: "srgb",
    intrinsics_model: "none",
    distortion_model: "none",
    frame: frame.ref,
  };
  const sensor = verifyRegistryEntry(
    "sensor",
    envelopeRef(peerId, prepareRegistryEntry("sensor", sensorEntry)),
    sensorEntry,
  );
  return { sensor, clock, frame };
}

function verifyRegistryEntry<T extends AukiRegistryEntry>(
  kind: AukiRegistryKind,
  ref: AukiRegistryRef,
  entry: T,
): VerifiedRegistryEntry<T> {
  const envelope = prepareRegistryEntry(kind, entry);
  assert(entry.peer_id === ref.peer_id, `${kind} Registry owner does not match Catalog`);
  assert(envelope.kind === kind, `${kind} Registry kind did not round-trip`);
  assert(envelope.id === ref.id, `${kind} Registry ID does not match Catalog`);
  assert(envelope.hash === ref.hash, `${kind} Registry hash does not match Catalog`);
  return {
    kind,
    id: envelope.id,
    hash: envelope.hash,
    canonicalJson: envelope.canonical_json,
    entry,
    ref: { ...ref },
  };
}

function parseCameraCatalog(
  resources: readonly AukiCatalogResource[],
  peerId: string,
): CameraCatalogDescription {
  const candidates = resources.filter((resource) =>
    resource.variant === "sensor_log" && resource["resource_id"] === CAMERA_RESOURCE_ID);
  assert(candidates.length > 0, "approval_required: camera Catalog row is unavailable");
  assert(candidates.length === 1, "camera Catalog contains duplicate resource IDs");
  const row = required(candidates[0], "camera Catalog row");
  assert(row["source_peer_id"] === peerId, "camera Catalog source does not match authenticated peer");
  assert(row["writer_peer_id"] === peerId, "camera Catalog writer does not match authenticated peer");
  assert(row["state"] === "live", "camera Catalog resource is not live");
  const sensorBlock = record(row["sensor"], "camera Catalog sensor");
  assert(sensorBlock["kind"] === "camera", "camera Catalog has the wrong sensor kind");
  assert(sensorBlock["type"] === "rgb", "camera Catalog has the wrong sensor type");
  const sensorId = stringField(sensorBlock, "sensor_id", "camera Catalog sensor");
  const sensorHash = registryHash(sensorBlock["sensor_hash"], "camera Catalog sensor hash");
  const manifest = record(row["manifest"], "camera Catalog manifest");
  const clock = registryRef(manifest["clock"], "camera Catalog clock");
  const frame = registryRef(manifest["frame"], "camera Catalog frame");
  assert(clock.peer_id === peerId, "camera clock is owned by another peer");
  assert(frame.peer_id === peerId, "camera frame is owned by another peer");
  return {
    row,
    sensor: { peer_id: peerId, id: sensorId, hash: sensorHash },
    clock,
    frame,
  };
}

function parseControlChannel(
  resources: readonly AukiCatalogResource[],
  peerId: string,
  clock: AukiRegistryRef,
): AukiMessageChannelResource {
  const candidates = resources.filter((resource) =>
    resource.variant === "message_channel"
      && resource["resource_id"] === CAMERA_CONTROL_RESOURCE_ID);
  assert(candidates.length === 1, "camera Catalog control channel is missing or duplicated");
  const row = required(candidates[0], "camera control channel");
  const channel: AukiMessageChannelResource = {
    variant: "message_channel",
    owner_peer_id: stringField(row, "owner_peer_id", "camera control channel"),
    resource_id: CAMERA_CONTROL_RESOURCE_ID,
    clock: registryRef(row["clock"], "camera control channel clock"),
  };
  validateControlChannel(channel, peerId);
  assert(sameRef(channel.clock, clock), "control channel and camera use different clocks");
  return channel;
}

function validateCameraEntries(
  sensor: AukiSensorRegistryEntry,
  clock: AukiClockRegistryEntry,
  frame: AukiFrameRegistryEntry,
  catalog: CameraCatalogDescription,
): void {
  assert(sensor.sensor_id === catalog.sensor.id, "camera Sensor ID mismatch");
  assert(sensor["kind"] === "camera", "camera Sensor Registry kind mismatch");
  assert(sensor["type"] === "rgb", "camera Sensor Registry type mismatch");
  assert(sensor["width"] === CAMERA_WIDTH, `camera Sensor width must be ${CAMERA_WIDTH}`);
  assert(sensor["height"] === CAMERA_HEIGHT, `camera Sensor height must be ${CAMERA_HEIGHT}`);
  assert(sensor["frame_rate_hz"] === CAMERA_RATE_HZ,
    `camera Sensor frame rate must be ${CAMERA_RATE_HZ}`);
  assert(sensor["image_encoding"] === "jpeg", "camera Sensor must describe JPEG frames");
  assert(sensor["pixel_format"] === "rgb8", "camera Sensor pixel format must be rgb8");
  assert(sensor["row_stride_bytes"] === 0, "compressed camera Sensor must have zero row stride");
  assert(sensor["color_space"] === "srgb", "camera Sensor color space must be sRGB");
  assert(sensor["intrinsics_model"] === "none", "camera Sensor intrinsics must be absent");
  assert(sensor["distortion_model"] === "none", "camera Sensor distortion must be absent");
  assert(sensor["calibration"] === undefined || sensor["calibration"] === null,
    "camera Sensor calibration must be absent");
  assert(sameRef(registryRef(sensor["frame"], "camera Sensor frame"), catalog.frame),
    "camera Sensor and Catalog reference different frames");

  assert(clock.clock_id === catalog.clock.id, "camera Clock ID mismatch");
  assert(clock["type"] === "utc_clock", "camera Clock must be UTC");
  assert(clock["unit"] === "ns", "camera Clock must use nanoseconds");
  assert(clock["monotonic"] === false, "camera UTC Clock cannot be monotonic");
  assert(clock["epoch"] === "1970-01-01T00:00:00Z", "camera UTC Clock has an unknown epoch");
  assert(clock["scope"] === "global", "camera Clock scope must be global");

  assert(frame.frame_id === catalog.frame.id, "camera Frame ID mismatch");
  assert(frame["handedness"] === "right", "camera Frame must be right-handed");
  assert(frame["units"] === "meters", "camera Frame must use meters");
  const axes = record(frame["axes"], "camera Frame axes");
  assert(
    axes["x"] === "right" && axes["y"] === "down" && axes["z"] === "forward",
    "camera Frame is not ROS optical",
  );
}

function validateControlChannel(channel: AukiMessageChannelResource, peerId: string): void {
  assert(channel.variant === "message_channel", "invalid camera control channel variant");
  assert(channel.owner_peer_id === peerId, "camera control channel owner mismatch");
  assert(channel.resource_id === CAMERA_CONTROL_RESOURCE_ID, "unexpected camera control resource");
  assert(channel.clock.peer_id === peerId, "camera control clock owner mismatch");
  registryHash(channel.clock.hash, "camera control clock hash");
}

function encodeControl(control: CameraControl, reply?: SnapshotReplyAddress): Uint8Array {
  if (control.type !== "camera.request_snapshot") return EMPTY_BYTES;
  validateRequestId(control.requestId);
  assert(reply !== undefined, "snapshot request is missing its reply address");
  return new TextEncoder().encode(JSON.stringify({
    version: 1,
    requestId: control.requestId,
    reply,
  }));
}

function encodeSnapshotReady(snapshot: SnapshotReady): Uint8Array {
  return new TextEncoder().encode(JSON.stringify({
    version: 1,
    requestId: snapshot.requestId,
    sha256: snapshot.sha256,
    size: snapshot.size,
  }));
}

function decodeSnapshotRequest(event: AukiMessageEvent): SnapshotRequest {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(event.payload));
  } catch (error) {
    throw new Error(`invalid snapshot request payload: ${errorMessage(error)}`);
  }
  const payload = record(value, "snapshot request");
  assert(payload["version"] === 1, "unsupported snapshot request version");
  const requestId = stringField(payload, "requestId", "snapshot request");
  validateRequestId(requestId);
  const reply = replyAddress(payload["reply"], event.sender.peerId);
  return { event, requester: event.sender, requestId, reply };
}

function decodeSnapshotReady(event: AukiMessageEvent): SnapshotAvailable {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(event.payload));
  } catch (error) {
    throw new Error(`invalid snapshot-ready payload: ${errorMessage(error)}`);
  }
  const payload = record(value, "snapshot-ready message");
  assert(payload["version"] === 1, "unsupported snapshot-ready version");
  const requestId = stringField(payload, "requestId", "snapshot-ready message");
  validateRequestId(requestId);
  const sha256 = stringField(payload, "sha256", "snapshot-ready message");
  assert(isSha256(sha256), "snapshot-ready SHA-256 is invalid");
  const size = positiveNumber(payload["size"], "snapshot-ready size");
  assert(size <= MAX_BLOB_BYTES, "snapshot-ready Blob exceeds the demo limit");
  return { event, publisher: event.sender, requestId, sha256, size };
}

function replyAddress(value: unknown, requesterPeerId: string): SnapshotReplyAddress {
  const source = record(value, "snapshot reply address");
  const target = record(source["target"], "snapshot reply target");
  const channel = record(source["channel"], "snapshot reply channel");
  assert(channel["variant"] === "message_channel", "invalid snapshot reply channel variant");
  const result: SnapshotReplyAddress = {
    target: {
      peerId: stringField(target, "peerId", "snapshot reply target"),
      routes: stringArray(target["routes"], "snapshot reply target.routes"),
    },
    channel: {
      variant: "message_channel",
      owner_peer_id: stringField(channel, "owner_peer_id", "snapshot reply channel"),
      resource_id: stringField(channel, "resource_id", "snapshot reply channel"),
      clock: registryRef(channel["clock"], "snapshot reply clock"),
    },
  };
  validateReplyAddress(result, requesterPeerId);
  return result;
}

function validateReplyAddress(reply: SnapshotReplyAddress, requesterPeerId: string): void {
  assert(reply.target.peerId === requesterPeerId, "snapshot reply target is not the requester");
  assert(reply.target.routes.length > 0, "snapshot reply routes are empty");
  assert(reply.target.routes.length <= 4, "snapshot reply has too many routes");
  assert(new Set(reply.target.routes).size === reply.target.routes.length,
    "snapshot reply routes contain duplicates");
  assert(reply.target.routes.every((route) => route.endsWith(`/p2p/${requesterPeerId}`)),
    "snapshot reply route does not terminate at the requester");
  assert(reply.channel.owner_peer_id === requesterPeerId, "snapshot reply channel is not requester-owned");
  assert(reply.channel.clock.peer_id === requesterPeerId, "snapshot reply clock is not requester-owned");
  assert(reply.channel.resource_id === CAMERA_REPLY_RESOURCE_ID, "unexpected snapshot reply resource");
  registryHash(reply.channel.clock.hash, "snapshot reply clock hash");
}

function validateRequestId(requestId: string): void {
  assert(
    /^[A-Za-z0-9._:-]{1,128}$/.test(requestId),
    "snapshot requestId must be 1-128 safe ASCII characters",
  );
}

function envelopeRef(peerId: string, envelope: AukiRegistryEntryEnvelope): AukiRegistryRef {
  return { peer_id: peerId, id: envelope.id, hash: envelope.hash };
}

function toEnvelope(entry: VerifiedRegistryEntry<AukiRegistryEntry>): AukiRegistryEntryEnvelope {
  return {
    kind: entry.kind,
    id: entry.id,
    hash: entry.hash,
    canonical_json: entry.canonicalJson,
  };
}

function registryRef(value: unknown, label: string): AukiRegistryRef {
  const source = record(value, label);
  return {
    peer_id: stringField(source, "peer_id", label),
    id: stringField(source, "id", label),
    hash: registryHash(source["hash"], `${label} hash`),
  };
}

function registryHash(value: unknown, label: string): string {
  assert(typeof value === "string" && /^[0-9a-f]{32}$/.test(value),
    `${label} must be a lowercase XXH3-128 hash`);
  return value;
}

function sameRef(left: AukiRegistryRef, right: AukiRegistryRef): boolean {
  return left.peer_id === right.peer_id && left.id === right.id && left.hash === right.hash;
}

function browserRoutes(routes: readonly string[]): string[] {
  return routes.filter((route) => route.split("/").includes("wss"));
}

function record(value: unknown, label: string): Record<string, unknown> {
  assert(typeof value === "object" && value !== null && !Array.isArray(value), `${label} is not an object`);
  return value as Record<string, unknown>;
}

function stringField(source: Record<string, unknown>, key: string, label: string): string {
  const value = source[key];
  assert(typeof value === "string" && value.length > 0, `${label}.${key} is missing`);
  return value;
}

function stringArray(value: unknown, label: string): string[] {
  assert(Array.isArray(value) && value.length > 0, `${label} must be a non-empty array`);
  return value.map((entry, index) => {
    assert(typeof entry === "string" && entry.length > 0, `${label}[${index}] is empty`);
    return entry;
  });
}

function positiveNumber(value: unknown, label: string): number {
  assert(typeof value === "number" && Number.isInteger(value) && value > 0, `${label} must be positive`);
  return value;
}

function required<T>(value: T | undefined, label: string): T {
  assert(value !== undefined, `${label} is unavailable`);
  return value;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function utcNowNs(): bigint {
  return BigInt(Date.now()) * 1_000_000n;
}

function isSha256(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const input = bytes.slice().buffer;
  const digest = await globalThis.crypto.subtle.digest("SHA-256", input);
  return [...new Uint8Array(digest)]
    .map((value) => value.toString(16).padStart(2, "0"))
    .join("");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

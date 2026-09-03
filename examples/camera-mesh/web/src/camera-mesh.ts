import {
  AukiDiscoveryMode,
  AukiPeer,
  AukiStreamClient,
  AukiStreamEndpoint,
  AukiStreamSubscription,
  decodeCameraFrameImage,
  encodeCameraFrameImage,
  type AukiAuthenticatedPeer,
  type AukiExactTarget,
  type AukiStreamDispatch,
  type AukiStreamManifest,
  type AukiStreamRequest,
} from "../pkg-web/auki_sdk_web.js";
import {
  CameraCapture,
  type CaptureDiagnostics,
  type CaptureMode,
} from "./capture.js";
import {
  CAMERA_RESOURCE_ID,
  CameraProtocols,
  type CameraRegistryMetadata,
  type RemoteCameraMetadata,
  type SnapshotAvailable,
  type VerifiedBlob,
} from "./protocols.js";
import {
  cameraProfileLabel,
  DEFAULT_CAMERA_PROFILE,
  type CameraStreamProfile,
} from "./profile.js";

export type CameraRole = "publisher" | "viewer";

export interface CameraCandidate {
  readonly peerId: string;
  readonly routes: string[];
  readonly servedProtocols: string[];
  readonly expiresAt: string;
}

export interface CameraPeerCard {
  readonly version: 1;
  readonly runtime: "browser";
  readonly domainId: string;
  readonly peerId: string;
  readonly protocols: string[];
  readonly routes: {
    readonly tcp: string;
    readonly wss: string;
  };
}

export interface RemoteFrame {
  readonly peerId: string;
  readonly jpeg: Uint8Array;
  readonly sequence: bigint;
  readonly timestampNs: bigint;
  readonly received: number;
  readonly bytes: number;
}

export interface RemoteConnection {
  readonly target: AukiExactTarget;
  readonly metadata: RemoteCameraMetadata;
  readonly streamManifest: AukiStreamManifest;
}

export interface RemoteSnapshot {
  readonly peerId: string;
  readonly requestId: string;
  readonly sha256: string;
  readonly jpeg: Uint8Array;
  readonly relayed: boolean;
}

export interface CameraMeshHooks {
  event(message: string): void;
  captureDiagnostics(diagnostics: CaptureDiagnostics): void;
  pendingChanged(peerIds: readonly string[]): void;
  remoteFrame(frame: RemoteFrame): void;
  remoteConnected(connection: RemoteConnection): void;
  remoteSnapshot(snapshot: RemoteSnapshot): void;
  snapshotExpired(requestId: string, peerId?: string): void;
  remoteEnded(reason: string, peerId?: string): void;
}

interface RemoteSession {
  readonly connection: RemoteConnection;
  readonly subscription: AukiStreamSubscription;
  task?: Promise<void>;
}

type Session = {
  startPeerWithDiscovery(domainId: string, mode: AukiDiscoveryMode): Promise<AukiPeer>;
};

export class CameraMesh {
  readonly peerId: string;
  readonly domainId: string;
  readonly streamProtocol: string;

  private readonly streamClient: AukiStreamClient;
  private readonly allowed = new Set<string>();
  private readonly denied = new Set<string>();
  private readonly pending = new Set<string>();
  private protocols?: CameraProtocols;
  private streamEndpoint?: AukiStreamEndpoint;
  private capture?: CameraCapture;
  private readonly remotes = new Map<string, RemoteSession>();
  private readonly connecting = new Map<string, Promise<RemoteConnection>>();
  private readonly pendingSnapshotTargets = new Map<string, AukiExactTarget>();
  private closing = false;
  private closePromise?: Promise<void>;

  private constructor(
    private readonly peer: AukiPeer,
    readonly role: CameraRole,
    private readonly displayName: string,
    readonly profile: CameraStreamProfile,
    private readonly hooks: CameraMeshHooks,
  ) {
    this.peerId = peer.peerId;
    this.domainId = peer.domainId;
    this.streamClient = new AukiStreamClient(peer);
    this.streamProtocol = this.streamClient.protocol;
    void peer.waitStopped().catch((error) => {
      if (!this.closing) hooks.remoteEnded(`Peer failed: ${errorMessage(error)}`);
    });
  }

  static async start(
    session: Session,
    domainId: string,
    role: CameraRole,
    displayName: string,
    hooks: CameraMeshHooks,
    profile: CameraStreamProfile = DEFAULT_CAMERA_PROFILE,
  ): Promise<CameraMesh> {
    const mode = role === "publisher"
      ? AukiDiscoveryMode.DiscoverAndAdvertise
      : AukiDiscoveryMode.DiscoverOnly;
    const peer = await session.startPeerWithDiscovery(domainId, mode);
    const mesh = new CameraMesh(peer, role, displayName, profile, hooks);
    try {
      await mesh.mountProtocols();
      return mesh;
    } catch (error) {
      await mesh.close().catch(() => undefined);
      throw error;
    }
  }

  get name(): string {
    return this.displayName;
  }

  get isPublishing(): boolean {
    return this.streamEndpoint !== undefined;
  }

  get connectedPeerIds(): readonly string[] {
    return [...this.remotes.keys()];
  }

  card(): CameraPeerCard {
    return {
      version: 1,
      runtime: "browser",
      domainId: this.domainId,
      peerId: this.peerId,
      protocols: [
        ...(this.protocols?.servedProtocols ?? []),
        ...(this.isPublishing ? [this.streamProtocol] : []),
      ],
      routes: {
        tcp: this.peer.tcpRoute,
        wss: this.peer.wssRoute,
      },
    };
  }

  async startPublishing(mode: CaptureMode, preview: HTMLCanvasElement): Promise<void> {
    this.assertOpen();
    if (this.role !== "publisher") throw new Error("this peer is not a publisher");
    if (this.streamEndpoint) throw new Error("camera is already published");

    const capture = new CameraCapture(
      preview,
      this.profile,
      (jpeg) => encodeCameraFrameImage(jpeg),
      (message) => this.hooks.event(message),
      (diagnostics) => this.hooks.captureDiagnostics(diagnostics),
    );
    await capture.start(mode);
    try {
      const endpoint = AukiStreamEndpoint.mount(
        this.peer,
        (requester: AukiAuthenticatedPeer, request: AukiStreamRequest): AukiStreamDispatch =>
          this.dispatchStream(requester, request, capture),
      );
      this.capture = capture;
      this.streamEndpoint = endpoint;
      this.protocolStack().setCameraAvailable(true);
      this.hooks.event(
        `Publishing ${CAMERA_RESOURCE_ID} at ${cameraProfileLabel(this.profile)}`,
      );
    } catch (error) {
      capture.stop();
      throw error;
    }
  }

  async stopPublishing(): Promise<void> {
    const capture = this.capture;
    const endpoint = this.streamEndpoint;
    this.capture = undefined;
    this.streamEndpoint = undefined;
    this.pending.clear();
    this.allowed.clear();
    this.denied.clear();
    this.emitPending();
    try {
      if (endpoint) this.protocolStack().setCameraAvailable(false);
      capture?.stop();
      if (endpoint) {
        await endpoint.close();
      }
    } finally {
      capture?.stop();
      endpoint?.free();
    }
    if (capture || endpoint) this.hooks.event("Camera publication stopped");
  }

  approve(peerId: string): void {
    this.assertOpen();
    if (this.role !== "publisher") throw new Error("only a publisher approves camera viewers");
    this.pending.delete(peerId);
    this.denied.delete(peerId);
    this.allowed.add(peerId);
    this.emitPending();
    this.hooks.event(`Approved ${peerId} for this camera session`);
  }

  deny(peerId: string): void {
    this.assertOpen();
    if (this.role !== "publisher") throw new Error("only a publisher denies camera viewers");
    this.pending.delete(peerId);
    this.allowed.delete(peerId);
    this.denied.add(peerId);
    this.emitPending();
    this.hooks.event(`Denied ${peerId} for this camera session`);
  }

  async discoverCameras(): Promise<CameraCandidate[]> {
    this.assertOpen();
    const candidates = await this.peer.discoverProtocol(this.streamProtocol);
    return candidates.map((candidate) => {
      try {
        return {
          peerId: candidate.peerId,
          routes: candidate.routes,
          servedProtocols: candidate.servedProtocols,
          expiresAt: candidate.expiresAt,
        };
      } finally {
        candidate.free();
      }
    });
  }

  async connectCamera(candidate: CameraCandidate): Promise<RemoteConnection> {
    this.assertOpen();
    if (this.role !== "viewer") throw new Error("this peer is not a viewer");
    if (candidate.peerId === this.peerId) throw new Error("cannot connect to this viewer");
    const existing = this.remotes.get(candidate.peerId);
    if (existing) return existing.connection;
    const pending = this.connecting.get(candidate.peerId);
    if (pending) return pending;

    const operation = this.connectCameraOwned(candidate);
    this.connecting.set(candidate.peerId, operation);
    try {
      return await operation;
    } finally {
      if (this.connecting.get(candidate.peerId) === operation) {
        this.connecting.delete(candidate.peerId);
      }
    }
  }

  private async connectCameraOwned(candidate: CameraCandidate): Promise<RemoteConnection> {
    const request: AukiStreamRequest = {
      sourcePeerId: candidate.peerId,
      resourceId: CAMERA_RESOURCE_ID,
      from: { kind: "latest" },
    };
    const routes = browserRoutes(candidate.routes);
    if (!routes.length) throw new Error("camera publisher has no browser-compatible WSS route");

    const failures: string[] = [];
    for (const route of routes) {
      const target: AukiExactTarget = { peerId: candidate.peerId, route };
      let subscription: AukiStreamSubscription | undefined;
      let adopted = false;
      try {
        const metadata = await this.protocolStack().resolveRemoteMetadata(target);
        subscription = await this.streamClient.subscribeExact(target, "camera", request);
        validateStreamManifest(subscription.manifest, metadata, candidate.peerId);
        if (this.closing) throw new Error("Camera Mesh peer is stopping");
        const connection: RemoteConnection = {
          target: { ...target },
          metadata,
          streamManifest: subscription.manifest,
        };
        const remote: RemoteSession = { connection, subscription };
        this.remotes.set(candidate.peerId, remote);
        adopted = true;
        remote.task = this.consume(candidate.peerId, remote);
        this.hooks.remoteConnected(connection);
        this.hooks.event(`Viewing ${candidate.peerId} through ${route}`);
        return connection;
      } catch (error) {
        const reasons = [errorMessage(error)];
        if (subscription) {
          try {
            if (adopted) {
              if (this.remotes.get(candidate.peerId)?.subscription === subscription) {
                await this.disconnectCamera(candidate.peerId);
              }
            } else {
              await cancelAndFree(subscription);
            }
          } catch (cleanupError) {
            reasons.push(`Stream cleanup failed: ${errorMessage(cleanupError)}`);
          }
        }
        failures.push(reasons.join("; "));
      }
    }
    throw new Error(`Camera request declined or unreachable: ${failures.join("; ")}`);
  }

  async pauseRemote(peerId?: string): Promise<void> {
    const { target, metadata } = this.remoteControlContext(peerId);
    await this.protocolStack().sendControl(target, metadata.controlChannel, {
      type: "camera.pause",
    });
    this.hooks.event(`Message camera.pause acknowledged by ${target.peerId}`);
  }

  async resumeRemote(peerId?: string): Promise<void> {
    const { target, metadata } = this.remoteControlContext(peerId);
    await this.protocolStack().sendControl(target, metadata.controlChannel, {
      type: "camera.resume",
    });
    this.hooks.event(`Message camera.resume acknowledged by ${target.peerId}`);
  }

  async requestSnapshot(peerId?: string): Promise<string> {
    const { target, metadata } = this.remoteControlContext(peerId);
    const requestId = globalThis.crypto.randomUUID();
    this.pendingSnapshotTargets.set(requestId, { ...target });
    try {
      await this.protocolStack().sendControl(target, metadata.controlChannel, {
        type: "camera.request_snapshot",
        requestId,
      });
    } catch (error) {
      this.pendingSnapshotTargets.delete(requestId);
      throw error;
    }
    this.hooks.event(`Message snapshot request ${requestId} acknowledged`);
    return requestId;
  }

  async disconnectCamera(peerId: string): Promise<void> {
    const remote = this.remotes.get(peerId);
    if (!remote) return;
    this.remotes.delete(peerId);
    try {
      await remote.subscription.cancel();
    } catch {
      // The reader task below owns reporting terminal transport failures.
    }
    if (remote.task) await remote.task.catch(() => undefined);
    remote.subscription.free();
  }

  async disconnectAllCameras(): Promise<void> {
    const results = await Promise.allSettled(
      [...this.remotes.keys()].map((peerId) => this.disconnectCamera(peerId)),
    );
    const failures = results.flatMap((result) =>
      result.status === "rejected" ? [errorMessage(result.reason)] : []);
    if (failures.length) {
      throw new Error(`Camera subscriptions failed to stop: ${failures.join("; ")}`);
    }
  }

  private async waitForConnecting(): Promise<void> {
    if (this.connecting.size === 0) return;
    await Promise.allSettled([...this.connecting.values()]);
  }

  private async closeRemoteConnections(): Promise<void> {
    await this.waitForConnecting();
    await this.disconnectAllCameras();
  }

  close(): Promise<void> {
    this.closePromise ??= this.closeOwned();
    return this.closePromise;
  }

  private async closeOwned(): Promise<void> {
    this.closing = true;
    const errors: string[] = [];
    try {
      await this.closeRemoteConnections();
    } catch (error) {
      errors.push(`viewer: ${errorMessage(error)}`);
    }
    this.pendingSnapshotTargets.clear();
    try {
      await this.stopPublishing();
    } catch (error) {
      errors.push(`publisher: ${errorMessage(error)}`);
    }
    if (this.protocols) {
      try {
        await this.protocols.close();
      } catch (error) {
        errors.push(`protocols: ${errorMessage(error)}`);
      }
      this.protocols = undefined;
    }
    this.streamClient.free();
    try {
      await this.peer.shutdown();
    } catch (error) {
      errors.push(`peer: ${errorMessage(error)}`);
    } finally {
      this.peer.free();
    }
    if (errors.length) throw new Error(`Camera Mesh shutdown failed: ${errors.join("; ")}`);
  }

  private async mountProtocols(): Promise<void> {
    this.protocols = await CameraProtocols.mount(this.peer, {
      role: this.role,
      displayName: this.displayName,
      profile: this.profile,
      access: {
        isAllowed: (requester) => this.allowed.has(requester.peerId),
        requestApproval: (requester) => this.requestApproval(requester.peerId),
      },
      controls: {
        pause: () => this.capture?.pause(),
        resume: () => this.capture?.resume(),
        requestSnapshot: () => this.capture?.latestJpeg(),
        snapshotReady: (snapshot) => {
          this.hooks.event(
            `Blob ${snapshot.sha256} staged for ${snapshot.requester.peerId}`,
          );
        },
        snapshotAvailable: (snapshot) => this.receiveSnapshot(snapshot),
        snapshotExpired: (requestId) => {
          const peerId = this.pendingSnapshotTargets.get(requestId)?.peerId;
          this.pendingSnapshotTargets.delete(requestId);
          this.hooks.snapshotExpired(requestId, peerId);
        },
        ignored: (event, reason) => {
          this.hooks.event(`Ignored Message ${event.type} from ${event.sender.peerId}: ${reason}`);
        },
      },
      event: (message) => this.hooks.event(message),
    });
  }

  private async receiveSnapshot(snapshot: SnapshotAvailable): Promise<void> {
    const target = this.pendingSnapshotTargets.get(snapshot.requestId);
    if (!target || target.peerId !== snapshot.publisher.peerId) {
      throw new Error("snapshot publisher does not match the pending camera request");
    }
    this.pendingSnapshotTargets.delete(snapshot.requestId);
    const blob: VerifiedBlob = await this.protocolStack().fetchVerifiedBlob(
      target,
      snapshot.sha256,
    );
    if (blob.bytes.byteLength !== snapshot.size) {
      throw new Error("verified Blob size differs from its Message announcement");
    }
    this.hooks.remoteSnapshot({
      peerId: target.peerId,
      requestId: snapshot.requestId,
      sha256: blob.sha256,
      jpeg: blob.bytes,
      relayed: blob.relayed,
    });
    this.hooks.event(`Blob snapshot ${blob.sha256} fetched and SHA-256 verified`);
  }

  private requestApproval(peerId: string): void {
    if (this.role !== "publisher" || this.allowed.has(peerId) || this.denied.has(peerId)) return;
    const firstRequest = !this.pending.has(peerId);
    this.pending.add(peerId);
    this.emitPending();
    if (firstRequest) this.hooks.event(`Camera access pending for ${peerId}`);
  }

  private remoteControlContext(peerId?: string): {
    target: AukiExactTarget;
    metadata: RemoteCameraMetadata;
  } {
    this.assertOpen();
    if (this.role !== "viewer") throw new Error("only a viewer sends camera controls");
    const resolvedPeerId = peerId ?? this.onlyConnectedPeerId();
    const remote = this.remotes.get(resolvedPeerId);
    if (!remote) throw new Error(`camera ${resolvedPeerId} is not connected`);
    return {
      target: remote.connection.target,
      metadata: remote.connection.metadata,
    };
  }

  private onlyConnectedPeerId(): string {
    if (this.remotes.size === 0) throw new Error("no camera is connected");
    if (this.remotes.size > 1) throw new Error("a camera Peer ID is required");
    return requiredFirst(this.remotes.keys());
  }

  private protocolStack(): CameraProtocols {
    const protocols = this.protocols;
    if (!protocols) throw new Error("camera protocols are unavailable");
    return protocols;
  }

  private dispatchStream(
    requester: AukiAuthenticatedPeer,
    request: AukiStreamRequest,
    capture: CameraCapture,
  ): AukiStreamDispatch {
    if (
      request.sourcePeerId !== this.peerId
      || request.resourceId !== CAMERA_RESOURCE_ID
      || request.from?.kind !== "latest"
    ) {
      return { kind: "decline", reason: { kind: "sensor_not_found" } };
    }
    if (!requester.domainIds.includes(this.domainId)) {
      return { kind: "decline", reason: { kind: "other", detail: "wrong_domain" } };
    }
    if (!this.allowed.has(requester.peerId)) {
      this.requestApproval(requester.peerId);
      return {
        kind: "decline",
        reason: {
          kind: "other",
          detail: this.denied.has(requester.peerId) ? "access_denied" : "approval_required",
        },
      };
    }
    return {
      kind: "accept",
      payloadKind: "camera",
      manifest: streamManifest(this.protocolStack().metadata),
      source: capture.source(),
    };
  }

  private async consume(peerId: string, remote: RemoteSession): Promise<void> {
    const { subscription } = remote;
    let received = 0;
    let bytes = 0;
    let terminalReason: string | undefined;
    try {
      while (this.remotes.get(peerId) === remote) {
        const next = await subscription.next();
        if (!next) {
          terminalReason = `Camera ${peerId} closed the Stream`;
          break;
        }
        if (next.kind === "end") {
          terminalReason = `Camera ${peerId} ended: ${next.reason.kind}`;
          break;
        }
        const jpeg = decodeCameraFrameImage(next.entry.payload);
        validateJpegDimensions(jpeg, remote.connection.metadata.profile);
        received += 1;
        bytes += jpeg.byteLength;
        this.hooks.remoteFrame({
          peerId,
          jpeg,
          sequence: next.entry.sequence,
          timestampNs: next.entry.timestampNs,
          received,
          bytes,
        });
      }
    } catch (error) {
      if (this.remotes.get(peerId) === remote && !this.closing) {
        terminalReason = `Camera ${peerId} failed: ${errorMessage(error)}`;
      }
    } finally {
      if (this.remotes.get(peerId) === remote) {
        this.remotes.delete(peerId);
        subscription.free();
        this.hooks.remoteEnded(terminalReason ?? `Camera ${peerId} Stream stopped`, peerId);
      }
    }
  }

  private emitPending(): void {
    this.hooks.pendingChanged([...this.pending].sort());
  }

  private assertOpen(): void {
    if (this.closing) throw new Error("Camera Mesh peer is stopping");
  }
}

function requiredFirst<T>(values: IterableIterator<T>): T {
  const value = values.next();
  if (value.done) throw new Error("expected a connected camera");
  return value.value;
}

async function cancelAndFree(subscription: AukiStreamSubscription): Promise<void> {
  let cancellationError: unknown;
  try {
    await subscription.cancel();
  } catch (error) {
    cancellationError = error;
  }
  subscription.free();
  if (cancellationError !== undefined) throw cancellationError;
}

function streamManifest(metadata: CameraRegistryMetadata): AukiStreamManifest {
  return {
    sensorId: metadata.sensor.id,
    sensorHash: metadata.sensor.hash,
    clockPeerId: metadata.clock.ref.peer_id,
    clockId: metadata.clock.id,
    clockHash: metadata.clock.hash,
    frameId: metadata.frame.id,
    frameHash: metadata.frame.hash,
    resourceId: CAMERA_RESOURCE_ID,
    payload: "camera_frame",
    fromFrameId: "",
    fromFrameHash: "",
    toFrameId: "",
    toFrameHash: "",
    writerMode: "live",
    expectedRateHz: metadata.profile.rateHz,
    mapPeerId: "",
    mapId: "",
    mapHash: "",
  };
}

function validateStreamManifest(
  manifest: AukiStreamManifest,
  metadata: RemoteCameraMetadata,
  peerId: string,
): void {
  const checks: ReadonlyArray<[unknown, unknown, string]> = [
    [manifest.resourceId, CAMERA_RESOURCE_ID, "resource ID"],
    [manifest.payload, "camera_frame", "payload"],
    [manifest.sensorId, metadata.sensor.id, "Sensor ID"],
    [manifest.sensorHash, metadata.sensor.hash, "Sensor hash"],
    [manifest.clockPeerId, peerId, "Clock owner"],
    [manifest.clockId, metadata.clock.id, "Clock ID"],
    [manifest.clockHash, metadata.clock.hash, "Clock hash"],
    [manifest.frameId, metadata.frame.id, "Frame ID"],
    [manifest.frameHash, metadata.frame.hash, "Frame hash"],
    [manifest.fromFrameId, "", "source Frame ID"],
    [manifest.fromFrameHash, "", "source Frame hash"],
    [manifest.toFrameId, "", "target Frame ID"],
    [manifest.toFrameHash, "", "target Frame hash"],
    [manifest.writerMode, "live", "writer mode"],
    [manifest.expectedRateHz, metadata.profile.rateHz, "expected frame rate"],
    [manifest.mapPeerId, "", "Map owner"],
    [manifest.mapId, "", "Map ID"],
    [manifest.mapHash, "", "Map hash"],
  ];
  for (const [actual, expected, label] of checks) {
    if (actual !== expected) throw new Error(`Stream ${label} does not match verified metadata`);
  }
}

function validateJpegDimensions(jpeg: Uint8Array, profile: CameraStreamProfile): void {
  const dimensions = jpegDimensions(jpeg);
  if (dimensions.width !== profile.width || dimensions.height !== profile.height) {
    throw new Error(
      `JPEG is ${dimensions.width}×${dimensions.height}; verified Sensor metadata requires ${profile.width}×${profile.height}`,
    );
  }
}

function jpegDimensions(jpeg: Uint8Array): { width: number; height: number } {
  if (jpeg.length < 4 || jpeg[0] !== 0xff || jpeg[1] !== 0xd8) {
    throw new Error("camera frame is not a JPEG image");
  }
  let offset = 2;
  while (offset + 1 < jpeg.length) {
    while (offset < jpeg.length && jpeg[offset] === 0xff) offset += 1;
    if (offset >= jpeg.length) break;
    const marker = jpeg[offset++]!;
    if (marker === 0xd9 || marker === 0xda) break;
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
    if (offset + 1 >= jpeg.length) break;
    const length = (jpeg[offset]! << 8) | jpeg[offset + 1]!;
    if (length < 2 || offset + length > jpeg.length) {
      throw new Error("camera frame has an invalid JPEG segment");
    }
    if (isStartOfFrame(marker)) {
      if (length < 7) throw new Error("camera frame has an invalid JPEG size segment");
      return {
        height: (jpeg[offset + 3]! << 8) | jpeg[offset + 4]!,
        width: (jpeg[offset + 5]! << 8) | jpeg[offset + 6]!,
      };
    }
    offset += length;
  }
  throw new Error("camera frame JPEG dimensions are unavailable");
}

function isStartOfFrame(marker: number): boolean {
  return marker >= 0xc0
    && marker <= 0xcf
    && marker !== 0xc4
    && marker !== 0xc8
    && marker !== 0xcc;
}

function browserRoutes(routes: readonly string[]): string[] {
  return routes.filter((route) => route.split("/").includes("wss"));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

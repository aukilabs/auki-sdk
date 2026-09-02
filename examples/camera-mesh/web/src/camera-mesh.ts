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
import { CameraCapture, type CaptureMode } from "./capture.js";
import {
  CAMERA_RATE_HZ,
  CAMERA_RESOURCE_ID,
  CameraProtocols,
  type CameraRegistryMetadata,
  type RemoteCameraMetadata,
  type SnapshotAvailable,
  type VerifiedBlob,
} from "./protocols.js";

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
  readonly requestId: string;
  readonly sha256: string;
  readonly jpeg: Uint8Array;
  readonly relayed: boolean;
}

export interface CameraMeshHooks {
  event(message: string): void;
  pendingChanged(peerIds: readonly string[]): void;
  remoteFrame(frame: RemoteFrame): void;
  remoteConnected(connection: RemoteConnection): void;
  remoteSnapshot(snapshot: RemoteSnapshot): void;
  snapshotExpired(requestId: string): void;
  remoteEnded(reason: string): void;
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
  private subscription?: AukiStreamSubscription;
  private viewerTask?: Promise<void>;
  private activeTarget?: AukiExactTarget;
  private remoteMetadata?: RemoteCameraMetadata;
  private closing = false;
  private closePromise?: Promise<void>;

  private constructor(
    private readonly peer: AukiPeer,
    readonly role: CameraRole,
    private readonly displayName: string,
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
  ): Promise<CameraMesh> {
    const mode = role === "publisher"
      ? AukiDiscoveryMode.DiscoverAndAdvertise
      : AukiDiscoveryMode.DiscoverOnly;
    const peer = await session.startPeerWithDiscovery(domainId, mode);
    const mesh = new CameraMesh(peer, role, displayName, hooks);
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
      (jpeg) => encodeCameraFrameImage(jpeg),
      (message) => this.hooks.event(message),
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
      this.hooks.event(`Publishing ${CAMERA_RESOURCE_ID} at ${CAMERA_RATE_HZ} fps`);
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

  async view(candidate: CameraCandidate): Promise<void> {
    this.assertOpen();
    if (this.role !== "viewer") throw new Error("this peer is not a viewer");
    await this.stopViewing();
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
        this.subscription = subscription;
        adopted = true;
        this.activeTarget = target;
        this.remoteMetadata = metadata;
        this.viewerTask = this.consume(subscription, candidate.peerId);
        this.hooks.remoteConnected({
          target: { ...target },
          metadata,
          streamManifest: subscription.manifest,
        });
        this.hooks.event(`Viewing ${candidate.peerId} through ${route}`);
        return;
      } catch (error) {
        const reasons = [errorMessage(error)];
        if (subscription) {
          try {
            if (adopted) {
              if (this.subscription === subscription) await this.stopViewing();
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

  async pauseRemote(): Promise<void> {
    const { target, metadata } = this.remoteControlContext();
    await this.protocolStack().sendControl(target, metadata.controlChannel, {
      type: "camera.pause",
    });
    this.hooks.event(`Message camera.pause acknowledged by ${target.peerId}`);
  }

  async resumeRemote(): Promise<void> {
    const { target, metadata } = this.remoteControlContext();
    await this.protocolStack().sendControl(target, metadata.controlChannel, {
      type: "camera.resume",
    });
    this.hooks.event(`Message camera.resume acknowledged by ${target.peerId}`);
  }

  async requestSnapshot(): Promise<string> {
    const { target, metadata } = this.remoteControlContext();
    const requestId = globalThis.crypto.randomUUID();
    await this.protocolStack().sendControl(target, metadata.controlChannel, {
      type: "camera.request_snapshot",
      requestId,
    });
    this.hooks.event(`Message snapshot request ${requestId} acknowledged`);
    return requestId;
  }

  async stopViewing(): Promise<void> {
    const subscription = this.subscription;
    const task = this.viewerTask;
    this.subscription = undefined;
    this.viewerTask = undefined;
    this.activeTarget = undefined;
    this.remoteMetadata = undefined;
    if (subscription) {
      try {
        await subscription.cancel();
      } catch {
        // The reader task below owns reporting terminal transport failures.
      }
    }
    if (task) await task.catch(() => undefined);
    subscription?.free();
  }

  close(): Promise<void> {
    this.closePromise ??= this.closeOwned();
    return this.closePromise;
  }

  private async closeOwned(): Promise<void> {
    this.closing = true;
    const errors: string[] = [];
    try {
      await this.stopViewing();
    } catch (error) {
      errors.push(`viewer: ${errorMessage(error)}`);
    }
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
        snapshotExpired: (requestId) => this.hooks.snapshotExpired(requestId),
        ignored: (event, reason) => {
          this.hooks.event(`Ignored Message ${event.type} from ${event.sender.peerId}: ${reason}`);
        },
      },
      event: (message) => this.hooks.event(message),
    });
  }

  private async receiveSnapshot(snapshot: SnapshotAvailable): Promise<void> {
    const target = this.activeTarget;
    if (!target || target.peerId !== snapshot.publisher.peerId) {
      throw new Error("snapshot publisher is not the active camera peer");
    }
    const blob: VerifiedBlob = await this.protocolStack().fetchVerifiedBlob(
      target,
      snapshot.sha256,
    );
    if (blob.bytes.byteLength !== snapshot.size) {
      throw new Error("verified Blob size differs from its Message announcement");
    }
    this.hooks.remoteSnapshot({
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

  private remoteControlContext(): {
    target: AukiExactTarget;
    metadata: RemoteCameraMetadata;
  } {
    this.assertOpen();
    if (this.role !== "viewer") throw new Error("only a viewer sends camera controls");
    if (!this.activeTarget || !this.remoteMetadata) throw new Error("no camera is connected");
    return { target: this.activeTarget, metadata: this.remoteMetadata };
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

  private async consume(subscription: AukiStreamSubscription, peerId: string): Promise<void> {
    let received = 0;
    let bytes = 0;
    let terminalReason: string | undefined;
    try {
      while (this.subscription === subscription) {
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
        received += 1;
        bytes += jpeg.byteLength;
        this.hooks.remoteFrame({
          jpeg,
          sequence: next.entry.sequence,
          timestampNs: next.entry.timestampNs,
          received,
          bytes,
        });
      }
    } catch (error) {
      if (this.subscription === subscription && !this.closing) {
        terminalReason = `Camera ${peerId} failed: ${errorMessage(error)}`;
      }
    } finally {
      if (this.subscription === subscription) {
        this.subscription = undefined;
        this.viewerTask = undefined;
        this.activeTarget = undefined;
        this.remoteMetadata = undefined;
        subscription.free();
        this.hooks.remoteEnded(terminalReason ?? `Camera ${peerId} Stream stopped`);
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
    expectedRateHz: CAMERA_RATE_HZ,
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
    [manifest.expectedRateHz, CAMERA_RATE_HZ, "expected frame rate"],
    [manifest.mapPeerId, "", "Map owner"],
    [manifest.mapId, "", "Map ID"],
    [manifest.mapHash, "", "Map hash"],
  ];
  for (const [actual, expected, label] of checks) {
    if (actual !== expected) throw new Error(`Stream ${label} does not match verified metadata`);
  }
}

function browserRoutes(routes: readonly string[]): string[] {
  return routes.filter((route) => route.split("/").includes("wss"));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

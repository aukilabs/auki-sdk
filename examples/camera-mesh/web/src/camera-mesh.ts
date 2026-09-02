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

const CAMERA_RESOURCE_ID = "camera/main";
const CAMERA_RATE_HZ = 5;
const PLACEHOLDER_HASH = "0".repeat(32);

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

export interface CameraMeshHooks {
  event(message: string): void;
  pendingChanged(peerIds: readonly string[]): void;
  remoteFrame(frame: RemoteFrame): void;
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
  private streamEndpoint?: AukiStreamEndpoint;
  private capture?: CameraCapture;
  private subscription?: AukiStreamSubscription;
  private viewerTask?: Promise<void>;
  private closing = false;

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
    return new CameraMesh(peer, role, displayName, hooks);
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
      protocols: this.isPublishing ? [this.streamProtocol] : [],
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
    capture?.stop();
    if (endpoint) {
      try {
        await endpoint.close();
      } finally {
        endpoint.free();
      }
    }
    this.pending.clear();
    this.allowed.clear();
    this.denied.clear();
    this.emitPending();
    if (capture || endpoint) this.hooks.event("Camera publication stopped");
  }

  approve(peerId: string): void {
    this.assertOpen();
    this.pending.delete(peerId);
    this.denied.delete(peerId);
    this.allowed.add(peerId);
    this.emitPending();
    this.hooks.event(`Approved ${peerId} for this camera session`);
  }

  deny(peerId: string): void {
    this.assertOpen();
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
      try {
        const subscription = await this.streamClient.subscribeExact(target, "camera", request);
        this.subscription = subscription;
        this.viewerTask = this.consume(subscription, candidate.peerId);
        this.hooks.event(`Viewing ${candidate.peerId} through ${route}`);
        return;
      } catch (error) {
        failures.push(errorMessage(error));
      }
    }
    throw new Error(`Camera request declined or unreachable: ${failures.join("; ")}`);
  }

  async stopViewing(): Promise<void> {
    const subscription = this.subscription;
    const task = this.viewerTask;
    this.subscription = undefined;
    this.viewerTask = undefined;
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

  async close(): Promise<void> {
    if (this.closing) return;
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
      if (!this.denied.has(requester.peerId)) {
        this.pending.add(requester.peerId);
        this.emitPending();
      }
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
      manifest: streamManifest(this.peerId),
      source: capture.source(),
    };
  }

  private async consume(subscription: AukiStreamSubscription, peerId: string): Promise<void> {
    let received = 0;
    let bytes = 0;
    try {
      while (this.subscription === subscription) {
        const next = await subscription.next();
        if (!next) return;
        if (next.kind === "end") {
          this.hooks.remoteEnded(`Camera ${peerId} ended: ${next.reason.kind}`);
          return;
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
        this.hooks.remoteEnded(`Camera ${peerId} failed: ${errorMessage(error)}`);
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

function streamManifest(peerId: string): AukiStreamManifest {
  return {
    sensorId: CAMERA_RESOURCE_ID,
    sensorHash: PLACEHOLDER_HASH,
    clockPeerId: peerId,
    clockId: "session/wall-clock",
    clockHash: PLACEHOLDER_HASH,
    frameId: "camera/optical",
    frameHash: PLACEHOLDER_HASH,
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

function browserRoutes(routes: readonly string[]): string[] {
  return routes.filter((route) => route.split("/").includes("wss"));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

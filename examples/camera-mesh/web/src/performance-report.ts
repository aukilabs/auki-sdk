export const CAMERA_PERFORMANCE_REPORT_KIND = "auki.camera-mesh.performance";
export const CAMERA_PERFORMANCE_REPORT_SCHEMA_VERSION = 1;
export const CAMERA_PERFORMANCE_SAMPLE_INTERVAL_MS = 1_000;

export interface CameraPerformanceContext {
  readonly runtime: "web" | "ios";
  readonly platform: string;
  readonly domainId: string;
  readonly localPeerId: string;
  readonly columnCount: number;
}

export interface CameraPerformanceSnapshot {
  readonly peerId: string;
  readonly name: string;
  readonly runtime: string;
  readonly status: string;
  readonly quality?: string;
  readonly width?: number;
  readonly height?: number;
  readonly targetFps?: number;
  readonly totalReceivedFrames: number;
  readonly totalRenderedFrames: number;
  readonly totalReceivedBytes: number;
  readonly receiveFps?: number;
  readonly renderFps?: number;
  readonly kibPerSecond?: number;
  readonly frameAgeMs?: number;
}

export interface CameraPerformanceEvent {
  readonly elapsedMs: number;
  readonly message: string;
}

export interface CameraPerformanceSample {
  readonly elapsedMs: number;
  readonly columns: number;
  readonly status: string;
  readonly quality?: string;
  readonly width?: number;
  readonly height?: number;
  readonly targetFps?: number;
  readonly receivedFrames: number;
  readonly renderedFrames: number;
  readonly receivedBytes: number;
  readonly receiveFps?: number;
  readonly renderFps?: number;
  readonly kibPerSecond?: number;
  readonly frameAgeMs?: number;
}

export interface CameraPerformanceNumberSummary {
  readonly min: number;
  readonly average: number;
  readonly max: number;
  readonly p50: number;
  readonly p95: number;
}

export interface CameraPerformancePeerSummary {
  readonly sampleCount: number;
  readonly receivedFrames: number;
  readonly renderedFrames: number;
  readonly receivedBytes: number;
  readonly renderToReceiveRatio?: number;
  readonly receiveFps?: CameraPerformanceNumberSummary;
  readonly renderFps?: CameraPerformanceNumberSummary;
  readonly kibPerSecond?: CameraPerformanceNumberSummary;
  readonly frameAgeMs?: CameraPerformanceNumberSummary;
}

export interface CameraPerformancePeerReport {
  readonly peerId: string;
  readonly name: string;
  readonly runtime: string;
  readonly samples: readonly CameraPerformanceSample[];
  readonly summary: CameraPerformancePeerSummary;
}

export interface CameraPerformanceReport {
  readonly schemaVersion: 1;
  readonly kind: typeof CAMERA_PERFORMANCE_REPORT_KIND;
  readonly runtime: "web" | "ios";
  readonly platform: string;
  readonly domainId: string;
  readonly localPeerId: string;
  readonly startedAt: string;
  readonly endedAt: string;
  readonly durationMs: number;
  readonly sampleIntervalMs: number;
  readonly initialColumns: number;
  readonly finalColumns: number;
  readonly peers: readonly CameraPerformancePeerReport[];
  readonly events: readonly CameraPerformanceEvent[];
}

interface PeerAccumulator {
  name: string;
  runtime: string;
  lastReceivedFrames?: number;
  lastRenderedFrames?: number;
  lastReceivedBytes?: number;
  receivedFrames: number;
  renderedFrames: number;
  receivedBytes: number;
  samples: CameraPerformanceSample[];
}

export class CameraPerformanceCapture {
  readonly startedAt: Date;
  readonly startedAtMonotonicMs: number;
  readonly context: CameraPerformanceContext;
  private readonly peers = new Map<string, PeerAccumulator>();
  private readonly events: CameraPerformanceEvent[] = [];

  constructor(
    context: CameraPerformanceContext,
    startedAt = new Date(),
    startedAtMonotonicMs = performance.now(),
  ) {
    this.context = context;
    this.startedAt = startedAt;
    this.startedAtMonotonicMs = startedAtMonotonicMs;
  }

  sample(
    snapshots: readonly CameraPerformanceSnapshot[],
    columnCount: number,
    nowMonotonicMs = performance.now(),
  ): void {
    const elapsedMs = elapsed(this.startedAtMonotonicMs, nowMonotonicMs);
    for (const snapshot of snapshots) {
      let peer = this.peers.get(snapshot.peerId);
      if (!peer) {
        peer = {
          name: snapshot.name,
          runtime: snapshot.runtime,
          receivedFrames: 0,
          renderedFrames: 0,
          receivedBytes: 0,
          samples: [],
        };
        this.peers.set(snapshot.peerId, peer);
      }

      peer.name = snapshot.name;
      peer.runtime = snapshot.runtime;
      peer.receivedFrames += counterDelta(
        snapshot.totalReceivedFrames,
        peer.lastReceivedFrames,
      );
      peer.renderedFrames += counterDelta(
        snapshot.totalRenderedFrames,
        peer.lastRenderedFrames,
      );
      peer.receivedBytes += counterDelta(
        snapshot.totalReceivedBytes,
        peer.lastReceivedBytes,
      );
      peer.lastReceivedFrames = snapshot.totalReceivedFrames;
      peer.lastRenderedFrames = snapshot.totalRenderedFrames;
      peer.lastReceivedBytes = snapshot.totalReceivedBytes;

      const next: CameraPerformanceSample = compactNumbers({
        elapsedMs,
        columns: columnCount,
        status: snapshot.status,
        quality: snapshot.quality,
        width: snapshot.width,
        height: snapshot.height,
        targetFps: snapshot.targetFps,
        receivedFrames: peer.receivedFrames,
        renderedFrames: peer.renderedFrames,
        receivedBytes: peer.receivedBytes,
        receiveFps: snapshot.receiveFps,
        renderFps: snapshot.renderFps,
        kibPerSecond: snapshot.kibPerSecond,
        frameAgeMs: snapshot.frameAgeMs,
      });
      const previous = peer.samples.at(-1);
      if (previous && previous.elapsedMs === elapsedMs) {
        peer.samples[peer.samples.length - 1] = next;
      } else {
        peer.samples.push(next);
      }
    }
  }

  recordEvent(message: string, nowMonotonicMs = performance.now()): void {
    this.events.push({
      elapsedMs: elapsed(this.startedAtMonotonicMs, nowMonotonicMs),
      message,
    });
  }

  finish(
    snapshots: readonly CameraPerformanceSnapshot[],
    finalColumnCount: number,
    endedAt = new Date(),
    endedAtMonotonicMs = performance.now(),
  ): CameraPerformanceReport {
    this.sample(snapshots, finalColumnCount, endedAtMonotonicMs);
    const peers = [...this.peers.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([peerId, peer]): CameraPerformancePeerReport => ({
        peerId,
        name: peer.name,
        runtime: peer.runtime,
        samples: peer.samples,
        summary: summarizePeer(peer),
      }));

    return {
      schemaVersion: CAMERA_PERFORMANCE_REPORT_SCHEMA_VERSION,
      kind: CAMERA_PERFORMANCE_REPORT_KIND,
      runtime: this.context.runtime,
      platform: this.context.platform,
      domainId: this.context.domainId,
      localPeerId: this.context.localPeerId,
      startedAt: this.startedAt.toISOString(),
      endedAt: endedAt.toISOString(),
      durationMs: elapsed(this.startedAtMonotonicMs, endedAtMonotonicMs),
      sampleIntervalMs: CAMERA_PERFORMANCE_SAMPLE_INTERVAL_MS,
      initialColumns: this.context.columnCount,
      finalColumns: finalColumnCount,
      peers,
      events: this.events,
    };
  }
}

export function serializeCameraPerformanceReport(report: CameraPerformanceReport): string {
  return JSON.stringify(report, null, 2);
}

export function cameraPerformanceReportFilename(report: CameraPerformanceReport): string {
  return `camera-mesh-${report.runtime}-${report.startedAt.replaceAll(":", "-")}.json`;
}

function summarizePeer(peer: PeerAccumulator): CameraPerformancePeerSummary {
  const renderToReceiveRatio = peer.receivedFrames > 0
    ? round(peer.renderedFrames / peer.receivedFrames)
    : undefined;
  return {
    sampleCount: peer.samples.length,
    receivedFrames: peer.receivedFrames,
    renderedFrames: peer.renderedFrames,
    receivedBytes: peer.receivedBytes,
    renderToReceiveRatio,
    receiveFps: summarize(peer.samples.flatMap((sample) => finite(sample.receiveFps))),
    renderFps: summarize(peer.samples.flatMap((sample) => finite(sample.renderFps))),
    kibPerSecond: summarize(peer.samples.flatMap((sample) => finite(sample.kibPerSecond))),
    frameAgeMs: summarize(peer.samples.flatMap((sample) => finite(sample.frameAgeMs))),
  };
}

function summarize(values: readonly number[]): CameraPerformanceNumberSummary | undefined {
  if (values.length === 0) return undefined;
  const sorted = [...values].sort((left, right) => left - right);
  return {
    min: round(sorted[0]!),
    average: round(sorted.reduce((total, value) => total + value, 0) / sorted.length),
    max: round(sorted.at(-1)!),
    p50: round(percentile(sorted, 0.5)),
    p95: round(percentile(sorted, 0.95)),
  };
}

function percentile(sorted: readonly number[], percentileValue: number): number {
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(percentileValue * sorted.length) - 1))]!;
}

function counterDelta(current: number, previous: number | undefined): number {
  if (previous === undefined) return 0;
  return Math.max(0, current >= previous ? current - previous : current);
}

function elapsed(startedAt: number, now: number): number {
  return Math.max(0, Math.round(now - startedAt));
}

function finite(value: number | undefined): number[] {
  return value !== undefined && Number.isFinite(value) ? [value] : [];
}

function round(value: number): number {
  return Math.round(value * 1_000) / 1_000;
}

function compactNumbers(sample: CameraPerformanceSample): CameraPerformanceSample {
  return {
    ...sample,
    receiveFps: optionalRound(sample.receiveFps),
    renderFps: optionalRound(sample.renderFps),
    kibPerSecond: optionalRound(sample.kibPerSecond),
    frameAgeMs: optionalRound(sample.frameAgeMs),
  };
}

function optionalRound(value: number | undefined): number | undefined {
  return value !== undefined && Number.isFinite(value) ? round(value) : undefined;
}

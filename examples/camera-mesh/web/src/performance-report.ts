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
  readonly sourceGapP95Ms?: number;
  readonly sourceGapMaxMs?: number;
  readonly receiveGapP95Ms?: number;
  readonly receiveGapMaxMs?: number;
  readonly renderGapP95Ms?: number;
  readonly renderGapMaxMs?: number;
  readonly renderer?: string;
  readonly pageVisibility?: string;
  readonly renderVisible?: boolean;
  readonly rendererEnabled?: boolean;
  readonly decodeInFlight?: boolean;
  readonly pendingFrames?: number;
  readonly activeDecodes?: number;
  readonly queuedRenderers?: number;
  readonly maximumActiveDecodes?: number;
  readonly displayWidth?: number;
  readonly displayHeight?: number;
  readonly queueMs?: number;
  readonly queueP95Ms?: number;
  readonly queueMaxMs?: number;
  readonly decodeMs?: number;
  readonly decodeP50Ms?: number;
  readonly decodeP95Ms?: number;
  readonly decodeMaxMs?: number;
  readonly presentMs?: number;
  readonly totalSupersededFrames?: number;
  readonly totalQueueOverflowFrames?: number;
  readonly eventLoopDelayP95Ms?: number;
  readonly eventLoopDelayMaxMs?: number;
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
  readonly sourceGapP95Ms?: number;
  readonly sourceGapMaxMs?: number;
  readonly receiveGapP95Ms?: number;
  readonly receiveGapMaxMs?: number;
  readonly renderGapP95Ms?: number;
  readonly renderGapMaxMs?: number;
  readonly renderer?: string;
  readonly pageVisibility?: string;
  readonly renderVisible?: boolean;
  readonly rendererEnabled?: boolean;
  readonly decodeInFlight?: boolean;
  readonly pendingFrames?: number;
  readonly activeDecodes?: number;
  readonly queuedRenderers?: number;
  readonly maximumActiveDecodes?: number;
  readonly displayWidth?: number;
  readonly displayHeight?: number;
  readonly queueMs?: number;
  readonly queueP95Ms?: number;
  readonly queueMaxMs?: number;
  readonly decodeMs?: number;
  readonly decodeP50Ms?: number;
  readonly decodeP95Ms?: number;
  readonly decodeMaxMs?: number;
  readonly presentMs?: number;
  readonly supersededFrames?: number;
  readonly queueOverflowFrames?: number;
  readonly eventLoopDelayP95Ms?: number;
  readonly eventLoopDelayMaxMs?: number;
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
  readonly supersededFrames?: number;
  readonly queueOverflowFrames?: number;
  readonly renderToReceiveRatio?: number;
  readonly receiveFps?: CameraPerformanceNumberSummary;
  readonly renderFps?: CameraPerformanceNumberSummary;
  readonly kibPerSecond?: CameraPerformanceNumberSummary;
  readonly frameAgeMs?: CameraPerformanceNumberSummary;
  readonly sourceGapP95Ms?: CameraPerformanceNumberSummary;
  readonly sourceGapMaxMs?: CameraPerformanceNumberSummary;
  readonly receiveGapP95Ms?: CameraPerformanceNumberSummary;
  readonly receiveGapMaxMs?: CameraPerformanceNumberSummary;
  readonly renderGapP95Ms?: CameraPerformanceNumberSummary;
  readonly renderGapMaxMs?: CameraPerformanceNumberSummary;
  readonly pendingFrames?: CameraPerformanceNumberSummary;
  readonly activeDecodes?: CameraPerformanceNumberSummary;
  readonly queuedRenderers?: CameraPerformanceNumberSummary;
  readonly queueMs?: CameraPerformanceNumberSummary;
  readonly queueP95Ms?: CameraPerformanceNumberSummary;
  readonly queueMaxMs?: CameraPerformanceNumberSummary;
  readonly decodeMs?: CameraPerformanceNumberSummary;
  readonly decodeP50Ms?: CameraPerformanceNumberSummary;
  readonly decodeP95Ms?: CameraPerformanceNumberSummary;
  readonly decodeMaxMs?: CameraPerformanceNumberSummary;
  readonly presentMs?: CameraPerformanceNumberSummary;
  readonly eventLoopDelayP95Ms?: CameraPerformanceNumberSummary;
  readonly eventLoopDelayMaxMs?: CameraPerformanceNumberSummary;
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
  lastSupersededFrames?: number;
  lastQueueOverflowFrames?: number;
  receivedFrames: number;
  renderedFrames: number;
  receivedBytes: number;
  supersededFrames: number;
  queueOverflowFrames: number;
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
          supersededFrames: 0,
          queueOverflowFrames: 0,
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
      peer.supersededFrames += counterDelta(
        snapshot.totalSupersededFrames ?? 0,
        peer.lastSupersededFrames,
      );
      peer.queueOverflowFrames += counterDelta(
        snapshot.totalQueueOverflowFrames ?? 0,
        peer.lastQueueOverflowFrames,
      );
      peer.lastReceivedFrames = snapshot.totalReceivedFrames;
      peer.lastRenderedFrames = snapshot.totalRenderedFrames;
      peer.lastReceivedBytes = snapshot.totalReceivedBytes;
      peer.lastSupersededFrames = snapshot.totalSupersededFrames ?? 0;
      peer.lastQueueOverflowFrames = snapshot.totalQueueOverflowFrames ?? 0;

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
        sourceGapP95Ms: snapshot.sourceGapP95Ms,
        sourceGapMaxMs: snapshot.sourceGapMaxMs,
        receiveGapP95Ms: snapshot.receiveGapP95Ms,
        receiveGapMaxMs: snapshot.receiveGapMaxMs,
        renderGapP95Ms: snapshot.renderGapP95Ms,
        renderGapMaxMs: snapshot.renderGapMaxMs,
        renderer: snapshot.renderer,
        pageVisibility: snapshot.pageVisibility,
        renderVisible: snapshot.renderVisible,
        rendererEnabled: snapshot.rendererEnabled,
        decodeInFlight: snapshot.decodeInFlight,
        pendingFrames: snapshot.pendingFrames,
        activeDecodes: snapshot.activeDecodes,
        queuedRenderers: snapshot.queuedRenderers,
        maximumActiveDecodes: snapshot.maximumActiveDecodes,
        displayWidth: snapshot.displayWidth,
        displayHeight: snapshot.displayHeight,
        queueMs: snapshot.queueMs,
        queueP95Ms: snapshot.queueP95Ms,
        queueMaxMs: snapshot.queueMaxMs,
        decodeMs: snapshot.decodeMs,
        decodeP50Ms: snapshot.decodeP50Ms,
        decodeP95Ms: snapshot.decodeP95Ms,
        decodeMaxMs: snapshot.decodeMaxMs,
        presentMs: snapshot.presentMs,
        supersededFrames: peer.supersededFrames,
        queueOverflowFrames: peer.queueOverflowFrames,
        eventLoopDelayP95Ms: snapshot.eventLoopDelayP95Ms,
        eventLoopDelayMaxMs: snapshot.eventLoopDelayMaxMs,
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
    supersededFrames: peer.supersededFrames,
    queueOverflowFrames: peer.queueOverflowFrames,
    renderToReceiveRatio,
    receiveFps: summarize(peer.samples.flatMap((sample) => finite(sample.receiveFps))),
    renderFps: summarize(peer.samples.flatMap((sample) => finite(sample.renderFps))),
    kibPerSecond: summarize(peer.samples.flatMap((sample) => finite(sample.kibPerSecond))),
    frameAgeMs: summarize(peer.samples.flatMap((sample) => finite(sample.frameAgeMs))),
    sourceGapP95Ms: summarize(peer.samples.flatMap((sample) => finite(sample.sourceGapP95Ms))),
    sourceGapMaxMs: summarize(peer.samples.flatMap((sample) => finite(sample.sourceGapMaxMs))),
    receiveGapP95Ms: summarize(peer.samples.flatMap((sample) => finite(sample.receiveGapP95Ms))),
    receiveGapMaxMs: summarize(peer.samples.flatMap((sample) => finite(sample.receiveGapMaxMs))),
    renderGapP95Ms: summarize(peer.samples.flatMap((sample) => finite(sample.renderGapP95Ms))),
    renderGapMaxMs: summarize(peer.samples.flatMap((sample) => finite(sample.renderGapMaxMs))),
    pendingFrames: summarize(peer.samples.flatMap((sample) => finite(sample.pendingFrames))),
    activeDecodes: summarize(peer.samples.flatMap((sample) => finite(sample.activeDecodes))),
    queuedRenderers: summarize(peer.samples.flatMap((sample) => finite(sample.queuedRenderers))),
    queueMs: summarize(peer.samples.flatMap((sample) => finite(sample.queueMs))),
    queueP95Ms: summarize(peer.samples.flatMap((sample) => finite(sample.queueP95Ms))),
    queueMaxMs: summarize(peer.samples.flatMap((sample) => finite(sample.queueMaxMs))),
    decodeMs: summarize(peer.samples.flatMap((sample) => finite(sample.decodeMs))),
    decodeP50Ms: summarize(peer.samples.flatMap((sample) => finite(sample.decodeP50Ms))),
    decodeP95Ms: summarize(peer.samples.flatMap((sample) => finite(sample.decodeP95Ms))),
    decodeMaxMs: summarize(peer.samples.flatMap((sample) => finite(sample.decodeMaxMs))),
    presentMs: summarize(peer.samples.flatMap((sample) => finite(sample.presentMs))),
    eventLoopDelayP95Ms: summarize(
      peer.samples.flatMap((sample) => finite(sample.eventLoopDelayP95Ms)),
    ),
    eventLoopDelayMaxMs: summarize(
      peer.samples.flatMap((sample) => finite(sample.eventLoopDelayMaxMs)),
    ),
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
    sourceGapP95Ms: optionalRound(sample.sourceGapP95Ms),
    sourceGapMaxMs: optionalRound(sample.sourceGapMaxMs),
    receiveGapP95Ms: optionalRound(sample.receiveGapP95Ms),
    receiveGapMaxMs: optionalRound(sample.receiveGapMaxMs),
    renderGapP95Ms: optionalRound(sample.renderGapP95Ms),
    renderGapMaxMs: optionalRound(sample.renderGapMaxMs),
    queueMs: optionalRound(sample.queueMs),
    queueP95Ms: optionalRound(sample.queueP95Ms),
    queueMaxMs: optionalRound(sample.queueMaxMs),
    decodeMs: optionalRound(sample.decodeMs),
    decodeP50Ms: optionalRound(sample.decodeP50Ms),
    decodeP95Ms: optionalRound(sample.decodeP95Ms),
    decodeMaxMs: optionalRound(sample.decodeMaxMs),
    presentMs: optionalRound(sample.presentMs),
    eventLoopDelayP95Ms: optionalRound(sample.eventLoopDelayP95Ms),
    eventLoopDelayMaxMs: optionalRound(sample.eventLoopDelayMaxMs),
  };
}

function optionalRound(value: number | undefined): number | undefined {
  return value !== undefined && Number.isFinite(value) ? round(value) : undefined;
}

export type JpegPresentationBackend = "bitmaprenderer" | "2d";

export interface CameraFrameSurface extends HTMLCanvasElement {
  aukiLatestJpeg?: Uint8Array;
}

export interface JpegFrame {
  readonly jpeg: Uint8Array;
  readonly revision: number;
  readonly timestampNs: bigint;
  readonly sourceWidth: number;
  readonly sourceHeight: number;
  readonly rateHz: number;
}

export interface JpegPresentation {
  readonly revision: number;
  readonly timestampNs: bigint;
  readonly sourceWidth: number;
  readonly sourceHeight: number;
  readonly displayWidth: number;
  readonly displayHeight: number;
  readonly queueMs: number;
  readonly decodeMs: number;
  readonly presentMs: number;
  readonly backend: JpegPresentationBackend;
}

export interface JpegRendererMetrics {
  readonly backend?: JpegPresentationBackend;
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
  readonly enabled: boolean;
  readonly decodeInFlight: boolean;
  readonly pendingFrames: number;
  readonly activeDecodes: number;
  readonly queuedRenderers: number;
  readonly maximumActiveDecodes: number;
  readonly totalSupersededFrames: number;
  readonly totalQueueOverflowFrames: number;
}

interface BrowserFrameSchedulerMetrics {
  readonly activeDecodes: number;
  readonly queuedRenderers: number;
  readonly maximumActiveDecodes: number;
}

interface TimedMeasurement {
  readonly at: number;
  readonly value: number;
}

interface QueuedJpegFrame extends JpegFrame {
  readonly queuedAt: number;
}

interface DecodedJpegFrame {
  readonly frame: QueuedJpegFrame;
  readonly bitmap: ImageBitmap;
  readonly displayWidth: number;
  readonly displayHeight: number;
  readonly queueMs: number;
  readonly decodeMs: number;
}

interface JpegRendererHooks {
  presented(presentation: JpegPresentation): void;
  failed(error: unknown): void;
}

const MAX_ACTIVE_DECODES = window.matchMedia("(pointer: coarse)").matches ? 2 : 8;
const MAX_PENDING_FRAMES = 2;
const MAX_DECODED_FRAMES = 4;
const PLAYOUT_BUFFER_TIMEOUT_MS = 400;
const LOW_BUFFER_PERIOD_FACTOR = 1.03;
const HIGH_BUFFER_PERIOD_FACTOR = 0.97;
const MAX_PRESENTATION_EARLY_MS = 4;
const RENDER_PIXEL_RATIO_CAP = 1;
const METRIC_WINDOW_MS = 5_000;
const MAX_METRIC_SAMPLES = 256;

class BrowserFrameScheduler {
  private readonly queued = new Set<LatestJpegRenderer>();
  private active = 0;
  private pumpPending = false;

  constructor() {
    document.addEventListener("visibilitychange", () => this.requestPump());
  }

  enqueue(renderer: LatestJpegRenderer): void {
    this.queued.add(renderer);
    this.requestPump();
  }

  forget(renderer: LatestJpegRenderer): void {
    this.queued.delete(renderer);
  }

  metrics(): BrowserFrameSchedulerMetrics {
    return {
      activeDecodes: this.active,
      queuedRenderers: this.queued.size,
      maximumActiveDecodes: MAX_ACTIVE_DECODES,
    };
  }

  private requestPump(): void {
    if (
      this.pumpPending
      || this.queued.size === 0
      || document.visibilityState !== "visible"
    ) return;
    this.pumpPending = true;
    queueMicrotask(() => this.pump());
  }

  private pump(): void {
    this.pumpPending = false;
    if (document.visibilityState !== "visible") return;

    for (const renderer of [...this.queued]) {
      if (this.active >= MAX_ACTIVE_DECODES) break;
      this.queued.delete(renderer);
      const task = renderer.beginScheduledRender();
      if (!task) continue;
      this.active += 1;
      void task.finally(() => {
        this.active -= 1;
        renderer.scheduleIfNeeded();
        this.requestPump();
      });
    }
    if (this.queued.size > 0 && this.active < MAX_ACTIVE_DECODES) this.requestPump();
  }
}

const frameScheduler = new BrowserFrameScheduler();

class BrowserFramePresenter {
  private readonly active = new Set<LatestJpegRenderer>();
  private animationFrame?: number;

  constructor() {
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") this.requestFrame();
      else this.cancelFrame();
    });
  }

  enqueue(renderer: LatestJpegRenderer): void {
    this.active.add(renderer);
    this.requestFrame();
  }

  forget(renderer: LatestJpegRenderer): void {
    this.active.delete(renderer);
    if (this.active.size === 0) this.cancelFrame();
  }

  private requestFrame(): void {
    if (
      this.animationFrame !== undefined
      || this.active.size === 0
      || document.visibilityState !== "visible"
    ) return;
    this.animationFrame = requestAnimationFrame((now) => this.present(now));
  }

  private cancelFrame(): void {
    if (this.animationFrame === undefined) return;
    cancelAnimationFrame(this.animationFrame);
    this.animationFrame = undefined;
  }

  private present(now: number): void {
    this.animationFrame = undefined;
    for (const renderer of [...this.active]) {
      if (!renderer.presentIfDue(now)) this.active.delete(renderer);
    }
    this.requestFrame();
  }
}

const framePresenter = new BrowserFramePresenter();

/**
 * A bounded JPEG presenter shared by every Camera Mesh tile.
 *
 * Decodes are globally bounded and resized to the useful display size. A tiny
 * decoded-frame jitter buffer then presents them on a stable animation cadence,
 * absorbing the short delivery bursts produced by concurrent relay streams.
 * The last complete canvas remains visible while a camera re-buffers.
 */
export class LatestJpegRenderer {
  private readonly pending: QueuedJpegFrame[] = [];
  private readonly decoded: DecodedJpegFrame[] = [];
  private rendering = false;
  private enabled = false;
  private generation = 0;
  private rateHz = 5;
  private bufferingSince?: number;
  private nextPresentationAt?: number;
  private bitmapContext?: ImageBitmapRenderingContext | null;
  private canvasContext?: CanvasRenderingContext2D | null;
  private backend?: JpegPresentationBackend;
  private displayWidth?: number;
  private displayHeight?: number;
  private readonly queueSamples: TimedMeasurement[] = [];
  private readonly decodeSamples: TimedMeasurement[] = [];
  private readonly presentSamples: TimedMeasurement[] = [];
  private supersededFrames = 0;
  private queueOverflowFrames = 0;

  constructor(
    readonly surface: CameraFrameSurface,
    private readonly hooks: JpegRendererHooks,
  ) {}

  submit(frame: JpegFrame): void {
    if (frame.rateHz !== this.rateHz) {
      this.rateHz = frame.rateHz;
      this.resetPlayout();
    }
    if (this.pending.length === MAX_PENDING_FRAMES) {
      this.pending.shift();
      this.supersededFrames += 1;
      this.queueOverflowFrames += 1;
    }
    this.pending.push({ ...frame, queuedAt: performance.now() });
    this.scheduleIfNeeded();
  }

  setEnabled(enabled: boolean): void {
    if (this.enabled === enabled) {
      if (enabled) this.scheduleIfNeeded();
      return;
    }
    this.enabled = enabled;
    if (!enabled) {
      // Cancel an in-flight presentation without clearing the last complete
      // frame. The bounded compressed queue stays pending for when it is visible.
      this.generation += 1;
      frameScheduler.forget(this);
      framePresenter.forget(this);
      this.discardDecodedFrames();
      this.resetPlayout();
    } else {
      this.scheduleIfNeeded();
    }
  }

  invalidate(): void {
    this.generation += 1;
    this.supersededFrames += this.pending.length;
    this.pending.length = 0;
    frameScheduler.forget(this);
    framePresenter.forget(this);
    this.discardDecodedFrames();
    this.resetPlayout();
  }

  clear(): void {
    this.invalidate();
    this.enabled = false;
    this.backend = undefined;
    this.displayWidth = undefined;
    this.displayHeight = undefined;
    delete this.surface.aukiLatestJpeg;
    delete this.surface.dataset.renderedRevision;
    delete this.surface.dataset.sourceWidth;
    delete this.surface.dataset.sourceHeight;
    delete this.surface.dataset.displayWidth;
    delete this.surface.dataset.displayHeight;
    delete this.surface.dataset.renderer;
    this.surface.hidden = true;
    this.surface.width = 1;
    this.surface.height = 1;
  }

  resetMeasurements(): void {
    this.queueSamples.length = 0;
    this.decodeSamples.length = 0;
    this.presentSamples.length = 0;
  }

  metrics(now = performance.now(), detailed = false): JpegRendererMetrics {
    pruneMeasurements(this.queueSamples, now);
    pruneMeasurements(this.decodeSamples, now);
    pruneMeasurements(this.presentSamples, now);
    const scheduler = frameScheduler.metrics();
    return {
      backend: this.backend,
      displayWidth: this.displayWidth,
      displayHeight: this.displayHeight,
      queueMs: average(this.queueSamples),
      queueP95Ms: detailed ? percentile(this.queueSamples, 0.95) : undefined,
      queueMaxMs: detailed ? maximum(this.queueSamples) : undefined,
      decodeMs: average(this.decodeSamples),
      decodeP50Ms: detailed ? percentile(this.decodeSamples, 0.5) : undefined,
      decodeP95Ms: detailed ? percentile(this.decodeSamples, 0.95) : undefined,
      decodeMaxMs: detailed ? maximum(this.decodeSamples) : undefined,
      presentMs: average(this.presentSamples),
      enabled: this.enabled,
      decodeInFlight: this.rendering,
      pendingFrames: this.pending.length + this.decoded.length,
      ...scheduler,
      totalSupersededFrames: this.supersededFrames,
      totalQueueOverflowFrames: this.queueOverflowFrames,
    };
  }

  scheduleIfNeeded(): void {
    if (this.canRender()) frameScheduler.enqueue(this);
    if (this.canPresent()) framePresenter.enqueue(this);
  }

  beginScheduledRender(): Promise<void> | undefined {
    if (!this.canRender()) return undefined;
    const frame = this.pending.shift()!;
    this.rendering = true;
    return this.render(frame).finally(() => {
      this.rendering = false;
    });
  }

  private canRender(): boolean {
    return this.enabled
      && !this.rendering
      && this.pending.length > 0
      && this.decoded.length < MAX_DECODED_FRAMES
      && this.surface.isConnected
      && document.visibilityState === "visible";
  }

  private canPresent(): boolean {
    return this.enabled
      && (this.decoded.length > 0 || this.nextPresentationAt !== undefined)
      && this.surface.isConnected
      && document.visibilityState === "visible";
  }

  private async render(frame: QueuedJpegFrame): Promise<void> {
    const generation = this.generation;
    const target = usefulDisplaySize(
      this.surface,
      frame.sourceWidth,
      frame.sourceHeight,
    );
    const decodeStarted = performance.now();
    const queueMs = decodeStarted - frame.queuedAt;
    recordMeasurement(this.queueSamples, decodeStarted, queueMs);
    let bitmap: ImageBitmap | undefined;
    try {
      const blob = jpegBlob(frame.jpeg);
      bitmap = target.width === frame.sourceWidth && target.height === frame.sourceHeight
        ? await createImageBitmap(blob)
        : await createImageBitmap(blob, {
          resizeWidth: target.width,
          resizeHeight: target.height,
          resizeQuality: "medium",
        });
      const decodedAt = performance.now();
      const decodeMs = decodedAt - decodeStarted;
      recordMeasurement(this.decodeSamples, decodedAt, decodeMs);

      if (!this.isCurrent(generation)) {
        this.supersededFrames += 1;
        return;
      }
      this.decoded.push({
        frame,
        bitmap,
        displayWidth: target.width,
        displayHeight: target.height,
        queueMs,
        decodeMs,
      });
      bitmap = undefined;
      if (this.decoded.length > MAX_DECODED_FRAMES) {
        const superseded = this.decoded.shift()!;
        superseded.bitmap.close();
        this.supersededFrames += 1;
        this.queueOverflowFrames += 1;
      }
      this.bufferingSince ??= decodedAt;
      framePresenter.enqueue(this);
    } catch (error) {
      if (this.generation === generation) this.hooks.failed(error);
    } finally {
      bitmap?.close();
    }
  }

  presentIfDue(now: number): boolean {
    if (!this.canPresent()) return false;
    const targetBuffer = playoutBufferFrames(this.rateHz);
    this.bufferingSince ??= now;
    if (this.nextPresentationAt === undefined) {
      const bufferedEnough = this.decoded.length >= targetBuffer;
      const waitedLongEnough = now - this.bufferingSince >= PLAYOUT_BUFFER_TIMEOUT_MS;
      if (!bufferedEnough && !waitedLongEnough) return true;
      this.nextPresentationAt = now;
    }
    const nominalPeriod = 1_000 / Math.max(1, this.rateHz);
    const earlyTolerance = Math.min(MAX_PRESENTATION_EARLY_MS, nominalPeriod * 0.1);
    if (now + earlyTolerance < this.nextPresentationAt) return true;

    const bufferedBeforePresentation = this.decoded.length;
    const decoded = this.decoded.shift();
    if (!decoded) {
      this.resetPlayout(now);
      return this.pending.length > 0 || this.rendering;
    }
    this.presentDecoded(decoded);

    const period = bufferedBeforePresentation < targetBuffer
      ? nominalPeriod * LOW_BUFFER_PERIOD_FACTOR
      : bufferedBeforePresentation > targetBuffer
        ? nominalPeriod * HIGH_BUFFER_PERIOD_FACTOR
        : nominalPeriod;
    this.nextPresentationAt += period;
    if (this.nextPresentationAt < now - nominalPeriod) {
      this.nextPresentationAt = now + period;
    }
    this.scheduleIfNeeded();
    return this.decoded.length > 0 || this.pending.length > 0 || this.rendering;
  }

  private presentDecoded(decoded: DecodedJpegFrame): void {
    const { frame } = decoded;
    let bitmap: ImageBitmap | undefined = decoded.bitmap;
    try {
      const presentStarted = performance.now();
      const backend = this.present(bitmap);
      if (backend === "bitmaprenderer") bitmap = undefined;
      const presentedAt = performance.now();
      const presentMs = presentedAt - presentStarted;
      recordMeasurement(this.presentSamples, presentedAt, presentMs);

      this.backend = backend;
      this.displayWidth = decoded.displayWidth;
      this.displayHeight = decoded.displayHeight;
      this.surface.aukiLatestJpeg = frame.jpeg;
      this.surface.dataset.renderedRevision = String(frame.revision);
      this.surface.dataset.sourceWidth = String(frame.sourceWidth);
      this.surface.dataset.sourceHeight = String(frame.sourceHeight);
      this.surface.dataset.displayWidth = String(decoded.displayWidth);
      this.surface.dataset.displayHeight = String(decoded.displayHeight);
      this.surface.dataset.renderer = backend;
      this.hooks.presented({
        revision: frame.revision,
        timestampNs: frame.timestampNs,
        sourceWidth: frame.sourceWidth,
        sourceHeight: frame.sourceHeight,
        displayWidth: decoded.displayWidth,
        displayHeight: decoded.displayHeight,
        queueMs: decoded.queueMs,
        decodeMs: decoded.decodeMs,
        presentMs,
        backend,
      });
    } catch (error) {
      this.hooks.failed(error);
    } finally {
      bitmap?.close();
    }
  }

  private discardDecodedFrames(): void {
    this.supersededFrames += this.decoded.length;
    for (const decoded of this.decoded) decoded.bitmap.close();
    this.decoded.length = 0;
  }

  private resetPlayout(bufferingSince?: number): void {
    this.bufferingSince = bufferingSince;
    this.nextPresentationAt = undefined;
  }

  private isCurrent(generation: number): boolean {
    return this.generation === generation
      && this.enabled
      && this.surface.isConnected
      && document.visibilityState === "visible";
  }

  private present(bitmap: ImageBitmap): JpegPresentationBackend {
    if (this.surface.width !== bitmap.width || this.surface.height !== bitmap.height) {
      this.surface.width = bitmap.width;
      this.surface.height = bitmap.height;
    }
    if (this.bitmapContext === undefined) {
      this.bitmapContext = this.surface.getContext(
        "bitmaprenderer",
        { alpha: false },
      ) as ImageBitmapRenderingContext | null;
    }
    if (this.bitmapContext) {
      this.bitmapContext.transferFromImageBitmap(bitmap);
      return "bitmaprenderer";
    }
    this.canvasContext ??= this.surface.getContext("2d", { alpha: false });
    if (!this.canvasContext) throw new Error("browser could not create the camera canvas");
    this.canvasContext.drawImage(bitmap, 0, 0, this.surface.width, this.surface.height);
    return "2d";
  }
}

function usefulDisplaySize(
  surface: HTMLCanvasElement,
  sourceWidth: number,
  sourceHeight: number,
): { width: number; height: number } {
  const surfaceBounds = surface.getBoundingClientRect();
  const bounds = surfaceBounds.width > 1 && surfaceBounds.height > 1
    ? surfaceBounds
    : surface.parentElement?.getBoundingClientRect() ?? surfaceBounds;
  if (bounds.width <= 1 || bounds.height <= 1) {
    return { width: sourceWidth, height: sourceHeight };
  }
  const pixelRatio = Math.min(window.devicePixelRatio || 1, RENDER_PIXEL_RATIO_CAP);
  const scale = Math.min(
    1,
    bounds.width * pixelRatio / sourceWidth,
    bounds.height * pixelRatio / sourceHeight,
  );
  return {
    width: evenDimension(sourceWidth * scale, sourceWidth),
    height: evenDimension(sourceHeight * scale, sourceHeight),
  };
}

function playoutBufferFrames(rateHz: number): number {
  return rateHz <= 5 ? 2 : 3;
}

function jpegBlob(jpeg: Uint8Array): Blob {
  if (jpeg.buffer instanceof ArrayBuffer) {
    const bytes = jpeg.byteOffset === 0 && jpeg.byteLength === jpeg.buffer.byteLength
      ? jpeg.buffer
      : jpeg.buffer.slice(jpeg.byteOffset, jpeg.byteOffset + jpeg.byteLength);
    return new Blob([bytes], { type: "image/jpeg" });
  }
  return new Blob([Uint8Array.from(jpeg).buffer], { type: "image/jpeg" });
}

function evenDimension(value: number, maximum: number): number {
  return Math.min(maximum, Math.max(2, Math.round(value / 2) * 2));
}

function recordMeasurement(
  samples: TimedMeasurement[],
  at: number,
  value: number,
): void {
  samples.push({ at, value });
  pruneMeasurements(samples, at);
  if (samples.length > MAX_METRIC_SAMPLES) {
    samples.splice(0, samples.length - MAX_METRIC_SAMPLES);
  }
}

function pruneMeasurements(samples: TimedMeasurement[], now: number): void {
  const cutoff = now - METRIC_WINDOW_MS;
  while (samples.length > 0 && samples[0]!.at < cutoff) samples.shift();
}

function average(samples: readonly TimedMeasurement[]): number | undefined {
  if (samples.length === 0) return undefined;
  return samples.reduce((total, sample) => total + sample.value, 0) / samples.length;
}

function percentile(
  samples: readonly TimedMeasurement[],
  percentileValue: number,
): number | undefined {
  if (samples.length === 0) return undefined;
  const sorted = samples.map((sample) => sample.value).sort((left, right) => left - right);
  const index = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(percentileValue * sorted.length) - 1),
  );
  return sorted[index];
}

function maximum(samples: readonly TimedMeasurement[]): number | undefined {
  if (samples.length === 0) return undefined;
  return samples.reduce((result, sample) => Math.max(result, sample.value), -Infinity);
}

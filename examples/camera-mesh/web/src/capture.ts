import type { CameraStreamProfile } from "./profile.js";

export type CaptureMode = "webcam" | "synthetic";

export interface CapturedFrame {
  readonly revision: number;
  readonly timestampNs: bigint;
  readonly jpeg: Uint8Array;
  readonly payload: Uint8Array;
}

export interface StreamSourceItem {
  readonly timestampNs: bigint;
  readonly payload: Uint8Array;
}

export interface CaptureDiagnostics {
  readonly targetFps: number;
  readonly encodedFps?: number;
  readonly kibPerSecond?: number;
  readonly averageFrameKib?: number;
  readonly encodeP50Ms?: number;
  readonly encodeP95Ms?: number;
  readonly missedDeadlines: number;
  readonly inputFps?: number;
}

type EncodeFrame = (jpeg: Uint8Array) => Uint8Array;
type CaptureEvent = (message: string) => void;
type CaptureDiagnosticsEvent = (diagnostics: CaptureDiagnostics) => void;

const JPEG_QUALITY = 0.65;
const MAX_JPEG_BYTES = 1024 * 1024;
const DIAGNOSTIC_WINDOW_MS = 5_000;
const DIAGNOSTIC_REPORT_INTERVAL_MS = 500;
const MAX_DIAGNOSTIC_SAMPLES = 512;

interface CaptureSample {
  readonly completedAt: number;
  readonly bytes: number;
  readonly encodeMs: number;
}

class LatestFrameHub {
  private frame?: CapturedFrame;
  private revision = 0;
  private closed = false;
  private readonly waiters = new Set<() => void>();

  publish(timestampNs: bigint, jpeg: Uint8Array, payload: Uint8Array): void {
    if (this.closed) return;
    this.revision += 1;
    this.frame = { revision: this.revision, timestampNs, jpeg, payload };
    const waiters = [...this.waiters];
    this.waiters.clear();
    for (const wake of waiters) wake();
  }

  latest(): CapturedFrame | undefined {
    return this.frame;
  }

  async *source(): AsyncIterable<StreamSourceItem> {
    let observed = this.revision;
    while (true) {
      const frame = await this.after(observed);
      if (!frame) return;
      observed = frame.revision;
      yield { timestampNs: frame.timestampNs, payload: frame.payload };
    }
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    const waiters = [...this.waiters];
    this.waiters.clear();
    for (const wake of waiters) wake();
  }

  private async after(revision: number): Promise<CapturedFrame | undefined> {
    while (!this.closed && this.revision <= revision) {
      await new Promise<void>((resolve) => this.waiters.add(resolve));
    }
    return this.closed ? undefined : this.frame;
  }
}

export class CameraCapture {
  readonly width: number;
  readonly height: number;
  readonly frameRate: number;
  readonly jpegQuality = JPEG_QUALITY;

  private readonly frames = new LatestFrameHub();
  private readonly video = document.createElement("video");
  private media?: MediaStream;
  private running = false;
  private paused = false;
  private syntheticFrame = 0;
  private inputFps?: number;
  private missedDeadlines = 0;
  private lastDiagnosticReport = 0;
  private readonly samples: CaptureSample[] = [];

  constructor(
    private readonly canvas: HTMLCanvasElement,
    profile: CameraStreamProfile,
    private readonly encodeFrame: EncodeFrame,
    private readonly onEvent: CaptureEvent,
    private readonly onDiagnostics: CaptureDiagnosticsEvent,
  ) {
    this.width = profile.width;
    this.height = profile.height;
    this.frameRate = profile.rateHz;
    canvas.width = profile.width;
    canvas.height = profile.height;
    this.video.muted = true;
    this.video.playsInline = true;
  }

  async start(mode: CaptureMode): Promise<void> {
    if (this.running) throw new Error("camera capture is already running");
    if (mode === "webcam") {
      try {
        this.media = await navigator.mediaDevices.getUserMedia({
          audio: false,
          video: {
            width: { ideal: this.width },
            height: { ideal: this.height },
            frameRate: { ideal: this.frameRate, max: this.frameRate },
          },
        });
        this.video.srcObject = this.media;
        await this.video.play();
        const settings = this.media.getVideoTracks()[0]?.getSettings();
        this.inputFps = settings?.frameRate;
        if (this.inputFps !== undefined && this.inputFps + 0.5 < this.frameRate) {
          throw new Error(
            `webcam provides ${this.inputFps} fps; selected profile requires ${this.frameRate} fps`,
          );
        }
        this.onEvent(
          `Webcam permission granted${this.inputFps ? ` at ${this.inputFps} fps` : ""}`,
        );
      } catch (error) {
        for (const track of this.media?.getTracks() ?? []) track.stop();
        this.media = undefined;
        this.video.srcObject = null;
        throw error;
      }
    } else {
      this.onEvent("Synthetic camera ready");
    }
    this.running = true;
    void this.captureLoop(mode);
  }

  source(): AsyncIterable<StreamSourceItem> {
    return this.frames.source();
  }

  latestJpeg(): Uint8Array | undefined {
    return this.frames.latest()?.jpeg.slice();
  }

  pause(): void {
    if (!this.running || this.paused) return;
    this.paused = true;
    this.onEvent("Camera paused by an approved viewer");
  }

  resume(): void {
    if (!this.running || !this.paused) return;
    this.paused = false;
    this.onEvent("Camera resumed by an approved viewer");
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;
    // Let the at-most-one pending frame timer resolve naturally. Clearing it
    // would strand captureLoop on an unresolved Promise during shutdown.
    for (const track of this.media?.getTracks() ?? []) track.stop();
    this.media = undefined;
    this.video.srcObject = null;
    this.frames.close();
  }

  private async captureLoop(mode: CaptureMode): Promise<void> {
    const periodMs = 1_000 / this.frameRate;
    let nextDeadline = performance.now();
    while (this.running) {
      const delay = nextDeadline - performance.now();
      if (delay > 0) {
        await new Promise<void>((resolve) => window.setTimeout(resolve, delay));
      }
      if (!this.running) break;
      const started = performance.now();
      try {
        if (!this.paused) {
          this.draw(mode);
          const encodeStarted = performance.now();
          const jpeg = await canvasJpeg(this.canvas, JPEG_QUALITY);
          const completedAt = performance.now();
          if (jpeg.byteLength > MAX_JPEG_BYTES) {
            throw new Error(
              `JPEG frame is ${jpeg.byteLength} bytes; maximum is ${MAX_JPEG_BYTES}`,
            );
          }
          const timestampNs = BigInt(Date.now()) * 1_000_000n;
          this.frames.publish(timestampNs, jpeg, this.encodeFrame(jpeg));
          this.recordDiagnostics({
            completedAt,
            bytes: jpeg.byteLength,
            encodeMs: completedAt - encodeStarted,
          });
        }
      } catch (error) {
        this.onEvent(`Capture frame failed: ${errorMessage(error)}`);
      }
      nextDeadline += periodMs;
      const overrun = performance.now() - nextDeadline;
      if (overrun >= 0) {
        const skipped = Math.floor(overrun / periodMs) + 1;
        this.missedDeadlines += skipped;
        nextDeadline += skipped * periodMs;
      }
      // Account for an unusually early or non-monotonic timer without drifting.
      if (nextDeadline < started) nextDeadline = started + periodMs;
    }
  }

  private draw(mode: CaptureMode): void {
    const context = this.canvas.getContext("2d");
    if (!context) throw new Error("2D canvas is unavailable");
    if (mode === "webcam") {
      drawCover(context, this.video, this.width, this.height);
      return;
    }

    this.syntheticFrame += 1;
    const hue = (this.syntheticFrame * 4) % 360;
    const gradient = context.createLinearGradient(0, 0, this.width, this.height);
    gradient.addColorStop(0, `hsl(${hue} 75% 48%)`);
    gradient.addColorStop(1, `hsl(${(hue + 90) % 360} 72% 18%)`);
    context.fillStyle = gradient;
    context.fillRect(0, 0, this.width, this.height);
    context.fillStyle = "rgba(255,255,255,.94)";
    const scale = this.width / 480;
    context.font = `700 ${30 * scale}px ui-sans-serif, system-ui`;
    context.fillText("Auki Camera Mesh", 28 * scale, 58 * scale);
    context.font = `500 ${18 * scale}px ui-monospace, monospace`;
    context.fillText(new Date().toISOString(), 28 * scale, 96 * scale);
    context.fillText(`frame ${this.syntheticFrame}`, 28 * scale, 128 * scale);
    context.beginPath();
    context.arc(
      400 * scale,
      190 * scale,
      (28 + Math.sin(this.syntheticFrame / 3) * 10) * scale,
      0,
      Math.PI * 2,
    );
    context.fill();
  }

  private recordDiagnostics(sample: CaptureSample): void {
    this.samples.push(sample);
    const cutoff = sample.completedAt - DIAGNOSTIC_WINDOW_MS;
    while (this.samples.length > 2 && this.samples[1]!.completedAt < cutoff) {
      this.samples.shift();
    }
    if (this.samples.length > MAX_DIAGNOSTIC_SAMPLES) {
      this.samples.splice(0, this.samples.length - MAX_DIAGNOSTIC_SAMPLES);
    }
    if (sample.completedAt - this.lastDiagnosticReport < DIAGNOSTIC_REPORT_INTERVAL_MS) return;
    this.lastDiagnosticReport = sample.completedAt;
    this.onDiagnostics(this.diagnostics());
  }

  private diagnostics(): CaptureDiagnostics {
    const totalBytes = this.samples.reduce((total, sample) => total + sample.bytes, 0);
    const encodeTimes = this.samples
      .map((sample) => sample.encodeMs)
      .sort((left, right) => left - right);
    const result: CaptureDiagnostics = {
      targetFps: this.frameRate,
      averageFrameKib: this.samples.length ? totalBytes / this.samples.length / 1_024 : undefined,
      encodeP50Ms: percentile(encodeTimes, 0.5),
      encodeP95Ms: percentile(encodeTimes, 0.95),
      missedDeadlines: this.missedDeadlines,
      inputFps: this.inputFps,
    };
    if (this.samples.length < 2) return result;
    const first = this.samples[0]!;
    const last = this.samples[this.samples.length - 1]!;
    const elapsedSeconds = Math.max(0.001, (last.completedAt - first.completedAt) / 1_000);
    return {
      ...result,
      encodedFps: (this.samples.length - 1) / elapsedSeconds,
      kibPerSecond: (totalBytes - first.bytes) / elapsedSeconds / 1_024,
    };
  }
}

function drawCover(
  context: CanvasRenderingContext2D,
  video: HTMLVideoElement,
  width: number,
  height: number,
): void {
  const sourceWidth = video.videoWidth || width;
  const sourceHeight = video.videoHeight || height;
  const scale = Math.max(width / sourceWidth, height / sourceHeight);
  const drawnWidth = sourceWidth * scale;
  const drawnHeight = sourceHeight * scale;
  context.drawImage(
    video,
    (width - drawnWidth) / 2,
    (height - drawnHeight) / 2,
    drawnWidth,
    drawnHeight,
  );
}

async function canvasJpeg(canvas: HTMLCanvasElement, quality: number): Promise<Uint8Array> {
  const blob = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (value) => value ? resolve(value) : reject(new Error("canvas returned no JPEG")),
      "image/jpeg",
      quality,
    );
  });
  return new Uint8Array(await blob.arrayBuffer());
}

function percentile(sorted: readonly number[], ratio: number): number | undefined {
  if (sorted.length === 0) return undefined;
  const index = Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * ratio));
  return sorted[index];
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

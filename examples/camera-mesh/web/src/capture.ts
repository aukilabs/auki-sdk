import {
  CAMERA_QUALITY_TIERS,
  cameraStreamProfile,
  type CameraQualityTier,
  type CameraStreamProfile,
} from "./profile.js";

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
  readonly quality: CameraQualityTier;
  readonly profile: CameraStreamProfile;
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

interface RenditionState {
  readonly profile: CameraStreamProfile;
  readonly canvas: HTMLCanvasElement;
  readonly frames: LatestFrameHub;
  readonly samples: CaptureSample[];
  syntheticFrame: number;
  missedDeadlines: number;
  lastDiagnosticReport: number;
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
  readonly jpegQuality = JPEG_QUALITY;

  private readonly renditions = new Map<CameraQualityTier, RenditionState>();
  private readonly video = document.createElement("video");
  private media?: MediaStream;
  private running = false;
  private paused = false;
  private inputFps?: number;

  constructor(
    preview: HTMLCanvasElement,
    private readonly encodeFrame: EncodeFrame,
    private readonly onEvent: CaptureEvent,
    private readonly onDiagnostics: CaptureDiagnosticsEvent,
  ) {
    for (const quality of CAMERA_QUALITY_TIERS) {
      const profile = cameraStreamProfile(quality);
      const canvas = quality === "high" ? preview : document.createElement("canvas");
      canvas.width = profile.width;
      canvas.height = profile.height;
      this.renditions.set(quality, {
        profile,
        canvas,
        frames: new LatestFrameHub(),
        samples: [],
        syntheticFrame: 0,
        missedDeadlines: 0,
        lastDiagnosticReport: 0,
      });
    }
    this.video.muted = true;
    this.video.playsInline = true;
  }

  async start(mode: CaptureMode): Promise<void> {
    if (this.running) throw new Error("camera capture is already running");
    const high = this.rendition("high").profile;
    if (mode === "webcam") {
      try {
        this.media = await navigator.mediaDevices.getUserMedia({
          audio: false,
          video: {
            width: { ideal: high.width },
            height: { ideal: high.height },
            frameRate: { ideal: high.rateHz, max: high.rateHz },
          },
        });
        this.video.srcObject = this.media;
        await this.video.play();
        const settings = this.media.getVideoTracks()[0]?.getSettings();
        this.inputFps = settings?.frameRate;
        if (this.inputFps !== undefined && this.inputFps + 0.5 < high.rateHz) {
          this.onEvent(
            `Webcam input is ${this.inputFps} fps; the high rendition remains capped at ${high.rateHz} fps`,
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
    for (const rendition of this.renditions.values()) {
      void this.captureLoop(rendition, mode);
    }
  }

  source(quality: CameraQualityTier): AsyncIterable<StreamSourceItem> {
    return this.rendition(quality).frames.source();
  }

  // Snapshot controls predate rendition selection and carry no resource ID.
  // Keep their bytes on the legacy camera/main profile for every runtime.
  latestJpeg(quality: CameraQualityTier = "low"): Uint8Array | undefined {
    return this.rendition(quality).frames.latest()?.jpeg.slice();
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
    for (const rendition of this.renditions.values()) rendition.frames.close();
  }

  private async captureLoop(rendition: RenditionState, mode: CaptureMode): Promise<void> {
    const { profile } = rendition;
    const periodMs = 1_000 / profile.rateHz;
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
          this.draw(rendition, mode);
          const encodeStarted = performance.now();
          const jpeg = await canvasJpeg(rendition.canvas, JPEG_QUALITY);
          const completedAt = performance.now();
          if (jpeg.byteLength > MAX_JPEG_BYTES) {
            throw new Error(
              `${profile.quality} JPEG frame is ${jpeg.byteLength} bytes; maximum is ${MAX_JPEG_BYTES}`,
            );
          }
          const timestampNs = BigInt(Date.now()) * 1_000_000n;
          rendition.frames.publish(timestampNs, jpeg, this.encodeFrame(jpeg));
          this.recordDiagnostics(rendition, {
            completedAt,
            bytes: jpeg.byteLength,
            encodeMs: completedAt - encodeStarted,
          });
        }
      } catch (error) {
        this.onEvent(`${profile.quality} capture frame failed: ${errorMessage(error)}`);
      }
      nextDeadline += periodMs;
      const overrun = performance.now() - nextDeadline;
      if (overrun >= 0) {
        const skipped = Math.floor(overrun / periodMs) + 1;
        rendition.missedDeadlines += skipped;
        nextDeadline += skipped * periodMs;
      }
      // Account for an unusually early or non-monotonic timer without drifting.
      if (nextDeadline < started) nextDeadline = started + periodMs;
    }
  }

  private draw(rendition: RenditionState, mode: CaptureMode): void {
    const { canvas, profile } = rendition;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("2D canvas is unavailable");
    if (mode === "webcam") {
      drawCover(context, this.video, profile.width, profile.height);
      return;
    }

    rendition.syntheticFrame += 1;
    const gradient = context.createLinearGradient(0, 0, profile.width, profile.height);
    gradient.addColorStop(0, "#123b32");
    gradient.addColorStop(1, "#061015");
    context.fillStyle = gradient;
    context.fillRect(0, 0, profile.width, profile.height);
    const scale = profile.width / 480;
    context.strokeStyle = "rgba(89,227,173,.12)";
    context.lineWidth = Math.max(1, scale);
    for (let column = 1; column < 8; column += 1) {
      const x = profile.width * column / 8;
      context.beginPath();
      context.moveTo(x, 0);
      context.lineTo(x, profile.height);
      context.stroke();
    }
    for (let row = 1; row < 5; row += 1) {
      const y = profile.height * row / 5;
      context.beginPath();
      context.moveTo(0, y);
      context.lineTo(profile.width, y);
      context.stroke();
    }
    context.fillStyle = "rgba(255,255,255,.94)";
    context.font = `700 ${30 * scale}px ui-sans-serif, system-ui`;
    context.fillText("Auki Camera Mesh", 28 * scale, 58 * scale);
    context.font = `500 ${18 * scale}px ui-monospace, monospace`;
    context.fillText(new Date().toISOString(), 28 * scale, 96 * scale);
    context.fillText(
      `${profile.quality} · frame ${rendition.syntheticFrame}`,
      28 * scale,
      128 * scale,
    );
    const phase = performance.now() / 1_000;
    const markerX = (240 + Math.sin(phase * 1.7) * 160) * scale;
    const markerY = (190 + Math.cos(phase * 1.2) * 40) * scale;
    context.fillStyle = "rgba(89,227,173,.22)";
    context.beginPath();
    context.arc(markerX, markerY, 30 * scale, 0, Math.PI * 2);
    context.fill();
    context.fillStyle = "#59e3ad";
    context.beginPath();
    context.arc(markerX, markerY, 12 * scale, 0, Math.PI * 2);
    context.fill();
  }

  private recordDiagnostics(rendition: RenditionState, sample: CaptureSample): void {
    rendition.samples.push(sample);
    const cutoff = sample.completedAt - DIAGNOSTIC_WINDOW_MS;
    while (rendition.samples.length > 2 && rendition.samples[1]!.completedAt < cutoff) {
      rendition.samples.shift();
    }
    if (rendition.samples.length > MAX_DIAGNOSTIC_SAMPLES) {
      rendition.samples.splice(0, rendition.samples.length - MAX_DIAGNOSTIC_SAMPLES);
    }
    if (sample.completedAt - rendition.lastDiagnosticReport < DIAGNOSTIC_REPORT_INTERVAL_MS) return;
    rendition.lastDiagnosticReport = sample.completedAt;
    this.onDiagnostics(this.diagnostics(rendition));
  }

  private diagnostics(rendition: RenditionState): CaptureDiagnostics {
    const { profile, samples } = rendition;
    const totalBytes = samples.reduce((total, sample) => total + sample.bytes, 0);
    const encodeTimes = samples
      .map((sample) => sample.encodeMs)
      .sort((left, right) => left - right);
    const result: CaptureDiagnostics = {
      quality: profile.quality,
      profile,
      targetFps: profile.rateHz,
      averageFrameKib: samples.length ? totalBytes / samples.length / 1_024 : undefined,
      encodeP50Ms: percentile(encodeTimes, 0.5),
      encodeP95Ms: percentile(encodeTimes, 0.95),
      missedDeadlines: rendition.missedDeadlines,
      inputFps: this.inputFps,
    };
    if (samples.length < 2) return result;
    const first = samples[0]!;
    const last = samples[samples.length - 1]!;
    const elapsedSeconds = Math.max(0.001, (last.completedAt - first.completedAt) / 1_000);
    return {
      ...result,
      encodedFps: (samples.length - 1) / elapsedSeconds,
      kibPerSecond: (totalBytes - first.bytes) / elapsedSeconds / 1_024,
    };
  }

  private rendition(quality: CameraQualityTier): RenditionState {
    const rendition = this.renditions.get(quality);
    if (!rendition) throw new Error(`camera rendition ${quality} is unavailable`);
    return rendition;
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

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

type EncodeFrame = (jpeg: Uint8Array) => Uint8Array;
type CaptureEvent = (message: string) => void;

const WIDTH = 480;
const HEIGHT = 270;
const FRAME_RATE = 5;
const JPEG_QUALITY = 0.65;
const MAX_JPEG_BYTES = 512 * 1024;

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
  readonly width = WIDTH;
  readonly height = HEIGHT;
  readonly frameRate = FRAME_RATE;
  readonly jpegQuality = JPEG_QUALITY;

  private readonly frames = new LatestFrameHub();
  private readonly video = document.createElement("video");
  private media?: MediaStream;
  private running = false;
  private paused = false;
  private syntheticFrame = 0;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly encodeFrame: EncodeFrame,
    private readonly onEvent: CaptureEvent,
  ) {
    canvas.width = WIDTH;
    canvas.height = HEIGHT;
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
            width: { ideal: WIDTH },
            height: { ideal: HEIGHT },
            frameRate: { ideal: FRAME_RATE, max: FRAME_RATE },
          },
        });
        this.video.srcObject = this.media;
        await this.video.play();
        this.onEvent("Webcam permission granted");
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
    const periodMs = 1_000 / FRAME_RATE;
    while (this.running) {
      const started = performance.now();
      try {
        if (!this.paused) {
          this.draw(mode);
          const jpeg = await canvasJpeg(this.canvas, JPEG_QUALITY);
          if (jpeg.byteLength > MAX_JPEG_BYTES) {
            throw new Error(
              `JPEG frame is ${jpeg.byteLength} bytes; maximum is ${MAX_JPEG_BYTES}`,
            );
          }
          const timestampNs = BigInt(Date.now()) * 1_000_000n;
          this.frames.publish(timestampNs, jpeg, this.encodeFrame(jpeg));
        }
      } catch (error) {
        this.onEvent(`Capture frame failed: ${errorMessage(error)}`);
      }
      const delay = Math.max(0, periodMs - (performance.now() - started));
      await new Promise<void>((resolve) => window.setTimeout(resolve, delay));
    }
  }

  private draw(mode: CaptureMode): void {
    const context = this.canvas.getContext("2d");
    if (!context) throw new Error("2D canvas is unavailable");
    if (mode === "webcam") {
      drawCover(context, this.video, WIDTH, HEIGHT);
      return;
    }

    this.syntheticFrame += 1;
    const hue = (this.syntheticFrame * 4) % 360;
    const gradient = context.createLinearGradient(0, 0, WIDTH, HEIGHT);
    gradient.addColorStop(0, `hsl(${hue} 75% 48%)`);
    gradient.addColorStop(1, `hsl(${(hue + 90) % 360} 72% 18%)`);
    context.fillStyle = gradient;
    context.fillRect(0, 0, WIDTH, HEIGHT);
    context.fillStyle = "rgba(255,255,255,.94)";
    context.font = "700 30px ui-sans-serif, system-ui";
    context.fillText("Auki Camera Mesh", 28, 58);
    context.font = "500 18px ui-monospace, monospace";
    context.fillText(new Date().toISOString(), 28, 96);
    context.fillText(`frame ${this.syntheticFrame}`, 28, 128);
    context.beginPath();
    context.arc(400, 190, 28 + Math.sin(this.syntheticFrame / 3) * 10, 0, Math.PI * 2);
    context.fill();
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

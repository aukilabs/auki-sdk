export type CameraStreamEntry = {
  timestamp_ns: number;
  seq: number;
  payload: Uint8Array;
};

export type CameraJpegSourceOptions = {
  intervalMs: number;
  captureFrame: () => Promise<Uint8Array | null>;
  sleep?: (durationMs: number) => Promise<void>;
  nowNs?: () => number;
  shouldContinue?: () => boolean;
};

export type WebcamJpegSourceHandle = {
  source: AsyncIterable<CameraStreamEntry>;
  stream: MediaStream;
  stop(): void;
};

export const WEBCAM_JPEG_DEFAULTS = {
  facingMode: "environment",
  intervalMs: 1000 / 30,
  quality: 0.5,
} as const;

export function createCameraJpegSource({
  intervalMs,
  captureFrame,
  sleep = defaultSleep,
  nowNs = defaultNowNs,
  shouldContinue = () => true,
}: CameraJpegSourceOptions): AsyncIterable<CameraStreamEntry> {
  return {
    async *[Symbol.asyncIterator]() {
      let seq = 0;
      while (shouldContinue()) {
        const payload = await captureFrame();
        if (payload != null) {
          yield {
            timestamp_ns: nowNs(),
            seq,
            payload,
          };
          seq += 1;
        }
        await sleep(intervalMs);
      }
    },
  };
}

export async function startWebcamJpegSource({
  facingMode = WEBCAM_JPEG_DEFAULTS.facingMode,
  intervalMs = WEBCAM_JPEG_DEFAULTS.intervalMs,
  quality = WEBCAM_JPEG_DEFAULTS.quality,
  width = 640,
  height = 360,
}: {
  facingMode?: "user" | "environment";
  intervalMs?: number;
  quality?: number;
  width?: number;
  height?: number;
} = {}): Promise<WebcamJpegSourceHandle> {
  if (!navigator.mediaDevices?.getUserMedia) {
    throw new Error("This browser does not expose webcam capture");
  }

  const stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      width: { ideal: width },
      height: { ideal: height },
      facingMode: { ideal: facingMode },
    },
  });
  const video = document.createElement("video");
  video.autoplay = true;
  video.muted = true;
  video.playsInline = true;
  video.srcObject = stream;
  attachHiddenVideo(video);
  try {
    await waitForVideo(video);
  } catch (error) {
    video.remove();
    stopTracks(stream);
    throw error;
  }

  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  if (context == null) {
    video.remove();
    stopTracks(stream);
    throw new Error("This browser does not expose 2D canvas capture");
  }

  let stopped = false;
  const source = createCameraJpegSource({
    intervalMs,
    shouldContinue: () => !stopped,
    captureFrame: async () => {
      if (stopped) {
        return null;
      }
      const frameWidth = video.videoWidth || width;
      const frameHeight = video.videoHeight || height;
      canvas.width = frameWidth;
      canvas.height = frameHeight;
      context.drawImage(video, 0, 0, frameWidth, frameHeight);
      return blobToBytes(await canvasToBlob(canvas, quality));
    },
  });

  return {
    source,
    stream,
    stop() {
      stopped = true;
      video.pause();
      video.srcObject = null;
      video.remove();
      stopTracks(stream);
    },
  };
}

function defaultSleep(durationMs: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, durationMs));
}

function defaultNowNs(): number {
  return Date.now() * 1_000_000;
}

async function waitForVideo(video: HTMLVideoElement): Promise<void> {
  if (video.readyState >= HTMLMediaElement.HAVE_METADATA && video.videoWidth > 0) {
    await video.play();
    return;
  }
  const metadataLoaded = new Promise<void>((resolve, reject) => {
    const cleanup = () => {
      video.removeEventListener("loadedmetadata", onLoaded);
      video.removeEventListener("error", onError);
    };
    const onLoaded = () => {
      cleanup();
      resolve();
    };
    const onError = () => {
      cleanup();
      reject(new Error("Webcam video could not be started"));
    };
    video.addEventListener("loadedmetadata", onLoaded, { once: true });
    video.addEventListener("error", onError, { once: true });
  });
  await Promise.all([metadataLoaded, video.play()]);
}

function canvasToBlob(canvas: HTMLCanvasElement, quality: number): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (blob == null) {
          reject(new Error("Canvas could not encode a JPEG frame"));
          return;
        }
        resolve(blob);
      },
      "image/jpeg",
      quality,
    );
  });
}

async function blobToBytes(blob: Blob): Promise<Uint8Array> {
  return new Uint8Array(await blob.arrayBuffer());
}

function stopTracks(stream: MediaStream) {
  for (const track of stream.getTracks()) {
    track.stop();
  }
}

function attachHiddenVideo(video: HTMLVideoElement) {
  video.style.position = "fixed";
  video.style.inset = "0 auto auto 0";
  video.style.width = "1px";
  video.style.height = "1px";
  video.style.opacity = "0";
  video.style.pointerEvents = "none";
  document.body.append(video);
}

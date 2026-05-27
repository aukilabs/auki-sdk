// Tile screenshot capture. Pure client-side: source the pixel data
// from the live <img> or past <video>, optionally burn a metadata
// banner along the bottom edge, write a PNG into a Blob, trigger a
// download via a temporary <a>. No server round-trip — Park doesn't
// need to know about the snapshot, the operator just gets a file.
//
// The banner is opt-in (default on) via the
// `park.screenshots.burnMetadata` localStorage flag — toggled in the
// settings overlay. Without the banner, the PNG is the raw frame, which
// is what you want for assembling montages or sharing without leaking
// session identifiers into the image.

import { toast } from "../../shell/toast";

const BURN_METADATA_KEY = "park.screenshots.burnMetadata";

export type LiveSnapshotMeta = {
  sensorId: string;
  daemonUrl: string;
  daemonName?: string;
  peerId?: string;
  /** Wall-clock ms when the frame arrived. */
  receivedAtWallMs?: number;
  /** Producer-stamped frame seq, useful for debugging drops. */
  seq?: number;
};

export type PastSnapshotMeta = {
  sessionId: string;
  sensorId: string;
  daemonUrl: string;
};

/** Default is true — operators almost always want context attached.
 * Settings overlay flips this. Read at click-time so changes apply
 * without a tile reload. */
export function shouldBurnMetadata(): boolean {
  try {
    const v = localStorage.getItem(BURN_METADATA_KEY);
    if (v == null) return true;
    return v !== "false";
  } catch {
    return true;
  }
}

export function setBurnMetadata(value: boolean): void {
  try {
    localStorage.setItem(BURN_METADATA_KEY, value ? "true" : "false");
  } catch {
    // Private mode etc. — silent best effort.
  }
}

export function captureLiveFrame(
  img: HTMLImageElement,
  meta: LiveSnapshotMeta,
): void {
  const w = img.naturalWidth;
  const h = img.naturalHeight;
  if (!w || !h) {
    toast.error("Snapshot: live tile has no frame yet.");
    return;
  }

  const lines: string[] = [];
  const sensorShort = meta.sensorId.split("/").pop() ?? meta.sensorId;
  lines.push(`${sensorShort}  ·  live`);
  if (meta.daemonName) lines.push(meta.daemonName);
  if (meta.receivedAtWallMs != null) {
    lines.push(new Date(meta.receivedAtWallMs).toISOString());
  }
  if (meta.seq != null) lines.push(`seq ${meta.seq}`);

  exportFrame({
    source: img,
    width: w,
    height: h,
    bannerLines: shouldBurnMetadata() ? lines : null,
    fileName: makeFileName({
      kind: "live",
      sensorId: meta.sensorId,
      daemonUrl: meta.daemonUrl,
      timestampMs: meta.receivedAtWallMs ?? Date.now(),
    }),
  });
}

export function capturePastFrame(
  video: HTMLVideoElement,
  meta: PastSnapshotMeta,
): void {
  const w = video.videoWidth;
  const h = video.videoHeight;
  if (!w || !h) {
    toast.error("Snapshot: video hasn't loaded a frame yet.");
    return;
  }
  const lines: string[] = [];
  const sensorShort = meta.sensorId.split("/").pop() ?? meta.sensorId;
  lines.push(`${sensorShort}  ·  past`);
  lines.push(`session ${meta.sessionId}`);
  lines.push(`t=${formatTimecode(video.currentTime)}`);

  exportFrame({
    source: video,
    width: w,
    height: h,
    bannerLines: shouldBurnMetadata() ? lines : null,
    fileName: makeFileName({
      kind: "past",
      sensorId: meta.sensorId,
      daemonUrl: meta.daemonUrl,
      timestampMs: Date.now(),
      sessionId: meta.sessionId,
    }),
  });
}

// ─── private ───────────────────────────────────────────────────────────────

type ExportArgs = {
  source: CanvasImageSource;
  width: number;
  height: number;
  /** When non-null, draw a translucent banner along the bottom and
   * render these lines into it. */
  bannerLines: string[] | null;
  fileName: string;
};

function exportFrame(args: ExportArgs): void {
  const { source, width, height, bannerLines, fileName } = args;
  const bannerH = bannerLines && bannerLines.length > 0 ? bannerHeight(bannerLines.length) : 0;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height + bannerH;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    toast.error("Snapshot: 2D canvas context unavailable.");
    return;
  }

  ctx.fillStyle = "#0F0F0F";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(source, 0, 0, width, height);

  if (bannerLines && bannerLines.length > 0) {
    drawBanner(ctx, width, height, bannerH, bannerLines);
  }

  canvas.toBlob((blob) => {
    if (!blob) {
      toast.error("Snapshot: PNG encode failed.");
      return;
    }
    triggerDownload(blob, fileName);
    toast.info(`Saved ${fileName}`);
  }, "image/png");
}

function bannerHeight(lineCount: number): number {
  return 14 + lineCount * 18;
}

function drawBanner(
  ctx: CanvasRenderingContext2D,
  width: number,
  imageHeight: number,
  bannerH: number,
  lines: string[],
): void {
  const top = imageHeight;
  ctx.fillStyle = "rgba(15, 15, 15, 0.92)";
  ctx.fillRect(0, top, width, bannerH);
  ctx.fillStyle = "#F5F5F5";
  ctx.font = "13px ui-monospace, SFMono-Regular, Menlo, monospace";
  ctx.textBaseline = "top";
  let y = top + 8;
  for (const line of lines) {
    ctx.fillText(line, 12, y, width - 24);
    y += 18;
  }
  // Right-aligned Park watermark — small, muted; helps trace where
  // the image came from when it shows up later in someone's slide deck.
  ctx.fillStyle = "rgba(245, 245, 245, 0.45)";
  ctx.font = "11px ui-monospace, SFMono-Regular, Menlo, monospace";
  ctx.textAlign = "right";
  ctx.fillText("park", width - 12, top + 8, width / 4);
  ctx.textAlign = "start";
}

function triggerDownload(blob: Blob, fileName: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Defer revoke so the download dialog has time to read the URL on
  // some browsers (Safari was historically picky).
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

type FileNameArgs = {
  kind: "live" | "past";
  sensorId: string;
  daemonUrl: string;
  timestampMs: number;
  sessionId?: string;
};

function makeFileName(args: FileNameArgs): string {
  const sensor = sanitize(args.sensorId.split("/").pop() ?? args.sensorId);
  const daemon = sanitize(daemonHostFromUrl(args.daemonUrl));
  const ts = new Date(args.timestampMs).toISOString().replace(/[:.]/g, "-").replace(/Z$/, "Z");
  if (args.kind === "past" && args.sessionId) {
    return `park-frame-${daemon}-${sensor}-session-${sanitize(args.sessionId)}-${ts}.png`;
  }
  return `park-frame-${daemon}-${sensor}-${ts}.png`;
}

function daemonHostFromUrl(url: string): string {
  try {
    const u = new URL(url);
    return u.host || u.hostname || url;
  } catch {
    return url;
  }
}

function sanitize(s: string): string {
  return s.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "");
}

function formatTimecode(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const m = Math.floor(total / 60);
  const s = (total % 60).toString().padStart(2, "0");
  const ms = Math.floor((seconds - total) * 1000).toString().padStart(3, "0");
  return `${m}:${s}.${ms}`;
}

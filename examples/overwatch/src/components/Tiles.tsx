import { useEffect, useState } from "react";

export type StageTileSpec = {
  kind: string;
  peerId: string;
  sensorId: string;
  latestMessage?: unknown;
};

export function Tile({
  tile,
  paused,
  onPauseToggle,
  onClose,
}: {
  tile: StageTileSpec;
  paused: boolean;
  onPauseToggle: () => void;
  onClose: () => void;
}) {
  return (
    <article
      className="grid min-h-[180px] grid-rows-[auto_minmax(0,1fr)] rounded-control border border-line bg-ink/65"
      data-testid="stage-tile"
    >
      <div className="flex items-center justify-between gap-3 border-b border-line px-3 py-2">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-white">{tile.kind}</h3>
          <p className="truncate font-mono text-xs text-slate-400">{tile.sensorId}</p>
        </div>
        <div className="flex items-center gap-1">
          <button
            aria-label={`${paused ? "resume" : "pause"} ${tile.sensorId}`}
            className="grid h-7 w-7 place-items-center rounded-control border border-line text-xs text-slate-300 hover:border-signal hover:text-white"
            onClick={onPauseToggle}
            type="button"
          >
            {paused ? "▶" : "Ⅱ"}
          </button>
          <button
            aria-label={`snapshot ${tile.sensorId}`}
            className="grid h-7 w-7 place-items-center rounded-control border border-line text-xs text-slate-300 hover:border-signal hover:text-white"
            onClick={() => downloadSnapshot(tile)}
            type="button"
          >
            ↓
          </button>
          <button
            aria-label={`close ${tile.sensorId}`}
            className="grid h-7 w-7 place-items-center rounded-control border border-line text-sm text-slate-300 hover:border-red-800 hover:text-red-200"
            onClick={onClose}
            type="button"
          >
            ×
          </button>
        </div>
      </div>
      <div className="grid place-items-center p-4 text-center">
        <div>
          <p className="text-xs uppercase text-slate-500">{paused ? "paused" : "streaming"}</p>
          <p className="mt-2 font-mono text-sm text-slate-200">{shortPeer(tile.peerId)}</p>
          <TilePayload tile={tile} />
        </div>
      </div>
    </article>
  );
}

function TilePayload({ tile }: { tile: StageTileSpec }) {
  const cameraFrameUrl = useCameraFrameUrl(tile);
  if (tile.kind === "camera" && cameraFrameUrl != null) {
    return (
      <img
        alt={`camera frame ${tile.sensorId}`}
        className="mt-3 max-h-64 w-full rounded-control border border-line object-contain"
        src={cameraFrameUrl}
      />
    );
  }

  return (
    <pre className="mt-3 max-h-24 max-w-full overflow-hidden rounded-control bg-panel p-2 text-left text-[11px] text-slate-400">
      {JSON.stringify(tile.latestMessage ?? { waiting: true }, null, 2)}
    </pre>
  );
}

function useCameraFrameUrl(tile: StageTileSpec): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    const payload = tile.kind === "camera" ? extractPayloadBytes(tile.latestMessage) : null;
    if (payload == null) {
      setUrl(null);
      return undefined;
    }
    const nextUrl = URL.createObjectURL(new Blob([payload], { type: "image/jpeg" }));
    setUrl(nextUrl);
    return () => {
      URL.revokeObjectURL(nextUrl);
    };
  }, [tile.kind, tile.latestMessage]);
  return url;
}

function downloadSnapshot(tile: StageTileSpec) {
  const cameraPayload = tile.kind === "camera" ? extractPayloadBytes(tile.latestMessage) : null;
  const blob =
    cameraPayload == null
      ? new Blob([JSON.stringify(tile, null, 2)], { type: "application/json" })
      : new Blob([cameraPayload], { type: "image/jpeg" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `${tile.sensorId.replaceAll("/", "_")}.${cameraPayload == null ? "json" : "jpg"}`;
  anchor.click();
  URL.revokeObjectURL(url);
}

function extractPayloadBytes(message: unknown): Uint8Array | null {
  if (typeof message !== "object" || message == null || !("entry" in message)) {
    return null;
  }
  const entry = (message as { entry?: unknown }).entry;
  if (typeof entry !== "object" || entry == null || !("payload" in entry)) {
    return null;
  }
  const payload = (entry as { payload?: unknown }).payload;
  if (payload instanceof Uint8Array) {
    return payload;
  }
  if (Array.isArray(payload) && payload.every((value) => Number.isInteger(value))) {
    return Uint8Array.from(payload);
  }
  return null;
}

function shortPeer(peerId: string): string {
  return peerId.length <= 12 ? peerId : `${peerId.slice(0, 6)}...${peerId.slice(-6)}`;
}

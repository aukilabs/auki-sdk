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
          <pre className="mt-3 max-h-24 max-w-full overflow-hidden rounded-control bg-panel p-2 text-left text-[11px] text-slate-400">
            {JSON.stringify(tile.latestMessage ?? { waiting: true }, null, 2)}
          </pre>
        </div>
      </div>
    </article>
  );
}

function downloadSnapshot(tile: StageTileSpec) {
  const blob = new Blob([JSON.stringify(tile, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `${tile.sensorId.replaceAll("/", "_")}.json`;
  anchor.click();
  URL.revokeObjectURL(url);
}

function shortPeer(peerId: string): string {
  return peerId.length <= 12 ? peerId : `${peerId.slice(0, 6)}...${peerId.slice(-6)}`;
}

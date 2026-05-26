import { useEffect, useMemo, useState } from "react";

import { Tile, type StageTileSpec } from "./Tiles";

export type { StageTileSpec };

export function Stage({
  tiles,
  onCloseTile,
}: {
  tiles: StageTileSpec[];
  onCloseTile?: (tile: StageTileSpec) => void;
}) {
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const [paused, setPaused] = useState<Set<string>>(new Set());

  useEffect(() => {
    setClosed((current) => new Set([...current].filter((key) => tiles.some((tile) => tileKey(tile) === key))));
  }, [tiles]);

  const visibleTiles = useMemo(
    () => tiles.filter((tile) => !closed.has(tileKey(tile))),
    [closed, tiles],
  );

  return (
    <section className="grid min-h-[360px] grid-rows-[auto_minmax(0,1fr)] rounded-control border border-line bg-panel">
      <div className="flex items-center justify-between border-b border-line px-4 py-3">
        <div>
          <h2 className="text-sm font-semibold text-white">Stage</h2>
          <p className="text-xs text-slate-400">{visibleTiles.length} active tile{visibleTiles.length === 1 ? "" : "s"}</p>
        </div>
      </div>
      {visibleTiles.length === 0 ? (
        <div className="grid place-items-center p-8 text-sm text-slate-500">No open streams</div>
      ) : (
        <div className={stageGridClass(visibleTiles.length)}>
          {visibleTiles.map((tile) => {
            const key = tileKey(tile);
            return (
              <Tile
                key={key}
                paused={paused.has(key)}
                tile={tile}
                onClose={() => {
                  setClosed((current) => new Set(current).add(key));
                  onCloseTile?.(tile);
                }}
                onPauseToggle={() => {
                  setPaused((current) => {
                    const next = new Set(current);
                    if (next.has(key)) next.delete(key);
                    else next.add(key);
                    return next;
                  });
                }}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}

function stageGridClass(count: number): string {
  const base = "grid gap-3 p-3";
  if (count === 1) return `${base} grid-cols-1`;
  if (count === 2) return `${base} md:grid-cols-2`;
  if (count <= 4) return `${base} md:grid-cols-2`;
  return `${base} grid-cols-[repeat(auto-fit,minmax(220px,1fr))]`;
}

function tileKey(tile: StageTileSpec): string {
  return `${tile.peerId}:${tile.sensorId}`;
}

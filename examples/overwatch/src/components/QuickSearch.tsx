import { useEffect, useMemo, useState } from "react";

import type { PeerSnapshot } from "../sdk/contract";
import { shortPeerId } from "../state/appState";

export function QuickSearch({
  snapshot,
  onSelectPeer,
}: {
  snapshot: PeerSnapshot | null;
  onSelectPeer: (peerId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const participants = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    return (snapshot?.participants ?? []).filter((participant) => {
      if (!normalized) return true;
      return `${participant.name ?? ""} ${participant.peer_id}`.toLowerCase().includes(normalized);
    });
  }, [query, snapshot]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setOpen(true);
      }
      if (event.key === "Escape") {
        setOpen(false);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 grid place-items-start bg-black/50 px-4 pt-[12vh]">
      <div className="mx-auto w-full max-w-xl rounded-control border border-line bg-panel shadow-2xl">
        <input
          autoFocus
          className="w-full border-b border-line bg-ink px-4 py-3 text-sm text-white outline-none"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search peers"
          value={query}
        />
        <div className="max-h-80 overflow-y-auto p-2">
          {participants.map((participant) => (
            <button
              key={participant.peer_id}
              className="flex w-full items-center justify-between rounded-control px-3 py-2 text-left text-sm text-slate-200 hover:bg-ink"
              onClick={() => {
                onSelectPeer(participant.peer_id);
                setOpen(false);
              }}
              type="button"
            >
              <span>{participant.name ?? "Browser peer"}</span>
              <span className="font-mono text-xs text-slate-500">{shortPeerId(participant.peer_id)}</span>
            </button>
          ))}
          {participants.length === 0 ? <p className="px-3 py-2 text-sm text-slate-500">No peers</p> : null}
        </div>
      </div>
    </div>
  );
}

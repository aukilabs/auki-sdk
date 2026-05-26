import type { PeerSnapshot } from "../sdk/contract";
import { deriveOperatorState, shortPeerId } from "../state/appState";

export function Directory({
  snapshot,
  selectedPeerId,
  onSelectPeer,
}: {
  snapshot: PeerSnapshot | null;
  selectedPeerId: string | null;
  onSelectPeer: (peerId: string) => void;
}) {
  const state = deriveOperatorState(snapshot);
  const participants = state.self ? [state.self, ...state.remotes] : state.remotes;

  return (
    <section className="rounded-control border border-line bg-panel">
      <div className="flex items-center justify-between border-b border-line px-4 py-3">
        <div>
          <h2 className="text-sm font-semibold text-white">Directory</h2>
          <p className="text-xs text-slate-400">{state.banner.text}</p>
        </div>
        <span className="rounded-control border border-line px-2 py-1 text-xs text-slate-300">
          {state.remotes.length} remote
        </span>
      </div>
      <div className="grid gap-2 p-3">
        {participants.map((participant) => (
          <button
            key={participant.peer_id}
            className={[
              "rounded-control border bg-ink/45 p-3 text-left transition",
              selectedPeerId === participant.peer_id ? "border-signal" : "border-line hover:border-slate-500",
            ].join(" ")}
            onClick={() => onSelectPeer(participant.peer_id)}
            type="button"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <h3 className="truncate text-sm font-semibold text-white">{participant.name ?? "Browser peer"}</h3>
                  {participant.is_self ? <span className="text-xs text-signal">you</span> : null}
                </div>
                <p className="truncate font-mono text-xs text-slate-400">{shortPeerId(participant.peer_id)}</p>
              </div>
              <span className="rounded-control border border-line px-2 py-1 text-xs text-slate-300">
                {participant.is_manager ? "Manager" : participant.connected ? "Live" : "Offline"}
              </span>
            </div>
            <div className="mt-3 grid gap-1">
              {(participant.sensors ?? []).map((sensor) => (
                <div key={sensor.sensor_id} className="flex items-center justify-between gap-2 rounded-control bg-panel px-2 py-1.5 text-xs">
                  <span className="truncate font-mono text-slate-200">{sensor.sensor_id}</span>
                  <span className="text-slate-500">{sensor.kind}</span>
                </div>
              ))}
              {(participant.sensors ?? []).length === 0 ? <p className="text-xs text-slate-500">No sensors</p> : null}
            </div>
          </button>
        ))}
        {participants.length === 0 ? (
          <div className="rounded-control border border-dashed border-line bg-ink/40 p-4 text-sm text-slate-500">
            No remote peers
          </div>
        ) : null}
      </div>
    </section>
  );
}

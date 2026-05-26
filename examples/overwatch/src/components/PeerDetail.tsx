import type { PeerSnapshot, SensorSummary } from "../sdk/contract";
import { shortPeerId } from "../state/appState";
import { SensorStrip } from "./SensorStrip";

export function PeerDetail({
  participant,
  onToggleSensor,
}: {
  participant: PeerSnapshot["participants"][number] | null;
  onToggleSensor: (participant: PeerSnapshot["participants"][number], sensor: SensorSummary, enabled: boolean) => void;
}) {
  if (!participant) {
    return (
      <section className="rounded-control border border-line bg-panel p-4 text-sm text-slate-500">
        Select or join a peer
      </section>
    );
  }

  const sensors = participant.sensors ?? [];
  return (
    <section className="grid rounded-control border border-line bg-panel">
      <div className="grid gap-4 p-4 md:grid-cols-[minmax(0,1fr)_220px]">
        <div className="min-w-0">
          <p className="text-xs uppercase text-slate-500">{participant.app ?? "overwatch"}</p>
          <h2 className="truncate text-lg font-semibold text-white">{participant.name ?? "Browser peer"}</h2>
          <p className="truncate font-mono text-xs text-slate-400">{participant.peer_id}</p>
        </div>
        <dl className="grid grid-cols-2 gap-2 text-xs md:grid-cols-1">
          <Fact label="Peer" value={shortPeerId(participant.peer_id)} />
          <Fact label="Role" value={participant.is_manager ? "Manager" : "Member"} />
          <Fact label="State" value={participant.connected === false ? "Offline" : "Connected"} />
          <Fact label="Sensors" value={String(sensors.length)} />
        </dl>
      </div>
      <SensorStrip
        peerId={participant.peer_id}
        sensors={sensors}
        onToggleSensor={(sensor, enabled) => onToggleSensor(participant, sensor, enabled)}
      />
    </section>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-control border border-line bg-ink/45 p-2">
      <dt className="text-slate-500">{label}</dt>
      <dd className="mt-1 truncate font-mono text-slate-200">{value}</dd>
    </div>
  );
}

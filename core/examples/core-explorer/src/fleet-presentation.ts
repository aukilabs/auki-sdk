import type { FleetMachine, FleetSnapshot } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import { parseCapability } from './fleet.ts';

export function fleetOverview(machines: FleetMachine[]) {
  return {
    robots: machines.filter(m => m.kind === 'robot').length,
    compute: machines.filter(m => m.kind === 'compute').length,
    online: machines.filter(m => m.presence === 'online').length,
    offline: machines.filter(m => m.presence === 'offline').length,
    presenceUnknown: machines.filter(m => m.presence === 'unknown').length,
    busy: machines.filter(m => m.work_state === 'busy').length,
    idle: machines.filter(m => m.work_state === 'idle').length,
    workUnknown: machines.filter(m => m.work_state === 'unknown').length,
  };
}

/** Only the exact demo protocol has a known human-readable operation. */
export function fleetCapability(value: string) {
  const parsed = parseCapability(value);
  if (parsed?.role === 'robot') return { name: 'Inspect file', description: 'Simulated inspection · byte count and SHA-256 report.' };
  if (parsed?.role === 'compute') return { name: 'Uppercase text', description: 'Transform a text record into its uppercase version.' };
  return { name: value, description: 'Custom capability reported by this machine.' };
}

export function fleetAssociation(machine: FleetMachine) {
  if (machine.association === 'assigned') return 'Assigned to this Domain';
  if (machine.association === 'active_task') return 'Observed task in this Domain';
  return 'Visible compute candidate';
}

/** Use the oldest read, since the snapshot is assembled from separate sources. */
export function fleetObservedAt(snapshots: FleetSnapshot[]) {
  const times = snapshots.map(s => Date.parse(s.observed_at)).filter(Number.isFinite);
  return times.length ? new Date(Math.min(...times)).toISOString() : undefined;
}

export function fleetTime(value: string | null | undefined) {
  const date = value ? new Date(value) : undefined;
  return date && Number.isFinite(date.getTime())
    ? date.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
    : 'Not reported';
}

export function fleetSourceNote(snapshots: FleetSnapshot[], failures: string[]) {
  const missing = new Set(snapshots.flatMap(s => s.sources.filter(source => source.state !== 'complete').map(source => source.source)));
  const labels = { robots: 'robot inventory', nodes: 'compute inventory', jobs: 'Domain activity', busy: 'work status' };
  const parts = [...missing].map(source => labels[source]);
  if (parts.length) return `Some ${parts.join(', ')} information is missing. Available observations are shown below.`;
  if (failures.length) return 'Some sources could not be read. Available observations are shown below.';
  if (snapshots.some(s => s.unresolved_activity.length)) return 'Some task activity could not be matched to a machine. Inspect source details for those observations.';
  return 'Some observations are incomplete. Inspect source details before drawing conclusions.';
}

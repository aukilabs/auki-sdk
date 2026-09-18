import type { FleetMachine, FleetSnapshot } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import type { FleetContext } from './fleet.ts';
import { safeError } from './safety.ts';
export type FleetMode = 'all' | 'dedicated' | 'public';
export function aggregateFleet(domain: string, snapshots: FleetSnapshot[], mode: FleetMode = 'all') {
  const machines = new Map<string, FleetMachine>();
  for (const snapshot of snapshots) {
    if (snapshot.domain_id !== domain) throw Error('Fleet response does not match this Domain.');
    const seen = new Set<string>();
    for (const machine of snapshot.machines) {
      if (seen.has(machine.id)) throw Error('Duplicate Fleet identity.');
      seen.add(machine.id);
      const previous = machines.get(machine.id);
      if (previous && (previous.kind !== machine.kind || previous.organization_id !== machine.organization_id)) throw Error('Conflicting Fleet identity.');
      if (!previous) machines.set(machine.id, { ...machine });
      else {
        // Domain observations retain authorized task attachments and association.
        const preferred = machine.association !== 'candidate' ? machine : previous;
        machines.set(machine.id, { ...preferred,
          mode: previous.mode === machine.mode ? machine.mode : 'unknown',
          presence: previous.presence === machine.presence ? machine.presence : 'unknown',
          work_state: previous.work_state === machine.work_state ? machine.work_state : 'unknown',
          capabilities: [...new Set([...previous.capabilities, ...machine.capabilities])],
          activity: [...new Map([...previous.activity, ...machine.activity].map(a => [`${a.job_id}/${a.task_id}`, a])).values()],
        });
      }
    }
  }
  return { machines: [...machines.values()].filter(m => mode === 'all' || m.mode === mode),
    complete: snapshots.length > 0 && snapshots.every(s => s.complete && s.sources.every(r => r.state === 'complete') && !s.unresolved_activity.length),
    sources: snapshots.flatMap(s => s.sources), unresolved: snapshots.flatMap(s => s.unresolved_activity) };
}
export class FleetScreenController {
  state: { loading: boolean; snapshots: FleetSnapshot[]; message: string; failures: string[] } = { loading: false, snapshots: [], message: 'Choose Refresh to read Fleet.', failures: [] };
  private generation = 0;
  private abort?: AbortController;
  private pending: Promise<void> = Promise.resolve();
  private blocked = false;
  private context: () => FleetContext | undefined;
  private changed: () => void;
  constructor(context: () => FleetContext | undefined, changed = () => {}) { this.context = context; this.changed = changed; }
  refresh(): Promise<void> {
    const context = this.context();
    if (!context || this.blocked) return Promise.resolve();
    const generation = ++this.generation; this.abort?.abort();
    const abort = this.abort = new AbortController();
    const current = () => { const now = this.context(); return generation === this.generation && now?.session === context.session && now?.domainId === context.domainId && now?.environment === context.environment; };
    this.state = { loading: true, snapshots: [], message: 'Reading Fleet…', failures: [] }; this.changed();
    const previous = this.pending;
    this.pending = previous.then(async () => {
      if (!current() || abort.signal.aborted || this.blocked) return;
      let client: ReturnType<FleetContext['createFleet']> | undefined;
      const timer = setTimeout(() => abort.abort(), 15_000);
      try {
        client = context.createFleet();
        const results = await Promise.allSettled([client.list({}, abort.signal), client.computePool({ mode: 'dedicated' }, abort.signal), client.computePool({ mode: 'public' }, abort.signal)]);
        if (!current()) return;
        if (abort.signal.aborted) throw Error('Fleet read timed out. Retry with Refresh.');
        const snapshots: FleetSnapshot[] = [], failures: string[] = [];
        results.forEach((result, index) => {
          const source = ['Domain inventory', 'Dedicated candidates', 'Public candidates'][index];
          if (result.status === 'fulfilled') {
            if (result.value.view !== (index ? 'compute_pool' : 'domain')) throw Error('Unexpected Fleet view.');
            snapshots.push(result.value);
          } else failures.push(`${source}: ${safeError(result.reason)}`);
        });
        const result = aggregateFleet(context.domainId, snapshots);
        this.state = { loading: false, snapshots, failures, message: !snapshots.length ? 'Fleet unavailable. Retry with Refresh.' : failures.length || !result.complete ? 'Partial observations · inspect Details. Refresh to retry.' : 'Sources complete · observed state.' };
      } catch (error) { if (current()) this.state = { loading: false, snapshots: [], failures: [safeError(error)], message: 'Fleet read failed. Retry with Refresh.' }; }
      finally {
        clearTimeout(timer);
        try { if (client) { await client.close(); client.free(); } }
        catch { this.blocked = true; this.state = { loading: false, snapshots: [], failures: [], message: 'Fleet cleanup failed. Reload before reconnecting.' }; }
        if (current()) this.changed();
      }
    });
    return this.pending;
  }
  close(clear = true): Promise<void> {
    ++this.generation; this.abort?.abort();
    this.state.loading = false;
    if (clear) this.state = { loading: false, snapshots: [], failures: [], message: 'Choose Refresh to read Fleet.' };
    else if (!this.state.snapshots.length) this.state.message = 'Read stopped. Choose Refresh to retry.';
    this.changed();
    return this.pending.then(() => { if (this.blocked) throw Error('Fleet cleanup failed.'); });
  }
}

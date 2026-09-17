import type { AukiFleet, FleetMachine, FleetSnapshot } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import type { DemoConfig, Role } from './jobs.ts';
import { safeError } from './safety.ts';
export type FleetPort = Pick<AukiFleet, 'list' | 'computePool' | 'close' | 'free'>;
export type FleetContext = { domainId: string; session: object; environment: string; createFleet: () => FleetPort };
export type Installation = { config: DemoConfig; compute: FleetMachine[]; robot: FleetMachine[]; issues: string[] };
export function parseCapability(value: string): { installationId: string; role: Role } | undefined {
  const match = /^\/examples\/compute-robot\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\/(compute|robot)\/v1$/.exec(value);
  return match ? { installationId: match[1], role: match[2] as Role } : undefined;
}
export function installations(domain: string, inventory?: FleetSnapshot, pool?: FleetSnapshot): Installation[] {
  const groups = new Map<string, Installation>(), identities = new Set<string>();
  const executors = new Map<string, { compute: FleetMachine[]; robot: FleetMachine[] }>();
  for (const [snapshot, view, role, association] of [[inventory, 'domain', 'robot', 'assigned'], [pool, 'compute_pool', 'compute', 'candidate']] as const) {
    if (!snapshot) continue;
    if (snapshot.domain_id !== domain || snapshot.view !== view) throw Error('Fleet response does not match this Domain or view.');
    const seen = new Set<string>();
    for (const machine of snapshot.machines) {
      if (seen.has(machine.id)) throw Error('Duplicate Fleet identity.');
      seen.add(machine.id);
      if (machine.kind !== role || machine.association !== association || machine.mode !== 'dedicated') continue;
      if (identities.has(machine.id)) throw Error('Duplicate Fleet identity across roles.');
      identities.add(machine.id);
      for (const capability of new Set(machine.capabilities)) {
        const parsed = parseCapability(capability);
        if (!parsed) continue;
        const group = groups.get(parsed.installationId) ?? { config: { installationId: parsed.installationId }, compute: [], robot: [], issues: [] };
        if (!group[role].includes(machine)) group[role].push(machine);
        groups.set(parsed.installationId, group);
        const candidates = executors.get(parsed.installationId) ?? { compute: [], robot: [] };
        candidates[parsed.role].push(machine); executors.set(parsed.installationId, candidates);
      }
    }
  }
  // DMS matches exact capabilities across both machine kinds. The suffix does
  // not restrict scheduling; keep provenance separate from executor eligibility.
  const complete = inventory?.sources.some(s => s.source === 'robots' && s.state === 'complete')
    && pool?.sources.some(s => s.source === 'nodes' && s.state === 'complete');
  for (const group of groups.values()) {
    for (const role of ['compute', 'robot'] as const) {
      const machines = executors.get(group.config.installationId)![role];
      if (machines.length > 1) group.issues.push(`${role}: ambiguous executors (${machines.length}); action unavailable.`);
      else if (machines.length && machines[0].kind !== role) group.issues.push(`${role}: capability advertised by another machine kind; action unavailable.`);
      else if (machines.length && !complete) group.issues.push(`${role}: inventory incomplete; action unavailable.`);
      else if (machines.length === 1) group.config[role === 'compute' ? 'computeId' : 'robotId'] = machines[0].id;
    }
  }
  return [...groups.values()].sort((a, b) => a.config.installationId.localeCompare(b.config.installationId));
}
export type FleetState = { loading: boolean; message: string; inventory?: FleetSnapshot; pool?: FleetSnapshot; installations: Installation[] };
export class FleetController {
  state: FleetState = { loading: false, message: 'Discover activated demo workers for this Domain.', installations: [] };
  private generation = 0;
  private abort?: AbortController;
  private client?: FleetPort;
  private pending?: Promise<void>;
  private closing?: Promise<void>;
  private getContext: () => FleetContext | undefined;
  private changed: () => void;
  constructor(getContext: () => FleetContext | undefined, changed: () => void = () => {}) { this.getContext = getContext; this.changed = changed; }
  refresh(): Promise<void> {
    if (this.pending) return this.pending;
    if (this.closing) return Promise.resolve();
    const context = this.getContext(); if (!context) return Promise.resolve();
    const generation = ++this.generation, abort = this.abort = new AbortController();
    this.state = { loading: true, message: 'Reading assigned robots and dedicated compute candidates…', installations: [] }; this.changed();
    const current = () => { const now = this.getContext(); return generation === this.generation && !abort.signal.aborted && now?.session === context.session && now?.domainId === context.domainId && now?.environment === context.environment; };
    const pending = Promise.resolve().then(async () => {
      try {
        if (!current()) return;
        const client = this.client ??= context.createFleet();
        const results = await Promise.allSettled([client.list({}, abort.signal), client.computePool({ mode: 'dedicated' }, abort.signal)]);
        if (!current()) return;
        // SDK hard errors (including wrong-Domain/duplicate identity) fail closed.
        const failure = results.find(r => r.status === 'rejected');
        if (failure?.status === 'rejected') throw failure.reason;
        const inventory = (results[0] as PromiseFulfilledResult<FleetSnapshot>).value, pool = (results[1] as PromiseFulfilledResult<FleetSnapshot>).value;
        const choices = installations(context.domainId, inventory, pool);
        this.state = { loading: false, inventory, pool, installations: choices, message: `${inventory.complete && pool.complete ? 'Sources complete.' : 'Partial observations; inspect source diagnostics.'} ${choices.length ? 'DMS decides availability and scheduling.' : inventory.complete && pool.complete ? 'No demo installations found.' : 'No demo installations visible in the available sources.'}` };
      } catch (error) { if (current()) this.state = { loading: false, installations: [], message: `Fleet discovery failed. ${safeError(error)}` }; }
    }).finally(() => { if (this.pending === pending) this.pending = undefined; if (current()) this.changed(); });
    this.pending = pending; return pending;
  }
  close(): Promise<void> {
    if (this.closing) return this.closing;
    ++this.generation; this.abort?.abort();
    const client = this.client; this.client = undefined;
    this.state = { loading: false, message: 'Discover activated demo workers for this Domain.', installations: [] };
    this.changed();
    const closing = Promise.resolve().then(async () => {
      const results = await Promise.allSettled([this.pending, Promise.resolve().then(() => client?.close())]);
      if (results.some(r => r.status === 'rejected')) { this.state.message = 'Fleet cleanup failed. Reload before reconnecting.'; this.changed(); throw Error('Fleet cleanup failed.'); }
      client?.free(); this.closing = undefined;
    });
    this.closing = closing; return closing;
  }
}

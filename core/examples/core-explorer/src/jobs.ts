import type { AukiDmsJobs, AukiDomainData, JobSpec, JobEstimate, JobDetails, JobListItem } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import { safeError, uuid } from './safety.ts';

export type Role = 'compute' | 'robot';
export type DemoConfig = { installationId: string; computeId?: string; robotId?: string };
export type JobsPort = Pick<AukiDmsJobs, 'estimate' | 'submit' | 'get' | 'cancel' | 'list' | 'close'>;
export type DataPort = Pick<AukiDomainData, 'get' | 'readTo'>;
export type JobsContext = { domainId: string; environment: string; session: object; createJobs: () => JobsPort; data: DataPort };
/** Display-only history enrichment; this never authorizes outputs or reconciles a submission. */
export type JobHistoryItem = JobListItem & { role?: Role };
export type JobsState = {
  phase: 'setup' | 'choose' | 'review' | 'submitting' | 'detail' | 'uncertain' | 'history';
  message: string; config?: DemoConfig; discoveryRequired?: boolean; role?: Role; inputId?: string;
  estimate?: JobEstimate; spec?: JobSpec; details?: JobDetails; items?: JobHistoryItem[];
  nextCursor?: string; jobId?: string; expectedWorkerId?: string; executorMatch?: boolean;
  /** Only these references are eligible for current-Domain data navigation. */
  outputs?: string[]; domainId?: string; environment?: string; errorCode?: string;
  reconciliation?: { label: string; domainId: string; environment: string };
};
export const MAX_INPUT_BYTES = 65536;
export const HISTORY_WARNING = 'History may be incomplete: provider pagination can skip jobs (#396). An empty page is not proof that a submission failed.';
export const capability = (installationId: string, role: Role): string => `/examples/compute-robot/${installationId}/${role}/v1`;

const initial = (): JobsState => ({ phase: 'setup', message: 'Configure activated demo workers for the selected Domain. Configured IDs do not indicate availability.' });
const uncertainMessage = 'Submission uncertain. A job may exist and consume credits. Reconcile the saved label in its original Domain; do not blindly resubmit.';
const codes = new Set(['authentication_required', 'configuration', 'authorization_denied', 'persistence', 'transient', 'cancelled', 'closed', 'invalid_input', 'invalid_response', 'http_status', 'transport', 'timed_out', 'too_large', 'submission_uncertain']);
const sameId = (a: string | null | undefined, b: string): boolean => typeof a === 'string' && a.toLowerCase() === b.toLowerCase();
class ValidationError extends Error {}
type Operation = { generation: number; abort: AbortController; context: JobsContext; kind: string };

/** Injectable SDK controller. The host owns the shared session and data client. */
export class JobsController {
  state: JobsState = initial();
  private context?: JobsContext;
  private client?: JobsPort;
  private generation = 0;
  private active?: { operation: Operation; promise: Promise<void> };
  private pending = new Set<Promise<void>>();
  private aborts = new Set<AbortController>();
  private closing?: Promise<void>;
  private reviewed?: JobSpec;
  private recovery?: { session: object; config: DemoConfig; spec: JobSpec; reconciliation: NonNullable<JobsState['reconciliation']> };
  private getContext: () => JobsContext | undefined;
  private changed: () => void;

  constructor(getContext: () => JobsContext | undefined, changed: () => void = () => {}) {
    this.getContext = getContext; this.changed = changed;
  }
  get busy(): boolean { return !!this.active || !!this.closing; }
  private publish(state: JobsState): void { this.state = state; this.changed(); }
  private matches(context: JobsContext): boolean {
    const now = this.getContext();
    return !!now && now.session === context.session && now.data === context.data && sameId(now.domainId, context.domainId) && now.environment === context.environment;
  }
  private current(op: Operation): boolean { return op.generation === this.generation && !op.abort.signal.aborted && this.matches(op.context); }
  private invalidate(): void {
    ++this.generation; this.reviewed = undefined;
    for (const abort of this.aborts) abort.abort();
    this.active = undefined;
  }
  private base(phase: JobsState['phase'], message: string): JobsState {
    return { phase, message, config: this.state.config, domainId: this.context?.domainId,
      environment: this.context?.environment, discoveryRequired: this.state.discoveryRequired, reconciliation: this.state.reconciliation };
  }
  private ready(): boolean {
    if (this.closing) return false;
    if (!this.context || !this.state.config) { this.publish({ ...initial(), reconciliation: this.state.reconciliation }); return false; }
    if (!this.matches(this.context)) { void this.close(this.getContext()?.session).catch(() => {}); return false; }
    return true;
  }
  private jobs(): JobsPort { return this.client ??= this.context!.createJobs(); }
  private writePending(): boolean { return this.active?.operation.kind === 'submit' || this.active?.operation.kind === 'cancel'; }

  async configure(config: DemoConfig): Promise<void> {
    if (this.closing || this.writePending()) return;
    const context = this.getContext();
    if (this.recovery && this.recovery.session !== context?.session) this.recovery = undefined;
    if (this.context && !this.matches(this.context)) { await this.close(context?.session); }
    if (this.state.reconciliation && this.state.config) return;
    this.invalidate();
    try {
      if (!context || !this.matches(context)) throw new ValidationError('Select a connected Domain first.');
      const normalized = Object.freeze({ installationId: uuid(config.installationId), computeId: config.computeId ? uuid(config.computeId) : undefined, robotId: config.robotId ? uuid(config.robotId) : undefined });
      this.context = { ...context, domainId: uuid(context.domainId) };
      this.publish({ ...this.base('choose', 'Choose a record and action. Worker availability is checked by DMS when estimating.'), config: normalized, discoveryRequired: false });
    } catch {
      this.publish({ ...initial(), reconciliation: this.state.reconciliation, message: 'Select a connected Domain and discover an unambiguous demo installation.' });
    }
  }

  /** Retain historical executor IDs/recovery, but revoke permission to prepare. */
  invalidateDiscovery(): void {
    if (this.closing || this.writePending()) return;
    this.invalidate();
    this.publish({ ...this.state, discoveryRequired: true, estimate: undefined, spec: undefined,
      phase: this.state.phase === 'review' ? 'choose' : this.state.phase });
  }

  choose(): void {
    if (!this.ready() || this.writePending()) return;
    this.invalidate();
    this.publish(this.base(this.state.reconciliation ? 'uncertain' : 'choose',
      this.state.reconciliation ? uncertainMessage : 'Choose a record and action.'));
  }

  prepare(role: Role, inputId: string): Promise<void> {
    if (!this.ready() || this.writePending()) return Promise.resolve();
    this.invalidate();
    if (this.recovery) {
      if (!this.state.reconciliation) { this.publish({ ...this.base('choose', 'An unresolved submission remains in another Domain. Return there to reconcile before submitting another job.') }); return Promise.resolve(); }
      this.publish({ ...this.base('uncertain', uncertainMessage) }); return Promise.resolve();
    }
    this.publish({ ...this.base('choose', 'Checking input metadata and bounded contents…'), role });
    return this.run('prepare', async op => {
      if (this.state.discoveryRequired) throw new ValidationError('Discover an unambiguous executor again before preparing a new job.');
      if (role !== 'compute' && role !== 'robot') throw new ValidationError('Choose compute or robot.');
      if (!this.state.config?.[role === 'compute' ? 'computeId' : 'robotId']) throw new ValidationError('This role is missing or ambiguous. Discover an unambiguous executor first.');
      let id: string;
      try { id = uuid(inputId); } catch { throw new ValidationError('Enter a complete input record UUID.'); }
      const metadata = await op.context.data.get(id, op.abort.signal);
      if (!this.current(op)) return;
      if (!sameId(metadata.id, id) || !sameId(metadata.domain_id, op.context.domainId) || !Number.isSafeInteger(metadata.size) || metadata.size < 0 || metadata.size > MAX_INPUT_BYTES) {
        throw new ValidationError('Input metadata must match this Domain and contain at most 65,536 bytes.');
      }
      const bytes = new Uint8Array(metadata.size);
      let length = 0;
      const received = await op.context.data.readTo(id, (chunk: Uint8Array) => {
        if (!this.current(op)) throw new ValidationError('Input read cancelled.');
        if (length + chunk.length > bytes.length) throw new ValidationError('Input exceeds its reviewed size.');
        bytes.set(chunk, length); length += chunk.length;
      }, { maxBytes: MAX_INPUT_BYTES, maxChunkBytes: MAX_INPUT_BYTES }, op.abort.signal);
      if (!this.current(op)) return;
      if (length !== metadata.size || received !== length) throw new ValidationError('Input size changed while reading. Review again.');
      if (role === 'compute') {
        try { new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
        catch { throw new ValidationError('Compute input must be valid UTF-8.'); }
      }
      const config = this.state.config!;
      const spec: JobSpec = { label: `core-explorer-${crypto.randomUUID()}`, meta: { installation_id: config.installationId }, edges: [], tasks: [{
        label: role, stage: role, capability: capability(config.installationId, role), mode: 'dedicated', maxAttempts: 1,
        inputsCids: [id], meta: { input_id: id, run_id: config.installationId },
      }] };
      const estimate = await this.jobs().estimate(spec, op.abort.signal);
      if (!this.current(op)) return;
      // Keep an independent private intent; host rendering cannot mutate what is submitted.
      this.reviewed = structuredClone(spec);
      this.publish({ ...this.base('review', 'Review this exact input, Domain and decimal credit estimate before confirming once.'), role, inputId: id, spec, estimate,
        expectedWorkerId: role === 'compute' ? config.computeId : config.robotId });
    });
  }

  submit(): Promise<void> {
    if (this.active) return this.active.promise;
    if (!this.ready() || this.state.phase !== 'review' || !this.reviewed || this.state.reconciliation) return Promise.resolve();
    const spec = this.reviewed; this.reviewed = undefined;
    this.publish({ ...this.state, phase: 'submitting', message: 'Submitting once… Cancellation of HTTP does not undo a server job.',
      reconciliation: { label: spec.label, domainId: this.context!.domainId, environment: this.context!.environment } });
    this.recovery = { session: this.context!.session, config: this.state.config!, spec: structuredClone(spec), reconciliation: this.state.reconciliation! };
    return this.run('submit', async op => {
      const jobId = await this.jobs().submit(spec, op.abort.signal);
      if (!this.current(op)) return;
      let id: string;
      try { id = uuid(jobId); } catch { throw { code: 'submission_uncertain' }; }
      this.recovery = undefined;
      this.publish({ ...this.state, phase: 'detail', jobId: id, reconciliation: undefined, message: 'Job accepted. Reading task state…' });
      await this.getDetails(id, op);
    });
  }

  refresh(): Promise<void> {
    if (this.active) return this.active.promise;
    if (!this.ready() || !this.state.jobId) return Promise.resolve();
    const id = this.state.jobId;
    return this.run('refresh', op => this.getDetails(id, op));
  }

  cancel(): Promise<void> {
    if (this.writePending()) return this.active!.promise;
    if (!this.ready() || !this.state.jobId) return Promise.resolve();
    const id = this.state.jobId;
    this.invalidate();
    this.publish({ ...this.state, message: 'Requesting whole-job cancellation. Tasks may still be running; credits may remain locked.' });
    return this.run('cancel', async op => {
      await this.jobs().cancel(id, op.abort.signal);
      if (!this.current(op)) return;
      await this.getDetails(id, op);
      if (this.current(op)) this.publish({ ...this.state, message: 'Cancellation requested. Inspect task states; this does not prove execution stopped or credits were released.' });
    });
  }

  list(cursor?: string): Promise<void> {
    if (!this.ready() || this.writePending()) return Promise.resolve();
    this.invalidate();
    this.publish(this.base('history', HISTORY_WARNING));
    return this.run('list', async op => {
      const installation = this.state.config!.installationId;
      const page = await this.jobs().list({ limit: 25, cursor, capabilities: [capability(installation, 'compute'), capability(installation, 'robot')], matchAllCapabilities: false }, op.abort.signal);
      if (!this.current(op)) return;
      if (page.items.length > 25 || page.items.some(item => !sameId(item.job.domain_id, op.context.domainId))) throw new ValidationError('Job history does not match this Domain or page limit.');
      const items = await this.historyItems(page.items, installation, op);
      if (this.current(op)) this.publish({ ...this.state, items, nextCursor: page.next_cursor ?? undefined });
    });
  }

  private async historyItems(page: JobListItem[], installation: string, op: Operation): Promise<JobHistoryItem[]> {
    const items: JobHistoryItem[] = page.map(({ job, tasks_summary }) => ({ job, tasks_summary }));
    // The list contract omits task capabilities. Read this page's actions with at
    // most four concurrent detail reads and a five-second total enrichment budget.
    const abort = new AbortController(), cancel = () => abort.abort();
    op.abort.signal.addEventListener('abort', cancel, { once: true });
    const timer = setTimeout(cancel, 5000);
    let next = 0;
    try {
      await Promise.all(Array.from({ length: Math.min(4, items.length) }, async () => {
        while (this.current(op) && !abort.signal.aborted && next < items.length) {
          const item = items[next++];
          try {
            const details = await this.jobs().get(uuid(item.job.id), abort.signal);
            if (!this.current(op) || abort.signal.aborted) return;
            const task = details.tasks.length === 1 ? details.tasks[0] : undefined;
            if (sameId(details.job.id, item.job.id) && sameId(details.job.domain_id, op.context.domainId) && task && sameId(task.job_id, item.job.id)) {
              item.role = (['compute', 'robot'] as const).find(role => task.capability === capability(installation, role));
            }
          } catch { /* A missing or denied detail must not hide the history row. */ }
        }
      }));
      return items;
    } finally {
      clearTimeout(timer); op.abort.signal.removeEventListener('abort', cancel);
    }
  }

  inspect(jobId: string): Promise<void> {
    if (!this.ready() || this.writePending()) return Promise.resolve();
    this.invalidate();
    this.publish(this.base('detail', 'Reading job details…'));
    return this.run('inspect', async op => {
      let id: string;
      try { id = uuid(jobId); } catch { throw new ValidationError('Enter a complete job UUID.'); }
      this.state = { ...this.state, jobId: id };
      await this.getDetails(id, op);
    });
  }

  private async getDetails(id: string, op: Operation): Promise<void> {
    const details = await this.jobs().get(id, op.abort.signal);
    if (!this.current(op)) return;
    if (!sameId(details.job.id, id) || !sameId(details.job.domain_id, op.context.domainId)) throw new ValidationError('Job response does not match this Domain.');
    const config = this.state.config!;
    const task = details.tasks.length === 1 ? details.tasks[0] : undefined;
    const role = task && (['compute', 'robot'] as const).find(role => task.capability === capability(config.installationId, role));
    const expectedWorkerId = role === 'compute' ? config.computeId : role === 'robot' ? config.robotId : undefined;
    const receipts = task ? details.receipts.filter(r => sameId(r.task_id, task.id)) : [];
    const input = task?.meta.input_id;
    const intent = this.recovery?.spec ?? this.state.spec;
    const identityMatch = !!task && sameId(details.job.meta.installation_id as string, config.installationId)
      && sameId(task.meta.run_id as string, config.installationId)
      && typeof input === 'string' && (() => { try { return uuid(input) === input.toLowerCase(); } catch { return false; } })()
      && task.inputs_cids.length === 1 && sameId(task.inputs_cids[0], input)
      && (!intent || (task.capability === intent.tasks[0].capability && sameId(input, intent.tasks[0].meta!.input_id as string)));
    const executorMatch = identityMatch && !!task && !!expectedWorkerId && sameId(task.job_id, id) && task.mode === 'dedicated'
      && (task.reserved_by === null || sameId(task.reserved_by, expectedWorkerId))
      && (receipts.length > 0 ? receipts.every(r => sameId(r.job_id, id) && sameId(r.node_id, expectedWorkerId)) : sameId(task.reserved_by, expectedWorkerId));
    const outputs: string[] = [];
    if (executorMatch && details.job.status === 'completed' && task?.status === 'completed' && task.attempts === 1 && task.max_attempts === 1 && receipts.length === 1) {
      for (const receipt of receipts) {
        // Failed/cancellation receipts cannot authorize navigation to their artifacts.
        if (receipt.meta.status !== undefined && receipt.meta.status !== 'completed') continue;
        if (receipt.meta.source !== undefined || !sameId(receipt.meta.run_id as string, config.installationId)
          || receipt.outputs.length !== 1 || !sameId(receipt.meta.data_id as string, receipt.outputs[0])
          || typeof receipt.meta.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(receipt.meta.sha256)
          || !Number.isSafeInteger(receipt.meta.bytes) || (receipt.meta.bytes as number) < 0 || (receipt.meta.bytes as number) > MAX_INPUT_BYTES) continue;
        try { outputs.push(uuid(receipt.outputs[0])); } catch { /* Non-data references cannot be opened. */ }
      }
    }
    const reconciliation = this.state.reconciliation;
    const reconciled = !!reconciliation && details.job.label === reconciliation.label && identityMatch;
    if (reconciled) this.recovery = undefined;
    this.publish({ ...this.state, phase: 'detail', details, role, expectedWorkerId, executorMatch, outputs: [...new Set(outputs)], errorCode: undefined,
      reconciliation: reconciled ? undefined : reconciliation,
      message: details.job.status === 'canceled' ? 'Job canceled. Tasks may still run and credits may remain locked; inspect task and credit fields.'
        : executorMatch && task?.status === 'completed' && !outputs.length ? 'Receipt metadata is missing or inconsistent. Outputs are withheld.'
        : executorMatch ? 'Observed executor matches the configured worker.' : 'Executor is missing or differs from the configured worker. Outputs are withheld.' });
  }

  private run(kind: string, work: (op: Operation) => Promise<void>): Promise<void> {
    const op: Operation = { generation: this.generation, abort: new AbortController(), context: this.context!, kind };
    this.aborts.add(op.abort);
    const pending = Promise.resolve().then(async () => {
      if (!this.current(op)) return;
      try { await work(op); }
      catch (error) {
        if (!this.current(op)) return;
        const e = error as { code?: string; kind?: string; status?: number };
        const code = e?.code && codes.has(e.code) ? e.code : undefined;
        const denied = e?.status === 401 || e?.status === 403 || code === 'authorization_denied';
        const ambiguous = kind === 'submit' && !this.state.jobId && (code === 'submission_uncertain' || e?.kind === 'submission_uncertain' || (!denied && !['invalid_input', 'authentication_required', 'configuration', 'persistence'].includes(code ?? '') && !(e?.status && e.status >= 400 && e.status < 500)));
        this.reviewed = undefined;
        if (kind === 'submit' && !ambiguous) this.recovery = undefined;
        this.publish({ ...this.state, phase: ambiguous ? 'uncertain' : kind === 'prepare' || (kind === 'submit' && !this.state.jobId) ? 'choose' : this.state.phase,
          estimate: kind === 'prepare' ? undefined : this.state.estimate,
          reconciliation: kind === 'submit' && !ambiguous ? undefined : this.state.reconciliation,
          errorCode: code,
          message: ambiguous ? uncertainMessage : error instanceof ValidationError ? error.message
            : kind === 'cancel' ? 'Cancellation outcome is uncertain. Refresh the job before explicitly trying again; tasks and credits may remain active.'
            : denied ? 'Jobs access denied. All jobs endpoints require Domain write permission.'
            : code === 'persistence' ? 'Session credential persistence failed. Retain this session and retry persistence before continuing.'
            : safeError(error) });
      }
    }).finally(() => {
      this.aborts.delete(op.abort); this.pending.delete(pending);
      if (this.active?.promise === pending) this.active = undefined;
      if (op.generation === this.generation && !this.matches(op.context)) void this.close(this.getContext()?.session).catch(() => {});
      else this.changed();
    });
    this.active = { operation: op, promise: pending }; this.pending.add(pending);
    return pending;
  }

  /** Restore only the unresolved intent for this exact session, Domain and environment. */
  restoreContext(): void {
    const context = this.getContext(), recovery = this.recovery;
    if (context && recovery && context.session !== recovery.session) this.recovery = undefined;
    if (this.closing || !context || !recovery || recovery.session !== context.session
      || !sameId(context.domainId, recovery.reconciliation.domainId) || context.environment !== recovery.reconciliation.environment) return;
    this.context = context;
    this.publish({ phase: 'uncertain', message: uncertainMessage, config: recovery.config, discoveryRequired: true,
      domainId: context.domainId, environment: context.environment, reconciliation: recovery.reconciliation });
  }

  /** Abort and drain local work. Omit retainSession on logout to erase all recovery. */
  close(retainSession?: object): Promise<void> {
    if (!retainSession || this.recovery?.session !== retainSession) this.recovery = undefined;
    if (this.closing) return this.closing;
    this.invalidate();
    const client = this.client; this.client = undefined; this.context = undefined;
    this.publish(initial());
    const pending = [...this.pending];
    const closing = Promise.resolve().then(async () => {
      const results = await Promise.allSettled([...pending, Promise.resolve().then(() => client?.close())]);
      if (results.some(result => result.status === 'rejected')) {
        this.publish({ ...initial(), message: 'Jobs cleanup failed. Local state has been cleared.' });
        throw new Error('Jobs cleanup failed.');
      }
    }).finally(() => { this.closing = undefined; this.restoreContext(); this.changed(); });
    this.closing = closing;
    return closing;
  }
}

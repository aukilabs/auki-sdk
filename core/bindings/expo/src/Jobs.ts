import module from "./AukiSdkExpoModule";
import type {
  JobCancellation,
  JobDetails,
  JobEstimate,
  JobListQuery,
  JobPage,
  JobSpec,
} from "./AukiSdkExpo.types";
import { aukiError, normalizeAukiError, type AukiError } from "./errors";
import { mapJobCancellation, mapJobDetails, mapJobEstimate, mapJobPage } from "./wire";

function newId(): string {
  return `jobs_operation_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;
}

function normalizeError(value: unknown): AukiError {
  if (value instanceof TypeError) return aukiError("input", "Invalid DMS jobs input", { cause: value });
  return normalizeAukiError(value, "DMS jobs operation failed");
}

function cancelledError(): AukiError {
  return aukiError("cancelled", "DMS jobs operation was cancelled");
}

function wireSpec(spec: JobSpec): object {
  return {
    label: spec.label,
    ...(spec.priority === undefined ? {} : { priority: spec.priority }),
    ...(spec.meta === undefined ? {} : { meta: spec.meta }),
    tasks: spec.tasks.map(task => ({
      label: task.label,
      stage: task.stage,
      capability: task.capability,
      ...(task.mode === undefined ? {} : { mode: task.mode }),
      ...(task.capabilityFilters === undefined ? {} : { capability_filters: task.capabilityFilters }),
      ...(task.priority === undefined ? {} : { priority: task.priority }),
      ...(task.inputsCids === undefined ? {} : { inputs_cids: task.inputsCids }),
      ...(task.outputsPrefix === undefined ? {} : { outputs_prefix: task.outputsPrefix }),
      ...(task.meta === undefined ? {} : { meta: task.meta }),
      ...(task.maxAttempts === undefined ? {} : { max_attempts: task.maxAttempts }),
    })),
    ...(spec.edges === undefined ? {} : { edges: spec.edges }),
  };
}

function wireQuery(query: JobListQuery): object {
  return {
    ...(query.limit === undefined ? {} : { limit: query.limit }),
    ...(query.cursor === undefined ? {} : { cursor: query.cursor }),
    ...(query.status === undefined ? {} : { status: query.status }),
    ...(query.capabilities === undefined ? {} : { capabilities: query.capabilities }),
    ...(query.matchAllCapabilities === undefined ? {} : {
      match_all_capabilities: query.matchAllCapabilities,
    }),
  };
}

/** A Domain-scoped DMS jobs client backed by an existing User session. */
export class AukiJobs {
  private closing: Promise<void> | null = null;
  private closed = false;

  /** @internal Use jobs(sessionId, domainId). */
  constructor(private readonly clientId: string) {}

  private operation<T>(
    signal: AbortSignal | undefined,
    invoke: (operationId: string) => Promise<T>,
  ): Promise<T> {
    if (this.closed || this.closing) return Promise.reject(aukiError("closed", "DMS jobs client is closed"));
    if (signal?.aborted) return Promise.reject(cancelledError());
    const operationId = newId();
    const cancel = () => { void module.jobsOperationCancel(operationId).catch(() => undefined); };
    signal?.addEventListener("abort", cancel, { once: true });
    return Promise.resolve().then(() => invoke(operationId))
      .catch(error => { throw normalizeError(error); })
      .finally(() => signal?.removeEventListener("abort", cancel));
  }

  estimate(spec: JobSpec, signal?: AbortSignal): Promise<JobEstimate> {
    return this.operation(signal, operationId =>
      module.jobsEstimate(this.clientId, JSON.stringify(wireSpec(spec)), operationId)
        .then(value => mapJobEstimate(JSON.parse(value))));
  }

  submit(spec: JobSpec, signal?: AbortSignal): Promise<string> {
    return this.operation(signal, operationId =>
      module.jobsSubmit(this.clientId, JSON.stringify(wireSpec(spec)), operationId));
  }

  /** Reuse this persisted key and spec on a verified idempotency-capable DMS. */
  submitWithKey(spec: JobSpec, idempotencyKey: string, signal?: AbortSignal): Promise<string> {
    return this.operation(signal, operationId =>
      module.jobsSubmitWithKey(this.clientId, JSON.stringify(wireSpec(spec)), idempotencyKey, operationId));
  }

  list(query: JobListQuery = {}, signal?: AbortSignal): Promise<JobPage> {
    return this.operation(signal, operationId =>
      module.jobsList(this.clientId, JSON.stringify(wireQuery(query)), operationId)
        .then(value => mapJobPage(JSON.parse(value))));
  }

  get(jobId: string, signal?: AbortSignal): Promise<JobDetails> {
    return this.operation(signal, operationId =>
      module.jobsGet(this.clientId, jobId, operationId)
        .then(value => mapJobDetails(JSON.parse(value))));
  }

  cancel(jobId: string, signal?: AbortSignal): Promise<JobCancellation> {
    return this.operation(signal, operationId =>
      module.jobsCancel(this.clientId, jobId, operationId)
        .then(value => mapJobCancellation(JSON.parse(value))));
  }

  close(): Promise<void> {
    if (this.closed) return Promise.resolve();
    if (this.closing) return this.closing;
    this.closing = module.jobsClose(this.clientId)
      .catch(error => { throw normalizeError(error); })
      .finally(() => { this.closed = true; this.closing = null; });
    return this.closing;
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}

/** Create a jobs client without starting a peer. */
export async function jobs(sessionId: string, domainId: string): Promise<AukiJobs> {
  try {
    return new AukiJobs(await module.jobsOpen(sessionId, domainId));
  } catch (error) {
    throw normalizeError(error);
  }
}

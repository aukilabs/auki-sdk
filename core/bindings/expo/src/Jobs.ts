import module from "./AukiSdkExpoModule";
import type {
  JobCancellation,
  JobDetails,
  JobEstimate,
  JobListQuery,
  JobPage,
  JobSpec,
  JobsError,
  JobsFailureKind,
} from "./AukiSdkExpo.types";

const failureKinds = new Set<JobsFailureKind>([
  "auth", "invalid_input", "invalid_response", "http_status", "transport",
  "timed_out", "cancelled", "closed", "too_large", "submission_uncertain", "submission_in_progress",
]);
const webFailureKinds: Record<string, JobsFailureKind> = {
  input: "invalid_input",
  response: "invalid_response",
  http: "http_status",
  timeout: "timed_out",
  limit: "too_large",
};

function newId(): string {
  return `jobs_operation_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;
}

function jobsError(kind: JobsFailureKind, message: string, cause?: unknown): JobsError {
  const error = new Error(message) as JobsError;
  Object.defineProperty(error, "kind", { value: kind, enumerable: true });
  if (cause !== undefined) Object.defineProperty(error, "cause", { value: cause });
  return error;
}

function normalizeError(value: unknown): JobsError {
  if (value instanceof TypeError) return jobsError("invalid_input", "Invalid DMS jobs input", value);
  if (value instanceof Error) {
    const candidate = value as Error & { code?: unknown; kind?: unknown };
    if (typeof candidate.code === "string" && candidate.code.startsWith("jobs:")) {
      const [, kind, status, authCode, source, retryAfter] = candidate.code.split(":");
      if (failureKinds.has(kind as JobsFailureKind)) {
        const converted = jobsError(kind as JobsFailureKind, candidate.message, candidate);
        if (status && Number.isInteger(Number(status))) {
          Object.defineProperty(converted, "status", { value: Number(status), enumerable: true });
        }
        if (authCode) Object.defineProperty(converted, "code", { value: authCode, enumerable: true });
        if (source) Object.defineProperty(converted, "source", { value: source, enumerable: true });
        if (retryAfter && /^\d+$/.test(retryAfter) && Number(retryAfter) <= 0xffffffff) {
          Object.defineProperty(converted, "retryAfterSeconds", { value: Number(retryAfter), enumerable: true });
        }
        return converted;
      }
    }
    if (typeof candidate.code === "string" && failureKinds.has(candidate.code as JobsFailureKind)) {
      Object.defineProperty(candidate, "kind", { value: candidate.code, enumerable: true });
      return candidate as JobsError;
    }
    if (typeof candidate.kind === "string") {
      const mapped = webFailureKinds[candidate.kind] ?? candidate.kind as JobsFailureKind;
      if (failureKinds.has(mapped)) {
        if (mapped !== candidate.kind) {
          Object.defineProperty(candidate, "kind", { value: mapped, enumerable: true });
        }
        return candidate as JobsError;
      }
    }
  }
  return jobsError("transport", "DMS jobs operation failed", value);
}

function cancelledError(): JobsError {
  return jobsError("cancelled", "DMS jobs operation was cancelled");
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
    if (this.closed || this.closing) return Promise.reject(jobsError("closed", "DMS jobs client is closed"));
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
        .then(value => JSON.parse(value) as JobEstimate));
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
        .then(value => JSON.parse(value) as JobPage));
  }

  get(jobId: string, signal?: AbortSignal): Promise<JobDetails> {
    return this.operation(signal, operationId =>
      module.jobsGet(this.clientId, jobId, operationId)
        .then(value => JSON.parse(value) as JobDetails));
  }

  cancel(jobId: string, signal?: AbortSignal): Promise<JobCancellation> {
    return this.operation(signal, operationId =>
      module.jobsCancel(this.clientId, jobId, operationId)
        .then(value => JSON.parse(value) as JobCancellation));
  }

  close(): Promise<void> {
    if (this.closed) return Promise.resolve();
    if (this.closing) return this.closing;
    this.closing = module.jobsClose(this.clientId)
      .catch(error => { throw normalizeError(error); })
      .finally(() => { this.closed = true; this.closing = null; });
    return this.closing;
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

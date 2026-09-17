import module from "./AukiSdkExpoModule";
import type { ComputePoolQuery, FleetError, FleetFailureKind, FleetQuery, FleetSnapshot } from "./AukiSdkExpo.types";

const kinds = new Set<FleetFailureKind>([
  "auth", "input", "response", "http", "transport", "timeout", "cancelled", "closed", "limit",
]);

function failure(kind: FleetFailureKind, code: string, message: string, cause?: unknown): FleetError {
  return Object.assign(new Error(message), { kind, code, ...(cause === undefined ? {} : { cause }) });
}

function normalizeError(value: unknown): FleetError {
  if (value instanceof TypeError) return failure("input", "invalid_input", "Invalid fleet query", value);
  if (value instanceof Error) {
    const candidate = value as Error & { kind?: unknown; code?: unknown };
    if (typeof candidate.code === "string" && candidate.code.startsWith("fleet:")) {
      const [, kind, status, code] = candidate.code.split(":");
      if (kinds.has(kind as FleetFailureKind)) {
        return Object.assign(failure(kind as FleetFailureKind, code, candidate.message, candidate),
          status && Number.isInteger(Number(status)) ? { status: Number(status) } : {});
      }
    }
    if (typeof candidate.kind === "string" && kinds.has(candidate.kind as FleetFailureKind)) {
      return candidate as FleetError;
    }
    if (candidate.code === "closed") return failure("closed", "closed", candidate.message, candidate);
  }
  return failure("transport", "transport", "Fleet operation failed", value);
}

function wireQuery(query: FleetQuery): object {
  return {
    ...(query.capabilities === undefined ? {} : { capabilities: query.capabilities }),
    ...(query.matchAllCapabilities === undefined ? {} : { match_all_capabilities: query.matchAllCapabilities }),
  };
}

/** Read-only, Domain-scoped inventory. Opening a client does not start a peer. */
export class AukiFleet {
  private closing: Promise<void> | null = null;
  private closed = false;

  /** @internal Use fleet(sessionId, domainId). */
  constructor(private readonly clientId: string) {}

  private operation(
    signal: AbortSignal | undefined,
    invoke: (operationId: string) => Promise<string>,
  ): Promise<FleetSnapshot> {
    if (this.closed || this.closing) return Promise.reject(failure("closed", "closed", "Fleet client is closed"));
    if (signal?.aborted) return Promise.reject(failure("cancelled", "cancelled", "Fleet request cancelled"));
    const id = `fleet_operation_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;
    const cancel = () => { void module.fleetOperationCancel(id).catch(() => undefined); };
    signal?.addEventListener("abort", cancel, { once: true });
    return Promise.resolve().then(() => invoke(id))
      .then(value => JSON.parse(value) as FleetSnapshot)
      .catch(error => { throw normalizeError(error); })
      .finally(() => signal?.removeEventListener("abort", cancel));
  }

  list(query: FleetQuery = {}, signal?: AbortSignal): Promise<FleetSnapshot> {
    return this.operation(signal, id => module.fleetList(this.clientId, JSON.stringify(wireQuery(query)), id));
  }

  computePool(query: ComputePoolQuery, signal?: AbortSignal): Promise<FleetSnapshot> {
    return this.operation(signal, id => module.fleetComputePool(this.clientId,
      JSON.stringify({ ...wireQuery(query), mode: query.mode }), id));
  }

  close(): Promise<void> {
    if (this.closed) return Promise.resolve();
    if (this.closing) return this.closing;
    this.closing = module.fleetClose(this.clientId)
      .catch(error => { throw normalizeError(error); })
      .finally(() => { this.closed = true; this.closing = null; });
    return this.closing;
  }
}

export async function fleet(sessionId: string, domainId: string): Promise<AukiFleet> {
  try { return new AukiFleet(await module.fleetOpen(sessionId, domainId)); }
  catch (error) { throw normalizeError(error); }
}

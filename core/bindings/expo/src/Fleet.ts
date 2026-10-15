import module from "./AukiSdkExpoModule";
import type { ComputePoolQuery, FleetQuery, FleetSnapshot } from "./AukiSdkExpo.types";
import { aukiError, normalizeAukiError, type AukiError } from "./errors";
import { mapFleetSnapshot } from "./wire";

function normalizeError(value: unknown): AukiError {
  if (value instanceof TypeError) return aukiError("input", "Invalid fleet query", { cause: value });
  return normalizeAukiError(value, "Fleet operation failed");
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
    if (this.closed || this.closing) return Promise.reject(aukiError("closed", "Fleet client is closed"));
    if (signal?.aborted) return Promise.reject(aukiError("cancelled", "Fleet request cancelled"));
    const id = `fleet_operation_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;
    const cancel = () => { void module.fleetOperationCancel(id).catch(() => undefined); };
    signal?.addEventListener("abort", cancel, { once: true });
    return Promise.resolve().then(() => invoke(id))
      .then(value => mapFleetSnapshot(JSON.parse(value)))
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

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}

export async function fleet(sessionId: string, domainId: string): Promise<AukiFleet> {
  try { return new AukiFleet(await module.fleetOpen(sessionId, domainId)); }
  catch (error) { throw normalizeError(error); }
}

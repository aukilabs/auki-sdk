import module from "./AukiSdkExpoModule";
import type {
  DataMetadata,
  DataQuery,
  DataSink,
  DataSource,
  DataTarget,
  DomainDataError,
  DomainDataFailureKind,
  DomainPage,
  PortalPage,
  PortalDomainPage,
  DomainQuery,
  Portal,
  PortalDomain,
  PortalPose,
  TransferOptions,
} from "./AukiSdkExpo.types";

// Keep each native bridge message comfortably below common React Native bridge
// limits. The SDK may assemble several chunks into one server multipart part.
const BRIDGE_CHUNK_BYTES = 256 * 1024;
const CALLBACK_TIMEOUT_MS = 30_000;
const failureKinds = new Set<DomainDataFailureKind>([
  "cancelled", "closed", "timeout", "auth", "http", "input", "response",
  "limit", "callback", "cleanup", "transport",
]);

function newId(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;
}

function dataError(kind: DomainDataFailureKind, message: string, cause?: unknown): DomainDataError {
  const error = new Error(message) as DomainDataError;
  Object.defineProperty(error, "kind", { value: kind, enumerable: true });
  if (cause !== undefined) Object.defineProperty(error, "cause", { value: cause });
  return error;
}

function normalizedError(value: unknown): DomainDataError {
  if (value instanceof Error) {
    const candidate = value as Error & { kind?: unknown; code?: unknown; status?: unknown };
    let rawKind = typeof candidate.kind === "string" ? candidate.kind : candidate.code;
    if (typeof rawKind === "string" && rawKind.startsWith("domain_data:")) {
      const [, encodedKind, encodedStatus, authCode] = rawKind.split(":");
      if (failureKinds.has(encodedKind as DomainDataFailureKind)) {
        const converted = dataError(encodedKind as DomainDataFailureKind, candidate.message, candidate);
        if (encodedStatus && Number.isInteger(Number(encodedStatus))) {
          Object.defineProperty(converted, "status", {
            value: Number(encodedStatus),
            enumerable: true,
          });
        }
        if (authCode) Object.defineProperty(converted, "code", { value: authCode, enumerable: true });
        return converted;
      }
      rawKind = encodedKind;
    }
    if (typeof rawKind === "string" && failureKinds.has(rawKind as DomainDataFailureKind)) {
      if (candidate.kind !== rawKind) {
        Object.defineProperty(candidate, "kind", { value: rawKind, enumerable: true });
      }
      return candidate as DomainDataError;
    }
  }
  return dataError("transport", "Domain data operation failed", value);
}

function cancelledError(): DomainDataError {
  return dataError("cancelled", "Domain data operation was cancelled");
}

function combinedCleanupError(
  message: string,
  operation: DomainDataError,
  cleanup: DomainDataError,
): DomainDataError {
  const error = dataError("cleanup", message, { operation, cleanup });
  const status = operation.status ?? cleanup.status;
  const code = operation.code ?? cleanup.code;
  if (status !== undefined) Object.defineProperty(error, "status", { value: status, enumerable: true });
  if (code !== undefined) Object.defineProperty(error, "code", { value: code, enumerable: true });
  return error;
}

class CallbackWaitError extends Error {
  constructor(readonly kind: "cancelled" | "timeout") {
    super(kind);
  }
}

async function awaitCallback<T>(
  invoke: () => T | Promise<T>,
  callback: { signal: AbortSignal; cancel: () => void },
): Promise<T> {
  if (callback.signal.aborted) throw new CallbackWaitError("cancelled");
  let timeout: ReturnType<typeof setTimeout> | undefined;
  let abort: (() => void) | undefined;
  const invoked = Promise.resolve().then(invoke);
  // A callback may settle after cancellation. Observe it without retaining the
  // transfer or creating an unhandled rejection.
  void invoked.catch(() => undefined);
  try {
    return await Promise.race([
      invoked,
      new Promise<never>((_resolve, reject) => {
        abort = () => reject(new CallbackWaitError("cancelled"));
        callback.signal.addEventListener("abort", abort, { once: true });
        if (callback.signal.aborted) abort();
      }),
      new Promise<never>((_resolve, reject) => {
        timeout = setTimeout(() => {
          reject(new CallbackWaitError("timeout"));
          callback.cancel();
        }, CALLBACK_TIMEOUT_MS);
      }),
    ]);
  } finally {
    if (timeout !== undefined) clearTimeout(timeout);
    if (abort) callback.signal.removeEventListener("abort", abort);
  }
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = "";
  const step = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += step) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + step));
  }
  return btoa(binary);
}

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

async function cancellable<T>(
  signal: AbortSignal | undefined,
  invoke: (operationId: string) => Promise<T>,
): Promise<T> {
  if (signal?.aborted) throw cancelledError();
  const operationId = newId("data_operation");
  const cancel = () => { void module.dataOperationCancel(operationId).catch(() => undefined); };
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    return await invoke(operationId);
  } catch (error) {
    throw normalizedError(error);
  } finally {
    signal?.removeEventListener("abort", cancel);
  }
}

/** Domain discovery and portal metadata backed by an existing login session. */
export class AukiDomains {
  /** @internal Use domains(sessionId). */
  constructor(private readonly sessionId: string) {}

  forPortalPage(portal: string, limit = 50, cursor?: string | null, organization = "own", signal?: AbortSignal): Promise<PortalDomainPage> {
    return cancellable(signal, operationId => module.domainsForPortalPage(this.sessionId, portal, organization, limit, cursor ?? null, operationId));
  }

  portalsPage(domainId: string, limit = 50, cursor?: string | null, signal?: AbortSignal): Promise<PortalPage> {
    return cancellable(signal, operationId => module.domainsPortalsPage(this.sessionId, domainId, limit, cursor ?? null, operationId));
  }

  list(query: DomainQuery = {}, signal?: AbortSignal): Promise<DomainPage> {
    return cancellable(signal, operationId =>
      module.domainsList(this.sessionId, JSON.stringify(query), operationId));
  }

  forPortal(
    portal: string,
    organization?: string,
    signal?: AbortSignal,
  ): Promise<PortalDomain[]> {
    return cancellable(signal, operationId =>
      module.domainsForPortal(this.sessionId, portal, organization ?? null, operationId));
  }

  portals(domainId: string, signal?: AbortSignal): Promise<Portal[]> {
    return cancellable(signal, operationId =>
      module.domainsPortals(this.sessionId, domainId, operationId));
  }

  portal(domainId: string, portal: string, signal?: AbortSignal): Promise<Portal> {
    return cancellable(signal, operationId =>
      module.domainsPortal(this.sessionId, domainId, portal, operationId));
  }
}

/** Select Domain discovery without starting a peer. */
export function domains(sessionId: string): AukiDomains {
  return new AukiDomains(sessionId);
}

/** HTTP data client backed by the same login session as peers and Domain discovery. */
export class AukiDomainData {
  private closing: Promise<void> | null = null;
  private closed = false;
  private readonly transfers = new Set<AbortController>();

  /** @internal Use data(sessionId, domainId). */
  constructor(private readonly clientId: string) {}

  private requireOpen(): void {
    if (this.closed || this.closing) throw dataError("closed", "Domain data client is closed");
  }

  private async operation<T>(
    signal: AbortSignal | undefined,
    invoke: (operationId: string) => Promise<T>,
  ): Promise<T> {
    this.requireOpen();
    return await cancellable(signal, invoke);
  }

  list(query: DataQuery = {}, signal?: AbortSignal): Promise<DataMetadata[]> {
    return this.operation(signal, operationId =>
      module.domainDataList(this.clientId, JSON.stringify(query), operationId));
  }

  get(dataId: string, signal?: AbortSignal): Promise<DataMetadata> {
    return this.operation(signal, operationId =>
      module.domainDataGet(this.clientId, dataId, operationId));
  }

  async read(dataId: string, signal?: AbortSignal): Promise<Uint8Array> {
    const encoded = await this.operation(signal, operationId =>
      module.domainDataRead(this.clientId, dataId, operationId));
    return decodeBase64(encoded);
  }

  write(
    target: DataTarget,
    bytes: Uint8Array,
    signal?: AbortSignal,
  ): Promise<DataMetadata> {
    return this.operation(signal, operationId => module.domainDataWrite(
      this.clientId,
      JSON.stringify(target),
      encodeBase64(bytes),
      operationId,
    ));
  }

  delete(dataId: string, signal?: AbortSignal): Promise<void> {
    return this.operation(signal, operationId =>
      module.domainDataDelete(this.clientId, dataId, operationId));
  }

  poses(signal?: AbortSignal): Promise<PortalPose[]> {
    return this.operation(signal, operationId =>
      module.domainDataPoses(this.clientId, operationId));
  }

  pose(portal: string, signal?: AbortSignal): Promise<PortalPose> {
    return this.operation(signal, operationId =>
      module.domainDataPose(this.clientId, portal, operationId));
  }

  async readTo(
    dataId: string,
    sink: DataSink,
    options: TransferOptions = {},
    signal?: AbortSignal,
  ): Promise<number> {
    this.requireOpen();
    if (signal?.aborted) throw cancelledError();
    const callback = this.transferSignal(signal);
    const operationId = newId("data_operation");
    let downloadId: string | null = null;
    let failure: DomainDataError | null = null;
    let cancelIssued = false;
    const cancel = () => {
      if (cancelIssued) return;
      cancelIssued = true;
      if (downloadId) void module.dataDownloadCancel(downloadId).catch(() => undefined);
      else void module.dataOperationCancel(operationId).catch(() => undefined);
    };
    callback.signal.addEventListener("abort", cancel, { once: true });
    try {
      downloadId = await module.dataDownloadStart(
        this.clientId,
        dataId,
        JSON.stringify(bridgeOptions(options)),
        operationId,
      );
      let total = 0;
      while (true) {
        const encoded = await module.dataDownloadNext(downloadId);
        if (encoded === null) return total;
        const chunk = decodeBase64(encoded);
        try {
          await awaitCallback(() => sink(chunk, callback.signal), callback);
        } catch (error) {
          cancel();
          if (error instanceof CallbackWaitError) {
            throw error.kind === "cancelled"
              ? cancelledError()
              : dataError("timeout", "Domain data destination timed out");
          }
          throw dataError("callback", "Domain data destination failed", error);
        }
        total += chunk.length;
      }
    } catch (error) {
      failure = normalizedError(error);
      throw failure;
    } finally {
      callback.signal.removeEventListener("abort", cancel);
      callback.dispose();
      if (downloadId) {
        try {
          await module.dataDownloadClose(downloadId);
        } catch (error) {
          const cleanup = normalizedError(error);
          if (failure) {
            // Transfer close awaits the same retained completion and can replay
            // its primary error. Only the core's explicit cleanup kind means
            // abort/drain itself failed.
            if (cleanup.kind === "cleanup") {
              throw combinedCleanupError(
                "Domain data download cleanup failed",
                failure,
                cleanup,
              );
            }
          } else {
            throw cleanup;
          }
        }
      }
    }
  }

  /** Named multipart uploads may replace an existing name. */
  async writeStream(
    target: DataTarget,
    size: number,
    source: DataSource,
    options: TransferOptions = {},
    signal?: AbortSignal,
  ): Promise<DataMetadata> {
    this.requireOpen();
    if (signal?.aborted) throw cancelledError();
    const callback = this.transferSignal(signal);
    const operationId = newId("data_operation");
    let uploadId: string | null = null;
    let failure: DomainDataError | null = null;
    let cancelIssued = false;
    const cancel = () => {
      if (cancelIssued) return;
      cancelIssued = true;
      if (uploadId) void module.dataUploadCancel(uploadId).catch(() => undefined);
      else void module.dataOperationCancel(operationId).catch(() => undefined);
    };
    callback.signal.addEventListener("abort", cancel, { once: true });
    try {
      uploadId = await module.dataUploadStart(
        this.clientId,
        JSON.stringify(target),
        size,
        JSON.stringify(options),
        operationId,
      );
      while (true) {
        const requested = await module.dataUploadNextMaximum(uploadId);
        if (requested === null) break;
        let bytes: Uint8Array;
        try {
          bytes = await awaitCallback(
            () => source(Math.min(requested, BRIDGE_CHUNK_BYTES), callback.signal),
            callback,
          );
        } catch (error) {
          cancel();
          if (error instanceof CallbackWaitError) {
            throw error.kind === "cancelled"
              ? cancelledError()
              : dataError("timeout", "Domain data source timed out");
          }
          throw dataError("callback", "Domain data source failed", error);
        }
        if (!(bytes instanceof Uint8Array)) {
          cancel();
          throw dataError("callback", "Domain data source must return Uint8Array");
        }
        if (bytes.length > requested || bytes.length > BRIDGE_CHUNK_BYTES) {
          cancel();
          throw dataError("input", "Domain data source exceeded the requested chunk size");
        }
        await module.dataUploadPush(uploadId, encodeBase64(bytes));
      }
      return await module.dataUploadResult(uploadId);
    } catch (error) {
      failure = normalizedError(error);
      throw failure;
    } finally {
      callback.signal.removeEventListener("abort", cancel);
      callback.dispose();
      if (uploadId) {
        try {
          await module.dataUploadClose(uploadId);
        } catch (error) {
          const cleanup = normalizedError(error);
          if (failure) {
            if (cleanup.kind === "cleanup") {
              throw combinedCleanupError(
                "Domain data upload cleanup failed",
                failure,
                cleanup,
              );
            }
          } else {
            throw cleanup;
          }
        }
      }
    }
  }

  close(): Promise<void> {
    if (this.closing) return this.closing;
    if (this.closed) return Promise.resolve();
    for (const transfer of this.transfers) transfer.abort();
    this.closing = module.domainDataClose(this.clientId)
      .catch(error => { throw normalizedError(error); })
      .finally(() => {
        this.closed = true;
        this.closing = null;
      });
    return this.closing;
  }

  private transferSignal(external?: AbortSignal): {
    signal: AbortSignal;
    cancel: () => void;
    dispose: () => void;
  } {
    const controller = new AbortController();
    const cancel = () => controller.abort();
    external?.addEventListener("abort", cancel, { once: true });
    this.transfers.add(controller);
    return {
      signal: controller.signal,
      cancel: () => controller.abort(),
      dispose: () => {
        external?.removeEventListener("abort", cancel);
        controller.abort();
        this.transfers.delete(controller);
      },
    };
  }
}

function bridgeOptions(options: TransferOptions): TransferOptions {
  return {
    ...options,
    maxChunkBytes: Math.min(options.maxChunkBytes ?? BRIDGE_CHUNK_BYTES, BRIDGE_CHUNK_BYTES),
  };
}

/** Select a Domain data client without starting a peer. Await close() when done. */
export async function data(sessionId: string, domainId: string): Promise<AukiDomainData> {
  try {
    return new AukiDomainData(await module.domainDataOpen(sessionId, domainId));
  } catch (error) {
    throw normalizedError(error);
  }
}

import module from "./AukiSdkExpoModule";
import type { AukiServiceEnvironment, ZitadelSessionCredentials } from "./AukiSdkExpo.types";

/** Opaque, redacted storage snapshot. Only explicit methods extract secrets. */
export class ZitadelCredentialsSnapshot {
  #credentials: ZitadelSessionCredentials;
  /** @internal Created by the session's storage adapter. */
  constructor(credentials: ZitadelSessionCredentials) { this.#credentials = credentials; }
  exposeAccessToken(): string { return this.#credentials.accessToken; }
  exposeRefreshToken(): string { return this.#credentials.refreshToken; }
  get clientId(): string { return this.#credentials.clientId; }
  get issuer(): string { return this.#credentials.issuer; }
  get accessTokenExpiresAt(): string | null { return this.#credentials.accessTokenExpiresAt ?? null; }
  toJSON(): string { return "[redacted Zitadel credentials]"; }
  toString(): string { return this.toJSON(); }
}

/** Await atomic durable persistence. Settle ALL writes on success or failure. */
export type ZitadelSessionStore = (credentials: ZitadelCredentialsSnapshot) => Promise<void>;

const stores = new Map<string, ZitadelSessionStore>();
const processing = new Set<string>();
let subscribed = false;

function subscribe(): void {
  if (subscribed) return;
  // One listener for this JS module's lifetime; session callbacks are removed
  // only after completed close. No tokens travel in an event.
  module.addListener("onZitadelSaveRequested", request => {
    const key = `${request.sessionId}/${request.requestId}`;
    if (processing.has(key)) return;
    processing.add(key);
    void (async () => {
      let success = false;
      try {
        const store = stores.get(request.sessionId);
        if (!store) throw new Error("session storage owner missing");
        const json = await module._zitadelCredentials(request.sessionId, request.requestId);
        const result = store(new ZitadelCredentialsSnapshot(JSON.parse(json)));
        if (!result || typeof result.then !== "function") throw new Error("storage must return a Promise");
        await result;
        success = true;
      } catch {
        // Host/platform error text is deliberately never sent across the bridge.
      }
      // This ACK (not event delivery) releases the core's awaited save.
      await module._ackZitadelSave(request.sessionId, request.requestId, success);
    })().catch(() => {
      // A lost bridge ACK must fail closed: keep the native save pending. Do not
      // pretend a completed close or start a competing rotation. No secret logs.
    }).finally(() => processing.delete(key));
  });
  subscribed = true;
}

/** Import and retain a session ID before any auth request or refresh can start.
 * Web may await Wasm loading. Host login and secure storage remain host-owned.
 */
export async function importZitadelSession(
  credentials: ZitadelSessionCredentials,
  store: ZitadelSessionStore,
  environment?: AukiServiceEnvironment,
): Promise<string> {
  if (typeof module._importZitadel !== "function") throw new Error("Zitadel sessions are supported on Web and iOS only");
  if (typeof store !== "function") throw Object.assign(new Error("Storage callback required"), { code: "configuration" });
  subscribe();
  const id = await module._importZitadel(JSON.stringify(credentials), environment ? JSON.stringify(environment) : null);
  stores.set(id, store);
  return id;
}

/** Stop owned peers separately. Await this drain BEFORE clearing host storage. */
export async function closeSession(sessionId: string): Promise<void> {
  await module._closeSession(sessionId);
  stores.delete(sessionId);
}

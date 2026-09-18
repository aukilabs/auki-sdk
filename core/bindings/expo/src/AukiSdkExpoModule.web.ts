import { NativeModule, registerWebModule } from "expo";

import type {
  AukiDiscoveryCandidateInfo,
  AukiDiscoveryModeName,
  AukiDomainInfo,
  AukiExactTarget,
  AukiSdkExpoModuleEvents,
  AukiServiceEnvironment,
  DataMetadata,
  DomainPage,
  DomainDiscoveryPage,
  DomainDiscoveryQuery,
  PortalPage,
  PortalDomainPage,
  ComputePoolQuery,
  FleetQuery,
  JobListQuery,
  JobSpec,
  Portal,
  PortalDomain,
  PortalPose,
  ZitadelSessionCredentials,
} from "./AukiSdkExpo.types";
import { jsonStringify } from "./json-stringify";
import { loadAukiSdkWasm, type AukiSdkWasm } from "./web/loadAukiSdkWasm";

type Session = Awaited<ReturnType<AukiSdkWasm["AukiUserSession"]["loginDev"]>>;
type Peer = Awaited<ReturnType<Session["startPeer"]>>;
type StreamClient = import("./web/generated/auki_sdk_web.js").AukiStreamClient;
type CatalogClient = import("./web/generated/auki_sdk_web.js").AukiCatalogClient;
type MessageClient = import("./web/generated/auki_sdk_web.js").AukiMessageClient;
type StreamSub = Awaited<ReturnType<StreamClient["subscribeExact"]>>;
type MessageSender = Awaited<ReturnType<MessageClient["openExact"]>>;
type DomainDataClient = import("./web/generated/auki_sdk_web.js").AukiDomainData;
type FleetClient = ReturnType<Session["fleet"]>;
type JobsClient = ReturnType<Session["jobs"]>;
type WebDownload = {
  controller: AbortController;
  task: Promise<number>;
  pending: { encoded: string; acknowledge: () => void; reject: (error: unknown) => void } | null;
  delivered: boolean;
  changed: (() => void) | null;
  observedError?: unknown;
};
type WebUpload = {
  controller: AbortController;
  task: Promise<DataMetadata>;
  pending: { maximum: number; supply: (bytes: Uint8Array) => void; reject: (error: unknown) => void } | null;
  changed: (() => void) | null;
  observedError?: unknown;
};

function newId(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function base64ToBytes(value: string): Uint8Array {
  if (!value) {
    return new Uint8Array();
  }
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function mapCandidate(candidate: {
  peerId: string;
  routes: string[];
  servedProtocols: string[];
  expiresAt: string;
  source: string;
  subjectId?: string | null;
  peerType?: string | null;
}): AukiDiscoveryCandidateInfo {
  return {
    peerId: candidate.peerId,
    routes: [...candidate.routes],
    servedProtocols: [...candidate.servedProtocols],
    expiresAt: candidate.expiresAt,
    source: candidate.source,
    subjectId: candidate.subjectId ?? null,
    peerType: candidate.peerType ?? null,
  };
}

function webJobSpec(value: Record<string, unknown>): JobSpec {
  const tasks = value.tasks as Array<Record<string, unknown>>;
  return {
    label: value.label as string,
    ...(value.priority === undefined ? {} : { priority: value.priority as number }),
    ...(value.meta === undefined ? {} : { meta: value.meta as JobSpec["meta"] }),
    tasks: tasks.map(task => ({
      label: task.label as string,
      stage: task.stage as string,
      capability: task.capability as string,
      ...(task.mode === undefined ? {} : { mode: task.mode as "public" | "dedicated" }),
      ...(task.capability_filters === undefined ? {} : {
        capabilityFilters: task.capability_filters as Record<string, string>,
      }),
      ...(task.priority === undefined ? {} : { priority: task.priority as number }),
      ...(task.inputs_cids === undefined ? {} : { inputsCids: task.inputs_cids as string[] }),
      ...(task.outputs_prefix === undefined ? {} : { outputsPrefix: task.outputs_prefix as string }),
      ...(task.meta === undefined ? {} : { meta: task.meta as JobSpec["meta"] }),
      ...(task.max_attempts === undefined ? {} : { maxAttempts: task.max_attempts as number }),
    })),
    ...(value.edges === undefined ? {} : { edges: value.edges as JobSpec["edges"] }),
  };
}

function webJobQuery(value: Record<string, unknown>): JobListQuery {
  return {
    ...(value.limit === undefined ? {} : { limit: value.limit as number }),
    ...(value.cursor === undefined ? {} : { cursor: value.cursor as string }),
    ...(value.status === undefined ? {} : { status: value.status as JobListQuery["status"] }),
    ...(value.capabilities === undefined ? {} : { capabilities: value.capabilities as string[] }),
    ...(value.match_all_capabilities === undefined ? {} : {
      matchAllCapabilities: value.match_all_capabilities as boolean,
    }),
  };
}

class AukiSdkExpoModule extends NativeModule<AukiSdkExpoModuleEvents> {
  private wasm: AukiSdkWasm | null = null;
  private sessions = new Map<string, Session>();
  private peers = new Map<string, Peer>();
  private streams = new Map<string, StreamSub>();
  private messages = new Map<string, { peerHandle: string; sender: MessageSender }>();
  private dataClients = new Map<string, { sessionId: string; client: DomainDataClient }>();
  private dataOperations = new Map<string, AbortController>();
  private fleetClients = new Map<string, { sessionId: string; client: FleetClient }>();
  private fleetOperations = new Map<string, AbortController>();
  private fleetCancelledBeforeStart = new Set<string>();
  private jobsClients = new Map<string, { sessionId: string; client: JobsClient }>();
  private jobsOperations = new Map<string, AbortController>();
  private jobsCancelledBeforeStart = new Set<string>();
  private downloads = new Map<string, WebDownload>();
  private uploads = new Map<string, WebUpload>();
  private saves = new Map<string, {
    requestId: string;
    snapshot: import("./web/generated/auki_sdk_web.js").ZitadelCredentialsSnapshot;
    resolve: () => void;
    reject: () => void;
  }>();

  async _importZitadel(credentialsJson: string, environmentJson: string | null): Promise<string> {
    const sdk = await this.sdk();
    const id = newId("session");
    try {
      const credentials = JSON.parse(credentialsJson) as ZitadelSessionCredentials;
      const environment = environmentJson ? JSON.parse(environmentJson) as AukiServiceEnvironment : null;
      const save = (snapshot: import("./web/generated/auki_sdk_web.js").ZitadelCredentialsSnapshot) => new Promise<void>((resolve, reject) => {
        const requestId = newId("save");
        this.saves.set(id, { requestId, snapshot, resolve, reject: () => reject(new Error("persistence")) });
        this.emit("onZitadelSaveRequested", { sessionId: id, requestId });
      });
      const session = environment
        ? sdk.AukiUserSession.importZitadelWithEnvironment(environment.apiBaseUrl, environment.ddsBaseUrl, environment.dmsBaseUrl, credentials, save)
        : sdk.AukiUserSession.importZitadelDev(credentials, save);
      this.sessions.set(id, session);
      return id;
    } catch {
      throw Object.assign(new Error("Invalid authentication configuration"), { code: "configuration" });
    }
  }

  async _zitadelCredentials(sessionId: string, requestId: string): Promise<string> {
    const pending = this.saves.get(sessionId);
    if (!pending || pending.requestId !== requestId) throw new Error("No matching storage request");
    const c = pending.snapshot;
    return JSON.stringify({ accessToken: c.exposeAccessToken(), refreshToken: c.exposeRefreshToken(),
      clientId: c.clientId, issuer: c.issuer, accessTokenExpiresAt: c.accessTokenExpiresAt ?? null });
  }

  async _ackZitadelSave(sessionId: string, requestId: string, success: boolean): Promise<boolean> {
    const pending = this.saves.get(sessionId);
    if (!pending || pending.requestId !== requestId) return false;
    this.saves.delete(sessionId);
    pending.snapshot.free();
    if (success) pending.resolve(); else pending.reject();
    return true;
  }

  async _closeSession(sessionId: string): Promise<void> {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    await session.close();
    if (this.sessions.get(sessionId) === session) {
      this.sessions.delete(sessionId);
      session.free();
    }
  }

  private async sdk(): Promise<AukiSdkWasm> {
    if (!this.wasm) {
      this.wasm = await loadAukiSdkWasm();
    }
    return this.wasm;
  }

  private session(sessionId: string): Session {
    const session = this.sessions.get(sessionId);
    if (!session) {
      throw Object.assign(new Error("The session is closed"), { code: "closed" });
    }
    return session;
  }

  private peer(peerHandle: string): Peer {
    const peer = this.peers.get(peerHandle);
    if (!peer) {
      throw new Error(`unknown peer: ${peerHandle}`);
    }
    return peer;
  }

  async loginDev(email: string, password: string, clientId?: string | null): Promise<string> {
    const sdk = await this.sdk();
    const session = await sdk.AukiUserSession.loginDev(email, password, clientId);
    const id = newId("session");
    this.sessions.set(id, session);
    return id;
  }

  async loginWithEnvironment(
    apiBaseUrl: string,
    ddsBaseUrl: string,
    dmsBaseUrl: string,
    email: string,
    password: string,
    clientId?: string | null,
  ): Promise<string> {
    const sdk = await this.sdk();
    const session = await sdk.AukiUserSession.loginWithEnvironment(
      apiBaseUrl,
      ddsBaseUrl,
      dmsBaseUrl,
      email,
      password,
      clientId,
    );
    const id = newId("session");
    this.sessions.set(id, session);
    return id;
  }

  async accessibleDomains(sessionId: string): Promise<AukiDomainInfo[]> {
    const domains = await this.session(sessionId).accessibleDomains();
    return domains.map((domain) => ({
      id: domain.id,
      name: domain.name ?? null,
      description: domain.description ?? null,
      organizationId: domain.organizationId ?? null,
    }));
  }

  async domainsDiscover(sessionId: string, queryJson: string, operationId: string): Promise<DomainDiscoveryPage> {
    return this.withDataOperation(operationId, async signal => {
      const domains = this.session(sessionId).domains();
      try { return await domains.discover(JSON.parse(queryJson) as DomainDiscoveryQuery, signal); }
      finally { domains.free(); }
    });
  }
  async domainsForPortalPage(sessionId: string, portal: string, organization: string, limit: number, cursor: string | null, operationId: string): Promise<PortalDomainPage> {
    return this.withDataOperation(operationId, async signal => {
      const domains = this.session(sessionId).domains();
      try { return await domains.forPortalPage(portal, organization, limit, cursor, signal); }
      finally { domains.free(); }
    });
  }
  async domainsPortalsPage(sessionId: string, domain: string, limit: number, cursor: string | null, operationId: string): Promise<PortalPage> {
    return this.withDataOperation(operationId, async signal => {
      const domains = this.session(sessionId).domains();
      try { return await domains.portalsPage(domain, limit, cursor, signal); }
      finally { domains.free(); }
    });
  }

  async domainsList(
    sessionId: string,
    queryJson: string,
    operationId: string,
  ): Promise<DomainPage> {
    return this.withDataOperation(operationId, async signal => {
      const domains = this.session(sessionId).domains();
      try {
        return await domains.list(JSON.parse(queryJson), signal);
      } finally {
        domains.free();
      }
    });
  }

  async domainsForPortal(
    sessionId: string,
    portal: string,
    organization: string | null,
    operationId: string,
  ): Promise<PortalDomain[]> {
    return this.withDataOperation(operationId, async signal => {
      const domains = this.session(sessionId).domains();
      try {
        return await domains.forPortal(portal, organization, signal);
      } finally {
        domains.free();
      }
    });
  }

  async domainsPortals(
    sessionId: string,
    domainId: string,
    operationId: string,
  ): Promise<Portal[]> {
    return this.withDataOperation(operationId, async signal => {
      const domains = this.session(sessionId).domains();
      try {
        return await domains.portals(domainId, signal);
      } finally {
        domains.free();
      }
    });
  }

  async domainsPortal(
    sessionId: string,
    domainId: string,
    portal: string,
    operationId: string,
  ): Promise<Portal> {
    return this.withDataOperation(operationId, async signal => {
      const domains = this.session(sessionId).domains();
      try {
        return await domains.portal(domainId, portal, signal);
      } finally {
        domains.free();
      }
    });
  }

  async domainDataOpen(sessionId: string, domainId: string): Promise<string> {
    const client = this.session(sessionId).data(domainId);
    const clientId = newId("data");
    this.dataClients.set(clientId, { sessionId, client });
    return clientId;
  }

  async domainDataList(
    clientId: string,
    queryJson: string,
    operationId: string,
  ): Promise<DataMetadata[]> {
    return this.withDataOperation(operationId, signal =>
      this.dataClient(clientId).list(JSON.parse(queryJson), signal));
  }

  async domainDataGet(
    clientId: string,
    dataId: string,
    operationId: string,
  ): Promise<DataMetadata> {
    return this.withDataOperation(operationId, signal =>
      this.dataClient(clientId).get(dataId, signal));
  }

  async domainDataRead(clientId: string, dataId: string, operationId: string): Promise<string> {
    const bytes = await this.withDataOperation(operationId, signal =>
      this.dataClient(clientId).read(dataId, signal));
    return bytesToBase64(bytes);
  }

  async domainDataWrite(
    clientId: string,
    targetJson: string,
    bytesBase64: string,
    operationId: string,
  ): Promise<DataMetadata> {
    return this.withDataOperation(operationId, signal => this.dataClient(clientId).write(
      JSON.parse(targetJson),
      base64ToBytes(bytesBase64),
      signal,
    ));
  }

  async domainDataDelete(clientId: string, dataId: string, operationId: string): Promise<void> {
    await this.withDataOperation(operationId, signal =>
      this.dataClient(clientId).delete(dataId, signal));
  }

  async domainDataPoses(clientId: string, operationId: string): Promise<PortalPose[]> {
    return this.withDataOperation(operationId, signal => this.dataClient(clientId).poses(signal));
  }

  async domainDataPose(
    clientId: string,
    portal: string,
    operationId: string,
  ): Promise<PortalPose> {
    return this.withDataOperation(operationId, signal =>
      this.dataClient(clientId).pose(portal, signal));
  }

  async domainDataClose(clientId: string): Promise<void> {
    const entry = this.dataClients.get(clientId);
    if (!entry) return;
    await entry.client.close();
    if (this.dataClients.get(clientId) === entry) {
      this.dataClients.delete(clientId);
      entry.client.free();
    }
  }

  async dataOperationCancel(operationId: string): Promise<void> {
    this.dataOperations.get(operationId)?.abort();
  }

  async dataDownloadStart(
    clientId: string,
    dataId: string,
    optionsJson: string,
    operationId: string,
  ): Promise<string> {
    const controller = this.beginDataOperation(operationId);
    const downloadId = newId("download");
    const download: WebDownload = {
      controller,
      task: Promise.resolve(0),
      pending: null,
      delivered: false,
      changed: null,
    };
    download.task = this.dataClient(clientId).readTo(
      dataId,
      bytes => new Promise<void>((acknowledge, reject) => {
        download.pending = { encoded: bytesToBase64(bytes), acknowledge, reject };
        download.changed?.();
        download.changed = null;
      }),
      JSON.parse(optionsJson),
      controller.signal,
    );
    void download.task.catch(() => undefined).finally(() => {
      download.changed?.();
      download.changed = null;
      this.dataOperations.delete(operationId);
    });
    this.downloads.set(downloadId, download);
    return downloadId;
  }

  async dataDownloadNext(downloadId: string): Promise<string | null> {
    const download = this.download(downloadId);
    if (download.delivered && download.pending) {
      download.pending.acknowledge();
      download.pending = null;
      download.delivered = false;
    }
    try {
      while (!download.pending) {
        const outcome = await Promise.race([
          download.task.then(() => "done" as const),
          new Promise<"changed">(resolve => { download.changed = () => resolve("changed"); }),
        ]);
        if (outcome === "done" && !download.pending) return null;
      }
      download.delivered = true;
      return download.pending.encoded;
    } catch (error) {
      download.observedError = error;
      throw error;
    }
  }

  async dataDownloadCancel(downloadId: string): Promise<void> {
    const download = this.downloads.get(downloadId);
    if (!download) return;
    download.controller.abort();
    download.pending?.reject(new Error("cancelled"));
    download.pending = null;
  }

  async dataDownloadClose(downloadId: string): Promise<void> {
    const download = this.downloads.get(downloadId);
    if (!download) return;
    await this.dataDownloadCancel(downloadId);
    try {
      await download.task;
    } catch (error) {
      const kind = (error as { kind?: unknown }).kind;
      if (error !== download.observedError && kind !== "cancelled") throw error;
    } finally {
      this.downloads.delete(downloadId);
    }
  }

  async dataUploadStart(
    clientId: string,
    targetJson: string,
    size: number,
    optionsJson: string,
    operationId: string,
  ): Promise<string> {
    const controller = this.beginDataOperation(operationId);
    const uploadId = newId("upload");
    const upload: WebUpload = {
      controller,
      task: Promise.resolve({} as DataMetadata),
      pending: null,
      changed: null,
    };
    upload.task = this.dataClient(clientId).writeStream(
      JSON.parse(targetJson),
      size,
      maximum => {
        return new Promise<Uint8Array>((supply, reject) => {
          upload.pending = { maximum, supply, reject };
          upload.changed?.();
          upload.changed = null;
        });
      },
      JSON.parse(optionsJson),
      controller.signal,
    );
    void upload.task.catch(() => undefined).finally(() => {
      upload.changed?.();
      upload.changed = null;
      this.dataOperations.delete(operationId);
    });
    this.uploads.set(uploadId, upload);
    return uploadId;
  }

  async dataUploadNextMaximum(uploadId: string): Promise<number | null> {
    const upload = this.upload(uploadId);
    try {
      while (!upload.pending) {
        const outcome = await Promise.race([
          upload.task.then(() => "done" as const),
          new Promise<"changed">(resolve => { upload.changed = () => resolve("changed"); }),
        ]);
        if (outcome === "done" && !upload.pending) return null;
      }
      return upload.pending.maximum;
    } catch (error) {
      upload.observedError = error;
      throw error;
    }
  }

  async dataUploadPush(uploadId: string, bytesBase64: string): Promise<void> {
    const upload = this.upload(uploadId);
    if (!upload.pending) throw new Error("upload is not requesting a chunk");
    const bytes = base64ToBytes(bytesBase64);
    const pending = upload.pending;
    upload.pending = null;
    pending.supply(bytes);
  }

  async dataUploadResult(uploadId: string): Promise<DataMetadata> {
    const upload = this.upload(uploadId);
    try {
      return await upload.task;
    } catch (error) {
      upload.observedError = error;
      throw error;
    }
  }

  async dataUploadCancel(uploadId: string): Promise<void> {
    const upload = this.uploads.get(uploadId);
    if (!upload) return;
    upload.controller.abort();
    upload.pending?.reject(new Error("cancelled"));
    upload.pending = null;
  }

  async dataUploadClose(uploadId: string): Promise<void> {
    const upload = this.uploads.get(uploadId);
    if (!upload) return;
    await this.dataUploadCancel(uploadId);
    try {
      await upload.task;
    } catch (error) {
      const kind = (error as { kind?: unknown }).kind;
      if (error !== upload.observedError && kind !== "cancelled") throw error;
    } finally {
      this.uploads.delete(uploadId);
    }
  }

  async fleetOpen(sessionId: string, domainId: string): Promise<string> {
    const client = this.session(sessionId).fleet(domainId);
    const clientId = newId("fleet");
    this.fleetClients.set(clientId, { sessionId, client });
    return clientId;
  }

  async fleetList(clientId: string, queryJson: string, operationId: string): Promise<string> {
    const query = JSON.parse(queryJson);
    const converted: FleetQuery = { capabilities: query.capabilities ?? [], matchAllCapabilities: query.match_all_capabilities ?? false };
    const value = await this.withFleetOperation(operationId, signal => this.fleetClient(clientId).list(converted, signal));
    return JSON.stringify(value);
  }

  async fleetComputePool(clientId: string, queryJson: string, operationId: string): Promise<string> {
    const query = JSON.parse(queryJson);
    const converted: ComputePoolQuery = { mode: query.mode, capabilities: query.capabilities ?? [], matchAllCapabilities: query.match_all_capabilities ?? false };
    const value = await this.withFleetOperation(operationId, signal => this.fleetClient(clientId).computePool(converted, signal));
    return JSON.stringify(value);
  }

  async fleetOperationCancel(operationId: string): Promise<void> {
    const operation = this.fleetOperations.get(operationId);
    if (operation) operation.abort();
    else if (this.fleetCancelledBeforeStart.size < 1_024) {
      this.fleetCancelledBeforeStart.add(operationId);
    }
  }

  async fleetClose(clientId: string): Promise<void> {
    const entry = this.fleetClients.get(clientId);
    if (!entry) return;
    await entry.client.close();
    if (this.fleetClients.get(clientId) === entry) {
      this.fleetClients.delete(clientId);
      entry.client.free();
    }
  }

  async jobsOpen(sessionId: string, domainId: string): Promise<string> {
    const client = this.session(sessionId).jobs(domainId);
    const clientId = newId("jobs");
    this.jobsClients.set(clientId, { sessionId, client });
    return clientId;
  }

  async jobsEstimate(clientId: string, specJson: string, operationId: string): Promise<string> {
    const value = await this.withJobsOperation(operationId, signal =>
      this.jobsClient(clientId).estimate(webJobSpec(JSON.parse(specJson)), signal));
    return JSON.stringify(value);
  }

  async jobsSubmit(clientId: string, specJson: string, operationId: string): Promise<string> {
    return this.withJobsOperation(operationId, signal =>
      this.jobsClient(clientId).submit(webJobSpec(JSON.parse(specJson)), signal));
  }

  async jobsSubmitWithKey(clientId: string, specJson: string, idempotencyKey: string, operationId: string): Promise<string> {
    return this.withJobsOperation(operationId, signal =>
      this.jobsClient(clientId).submitWithKey(webJobSpec(JSON.parse(specJson)), idempotencyKey, signal));
  }

  async jobsList(clientId: string, queryJson: string, operationId: string): Promise<string> {
    const value = await this.withJobsOperation(operationId, signal =>
      this.jobsClient(clientId).list(webJobQuery(JSON.parse(queryJson)), signal));
    return JSON.stringify(value);
  }

  async jobsGet(clientId: string, jobId: string, operationId: string): Promise<string> {
    const value = await this.withJobsOperation(operationId, signal =>
      this.jobsClient(clientId).get(jobId, signal));
    return JSON.stringify(value);
  }

  async jobsCancel(clientId: string, jobId: string, operationId: string): Promise<string> {
    const value = await this.withJobsOperation(operationId, signal =>
      this.jobsClient(clientId).cancel(jobId, signal));
    return JSON.stringify(value);
  }

  async jobsOperationCancel(operationId: string): Promise<void> {
    const operation = this.jobsOperations.get(operationId);
    if (operation) operation.abort();
    else if (this.jobsCancelledBeforeStart.size < 1_024) {
      this.jobsCancelledBeforeStart.add(operationId);
    }
  }

  async jobsClose(clientId: string): Promise<void> {
    const entry = this.jobsClients.get(clientId);
    if (!entry) return;
    await entry.client.close();
    if (this.jobsClients.get(clientId) === entry) {
      this.jobsClients.delete(clientId);
      entry.client.free();
    }
  }

  async startPeer(sessionId: string, domainId: string): Promise<string> {
    const peer = await this.session(sessionId).startPeer(domainId);
    const id = newId("peer");
    this.peers.set(id, peer);
    return id;
  }

  async startPeerWithDiscovery(
    sessionId: string,
    domainId: string,
    mode: AukiDiscoveryModeName,
  ): Promise<string> {
    const sdk = await this.sdk();
    const discoveryMode =
      mode === "DiscoverAndAdvertise"
        ? sdk.AukiDiscoveryMode.DiscoverAndAdvertise
        : sdk.AukiDiscoveryMode.DiscoverOnly;
    const peer = await this.session(sessionId).startPeerWithDiscovery(
      domainId,
      discoveryMode,
    );
    const id = newId("peer");
    this.peers.set(id, peer);
    return id;
  }

  async peerId(peerHandle: string): Promise<string> {
    return this.peer(peerHandle).peerId;
  }

  async domainId(peerHandle: string): Promise<string> {
    return this.peer(peerHandle).domainId;
  }

  async discover(peerHandle: string): Promise<AukiDiscoveryCandidateInfo[]> {
    const candidates = await this.peer(peerHandle).discover();
    // wasm-bindgen objects must be freed (standard-protocols does this per candidate).
    try {
      return candidates.map(mapCandidate);
    } finally {
      for (const candidate of candidates) {
        candidate.free();
      }
    }
  }

  async discoverProtocol(
    peerHandle: string,
    protocolId: string,
  ): Promise<AukiDiscoveryCandidateInfo[]> {
    const candidates = await this.peer(peerHandle).discoverProtocol(protocolId);
    try {
      return candidates.map(mapCandidate);
    } finally {
      for (const candidate of candidates) {
        candidate.free();
      }
    }
  }

  async infoFetchExact(
    peerHandle: string,
    target: AukiExactTarget,
  ): Promise<string> {
    const sdk = await this.sdk();
    const info = await new sdk.AukiInfoClient(this.peer(peerHandle)).fetchExact(
      target,
    );
    return jsonStringify(info);
  }

  async catalogFetchResourcesExact(
    peerHandle: string,
    target: AukiExactTarget,
    variants: string[],
  ): Promise<string> {
    const sdk = await this.sdk();
    const resources = await new sdk.AukiCatalogClient(
      this.peer(peerHandle),
    ).fetchResourcesExact(
      target,
      variants as Parameters<CatalogClient["fetchResourcesExact"]>[1],
    );
    return jsonStringify(resources);
  }

  async registryListExact(
    peerHandle: string,
    target: AukiExactTarget,
    kind: string,
  ): Promise<string> {
    const sdk = await this.sdk();
    const entries = await new sdk.AukiRegistryClient(
      this.peer(peerHandle),
    ).listExact(target, kind as "device_model");
    return jsonStringify(entries);
  }

  async registryFetchExact(
    peerHandle: string,
    target: AukiExactTarget,
    kind: string,
    id: string,
    hash: string,
  ): Promise<string> {
    const sdk = await this.sdk();
    const entry = await new sdk.AukiRegistryClient(
      this.peer(peerHandle),
    ).fetchExact(target, kind as "device_model", id, hash);
    return jsonStringify(entry);
  }

  async blobFetchExact(
    peerHandle: string,
    target: AukiExactTarget,
    sha256: string,
  ): Promise<string> {
    const sdk = await this.sdk();
    const receipt = await new sdk.AukiBlobClient(
      this.peer(peerHandle),
    ).fetchExact(target, sha256);
    const bytes = receipt.bytes as unknown;
    let payload: Uint8Array;
    if (bytes instanceof Uint8Array) {
      payload = bytes;
    } else if (bytes instanceof ArrayBuffer) {
      payload = new Uint8Array(bytes);
    } else if (Array.isArray(bytes)) {
      payload = Uint8Array.from(bytes);
    } else {
      throw new Error("blobFetchExact: unexpected bytes type");
    }
    return jsonStringify({
      peerId: receipt.peerId,
      sha256: receipt.sha256,
      relayed: receipt.relayed,
      bytesBase64: bytesToBase64(payload),
    });
  }

  async streamSubscribeExact(
    peerHandle: string,
    target: AukiExactTarget,
    payloadKind: string,
    requestJson: string,
  ): Promise<string> {
    const sdk = await this.sdk();
    const request = JSON.parse(requestJson);
    const subscription = await new sdk.AukiStreamClient(
      this.peer(peerHandle),
    ).subscribeExact(
      target,
      payloadKind as Parameters<StreamClient["subscribeExact"]>[1],
      request,
    );
    const id = newId("stream");
    this.streams.set(id, subscription);
    return id;
  }

  async streamNext(subscriptionId: string): Promise<string | null> {
    const subscription = this.streams.get(subscriptionId);
    if (!subscription) {
      throw new Error(`unknown stream subscription: ${subscriptionId}`);
    }
    const next = await subscription.next();
    if (next == null) {
      return null;
    }
    if (next.kind !== "entry") {
      return jsonStringify({
        kind: next.kind,
        reason: next.reason,
        entry: null,
      });
    }
    const entry = next.entry;
    const payload = entry.payload as Uint8Array | ArrayBuffer | number[] | undefined;
    let payloadBase64: string | null = null;
    if (payload instanceof Uint8Array) {
      payloadBase64 = bytesToBase64(payload);
    } else if (payload instanceof ArrayBuffer) {
      payloadBase64 = bytesToBase64(new Uint8Array(payload));
    } else if (Array.isArray(payload)) {
      payloadBase64 = bytesToBase64(Uint8Array.from(payload));
    }
    return jsonStringify({
      kind: "entry",
      entry: {
        timestampNs: String(entry.timestampNs),
        sequence: String(entry.sequence),
        payloadBase64,
      },
    });
  }

  async streamCancel(subscriptionId: string): Promise<void> {
    const subscription = this.streams.get(subscriptionId);
    if (!subscription) {
      return;
    }
    await subscription.cancel();
    this.streams.delete(subscriptionId);
  }

  async messageOpenExact(
    peerHandle: string,
    target: AukiExactTarget,
    channelJson: string,
  ): Promise<string> {
    const sdk = await this.sdk();
    const channel = JSON.parse(channelJson);
    const sender = await new sdk.AukiMessageClient(
      this.peer(peerHandle),
    ).openExact(target, channel);
    const id = newId("message");
    this.messages.set(id, { peerHandle, sender });
    return id;
  }

  async messageSend(
    senderHandle: string,
    type: string,
    timestampNs: string,
    payloadBase64: string,
  ): Promise<void> {
    const entry = this.messages.get(senderHandle);
    if (!entry) {
      throw new Error(`unknown message sender: ${senderHandle}`);
    }
    await entry.sender.send(type, BigInt(timestampNs), base64ToBytes(payloadBase64));
  }

  async messageClose(senderHandle: string): Promise<void> {
    const entry = this.messages.get(senderHandle);
    if (!entry) {
      return;
    }
    this.messages.delete(senderHandle);
    await entry.sender.close();
    entry.sender.free();
  }

  async urdfModelFromXml(_xml: string): Promise<string> {
    throw new Error("urdfModelFromXml is native-only; use auki-urdf-fk wasm on web");
  }

  async urdfJointCount(_handle: string): Promise<number> {
    throw new Error("urdfJointCount is native-only");
  }

  async urdfResolve(_handle: string, _angles: number[]): Promise<string> {
    throw new Error("urdfResolve is native-only");
  }

  async urdfResolveIdentity(_handle: string): Promise<string> {
    throw new Error("urdfResolveIdentity is native-only");
  }

  async urdfModelFree(_handle: string): Promise<void> {
    throw new Error("urdfModelFree is native-only");
  }

  async shutdown(peerHandle: string): Promise<void> {
    const peer = this.peers.get(peerHandle);
    if (!peer) {
      return;
    }
    const owned = [...this.messages.entries()].filter(
      ([, entry]) => entry.peerHandle === peerHandle,
    );
    for (const [id, entry] of owned) {
      this.messages.delete(id);
      try {
        await entry.sender.close();
      } catch {
        /* ignore */
      }
      entry.sender.free();
    }
    await peer.shutdown();
    this.peers.delete(peerHandle);
  }

  async waitStopped(peerHandle: string): Promise<void> {
    await this.peer(peerHandle).waitStopped();
  }

  private dataClient(clientId: string): DomainDataClient {
    const entry = this.dataClients.get(clientId);
    if (!entry) throw Object.assign(new Error("The Domain data client is closed"), { kind: "closed" });
    return entry.client;
  }

  private beginDataOperation(operationId: string): AbortController {
    const controller = new AbortController();
    this.dataOperations.set(operationId, controller);
    return controller;
  }

  private async withDataOperation<T>(
    operationId: string,
    operation: (signal: AbortSignal) => Promise<T>,
  ): Promise<T> {
    const controller = this.beginDataOperation(operationId);
    try {
      return await operation(controller.signal);
    } finally {
      if (this.dataOperations.get(operationId) === controller) {
        this.dataOperations.delete(operationId);
      }
    }
  }

  private fleetClient(clientId: string): FleetClient {
    const entry = this.fleetClients.get(clientId);
    if (!entry) throw Object.assign(new Error("The Fleet client is closed"), { kind: "closed", code: "closed" });
    return entry.client;
  }

  private async withFleetOperation<T>(
    operationId: string,
    operation: (signal: AbortSignal) => Promise<T>,
  ): Promise<T> {
    const controller = new AbortController();
    this.fleetOperations.set(operationId, controller);
    if (this.fleetCancelledBeforeStart.delete(operationId)) controller.abort();
    try {
      return await operation(controller.signal);
    } finally {
      if (this.fleetOperations.get(operationId) === controller) {
        this.fleetOperations.delete(operationId);
      }
    }
  }

  private jobsClient(clientId: string): JobsClient {
    const entry = this.jobsClients.get(clientId);
    if (!entry) throw Object.assign(new Error("The DMS jobs client is closed"), { kind: "closed" });
    return entry.client;
  }

  private async withJobsOperation<T>(
    operationId: string,
    operation: (signal: AbortSignal) => Promise<T>,
  ): Promise<T> {
    const controller = new AbortController();
    this.jobsOperations.set(operationId, controller);
    if (this.jobsCancelledBeforeStart.delete(operationId)) controller.abort();
    try {
      return await operation(controller.signal);
    } finally {
      if (this.jobsOperations.get(operationId) === controller) {
        this.jobsOperations.delete(operationId);
      }
    }
  }

  private download(downloadId: string): WebDownload {
    const download = this.downloads.get(downloadId);
    if (!download) throw Object.assign(new Error("The download is closed"), { kind: "closed" });
    return download;
  }

  private upload(uploadId: string): WebUpload {
    const upload = this.uploads.get(uploadId);
    if (!upload) throw Object.assign(new Error("The upload is closed"), { kind: "closed" });
    return upload;
  }
}

export default registerWebModule(AukiSdkExpoModule, "AukiSdkExpo");

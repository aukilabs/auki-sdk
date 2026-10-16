import { NativeModule, requireNativeModule } from "expo";

import type {
  AukiDiscoveryCandidateInfo,
  AukiDiscoveryModeName,
  AukiPeerReachabilityModeName,
  AukiDomainInfo,
  AukiExactTarget,
  AukiSdkExpoModuleEvents,
} from "./AukiSdkExpo.types";

declare class AukiSdkExpoModuleType extends NativeModule<AukiSdkExpoModuleEvents> {
  importSession(credentialsJson: string, environmentJson: string | null): Promise<string>;
  sessionCredentials(sessionId: string, requestId: string): Promise<string>;
  ackSessionSave(sessionId: string, requestId: string, success: boolean): Promise<boolean>;
  closeSession(sessionId: string): Promise<void>;
  login(email: string, password: string, clientId?: string | null): Promise<string>;
  loginWithEnvironment(
    apiBaseUrl: string,
    ddsBaseUrl: string,
    dmsBaseUrl: string,
    email: string,
    password: string,
    clientId?: string | null,
  ): Promise<string>;
  accessibleDomains(sessionId: string): Promise<AukiDomainInfo[]>;
  /** Wire records. The facade maps these to camelCase. */
  domainsForPortalPage(sessionId: string, portal: string, organization: string, limit: number, cursor: string | null, operationId: string): Promise<unknown>;
  domainsPortalsPage(sessionId: string, domainId: string, limit: number, cursor: string | null, operationId: string): Promise<unknown>;
  domainsList(sessionId: string, queryJson: string, operationId: string): Promise<unknown>;
  domainsForPortal(
    sessionId: string,
    portal: string,
    organization: string | null,
    operationId: string,
  ): Promise<unknown>;
  domainsPortals(sessionId: string, domainId: string, operationId: string): Promise<unknown>;
  domainsPortal(
    sessionId: string,
    domainId: string,
    portal: string,
    operationId: string,
  ): Promise<unknown>;
  domainDataOpen(sessionId: string, domainId: string): Promise<string>;
  domainDataList(clientId: string, queryJson: string, operationId: string): Promise<unknown>;
  domainDataGet(clientId: string, dataId: string, operationId: string): Promise<unknown>;
  domainDataRead(clientId: string, dataId: string, operationId: string): Promise<string>;
  domainDataWrite(
    clientId: string,
    targetJson: string,
    bytesBase64: string,
    operationId: string,
  ): Promise<unknown>;
  domainDataDelete(clientId: string, dataId: string, operationId: string): Promise<void>;
  domainDataPoses(clientId: string, operationId: string): Promise<unknown>;
  domainDataPose(clientId: string, portal: string, operationId: string): Promise<unknown>;
  domainDataClose(clientId: string): Promise<void>;
  dataOperationCancel(operationId: string): Promise<void>;
  dataDownloadStart(
    clientId: string,
    dataId: string,
    optionsJson: string,
    operationId: string,
  ): Promise<string>;
  dataDownloadNext(downloadId: string): Promise<string | null>;
  dataDownloadCancel(downloadId: string): Promise<void>;
  dataDownloadClose(downloadId: string): Promise<void>;
  dataUploadStart(
    clientId: string,
    targetJson: string,
    size: number,
    optionsJson: string,
    operationId: string,
  ): Promise<string>;
  dataUploadNextMaximum(uploadId: string): Promise<number | null>;
  dataUploadPush(uploadId: string, bytesBase64: string): Promise<void>;
  dataUploadResult(uploadId: string): Promise<unknown>;
  dataUploadCancel(uploadId: string): Promise<void>;
  dataUploadClose(uploadId: string): Promise<void>;
  fleetOpen(sessionId: string, domainId: string): Promise<string>;
  fleetList(clientId: string, queryJson: string, operationId: string): Promise<string>;
  fleetComputePool(clientId: string, queryJson: string, operationId: string): Promise<string>;
  fleetOperationCancel(operationId: string): Promise<void>;
  fleetClose(clientId: string): Promise<void>;

  jobsOpen(sessionId: string, domainId: string): Promise<string>;
  jobsEstimate(clientId: string, specJson: string, operationId: string): Promise<string>;
  jobsSubmit(clientId: string, specJson: string, operationId: string): Promise<string>;
  jobsSubmitWithKey(clientId: string, specJson: string, idempotencyKey: string, operationId: string): Promise<string>;
  jobsList(clientId: string, queryJson: string, operationId: string): Promise<string>;
  jobsGet(clientId: string, jobId: string, operationId: string): Promise<string>;
  jobsCancel(clientId: string, jobId: string, operationId: string): Promise<string>;
  jobsOperationCancel(operationId: string): Promise<void>;
  jobsClose(clientId: string): Promise<void>;
  startPeer(
    sessionId: string,
    domainId: string,
    reachability?: AukiPeerReachabilityModeName | null,
  ): Promise<string>;
  startPeerWithDiscovery(
    sessionId: string,
    domainId: string,
    mode: AukiDiscoveryModeName,
    reachability?: AukiPeerReachabilityModeName | null,
  ): Promise<string>;
  peerId(peerHandle: string): Promise<string>;
  domainId(peerHandle: string): Promise<string>;
  discover(peerHandle: string): Promise<AukiDiscoveryCandidateInfo[]>;
  discoverProtocol(
    peerHandle: string,
    protocolId: string,
  ): Promise<AukiDiscoveryCandidateInfo[]>;
  infoFetch(peerHandle: string, target: AukiExactTarget): Promise<string>;
  catalogFetchResources(
    peerHandle: string,
    target: AukiExactTarget,
    variants: string[],
  ): Promise<string>;
  registryList(
    peerHandle: string,
    target: AukiExactTarget,
    kind: string,
  ): Promise<string>;
  registryFetch(
    peerHandle: string,
    target: AukiExactTarget,
    kind: string,
    id: string,
    hash: string,
  ): Promise<string>;
  blobFetch(
    peerHandle: string,
    target: AukiExactTarget,
    sha256: string,
  ): Promise<string>;
  streamSubscribe(
    peerHandle: string,
    target: AukiExactTarget,
    payloadKind: string,
    requestJson: string,
  ): Promise<string>;
  streamNext(subscriptionId: string): Promise<string | null>;
  streamCancel(subscriptionId: string): Promise<void>;
  messageOpen(
    peerHandle: string,
    target: AukiExactTarget,
    channelJson: string,
  ): Promise<string>;
  messageSend(
    senderHandle: string,
    type: string,
    timestampNs: string,
    payloadBase64: string,
  ): Promise<void>;
  messageClose(senderHandle: string): Promise<void>;
  urdfFromXml(xml: string): Promise<string>;
  urdfJointCount(handle: string): Promise<number>;
  urdfResolve(handle: string, angles: number[]): Promise<string>;
  urdfResolveIdentity(handle: string): Promise<string>;
  urdfDispose(handle: string): Promise<void>;
  shutdown(peerHandle: string): Promise<void>;
  waitUntilStopped(peerHandle: string): Promise<void>;
}

export default requireNativeModule<AukiSdkExpoModuleType>("AukiSdkExpo");

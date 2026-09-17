import { NativeModule, requireNativeModule } from "expo";

import type {
  AukiDiscoveryCandidateInfo,
  AukiDiscoveryModeName,
  AukiDomainInfo,
  AukiExactTarget,
  AukiSdkExpoModuleEvents,
  DataMetadata,
  DomainPage,
  Portal,
  PortalDomain,
  PortalPose,
} from "./AukiSdkExpo.types";

declare class AukiSdkExpoModuleType extends NativeModule<AukiSdkExpoModuleEvents> {
  /** @internal Use importZitadelSession/closeSession, not these bridge methods. */
  _importZitadel(credentialsJson: string, environmentJson: string | null): Promise<string>;
  _zitadelCredentials(sessionId: string, requestId: string): Promise<string>;
  _ackZitadelSave(sessionId: string, requestId: string, success: boolean): Promise<boolean>;
  _closeSession(sessionId: string): Promise<void>;
  loginDev(email: string, password: string, clientId?: string | null): Promise<string>;
  loginWithEnvironment(
    apiBaseUrl: string,
    ddsBaseUrl: string,
    dmsBaseUrl: string,
    email: string,
    password: string,
    clientId?: string | null,
  ): Promise<string>;
  accessibleDomains(sessionId: string): Promise<AukiDomainInfo[]>;
  domainsList(sessionId: string, queryJson: string, operationId: string): Promise<DomainPage>;
  domainsForPortal(
    sessionId: string,
    portal: string,
    organization: string | null,
    operationId: string,
  ): Promise<PortalDomain[]>;
  domainsPortals(sessionId: string, domainId: string, operationId: string): Promise<Portal[]>;
  domainsPortal(
    sessionId: string,
    domainId: string,
    portal: string,
    operationId: string,
  ): Promise<Portal>;
  domainDataOpen(sessionId: string, domainId: string): Promise<string>;
  domainDataList(clientId: string, queryJson: string, operationId: string): Promise<DataMetadata[]>;
  domainDataGet(clientId: string, dataId: string, operationId: string): Promise<DataMetadata>;
  domainDataRead(clientId: string, dataId: string, operationId: string): Promise<string>;
  domainDataWrite(
    clientId: string,
    targetJson: string,
    bytesBase64: string,
    operationId: string,
  ): Promise<DataMetadata>;
  domainDataDelete(clientId: string, dataId: string, operationId: string): Promise<void>;
  domainDataPoses(clientId: string, operationId: string): Promise<PortalPose[]>;
  domainDataPose(clientId: string, portal: string, operationId: string): Promise<PortalPose>;
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
  dataUploadResult(uploadId: string): Promise<DataMetadata>;
  dataUploadCancel(uploadId: string): Promise<void>;
  dataUploadClose(uploadId: string): Promise<void>;
  jobsOpen(sessionId: string, domainId: string): Promise<string>;
  jobsEstimate(clientId: string, specJson: string, operationId: string): Promise<string>;
  jobsSubmit(clientId: string, specJson: string, operationId: string): Promise<string>;
  jobsList(clientId: string, queryJson: string, operationId: string): Promise<string>;
  jobsGet(clientId: string, jobId: string, operationId: string): Promise<string>;
  jobsCancel(clientId: string, jobId: string, operationId: string): Promise<string>;
  jobsOperationCancel(operationId: string): Promise<void>;
  jobsClose(clientId: string): Promise<void>;
  startPeer(sessionId: string, domainId: string): Promise<string>;
  startPeerWithDiscovery(
    sessionId: string,
    domainId: string,
    mode: AukiDiscoveryModeName,
  ): Promise<string>;
  peerId(peerHandle: string): Promise<string>;
  domainId(peerHandle: string): Promise<string>;
  discover(peerHandle: string): Promise<AukiDiscoveryCandidateInfo[]>;
  discoverProtocol(
    peerHandle: string,
    protocolId: string,
  ): Promise<AukiDiscoveryCandidateInfo[]>;
  infoFetchExact(peerHandle: string, target: AukiExactTarget): Promise<string>;
  catalogFetchResourcesExact(
    peerHandle: string,
    target: AukiExactTarget,
    variants: string[],
  ): Promise<string>;
  registryListExact(
    peerHandle: string,
    target: AukiExactTarget,
    kind: string,
  ): Promise<string>;
  registryFetchExact(
    peerHandle: string,
    target: AukiExactTarget,
    kind: string,
    id: string,
    hash: string,
  ): Promise<string>;
  blobFetchExact(
    peerHandle: string,
    target: AukiExactTarget,
    sha256: string,
  ): Promise<string>;
  streamSubscribeExact(
    peerHandle: string,
    target: AukiExactTarget,
    payloadKind: string,
    requestJson: string,
  ): Promise<string>;
  streamNext(subscriptionId: string): Promise<string | null>;
  streamCancel(subscriptionId: string): Promise<void>;
  messageOpenExact(
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
  urdfModelFromXml(xml: string): Promise<string>;
  urdfJointCount(handle: string): Promise<number>;
  urdfResolve(handle: string, angles: number[]): Promise<string>;
  urdfResolveIdentity(handle: string): Promise<string>;
  urdfModelFree(handle: string): Promise<void>;
  shutdown(peerHandle: string): Promise<void>;
  waitStopped(peerHandle: string): Promise<void>;
}

export default requireNativeModule<AukiSdkExpoModuleType>("AukiSdkExpo");

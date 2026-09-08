import { NativeModule, requireNativeModule } from "expo";

import type {
  AukiDiscoveryCandidateInfo,
  AukiDiscoveryModeName,
  AukiDomainInfo,
  AukiExactTarget,
  AukiSdkExpoModuleEvents,
} from "./AukiSdkExpo.types";

declare class AukiSdkExpoModuleType extends NativeModule<AukiSdkExpoModuleEvents> {
  /** @internal Use importZitadelSession/closeSession, not these bridge methods. */
  _importZitadel(credentialsJson: string, environmentJson: string | null): Promise<string>;
  _zitadelCredentials(sessionId: string, requestId: string): Promise<string>;
  _ackZitadelSave(sessionId: string, requestId: string, success: boolean): Promise<boolean>;
  _closeSession(sessionId: string): Promise<void>;
  loginDev(email: string, password: string): Promise<string>;
  accessibleDomains(sessionId: string): Promise<AukiDomainInfo[]>;
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

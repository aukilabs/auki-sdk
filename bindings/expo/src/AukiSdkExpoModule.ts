import { NativeModule, requireNativeModule } from "expo";

import type {
  AukiDiscoveryCandidateInfo,
  AukiDiscoveryModeName,
  AukiDomainInfo,
  AukiExactTarget,
  AukiSdkExpoModuleEvents,
} from "./AukiSdkExpo.types";

declare class AukiSdkExpoModuleType extends NativeModule<AukiSdkExpoModuleEvents> {
  loginDev(email: string, password: string): Promise<string>;
  loginWithDomainAccessToken(
    apiBaseUrl: string,
    ddsBaseUrl: string,
    dmsBaseUrl: string,
    domainAccessToken: string,
  ): Promise<string>;
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
  streamSubscribeExact(
    peerHandle: string,
    target: AukiExactTarget,
    payloadKind: string,
    requestJson: string,
  ): Promise<string>;
  streamNext(subscriptionId: string): Promise<string | null>;
  streamCancel(subscriptionId: string): Promise<void>;
  shutdown(peerHandle: string): Promise<void>;
  waitStopped(peerHandle: string): Promise<void>;
}

export default requireNativeModule<AukiSdkExpoModuleType>("AukiSdkExpo");

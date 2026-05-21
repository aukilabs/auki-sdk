import type {
  BrowserDomainPeer,
  PeerId,
  PeerSnapshot,
  Result,
  SensorSummary,
} from "./contract.js";
import { listDomains as listDiscoveryDomains } from "./discovery.js";
import { ok, transportUnavailable } from "./errors.js";

type Fetcher = (url: string) => Promise<Response>;

export type CreateBrowserDomainPeerOptions = {
  peerId: PeerId;
  fetcher?: Fetcher;
};

export async function createBrowserDomainPeer(
  options: CreateBrowserDomainPeerOptions,
): Promise<BrowserDomainPeer> {
  let snapshot: PeerSnapshot = {
    selfPeerId: options.peerId,
    domainName: null,
    participants: [],
    managerPeerId: null,
    electionState: "unknown",
  };
  const observers = new Set<(snapshot: PeerSnapshot) => void>();
  const fetcher = options.fetcher;

  const emit = () => {
    for (const observer of observers) observer(snapshot);
  };

  return {
    async getSelfPeerId() {
      return options.peerId;
    },
    listDomains(discoveryUrl) {
      return listDiscoveryDomains(discoveryUrl, fetcher);
    },
    async createDomain() {
      return transportUnavailable();
    },
    async joinDomain() {
      return transportUnavailable();
    },
    async leaveDomain() {
      snapshot = {
        selfPeerId: options.peerId,
        domainName: null,
        participants: [],
        managerPeerId: null,
        electionState: "unknown",
      };
      emit();
      return ok<void>(undefined);
    },
    observeParticipants(onSnapshot) {
      observers.add(onSnapshot);
      onSnapshot(snapshot);
      return () => {
        observers.delete(onSnapshot);
      };
    },
    async setParticipantMetadata() {
      return ok<void>(undefined);
    },
    async declareLocalSensors(_sensors: SensorSummary[]): Promise<Result<void>> {
      return ok<void>(undefined);
    },
    async setSensorPublication() {
      return transportUnavailable();
    },
    async subscribeToSensor() {
      return transportUnavailable();
    },
    async unsubscribeFromSensor() {
      return transportUnavailable();
    },
  };
}

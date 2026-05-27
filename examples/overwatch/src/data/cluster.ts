import {
  sdkRuntime,
  type ClusterPeerWire,
  type ClusterSnapshot,
  type ClusterStatus,
  type RuntimeParticipant as Participant,
} from "../sdk/runtime";
import type { ParticipantInfo } from "./participantInfo";

export type { ParticipantInfo, ClusterPeerWire, ClusterStatus, Participant, ClusterSnapshot };

type Listener = (snap: ClusterSnapshot) => void;

export function subscribeCluster(cb: Listener): () => void {
  return sdkRuntime.subscribeCluster(cb);
}

export function getCluster(): ClusterSnapshot {
  return sdkRuntime.getCluster();
}

export function shortPeer(id: string): string {
  if (id.length <= 14) return id;
  return id.slice(0, 4) + "..." + id.slice(-10);
}

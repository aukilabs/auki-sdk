import { sdkRuntime } from "../sdk/runtime";
import type { ParticipantInfo } from "./participantInfo";

export type InfoStatus =
  | "pending"
  | "ok"
  | "no_info"
  | "unreachable"
  | "bad_json";

export type InfoSnapshot = {
  info: ParticipantInfo;
  receivedAtMs: number;
  sessionStartedEstMs: number;
  clusterJoinedEstMs: number | null;
};

type Listener = (snap: InfoSnapshot | null, status: InfoStatus) => void;

export function subscribeInfo(url: string, cb: Listener): () => void {
  return sdkRuntime.subscribeCluster((cluster) => {
    const participant =
      cluster.self?.peer_id === url
        ? cluster.self
        : cluster.peers.find((p) => p.peer_id === url);
    if (participant) {
      cb(
        {
          info: participant.info,
          receivedAtMs: participant.receivedAtMs,
          sessionStartedEstMs: participant.sessionStartedEstMs,
          clusterJoinedEstMs: participant.clusterJoinedEstMs,
        },
        "ok",
      );
      return;
    }
    cb(null, cluster.status === null ? "pending" : "unreachable");
  });
}

export function formatAge(ms: number): string {
  if (ms < 0) return "0s";
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  const remMin = min % 60;
  return remMin === 0 ? `${hr}h` : `${hr}h ${remMin}m`;
}

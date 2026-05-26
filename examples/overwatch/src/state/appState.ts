import type { PeerSnapshot } from "../sdk/contract";

export type OperatorBanner =
  | { kind: "idle"; text: string }
  | { kind: "empty"; text: string }
  | { kind: "ok"; text: string }
  | { kind: "error"; text: string };

export type OperatorState = {
  self: PeerSnapshot["participants"][number] | null;
  remotes: PeerSnapshot["participants"];
  banner: OperatorBanner;
};

export function deriveOperatorState(snapshot: PeerSnapshot | null): OperatorState {
  if (!snapshot || snapshot.participants.length === 0) {
    return {
      self: null,
      remotes: [],
      banner: { kind: "idle", text: "Join or create a Domain" },
    };
  }

  const self =
    snapshot.participants.find((participant) => participant.is_self) ??
    snapshot.participants.find((participant) => participant.peer_id === snapshot.selfPeerId) ??
    null;
  const remotes = snapshot.participants.filter(
    (participant) => participant.peer_id !== snapshot.selfPeerId && !participant.is_self,
  );

  if (!self) {
    return {
      self: null,
      remotes,
      banner: {
        kind: "error",
        text: "This browser peer is missing from the Domain roster",
      },
    };
  }

  if (remotes.length === 0) {
    return {
      self,
      remotes,
      banner: { kind: "empty", text: "No remote peers" },
    };
  }

  return {
    self,
    remotes,
    banner: {
      kind: "ok",
      text: `${remotes.length} remote peer${remotes.length === 1 ? "" : "s"}`,
    },
  };
}

export function shortPeerId(peerId: string | null | undefined): string {
  if (!peerId) {
    return "pending";
  }
  return peerId.length <= 12 ? peerId : `${peerId.slice(0, 6)}...${peerId.slice(-6)}`;
}

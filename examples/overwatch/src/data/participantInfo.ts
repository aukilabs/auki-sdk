// `ParticipantInfo` — the wire shape every Auki participant exchanges
// to introduce itself. One schema, two transports:
//
//   - HTTP: `GET /api/info` on the cross-app Control API
//     (auki-sdk/docs/control-api.md). Park hits this both for itself
//     (`/api/info`) and for any discovered daemon
//     (`/api/daemons/<url>/info`).
//   - libp2p: the `/auki/cluster/1.0.0` participant protocol — same
//     content, same field names. Park's server-side ClusterRuntime
//     re-serializes peers' ParticipantInfo as JSON for the UI under
//     `/api/cluster/peers`.
//
// Defined here so `data/info.ts` (per-daemon polling) and
// `data/cluster.ts` (Park-itself + cluster peers) share the same type
// and drift between the two would surface as a TS error.

export type ParticipantInfo = {
  /** Application identifier (`"boosterapp"`, `"sentinel"`, `"park"`). */
  app: string;
  /** Operator-friendly label (`"k1-walker"`, `"webcam-front"`). */
  name: string;
  /** UUIDv4 minted at session boot. One daemon run = one session. */
  session_id: string;
  /** Identifier of the session's monotonic clock in the clock registry. */
  session_clock_id: string;
  /** Content-addressed hash pinning the exact clock-registry entry. */
  session_clock_hash: string;
  /** ns on the daemon's session monotonic clock at poll time. */
  session_now_ns: number;
  /** Session-clock value at the first peer connection. `null` while the
   * participant is alone; set once and sticky thereafter. */
  cluster_joined_at_ns: number | null;
  /** libp2p PeerId derived from `Wallet::derive_child("peer/v1")`,
   * canonical multibase-base58 (`12D3KooW…`). Stable across daemon
   * restarts when the wallet seed is persisted. */
  peer_id: string;
  /** Per-machine identifier — first non-loopback IEEE-administered MAC,
   * lowercased hex without separators (e.g. `"aabbccddeeff"`). */
  app_instance: string;
  /** Whether this peer is currently the cluster's Manager. Added in
   * the v0.0.35 (Hagall) SDK alongside `manager_peer_id` per BA-Q3. */
  is_manager: boolean;
  /** Canonical peer-id of whoever the cluster currently agrees is
   * the Manager. May equal `peer_id` when `is_manager` is true.
   * Empty string when the daemon is not in a cluster. */
  manager_peer_id: string;
};

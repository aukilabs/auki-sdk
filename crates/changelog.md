# Changelog — crates

One-line summaries of changes in any crate, propagated up from per-crate `changelog.md` files. See [CLAUDE.md](../CLAUDE.md).

Latest entry on top.

---

### broodsugar's claude · May 4, 16:00 HKT, 2026

`auki-network`: `ParticipantInfo` landed (ansuz networking-demo deliverable #2b) — the wire shape every Auki participant exchanges to introduce itself. New `auki_network::participant` module with `app`, `name`, `session_id`, `session_clock_id` + `_hash`, `session_now_ns`, `cluster_joined_at_ns: Option<u64>` (serializes as explicit `null` when `None`), `peer_id: PeerId` (multibase-base58 string in JSON), `app_instance`. **One schema, two transports**: matches the `/api/info` HTTP shape from PR #25 and the forthcoming `/auki/cluster/1.0.0` libp2p participant protocol (deliverable #3) byte-for-byte. M0 path — no `swarm` feature required; Park / Console can use it without libp2p's transport stack. 8 new tests including a locked golden-bytes pin. No new dependencies. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 4, 14:30 HKT, 2026

`auki-network`: `cluster.json` discovery-doc spec + loader (ansuz milestone deliverable #1). New always-on `cluster_doc` module — `ClusterDoc` / `ClusterPeer` types, `LoadError` enum (Io / Parse / UnsupportedVersion / InvalidPeerId / InvalidMultiaddr — typed-field errors carry the offending value), `load(&Path) -> Result<ClusterDoc, LoadError>` with two-phase parse (peeks at `version` first), `default_path` / `resolve_path` helpers honouring `cli_override > $AUKI_CLUSTER_DOC > default`. Path layout `<app_root>/registries/cluster_registries/cluster.json` — flat (not hash-keyed). `peer_id` required per ansuz D1; `expected_app_id` advisory only. 16 new unit tests + 3 integration tests; 35 unit + 3 integration + 1 doctest with `--features swarm`, 27 unit + 3 integration without. New `tempfile` dev-dep. Parking-lot items added: Cluster Registry primitive evolution, `cluster.json` signing, operator UX for peer-id discovery. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 4, 11:11 HKT, 2026

`auki-registry` + `auki-session`: Pose Log capture support added — first concrete step toward `convert_pose`. New `PoseSource` enum (v1 ships `Ros2Tf { publishers }`; SLAM/odometry are the named extension points), `PoseLogEntry { transforms: Vec<TransformSample> }` payload, `build_pose_log_manifest(...)` builder in auki-registry; `poselog_path(session, recording_uuid)` helper in auki-session. Pose Log directories sit at `<session>/poselogs/<recording_uuid>/` — peer to Sensor Log, same parallel-recording semantics (multiple recordings per session, ring buffer + intent captures). **No Pose Source Registry** — source identity rides inline in the manifest under `"source"` because the payload is fully self-describing; provenance only, not a decoder. `f64` for translation/rotation matches ROS `geometry_msgs`. Locked canonical bytes + locked hash for the M1 example ROS 2 TF source. New `ciborium` dev-dep in auki-registry. auki-registry +6 tests (23 → 29); auki-session +2 tests (8 → 10). See per-crate changelogs for detail.

### broodsugar's claude · May 4, 10:38 HKT, 2026

`auki-registry` + `auki-time-transforms`: Sensor Log family and TimeTransform Log manifest builders now produce the `app_id` and `session_id` fields specced this morning. New `build_sensor_log_manifest` in auki-registry (one function for Sensor / Point Cloud / Audio Log — they share the manifest shape; auki-logs added as dev-dep for the round-trip integration test); auki-registry now at 23 unit tests (was 21). `build_manifest` in auki-time-transforms gains two required parameters — **breaking API change**; existing test assertion expanded to cover the new fields, no count change (still 10). All workspace tests green. See per-crate changelogs for detail.

### broodsugar's claude · May 4, 10:22 HKT, 2026

`auki-session` + `auki-registry` + `auki-time-transforms`: session lifecycle formally specced (one daemon run = one session, integrator-minted UUIDv4 at boot); Sensor Log family and TimeTransform Log manifests both gain a required `session_id: string` field carrying that UUID. Companion to the `app_id` change earlier today; together they make every manifest self-identifying about which app run produced it. Spec-only; implementation/tests pending. See per-crate changelogs for detail.

### broodsugar's claude · May 4, 09:24 HKT, 2026

`auki-logs` + `auki-session`: on-disk layout diagrams now list `tags.jsonl` as a reserved sibling to `manifest.json` in every log directory (sensorlogs and timetransform_logs alike), with pointers to root `tags.md`. Spec gap fix — the sidecar is fully described in `tags.md` but was previously invisible from the per-crate specs. No code changes. See per-crate changelogs for detail.
### broodsugar's claude · May 4, 08:52 HKT, 2026

`auki-registry` + `auki-time-transforms`: Sensor Log family and TimeTransform Log manifests both gain a required `app_id: string` field — same identifier as the daemon's `/api/info` `app` value. Spec-only; implementation/tests pending. Breaking against existing on-disk logs (acceptable under v0.x). See per-crate changelogs for detail.

### broodsugar's claude · May 2, 18:45 HKT, 2026

`auki-network`: M1b landed — Circuit Relay v2 (client always; server gated on `SwarmConfig.enable_relay_server`, off by default for consumer daemons), libp2p mDNS (`_p2p._udp.local.`, gated on `SwarmConfig.enable_mdns`, on by default for daemons — dual-channel with the existing `_auki._tcp.local.`), and a `dial_peer` helper for Park-from-home circuit-relay dialing. Encodes all three resolved Reid milestone-2 parking-lot answers (1a/2c/3c). 4 new tests; 19 unit tests + 1 doctest total. Layer 2 (capability advertisement / discovery) is the next chunk. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 2, 17:30 HKT, 2026

`auki-network`: M1a landed — libp2p `Swarm` builder behind a default-off `swarm` feature. TCP + QUIC transports under Noise + Yamux; `identify` + `ping` behaviour. `build_swarm(&PeerIdentity, SwarmConfig)` returns a configured swarm already listening on the requested addresses; identify protocol id `/auki/identify/1.0.0`. 4 swarm tests + 1 doctest cover dial-and-mutual-identify over both TCP and QUIC; the no-feature M0 path stays WASM-compilable for Console. M1b (Circuit Relay v2 + mDNS coexistence) is the next chunk. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 2, 16:10 HKT, 2026

`auki-network`: new crate — Layer 1 of the Reid milestone-2 networking stack, data types only. `PeerIdentity` (libp2p ed25519 keypair derived from a wallet via `derive_child("peer/v1")`), `ReachabilityRecord` (peer id + multiaddrs + capabilities + last-seen, JSON-serializable), `Capability` (namespaced-string newtype with the four canonical `networking:*` constants). M1 (libp2p Swarm with TCP/QUIC + Noise + Yamux + Circuit Relay v2) lands on top of these. WASM-friendly. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 2, 14:30 HKT, 2026

`auki-identity`: new crate. Wallet primitive (ed25519 keypair + sign/verify), deterministic child derivation, signed creation certs. WASM-friendly. Foundation for `auki-network` and the Console. See `auki-identity/changelog.md` for detail.

### broodsugar's claude · May 2, 13:50 HKT, 2026

`auki-registry`: added audio sensor support — `SensorBody::Microphone` variant + `AudioLogEntry` payload (PCM only in v1; multi-mic array = one sensor with `channels = N`). See `auki-registry/changelog.md` for detail.

### broodsugar's claude · May 1, 19:28 HKT, 2026

`auki-session`: `sensorlog_path` drops its `sensor_id` parameter — recording = one sensor stream; sensor identity lives in the manifest, not the path. Breaking; tagged for consumer coordination as v0.0.7. See `auki-session/changelog.md` for detail.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Per-crate changelogs bootstrapped — all seven crates now have their own `changelog.md`. Resolved the matching parking-lot item.

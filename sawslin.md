# sawslin — joint-angle (encoder) sensor support

Quest brief authored by Nils via broodsugar's dobby, May 8, 20:35 HKT, 2026. Internal-audience document — references the quest codename `sawslin` (Park animates K1 URDF from streamed pose). External-facing PR titles, changelogs, and READMEs use "joint-angle (encoder) sensor support" instead.

This file is the kickoff brief handed to the SDK-side implementing agent. The boosterapp-side port (consuming this once shipped) is tracked separately.

## What this brief asks for

Add a fourth sensor-body variant to `auki_registry::SensorBody`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SensorBody {
    RgbCamera(RgbCamera),
    PointCloud(PointCloud),
    Microphone(Microphone),
    JointEncoders(JointEncoders),  // <-- new
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JointEncoders {
    /// Number of joints in each per-frame angle vector. Sanity-check
    /// invariant for deserialization — the per-frame payload's
    /// `angles_rad` length MUST equal this. Equivalent in spirit to
    /// `Microphone::channels`.
    pub joint_count: u32,
    /// Expected publish rate in Hz, observed at sensor bootstrap.
    /// Sizing hint for segment duration / consumer buffers; not part
    /// of identity logic. Same role as `RgbCamera::frame_rate_hz` and
    /// `PointCloud::frame_rate_hz`.
    pub frame_rate_hz: u32,
}
```

Plus the matching per-frame payload in `auki-datatypes`:

```proto
// crates/auki-datatypes/proto/joint_encoders.proto
syntax = "proto3";

package auki.joint_encoders;

// Per-frame joint-encoder log payload — segment payload for a Sensor
// Log whose registry entry has body kind `joint_encoders`. The frame's
// timestamp rides in the auki-logs framing's `timestamp_ns`, not here.
//
// Joint ordering is producer-defined and immutable per log; the
// registry entry's `joint_count` pins vector length. Mapping joint
// indices to URDF links is a consumer-side concern — the URDF lives
// with the consumer that interprets these readings, not the producer
// that emits them.
//
// Field number ledger (never reuse, never renumber):
//   JointEncodersLogEntry.angles_rad = 1
message JointEncodersLogEntry {
  // Joint angle readings in radians, indexed in the producer's emit
  // order. Length MUST equal the registry entry's `joint_count`.
  repeated float angles_rad = 1;
}
```

Wire up via `impl_log_payload!(joint_encoders::JointEncodersLogEntry);` in `crates/auki-datatypes/src/lib.rs` next to `pose::SpatialTransform`, `point_cloud::PointCloudLogEntry`, etc.

## Layering rationale (why sensor-data, not pose)

Joint angles are encoder readings — measurements of joint positions, before any kinematic interpretation. Pose (cartesian TF) is what you *compute* from them via forward kinematics. That makes joint angles the producer-side raw signal, and pose the consumer-side derived quantity. Structurally identical to:

- **`RgbCamera`** — producer ships pixel bytes; consumer holds intrinsics + extrinsics to project into world space.
- **`PointCloud`** — producer ships CDR bytes; consumer parses + transforms.
- **`Microphone`** — producer ships PCM bytes; consumer holds sample-format knowledge to decode.
- **`JointEncoders`** — producer ships angle floats; consumer holds URDF and does FK.

In every case the producer ships raw measurements with just enough deserialization metadata (`channels`, `point_step`, `joint_count`) for the consumer to read the bytes correctly. Schema-for-interpretation lives downstream. That's what the existing `SensorBody` shape encodes, and `JointEncoders` is a clean fourth variant of it.

This is the layering call Nils made — overriding an earlier reach for a `PoseSource::JointAngles` pose-log variant. The pose-log path forced a manifest-keying decision (`(from_frame_id, to_frame_id)` is required for `Ros2Tf` but conceptually wrong for joint-space readings) and conflated the measurement layer with the interpretation layer. Sensor-log placement collapses both problems.

## What's deliberately NOT in `JointEncoders`

- **No `joint_names: Vec<String>`.** URDF lives on consumer. The producer doesn't read URDF and shouldn't be authoritative for joint names. Consumers (Park, future analyses) hold the URDF and do FK; ordering is a producer-defined invariant per log, agreed by hand-coordination at integration time.
- **No `urdf_id` / `urdf_hash`.** Speculative — Park is K1-monoculture today. File a parking-lot item with a "revisit when ≥2 robot models share a Park instance" trigger; don't ship speculatively.
- **No `velocity_rad_per_s` / `effort_nm` companion in the per-frame payload.** ROS `sensor_msgs/JointState` carries `position` + `velocity` + `effort`. K1 publishes velocity. v1 ships positions only — same minimal stance Step 3/4 took for point cloud and audio (opaque-bytes / minimal-fields, lift to richer encodings only when a consumer earns them). File parking-lot item; revisit when a non-Park consumer needs it.
- **No `frame_id` like `RgbCamera` and `PointCloud` carry.** Joint encoders aren't in any cartesian frame — they're in joint space. Including a `frame_id` would invite consumers to look up a Frame Registry entry that doesn't make sense for this sensor type.

## Tests to add

In `crates/auki-datatypes/src/lib.rs` `tests` module — mirror the Step 3 (point cloud) and Step 4 (audio) test shapes:

```rust
fn step_joint_encoders_example() -> JointEncodersLogEntry {
    // 6-DOF arm fixture, integer-valued radians for stable wire bytes.
    JointEncodersLogEntry {
        angles_rad: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
    }
}

#[test]
fn joint_encoders_round_trips() { /* prost encode/decode */ }

#[test]
fn joint_encoders_log_payload_round_trips() { /* via LogPayload trait */ }

#[test]
fn joint_encoders_wire_bytes_locked() {
    // Cross-language conformance vector — Python binding work pinned to these bytes.
    let entry = step_joint_encoders_example();
    let bytes = entry.encode_to_vec();
    assert_eq!(hex::encode(&bytes), "EXPECTED_HEX");  // fill in after first run
    assert_eq!(auki_hash::hash_jcs_bytes(&bytes), "EXPECTED_HASH");  // ditto
}
```

In `crates/auki-registry/src/lib.rs` `tests` module:

```rust
#[test]
fn sensor_registry_entry_jointencoders_round_trips() { /* JCS round-trip */ }

#[test]
fn sensor_registry_entry_jointencoders_canonical_bytes_locked() {
    // Lock the JCS bytes for a JointEncoders example so cross-language
    // readers (Python binding, Park) reproduce them exactly.
}

#[test]
fn sensor_registry_entry_jointencoders_hash_locked() {
    // Lock the XXH3-128 hash of the canonical bytes.
}
```

## Parking-lot items to file BEFORE the implementing PR

Per the standard Auki convention (architectural decisions land before the implementing PR, so per-step PRs reference the parking-lot decisions instead of relitigating them mid-review). Each entry terse — decision, one-sentence rationale, revisit trigger.

**At `crates/auki-registry/parking_lot.md`:**

1. **`joint_names` placement.** Decided: not on the registry entry. URDF lives with the consumer (Park). Reason: producer doesn't read URDF; making it declare names asks it to be authoritative for a schema it doesn't own. Revisit when ≥2 robot models share a Park instance — at that point either `urdf_id` for explicit coupling or `joint_name_hash` for opaque sanity-check.

2. **`SensorBody::JointEncoders` minimalism — `joint_count` only.** Decided: no `joint_name_hash`, no `urdf_id`, no per-joint metadata. Reason: `joint_count` is the deserialization invariant (matches `Microphone::channels`); anything richer is interpretation, not deserialization. Revisit when a real cross-robot mismatch shows up that `joint_count` alone doesn't catch.

**At `crates/auki-datatypes/parking_lot.md`:**

1. **`angles_rad` precision: f32 vs f64.** Going with `repeated float` (f32) to match `SpatialTransform`'s quaternion components. Revisit if a consumer needs higher precision for low-rate slow-motion replay.

2. **`velocity_rad_per_s` / `effort_nm` companion fields on `JointEncodersLogEntry`.** v1 is positions only — minimal-fields stance from Steps 3/4. ROS `JointState` carries velocity and effort and the K1 publishes velocity, so the upstream signal is there. Revisit when a consumer (predictive smoothing, force-controlled teleop, non-Park use) earns the addition. Adding new proto fields later is cheap (new field number); adding them now bakes them in for everyone.

## Sequencing — one PR

Single implementing PR. No multi-PR split needed because:

- `SensorBody` is an additive enum extension. No breaking changes to existing variants.
- `auki-datatypes` adds a new module behind a new proto file; existing modules unchanged.
- No manifest-shape churn (this is the saving grace of the sensor-log path).

Pre-file the parking-lot items in a separate **PR 0 (doc-only)** if the implementer wants the decisions visible before the code lands; otherwise fold them into the same PR.

PR 1 (this brief): `feat(registry,datatypes): add JointEncoders sensor body + per-frame log entry`.

PR 2 (out of scope, different repo): port `aukilabs/boosterapp` branch `sawslin/phase1-laneB-joint-states` from its current `SensorBody::JointState { joint_names, frame_rate_hz }` invention to the canonical `SensorBody::JointEncoders { joint_count, frame_rate_hz }`. The boosterapp branch needs:

- Drop `joint_names` extraction in `bootstrap_joint_state`.
- Drop name-uniqueness / name-vs-position-length validation.
- Rename `JointStateFanout` → `JointEncodersFanout` (or keep — boosterapp-internal naming is its own call).
- Update `build_joint_state_entry` to construct the canonical SDK shape.
- Pin `auki-sdk` dependency to whatever version ships this PR.

That porting is a separate PR opened against `boosterapp` after this lands.

## Auki SDK repo conventions reminder

- **Default branch is `develop`.** Branch off it, PR back to it.
- **No quest names in public-facing files.** "sawslin" doesn't appear in PR title, body, README, root `changelog.md` (internal `parking_lot.md` is OK to mention; this file you're reading is also internal-audience). External framing: "joint-angle / encoder sensor support."
- **Append-only changelogs at every level.** Leaf gets the full entry, parent gets a one-liner, root gets a one-liner. Update them in the same PR as the code change. Stamp with **real wall-clock HKT** — run `TZ='Asia/Hong_Kong' date '+%b %-d, %H:%M HKT, %Y'` immediately before stamping (format: `May 8, 20:35 HKT, 2026`). Don't reuse a stamp from earlier in the session.
- **Author the changelog entry as `broodsugar's claude`** (the persona running the implementing PR).
- **Conventional Commits prefix on the squash commit.** Suggested: `feat(registry,datatypes): add JointEncoders sensor body`.
- **Proto field numbers are append-only and never reused.** The ledger comment in the proto file is the contract.
- **Re-fetch `develop` before opening the PR.** Repo moves fast.

## Open questions to surface, not resolve

- **`producer: String` field on `JointEncoders`?** Considered but not added — `RgbCamera` / `PointCloud` / `Microphone` don't carry one either; producer identity comes from `(sensor_id, sensor_hash)` keying through the registry. If there's a reason for asymmetry, surface it. Default: no `producer` field.
- **`expected_segment_duration` knob?** Other sensor bodies don't carry it; segment duration is set by `build_*_log_manifest` callers, not pinned in the registry entry. Same call as the `producer` question — keep it minimal unless told otherwise.
- **Should the Python binding be touched in this PR?** `auki-network-py` and `auki-identity-py` are siblings. If `JointEncoders` needs a Python representation for boosterapp's Python code to construct registry entries, it's either (a) a follow-up PR after the Rust types land, or (b) folded into this PR. Default: follow-up. Boosterapp can hand-construct the JCS dict for v1 shipping until the PyO3 binding catches up — same way it does for `Microphone` if Microphone's Python binding doesn't exist yet either.

## What NOT to do

- **Don't draft the boosterapp port.** Different repo, separate PR.
- **Don't add `urdf_id` / `urdf_hash` / `joint_name_hash`** "to be safe." File the parking-lot item; ship without.
- **Don't add `velocity_rad_per_s` / `effort_nm`** to v1. File the parking-lot item; ship positions-only.
- **Don't touch `PoseSource` or `build_pose_log_manifest`.** This is fully on the sensor-log path; PoseLog is unchanged.
- **Don't bump the SDK version.** Versioning is a separate motion.
- **Don't write integration tests against boosterapp.** SDK tests are in-crate.

## Verification before opening the PR

- Three layers of `changelog.md` updated: leaf `crates/auki-registry/changelog.md` + leaf `crates/auki-datatypes/changelog.md` → `crates/changelog.md` → root `changelog.md`.
- Parking-lot items resolved or filed per the rules.
- No quest names ("sawslin") in any public-facing file (PR title, body, READMEs, root changelogs).
- `cargo build --workspace --all-features` clean.
- `cargo test --workspace` green, including new proto round-trips and the locked-bytes / locked-hash conformance tests.
- The locked-wire-bytes and locked-JCS-bytes / locked-hash assertions are filled with the actual computed values (run once with placeholder, paste the actual into the assertion, run again to confirm).

---

**TL;DR:** New `SensorBody::JointEncoders { joint_count, frame_rate_hz }` variant. New `auki.joint_encoders.JointEncodersLogEntry { repeated float angles_rad }` proto. Producer ships angle vectors; consumer holds URDF and does FK. Mirrors the `Microphone` / `PointCloud` shape exactly. Single PR. Boosterapp port is a separate PR after this lands.

# Odometry Components and pose history

`OdometryComponent` adapts a driver's measurements into the standard Component
runtime. It publishes `poses` observations (`auki.odometry-pose/v1`) without owning
any history or polling a vendor SDK. `capture_pose_history` attaches the existing
`ComponentRuntime` Buffer Product machinery, with caller-selected entry, byte and
source-time duration limits. There is no second pose queue.

## Declarations and measurements

Each payload carries a mandatory `contract`:

- `from_frame` and `to_frame`: full `FrameRegistryEntry` declarations, including
  owner, ID, axes, handedness and units. Their registry hashes are computed by the
  existing registry canonicalization. The host retains/serves the definitions.
- `clock`: full registry reference `{peer_id, id, hash}`. Its registered definition
  declares the epoch, native units, scope and boot/session. All observation times
  are nanoseconds relative to that epoch. A reference does not synchronize clocks.
- `session_id`: explicitly identifies the continuous odometry session.

`tracking` is either `{"status":"valid","pose":{...}}` or `{"status":"lost"}`.
A valid pose contains `translation: [x,y,z]` and `rotation_xyzw: [x,y,z,w]`.
The active Hamilton quaternion maps source coordinates into destination
coordinates. Translation uses destination-frame units. Endpoints must have equal
units and handedness for this rigid representation; convert conventions/units
explicitly before publishing if necessary. Nonfinite values and nonunit
quaternions are rejected. No covariance or accuracy claim is invented.

Measurement time is the enclosing `Observation.timestamp_ns`, under its exact
Output Manifest clock, which must match the payload contract. This is capture
(or vendor measurement) time, not receipt/publication time. Publishing requires
strictly increasing timestamps; rejected samples do not consume a sequence.

On an odometry reset, end the old output and create a new session Component/output
with a **new destination frame ID** and session ID. Use a new clock definition if
its epoch changes. Existing history remains interpretable; old map alignment
must not be attached to the new frame. The host detects vendor resets and owns
these identities; the SDK cannot detect an unreported physical origin reset.

## Peyote / remote consumer contract

1. The host mounts `ComponentProtocolEndpoint` on its existing peer and calls
   `export_product(&history.product())`. Creating a local Component alone does
   not export it over P2P.
2. Fetch that peer's Component catalog. Find Products whose metadata schema is
   `auki.odometry-product/v1`. Metadata `value` is the complete `PoseContract`,
   and `source_sequence` identifies the accepted observation it describes.
   Metadata appears after the first sample; it is not an invented initial pose.
3. Fetch/subscribe to that exact advertised Product reference using Component
   protocols. `mirror_product_exact::<PoseUpdate>` starts at the earliest retained sample;
   it initially fetches one observation. Call `sync_once` to catch up remaining
   history and receive subsequent observations. Existing live subscription APIs
   can follow updates. Keep the original Output, sequence, time, clock and frames.
4. For a trail, render valid samples' translations in the declared destination
   frame, using the declared units and axes. Break the trail on `lost`, sequence
   gaps or a changed frame/session. Do not connect unrelated odometry sessions.
5. To overlay portals, obtain an explicit odometry-to-map alignment. Domain
   membership, equal conventions, or naming a frame `odom` supplies no alignment.

Raw pose Products follow the existing Component wire protocols. This additive
schema needs a consumer decoder, but no protocol ID/version changes. It is not
the legacy pose-log schema. Consumers should reject unsupported schema versions.

The authenticated two-peer integration test in `auki-component-protocol` exercises
catalog discovery, metadata decoding, retained history, a subsequent update and
terminal propagation. Its measurements are explicit synthetic fixtures. Live
Galbot publication and actual Peyote rendering are separate integration gates.

## Capture-time queries

`pose_at(&product, &PoseQuery { contract, timestamp_ns, max_gap_ns })` works on the
same retained Product locally or after remote import. It requires the complete
expected contract. Exact measurements preserve their original pose. Otherwise,
translation is linear and rotation uses shortest-arc SLERP between the adjacent
valid measurements, only within `max_gap_ns`. Both bounds are required.

The result includes the Product identity, clock/frame/session contract, query
time and derivation: exact source sequence, or both bounding sequences and
interpolation fraction. Lost tracking, clock/frame/session mismatch, absent or
evicted bounds, ambiguous timestamp ordering and excessive gaps are errors.
There is no extrapolation, latest-pose fallback or implicit clock conversion.
Interpolation is an estimate, not a newly measured pose.

`Buffer::bracket_time_ns` is the generic underlying helper. It atomically leases
two adjacent samples without copying history, and requires strictly increasing
source timestamps. At that lower level the enclosing Product supplies the clock;
pose consumers should use the contract-checking `pose_at` helper.

The Galbot adapter must still verify what `get_odom` measures, declare its real
frame/session and clock, and publish measurement timestamps. It must not label
odometry as the legacy navigation map or timestamp an untimed navigation pose
with the camera's time.

# QR localization

Experimental calibrated camera localization against individual resolved QR
anchors. The convention is **local maps first**: the peer looks up a detection
in its own maps, asks the Mapper to admit an unknown QR, then localizes using
its accepted local placement. A whole snapshot is not a localizer input.

## Data flow

1. Retain a Camera Component's `VideoFrame` output as a Buffer Product. Declare
   an optical frame: +X right, +Y down, +Z forward.
2. Create `Calibration::for_product` with measured intrinsics and distortion for
   that exact image resolution, crop and optics. Supported models are pinhole,
   Brown–Conrady (4/5/8 coefficients), and OpenCV fisheye (4 coefficients).
3. Bind `QrDetector` to that camera Product and retain its detections.
4. Create `PortalMaps` with the host's local invocation context and register
   local `Arc<MapComponent>` instances. Its authorized `find_qr` queries return
   matching anchors, map definitions and exact snapshot references, not scenes.
5. Bind `QrLocalizerComponent` with `Arc::new(local_maps.clone())` as its
   `QrResolver`. It queries the live maps for each detection. A newly accepted
   anchor is available on the next processed observation without rebinding.
6. Route `needs_mapping` detection indices to
   [PortalMapper](../auki-qr-mapper/README.md). This helper checks local maps
   again before querying Portal metadata and proposing an authorized placement.
   The host owns this asynchronous work; the localizer never makes HTTP calls.

The [pipeline test](tests/pipeline.rs) renders a real QR image, observes a local
map miss, commits an anchor, and verifies that the same running localizer now
produces a pose. An empty image afterward produces no stale estimate.

The default feature set offers the geometry API without the native detector's
Session dependency; it compiles for WASM. The `qr-detector` feature enables the
native Component adapter. Browser and hardware acceptance remain untested.

## Application-controlled processing

`bind` preserves continuous processing and permits control by the local peer.
Use `bind_with_controls` to supply an explicit caller authorization policy and
optionally start paused. The Component advertises four Operables:

- `pause(ControlRequest {})` stops continuous localization. A successful reply
  means no continuous localization is in flight. Detection input continues to
  drain, counting skipped observations.
- `resume(ControlRequest {})` resumes on observations captured after the call's
  retained-buffer cutoff. It does not replay paused backlog. Calling it while
  already running leaves processing unchanged.
- `localize_once(LocalizeOnce { detection_product, detection_sequence })`
  processes exactly that retained detection observation, including while paused.
  It returns a `LocalizationBatch` directly and leaves the processing mode
  unchanged. It does not publish to `poses`, so a historical request cannot
  disturb the continuous stream's timestamp order.
- `status(ControlRequest {})` reports running/paused/closed mode, continuous and
  one-shot processed counts, skipped count, and the last processed detection
  sequence (which can move backward after a historical one-shot request).

The application selects the Product and sequence from retained detector output;
there is no implicit “latest” selection. Wrong Products and missing or evicted
sequences are rejected. Configure sufficient detection retention before binding.
A one-shot request uses current map state at lookup time and reports the actual
map snapshot used; it does not reconstruct historical map state.

Processing is serialized. Concurrent controls return `Rejected("localizer busy")`
while processing owns the component; there is no private unbounded work queue.
The application decides whether and when to retry. Counts include processed
batches with no estimates or rejected detections, not only successful poses.
After close or drop, retained processing/control handles cannot restart work;
status can still report closed. Network export and remote caller authorization
remain explicit host responsibilities.

These controls schedule localization only. The application still chooses when
to run detection, admit a portal through the mapper, or refresh a pose based on
elapsed time, movement, or tracking confidence. Pausing localization does not
pause the detector or provide camera tracking between QR observations.

## Individual anchor contract

`ResolvedQr` contains exactly one `QrAnchor`, its `MapDefinition` and the
`SnapshotReference` at which it was read. `QrResolver` is a replaceable interface;
`PortalMaps` is the default implementation. An application may explicitly supply
a resolver backed by someone else's map. The convention for normal operation
is to admit imported placements into a local map first, with explicit alignment
when the source and destination frames differ.

PnP solvers and camera models are provided by `auki_geometry::pnp`, enabled
through its optional `pnp` feature. The localizer retains its observation quality
checks and map-specific placement composition.

The pure `estimate_camera_pose(&map_definition, &anchor, ...)` function does not
perform lookup or require a scenegraph snapshot. It uses the QR's encoded-square
side length in meters, excluding its quiet zone, and printed TL/TR/BR/BL corners.
Refined corners are preferred when available. Positive depth, front-facing
geometry and reprojection RMS checks reject unusable results. Default thresholds
are 16 square pixels of area and 2 pixels RMS, requiring camera-specific tuning.

`PortalMaps` accepts only Map Components owned by its local peer. It is bounded
to 64 registered maps and uses the maps' read-authorized Operables. A closed,
unavailable or denied map is an error, not evidence that a QR is unknown.
Matching uses exact decoded payload equality. Duplicate payloads within one map
are ambiguous and rejected. The same payload in several maps yields separately
identified estimates; shared Domain membership does not align those maps.

## Results and lifecycle

The `poses` Observable uses `auki.qr-localization/v3`. Each `LocalizedQr` has a
source detection index, resolved QR (including map/frame/snapshot), and camera
pose estimate. Translation is in that map's units and quaternion order is WXYZ.
There is no single output-level spatial frame because a batch can contain results
in several maps. Each camera pose explicitly carries `from_frame_id` (the calibrated optical
camera frame) and `to_frame_id` (the selected map frame). An anchor whose
destination differs from that map is rejected before solving. The v3 contract
requires these labels; unlabelled v2 poses are not silently migrated.

Each batch also identifies the detection Product/sequence, source camera frame,
clock timestamp and full calibration. Missing/mismatched camera provenance rejects
the batch. Unknown QRs appear in `needs_mapping`; errors appear in `rejected_codes`.
Batches with no usable QRs contain no estimates. Multiple anchors are not fused.
The host must apply freshness policy; source timestamps are not processing times,
and clocks from different peers are not assumed comparable. Buffered detections
are processed against map state at lookup time, not a historical map version.

Calibration and camera binding remain fixed for a localizer's lifetime. Changed
optics/camera require a replacement calibration and binding. Local map updates
and registry changes do not. `close(timestamp_ns)` joins the input worker then
ends the output with a reconfiguration notice; Drop also joins the worker. The
host owns retention, network authorization and awaited shutdown. Input reader
failures/overruns are available through `input().stats()`.

Planar pose ambiguity, motion blur, incorrect size, moved anchors and calibration
error can still produce plausible but wrong poses. These are individual camera
pose observations, not navigation-ready tracking. Robot-body pose additionally
requires camera-to-body extrinsics. Creating the first map frame and deriving
new anchor placements remain explicit Mapper/host responsibilities.

```sh
cargo test --locked -p auki-qr-localizer --all-features
cargo clippy --locked -p auki-qr-localizer --all-features --all-targets --no-deps -- -D warnings
cargo check --locked -p auki-qr-localizer --target wasm32-unknown-unknown
```

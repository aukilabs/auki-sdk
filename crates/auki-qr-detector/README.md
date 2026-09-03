# `auki-qr-detector`

QR Lab-backed QR detection for the Auki SDK. This is the first reference
implementation of the session crate's bring-your-own `CameraDetector`
package, not a privileged or closed SDK detector API. Other developers can
register their own implementation, input contracts, configuration, and output
types with `RegisteredCameraDetector` without depending on this crate.

The crate is the typed producer side of the SDK's intentionally generic
`DetectionFrame` envelope. It scans raw 8-bit luminance, packed RGB, or NV12
camera frames and emits one versioned JSON payload per source frame with
`type = "qr"`. NV12 is scanned directly through its full-resolution Y plane; the
interleaved chroma plane is validated but does not need conversion. Consumers
can decode the payload with [`QrDetections::decode`].

`RegisteredQrDetector` pins the implementation to its content-addressed
Detector Registry entry. Its `start` method tails an open local Camera Sensor
Log; `start_stream` consumes an asynchronous local or remote camera stream.
Both declare and write the named Detection Log resource that other peers
discover and consume.

For applications built on the network-independent typed-dataflow primitives,
`QrDetector::bind_product` constructs a real Detector Component whose `frames`
Product input binds directly to an existing `RetainedProduct<VideoFrame>`.
Results are exposed as a typed `detections` Observable, so an application can
retain them as a normal Buffer Product. This path does not create a parallel
Sensor or Sensor Log. `QrDetectorComponent::replace_product` moves the stable
Detector Component to a compatible replacement camera Product while keeping
its Catalog input binding current.

It accepts `image_encoding = "raw"` with `pixel_format = "luma8"`, `rgb8`, or
`YUV_NV12`, plus `image_encoding = "jpeg"`. Raw layout comes from the Camera
Registry; JPEG dimensions are checked against that immutable contract after
decoding. The SDK detector boundary owns JPEG decompression, while QR Lab
converts decoded RGB8 with BT.601 luma weights and reuses its scratch storage.
Every-frame and timestamp-based periodic cadence are both supported.

Each decoded code retains the source-frame pixel corners in `TL → TR → BR →
BL` order. When QR Lab has subpixel refinement available, those are exposed in
`refined_corners_px`; consumers should prefer them for PnP.

Portal recognition and portal-service lookup intentionally do not live here:
this crate detects all valid QR codes. A portal-specific detector or mapper can
filter `QrDetections` by payload and attach portal geometry without coupling QR
scanning to a service.

```rust,no_run
use auki_qr_detector::{DetectionCadence, QrDetector, QrDetectorConfig};
use auki_session::{DetectorInstanceSpec, Peer, SensorLogHandle};
use std::time::Duration;

let peer = Peer::new("robot", "mapping");
let detector = QrDetector::new(QrDetectorConfig::robust_fast())
    .register(&peer, "aukilabs/qr-lab/v1")?;
let session = peer.start_session()?;

# let input_log: SensorLogHandle = todo!();
let task = detector.start(
    &session,
    DetectorInstanceSpec::rolling(
        "qr-left-1hz",
        DetectionCadence::Periodic { period_ns: 1_000_000_000 },
        Duration::from_secs(5),
        Duration::from_secs(1),
    ),
    &input_log,
)?;

// The task now processes frames as the camera producer appends them to
// `input_log` and writes results to `task.detection_log()`.
task.shutdown()?;
# Ok::<(), auki_qr_detector::QrDetectorError>(())
```

Park adapts an accepted live subscription without buffering or materializing
the Sensor Log:

```rust,ignore
use auki_session::CameraFrameSample;
use futures::StreamExt;
use std::sync::Arc;

let frames = subscription.entries.map(|entry| {
    entry
        .map(|entry| CameraFrameSample {
            timestamp_ns: entry.timestamp_ns,
            frame: Arc::new(entry.payload),
        })
        .map_err(|error| error.to_string())
});

let task = detector.start_stream(&session, instance, descriptor, frames)?;
// `descriptor` carries the remote LogRef, resolved Sensor Registry entry,
// and clock reference. The resulting Detection Log retains that provenance.
task.shutdown().await?;
```

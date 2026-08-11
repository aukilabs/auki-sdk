# `auki-barcode-detector`

rxing-backed barcode detection for the Auki SDK. This is a first-class
`CameraDetector` package: it registers a content-addressed Detector Registry
entry, tails a Camera Sensor Log (or live stream), and writes typed Detection
Log frames with `type = "barcode"`.

By default the detector enables all rxing **1D** formats. Apps may pass an
explicit format allowlist (including 2D formats such as `QR_CODE` when needed).
Higher-level mapping of payloads to application entities stays outside this
crate.

It accepts `image_encoding = "raw"` with `pixel_format = "luma8"`, `rgb8`, or
`YUV_NV12`, plus `image_encoding = "jpeg"`. NV12 is scanned through its
full-resolution Y plane (chroma is validated but unused). JPEG is decoded to
RGB8 then converted to luminance before rxing multi-decode. Returned corners
are always in source-frame pixels, in `TL → TR → BR → BL` order.

## Format allowlist

`BarcodeDetectorConfig.formats`:

| Value | Behavior |
|-------|----------|
| `None` (default) | All default 1D formats |
| `Some(list)` | Only the listed rxing `BarcodeFormat` variant names |
| `Some([])` | Error (`EmptyFormatsAllowlist`) |

Format names are rxing enum variant spellings (`EAN_13`, `CODE_128`, `RSS_14`,
`DXFilmEdge`, …). Allowlist matching is case-insensitive; emitted payloads use
the exact variant spelling. Unknown names return `UnknownFormat`.

**Default 1D set:** `CODABAR`, `CODE_39`, `CODE_93`, `CODE_128`, `EAN_8`,
`EAN_13`, `ITF`, `RSS_14`, `RSS_EXPANDED`, `TELEPEN`, `UPC_A`, `UPC_E`,
`UPC_EAN_EXTENSION`, `DXFilmEdge`.

**Excluded by default (2D / unsupported):** `AZTEC`, `DATA_MATRIX`,
`MAXICODE`, `PDF_417`, `QR_CODE`, `MICRO_QR_CODE`,
`RECTANGULAR_MICRO_QR_CODE`, `UNSUPORTED_FORMAT`.

## Payload

```json
{
  "schema_version": 1,
  "codes": [
    {
      "payload": "4012345678901",
      "format": "EAN_13",
      "corners_px": [
        {"x": 10.0, "y": 20.0},
        {"x": 110.0, "y": 20.0},
        {"x": 110.0, "y": 40.0},
        {"x": 10.0, "y": 40.0}
      ]
    }
  ]
}
```

Decode with [`BarcodeDetections::decode`].

```rust,no_run
use auki_barcode_detector::{BarcodeDetector, BarcodeDetectorConfig, DetectionCadence};
use auki_session::{DetectorInstanceSpec, Peer, SensorLogHandle};
use std::time::Duration;

let peer = Peer::new("robot", "mapping");

// Default (`formats: None`): all rxing 1D formats.
let detector = BarcodeDetector::new(BarcodeDetectorConfig::default())?
    .register(&peer, "aukilabs/barcode/v1")?;

// Optional allowlist — rxing enum variant names (may include 2D):
// let detector = BarcodeDetector::new(BarcodeDetectorConfig {
//     formats: Some(vec!["EAN_13".into(), "CODE_128".into()]),
// })?
// .register(&peer, "aukilabs/barcode/v1")?;

let session = peer.start_session()?;

# let input_log: SensorLogHandle = todo!();
let task = detector.start(
    &session,
    DetectorInstanceSpec::rolling(
        "barcode-left-1hz",
        DetectionCadence::Periodic { period_ns: 1_000_000_000 },
        Duration::from_secs(5),
        Duration::from_secs(1),
    ),
    &input_log,
)?;

task.shutdown()?;
# Ok::<(), auki_barcode_detector::BarcodeDetectorError>(())
```

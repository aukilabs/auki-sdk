# `auki-barcode-detector`

rxing-backed 1D / retail barcode detection for the Auki SDK. This crate mirrors
`auki-qr-detector` as a bring-your-own `CameraDetector` package: it registers a
content-addressed Detector Registry entry, tails a Camera Sensor Log (or live
stream), and writes typed Detection Log frames with `type = "barcode"`.

The crate is deliberately limited to barcode scanning. It does not associate
codes with portals, ESL SKUs, shelf geometry, or product regex validation;
those product-specific responsibilities belong to a later consumer.

It accepts `image_encoding = "raw"` with `pixel_format = "luma8"`, `rgb8`, or
`YUV_NV12`, plus `image_encoding = "jpeg"`. NV12 is scanned through its
full-resolution Y plane (chroma is validated but unused). JPEG is decoded to
RGB8 then converted to luminance before rxing multi-decode. Returned corners
are always in source-frame pixels, in `TL → TR → BR → BL` order.

## Symbology profiles

Config selects which rxing formats may be emitted (Cactus-aligned allowlists):

| Profile | Formats |
|---------|---------|
| `Product` | EAN-13, EAN-8, UPC-E, Code128, GS1 DataBar (`RSS_14`), GS1 DataBar Expanded (`RSS_EXPANDED`) |
| `Esl` | Code128, Code39, Code93, Codabar, ITF |
| `All` | union of the above |

Wire labels collapse to the Cactus `eslLabel` vocabulary:

- Code128 / Code39 / Code93 → `"code128"`
- EAN-13 / EAN-8 / UPC-E → `"ean13"`
- GS1 DataBar family → `"gs1DataBar"`
- ITF → `"itf"`
- Codabar → `"codabar"`

### Decoder gaps vs Cactus / Apple Vision

- **GS1 DataBar Limited:** rxing exposes `RSS_14` / `RSS_EXPANDED` only; there
  is no separate Limited enum. Limited codes may decode as `RSS_14` when the
  reader supports them, or may be missed.
- **ITF-14 vs Interleaved 2 of 5:** both map to rxing `ITF` and the wire label
  `"itf"`. There is no distinct ITF-14-only format.
- Decode quality is not bit-identical to Apple Vision; profiles match allowlists,
  not Vision confidence or distance behavior.

## Payload

```json
{
  "schema_version": 1,
  "codes": [
    {
      "payload": "4012345678901",
      "symbology": "ean13",
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
use auki_barcode_detector::{
    BarcodeDetector, BarcodeDetectorConfig, BarcodeSymbologyProfile, DetectionCadence,
};
use auki_session::{DetectorInstanceSpec, Peer, SensorLogHandle};
use std::time::Duration;

let peer = Peer::new("robot", "mapping");
let detector = BarcodeDetector::new(BarcodeDetectorConfig {
    profile: BarcodeSymbologyProfile::Product,
})
.register(&peer, "aukilabs/barcode/v1")?;
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

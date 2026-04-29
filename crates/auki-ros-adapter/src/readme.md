# `auki-ros-adapter/src/`

ROS2 → Auki translation: `sensor_msgs/CameraInfo` + `sensor_msgs/Image` into `SensorRegistryEntry` + `DynamicIntrinsics` + `SensorLogEntry`.

Sensor-Log payload schema spec: [`docs/sensor-log.md`](../../../docs/sensor-log.md).

## What's here

A single source file: [`lib.rs`](lib.rs).

## Architecture: two layers behind a feature gate

**Translation layer (always built).** Pure-data conversion functions, plain Rust mirrors of the ROS2 message shapes we use, the `CameraSubscriber` trait, and `MockCameraSubscriber`. **No `r2r` dependency.** Compiles and unit-tests on macOS without ROS2 installed.

**`r2r`-backed subscriber (`feature = "ros2"`).** Real subscriber that wires `r2r`'s generated message types into the mirror types. Currently a scaffold (`unimplemented!()` — concrete wiring lands at K1 bring-up). Requires ROS2 client libraries at link time.

This split is the central design decision: the translation logic — which is where bugs hide — is exercised by the test suite on every PR. The transport-layer wiring is what only works on the K1.

## ROS2 message mirrors

```rust
pub struct StampMsg     { pub sec: i32, pub nanosec: u32 }
pub struct CameraInfoMsg { pub stamp, pub width, pub height, pub distortion_model, pub k: [f64; 9], pub d: Vec<f64> }
pub struct ImageMsg     { pub stamp, pub width, pub height, pub encoding, pub step, pub data: Vec<u8> }
```

Only the M1 fields. `K` is row-major OpenCV/ROS2 convention: `[fx, 0, cx, 0, fy, cy, 0, 0, 1]`.

## Sensor Log payload types

```rust
pub struct DynamicIntrinsics {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub distortion_coefficients: Vec<f64>,
}

pub struct SensorLogEntry {
    pub dynamic_intrinsics: DynamicIntrinsics,
    pub frame: Vec<u8>,
}
```

`DynamicIntrinsics` is split out of the registry-side identity because intrinsics can refine at runtime (autofocus, calibration updates). The frame timestamp lives in `auki-logs`'s framing, not in the payload.

## Translation functions

```rust
pub fn stamp_to_ns(stamp: StampMsg) -> i64;
pub fn dynamic_intrinsics_from(info: &CameraInfoMsg) -> DynamicIntrinsics;
pub fn build_rgb_camera_registry_entry(
    sensor_id: impl Into<String>,
    info: &CameraInfoMsg,
    meta: &StaticCameraMetadata<'_>,
) -> auki_registry::SensorRegistryEntry;
pub fn build_sensor_log_entry(
    info: &CameraInfoMsg,
    image: &ImageMsg,
) -> (i64, SensorLogEntry);
```

## `StaticCameraMetadata`

The bits not present in `sensor_msgs/CameraInfo` but required by the registry entry — supplied by the integrator from out-of-band knowledge of the platform.

```rust
pub struct StaticCameraMetadata<'a> {
    pub pixel_format: &'a str,
    pub color_space: &'a str,
    pub frame_rate_hz: u32,
    pub intrinsics_model: &'a str,
}

impl StaticCameraMetadata<'static> {
    pub const K1_HEAD_RGB: Self = Self {
        pixel_format: "YUV_NV12",
        color_space: "BT.709",
        frame_rate_hz: 20,
        intrinsics_model: "pinhole",
    };
}
```

`K1_HEAD_RGB` captures the Booster K1 head camera's known-static configuration as a const. Calling `build_rgb_camera_registry_entry(..., &StaticCameraMetadata::K1_HEAD_RGB)` is the platform's one-line configuration call.

## `CameraSubscriber` trait

```rust
pub trait CameraSubscriber: Send {
    fn bootstrap(&mut self, timeout: Duration) -> Result<CameraInfoMsg, BootstrapError>;
    fn poll(&mut self) -> Vec<SubscriptionEvent>;
}

pub enum SubscriptionEvent {
    CameraInfo(CameraInfoMsg),
    Frame(ImageMsg),
}
```

`bootstrap` blocks for the first `CameraInfo` (used once at startup to mint the registry entry). `poll` is non-blocking and drains events that arrived since the last call.

`MockCameraSubscriber` (in test scope, but also used by `auki-k1-binary`'s tests) scripts a bootstrap response + an event queue.

## `r2r_subscriber` module (feature-gated)

Compiled only with `feature = "ros2"`. Currently a scaffold:

```rust
pub struct R2rCameraSubscriber { /* TODO(task-9) */ }

impl R2rCameraSubscriber {
    pub fn new(_namespace: &str, _node_name: &str) -> Result<Self, BootstrapError> {
        Err(BootstrapError::Transport(
            "r2r subscriber not yet implemented; wire up at K1 bring-up".into(),
        ))
    }
}
```

The struct + trait impl exist so the feature compiles on a Linux+ROS2 box. The actual `r2r::Node` + subscription wiring lands at task 9 against the real DDS bus, where it can be validated for free during the bring-up walkthrough. Topics: `/boostercamera/head/rgb/camera_info` and `/boostercamera/head/rgb`.

## Tests (12 total)

| Test | Asserts |
|------|---------|
| `stamp_to_ns_combines_seconds_and_nanoseconds` | `(5, 250_000_000)` → `5_250_000_000` |
| `stamp_to_ns_handles_zero_seconds` | Edge case |
| `stamp_to_ns_handles_max_representable_ros2_time` | `i32::MAX` seconds + max ns fits in i64 |
| `dynamic_intrinsics_extracts_correct_indices` | K[0]/K[4]/K[2]/K[5] → fx/fy/cx/cy |
| `dynamic_intrinsics_passes_distortion_through_unchanged` | D vector preserved verbatim |
| `dynamic_intrinsics_accepts_empty_distortion_for_none_model` | Empty D is allowed |
| `build_rgb_camera_registry_entry_matches_m1_example_hash` | Output hash matches the locked M1 example (`e8cb3879...`) |
| `build_sensor_log_entry_combines_info_and_image` | Frame timestamp from image, intrinsics from info |
| `sensor_log_entry_round_trips_through_cbor` | Full payload survives CBOR encode/decode |
| `mock_subscriber_returns_scripted_bootstrap_then_drains_events` | Mock semantics |
| `mock_subscriber_bootstrap_timeout_when_unscripted` | Default mock returns `Timeout` |
| `mock_subscriber_bootstrap_can_be_scripted_to_error` | Mock can simulate transport failures |
| `end_to_end_translation_from_subscription_to_log_entry` | Full subscription → registry → log-entry path against the mock |

## Consumers in this workspace

- `auki-k1-binary` — uses the trait, the mock (in tests), and `r2r_subscriber::R2rCameraSubscriber` (in production with `feature = "ros2"`)
- `auki-renderer` — reads back `SensorLogEntry` payloads via `auki-logs`

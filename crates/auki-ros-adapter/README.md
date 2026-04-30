# auki-ros-adapter

Translation contract from ROS2 sensor messages into Auki SDK types. Lets a ROS2 node mint registry entries and write to a Sensor Log without itself knowing the SDK's on-disk format.

## Two layers behind a feature gate

The crate is split intentionally:

1. **Translation (always built).** Pure-data conversion functions over plain Rust mirrors of the ROS2 message shapes, plus the `CameraSubscriber` trait and `MockCameraSubscriber`. **No `r2r` dependency.** Compiles and unit-tests on macOS without ROS2 installed.
2. **`r2r`-backed subscriber (`feature = "ros2"`).** Wires `r2r`'s generated message types into the mirror types and into the SDK pipeline. Requires ROS2 client libraries at link time.

This split is the central design decision: the translation logic — where bugs hide — is exercised everywhere; the transport-layer wiring only works on a real ROS2 host.

## Translation contract

### Inputs (ROS2 message mirrors)

```
StampMsg      { sec: i32, nanosec: u32 }
CameraInfoMsg { stamp, width, height, distortion_model, k: [f64; 9], d: Vec<f64> }
ImageMsg      { stamp, width, height, encoding, step, data: Vec<u8> }
```

`K` is row-major OpenCV/ROS2 convention: `[fx, 0, cx, 0, fy, cy, 0, 0, 1]`.

### Field mapping

From `sensor_msgs/CameraInfo`:

| ROS2 field                      | Goes to                                                          |
| ------------------------------- | ---------------------------------------------------------------- |
| `width`                         | `SensorRegistryEntry.body.RgbCamera.width`                       |
| `height`                        | `SensorRegistryEntry.body.RgbCamera.height`                      |
| `distortion_model`              | `SensorRegistryEntry.body.RgbCamera.distortion_model`            |
| `k[0]`, `k[4]`, `k[2]`, `k[5]`  | `DynamicIntrinsics.fx`, `fy`, `cx`, `cy` (row-major K matrix)    |
| `d`                             | `DynamicIntrinsics.distortion_coefficients`                      |

From `sensor_msgs/Image`:

| ROS2 field        | Goes to                                                                 |
| ----------------- | ----------------------------------------------------------------------- |
| `header.stamp`    | auki-logs framing `timestamp_ns` (sec × 10⁹ + nanosec)                  |
| `data`            | `SensorLogEntry.frame`                                                  |
| `width`/`height`  | discarded (must agree with the registry entry)                          |
| `encoding`        | discarded (must agree with the registry entry's `pixel_format`)         |
| `step`            | discarded (NV12 stride is recoverable from width)                       |

Discarded fields are *checked* (debug-only or first-frame validation) but not stored; storing them per-frame would be redundant with the registry entry.

For the SDK-side schemas (`SensorRegistryEntry`, `SensorLogEntry`, `DynamicIntrinsics`), see [`auki-registry`](../auki-registry).

## `StaticCameraMetadata`

Some registry-required fields aren't carried in `sensor_msgs/CameraInfo`:

- `pixel_format` (e.g. `YUV_NV12`)
- `color_space` (e.g. `BT.709`)
- `frame_rate_hz`
- `intrinsics_model` (e.g. `pinhole`)

These are supplied by the integrator from out-of-band knowledge of the platform. The SDK does not ship platform-specific constants — each integrator (boosterapp, future platforms) defines and owns its own values. The SDK's job is the contract; the integrator's job is the configuration.

## `CameraSubscriber` trait

The subscription interface is abstract over transport:

```
trait CameraSubscriber {
    fn bootstrap(timeout) -> Result<CameraInfoMsg, BootstrapError>;
    fn poll() -> Vec<SubscriptionEvent>;        // CameraInfo | Frame
}
```

`bootstrap` blocks for the first `CameraInfo` (used once at startup to mint the registry entry). `poll` is non-blocking and drains events that arrived since the last call.

Implementations: `MockCameraSubscriber` (tests), `R2rCameraSubscriber` (production, feature-gated).

## ROS2 topic conventions (Booster K1)

- `/boostercamera/head/rgb/camera_info` — `sensor_msgs/CameraInfo`
- `/boostercamera/head/rgb` — `sensor_msgs/Image`

These conventions live with the SDK so multiple downstream apps target the same topics.

## Status

Translation layer is complete and exercised by the test suite. The `r2r`-backed subscriber currently fails at runtime due to an `r2r` 0.9.5 typesupport bug that mismatches the CDR layout of the realsense camera driver's messages — fix in flight. Until then, downstream apps may bypass with a Python sidecar; see `boosterapp`.

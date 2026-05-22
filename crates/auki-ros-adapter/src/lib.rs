//! ROS2 → Auki translation: `sensor_msgs/CameraInfo` + `sensor_msgs/Image`
//! into `SensorRegistryEntry` + `DynamicIntrinsics` + `CameraFrame`.
//!
//! Sensor Log payload schema: see [`auki-proto`](../../auki-proto/README.md).
//! Translation contract: [`../README.md`](../README.md).
//!
//! ## Architecture
//!
//! Two layers, on either side of a feature gate:
//!
//! - **Translation (always built).** Pure-data conversion functions and the
//!   `CameraSubscriber` trait. Plain Rust mirrors of the ROS2 message shapes
//!   we care about — no r2r dep — so the logic is unit-testable on macOS.
//! - **r2r-backed subscriber (`feature = "ros2"`).** The real subscriber that
//!   wires r2r's generated message types into our mirror types. Requires
//!   ROS2 client libs at link time; only built on ROS2-aware Linux toolchains.
//!
//! Tests run on the translation layer with a `MockCameraSubscriber` driven by
//! scripted events. Real DDS integration testing lives at task 9 (K1 bring-up).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

pub use auki_registry;

// ─── ROS2 message mirror types ───────────────────────────────────────────────

/// Mirror of `std_msgs/Header.stamp` (the only field we use from `Header`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StampMsg {
    pub sec: i32,
    pub nanosec: u32,
}

/// Mirror of the `sensor_msgs/CameraInfo` fields used by M1.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraInfoMsg {
    pub stamp: StampMsg,
    pub width: u32,
    pub height: u32,
    pub distortion_model: String,
    /// `K` in row-major order: `[fx, 0, cx, 0, fy, cy, 0, 0, 1]`.
    pub k: [f64; 9],
    pub d: Vec<f64>,
}

/// Mirror of the `sensor_msgs/Image` fields used by M1.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageMsg {
    pub stamp: StampMsg,
    pub width: u32,
    pub height: u32,
    pub encoding: String,
    pub step: u32,
    pub data: Vec<u8>,
}

/// Mirror of the `sensor_msgs/PointCloud2` fields we use.
#[derive(Debug, Clone, PartialEq)]
pub struct PointCloud2Msg {
    pub stamp: StampMsg,
    /// Organized: rows. Unorganized: 1.
    pub height: u32,
    /// Organized: cols. Unorganized: total point count.
    pub width: u32,
    pub fields: Vec<PointFieldMsg>,
    pub is_bigendian: bool,
    pub point_step: u32,
    pub row_step: u32,
    pub data: Vec<u8>,
    pub is_dense: bool,
}

/// Mirror of `sensor_msgs/PointField`. The `datatype` byte is the ROS2 enum:
/// `1=int8`, `2=uint8`, `3=int16`, `4=uint16`, `5=int32`, `6=uint32`,
/// `7=float32`, `8=float64`.
#[derive(Debug, Clone, PartialEq)]
pub struct PointFieldMsg {
    pub name: String,
    pub offset: u32,
    pub datatype: u8,
    pub count: u32,
}

// ─── Output types written to the Sensor Log ─────────────────────────────────
//
// `DynamicIntrinsics` + the camera log entry moved to
// [`auki-proto`](../../auki-proto)'s `auki.camera` `.proto` at
// Step 1; `PointCloudLogEntry` followed at Step 3 under `auki.point_cloud`
// (now opaque-bytes-only). Re-exported here so existing call sites stay
// short.

pub use auki_proto::camera::{CameraFrame, DynamicIntrinsics};
pub use auki_proto::point_cloud::PointCloudLogEntry;

// ─── Translation functions ──────────────────────────────────────────────────

/// `header.stamp` → nanoseconds since the stamp's epoch.
///
/// On the K1 the camera publishes UTC stamps, so the result is UNIX-epoch ns.
/// The Sensor Log manifest's `clock_id` records which clock is in use.
pub fn stamp_to_ns(stamp: StampMsg) -> i64 {
    (stamp.sec as i64).saturating_mul(1_000_000_000) + stamp.nanosec as i64
}

/// Pull per-frame intrinsics out of a `CameraInfo`. The K matrix is row-major:
/// fx = K[0], cx = K[2], fy = K[4], cy = K[5].
pub fn dynamic_intrinsics_from(info: &CameraInfoMsg) -> DynamicIntrinsics {
    DynamicIntrinsics {
        fx: info.k[0],
        fy: info.k[4],
        cx: info.k[2],
        cy: info.k[5],
        distortion_coefficients: info.d.clone(),
    }
}

/// Static metadata supplied by the integrator — the bits not present in
/// `sensor_msgs/CameraInfo` but required by `SensorRegistryEntry`. For the K1
/// these come from out-of-band knowledge of the platform.
#[derive(Debug, Clone)]
pub struct StaticCameraMetadata<'a> {
    pub pixel_format: &'a str,
    pub color_space: &'a str,
    pub frame_rate_hz: u32,
    pub intrinsics_model: &'a str,
    /// Frame Registry id for the camera optical frame. Threaded into
    /// `Camera.frame_id` so consumers can resolve a
    /// `FrameRegistryEntry` for the camera's coordinate system.
    /// Conventionally REP-103 optical (`X right, Y down, Z forward`).
    pub frame_id: &'a str,
    /// Content hash of the exact Frame Registry entry named by
    /// [`StaticCameraMetadata::frame_id`].
    pub frame_hash: &'a str,
}

/// Build a `SensorRegistryEntry` from a bootstrap `CameraInfo` + integrator-
/// supplied static metadata. Currently only emits `Camera` bodies.
pub fn build_camera_registry_entry(
    sensor_id: impl Into<String>,
    info: &CameraInfoMsg,
    meta: &StaticCameraMetadata<'_>,
) -> auki_registry::SensorRegistryEntry {
    auki_registry::SensorRegistryEntry {
        sensor_id: sensor_id.into(),
        body: auki_registry::SensorBody::Camera(auki_registry::Camera {
            width: info.width,
            height: info.height,
            frame_rate_hz: meta.frame_rate_hz,
            pixel_format: meta.pixel_format.to_string(),
            color_space: meta.color_space.to_string(),
            intrinsics_model: meta.intrinsics_model.to_string(),
            distortion_model: info.distortion_model.clone(),
            frame_id: meta.frame_id.to_string(),
            frame_hash: meta.frame_hash.to_string(),
        }),
    }
}

/// Build a `CameraFrame` from the latest `CameraInfo` snapshot + an
/// Image. Returns `(timestamp_ns, entry)` ready for `auki_logs::Log::append`.
pub fn build_sensor_log_entry(info: &CameraInfoMsg, image: &ImageMsg) -> (i64, CameraFrame) {
    let timestamp_ns = stamp_to_ns(image.stamp);
    let entry = CameraFrame {
        dynamic_intrinsics: Some(dynamic_intrinsics_from(info)),
        frame: image.data.clone(),
    };
    (timestamp_ns, entry)
}

// ─── CameraSubscriber trait + mock ──────────────────────────────────────────

#[derive(Debug)]
pub enum BootstrapError {
    Timeout,
    /// Underlying transport failure. Real impl carries an r2r/io error.
    Transport(String),
}

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootstrapError::Timeout => write!(f, "bootstrap timed out"),
            BootstrapError::Transport(s) => write!(f, "transport: {s}"),
        }
    }
}

impl std::error::Error for BootstrapError {}

#[derive(Debug, Clone, PartialEq)]
pub enum SubscriptionEvent {
    CameraInfo(CameraInfoMsg),
    Frame(ImageMsg),
}

/// Source of `CameraInfo` + `Image` messages. Production impl wraps r2r
/// subscriptions; tests use [`MockCameraSubscriber`].
pub trait CameraSubscriber: Send {
    /// Block up to `timeout` waiting for the first `CameraInfo`. The K1 binary
    /// uses this once at startup to mint the registry entry; ongoing updates
    /// arrive via [`poll`](Self::poll).
    fn bootstrap(&mut self, timeout: Duration) -> Result<CameraInfoMsg, BootstrapError>;

    /// Non-blocking: drain any subscription events that have arrived since
    /// the last call. Returns events in arrival order.
    fn poll(&mut self) -> Vec<SubscriptionEvent>;
}

/// Test subscriber: scripts a bootstrap response and a queue of poll events.
pub struct MockCameraSubscriber {
    bootstrap: Mutex<Option<Result<CameraInfoMsg, BootstrapError>>>,
    events: Mutex<VecDeque<SubscriptionEvent>>,
}

impl MockCameraSubscriber {
    pub fn new() -> Self {
        Self {
            bootstrap: Mutex::new(None),
            events: Mutex::new(VecDeque::new()),
        }
    }

    pub fn with_bootstrap_ok(self, info: CameraInfoMsg) -> Self {
        *self.bootstrap.lock().unwrap() = Some(Ok(info));
        self
    }

    pub fn with_bootstrap_err(self, err: BootstrapError) -> Self {
        *self.bootstrap.lock().unwrap() = Some(Err(err));
        self
    }

    pub fn enqueue(&self, event: SubscriptionEvent) {
        self.events.lock().unwrap().push_back(event);
    }
}

impl Default for MockCameraSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraSubscriber for MockCameraSubscriber {
    fn bootstrap(&mut self, _timeout: Duration) -> Result<CameraInfoMsg, BootstrapError> {
        self.bootstrap
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Err(BootstrapError::Timeout))
    }

    fn poll(&mut self) -> Vec<SubscriptionEvent> {
        self.events.lock().unwrap().drain(..).collect()
    }
}

// ─── Point cloud translation ────────────────────────────────────────────────

/// Convert a ROS2 `PointField.datatype` byte to the SDK's typed enum.
/// Panics on unknown discriminants — surfaces wire-format drift loudly.
fn ros_datatype_to_sdk(datatype: u8) -> auki_registry::PointFieldDataType {
    match datatype {
        1 => auki_registry::PointFieldDataType::Int8,
        2 => auki_registry::PointFieldDataType::Uint8,
        3 => auki_registry::PointFieldDataType::Int16,
        4 => auki_registry::PointFieldDataType::Uint16,
        5 => auki_registry::PointFieldDataType::Int32,
        6 => auki_registry::PointFieldDataType::Uint32,
        7 => auki_registry::PointFieldDataType::Float32,
        8 => auki_registry::PointFieldDataType::Float64,
        n => panic!("unknown ROS2 PointField datatype: {n}"),
    }
}

/// Per-source-field instruction for repacking the byte stream.
enum NormalizationPlan {
    /// Field stays as-is (datatype/count unchanged); copy `size` bytes.
    PassThrough {
        src_offset: u32,
        dst_offset: u32,
        size: u32,
    },
    /// `name="rgb"`, `float32` → three `uint8` fields. 4 src bytes (`B,G,R,pad`)
    /// become 3 dst bytes (`R,G,B`).
    ExpandRgb { src_offset: u32, dst_offset: u32 },
    /// `name="rgba"`, `float32` → four `uint8` fields. 4 src bytes (`B,G,R,A`)
    /// become 4 dst bytes (`R,G,B,A`).
    ExpandRgba { src_offset: u32, dst_offset: u32 },
}

/// Normalized layout: SDK fields, packed point_step, and the plan to repack
/// per-frame data per-message.
struct Normalized {
    fields: Vec<auki_registry::PointField>,
    point_step: u32,
    plans: Vec<NormalizationPlan>,
}

/// Apply RGB(A) normalization to the field list. ROS2's `rgb`/`rgba` are
/// float32-packed (`0x00RRGGBB` / `0xAARRGGBB`); we expand them into separate
/// `uint8` channels at sequential offsets so cross-language readers don't
/// have to special-case the float-packing convention.
///
/// Other fields pass through unchanged. The output `point_step` packs fields
/// tightly (no padding); any padding bytes in the source are discarded.
fn normalize_layout(src_fields: &[PointFieldMsg]) -> Normalized {
    use auki_registry::{PointField, PointFieldDataType};

    const ROS2_FLOAT32: u8 = 7;

    let mut fields = Vec::with_capacity(src_fields.len() + 2);
    let mut plans = Vec::with_capacity(src_fields.len());
    let mut dst_offset: u32 = 0;

    for f in src_fields {
        match (f.name.as_str(), f.datatype, f.count) {
            ("rgb", ROS2_FLOAT32, 1) => {
                let dst = dst_offset;
                for (idx, ch) in ["r", "g", "b"].iter().enumerate() {
                    fields.push(PointField {
                        name: (*ch).to_string(),
                        offset: dst + idx as u32,
                        datatype: PointFieldDataType::Uint8,
                        count: 1,
                    });
                }
                plans.push(NormalizationPlan::ExpandRgb {
                    src_offset: f.offset,
                    dst_offset: dst,
                });
                dst_offset += 3;
            }
            ("rgba", ROS2_FLOAT32, 1) => {
                let dst = dst_offset;
                for (idx, ch) in ["r", "g", "b", "a"].iter().enumerate() {
                    fields.push(PointField {
                        name: (*ch).to_string(),
                        offset: dst + idx as u32,
                        datatype: PointFieldDataType::Uint8,
                        count: 1,
                    });
                }
                plans.push(NormalizationPlan::ExpandRgba {
                    src_offset: f.offset,
                    dst_offset: dst,
                });
                dst_offset += 4;
            }
            _ => {
                let datatype = ros_datatype_to_sdk(f.datatype);
                let size = datatype.byte_width() * f.count;
                fields.push(PointField {
                    name: f.name.clone(),
                    offset: dst_offset,
                    datatype,
                    count: f.count,
                });
                plans.push(NormalizationPlan::PassThrough {
                    src_offset: f.offset,
                    dst_offset,
                    size,
                });
                dst_offset += size;
            }
        }
    }

    Normalized {
        fields,
        point_step: dst_offset,
        plans,
    }
}

/// Repack `src_data` into `num_points × dst_point_step` bytes per the plans.
fn apply_normalization(
    plans: &[NormalizationPlan],
    src_data: &[u8],
    src_point_step: u32,
    num_points: usize,
    dst_point_step: u32,
) -> Vec<u8> {
    let dst_step = dst_point_step as usize;
    let src_step = src_point_step as usize;
    let mut out = vec![0u8; num_points * dst_step];

    for p in 0..num_points {
        let src_base = p * src_step;
        let dst_base = p * dst_step;
        for plan in plans {
            match *plan {
                NormalizationPlan::PassThrough {
                    src_offset,
                    dst_offset,
                    size,
                } => {
                    let so = src_base + src_offset as usize;
                    let d = dst_base + dst_offset as usize;
                    let n = size as usize;
                    out[d..d + n].copy_from_slice(&src_data[so..so + n]);
                }
                NormalizationPlan::ExpandRgb {
                    src_offset,
                    dst_offset,
                } => {
                    // src bytes (little-endian float32): [B, G, R, padding]
                    // dst bytes:                        [R, G, B]
                    let so = src_base + src_offset as usize;
                    let d = dst_base + dst_offset as usize;
                    out[d] = src_data[so + 2];
                    out[d + 1] = src_data[so + 1];
                    out[d + 2] = src_data[so];
                }
                NormalizationPlan::ExpandRgba {
                    src_offset,
                    dst_offset,
                } => {
                    // src bytes (little-endian float32): [B, G, R, A]
                    // dst bytes:                        [R, G, B, A]
                    let so = src_base + src_offset as usize;
                    let d = dst_base + dst_offset as usize;
                    out[d] = src_data[so + 2];
                    out[d + 1] = src_data[so + 1];
                    out[d + 2] = src_data[so];
                    out[d + 3] = src_data[so + 3];
                }
            }
        }
    }
    out
}

/// Build a `SensorRegistryEntry` (with `SensorBody::PointCloud`) from a
/// bootstrap `PointCloud2` message. The integrator supplies `frame_rate_hz`
/// plus `(frame_id, frame_hash)` out-of-band — the same way
/// `StaticCameraMetadata` works for cameras. (`PointCloud2Msg` does not
/// currently mirror ROS's `header.frame_id`; integrators source the frame id
/// from their topic configuration or platform knowledge.)
///
/// The output `fields`/`point_step` reflect [RGB(A) normalization](crate),
/// not the raw ROS2 layout — so the registry describes the bytes that
/// downstream readers will see in the log payload.
pub fn build_point_cloud_registry_entry(
    sensor_id: impl Into<String>,
    msg: &PointCloud2Msg,
    frame_rate_hz: u32,
    frame_id: impl Into<String>,
    frame_hash: impl Into<String>,
) -> auki_registry::SensorRegistryEntry {
    let normalized = normalize_layout(&msg.fields);
    auki_registry::SensorRegistryEntry {
        sensor_id: sensor_id.into(),
        body: auki_registry::SensorBody::PointCloud(auki_registry::PointCloud {
            fields: normalized.fields,
            point_step: normalized.point_step,
            is_bigendian: msg.is_bigendian,
            frame_rate_hz,
            frame_id: frame_id.into(),
            frame_hash: frame_hash.into(),
        }),
    }
}

/// Build a `PointCloudLogEntry` from a `PointCloud2` message. Returns
/// `(timestamp_ns, entry)` ready for `auki_logs::Log::append`. Applies the
/// same RGB(A) normalization as `build_point_cloud_registry_entry`.
///
/// Step 3 (2026-05-08): the entry is now opaque-bytes-only. `width` /
/// `height` / `is_dense` no longer ride on the per-frame entry — readers
/// resolve them via the `(sensor_id, sensor_hash)` pointing at the
/// `SensorBody::PointCloud` registry entry. ROS-shape interpretation
/// (`width × height × is_dense`) lives in the producer (here) and is
/// flattened into the bytes via the registry's `point_step` and `fields`.
pub fn build_point_cloud_log_entry(msg: &PointCloud2Msg) -> (i64, PointCloudLogEntry) {
    let timestamp_ns = stamp_to_ns(msg.stamp);
    let normalized = normalize_layout(&msg.fields);
    let num_points = (msg.width as usize).saturating_mul(msg.height as usize);
    let data = apply_normalization(
        &normalized.plans,
        &msg.data,
        msg.point_step,
        num_points,
        normalized.point_step,
    );
    let entry = PointCloudLogEntry { data };
    (timestamp_ns, entry)
}

// ─── PointCloudSubscriber trait + mock ──────────────────────────────────────

/// Source of `PointCloud2` messages. Production impl wraps r2r subscriptions;
/// tests use [`MockPointCloudSubscriber`].
///
/// Simpler than `CameraSubscriber` — there's no separate "info" topic to
/// bootstrap from; the static layout is embedded in every PointCloud2 message,
/// so `bootstrap` just blocks for the first one.
pub trait PointCloudSubscriber: Send {
    fn bootstrap(&mut self, timeout: Duration) -> Result<PointCloud2Msg, BootstrapError>;
    fn poll(&mut self) -> Vec<PointCloud2Msg>;
}

pub struct MockPointCloudSubscriber {
    bootstrap: Mutex<Option<Result<PointCloud2Msg, BootstrapError>>>,
    events: Mutex<VecDeque<PointCloud2Msg>>,
}

impl MockPointCloudSubscriber {
    pub fn new() -> Self {
        Self {
            bootstrap: Mutex::new(None),
            events: Mutex::new(VecDeque::new()),
        }
    }

    pub fn with_bootstrap_ok(self, msg: PointCloud2Msg) -> Self {
        *self.bootstrap.lock().unwrap() = Some(Ok(msg));
        self
    }

    pub fn with_bootstrap_err(self, err: BootstrapError) -> Self {
        *self.bootstrap.lock().unwrap() = Some(Err(err));
        self
    }

    pub fn enqueue(&self, msg: PointCloud2Msg) {
        self.events.lock().unwrap().push_back(msg);
    }
}

impl Default for MockPointCloudSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl PointCloudSubscriber for MockPointCloudSubscriber {
    fn bootstrap(&mut self, _timeout: Duration) -> Result<PointCloud2Msg, BootstrapError> {
        self.bootstrap
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Err(BootstrapError::Timeout))
    }

    fn poll(&mut self) -> Vec<PointCloud2Msg> {
        self.events.lock().unwrap().drain(..).collect()
    }
}

// ─── r2r-backed subscriber (gated) ──────────────────────────────────────────

#[cfg(feature = "ros2")]
pub mod r2r_subscriber {
    //! r2r-backed `CameraSubscriber`. Compiled only with `feature = "ros2"`,
    //! which requires ROS2 client libraries at link time.
    //!
    //! Pattern: a background thread runs a current-thread tokio runtime that
    //! drives both the r2r executor (`spin_once`) and two stream consumers
    //! that translate r2r message types into our mirror types and push them
    //! into bounded queues. `poll()` drains those queues from the main
    //! thread without blocking.

    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};

    use futures::stream::StreamExt;

    use super::*;

    /// Topic configuration. Defaults match the K1 image's published names
    /// (`/image_left_raw` + `/image_left_raw/camera_info`); pass through
    /// constants here that the binary exposes as CLI flags.
    #[derive(Debug, Clone)]
    pub struct R2rTopics {
        pub camera_info_topic: String,
        pub image_topic: String,
        pub node_name: String,
        pub node_namespace: String,
    }

    impl Default for R2rTopics {
        fn default() -> Self {
            Self {
                camera_info_topic: "/image_left_raw/camera_info".into(),
                image_topic: "/image_left_raw".into(),
                node_name: "auki_k1".into(),
                node_namespace: "".into(),
            }
        }
    }

    pub struct R2rCameraSubscriber {
        info_queue: Arc<Mutex<VecDeque<CameraInfoMsg>>>,
        image_queue: Arc<Mutex<VecDeque<ImageMsg>>>,
        stop: Arc<AtomicBool>,
        executor: Option<JoinHandle<()>>,
    }

    impl R2rCameraSubscriber {
        pub fn new(topics: R2rTopics) -> Result<Self, BootstrapError> {
            let ctx = r2r::Context::create()
                .map_err(|e| BootstrapError::Transport(format!("r2r context: {e}")))?;
            let mut node = r2r::Node::create(ctx, &topics.node_name, &topics.node_namespace)
                .map_err(|e| BootstrapError::Transport(format!("r2r node: {e}")))?;

            // Match the K1 mipi_cam publisher: RELIABLE / VOLATILE.
            let qos = r2r::QosProfile::default();
            let info_sub = node
                .subscribe::<r2r::sensor_msgs::msg::CameraInfo>(
                    &topics.camera_info_topic,
                    qos.clone(),
                )
                .map_err(|e| {
                    BootstrapError::Transport(format!(
                        "subscribe {}: {e}",
                        topics.camera_info_topic
                    ))
                })?;
            let image_sub = node
                .subscribe::<r2r::sensor_msgs::msg::Image>(&topics.image_topic, qos)
                .map_err(|e| {
                    BootstrapError::Transport(format!("subscribe {}: {e}", topics.image_topic))
                })?;

            let info_queue = Arc::new(Mutex::new(VecDeque::<CameraInfoMsg>::new()));
            let image_queue = Arc::new(Mutex::new(VecDeque::<ImageMsg>::new()));
            let stop = Arc::new(AtomicBool::new(false));

            let info_q = Arc::clone(&info_queue);
            let image_q = Arc::clone(&image_queue);
            let stop_clone = Arc::clone(&stop);

            let executor = thread::spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("auki-ros-adapter: tokio runtime: {e}");
                        return;
                    }
                };
                eprintln!("[auki-ros-adapter] worker thread up, runtime started");
                runtime.block_on(async move {
                    let info_q_inner = Arc::clone(&info_q);
                    let info_handler = async move {
                        let mut sub = info_sub;
                        let mut count = 0u32;
                        while let Some(msg) = sub.next().await {
                            count += 1;
                            if count <= 3 || count % 25 == 0 {
                                eprintln!(
                                    "[auki-ros-adapter] info #{count}: {}x{}",
                                    msg.width, msg.height
                                );
                            }
                            info_q_inner
                                .lock()
                                .unwrap()
                                .push_back(translate_camera_info(&msg));
                        }
                        eprintln!("[auki-ros-adapter] info stream ended after {count}");
                    };
                    let image_q_inner = Arc::clone(&image_q);
                    let image_handler = async move {
                        let mut sub = image_sub;
                        let mut count = 0u32;
                        while let Some(msg) = sub.next().await {
                            count += 1;
                            if count <= 3 || count % 20 == 0 {
                                eprintln!(
                                    "[auki-ros-adapter] image #{count}: {}x{} {} ({}B)",
                                    msg.width,
                                    msg.height,
                                    msg.encoding,
                                    msg.data.len()
                                );
                            }
                            image_q_inner
                                .lock()
                                .unwrap()
                                .push_back(translate_image(&msg));
                        }
                        eprintln!("[auki-ros-adapter] image stream ended after {count}");
                    };
                    let spinner = async move {
                        let mut spins = 0u32;
                        while !stop_clone.load(Ordering::Relaxed) {
                            // 100 ms is a good middle ground: responsive enough
                            // to drain a 25 Hz CameraInfo + 20 Hz Image stream
                            // without burning the CPU between callbacks.
                            node.spin_once(Duration::from_millis(100));
                            spins += 1;
                            if spins == 10 {
                                eprintln!("[auki-ros-adapter] 10 spin_onces done, still running");
                            }
                            tokio::task::yield_now().await;
                        }
                        eprintln!("[auki-ros-adapter] spinner stopping at {spins} spins");
                    };
                    tokio::join!(info_handler, image_handler, spinner);
                });
                eprintln!("[auki-ros-adapter] runtime exited");
            });

            Ok(Self {
                info_queue,
                image_queue,
                stop,
                executor: Some(executor),
            })
        }
    }

    impl CameraSubscriber for R2rCameraSubscriber {
        fn bootstrap(&mut self, timeout: Duration) -> Result<CameraInfoMsg, BootstrapError> {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                if let Some(info) = self.info_queue.lock().unwrap().pop_front() {
                    return Ok(info);
                }
                if std::time::Instant::now() >= deadline {
                    return Err(BootstrapError::Timeout);
                }
                thread::sleep(Duration::from_millis(50));
            }
        }

        fn poll(&mut self) -> Vec<SubscriptionEvent> {
            let mut events = Vec::new();
            // Strict cross-stream ordering would require a single timestamped
            // queue; for M1 we don't need it — the frame loop applies the
            // most-recent CameraInfo to subsequent frames regardless of
            // order. CameraInfo first so any pending intrinsics update lands
            // before the frames in the same poll() see it.
            for info in self.info_queue.lock().unwrap().drain(..) {
                events.push(SubscriptionEvent::CameraInfo(info));
            }
            for image in self.image_queue.lock().unwrap().drain(..) {
                events.push(SubscriptionEvent::Frame(image));
            }
            events
        }
    }

    impl Drop for R2rCameraSubscriber {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.executor.take() {
                let _ = handle.join();
            }
        }
    }

    fn translate_camera_info(msg: &r2r::sensor_msgs::msg::CameraInfo) -> CameraInfoMsg {
        let mut k = [0.0f64; 9];
        for (i, &v) in msg.k.iter().take(9).enumerate() {
            k[i] = v;
        }
        CameraInfoMsg {
            stamp: StampMsg {
                sec: msg.header.stamp.sec,
                nanosec: msg.header.stamp.nanosec,
            },
            width: msg.width,
            height: msg.height,
            distortion_model: msg.distortion_model.clone(),
            k,
            d: msg.d.clone(),
        }
    }

    fn translate_image(msg: &r2r::sensor_msgs::msg::Image) -> ImageMsg {
        ImageMsg {
            stamp: StampMsg {
                sec: msg.header.stamp.sec,
                nanosec: msg.header.stamp.nanosec,
            },
            width: msg.width,
            height: msg.height,
            encoding: msg.encoding.clone(),
            step: msg.step,
            data: msg.data.clone(),
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const K1_HEAD_LEFT_CAM_OPTICAL_HASH: &str = "e0d40e7b526e04f15f83f75897f53825";

    fn k1_bootstrap_camera_info() -> CameraInfoMsg {
        CameraInfoMsg {
            stamp: StampMsg {
                sec: 1_745_000_000,
                nanosec: 123_456_789,
            },
            width: 544,
            height: 488,
            distortion_model: "plumb_bob".into(),
            // K = [fx, 0, cx, 0, fy, cy, 0, 0, 1]
            k: [400.0, 0.0, 272.5, 0.0, 401.0, 244.5, 0.0, 0.0, 1.0],
            d: vec![-0.1, 0.05, 0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn stamp_to_ns_combines_seconds_and_nanoseconds() {
        let s = StampMsg {
            sec: 5,
            nanosec: 250_000_000,
        };
        assert_eq!(stamp_to_ns(s), 5_250_000_000);
    }

    #[test]
    fn stamp_to_ns_handles_zero_seconds() {
        let s = StampMsg { sec: 0, nanosec: 1 };
        assert_eq!(stamp_to_ns(s), 1);
    }

    #[test]
    fn stamp_to_ns_handles_max_representable_ros2_time() {
        // ROS2 header.stamp.sec is i32, so i32::MAX seconds (year 2038) is
        // the genuine ceiling. The conversion to i64 ns fits comfortably
        // (i64::MAX is ~292 years of nanoseconds).
        let s = StampMsg {
            sec: i32::MAX,
            nanosec: 999_999_999,
        };
        assert_eq!(
            stamp_to_ns(s),
            i32::MAX as i64 * 1_000_000_000 + 999_999_999
        );
    }

    #[test]
    fn dynamic_intrinsics_extracts_correct_indices() {
        let info = k1_bootstrap_camera_info();
        let di = dynamic_intrinsics_from(&info);
        assert_eq!(di.fx, 400.0); // k[0]
        assert_eq!(di.fy, 401.0); // k[4]
        assert_eq!(di.cx, 272.5); // k[2]
        assert_eq!(di.cy, 244.5); // k[5]
    }

    #[test]
    fn dynamic_intrinsics_passes_distortion_through_unchanged() {
        let info = k1_bootstrap_camera_info();
        let di = dynamic_intrinsics_from(&info);
        assert_eq!(di.distortion_coefficients, vec![-0.1, 0.05, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn dynamic_intrinsics_accepts_empty_distortion_for_none_model() {
        let info = CameraInfoMsg {
            d: vec![],
            ..k1_bootstrap_camera_info()
        };
        let di = dynamic_intrinsics_from(&info);
        assert!(di.distortion_coefficients.is_empty());
    }

    #[test]
    fn build_camera_registry_entry_matches_m1_example_hash() {
        // The auki-registry test suite locks the M1 example sensor entry's
        // hash. Driving build_camera_registry_entry with a CameraInfo
        // whose width/height/distortion_model match should produce the same
        // entry — same canonical bytes, same hash.
        let info = CameraInfoMsg {
            stamp: StampMsg { sec: 0, nanosec: 0 },
            width: 544,
            height: 488,
            distortion_model: "plumb_bob".into(),
            k: [0.0; 9], // unused by the registry side
            d: vec![],   // unused by the registry side
        };
        let entry = build_camera_registry_entry(
            "K1-AABBCCDDEEFF/head_left_cam",
            &info,
            &StaticCameraMetadata {
                pixel_format: "YUV_NV12",
                color_space: "BT.709",
                frame_rate_hz: 20,
                intrinsics_model: "pinhole",
                frame_id: "K1-AABBCCDDEEFF/head_left_cam_optical",
                frame_hash: K1_HEAD_LEFT_CAM_OPTICAL_HASH,
            },
        );
        // Recomputed when `frame_hash` was added to Camera and when
        // the camera registry tag was renamed.
        // Same hash as auki-registry's `sensor_entry_hash_is_locked`.
        assert_eq!(entry.hash(), "5559c9648e31eee2410b692fef393489");
    }

    #[test]
    fn build_sensor_log_entry_combines_info_and_image() {
        let info = k1_bootstrap_camera_info();
        let image = ImageMsg {
            stamp: StampMsg {
                sec: 100,
                nanosec: 500,
            },
            width: 544,
            height: 488,
            encoding: "nv12".into(),
            step: 544,
            data: vec![0xAA, 0xBB, 0xCC, 0xDD],
        };
        let (ts, entry) = build_sensor_log_entry(&info, &image);
        assert_eq!(ts, 100_000_000_500);
        assert_eq!(entry.frame, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(entry.dynamic_intrinsics.as_ref().unwrap().fx, 400.0);
    }

    // Prost round-trip lives in `auki-proto` (locked vector). The previous
    // ciborium round-trip test here covered the same surface for the old CBOR
    // shape and was deleted at Step 1 of the migration.

    #[test]
    fn mock_subscriber_returns_scripted_bootstrap_then_drains_events() {
        let mut sub = MockCameraSubscriber::new().with_bootstrap_ok(k1_bootstrap_camera_info());
        let info = sub.bootstrap(Duration::from_secs(5)).unwrap();
        assert_eq!(info.width, 544);

        sub.enqueue(SubscriptionEvent::Frame(ImageMsg {
            stamp: StampMsg { sec: 1, nanosec: 0 },
            width: 544,
            height: 488,
            encoding: "nv12".into(),
            step: 544,
            data: vec![1, 2, 3],
        }));
        sub.enqueue(SubscriptionEvent::CameraInfo(k1_bootstrap_camera_info()));

        let events = sub.poll();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], SubscriptionEvent::Frame(_)));
        assert!(matches!(events[1], SubscriptionEvent::CameraInfo(_)));

        // Subsequent poll is empty.
        assert!(sub.poll().is_empty());
    }

    #[test]
    fn mock_subscriber_bootstrap_timeout_when_unscripted() {
        let mut sub = MockCameraSubscriber::new();
        let err = sub.bootstrap(Duration::from_millis(1)).unwrap_err();
        assert!(matches!(err, BootstrapError::Timeout));
    }

    #[test]
    fn mock_subscriber_bootstrap_can_be_scripted_to_error() {
        let mut sub = MockCameraSubscriber::new()
            .with_bootstrap_err(BootstrapError::Transport("DDS down".into()));
        let err = sub.bootstrap(Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, BootstrapError::Transport(_)));
    }

    #[test]
    fn end_to_end_translation_from_subscription_to_log_entry() {
        let mut sub = MockCameraSubscriber::new().with_bootstrap_ok(k1_bootstrap_camera_info());
        let info = sub.bootstrap(Duration::from_secs(5)).unwrap();
        let registry_entry = build_camera_registry_entry(
            "K1-AABBCCDDEEFF/head_left_cam",
            &info,
            &StaticCameraMetadata {
                pixel_format: "YUV_NV12",
                color_space: "BT.709",
                frame_rate_hz: 20,
                intrinsics_model: "pinhole",
                frame_id: "K1-AABBCCDDEEFF/head_left_cam_optical",
                frame_hash: K1_HEAD_LEFT_CAM_OPTICAL_HASH,
            },
        );
        // Recomputed when `frame_hash` was added to Camera and when
        // the camera registry tag was renamed.
        assert_eq!(registry_entry.hash(), "5559c9648e31eee2410b692fef393489");

        // Now a frame arrives.
        sub.enqueue(SubscriptionEvent::Frame(ImageMsg {
            stamp: StampMsg {
                sec: 200,
                nanosec: 0,
            },
            width: 544,
            height: 488,
            encoding: "nv12".into(),
            step: 544,
            data: vec![0xFF; 16],
        }));

        let mut latest_info = info;
        let mut entries = Vec::new();
        for ev in sub.poll() {
            match ev {
                SubscriptionEvent::CameraInfo(new_info) => latest_info = new_info,
                SubscriptionEvent::Frame(img) => {
                    entries.push(build_sensor_log_entry(&latest_info, &img));
                }
            }
        }

        assert_eq!(entries.len(), 1);
        let (ts, entry) = &entries[0];
        assert_eq!(*ts, 200_000_000_000);
        assert_eq!(entry.frame.len(), 16);
        assert_eq!(entry.dynamic_intrinsics.as_ref().unwrap().fx, 400.0);
    }

    // ─── Point cloud tests ──────────────────────────────────────────────────

    /// Build a minimal XYZ-only PointCloud2 message with 2 points. Used as a
    /// fixture by several tests below; values are deliberately small and
    /// distinct so byte-level assertions read cleanly.
    fn xyz_pc2(num_points: u32) -> PointCloud2Msg {
        let mut data = Vec::new();
        for i in 0..num_points {
            data.extend_from_slice(&((i as f32) + 1.0).to_le_bytes()); // x
            data.extend_from_slice(&((i as f32) + 2.0).to_le_bytes()); // y
            data.extend_from_slice(&((i as f32) + 3.0).to_le_bytes()); // z
        }
        PointCloud2Msg {
            stamp: StampMsg {
                sec: 100,
                nanosec: 500,
            },
            height: 1,
            width: num_points,
            fields: vec![
                PointFieldMsg {
                    name: "x".into(),
                    offset: 0,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "y".into(),
                    offset: 4,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "z".into(),
                    offset: 8,
                    datatype: 7,
                    count: 1,
                },
            ],
            is_bigendian: false,
            point_step: 12,
            row_step: 12 * num_points,
            data,
            is_dense: true,
        }
    }

    #[test]
    fn build_point_cloud_registry_entry_matches_locked_hash() {
        let msg = PointCloud2Msg {
            stamp: StampMsg { sec: 0, nanosec: 0 },
            height: 1,
            width: 0, // unused for the registry side
            fields: vec![
                PointFieldMsg {
                    name: "x".into(),
                    offset: 0,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "y".into(),
                    offset: 4,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "z".into(),
                    offset: 8,
                    datatype: 7,
                    count: 1,
                },
            ],
            is_bigendian: false,
            point_step: 12,
            row_step: 0,
            data: vec![],
            is_dense: true,
        };
        let entry = build_point_cloud_registry_entry(
            "K1-AABBCCDDEEFF/head_depth_points",
            &msg,
            10,
            "K1-AABBCCDDEEFF/head_left_cam_optical",
            K1_HEAD_LEFT_CAM_OPTICAL_HASH,
        );
        // Locked: this is the same hash exercised by auki-registry's
        // `point_cloud_entry_hash_is_locked`. If the two diverge, one of the
        // crates drifted from the schema. Recomputed when `frame_hash`
        // was added to PointCloud.
        assert_eq!(entry.hash(), "2c480838a9be0b14608a8a0d72ee319f");
    }

    #[test]
    fn build_point_cloud_log_entry_extracts_timestamp_and_data() {
        let msg = xyz_pc2(2);
        let (ts, entry) = build_point_cloud_log_entry(&msg);
        assert_eq!(ts, 100_000_000_500);
        // 2 points × 12 bytes each, no normalization for xyz-only.
        assert_eq!(entry.data.len(), 24);
        assert_eq!(entry.data, msg.data);
    }

    #[test]
    fn rgb_field_normalizes_to_three_uint8s_and_repacks_bytes() {
        // One point with x,y,z (12 bytes) + rgb-as-float32 (4 bytes, with
        // little-endian memory layout [B, G, R, padding]).
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes()); // x
        data.extend_from_slice(&2.0f32.to_le_bytes()); // y
        data.extend_from_slice(&3.0f32.to_le_bytes()); // z
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0x00]); // [B=0xAA, G=0xBB, R=0xCC, pad]

        let msg = PointCloud2Msg {
            stamp: StampMsg { sec: 1, nanosec: 0 },
            height: 1,
            width: 1,
            fields: vec![
                PointFieldMsg {
                    name: "x".into(),
                    offset: 0,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "y".into(),
                    offset: 4,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "z".into(),
                    offset: 8,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "rgb".into(),
                    offset: 12,
                    datatype: 7,
                    count: 1,
                },
            ],
            is_bigendian: false,
            point_step: 16,
            row_step: 16,
            data,
            is_dense: true,
        };

        // Registry side: rgb expanded to r/g/b uint8 fields, point_step 16 → 15.
        let entry =
            build_point_cloud_registry_entry("test/cam", &msg, 30, "test/cam_optical", "fh");
        let auki_registry::SensorBody::PointCloud(pc) = &entry.body else {
            panic!("expected PointCloud variant");
        };
        let names: Vec<_> = pc.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["x", "y", "z", "r", "g", "b"]);
        let datatypes: Vec<_> = pc.fields.iter().map(|f| f.datatype).collect();
        assert_eq!(
            datatypes,
            vec![
                auki_registry::PointFieldDataType::Float32,
                auki_registry::PointFieldDataType::Float32,
                auki_registry::PointFieldDataType::Float32,
                auki_registry::PointFieldDataType::Uint8,
                auki_registry::PointFieldDataType::Uint8,
                auki_registry::PointFieldDataType::Uint8,
            ]
        );
        let offsets: Vec<_> = pc.fields.iter().map(|f| f.offset).collect();
        assert_eq!(offsets, vec![0, 4, 8, 12, 13, 14]);
        assert_eq!(pc.point_step, 15);

        // Log side: data repacked. The 4 RGB bytes [B=0xAA, G=0xBB, R=0xCC, pad]
        // become 3 RGB bytes [R=0xCC, G=0xBB, B=0xAA] at offsets 12,13,14.
        let (_, log) = build_point_cloud_log_entry(&msg);
        assert_eq!(log.data.len(), 15);
        assert_eq!(&log.data[0..12], &msg.data[0..12]); // xyz pass-through
        assert_eq!(log.data[12], 0xCC); // R
        assert_eq!(log.data[13], 0xBB); // G
        assert_eq!(log.data[14], 0xAA); // B
    }

    #[test]
    fn rgba_field_normalizes_to_four_uint8s_with_alpha_preserved() {
        // [B=0x11, G=0x22, R=0x33, A=0x44]
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes()); // x
        data.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);

        let msg = PointCloud2Msg {
            stamp: StampMsg { sec: 0, nanosec: 0 },
            height: 1,
            width: 1,
            fields: vec![
                PointFieldMsg {
                    name: "x".into(),
                    offset: 0,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "rgba".into(),
                    offset: 4,
                    datatype: 7,
                    count: 1,
                },
            ],
            is_bigendian: false,
            point_step: 8,
            row_step: 8,
            data,
            is_dense: true,
        };

        let entry =
            build_point_cloud_registry_entry("test/cam", &msg, 30, "test/cam_optical", "fh");
        let auki_registry::SensorBody::PointCloud(pc) = &entry.body else {
            panic!("expected PointCloud variant");
        };
        let names: Vec<_> = pc.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["x", "r", "g", "b", "a"]);
        // point_step unchanged: 8 (4 bytes x + 4 bytes rgba) → 8 (4 bytes x + 4 bytes rgba)
        assert_eq!(pc.point_step, 8);

        let (_, log) = build_point_cloud_log_entry(&msg);
        assert_eq!(log.data.len(), 8);
        assert_eq!(&log.data[0..4], &1.0f32.to_le_bytes()); // x pass-through
        assert_eq!(log.data[4], 0x33); // R
        assert_eq!(log.data[5], 0x22); // G
        assert_eq!(log.data[6], 0x11); // B
        assert_eq!(log.data[7], 0x44); // A preserved
    }

    #[test]
    fn non_rgb_fields_pass_through_unchanged() {
        // intensity (float32) and ring (uint16) — neither is rgb/rgba; both
        // should pass through with original datatypes.
        let mut data = Vec::new();
        data.extend_from_slice(&1.5f32.to_le_bytes()); // intensity
        data.extend_from_slice(&7u16.to_le_bytes()); // ring

        let msg = PointCloud2Msg {
            stamp: StampMsg { sec: 0, nanosec: 0 },
            height: 1,
            width: 1,
            fields: vec![
                PointFieldMsg {
                    name: "intensity".into(),
                    offset: 0,
                    datatype: 7,
                    count: 1,
                },
                PointFieldMsg {
                    name: "ring".into(),
                    offset: 4,
                    datatype: 4,
                    count: 1,
                },
            ],
            is_bigendian: false,
            point_step: 6,
            row_step: 6,
            data: data.clone(),
            is_dense: true,
        };

        let entry =
            build_point_cloud_registry_entry("test/lidar", &msg, 10, "test/lidar_frame", "fh");
        let auki_registry::SensorBody::PointCloud(pc) = &entry.body else {
            panic!("expected PointCloud variant");
        };
        assert_eq!(pc.fields.len(), 2);
        assert_eq!(pc.fields[0].name, "intensity");
        assert_eq!(
            pc.fields[0].datatype,
            auki_registry::PointFieldDataType::Float32
        );
        assert_eq!(pc.fields[1].name, "ring");
        assert_eq!(
            pc.fields[1].datatype,
            auki_registry::PointFieldDataType::Uint16
        );
        assert_eq!(pc.point_step, 6);

        let (_, log) = build_point_cloud_log_entry(&msg);
        assert_eq!(log.data, data);
    }

    #[test]
    fn mock_point_cloud_subscriber_bootstrap_then_drains() {
        let msg = xyz_pc2(2);
        let mut sub = MockPointCloudSubscriber::new().with_bootstrap_ok(msg.clone());
        let booted = sub.bootstrap(Duration::from_secs(1)).unwrap();
        assert_eq!(booted.width, 2);

        sub.enqueue(xyz_pc2(3));
        sub.enqueue(xyz_pc2(4));
        let drained = sub.poll();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].width, 3);
        assert_eq!(drained[1].width, 4);

        // Subsequent poll is empty.
        assert!(sub.poll().is_empty());
    }

    #[test]
    fn mock_point_cloud_subscriber_bootstrap_timeout_when_unscripted() {
        let mut sub = MockPointCloudSubscriber::new();
        let err = sub.bootstrap(Duration::from_millis(1)).unwrap_err();
        assert!(matches!(err, BootstrapError::Timeout));
    }
}

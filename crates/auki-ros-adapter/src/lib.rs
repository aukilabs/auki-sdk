//! ROS2 → Auki translation: `sensor_msgs/CameraInfo` + `sensor_msgs/Image`
//! into `SensorRegistryEntry` + `DynamicIntrinsics` + `SensorLogEntry`.
//!
//! Schema spec: [`docs/sensor-log.md`](../../../docs/sensor-log.md).
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

// ─── Output types written to the Sensor Log ─────────────────────────────────
//
// `SensorLogEntry` and `DynamicIntrinsics` previously lived here; they moved
// into `auki-registry` so that consumers of a sensor log (renderers, analysis
// tools) don't have to pull in a ROS adapter just to deserialize the payload.
// Re-exported here so existing call sites keep compiling.

pub use auki_registry::{DynamicIntrinsics, SensorLogEntry};

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
}

impl StaticCameraMetadata<'static> {
    /// The K1 head RGB camera's known-static configuration.
    pub const K1_HEAD_RGB: Self = Self {
        pixel_format: "YUV_NV12",
        color_space: "BT.709",
        frame_rate_hz: 20,
        intrinsics_model: "pinhole",
    };
}

/// Build a `SensorRegistryEntry` from a bootstrap `CameraInfo` + integrator-
/// supplied static metadata. Currently only emits `RgbCamera` bodies.
pub fn build_rgb_camera_registry_entry(
    sensor_id: impl Into<String>,
    info: &CameraInfoMsg,
    meta: &StaticCameraMetadata<'_>,
) -> auki_registry::SensorRegistryEntry {
    auki_registry::SensorRegistryEntry {
        sensor_id: sensor_id.into(),
        body: auki_registry::SensorBody::RgbCamera(auki_registry::RgbCamera {
            width: info.width,
            height: info.height,
            frame_rate_hz: meta.frame_rate_hz,
            pixel_format: meta.pixel_format.to_string(),
            color_space: meta.color_space.to_string(),
            intrinsics_model: meta.intrinsics_model.to_string(),
            distortion_model: info.distortion_model.clone(),
        }),
    }
}

/// Build a `SensorLogEntry` from the latest `CameraInfo` snapshot + an Image.
/// Returns `(timestamp_ns, entry)` ready for `auki_logs::Log::append`.
pub fn build_sensor_log_entry(
    info: &CameraInfoMsg,
    image: &ImageMsg,
) -> (i64, SensorLogEntry) {
    let timestamp_ns = stamp_to_ns(image.stamp);
    let entry = SensorLogEntry {
        dynamic_intrinsics: dynamic_intrinsics_from(info),
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
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicBool, Ordering};
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
                .subscribe::<r2r::sensor_msgs::msg::CameraInfo>(&topics.camera_info_topic, qos.clone())
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
                                    msg.width, msg.height, msg.encoding, msg.data.len()
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

    fn k1_bootstrap_camera_info() -> CameraInfoMsg {
        CameraInfoMsg {
            stamp: StampMsg { sec: 1_745_000_000, nanosec: 123_456_789 },
            width: 544,
            height: 488,
            distortion_model: "plumb_bob".into(),
            // K = [fx, 0, cx, 0, fy, cy, 0, 0, 1]
            k: [
                400.0, 0.0, 272.5,
                0.0, 401.0, 244.5,
                0.0, 0.0, 1.0,
            ],
            d: vec![-0.1, 0.05, 0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn stamp_to_ns_combines_seconds_and_nanoseconds() {
        let s = StampMsg { sec: 5, nanosec: 250_000_000 };
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
        let s = StampMsg { sec: i32::MAX, nanosec: 999_999_999 };
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
    fn build_rgb_camera_registry_entry_matches_m1_example_hash() {
        // The auki-registry test suite locks the M1 example sensor entry's
        // hash. Driving build_rgb_camera_registry_entry with a CameraInfo
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
        let entry = build_rgb_camera_registry_entry(
            "K1-AABBCCDDEEFF/head_left_cam",
            &info,
            &StaticCameraMetadata::K1_HEAD_RGB,
        );
        assert_eq!(entry.hash(), "e8cb3879fcfa7f716047aa0892b0c0c0");
    }

    #[test]
    fn build_sensor_log_entry_combines_info_and_image() {
        let info = k1_bootstrap_camera_info();
        let image = ImageMsg {
            stamp: StampMsg { sec: 100, nanosec: 500 },
            width: 544,
            height: 488,
            encoding: "nv12".into(),
            step: 544,
            data: vec![0xAA, 0xBB, 0xCC, 0xDD],
        };
        let (ts, entry) = build_sensor_log_entry(&info, &image);
        assert_eq!(ts, 100_000_000_500);
        assert_eq!(entry.frame, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(entry.dynamic_intrinsics.fx, 400.0);
    }

    #[test]
    fn sensor_log_entry_round_trips_through_cbor() {
        let entry = SensorLogEntry {
            dynamic_intrinsics: DynamicIntrinsics {
                fx: 400.5,
                fy: 401.5,
                cx: 272.0,
                cy: 244.0,
                distortion_coefficients: vec![1.0, 2.0, 3.0],
            },
            frame: vec![0; 1024],
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&entry, &mut buf).unwrap();
        let back: SensorLogEntry = ciborium::from_reader(&buf[..]).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn mock_subscriber_returns_scripted_bootstrap_then_drains_events() {
        let mut sub = MockCameraSubscriber::new()
            .with_bootstrap_ok(k1_bootstrap_camera_info());
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
        let mut sub = MockCameraSubscriber::new()
            .with_bootstrap_ok(k1_bootstrap_camera_info());
        let info = sub.bootstrap(Duration::from_secs(5)).unwrap();
        let registry_entry = build_rgb_camera_registry_entry(
            "K1-AABBCCDDEEFF/head_left_cam",
            &info,
            &StaticCameraMetadata::K1_HEAD_RGB,
        );
        assert_eq!(registry_entry.hash(), "e8cb3879fcfa7f716047aa0892b0c0c0");

        // Now a frame arrives.
        sub.enqueue(SubscriptionEvent::Frame(ImageMsg {
            stamp: StampMsg { sec: 200, nanosec: 0 },
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
        assert_eq!(entry.dynamic_intrinsics.fx, 400.0);
    }
}

//! QR Lab-backed typed QR detections carried by the Auki SDK's generic
//! [`DetectionFrame`](auki_datatypes::detection::DetectionFrame) envelope.
//!
//! This crate is deliberately limited to QR scanning. It does not determine
//! whether a QR payload names a Portal and does not call a portal service;
//! those product-specific responsibilities belong to a later consumer.

use std::sync::{Arc, Mutex};

use auki_components::{
    CameraPayloadContract, Component, ComponentBuildError, ComponentReference, ComponentRuntime,
    ComponentSpec, ConfiguredBufferInput, ConfiguredObservable, ConfiguredObservableSpec,
    ContractType, CursorStart, Exposure, InputPort, InvocationError, InvocationOrdering,
    ObservableContract, Observation, ObservationAccess, Operable, OperableContract,
    OutputReference, PayloadContract, ProductForm, ProductInputContract, ProductReference,
    RetainedProduct, StructuredPayloadContract, VideoFrame,
};
use auki_datatypes::detection::DetectionFrame;
use auki_registry::{Camera, DetectorBody, DetectorInput, Qr, RegistryRef, SensorBody};
use auki_session::{
    CameraDetector, CameraDetectorPackageError, CameraDetectorTask, CameraFrameSample,
    DetectionLogHandle, DetectorInstanceSpec, DetectorOutput, Peer, RegisteredCameraDetector,
    SensorLogHandle, Session, StreamingCameraDetectorTask,
};
pub use auki_session::{CameraStreamDescriptor, DetectionCadence};
use futures::Stream;
use qr_lab::image::{Gray8View, Rgb8View};
use qr_lab::{Scanner, ScannerConfig};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A running QR detector and its peer-consumable Detection Log declaration.
pub struct QrDetectorTask {
    task: CameraDetectorTask,
}

/// A running QR detector consuming a live asynchronous camera stream.
pub struct StreamingQrDetectorTask {
    task: StreamingCameraDetectorTask,
}

/// A content-addressed QR detector installation that can start independent
/// instances on compatible Camera Sensor Logs.
#[derive(Clone)]
pub struct RegisteredQrDetector {
    inner: RegisteredCameraDetector,
}

type QrProductResolver =
    Arc<dyn Fn(&ProductReference) -> Option<RetainedProduct<VideoFrame>> + Send + Sync>;

/// A live QR Detector Component bound directly to one typed camera Buffer
/// Product.
///
/// Unlike [`RegisteredQrDetector`], this adapter does not require a parallel
/// Session Sensor Log. Its current Product input binding is the authoritative
/// source identity exposed through the component Catalog.
pub struct QrDetectorComponent {
    component: Component,
    detections: ConfiguredObservable<QrDetections>,
    detector: Arc<Mutex<QrDetector>>,
    input: Arc<Mutex<ConfiguredBufferInput<VideoFrame>>>,
    set_product: Option<Operable<SetQrDetectorProduct, AppliedQrDetectorProduct>>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl QrDetectorComponent {
    /// The stable Detector Component whose Catalog entry exposes the exact
    /// current camera Product binding.
    pub fn component(&self) -> &Component {
        &self.component
    }

    /// Typed live QR results, with one observation emitted per processed
    /// camera observation (including observations containing zero QR codes).
    pub fn detections(&self) -> &ConfiguredObservable<QrDetections> {
        &self.detections
    }

    /// The currently selected camera Buffer Product binding.
    pub fn input(&self) -> std::sync::MutexGuard<'_, ConfiguredBufferInput<VideoFrame>> {
        self.input.lock().unwrap()
    }

    /// Typed control for selecting this Detector Component's camera Buffer
    /// Product. Present only when constructed with
    /// [`QrDetector::bind_product_operable`].
    pub fn set_product_operable(
        &self,
    ) -> Option<&Operable<SetQrDetectorProduct, AppliedQrDetectorProduct>> {
        self.set_product.as_ref()
    }

    /// Terminal input or frame-contract errors reported by the detector's
    /// Buffer reader.
    pub fn errors(&self) -> Vec<String> {
        self.errors.lock().unwrap().clone()
    }

    /// Move this stable Detector Component to a replacement camera Buffer
    /// Product without creating a second webcam identity or Sensor Log.
    pub fn replace_product(
        &self,
        product: &RetainedProduct<VideoFrame>,
    ) -> Result<(), QrDetectorError> {
        let camera = typed_camera_contract(product)?;
        if product.producer.clock_id != self.detections.manifest().clock_id {
            return Err(QrDetectorError::ProductClockMismatch {
                expected: self.detections.manifest().clock_id.clone(),
                actual: product.producer.clock_id.clone(),
            });
        }
        let input = qr_product_input(
            Arc::clone(&self.detector),
            self.detections.clone(),
            camera,
            Arc::clone(&self.errors),
        );
        let mut current = self.input.lock().unwrap();
        let replacement = self.component.replace_configured_buffer_input(
            &current,
            product,
            CursorStart::FromSequence(0),
            &input,
        )?;
        *current = replacement;
        Ok(())
    }
}

impl RegisteredQrDetector {
    pub fn registry_ref(&self) -> &RegistryRef {
        self.inner.registry_ref()
    }

    /// Start this detector on `input_log` using application-selected cadence
    /// and output-log lifecycle settings.
    pub fn start(
        &self,
        session: &Session,
        instance: DetectorInstanceSpec,
        input_log: &SensorLogHandle,
    ) -> Result<QrDetectorTask, QrDetectorError> {
        let input_sensor = session
            .sensor_registry_entry(&input_log.manifest.sensor)
            .ok_or(QrDetectorError::UnknownInputSensorReference)?;
        let SensorBody::Camera(camera) = &input_sensor.body else {
            return Err(QrDetectorError::IncompatibleSensorKind);
        };
        QrDetector::validate_camera(camera)?;
        let task = self.inner.start(session, instance, input_log)?;
        Ok(QrDetectorTask { task })
    }

    /// Start this detector on an asynchronous camera stream, such as Park's
    /// live remote `StreamSubscription` after mapping its entries into
    /// [`CameraFrameSample`] values.
    pub fn start_stream<S>(
        &self,
        session: &Session,
        instance: DetectorInstanceSpec,
        input: CameraStreamDescriptor,
        frames: S,
    ) -> Result<StreamingQrDetectorTask, QrDetectorError>
    where
        S: Stream<Item = std::result::Result<CameraFrameSample, String>> + Send + 'static,
    {
        let SensorBody::Camera(camera) = &input.sensor.body else {
            return Err(QrDetectorError::IncompatibleSensorKind);
        };
        QrDetector::validate_camera(camera)?;
        let task = self.inner.start_stream(session, instance, input, frames)?;
        Ok(StreamingQrDetectorTask { task })
    }
}

impl QrDetectorTask {
    pub fn detection_log(&self) -> &DetectionLogHandle {
        self.task.detection_log()
    }

    pub fn request_shutdown(&self) {
        self.task.request_shutdown();
    }

    pub fn shutdown(self) -> Result<(), QrDetectorError> {
        self.task.shutdown().map_err(Into::into)
    }
}

impl StreamingQrDetectorTask {
    pub fn detection_log(&self) -> &DetectionLogHandle {
        self.task.detection_log()
    }

    pub fn request_shutdown(&self) {
        self.task.request_shutdown();
    }

    /// Live frames replaced by fresher pending work while QR detection was
    /// busy. Frames skipped by cadence are not included.
    pub fn dropped_frames(&self) -> u64 {
        self.task.dropped_frames()
    }

    pub async fn shutdown(self) -> Result<(), QrDetectorError> {
        self.task.shutdown().await.map_err(Into::into)
    }
}

/// The `DetectionFrame.type` emitted by [`QrDetections::into_detection_frame`].
pub const QR_DETECTION_TYPE: &str = "qr";

/// Current on-wire schema version for [`QrDetections`].
pub const QR_DETECTION_SCHEMA_VERSION: u32 = 1;

/// Component datatype emitted by [`QrDetectorComponent`].
pub const QR_DETECTIONS_DATATYPE: &str = "auki.qr-detections";

/// Component schema emitted by [`QrDetectorComponent`].
pub const QR_DETECTIONS_SCHEMA: &str = "auki.qr-detections/v1";

/// Instruction selecting the exact camera Buffer Product consumed by a QR
/// Detector Component.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetQrDetectorProduct {
    pub product: ProductReference,
}

impl ContractType for SetQrDetectorProduct {
    const DATATYPE: &'static str = "auki.qr-detector.set-product/v1";
}

/// Result of applying a QR Detector Product selection.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppliedQrDetectorProduct {
    pub changed: bool,
    pub previous_product: ProductReference,
    pub product: ProductReference,
    pub producer: OutputReference,
    pub detector: ComponentReference,
    pub controller: ComponentReference,
}

impl ContractType for AppliedQrDetectorProduct {
    const DATATYPE: &'static str = "auki.qr-detector.applied-product/v1";
}

/// Coordinates of one QR module-region corner in source-frame pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelCorner {
    /// Horizontal pixel coordinate, increasing rightward.
    pub x: f64,
    /// Vertical pixel coordinate, increasing downward.
    pub y: f64,
}

/// A QR code successfully decoded in one source frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QrDetection {
    /// UTF-8 QR payload.
    pub payload: String,
    /// QR version in the inclusive range 1–40.
    pub version: u32,
    /// QR error-correction level (`L`, `M`, `Q`, `H`, or `?`).
    pub ecc: char,
    /// Whether the scanner decoded a mirrored image.
    pub mirrored: bool,
    /// Whether the QR polarity was light-on-dark.
    pub inverted: bool,
    /// Coarse source-frame corners in strict `TL, TR, BR, BL` order.
    pub corners_px: [PixelCorner; 4],
    /// Optional subpixel source-frame corners in the same order. Prefer these
    /// to [`Self::corners_px`] for pose estimation when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refined_corners_px: Option<[PixelCorner; 4]>,
    /// QR Lab recovery ladder stage that accepted this code.
    pub scanner_stage: u8,
}

/// All QR detections produced from one source frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QrDetections {
    /// Version of this payload schema.
    pub schema_version: u32,
    /// Decoded QR codes, in QR Lab acceptance order.
    pub codes: Vec<QrDetection>,
}

impl QrDetections {
    /// Serialize the versioned typed payload for `DetectionFrame.data`.
    pub fn encode(&self) -> Result<Vec<u8>, QrDetectorError> {
        serde_json::to_vec(self).map_err(QrDetectorError::Encode)
    }

    /// Decode a QR payload from `DetectionFrame.data`.
    pub fn decode(bytes: &[u8]) -> Result<Self, QrDetectorError> {
        let decoded: Self = serde_json::from_slice(bytes).map_err(QrDetectorError::Decode)?;
        if decoded.schema_version != QR_DETECTION_SCHEMA_VERSION {
            return Err(QrDetectorError::UnsupportedSchemaVersion(
                decoded.schema_version,
            ));
        }
        Ok(decoded)
    }

    /// Wrap these QR detections in the SDK's generic per-frame envelope.
    pub fn into_detection_frame(self, input_sensor_hash: impl Into<String>) -> DetectionFrame {
        DetectionFrame {
            data: self
                .encode()
                .expect("QrDetections generated by this crate serialize"),
            sensor_hash: input_sensor_hash.into(),
            r#type: QR_DETECTION_TYPE.into(),
        }
    }
}

impl ContractType for QrDetections {
    const DATATYPE: &'static str = QR_DETECTIONS_DATATYPE;
}

/// QR Lab scanner configuration used by [`QrDetector`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QrDetectorConfig {
    /// Underlying QR Lab scanner configuration.
    pub scanner: ScannerConfig,
}

impl QrDetectorConfig {
    /// Production-lean robust scanner with source-pixel corner refinement on.
    pub fn robust_fast() -> Self {
        let mut scanner = ScannerConfig::robust_fast();
        scanner.options.refine = true;
        Self { scanner }
    }
}

impl Default for QrDetectorConfig {
    fn default() -> Self {
        Self::robust_fast()
    }
}

/// Stateful QR detector. Retain one instance when processing a video stream.
pub struct QrDetector {
    config: QrDetectorConfig,
    scanner: Scanner,
}

impl QrDetector {
    pub fn new(config: QrDetectorConfig) -> Self {
        Self {
            config,
            scanner: Scanner::new(config.scanner),
        }
    }

    /// Construct and expose a typed Detector Component whose input is the
    /// supplied camera Buffer Product.
    ///
    /// The detector follows the Product from its first retained sequence and
    /// publishes one typed [`QrDetections`] observation for every processed
    /// camera frame. No Sensor, Sensor Log, or application-side frame fanout is
    /// introduced by this path.
    pub fn bind_product(
        self,
        runtime: &ComponentRuntime,
        component_id: impl Into<String>,
        output_id: impl Into<String>,
        product: &RetainedProduct<VideoFrame>,
    ) -> Result<QrDetectorComponent, QrDetectorError> {
        self.bind_product_inner(runtime, component_id, output_id, product, None)
    }

    /// Construct and expose a typed Detector Component whose camera Product
    /// input can be reconfigured through a declared `set_product` Operable.
    ///
    /// `resolve_product` is the host's typed Product registry. The Operable
    /// resolves an exact [`ProductReference`], validates its immutable camera
    /// contract, and atomically replaces the Component's configured `frames`
    /// input. Only the supplied controller Component is authorized.
    pub fn bind_product_operable(
        self,
        runtime: &ComponentRuntime,
        component_id: impl Into<String>,
        output_id: impl Into<String>,
        product: &RetainedProduct<VideoFrame>,
        controller: ComponentReference,
        resolve_product: impl Fn(&ProductReference) -> Option<RetainedProduct<VideoFrame>>
        + Send
        + Sync
        + 'static,
    ) -> Result<QrDetectorComponent, QrDetectorError> {
        self.bind_product_inner(
            runtime,
            component_id,
            output_id,
            product,
            Some((controller, Arc::new(resolve_product))),
        )
    }

    fn bind_product_inner(
        self,
        runtime: &ComponentRuntime,
        component_id: impl Into<String>,
        output_id: impl Into<String>,
        product: &RetainedProduct<VideoFrame>,
        control: Option<(ComponentReference, QrProductResolver)>,
    ) -> Result<QrDetectorComponent, QrDetectorError> {
        let camera = typed_camera_contract(product)?;
        let mut component_spec = ComponentSpec::new(component_id)
            .product_input(ProductInputContract {
                name: "frames".into(),
                form: ProductForm::Buffer,
                datatype: VideoFrame::DATATYPE.into(),
                schema: "auki.video-frame/v1".into(),
                exposure: Exposure::Cluster,
            })
            .observable(ObservableContract {
                name: "detections".into(),
                datatype: QrDetections::DATATYPE.into(),
                schema: QR_DETECTIONS_SCHEMA.into(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            });
        if control.is_some() {
            component_spec = component_spec.operable(OperableContract {
                name: "set_product".into(),
                instruction: SetQrDetectorProduct::DATATYPE.into(),
                result: AppliedQrDetectorProduct::DATATYPE.into(),
                exposure: Exposure::Cluster,
            });
        }
        let component = runtime.component(component_spec)?;
        let mut output_spec = ConfiguredObservableSpec::new(
            "detections",
            output_id,
            product.producer.clock_id.clone(),
            PayloadContract::Structured(StructuredPayloadContract {
                modality: "detection".into(),
                datatype: QrDetections::DATATYPE.into(),
                schema: QR_DETECTIONS_SCHEMA.into(),
                observes: "QR codes in the configured camera Buffer Product".into(),
                unit: None,
            }),
        );
        output_spec.spatial_frame_id = product.producer.spatial_frame_id.clone();
        let detections = component.configured_observable::<QrDetections>(output_spec)?;
        let detector = Arc::new(Mutex::new(self));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let input_port = qr_product_input(
            Arc::clone(&detector),
            detections.clone(),
            camera,
            Arc::clone(&errors),
        );
        let input = Arc::new(Mutex::new(component.configured_buffer_input(
            "frames",
            product,
            CursorStart::FromSequence(0),
            &input_port,
        )?));
        let set_product = if let Some((controller, resolve_product)) = control {
            let authorizer = controller.clone();
            let result_controller = controller;
            let result_detector = component.reference().clone();
            let operation_component = component.clone();
            let operation_input = Arc::clone(&input);
            let operation_detector = Arc::clone(&detector);
            let operation_detections = detections.clone();
            let operation_errors = Arc::clone(&errors);
            Some(
                component.operable_ordered::<SetQrDetectorProduct, AppliedQrDetectorProduct>(
                    "set_product",
                    InvocationOrdering::SerialInAcceptanceOrder,
                    move |context| {
                        context.caller_peer_id == authorizer.peer_id
                            && context.caller_component_id == authorizer.component_id
                    },
                    move |_context, instruction| {
                        let product = resolve_product(&instruction.product).ok_or_else(|| {
                            InvocationError::Rejected(format!(
                                "{}.{} is not an available compatible camera Buffer Product",
                                instruction.product.peer_id, instruction.product.product_id
                            ))
                        })?;
                        let camera = typed_camera_contract(&product)
                            .map_err(|error| InvocationError::Rejected(error.to_string()))?;
                        if product.producer.clock_id != operation_detections.manifest().clock_id {
                            return Err(InvocationError::Rejected(
                                QrDetectorError::ProductClockMismatch {
                                    expected: operation_detections.manifest().clock_id.clone(),
                                    actual: product.producer.clock_id.clone(),
                                }
                                .to_string(),
                            ));
                        }
                        let mut current = operation_input.lock().unwrap();
                        let previous_product = current.manifest().product.reference();
                        let changed = previous_product != instruction.product;
                        if changed {
                            let replacement_port = qr_product_input(
                                Arc::clone(&operation_detector),
                                operation_detections.clone(),
                                camera,
                                Arc::clone(&operation_errors),
                            );
                            let replacement = operation_component
                                .replace_configured_buffer_input(
                                    &current,
                                    &product,
                                    CursorStart::FromSequence(0),
                                    &replacement_port,
                                )
                                .map_err(|error| InvocationError::Rejected(error.to_string()))?;
                            *current = replacement;
                        }
                        Ok(AppliedQrDetectorProduct {
                            changed,
                            previous_product,
                            product: instruction.product,
                            producer: product.manifest.producer,
                            detector: result_detector.clone(),
                            controller: result_controller.clone(),
                        })
                    },
                )?,
            )
        } else {
            None
        };
        component.expose()?;
        Ok(QrDetectorComponent {
            component,
            detections,
            detector,
            input,
            set_product,
            errors,
        })
    }

    /// Validate that a Camera Registry contract can be consumed by this
    /// detector. NV12 is scanned through its Y plane; JPEG is decoded to RGB8
    /// before QR Lab converts to luminance.
    pub fn validate_camera(camera: &Camera) -> Result<(), QrDetectorError> {
        if camera.image_encoding == "jpeg" {
            return Ok(());
        }
        if camera.image_encoding != "raw"
            || !matches!(camera.pixel_format.as_str(), "luma8" | "rgb8" | "YUV_NV12")
        {
            return Err(QrDetectorError::IncompatibleCamera {
                image_encoding: camera.image_encoding.clone(),
                pixel_format: camera.pixel_format.clone(),
            });
        }
        let bytes_per_pixel = if camera.pixel_format == "rgb8" { 3 } else { 1 };
        let minimum_row_bytes = camera.width.saturating_mul(bytes_per_pixel);
        if camera.row_stride_bytes < minimum_row_bytes {
            return Err(QrDetectorError::InvalidCameraStride {
                minimum_row_bytes,
                row_stride_bytes: camera.row_stride_bytes,
            });
        }
        Ok(())
    }

    /// Register this detector's immutable SDK identity on `peer`.
    ///
    /// The detector emits the generic `qr` capability. A portal-specific
    /// consumer may subsequently interpret selected QR payloads as portals.
    pub fn register(
        &self,
        peer: &Peer,
        detector_id: &str,
    ) -> Result<RegisteredQrDetector, QrDetectorError> {
        let config = self.config;
        let inner = RegisteredCameraDetector::register(
            peer,
            detector_id,
            DetectorBody::Qr(Qr {}),
            vec![
                DetectorInput {
                    sensor_kind: "camera".into(),
                    sensor_type: None,
                    image_encoding: Some("raw".into()),
                    pixel_format: Some("luma8".into()),
                },
                DetectorInput {
                    sensor_kind: "camera".into(),
                    sensor_type: None,
                    image_encoding: Some("raw".into()),
                    pixel_format: Some("rgb8".into()),
                },
                DetectorInput {
                    sensor_kind: "camera".into(),
                    sensor_type: None,
                    image_encoding: Some("raw".into()),
                    pixel_format: Some("YUV_NV12".into()),
                },
                DetectorInput {
                    sensor_kind: "camera".into(),
                    sensor_type: None,
                    image_encoding: Some("jpeg".into()),
                    pixel_format: None,
                },
            ],
            vec![QR_DETECTION_TYPE.into()],
            move || QrDetector::new(config),
        )?;
        Ok(RegisteredQrDetector { inner })
    }

    /// Scan an 8-bit grayscale source frame.
    ///
    /// `stride` is in bytes and may exceed `width`, allowing direct use of a
    /// padded camera Y plane. Returned corners are always in this source
    /// frame's pixels, never in an internal downscaled working image.
    pub fn detect_luma(
        &mut self,
        pixels: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<QrDetections, QrDetectorError> {
        let frame = Gray8View::new(pixels, width, height, stride)
            .map_err(|error| QrDetectorError::InvalidFrame(error.to_string()))?;
        Ok(qr_detections(self.scanner.scan(&frame)))
    }

    /// Scan a packed RGB8 source frame after converting it to luminance.
    ///
    /// `stride` is in bytes and may exceed `width * 3`. QR Lab owns the
    /// BT.601 luminance conversion and its reusable scratch storage. Returned
    /// corners remain in source-frame pixels.
    pub fn detect_rgb8(
        &mut self,
        pixels: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<QrDetections, QrDetectorError> {
        let frame = Rgb8View::new(pixels, width, height, stride)
            .map_err(|error| QrDetectorError::InvalidFrame(error.to_string()))?;
        Ok(qr_detections(self.scanner.scan_rgb8(&frame)))
    }

    /// Scan the full-resolution Y plane of an NV12 frame.
    ///
    /// NV12 stores `height` padded luminance rows followed by half as many
    /// interleaved UV rows using the same stride. The chroma plane is not
    /// needed for QR detection, but its presence is validated so truncated
    /// payloads cannot masquerade as valid NV12 frames.
    pub fn detect_nv12(
        &mut self,
        pixels: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<QrDetections, QrDetectorError> {
        let y_plane_len = stride.checked_mul(height).ok_or_else(|| {
            QrDetectorError::InvalidFrame("NV12 Y-plane dimensions overflow".into())
        })?;
        let chroma_rows = height.div_ceil(2);
        let frame_rows = height.checked_add(chroma_rows).ok_or_else(|| {
            QrDetectorError::InvalidFrame("NV12 frame dimensions overflow".into())
        })?;
        let required_len = stride.checked_mul(frame_rows).ok_or_else(|| {
            QrDetectorError::InvalidFrame("NV12 frame dimensions overflow".into())
        })?;
        if pixels.len() < required_len {
            return Err(QrDetectorError::InvalidFrame(format!(
                "NV12 frame has {} bytes, requires at least {required_len}",
                pixels.len()
            )));
        }
        self.detect_luma(&pixels[..y_plane_len], width, height, stride)
    }

    /// Decode and scan one JPEG frame.
    ///
    /// Decoded dimensions must match the immutable Camera Registry contract.
    pub fn detect_jpeg(
        &mut self,
        jpeg: &[u8],
        expected_width: usize,
        expected_height: usize,
    ) -> Result<QrDetections, QrDetectorError> {
        let expected_width_u32 = u32::try_from(expected_width)
            .map_err(|_| QrDetectorError::InvalidFrame("JPEG width exceeds u32".into()))?;
        let expected_height_u32 = u32::try_from(expected_height)
            .map_err(|_| QrDetectorError::InvalidFrame("JPEG height exceeds u32".into()))?;
        let dimensions =
            image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg)
                .into_dimensions()
                .map_err(QrDetectorError::JpegDecode)?;
        if dimensions != (expected_width_u32, expected_height_u32) {
            return Err(QrDetectorError::JpegDimensionMismatch {
                expected_width,
                expected_height,
                actual_width: dimensions.0 as usize,
                actual_height: dimensions.1 as usize,
            });
        }

        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(expected_width_u32);
        limits.max_image_height = Some(expected_height_u32);
        reader.limits(limits);
        let decoded = reader.decode().map_err(QrDetectorError::JpegDecode)?;
        let rgb = decoded.into_rgb8();
        let width = rgb.width() as usize;
        let height = rgb.height() as usize;
        self.detect_rgb8(rgb.as_raw(), width, height, width * 3)
    }
}

fn typed_camera_contract(
    product: &RetainedProduct<VideoFrame>,
) -> Result<CameraPayloadContract, QrDetectorError> {
    let PayloadContract::Camera(camera) = &product.producer.payload else {
        return Err(QrDetectorError::IncompatibleProductPayload);
    };
    if camera.datatype != VideoFrame::DATATYPE
        || camera.schema != "auki.video-frame/v1"
        || !matches!(camera.encoding.as_str(), "jpeg" | "rgb8")
        || camera.width == 0
        || camera.height == 0
    {
        return Err(QrDetectorError::IncompatibleProductCamera {
            datatype: camera.datatype.clone(),
            schema: camera.schema.clone(),
            encoding: camera.encoding.clone(),
            width: camera.width,
            height: camera.height,
        });
    }
    Ok(camera.clone())
}

fn qr_product_input(
    detector: Arc<Mutex<QrDetector>>,
    detections: ConfiguredObservable<QrDetections>,
    camera: CameraPayloadContract,
    errors: Arc<Mutex<Vec<String>>>,
) -> InputPort<Observation<VideoFrame>> {
    InputPort::<Observation<VideoFrame>>::try_new("qr-detector.frames", move |entry| {
        let observation = &entry.payload;
        let frame = observation.payload.as_ref();
        let result = if frame.width != camera.width
            || frame.height != camera.height
            || frame.encoding != camera.encoding
        {
            Err(QrDetectorError::ProductFrameContractMismatch {
                expected_width: camera.width,
                expected_height: camera.height,
                expected_encoding: camera.encoding.clone(),
                actual_width: frame.width,
                actual_height: frame.height,
                actual_encoding: frame.encoding.clone(),
            })
        } else {
            let mut detector = detector.lock().unwrap();
            match camera.encoding.as_str() {
                "jpeg" => detector.detect_jpeg(
                    &frame.bytes,
                    camera.width as usize,
                    camera.height as usize,
                ),
                "rgb8" => detector.detect_rgb8(
                    &frame.bytes,
                    camera.width as usize,
                    camera.height as usize,
                    camera.width as usize * 3,
                ),
                _ => unreachable!("typed_camera_contract accepts only JPEG and RGB8"),
            }
        };
        match result {
            Ok(result) => detections
                .publish(observation.timestamp_ns, Arc::new(result))
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Err(error) => {
                let error = error.to_string();
                errors.lock().unwrap().push(error.clone());
                Err(error)
            }
        }
    })
}

impl CameraDetector for QrDetector {
    fn process(
        &mut self,
        frame: &auki_datatypes::camera::CameraFrame,
        camera: &Camera,
    ) -> std::result::Result<Vec<DetectorOutput>, String> {
        let detections = match (camera.image_encoding.as_str(), camera.pixel_format.as_str()) {
            ("raw", "luma8") => self.detect_luma(
                &frame.frame,
                camera.width as usize,
                camera.height as usize,
                camera.row_stride_bytes as usize,
            ),
            ("raw", "YUV_NV12") => self.detect_nv12(
                &frame.frame,
                camera.width as usize,
                camera.height as usize,
                camera.row_stride_bytes as usize,
            ),
            ("raw", "rgb8") => self.detect_rgb8(
                &frame.frame,
                camera.width as usize,
                camera.height as usize,
                camera.row_stride_bytes as usize,
            ),
            ("jpeg", _) => {
                self.detect_jpeg(&frame.frame, camera.width as usize, camera.height as usize)
            }
            _ => Err(QrDetectorError::IncompatibleCamera {
                image_encoding: camera.image_encoding.clone(),
                pixel_format: camera.pixel_format.clone(),
            }),
        }
        .map_err(|error| error.to_string())?;
        Ok(vec![DetectorOutput {
            r#type: QR_DETECTION_TYPE.into(),
            data: detections.encode().map_err(|error| error.to_string())?,
        }])
    }
}

impl Default for QrDetector {
    fn default() -> Self {
        Self::new(QrDetectorConfig::default())
    }
}

fn pixel_corner([x, y]: [f64; 2]) -> PixelCorner {
    PixelCorner { x, y }
}

fn qr_detections(result: qr_lab::RobustDetections) -> QrDetections {
    QrDetections {
        schema_version: QR_DETECTION_SCHEMA_VERSION,
        codes: result
            .codes
            .into_iter()
            .map(|code| QrDetection {
                payload: code.code.payload,
                version: code.code.version,
                ecc: code.code.ecc,
                mirrored: code.code.mirrored,
                inverted: code.code.inverted,
                corners_px: code.corners_source.map(pixel_corner),
                refined_corners_px: code
                    .refined_corners_source
                    .map(|corners| corners.map(pixel_corner)),
                scanner_stage: code.stage,
            })
            .collect(),
    }
}

/// Errors raised while scanning, encoding, or registering QR detections.
#[derive(Debug, Error)]
pub enum QrDetectorError {
    /// Generic detector package registration, startup, or execution failed.
    #[error("detector package: {0}")]
    Package(#[from] CameraDetectorPackageError),
    /// Typed Component or Product binding construction failed.
    #[error("typed dataflow: {0}")]
    Component(Box<ComponentBuildError>),
    /// The selected typed Product is not produced by a Camera payload.
    #[error("QR Detector Component input must be a camera Buffer Product")]
    IncompatibleProductPayload,
    /// The selected typed Camera Product has a contract this adapter cannot scan.
    #[error(
        "QR Detector Component requires video_frame/auki.video-frame/v1 with jpeg or rgb8 data and positive dimensions, got {datatype}/{schema} {encoding} {width}x{height}"
    )]
    IncompatibleProductCamera {
        datatype: String,
        schema: String,
        encoding: String,
        width: u32,
        height: u32,
    },
    /// A retained frame violates its producing Output's immutable Camera contract.
    #[error(
        "camera Product frame contract mismatch: expected {expected_encoding} {expected_width}x{expected_height}, got {actual_encoding} {actual_width}x{actual_height}"
    )]
    ProductFrameContractMismatch {
        expected_width: u32,
        expected_height: u32,
        expected_encoding: String,
        actual_width: u32,
        actual_height: u32,
        actual_encoding: String,
    },
    /// A replacement Product uses a different clock from the detector Output.
    #[error("replacement camera Product clock is {actual}, expected {expected}")]
    ProductClockMismatch { expected: String, actual: String },
    /// The camera registry does not expose a supported raw or JPEG image.
    #[error(
        "QR detector requires raw/luma8, raw/rgb8, raw/YUV_NV12, or jpeg camera frames, got {image_encoding}/{pixel_format}"
    )]
    IncompatibleCamera {
        /// Camera registry image encoding.
        image_encoding: String,
        /// Camera registry pixel format.
        pixel_format: String,
    },
    /// The row stride cannot contain the declared pixels.
    #[error(
        "camera row stride {row_stride_bytes} is smaller than required row size {minimum_row_bytes}"
    )]
    InvalidCameraStride {
        /// Minimum bytes required for one row of the declared pixel format.
        minimum_row_bytes: u32,
        /// Declared bytes between image rows.
        row_stride_bytes: u32,
    },
    /// The supplied camera plane is not a valid QR Lab grayscale view.
    #[error("invalid grayscale frame: {0}")]
    InvalidFrame(String),
    /// A compressed frame is not a valid supported JPEG image.
    #[error("could not decode JPEG frame: {0}")]
    JpegDecode(image::ImageError),
    /// JPEG headers disagree with the immutable Camera Registry dimensions.
    #[error(
        "JPEG dimensions {actual_width}x{actual_height} do not match Camera Registry dimensions {expected_width}x{expected_height}"
    )]
    JpegDimensionMismatch {
        expected_width: usize,
        expected_height: usize,
        actual_width: usize,
        actual_height: usize,
    },
    /// Typed payload serialization failed.
    #[error("could not encode QR detection payload: {0}")]
    Encode(serde_json::Error),
    /// Typed payload deserialization failed.
    #[error("could not decode QR detection payload: {0}")]
    Decode(serde_json::Error),
    /// The payload uses an unknown schema version.
    #[error("unsupported QR detection schema version: {0}")]
    UnsupportedSchemaVersion(u32),
    /// The selected registry entry is not a camera stream.
    #[error("QR detector input must be a camera sensor")]
    IncompatibleSensorKind,
    /// The Sensor Log's exact Sensor Registry entry cannot be resolved locally.
    #[error("input Sensor Registry reference does not exist in this session's peer")]
    UnknownInputSensorReference,
}

impl From<ComponentBuildError> for QrDetectorError {
    fn from(error: ComponentBuildError) -> Self {
        Self::Component(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_datatypes::camera::CameraFrame;
    use auki_registry::{LogRef, SensorRegistryEntry};
    use auki_session::{FrameDef, HeadSpec, SensorLogSpec};
    use qrcode::{Color, EcLevel, QrCode};
    use std::thread;
    use std::time::Duration;

    fn luma_camera() -> Camera {
        Camera {
            r#type: "mono".into(),
            width: 32,
            height: 32,
            frame_rate_hz: 30,
            image_encoding: "raw".into(),
            pixel_format: "luma8".into(),
            row_stride_bytes: 32,
            color_space: "linear".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "none".into(),
            calibration: None,
            frame: RegistryRef {
                peer_id: "robot".into(),
                id: "camera_optical".into(),
                hash: "frame-hash".into(),
            },
        }
    }

    fn sample() -> QrDetections {
        QrDetections {
            schema_version: QR_DETECTION_SCHEMA_VERSION,
            codes: vec![QrDetection {
                payload: "auki://portal/01HXYZ".into(),
                version: 4,
                ecc: 'Q',
                mirrored: false,
                inverted: false,
                corners_px: [
                    PixelCorner { x: 1.0, y: 2.0 },
                    PixelCorner { x: 3.0, y: 2.0 },
                    PixelCorner { x: 3.0, y: 4.0 },
                    PixelCorner { x: 1.0, y: 4.0 },
                ],
                refined_corners_px: None,
                scanner_stage: 0,
            }],
        }
    }

    fn rendered_qr_rgb8(payload: &str) -> (Vec<u8>, usize, usize, usize) {
        let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M).unwrap();
        let quiet = 4usize;
        let scale = 8usize;
        let side = (code.width() + quiet * 2) * scale;
        let stride = side * 3 + 7;
        let mut pixels = vec![255u8; stride * side];
        for module_y in 0..code.width() {
            for module_x in 0..code.width() {
                if code[(module_x, module_y)] != Color::Dark {
                    continue;
                }
                let x0 = (module_x + quiet) * scale;
                let y0 = (module_y + quiet) * scale;
                for y in y0..y0 + scale {
                    for x in x0..x0 + scale {
                        pixels[y * stride + x * 3..y * stride + x * 3 + 3].fill(0);
                    }
                }
            }
        }
        (pixels, side, side, stride)
    }

    fn rendered_qr_jpeg(payload: &str) -> (Vec<u8>, usize, usize) {
        let (padded, width, height, stride) = rendered_qr_rgb8(payload);
        let mut rgb = vec![0; width * height * 3];
        for y in 0..height {
            rgb[y * width * 3..(y + 1) * width * 3]
                .copy_from_slice(&padded[y * stride..y * stride + width * 3]);
        }
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
            .encode(
                &rgb,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        (jpeg, width, height)
    }

    fn typed_jpeg_payload(width: u32, height: u32, frame_rate_hz: u32) -> PayloadContract {
        PayloadContract::Camera(CameraPayloadContract {
            datatype: VideoFrame::DATATYPE.into(),
            schema: "auki.video-frame/v1".into(),
            encoding: "jpeg".into(),
            width,
            height,
            nominal_frame_rate_hz: Some(frame_rate_hz),
            observes: "test camera".into(),
        })
    }

    #[test]
    fn typed_component_consumes_and_rebinds_camera_buffer_products() {
        use auki_components::{
            BufferLimits, ConfiguredObservableSpec, ObservableContract, ObservationAccess,
        };

        let runtime = ComponentRuntime::new("peer-a");
        let camera = runtime
            .component(
                ComponentSpec::new("webcam-capture").observable(ObservableContract {
                    name: "frames".into(),
                    datatype: VideoFrame::DATATYPE.into(),
                    schema: "auki.video-frame/v1".into(),
                    access: vec![ObservationAccess::FollowNew],
                    exposure: Exposure::Cluster,
                }),
            )
            .unwrap();
        let (first_jpeg, width, height) = rendered_qr_jpeg("auki://typed-product/alpha");
        let first_output = camera
            .configured_observable::<VideoFrame>(ConfiguredObservableSpec::new(
                "frames",
                "webcam-jpeg-output-1",
                "monotonic-ns",
                typed_jpeg_payload(width as u32, height as u32, 10),
            ))
            .unwrap();
        camera.expose().unwrap();
        let first_capture = runtime
            .capture_buffer(
                "webcam-frame-history",
                &first_output,
                BufferLimits::entries(32),
                |frame| frame.bytes.len(),
            )
            .unwrap();
        let detector = QrDetector::default()
            .bind_product(
                &runtime,
                "qr-detector",
                "qr-detections-output-1",
                &first_capture.product(),
            )
            .unwrap();
        let detection_capture = runtime
            .capture_buffer(
                "webcam-qr-detections",
                detector.detections(),
                BufferLimits::entries(32),
                |_| std::mem::size_of::<QrDetections>(),
            )
            .unwrap();

        first_output
            .publish(
                1,
                Arc::new(VideoFrame {
                    width: width as u32,
                    height: height as u32,
                    encoding: "jpeg".into(),
                    bytes: Arc::from(first_jpeg),
                }),
            )
            .unwrap();
        for _ in 0..100 {
            if detection_capture.product().buffer().range().entries == 1 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let first_detection = detection_capture
            .product()
            .latest_existing()
            .unwrap()
            .unwrap();
        assert!(
            first_detection
                .payload
                .codes
                .iter()
                .any(|code| code.payload == "auki://typed-product/alpha")
        );
        assert_eq!(
            runtime
                .catalog()
                .component("qr-detector")
                .unwrap()
                .current_product_inputs["frames"]
                .manifest
                .product
                .product_id,
            "webcam-frame-history"
        );

        let replacement = camera
            .replace_configured_observable::<VideoFrame>(
                &first_output,
                ConfiguredObservableSpec::new(
                    "frames",
                    "webcam-jpeg-output-2",
                    "monotonic-ns",
                    typed_jpeg_payload(width as u32, height as u32, 5),
                ),
                2,
            )
            .unwrap();
        let second_capture = runtime
            .capture_buffer(
                "webcam-frame-history-2",
                &replacement.replacement,
                BufferLimits::entries(32),
                |frame| frame.bytes.len(),
            )
            .unwrap();
        detector.replace_product(&second_capture.product()).unwrap();
        let (second_jpeg, _, _) = rendered_qr_jpeg("auki://typed-product/omega");
        replacement
            .replacement
            .publish(
                3,
                Arc::new(VideoFrame {
                    width: width as u32,
                    height: height as u32,
                    encoding: "jpeg".into(),
                    bytes: Arc::from(second_jpeg),
                }),
            )
            .unwrap();
        for _ in 0..100 {
            if detection_capture.product().buffer().range().entries == 2 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            detection_capture.product().buffer().range().entries,
            2,
            "replacement input stats: {:?}, errors: {:?}",
            detector.input().stats(),
            detector.errors()
        );
        let second_detection = detection_capture
            .product()
            .latest_existing()
            .unwrap()
            .unwrap();
        assert!(
            second_detection
                .payload
                .codes
                .iter()
                .any(|code| code.payload == "auki://typed-product/omega"),
            "unexpected second detection: {:?}",
            second_detection.payload
        );
        assert_eq!(
            detector.input().manifest().product.product_id,
            "webcam-frame-history-2"
        );
        assert_eq!(detector.input().stats().delivered, 1);
        assert!(detector.errors().is_empty());
    }

    #[test]
    fn typed_set_product_operable_rebinds_the_cataloged_input() {
        use auki_components::{
            BufferLimits, ConfiguredObservableSpec, InvocationContext, ObservableContract,
            ObservationAccess,
        };
        use std::collections::HashMap;

        let runtime = ComponentRuntime::new("peer-a");
        let controller = runtime
            .component(ComponentSpec::new("operator-console"))
            .unwrap();
        controller.expose().unwrap();
        let camera = runtime
            .component(
                ComponentSpec::new("webcam-capture").observable(ObservableContract {
                    name: "frames".into(),
                    datatype: VideoFrame::DATATYPE.into(),
                    schema: "auki.video-frame/v1".into(),
                    access: vec![ObservationAccess::FollowNew],
                    exposure: Exposure::Cluster,
                }),
            )
            .unwrap();
        let first_output = camera
            .configured_observable::<VideoFrame>(ConfiguredObservableSpec::new(
                "frames",
                "webcam-jpeg-output-1",
                "monotonic-ns",
                typed_jpeg_payload(64, 64, 10),
            ))
            .unwrap();
        camera.expose().unwrap();
        let first_capture = runtime
            .capture_buffer(
                "webcam-frame-history",
                &first_output,
                BufferLimits::entries(32),
                |frame| frame.bytes.len(),
            )
            .unwrap();
        let replacement = camera
            .replace_configured_observable::<VideoFrame>(
                &first_output,
                ConfiguredObservableSpec::new(
                    "frames",
                    "webcam-jpeg-output-2",
                    "monotonic-ns",
                    typed_jpeg_payload(64, 64, 5),
                ),
                2,
            )
            .unwrap();
        let second_capture = runtime
            .capture_buffer(
                "webcam-frame-history-2",
                &replacement.replacement,
                BufferLimits::entries(32),
                |frame| frame.bytes.len(),
            )
            .unwrap();
        let sources = Arc::new(Mutex::new(HashMap::from([
            (first_capture.product().reference(), first_capture.product()),
            (
                second_capture.product().reference(),
                second_capture.product(),
            ),
        ])));
        let resolved_sources = Arc::clone(&sources);
        let detector = QrDetector::default()
            .bind_product_operable(
                &runtime,
                "qr-detector",
                "qr-detections-output-1",
                &first_capture.product(),
                controller.reference().clone(),
                move |reference| resolved_sources.lock().unwrap().get(reference).cloned(),
            )
            .unwrap();
        let target = second_capture.product().reference();
        let applied = detector
            .set_product_operable()
            .unwrap()
            .invoke(
                InvocationContext {
                    invocation_id: "set-qr-source-1".into(),
                    caller_peer_id: controller.reference().peer_id.clone(),
                    caller_component_id: controller.reference().component_id.clone(),
                },
                SetQrDetectorProduct {
                    product: target.clone(),
                },
            )
            .unwrap();

        assert!(applied.result.changed);
        assert_eq!(applied.result.product, target);
        assert_eq!(detector.input().manifest().product.reference(), target);
        let cataloged = runtime.catalog().component("qr-detector").unwrap();
        assert_eq!(
            cataloged.current_product_inputs["frames"]
                .manifest
                .product
                .reference(),
            target
        );
        assert_eq!(cataloged.manifest.operables[0].name, "set_product");
    }

    #[test]
    fn typed_payload_round_trips_through_sdk_envelope() {
        let frame = sample().into_detection_frame("sensor-hash");
        assert_eq!(frame.r#type, QR_DETECTION_TYPE);
        assert_eq!(frame.sensor_hash, "sensor-hash");
        assert_eq!(QrDetections::decode(&frame.data).unwrap(), sample());
    }

    #[test]
    fn blank_luma_frame_has_no_codes() {
        let mut detector = QrDetector::default();
        let detections = detector.detect_luma(&vec![0; 32 * 32], 32, 32, 32).unwrap();
        assert!(detections.codes.is_empty());
    }

    #[test]
    fn rgb8_frame_with_padded_rows_decodes_after_internal_luma_conversion() {
        let payload = "auki://portal/rgb8-test";
        let (pixels, width, height, stride) = rendered_qr_rgb8(payload);
        let mut detector = QrDetector::default();
        let detections = detector
            .detect_rgb8(&pixels, width, height, stride)
            .unwrap();
        assert!(detections.codes.iter().any(|code| code.payload == payload));
    }

    #[tokio::test]
    async fn live_stream_writes_remote_provenance_and_applies_cadence() {
        let tmp = tempfile::tempdir().unwrap();
        let peer = Peer::new("park", "mapping").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let detector = QrDetector::default().register(&peer, "qr-live").unwrap();

        let mut camera = luma_camera();
        camera.frame.peer_id = "remote-robot".into();
        let sensor = SensorRegistryEntry {
            peer_id: "remote-robot".into(),
            sensor_id: "head-left-eye".into(),
            body: SensorBody::Camera(camera),
        };
        let sensor_hash = sensor.hash();
        let frames = futures::stream::iter([0, 500_000_000, 1_000_000_000].map(|timestamp_ns| {
            Ok(CameraFrameSample {
                timestamp_ns,
                frame: std::sync::Arc::new(CameraFrame {
                    dynamic_intrinsics: None,
                    frame: vec![0; 32 * 32],
                }),
            })
        }));
        let task = detector
            .start_stream(
                &session,
                DetectorInstanceSpec::rolling(
                    "qr-remote-1hz",
                    DetectionCadence::Periodic {
                        period_ns: 1_000_000_000,
                    },
                    Duration::from_secs(5),
                    Duration::from_secs(1),
                ),
                CameraStreamDescriptor {
                    log_ref: LogRef {
                        source_peer_id: "remote-robot".into(),
                        resource_id: "head-left-eye".into(),
                    },
                    sensor,
                    clock: RegistryRef {
                        peer_id: "remote-robot".into(),
                        id: "monotonic".into(),
                        hash: "remote-clock-hash".into(),
                    },
                },
                frames,
            )
            .unwrap();

        let log = task.detection_log();
        let mut entries = Vec::new();
        for _ in 0..100 {
            entries = auki_logs::Log::<DetectionFrame>::read(log.root())
                .and_then(|reader| reader.entries())
                .unwrap_or_default();
            if entries.len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 0);
        assert_eq!(entries[1].timestamp_ns, 1_000_000_000);
        assert!(
            entries
                .iter()
                .all(|entry| entry.payload.sensor_hash == sensor_hash)
        );
        assert_eq!(log.manifest.input_log.source_peer_id, "remote-robot");
        task.shutdown().await.unwrap();
    }

    #[test]
    fn unknown_schema_is_rejected() {
        let bytes = br#"{"schema_version":2,"codes":[]}"#;
        assert!(matches!(
            QrDetections::decode(bytes),
            Err(QrDetectorError::UnsupportedSchemaVersion(2))
        ));
    }

    #[test]
    fn start_tails_sensor_log_applies_cadence_and_writes_detection_log() {
        let tmp = tempfile::tempdir().unwrap();
        let peer = Peer::new("robot", "mapping").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame_ref = peer
            .register_frame("camera_optical", FrameDef::RosOptical)
            .unwrap();
        let mut camera = luma_camera();
        camera.frame = frame_ref.clone();
        let sensor_ref = peer
            .register_sensor("left-camera-luma", SensorBody::Camera(camera.clone()))
            .unwrap();
        let sensor_entry = SensorRegistryEntry {
            peer_id: "robot".into(),
            sensor_id: "left-camera-luma".into(),
            body: SensorBody::Camera(camera),
        };
        assert_eq!(sensor_ref.hash, sensor_entry.hash());
        let input = session
            .register_sensor_log(SensorLogSpec {
                sensor: sensor_ref.clone(),
                clock: session.monotonic_clock(),
                frame: Some(frame_ref),
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();
        let mut input_log = auki_logs::Log::<CameraFrame>::open(
            input.root(),
            serde_json::to_value(&input.manifest).unwrap(),
        )
        .unwrap();

        let detector = QrDetector::default().register(&peer, "qr-beta-2").unwrap();
        let task = detector
            .start(
                &session,
                DetectorInstanceSpec::rolling(
                    "qr-left-1hz",
                    DetectionCadence::Periodic {
                        period_ns: 1_000_000_000,
                    },
                    Duration::from_secs(5),
                    Duration::from_secs(1),
                ),
                &input,
            )
            .unwrap();

        let log = task.detection_log();
        assert_eq!(log.resource_id(), "qr-left-1hz");
        assert_eq!(log.manifest.detector, *detector.registry_ref());
        assert_eq!(log.manifest.input_log, input.log_ref);
        assert_eq!(log.manifest.clock, input.manifest.clock);
        assert_eq!(log.manifest.input_sensor.hash, sensor_entry.hash());

        for timestamp_ns in [0, 500_000_000, 1_000_000_000] {
            input_log
                .append(
                    timestamp_ns,
                    &CameraFrame {
                        dynamic_intrinsics: None,
                        frame: vec![0; 32 * 32],
                    },
                )
                .unwrap();
            input_log.flush().unwrap();
        }

        let mut entries = Vec::new();
        for _ in 0..100 {
            entries = auki_logs::Log::<DetectionFrame>::read(log.root())
                .and_then(|reader| reader.entries())
                .unwrap_or_default();
            if entries.len() == 2 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 0);
        assert_eq!(entries[1].timestamp_ns, 1_000_000_000);
        assert!(
            entries
                .iter()
                .all(|entry| entry.payload.sensor_hash == sensor_entry.hash())
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.payload.r#type == QR_DETECTION_TYPE)
        );
        task.shutdown().unwrap();
    }

    #[test]
    fn raw_rgb8_camera_is_compatible() {
        let mut camera = luma_camera();
        camera.r#type = "rgb".into();
        camera.pixel_format = "rgb8".into();
        camera.row_stride_bytes = camera.width * 3;
        QrDetector::validate_camera(&camera).unwrap();
    }

    #[test]
    fn raw_nv12_camera_decodes_qr_from_padded_y_plane() {
        let payload = "auki://portal/nv12-test";
        let (rgb, width, height, rgb_stride) = rendered_qr_rgb8(payload);
        let y_stride = width + 8;
        let mut nv12 = vec![128u8; y_stride * (height + height.div_ceil(2))];
        for y in 0..height {
            for x in 0..width {
                nv12[y * y_stride + x] = rgb[y * rgb_stride + x * 3];
            }
        }

        let mut camera = luma_camera();
        camera.r#type = "rgb".into();
        camera.width = width as u32;
        camera.height = height as u32;
        camera.pixel_format = "YUV_NV12".into();
        camera.row_stride_bytes = y_stride as u32;
        QrDetector::validate_camera(&camera).unwrap();

        let outputs = QrDetector::default()
            .process(
                &CameraFrame {
                    dynamic_intrinsics: None,
                    frame: nv12,
                },
                &camera,
            )
            .unwrap();
        let detections = QrDetections::decode(&outputs[0].data).unwrap();
        assert!(detections.codes.iter().any(|code| code.payload == payload));
    }

    #[test]
    fn truncated_nv12_chroma_plane_is_rejected() {
        let mut camera = luma_camera();
        camera.r#type = "rgb".into();
        camera.pixel_format = "YUV_NV12".into();
        let y_plane_only = vec![0; (camera.row_stride_bytes * camera.height) as usize];

        let error = QrDetector::default()
            .process(
                &CameraFrame {
                    dynamic_intrinsics: None,
                    frame: y_plane_only,
                },
                &camera,
            )
            .unwrap_err();
        assert!(
            error.contains("NV12 frame has"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn jpeg_camera_is_compatible_and_decodes_through_process() {
        let payload = "auki://portal/jpeg-test";
        let (jpeg, width, height) = rendered_qr_jpeg(payload);
        let mut camera = luma_camera();
        camera.r#type = "rgb".into();
        camera.width = width as u32;
        camera.height = height as u32;
        camera.image_encoding = "jpeg".into();
        camera.pixel_format = "rgb8".into();
        camera.row_stride_bytes = 0;
        QrDetector::validate_camera(&camera).unwrap();

        let mut detector = QrDetector::default();
        let outputs = detector
            .process(
                &CameraFrame {
                    dynamic_intrinsics: None,
                    frame: jpeg,
                },
                &camera,
            )
            .unwrap();
        let detections = QrDetections::decode(&outputs[0].data).unwrap();
        assert!(detections.codes.iter().any(|code| code.payload == payload));
    }

    #[test]
    fn jpeg_dimensions_must_match_camera_registry() {
        let (jpeg, width, height) = rendered_qr_jpeg("dimension-test");
        assert!(matches!(
            QrDetector::default().detect_jpeg(&jpeg, width + 1, height),
            Err(QrDetectorError::JpegDimensionMismatch { .. })
        ));
    }

    #[test]
    fn malformed_jpeg_is_rejected_without_panicking() {
        assert!(matches!(
            QrDetector::default().detect_jpeg(b"not a jpeg", 32, 32),
            Err(QrDetectorError::JpegDecode(_))
        ));
    }
}

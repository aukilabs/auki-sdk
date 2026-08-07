//! rxing-backed typed barcode detections carried by the Auki SDK's generic
//! [`DetectionFrame`](auki_datatypes::detection::DetectionFrame) envelope.
//!
//! This crate is deliberately limited to 1D / retail barcode scanning. It does
//! not associate payloads with portals, ESL SKUs, or shelf geometry; those
//! product-specific responsibilities belong to a later consumer.

use auki_datatypes::detection::DetectionFrame;
use auki_registry::{Barcode, Camera, DetectorBody, DetectorInput, RegistryRef, SensorBody};
use auki_session::{
    CameraDetector, CameraDetectorPackageError, CameraDetectorTask, CameraFrameSample,
    DetectionLogHandle, DetectorInstanceSpec, DetectorOutput, Peer, RegisteredCameraDetector,
    SensorLogHandle, Session, StreamingCameraDetectorTask,
};
pub use auki_session::{CameraStreamDescriptor, DetectionCadence};
use futures::Stream;
use rxing::helpers::detect_multiple_in_luma_with_hints;
use rxing::{BarcodeFormat, DecodeHints, Exceptions, Point, RXingResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

/// A running barcode detector and its peer-consumable Detection Log declaration.
pub struct BarcodeDetectorTask {
    task: CameraDetectorTask,
}

/// A running barcode detector consuming a live asynchronous camera stream.
pub struct StreamingBarcodeDetectorTask {
    task: StreamingCameraDetectorTask,
}

/// A content-addressed barcode detector installation that can start independent
/// instances on compatible Camera Sensor Logs.
#[derive(Clone)]
pub struct RegisteredBarcodeDetector {
    inner: RegisteredCameraDetector,
}

impl RegisteredBarcodeDetector {
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
    ) -> Result<BarcodeDetectorTask, BarcodeDetectorError> {
        let input_sensor = session
            .sensor_registry_entry(&input_log.manifest.sensor)
            .ok_or(BarcodeDetectorError::UnknownInputSensorReference)?;
        let SensorBody::Camera(camera) = &input_sensor.body else {
            return Err(BarcodeDetectorError::IncompatibleSensorKind);
        };
        BarcodeDetector::validate_camera(camera)?;
        let task = self.inner.start(session, instance, input_log)?;
        Ok(BarcodeDetectorTask { task })
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
    ) -> Result<StreamingBarcodeDetectorTask, BarcodeDetectorError>
    where
        S: Stream<Item = std::result::Result<CameraFrameSample, String>> + Send + 'static,
    {
        let SensorBody::Camera(camera) = &input.sensor.body else {
            return Err(BarcodeDetectorError::IncompatibleSensorKind);
        };
        BarcodeDetector::validate_camera(camera)?;
        let task = self.inner.start_stream(session, instance, input, frames)?;
        Ok(StreamingBarcodeDetectorTask { task })
    }
}

impl BarcodeDetectorTask {
    pub fn detection_log(&self) -> &DetectionLogHandle {
        self.task.detection_log()
    }

    pub fn request_shutdown(&self) {
        self.task.request_shutdown();
    }

    pub fn shutdown(self) -> Result<(), BarcodeDetectorError> {
        self.task.shutdown().map_err(Into::into)
    }
}

impl StreamingBarcodeDetectorTask {
    pub fn detection_log(&self) -> &DetectionLogHandle {
        self.task.detection_log()
    }

    pub fn request_shutdown(&self) {
        self.task.request_shutdown();
    }

    /// Live frames replaced by fresher pending work while barcode detection was
    /// busy. Frames skipped by cadence are not included.
    pub fn dropped_frames(&self) -> u64 {
        self.task.dropped_frames()
    }

    pub async fn shutdown(self) -> Result<(), BarcodeDetectorError> {
        self.task.shutdown().await.map_err(Into::into)
    }
}

/// The `DetectionFrame.type` emitted by [`BarcodeDetections::into_detection_frame`].
pub const BARCODE_DETECTION_TYPE: &str = "barcode";

/// Current on-wire schema version for [`BarcodeDetections`].
pub const BARCODE_DETECTION_SCHEMA_VERSION: u32 = 1;

/// Cactus-aligned symbology allowlist for the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarcodeSymbologyProfile {
    /// Retail product codes: EAN-13/8, UPC-E, Code128, GS1 DataBar (+ Expanded).
    #[default]
    Product,
    /// ESL / shelf-edge codes: Code128/39/93, Codabar, ITF.
    Esl,
    /// Union of [`Self::Product`] and [`Self::Esl`].
    All,
}

impl BarcodeSymbologyProfile {
    fn formats(self) -> HashSet<BarcodeFormat> {
        match self {
            Self::Product => HashSet::from([
                BarcodeFormat::EAN_13,
                BarcodeFormat::EAN_8,
                BarcodeFormat::UPC_E,
                BarcodeFormat::CODE_128,
                BarcodeFormat::RSS_14,
                BarcodeFormat::RSS_EXPANDED,
            ]),
            Self::Esl => HashSet::from([
                BarcodeFormat::CODE_128,
                BarcodeFormat::CODE_39,
                BarcodeFormat::CODE_93,
                BarcodeFormat::CODABAR,
                BarcodeFormat::ITF,
            ]),
            Self::All => {
                let mut formats = Self::Product.formats();
                formats.extend(Self::Esl.formats());
                formats
            }
        }
    }

    fn allows(self, format: BarcodeFormat) -> bool {
        self.formats().contains(&format)
    }
}

/// Collapse rxing formats to Cactus `eslLabel` wire labels.
fn wire_symbology(format: BarcodeFormat) -> Option<&'static str> {
    match format {
        BarcodeFormat::CODE_128 | BarcodeFormat::CODE_39 | BarcodeFormat::CODE_93 => {
            Some("code128")
        }
        BarcodeFormat::EAN_13 | BarcodeFormat::EAN_8 | BarcodeFormat::UPC_E => Some("ean13"),
        BarcodeFormat::RSS_14 | BarcodeFormat::RSS_EXPANDED => Some("gs1DataBar"),
        BarcodeFormat::ITF => Some("itf"),
        BarcodeFormat::CODABAR => Some("codabar"),
        _ => None,
    }
}

/// Coordinates of one barcode-region corner in source-frame pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelCorner {
    /// Horizontal pixel coordinate, increasing rightward.
    pub x: f64,
    /// Vertical pixel coordinate, increasing downward.
    pub y: f64,
}

/// A barcode successfully decoded in one source frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarcodeDetection {
    /// Decoded payload text.
    pub payload: String,
    /// Cactus wire symbology label (`code128`, `ean13`, `gs1DataBar`, `codabar`, `itf`).
    pub symbology: String,
    /// Coarse source-frame corners in strict `TL, TR, BR, BL` order.
    pub corners_px: [PixelCorner; 4],
}

/// All barcode detections produced from one source frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarcodeDetections {
    /// Version of this payload schema.
    pub schema_version: u32,
    /// Decoded barcodes, in rxing acceptance order.
    pub codes: Vec<BarcodeDetection>,
}

impl BarcodeDetections {
    /// Serialize the versioned typed payload for `DetectionFrame.data`.
    pub fn encode(&self) -> Result<Vec<u8>, BarcodeDetectorError> {
        serde_json::to_vec(self).map_err(BarcodeDetectorError::Encode)
    }

    /// Decode a barcode payload from `DetectionFrame.data`.
    pub fn decode(bytes: &[u8]) -> Result<Self, BarcodeDetectorError> {
        let decoded: Self = serde_json::from_slice(bytes).map_err(BarcodeDetectorError::Decode)?;
        if decoded.schema_version != BARCODE_DETECTION_SCHEMA_VERSION {
            return Err(BarcodeDetectorError::UnsupportedSchemaVersion(
                decoded.schema_version,
            ));
        }
        Ok(decoded)
    }

    /// Wrap these barcode detections in the SDK's generic per-frame envelope.
    pub fn into_detection_frame(self, input_sensor_hash: impl Into<String>) -> DetectionFrame {
        DetectionFrame {
            data: self
                .encode()
                .expect("BarcodeDetections generated by this crate serialize"),
            sensor_hash: input_sensor_hash.into(),
            r#type: BARCODE_DETECTION_TYPE.into(),
        }
    }
}

/// Decoder configuration used by [`BarcodeDetector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BarcodeDetectorConfig {
    /// Active symbology allowlist.
    pub profile: BarcodeSymbologyProfile,
}

/// Stateful barcode detector. Retain one instance when processing a video stream.
pub struct BarcodeDetector {
    config: BarcodeDetectorConfig,
}

impl BarcodeDetector {
    pub fn new(config: BarcodeDetectorConfig) -> Self {
        Self { config }
    }

    /// Validate that a Camera Registry contract can be consumed by this
    /// detector. NV12 is scanned through its Y plane; JPEG is decoded to RGB8
    /// before luminance conversion.
    pub fn validate_camera(camera: &Camera) -> Result<(), BarcodeDetectorError> {
        if camera.image_encoding == "jpeg" {
            return Ok(());
        }
        if camera.image_encoding != "raw"
            || !matches!(camera.pixel_format.as_str(), "luma8" | "rgb8" | "YUV_NV12")
        {
            return Err(BarcodeDetectorError::IncompatibleCamera {
                image_encoding: camera.image_encoding.clone(),
                pixel_format: camera.pixel_format.clone(),
            });
        }
        let bytes_per_pixel = if camera.pixel_format == "rgb8" { 3 } else { 1 };
        let minimum_row_bytes = camera.width.saturating_mul(bytes_per_pixel);
        if camera.row_stride_bytes < minimum_row_bytes {
            return Err(BarcodeDetectorError::InvalidCameraStride {
                minimum_row_bytes,
                row_stride_bytes: camera.row_stride_bytes,
            });
        }
        Ok(())
    }

    /// Register this detector's immutable SDK identity on `peer`.
    pub fn register(
        &self,
        peer: &Peer,
        detector_id: &str,
    ) -> Result<RegisteredBarcodeDetector, BarcodeDetectorError> {
        let config = self.config;
        let inner = RegisteredCameraDetector::register(
            peer,
            detector_id,
            DetectorBody::Barcode(Barcode {}),
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
            vec![BARCODE_DETECTION_TYPE.into()],
            move || BarcodeDetector::new(config),
        )?;
        Ok(RegisteredBarcodeDetector { inner })
    }

    /// Scan an 8-bit grayscale source frame.
    ///
    /// `stride` is in bytes and may exceed `width`, allowing direct use of a
    /// padded camera Y plane. Returned corners are always in this source
    /// frame's pixels.
    pub fn detect_luma(
        &mut self,
        pixels: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<BarcodeDetections, BarcodeDetectorError> {
        let luma = pack_luma_plane(pixels, width, height, stride)?;
        self.decode_packed_luma(luma, width, height)
    }

    /// Scan a packed RGB8 source frame after converting it to luminance.
    ///
    /// `stride` is in bytes and may exceed `width * 3`. Conversion uses BT.601
    /// weights. Returned corners remain in source-frame pixels.
    pub fn detect_rgb8(
        &mut self,
        pixels: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<BarcodeDetections, BarcodeDetectorError> {
        let luma = rgb8_to_luma(pixels, width, height, stride)?;
        self.decode_packed_luma(luma, width, height)
    }

    /// Scan the full-resolution Y plane of an NV12 frame.
    ///
    /// NV12 stores `height` padded luminance rows followed by half as many
    /// interleaved UV rows using the same stride. The chroma plane is not
    /// needed for barcode detection, but its presence is validated so truncated
    /// payloads cannot masquerade as valid NV12 frames.
    pub fn detect_nv12(
        &mut self,
        pixels: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<BarcodeDetections, BarcodeDetectorError> {
        let y_plane_len = stride.checked_mul(height).ok_or_else(|| {
            BarcodeDetectorError::InvalidFrame("NV12 Y-plane dimensions overflow".into())
        })?;
        let chroma_rows = height.div_ceil(2);
        let frame_rows = height.checked_add(chroma_rows).ok_or_else(|| {
            BarcodeDetectorError::InvalidFrame("NV12 frame dimensions overflow".into())
        })?;
        let required_len = stride.checked_mul(frame_rows).ok_or_else(|| {
            BarcodeDetectorError::InvalidFrame("NV12 frame dimensions overflow".into())
        })?;
        if pixels.len() < required_len {
            return Err(BarcodeDetectorError::InvalidFrame(format!(
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
    ) -> Result<BarcodeDetections, BarcodeDetectorError> {
        let expected_width_u32 = u32::try_from(expected_width)
            .map_err(|_| BarcodeDetectorError::InvalidFrame("JPEG width exceeds u32".into()))?;
        let expected_height_u32 = u32::try_from(expected_height)
            .map_err(|_| BarcodeDetectorError::InvalidFrame("JPEG height exceeds u32".into()))?;
        let dimensions =
            image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg)
                .into_dimensions()
                .map_err(BarcodeDetectorError::JpegDecode)?;
        if dimensions != (expected_width_u32, expected_height_u32) {
            return Err(BarcodeDetectorError::JpegDimensionMismatch {
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
        let decoded = reader.decode().map_err(BarcodeDetectorError::JpegDecode)?;
        let rgb = decoded.into_rgb8();
        let width = rgb.width() as usize;
        let height = rgb.height() as usize;
        self.detect_rgb8(rgb.as_raw(), width, height, width * 3)
    }

    fn decode_packed_luma(
        &mut self,
        luma: Vec<u8>,
        width: usize,
        height: usize,
    ) -> Result<BarcodeDetections, BarcodeDetectorError> {
        let width_u32 = u32::try_from(width)
            .map_err(|_| BarcodeDetectorError::InvalidFrame("width exceeds u32".into()))?;
        let height_u32 = u32::try_from(height)
            .map_err(|_| BarcodeDetectorError::InvalidFrame("height exceeds u32".into()))?;
        if luma.len() != width.saturating_mul(height) {
            return Err(BarcodeDetectorError::InvalidFrame(format!(
                "packed luma has {} bytes, expected {}",
                luma.len(),
                width.saturating_mul(height)
            )));
        }

        let mut hints = DecodeHints {
            TryHarder: Some(true),
            PossibleFormats: Some(self.config.profile.formats()),
            ..Default::default()
        };

        let results = match detect_multiple_in_luma_with_hints(luma, width_u32, height_u32, &mut hints)
        {
            Ok(results) => results,
            Err(Exceptions::NotFoundException(_)) => Vec::new(),
            Err(error) => {
                return Err(BarcodeDetectorError::DecodeFailed(error.to_string()));
            }
        };

        Ok(BarcodeDetections {
            schema_version: BARCODE_DETECTION_SCHEMA_VERSION,
            codes: results
                .into_iter()
                .filter_map(|result| map_rxing_result(result, self.config.profile))
                .collect(),
        })
    }
}

impl CameraDetector for BarcodeDetector {
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
            _ => Err(BarcodeDetectorError::IncompatibleCamera {
                image_encoding: camera.image_encoding.clone(),
                pixel_format: camera.pixel_format.clone(),
            }),
        }
        .map_err(|error| error.to_string())?;
        Ok(vec![DetectorOutput {
            r#type: BARCODE_DETECTION_TYPE.into(),
            data: detections.encode().map_err(|error| error.to_string())?,
        }])
    }
}

impl Default for BarcodeDetector {
    fn default() -> Self {
        Self::new(BarcodeDetectorConfig::default())
    }
}

fn map_rxing_result(
    result: RXingResult,
    profile: BarcodeSymbologyProfile,
) -> Option<BarcodeDetection> {
    let format = *result.getBarcodeFormat();
    if !profile.allows(format) {
        return None;
    }
    let symbology = wire_symbology(format)?.to_string();
    Some(BarcodeDetection {
        payload: result.getText().to_string(),
        symbology,
        corners_px: corners_from_points(result.getPoints()),
    })
}

fn corners_from_points(points: &[Point]) -> [PixelCorner; 4] {
    match points {
        [] => [PixelCorner { x: 0.0, y: 0.0 }; 4],
        [only] => {
            let c = pixel_corner(*only);
            [c, c, c, c]
        }
        [a, b] => thin_rectangle(*a, *b),
        [a, b, c] => {
            // Approximate a quad from three finder-style points.
            let p0 = pixel_corner(*a);
            let p1 = pixel_corner(*b);
            let p2 = pixel_corner(*c);
            let p3 = PixelCorner {
                x: p0.x + (p2.x - p1.x),
                y: p0.y + (p2.y - p1.y),
            };
            [p0, p1, p2, p3]
        }
        [a, b, c, d, ..] => [
            pixel_corner(*a),
            pixel_corner(*b),
            pixel_corner(*c),
            pixel_corner(*d),
        ],
    }
}

/// Build a thin rectangle around a 1D barcode's start/end result points.
fn thin_rectangle(a: Point, b: Point) -> [PixelCorner; 4] {
    let dx = (b.x - a.x) as f64;
    let dy = (b.y - a.y) as f64;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    const HALF_THICKNESS_PX: f64 = 2.0;
    let nx = -dy / len * HALF_THICKNESS_PX;
    let ny = dx / len * HALF_THICKNESS_PX;
    [
        PixelCorner {
            x: a.x as f64 + nx,
            y: a.y as f64 + ny,
        },
        PixelCorner {
            x: b.x as f64 + nx,
            y: b.y as f64 + ny,
        },
        PixelCorner {
            x: b.x as f64 - nx,
            y: b.y as f64 - ny,
        },
        PixelCorner {
            x: a.x as f64 - nx,
            y: a.y as f64 - ny,
        },
    ]
}

fn pixel_corner(point: Point) -> PixelCorner {
    PixelCorner {
        x: point.x as f64,
        y: point.y as f64,
    }
}

fn pack_luma_plane(
    pixels: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Result<Vec<u8>, BarcodeDetectorError> {
    if stride < width {
        return Err(BarcodeDetectorError::InvalidFrame(format!(
            "luma stride {stride} is smaller than width {width}"
        )));
    }
    let required = stride
        .checked_mul(height)
        .ok_or_else(|| BarcodeDetectorError::InvalidFrame("luma dimensions overflow".into()))?;
    if pixels.len() < required {
        return Err(BarcodeDetectorError::InvalidFrame(format!(
            "luma frame has {} bytes, requires at least {required}",
            pixels.len()
        )));
    }
    if stride == width {
        return Ok(pixels[..required].to_vec());
    }
    let mut packed = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = y * stride;
        packed.extend_from_slice(&pixels[row..row + width]);
    }
    Ok(packed)
}

fn rgb8_to_luma(
    pixels: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Result<Vec<u8>, BarcodeDetectorError> {
    let row_bytes = width
        .checked_mul(3)
        .ok_or_else(|| BarcodeDetectorError::InvalidFrame("rgb8 row size overflow".into()))?;
    if stride < row_bytes {
        return Err(BarcodeDetectorError::InvalidFrame(format!(
            "rgb8 stride {stride} is smaller than required row size {row_bytes}"
        )));
    }
    let required = stride
        .checked_mul(height)
        .ok_or_else(|| BarcodeDetectorError::InvalidFrame("rgb8 dimensions overflow".into()))?;
    if pixels.len() < required {
        return Err(BarcodeDetectorError::InvalidFrame(format!(
            "rgb8 frame has {} bytes, requires at least {required}",
            pixels.len()
        )));
    }
    let mut luma = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = y * stride;
        for x in 0..width {
            let i = row + x * 3;
            let r = pixels[i] as u32;
            let g = pixels[i + 1] as u32;
            let b = pixels[i + 2] as u32;
            // BT.601 integer approximation matching common camera pipelines.
            luma.push(((77 * r + 150 * g + 29 * b) >> 8) as u8);
        }
    }
    Ok(luma)
}

/// Errors raised while scanning, encoding, or registering barcode detections.
#[derive(Debug, Error)]
pub enum BarcodeDetectorError {
    /// Generic detector package registration, startup, or execution failed.
    #[error("detector package: {0}")]
    Package(#[from] CameraDetectorPackageError),
    /// The camera registry does not expose a supported raw or JPEG image.
    #[error(
        "barcode detector requires raw/luma8, raw/rgb8, raw/YUV_NV12, or jpeg camera frames, got {image_encoding}/{pixel_format}"
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
    /// The supplied camera plane is not a valid luminance/RGB view.
    #[error("invalid frame: {0}")]
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
    /// rxing failed for a reason other than "no barcode found".
    #[error("barcode decode failed: {0}")]
    DecodeFailed(String),
    /// Typed payload serialization failed.
    #[error("could not encode barcode detection payload: {0}")]
    Encode(serde_json::Error),
    /// Typed payload deserialization failed.
    #[error("could not decode barcode detection payload: {0}")]
    Decode(serde_json::Error),
    /// The payload uses an unknown schema version.
    #[error("unsupported barcode detection schema version: {0}")]
    UnsupportedSchemaVersion(u32),
    /// The selected registry entry is not a camera stream.
    #[error("barcode detector input must be a camera sensor")]
    IncompatibleSensorKind,
    /// The Sensor Log's exact Sensor Registry entry cannot be resolved locally.
    #[error("input Sensor Registry reference does not exist in this session's peer")]
    UnknownInputSensorReference,
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_datatypes::camera::CameraFrame;
    use auki_registry::SensorRegistryEntry;
    use auki_session::{FrameDef, HeadSpec, SensorLogSpec};
    use rxing::{MultiFormatWriter, Writer};
    use std::thread;
    use std::time::Duration;

    const EAN13_PAYLOAD: &str = "4012345678901";

    fn sample() -> BarcodeDetections {
        BarcodeDetections {
            schema_version: BARCODE_DETECTION_SCHEMA_VERSION,
            codes: vec![BarcodeDetection {
                payload: EAN13_PAYLOAD.into(),
                symbology: "ean13".into(),
                corners_px: [
                    PixelCorner { x: 1.0, y: 2.0 },
                    PixelCorner { x: 3.0, y: 2.0 },
                    PixelCorner { x: 3.0, y: 4.0 },
                    PixelCorner { x: 1.0, y: 4.0 },
                ],
            }],
        }
    }

    /// Encode an EAN-13 via rxing and rasterize to a padded luma8 plane.
    fn rendered_ean13_luma(payload: &str) -> (Vec<u8>, usize, usize) {
        let matrix = MultiFormatWriter
            .encode(payload, &BarcodeFormat::EAN_13, 400, 120)
            .expect("ean13 encode");
        let width = matrix.width() as usize;
        let height = matrix.height() as usize;
        // Extra white margin helps the 1D reader lock onto quiet zones.
        let pad = 16usize;
        let out_w = width + pad * 2;
        let out_h = height + pad * 2;
        let mut luma = vec![255u8; out_w * out_h];
        for y in 0..height {
            for x in 0..width {
                if matrix.get(x as u32, y as u32) {
                    luma[(y + pad) * out_w + (x + pad)] = 0;
                }
            }
        }
        (luma, out_w, out_h)
    }

    fn luma_camera(width: u32, height: u32) -> Camera {
        Camera {
            r#type: "mono".into(),
            width,
            height,
            frame_rate_hz: 30,
            image_encoding: "raw".into(),
            pixel_format: "luma8".into(),
            row_stride_bytes: width,
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

    #[test]
    fn typed_payload_round_trips_through_sdk_envelope() {
        let frame = sample().into_detection_frame("sensor-hash");
        assert_eq!(frame.r#type, BARCODE_DETECTION_TYPE);
        assert_eq!(frame.sensor_hash, "sensor-hash");
        assert_eq!(BarcodeDetections::decode(&frame.data).unwrap(), sample());
    }

    #[test]
    fn unknown_schema_is_rejected() {
        let bytes = br#"{"schema_version":2,"codes":[]}"#;
        assert!(matches!(
            BarcodeDetections::decode(bytes),
            Err(BarcodeDetectorError::UnsupportedSchemaVersion(2))
        ));
    }

    #[test]
    fn blank_luma_frame_has_no_codes() {
        let mut detector = BarcodeDetector::default();
        let detections = detector.detect_luma(&vec![0; 32 * 32], 32, 32, 32).unwrap();
        assert!(detections.codes.is_empty());
    }

    #[test]
    fn wire_labels_match_cactus_collapse() {
        assert_eq!(wire_symbology(BarcodeFormat::CODE_39), Some("code128"));
        assert_eq!(wire_symbology(BarcodeFormat::EAN_8), Some("ean13"));
        assert_eq!(wire_symbology(BarcodeFormat::RSS_14), Some("gs1DataBar"));
        assert_eq!(wire_symbology(BarcodeFormat::ITF), Some("itf"));
        assert_eq!(wire_symbology(BarcodeFormat::CODABAR), Some("codabar"));
        assert_eq!(wire_symbology(BarcodeFormat::QR_CODE), None);
    }

    #[test]
    fn two_points_synthesize_thin_rectangle() {
        let corners = thin_rectangle(Point { x: 0.0, y: 10.0 }, Point { x: 100.0, y: 10.0 });
        assert_eq!(corners[0].x, 0.0);
        assert_eq!(corners[1].x, 100.0);
        assert!((corners[0].y - 12.0).abs() < 1e-9);
        assert!((corners[2].y - 8.0).abs() < 1e-9);
    }

    #[test]
    fn synthetic_ean13_product_profile_decodes_payload_and_corners() {
        let (luma, width, height) = rendered_ean13_luma(EAN13_PAYLOAD);
        let mut detector = BarcodeDetector::new(BarcodeDetectorConfig {
            profile: BarcodeSymbologyProfile::Product,
        });
        let detections = detector
            .detect_luma(&luma, width, height, width)
            .expect("detect_luma");
        assert_eq!(detections.codes.len(), 1);
        let code = &detections.codes[0];
        assert_eq!(code.payload, EAN13_PAYLOAD);
        assert_eq!(code.symbology, "ean13");
        assert_eq!(code.corners_px.len(), 4);
        // Corners should span a non-degenerate region inside the padded frame.
        let xs: Vec<f64> = code.corners_px.iter().map(|c| c.x).collect();
        let ys: Vec<f64> = code.corners_px.iter().map(|c| c.y).collect();
        let x_span = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
            - xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let y_span = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
            - ys.iter().cloned().fold(f64::INFINITY, f64::min);
        assert!(x_span > 10.0, "expected horizontal barcode extent, got {x_span}");
        assert!(y_span >= 0.0);
        for corner in &code.corners_px {
            assert!(corner.x >= 0.0 && corner.x < width as f64);
            assert!(corner.y >= 0.0 && corner.y < height as f64);
        }

        let mut camera = luma_camera(width as u32, height as u32);
        camera.row_stride_bytes = width as u32;
        let outputs = detector
            .process(
                &CameraFrame {
                    dynamic_intrinsics: None,
                    frame: luma,
                },
                &camera,
            )
            .unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].r#type, BARCODE_DETECTION_TYPE);
        let via_process = BarcodeDetections::decode(&outputs[0].data).unwrap();
        assert_eq!(via_process.codes[0].payload, EAN13_PAYLOAD);
        assert_eq!(via_process.codes[0].symbology, "ean13");
    }

    #[test]
    fn start_tails_sensor_log_writes_barcode_detection_log() {
        let (luma, width, height) = rendered_ean13_luma(EAN13_PAYLOAD);
        let tmp = tempfile::tempdir().unwrap();
        let peer = Peer::new("robot", "mapping").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame_ref = peer
            .register_frame("camera_optical", FrameDef::RosOptical)
            .unwrap();
        let mut camera = luma_camera(width as u32, height as u32);
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

        let detector = BarcodeDetector::new(BarcodeDetectorConfig {
            profile: BarcodeSymbologyProfile::Product,
        })
        .register(&peer, "barcode-beta-1")
        .unwrap();
        let task = detector
            .start(
                &session,
                DetectorInstanceSpec::rolling(
                    "barcode-left-1hz",
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
        assert_eq!(log.resource_id(), "barcode-left-1hz");
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
                        frame: luma.clone(),
                    },
                )
                .unwrap();
            input_log.flush().unwrap();
        }

        let mut entries = Vec::new();
        for _ in 0..200 {
            entries = auki_logs::Log::<DetectionFrame>::read(log.root())
                .and_then(|reader| reader.entries())
                .unwrap_or_default();
            if entries.len() >= 2
                && entries
                    .iter()
                    .all(|entry| entry.payload.r#type == BARCODE_DETECTION_TYPE)
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            entries.len() >= 2,
            "expected cadence-sampled Detection Log entries, got {}",
            entries.len()
        );
        assert_eq!(entries[0].timestamp_ns, 0);
        assert!(
            entries
                .iter()
                .all(|entry| entry.payload.sensor_hash == sensor_entry.hash())
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.payload.r#type == BARCODE_DETECTION_TYPE)
        );
        let decoded = BarcodeDetections::decode(&entries[0].payload.data).unwrap();
        assert_eq!(decoded.codes.len(), 1);
        assert_eq!(decoded.codes[0].payload, EAN13_PAYLOAD);
        assert_eq!(decoded.codes[0].symbology, "ean13");
        assert_eq!(decoded.codes[0].corners_px.len(), 4);

        task.shutdown().unwrap();
    }
}

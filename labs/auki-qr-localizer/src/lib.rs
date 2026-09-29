//! Calibrated monocular QR pose estimates. These are observations, not tracking or map authority.
//! Source camera coordinates are always optical: +X right, +Y down, +Z forward.

use auki_components::{
    CameraPayloadContract, ContractType, PayloadContract, ProductReference, RetainedProduct,
    VideoFrame,
};
use auki_geometry::pnp::{
    Camera, Quaternion, Vector2, Vector3, estimate_square_pose_from_pixels, pose_tools,
};
use auki_scenegraph::{MapDefinition, QrAnchor, RigidTransform, Scenegraph};
use serde::{Deserialize, Serialize};

#[cfg(feature = "qr-detector")]
pub mod component;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "model", content = "coefficients", rename_all = "snake_case")]
pub enum Distortion {
    None,
    BrownConrady(Vec<f64>),
    OpenCvFisheye([f64; 4]),
}

/// Static calibration tied to one exact camera Product. New optics/crop/resolution
/// requires a replacement calibration and source Product, never silent reuse.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Calibration {
    pub id: String,
    pub camera_product: ProductReference,
    pub camera_frame_id: String,
    pub clock: auki_components::ClockReference,
    pub width: u32,
    pub height: u32,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub distortion: Distortion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Intrinsics {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub distortion: Distortion,
}

#[derive(Debug, thiserror::Error)]
pub enum LocalizationError {
    #[error("invalid calibration or source camera contract")]
    Calibration,
    #[error("invalid or incompatible map snapshot: {0}")]
    Map(String),
    #[error("anchor is not present in this map")]
    UnknownAnchor,
    #[error("corners are outside the image, degenerate, too small, or not ordered TL,TR,BR,BL")]
    Corners,
    #[error("pose solver failed")]
    Solver,
    #[error("pose is behind the camera, back-facing or exceeds the reprojection threshold")]
    Quality,
}

impl Calibration {
    pub fn for_product(
        id: impl Into<String>,
        product: &RetainedProduct<VideoFrame>,
        intrinsics: Intrinsics,
    ) -> Result<Self, LocalizationError> {
        let PayloadContract::Camera(CameraPayloadContract {
            width,
            height,
            datatype,
            schema,
            ..
        }) = &product.producer.payload
        else {
            return Err(LocalizationError::Calibration);
        };
        if datatype != VideoFrame::DATATYPE || schema != "auki.video-frame/v1" {
            return Err(LocalizationError::Calibration);
        }
        let value = Self {
            id: id.into(),
            camera_product: product.reference(),
            camera_frame_id: product
                .producer
                .spatial_frame_id
                .clone()
                .ok_or(LocalizationError::Calibration)?,
            clock: product.producer.clock.clone(),
            width: *width,
            height: *height,
            fx: intrinsics.fx,
            fy: intrinsics.fy,
            cx: intrinsics.cx,
            cy: intrinsics.cy,
            distortion: intrinsics.distortion,
        };
        value.camera()?;
        Ok(value)
    }

    fn camera(&self) -> Result<Camera, LocalizationError> {
        if self.id.is_empty()
            || self.camera_frame_id.is_empty()
            || auki_components::clock::validate_clock(&self.clock).is_err()
            || self.width == 0
            || self.height == 0
            || ![self.fx, self.fy, self.cx, self.cy]
                .iter()
                .all(|x| x.is_finite())
            || self.fx <= 0.0
            || self.fy <= 0.0
        {
            return Err(LocalizationError::Calibration);
        }
        let camera = match &self.distortion {
            Distortion::None => Camera::pinhole(self.fx, self.fy, self.cx, self.cy),
            Distortion::BrownConrady(c) => {
                if !matches!(c.len(), 4 | 5 | 8) || !c.iter().all(|v| v.is_finite()) {
                    return Err(LocalizationError::Calibration);
                }
                Camera::new(self.fx, self.fy, self.cx, self.cy, c)
            }
            Distortion::OpenCvFisheye(c) => {
                if !c.iter().all(|v| v.is_finite()) {
                    return Err(LocalizationError::Calibration);
                }
                Camera::opencv_fisheye(self.fx, self.fy, self.cx, self.cy, c)
            }
        };
        camera.map_err(|_| LocalizationError::Calibration)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct QualityGate {
    pub minimum_area_px2: f64,
    pub maximum_reprojection_rms_px: f64,
}
impl QualityGate {
    fn validate(self) -> Result<(), LocalizationError> {
        if !self.minimum_area_px2.is_finite()
            || self.minimum_area_px2 <= 0.0
            || !self.maximum_reprojection_rms_px.is_finite()
            || self.maximum_reprojection_rms_px <= 0.0
        {
            return Err(LocalizationError::Quality);
        }
        Ok(())
    }
}
impl Default for QualityGate {
    fn default() -> Self {
        Self {
            minimum_area_px2: 16.0,
            maximum_reprojection_rms_px: 2.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CameraPoseEstimate {
    pub anchor_id: String,
    /// Position in map units, orientation of the optical camera axes in the map frame.
    pub camera_pose_in_map: RigidTransform,
    pub reprojection_rms_px: f64,
    /// Solver residual, not a calibrated probability or pose covariance.
    pub normalized_corner_error: f64,
}

/// Single-anchor estimate. No temporal filter, motion extrapolation or multi-anchor fusion.
/// Corners name the marker's printed corners, not sorted image bounding-box corners.
pub fn estimate_camera_pose(
    map: &MapDefinition,
    anchor: &QrAnchor,
    corners: [[f64; 2]; 4],
    calibration: &Calibration,
    gate: QualityGate,
) -> Result<CameraPoseEstimate, LocalizationError> {
    Scenegraph::new(map.clone()).map_err(|e| LocalizationError::Map(e.to_string()))?;
    anchor
        .validate_in_map(map)
        .map_err(|e| LocalizationError::Map(e.to_string()))?;
    let camera = calibration.camera()?;
    gate.validate()?;
    let mut twice_area = 0.0;
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        let c = corners[(i + 2) % 4];
        if !a.iter().all(|x| x.is_finite())
            || a[0] < 0.0
            || a[1] < 0.0
            || a[0] >= calibration.width as f64
            || a[1] >= calibration.height as f64
            || (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) <= 1e-9
        {
            return Err(LocalizationError::Corners);
        }
        twice_area += a[0] * b[1] - a[1] * b[0];
    }
    if twice_area * 0.5 < gate.minimum_area_px2 {
        return Err(LocalizationError::Corners);
    }
    let estimated = estimate_square_pose_from_pixels(
        corners.map(|p| Vector2::new(p[0], p[1])),
        anchor.side_length_m,
        &camera,
    )
    .map_err(|_| LocalizationError::Solver)?;
    // PnPLab's public square API returns marker-to-OpenGL-camera. Convert before inversion.
    let marker_to_camera = pose_tools::from_opengl_to_opencv(&estimated.pose);
    let rotation = marker_to_camera.rotation.to_na_unit();
    let t = marker_to_camera.position.to_na();
    let normal = rotation * Vector3::new(0.0, 0.0, 1.0).to_na();
    if normal.dot(&t) >= 0.0 {
        return Err(LocalizationError::Quality);
    }
    let h = anchor.side_length_m * 0.5;
    let points = [[-h, h, 0.0], [h, h, 0.0], [h, -h, 0.0], [-h, -h, 0.0]];
    let mut squared_error = 0.0;
    for (point, observed) in points.into_iter().zip(corners) {
        let p = rotation * Vector3::new(point[0], point[1], point[2]).to_na() + t;
        if p.z <= 0.0 {
            return Err(LocalizationError::Quality);
        }
        let projected = camera
            .project(Vector3::from_na(&p))
            .ok_or(LocalizationError::Quality)?;
        squared_error += (projected.x - observed[0]).powi(2) + (projected.y - observed[1]).powi(2);
    }
    let rms = (squared_error / 4.0).sqrt();
    if !rms.is_finite() || rms > gate.maximum_reprojection_rms_px {
        return Err(LocalizationError::Quality);
    }
    let camera_to_marker = pose_tools::invert_pose(&marker_to_camera);
    let [w, x, y, z] = anchor.pose_in_map.rotation_wxyz;
    let anchor_rotation = Quaternion::new(x, y, z, w);
    let offset = anchor_rotation.to_na_unit() * camera_to_marker.position.to_na()
        / map.frame.meters_per_unit;
    let camera_rotation = anchor_rotation
        .multiply(&camera_to_marker.rotation)
        .normalize();
    let position = std::array::from_fn(|i| anchor.pose_in_map.translation[i] + offset[i]);
    if !position.iter().all(|v| v.is_finite()) {
        return Err(LocalizationError::Quality);
    }
    Ok(CameraPoseEstimate {
        anchor_id: anchor.anchor_id.clone(),
        camera_pose_in_map: RigidTransform {
            from_frame_id: calibration.camera_frame_id.clone(),
            to_frame_id: map.frame.id.clone(),
            translation: position,
            rotation_wxyz: [
                camera_rotation.w,
                camera_rotation.x,
                camera_rotation.y,
                camera_rotation.z,
            ],
        },
        reprojection_rms_px: rms,
        normalized_corner_error: estimated.normalized_corner_error,
    })
}

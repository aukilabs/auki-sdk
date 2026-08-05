//! Detector-agnostic Portal pose estimation from four image corners.

use crate::{CameraCalibrationError, effective_camera_calibration};
use auki_datatypes::{
    camera::CameraFrame,
    pose::{Quat, SpatialTransform, Vec3},
};
use auki_geometry::convert_transform_target_convention;
use auki_registry::{Camera, FrameRegistryEntry, RegistryRef};
use pnp_core::{Camera as PnpCamera, Vector2, estimate_square_pose_from_pixels, pose_tools};
use thiserror::Error;

/// One point in source-image pixels: origin at top-left, +X right, +Y down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImagePoint {
    pub x: f64,
    pub y: f64,
}

/// Canonical Portal geometry resolved by an application or Portal service.
///
/// `physical_size_m` is the side length of the square whose four detected
/// corners are supplied to [`estimate_portal_observation`].
#[derive(Debug, Clone, PartialEq)]
pub struct PortalDefinition {
    pub portal_id: String,
    pub physical_size_m: f64,
}

/// One metric Portal observation in the camera's registered optical frame.
///
/// `portal_to_camera` transforms Portal-local coordinates into the camera
/// frame. Portal-local +X runs TL→TR, +Y points upward on the Portal face,
/// and +Z is their right-handed cross product. The observation is deliberately
/// not authoritative map state; a later materializer may fuse repeated or
/// multi-peer observations into a Portal placement.
#[derive(Debug, Clone, PartialEq)]
pub struct PortalObservation {
    pub portal_id: String,
    pub physical_size_m: f64,
    pub camera_frame: RegistryRef,
    pub portal_to_camera: SpatialTransform,
    pub corners_px: [ImagePoint; 4],
    pub confidence: f64,
    pub normalized_corner_error: f64,
}

/// Estimate a square Portal pose from detector output in strict
/// `TL, TR, BR, BL` order.
///
/// This API intentionally does not depend on the SDK QR reference detector.
/// Any detector can supply ordered corners, while the application remains
/// responsible for recognizing the payload and resolving canonical Portal
/// geometry.
pub fn estimate_portal_observation(
    camera: &Camera,
    camera_frame: &FrameRegistryEntry,
    frame: &CameraFrame,
    portal: &PortalDefinition,
    corners_px: [ImagePoint; 4],
) -> Result<PortalObservation, PortalPnpError> {
    if portal.portal_id.is_empty() {
        return Err(PortalPnpError::EmptyPortalId);
    }
    if !portal.physical_size_m.is_finite() || portal.physical_size_m <= 0.0 {
        return Err(PortalPnpError::InvalidPhysicalSize);
    }
    if camera.intrinsics_model != "pinhole" {
        return Err(PortalPnpError::UnsupportedIntrinsicsModel(
            camera.intrinsics_model.clone(),
        ));
    }
    if camera.frame.peer_id != camera_frame.peer_id
        || camera.frame.id != camera_frame.frame_id
        || camera.frame.hash != camera_frame.hash()
    {
        return Err(PortalPnpError::CameraFrameReferenceMismatch);
    }

    let calibration = effective_camera_calibration(camera, frame)?;
    let coefficients: Vec<f64> = calibration
        .distortion_coefficients
        .iter()
        .map(|value| value.0)
        .collect();
    let pnp_camera = match camera.distortion_model.as_str() {
        "none" if coefficients.is_empty() => PnpCamera::pinhole(
            calibration.fx.0,
            calibration.fy.0,
            calibration.cx.0,
            calibration.cy.0,
        ),
        "brown_conrady" | "plumb_bob" => PnpCamera::new(
            calibration.fx.0,
            calibration.fy.0,
            calibration.cx.0,
            calibration.cy.0,
            &coefficients,
        ),
        "opencv_fisheye" | "equidistant" => PnpCamera::opencv_fisheye(
            calibration.fx.0,
            calibration.fy.0,
            calibration.cx.0,
            calibration.cy.0,
            &coefficients,
        ),
        model => return Err(PortalPnpError::UnsupportedDistortionModel(model.to_owned())),
    }
    .map_err(|_| PortalPnpError::InvalidPnpCalibration)?;

    let pixels = corners_px.map(|corner| Vector2::new(corner.x, corner.y));
    let estimate = estimate_square_pose_from_pixels(pixels, portal.physical_size_m, &pnp_camera)
        .map_err(|_| PortalPnpError::PoseEstimationFailed)?;

    // pnp-core's public square-pixel API returns OpenGL camera coordinates.
    // SDK camera frames use the calibrated optical (OpenCV/ROS) convention.
    let pose = pose_tools::from_opengl_to_opencv(&estimate.pose);
    let portal_to_optical = SpatialTransform {
        translation: Some(Vec3 {
            x: pose.position.x,
            y: pose.position.y,
            z: pose.position.z,
        }),
        orientation: Some(Quat {
            x: pose.rotation.x,
            y: pose.rotation.y,
            z: pose.rotation.z,
            w: pose.rotation.w,
        }),
    };
    let pnp_optical_frame = FrameRegistryEntry::ros_optical("pnp-core", "camera_optical");
    let portal_to_camera =
        convert_transform_target_convention(&portal_to_optical, &pnp_optical_frame, camera_frame)
            .map_err(|error| PortalPnpError::CameraFrameConvention(error.to_string()))?;

    Ok(PortalObservation {
        portal_id: portal.portal_id.clone(),
        physical_size_m: portal.physical_size_m,
        camera_frame: camera.frame.clone(),
        portal_to_camera,
        corners_px,
        confidence: estimate.confidence,
        normalized_corner_error: estimate.normalized_corner_error,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PortalPnpError {
    #[error("portal id must be non-empty")]
    EmptyPortalId,
    #[error("portal physical size must be finite and positive")]
    InvalidPhysicalSize,
    #[error("unsupported camera intrinsics model {0:?}")]
    UnsupportedIntrinsicsModel(String),
    #[error("unsupported camera distortion model {0:?}")]
    UnsupportedDistortionModel(String),
    #[error(
        "resolved Camera Frame Registry entry does not match the Camera's exact frame reference"
    )]
    CameraFrameReferenceMismatch,
    #[error("camera frame convention cannot represent the PnP pose: {0}")]
    CameraFrameConvention(String),
    #[error("camera calibration cannot be represented by the selected PnP model")]
    InvalidPnpCalibration,
    #[error("portal pose estimation failed")]
    PoseEstimationFailed,
    #[error(transparent)]
    CameraCalibration(#[from] CameraCalibrationError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_registry::{CameraCalibration, FiniteF64};
    use pnp_core::Vector3;

    fn calibration(fx: f64) -> CameraCalibration {
        CameraCalibration {
            fx: FiniteF64(fx),
            fy: FiniteF64(fx),
            cx: FiniteF64(640.0),
            cy: FiniteF64(360.0),
            distortion_coefficients: vec![
                FiniteF64(0.012),
                FiniteF64(-0.004),
                FiniteF64(0.0007),
                FiniteF64(-0.0001),
            ],
        }
    }

    fn camera() -> Camera {
        let camera_frame = camera_frame();
        Camera {
            r#type: "rgb".into(),
            width: 1280,
            height: 720,
            frame_rate_hz: 30,
            image_encoding: "jpeg".into(),
            pixel_format: "rgb8".into(),
            row_stride_bytes: 0,
            color_space: "srgb".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "opencv_fisheye".into(),
            calibration: Some(calibration(420.0)),
            frame: RegistryRef {
                peer_id: camera_frame.peer_id.clone(),
                id: camera_frame.frame_id.clone(),
                hash: camera_frame.hash(),
            },
        }
    }

    fn camera_frame() -> FrameRegistryEntry {
        FrameRegistryEntry::ros_optical("bracketbot", "head_left_camera_optical")
    }

    fn empty_frame() -> CameraFrame {
        CameraFrame {
            frame: vec![],
            dynamic_intrinsics: None,
        }
    }

    fn frontal_fisheye_corners(
        size: f64,
        depth: f64,
        calibration: &CameraCalibration,
    ) -> [ImagePoint; 4] {
        let model = PnpCamera::opencv_fisheye(
            calibration.fx.0,
            calibration.fy.0,
            calibration.cx.0,
            calibration.cy.0,
            &calibration
                .distortion_coefficients
                .iter()
                .map(|value| value.0)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let half = size * 0.5;
        [
            Vector3::new(-half, -half, depth),
            Vector3::new(half, -half, depth),
            Vector3::new(half, half, depth),
            Vector3::new(-half, half, depth),
        ]
        .map(|point| {
            let pixel = model.project(point).unwrap();
            ImagePoint {
                x: pixel.x,
                y: pixel.y,
            }
        })
    }

    #[test]
    fn estimates_frontal_portal_with_fisheye_calibration() {
        let camera = camera();
        let portal = PortalDefinition {
            portal_id: "portal:office".into(),
            physical_size_m: 0.2,
        };
        let corners = frontal_fisheye_corners(
            portal.physical_size_m,
            2.0,
            camera.calibration.as_ref().unwrap(),
        );

        let observation =
            estimate_portal_observation(&camera, &camera_frame(), &empty_frame(), &portal, corners)
                .unwrap();
        let translation = observation.portal_to_camera.translation.unwrap();

        assert!(translation.x.abs() < 1e-6);
        assert!(translation.y.abs() < 1e-6);
        assert!((translation.z - 2.0).abs() < 1e-5);
        assert_eq!(observation.camera_frame, camera.frame);
        assert!(observation.confidence > 0.999);
    }

    #[test]
    fn rejects_model_mismatch_instead_of_guessing() {
        let mut camera = camera();
        camera.distortion_model = "none".into();
        let portal = PortalDefinition {
            portal_id: "portal:office".into(),
            physical_size_m: 0.2,
        };

        assert_eq!(
            estimate_portal_observation(
                &camera,
                &camera_frame(),
                &empty_frame(),
                &portal,
                [ImagePoint { x: 0.0, y: 0.0 }; 4],
            ),
            Err(PortalPnpError::UnsupportedDistortionModel("none".into()))
        );
    }

    #[test]
    fn rejects_invalid_portal_definition_before_solving() {
        let mut portal = PortalDefinition {
            portal_id: String::new(),
            physical_size_m: 0.2,
        };
        assert_eq!(
            estimate_portal_observation(
                &camera(),
                &camera_frame(),
                &empty_frame(),
                &portal,
                [ImagePoint { x: 0.0, y: 0.0 }; 4],
            ),
            Err(PortalPnpError::EmptyPortalId)
        );

        portal.portal_id = "portal:office".into();
        portal.physical_size_m = 0.0;
        assert_eq!(
            estimate_portal_observation(
                &camera(),
                &camera_frame(),
                &empty_frame(),
                &portal,
                [ImagePoint { x: 0.0, y: 0.0 }; 4],
            ),
            Err(PortalPnpError::InvalidPhysicalSize)
        );
    }

    #[test]
    fn rejects_a_frame_entry_that_does_not_match_the_camera_reference() {
        let portal = PortalDefinition {
            portal_id: "portal:office".into(),
            physical_size_m: 0.2,
        };
        let wrong_frame =
            FrameRegistryEntry::ros_optical("bracketbot", "head_right_camera_optical");

        assert_eq!(
            estimate_portal_observation(
                &camera(),
                &wrong_frame,
                &empty_frame(),
                &portal,
                [ImagePoint { x: 0.0, y: 0.0 }; 4],
            ),
            Err(PortalPnpError::CameraFrameReferenceMismatch)
        );
    }

    #[test]
    fn expresses_the_observation_in_the_registered_camera_convention() {
        let camera_frame = FrameRegistryEntry::opengl("park", "render_camera");
        let mut camera = camera();
        camera.frame = RegistryRef {
            peer_id: camera_frame.peer_id.clone(),
            id: camera_frame.frame_id.clone(),
            hash: camera_frame.hash(),
        };
        let portal = PortalDefinition {
            portal_id: "portal:office".into(),
            physical_size_m: 0.2,
        };
        let corners = frontal_fisheye_corners(
            portal.physical_size_m,
            2.0,
            camera.calibration.as_ref().unwrap(),
        );

        let observation =
            estimate_portal_observation(&camera, &camera_frame, &empty_frame(), &portal, corners)
                .unwrap();
        let translation = observation.portal_to_camera.translation.unwrap();

        assert!(translation.x.abs() < 1e-6);
        assert!(translation.y.abs() < 1e-6);
        assert!((translation.z + 2.0).abs() < 1e-5);
    }
}

//! Camera calibration resolution shared by metric Mapper implementations.

use auki_datatypes::camera::CameraFrame;
use auki_registry::{Camera, CameraCalibration, FiniteF64};
use thiserror::Error;

/// Resolve the calibration that applies to one published Camera frame.
///
/// A per-frame override wins when present. Otherwise the content-addressed
/// Camera Registry calibration is used. Image-space-only cameras may omit both,
/// but geometric Mappers fail closed rather than guessing intrinsics.
pub fn effective_camera_calibration(
    camera: &Camera,
    frame: &CameraFrame,
) -> Result<CameraCalibration, CameraCalibrationError> {
    let calibration = match &frame.dynamic_intrinsics {
        Some(dynamic) => CameraCalibration {
            fx: FiniteF64(dynamic.fx),
            fy: FiniteF64(dynamic.fy),
            cx: FiniteF64(dynamic.cx),
            cy: FiniteF64(dynamic.cy),
            distortion_coefficients: dynamic
                .distortion_coefficients
                .iter()
                .copied()
                .map(FiniteF64)
                .collect(),
        },
        None => camera
            .calibration
            .clone()
            .ok_or(CameraCalibrationError::Missing)?,
    };

    let mut calibrated_camera = camera.clone();
    calibrated_camera.calibration = Some(calibration.clone());
    calibrated_camera
        .validate_calibration()
        .map_err(|error| CameraCalibrationError::Invalid(error.to_string()))?;
    Ok(calibration)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CameraCalibrationError {
    #[error("camera has neither static calibration nor a per-frame dynamic override")]
    Missing,
    #[error("camera calibration is invalid: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_datatypes::camera::DynamicIntrinsics;
    use auki_registry::RegistryRef;

    fn camera(calibration: Option<CameraCalibration>) -> Camera {
        Camera {
            r#type: "rgb".into(),
            width: 640,
            height: 480,
            frame_rate_hz: 30,
            image_encoding: "jpeg".into(),
            pixel_format: "rgb8".into(),
            row_stride_bytes: 0,
            color_space: "srgb".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "brown_conrady".into(),
            calibration,
            frame: RegistryRef {
                peer_id: "robot".into(),
                id: "head_camera_optical".into(),
                hash: "frame-hash".into(),
            },
        }
    }

    fn calibration(fx: f64) -> CameraCalibration {
        CameraCalibration {
            fx: FiniteF64(fx),
            fy: FiniteF64(501.0),
            cx: FiniteF64(320.0),
            cy: FiniteF64(240.0),
            distortion_coefficients: vec![FiniteF64(0.1)],
        }
    }

    #[test]
    fn static_registry_calibration_is_the_fallback() {
        let expected = calibration(500.0);
        let frame = CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![],
        };
        assert_eq!(
            effective_camera_calibration(&camera(Some(expected.clone())), &frame).unwrap(),
            expected
        );
    }

    #[test]
    fn dynamic_intrinsics_override_static_calibration() {
        let frame = CameraFrame {
            dynamic_intrinsics: Some(DynamicIntrinsics {
                fx: 700.0,
                fy: 701.0,
                cx: 300.0,
                cy: 200.0,
                distortion_coefficients: vec![-0.2],
            }),
            frame: vec![],
        };
        let resolved =
            effective_camera_calibration(&camera(Some(calibration(500.0))), &frame).unwrap();
        assert_eq!(resolved.fx.0, 700.0);
        assert_eq!(resolved.distortion_coefficients[0].0, -0.2);
    }

    #[test]
    fn missing_or_invalid_calibration_fails_closed() {
        let missing = CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![],
        };
        assert_eq!(
            effective_camera_calibration(&camera(None), &missing),
            Err(CameraCalibrationError::Missing)
        );

        let invalid = CameraFrame {
            dynamic_intrinsics: Some(DynamicIntrinsics {
                fx: 0.0,
                fy: 1.0,
                cx: 0.0,
                cy: 0.0,
                distortion_coefficients: vec![],
            }),
            frame: vec![],
        };
        assert!(matches!(
            effective_camera_calibration(&camera(None), &invalid),
            Err(CameraCalibrationError::Invalid(_))
        ));
    }
}

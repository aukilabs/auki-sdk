//! Pure spatial math helpers for the Auki SDK.
//!
//! This crate is deliberately small and IO-free. Registry entries
//! declare coordinate conventions; datatypes carry poses; this crate
//! converts, composes, and eventually queries geometry over those
//! values.

use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_registry::{AxisConvention, AxisDirection, FrameRegistryEntry, Handedness, LengthUnit};
use std::{error, fmt};

pub type Matrix3 = [[f64; 3]; 3];
pub type Matrix4 = [[f64; 4]; 4];

const EPSILON: f64 = 1.0e-12;

#[derive(Debug, Clone, PartialEq)]
pub enum GeometryError {
    InvalidAxes(String),
    HandednessMismatch {
        frame_id: String,
        declared: Handedness,
        axes_determinant: i8,
    },
    ZeroQuaternion,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeometryError::InvalidAxes(msg) => write!(f, "invalid axes: {msg}"),
            GeometryError::HandednessMismatch {
                frame_id,
                declared,
                axes_determinant,
            } => write!(
                f,
                "frame {frame_id:?} declares {declared:?} handedness but axes have determinant {axes_determinant}"
            ),
            GeometryError::ZeroQuaternion => write!(f, "orientation quaternion has zero length"),
        }
    }
}

impl error::Error for GeometryError {}

pub type Result<T> = std::result::Result<T, GeometryError>;

/// Multiplier that converts a scalar length in `unit` to meters.
pub fn meters_per_unit(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Meters => 1.0,
        LengthUnit::Centimeters => 0.01,
        LengthUnit::Millimeters => 0.001,
    }
}

/// Signed axis-permutation matrix converting vectors expressed in
/// `from` into vectors expressed in `to`.
///
/// This is convention math only: no unit scale and no pose-log
/// geometry. The implementation uses a private semantic basis; callers
/// should think of the contract as direct convention A -> convention B.
pub fn axis_convention_matrix(from: &AxisConvention, to: &AxisConvention) -> Result<Matrix3> {
    validate_axes(from)?;
    validate_axes(to)?;
    let from_basis = basis_matrix(from);
    let to_basis = basis_matrix(to);
    Ok(mul3(transpose3(to_basis), from_basis))
}

/// Unit scale multiplied by [`axis_convention_matrix`] in homogeneous
/// row-major form. Translation column is zero because conventions do
/// not encode physical offsets between named frames.
pub fn convention_matrix(from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<Matrix4> {
    validate_frame(from)?;
    validate_frame(to)?;

    let axes = axis_convention_matrix(&from.axes, &to.axes)?;
    let scale = meters_per_unit(from.units) / meters_per_unit(to.units);
    Ok([
        [
            scale * axes[0][0],
            scale * axes[0][1],
            scale * axes[0][2],
            0.0,
        ],
        [
            scale * axes[1][0],
            scale * axes[1][1],
            scale * axes[1][2],
            0.0,
        ],
        [
            scale * axes[2][0],
            scale * axes[2][1],
            scale * axes[2][2],
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

/// Convert a length-bearing point coordinate from one declared frame
/// convention into another.
pub fn convert_point_convention(
    point: Vec3,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Vec3> {
    apply_matrix3_to_vec3(length_scaled_axis_matrix(from, to)?, point)
}

/// Convert a length-bearing displacement vector from one declared
/// frame convention into another.
///
/// For unitless ray directions, use [`convert_direction_convention`].
pub fn convert_vector_convention(
    vector: Vec3,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Vec3> {
    convert_point_convention(vector, from, to)
}

/// Convert a unitless direction from one declared frame convention into
/// another. This applies the signed axis permutation but no unit scale.
pub fn convert_direction_convention(
    direction: Vec3,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Vec3> {
    validate_frame(from)?;
    validate_frame(to)?;
    apply_matrix3_to_vec3(axis_convention_matrix(&from.axes, &to.axes)?, direction)
}

/// Re-express the same physical pose in another declared coordinate
/// convention.
///
/// This is not full `convert_pose`: it does not traverse a pose-log
/// graph or account for physical offsets between named frames. It is
/// the convention-only layer that full `convert_pose` will call while
/// composing frame edges.
pub fn convert_pose_convention(
    pose: &SpatialTransform,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<SpatialTransform> {
    validate_frame(from)?;
    validate_frame(to)?;

    let translation = pose
        .translation
        .clone()
        .map(|t| convert_vector_convention(t, from, to))
        .transpose()?;

    let orientation = pose
        .orientation
        .clone()
        .map(|q| convert_orientation_convention(q, from, to))
        .transpose()?;

    Ok(SpatialTransform {
        translation,
        orientation,
    })
}

/// Invert a transform from `from` into `to`, returning the transform from
/// `to` back into `from`.
///
/// A missing translation is treated as zero. A missing orientation is
/// treated as identity. The returned transform stores explicit translation
/// and orientation values.
pub fn inverse_spatial_transform(transform: &SpatialTransform) -> Result<SpatialTransform> {
    let rotation = spatial_transform_rotation(transform)?;
    let inverse_rotation = transpose3(rotation);
    let translation = spatial_transform_translation(transform);
    let inverse_translation = negate_vec3(apply_matrix3_to_vec3(inverse_rotation, translation)?);

    Ok(SpatialTransform {
        translation: Some(inverse_translation),
        orientation: Some(matrix_to_quat(inverse_rotation)?),
    })
}

/// Compose `from->mid` with `mid->to`, returning `from->to`.
///
/// The transform contract is `p_to = R_from_to * p_from + t_from_to`.
pub fn compose_spatial_transforms(
    from_to_mid: &SpatialTransform,
    mid_to_to: &SpatialTransform,
) -> Result<SpatialTransform> {
    let first_rotation = spatial_transform_rotation(from_to_mid)?;
    let second_rotation = spatial_transform_rotation(mid_to_to)?;
    let first_translation = spatial_transform_translation(from_to_mid);
    let second_translation = spatial_transform_translation(mid_to_to);

    let rotation = mul3(second_rotation, first_rotation);
    let rotated_translation = apply_matrix3_to_vec3(second_rotation, first_translation)?;
    let translation = add_vec3(rotated_translation, second_translation);

    Ok(SpatialTransform {
        translation: Some(translation),
        orientation: Some(matrix_to_quat(rotation)?),
    })
}

/// Given `common->from` and `common->to`, derive `from->to`.
pub fn relative_spatial_transform(
    common_to_from: &SpatialTransform,
    common_to_to: &SpatialTransform,
) -> Result<SpatialTransform> {
    let from_to_common = inverse_spatial_transform(common_to_from)?;
    compose_spatial_transforms(&from_to_common, common_to_to)
}

fn convert_orientation_convention(
    orientation: Quat,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Quat> {
    let basis = axis_convention_matrix(&from.axes, &to.axes)?;
    let rotation = quat_to_matrix(normalize_quat(orientation)?);
    let converted = mul3(mul3(basis, rotation), transpose3(basis));
    Ok(normalize_quat(matrix_to_quat(converted)?).expect("matrix_to_quat returns non-zero"))
}

fn length_scaled_axis_matrix(
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Matrix3> {
    validate_frame(from)?;
    validate_frame(to)?;
    let axes = axis_convention_matrix(&from.axes, &to.axes)?;
    let scale = meters_per_unit(from.units) / meters_per_unit(to.units);
    Ok(scale3(scale, axes))
}

fn validate_frame(entry: &FrameRegistryEntry) -> Result<()> {
    entry
        .validate()
        .map_err(|err| GeometryError::InvalidAxes(err.to_string()))?;
    let det = determinant3(basis_matrix(&entry.axes));
    let expected = match entry.handedness {
        Handedness::Right => 1,
        Handedness::Left => -1,
    };
    if det != expected {
        return Err(GeometryError::HandednessMismatch {
            frame_id: entry.frame_id.clone(),
            declared: entry.handedness,
            axes_determinant: det,
        });
    }
    Ok(())
}

fn validate_axes(axes: &AxisConvention) -> Result<()> {
    let entry = FrameRegistryEntry {
        frame_id: "<anonymous>".into(),
        handedness: if determinant3(basis_matrix(axes)) >= 0 {
            Handedness::Right
        } else {
            Handedness::Left
        },
        axes: *axes,
        units: LengthUnit::Meters,
    };
    entry
        .validate()
        .map_err(|err| GeometryError::InvalidAxes(err.to_string()))
}

fn basis_matrix(axes: &AxisConvention) -> Matrix3 {
    let x = direction_vector(axes.x);
    let y = direction_vector(axes.y);
    let z = direction_vector(axes.z);
    [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]]
}

fn direction_vector(direction: AxisDirection) -> [f64; 3] {
    match direction {
        AxisDirection::Right => [1.0, 0.0, 0.0],
        AxisDirection::Left => [-1.0, 0.0, 0.0],
        AxisDirection::Up => [0.0, 1.0, 0.0],
        AxisDirection::Down => [0.0, -1.0, 0.0],
        AxisDirection::Backward => [0.0, 0.0, 1.0],
        AxisDirection::Forward => [0.0, 0.0, -1.0],
    }
}

fn determinant3(m: Matrix3) -> i8 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det > 0.0 {
        1
    } else if det < 0.0 {
        -1
    } else {
        0
    }
}

fn transpose3(m: Matrix3) -> Matrix3 {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

fn mul3(a: Matrix3, b: Matrix3) -> Matrix3 {
    let mut out = [[0.0; 3]; 3];
    for row in 0..3 {
        for col in 0..3 {
            out[row][col] = a[row][0] * b[0][col] + a[row][1] * b[1][col] + a[row][2] * b[2][col];
        }
    }
    out
}

fn scale3(scale: f64, m: Matrix3) -> Matrix3 {
    [
        [scale * m[0][0], scale * m[0][1], scale * m[0][2]],
        [scale * m[1][0], scale * m[1][1], scale * m[1][2]],
        [scale * m[2][0], scale * m[2][1], scale * m[2][2]],
    ]
}

fn apply_matrix3_to_vec3(matrix: Matrix3, vector: Vec3) -> Result<Vec3> {
    Ok(Vec3 {
        x: matrix[0][0] * vector.x + matrix[0][1] * vector.y + matrix[0][2] * vector.z,
        y: matrix[1][0] * vector.x + matrix[1][1] * vector.y + matrix[1][2] * vector.z,
        z: matrix[2][0] * vector.x + matrix[2][1] * vector.y + matrix[2][2] * vector.z,
    })
}

fn spatial_transform_translation(transform: &SpatialTransform) -> Vec3 {
    transform.translation.clone().unwrap_or(Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
}

fn spatial_transform_rotation(transform: &SpatialTransform) -> Result<Matrix3> {
    let orientation = transform.orientation.clone().unwrap_or(Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    });
    Ok(quat_to_matrix(normalize_quat(orientation)?))
}

fn add_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn negate_vec3(v: Vec3) -> Vec3 {
    Vec3 {
        x: -v.x,
        y: -v.y,
        z: -v.z,
    }
}

fn normalize_quat(q: Quat) -> Result<Quat> {
    let norm = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
    if norm <= EPSILON {
        return Err(GeometryError::ZeroQuaternion);
    }
    Ok(Quat {
        x: q.x / norm,
        y: q.y / norm,
        z: q.z / norm,
        w: q.w / norm,
    })
}

fn quat_to_matrix(q: Quat) -> Matrix3 {
    let xx = q.x * q.x;
    let yy = q.y * q.y;
    let zz = q.z * q.z;
    let xy = q.x * q.y;
    let xz = q.x * q.z;
    let yz = q.y * q.z;
    let wx = q.w * q.x;
    let wy = q.w * q.y;
    let wz = q.w * q.z;

    [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)],
        [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)],
        [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)],
    ]
}

fn matrix_to_quat(m: Matrix3) -> Result<Quat> {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        Quat {
            w: 0.25 * s,
            x: (m[2][1] - m[1][2]) / s,
            y: (m[0][2] - m[2][0]) / s,
            z: (m[1][0] - m[0][1]) / s,
        }
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        Quat {
            w: (m[2][1] - m[1][2]) / s,
            x: 0.25 * s,
            y: (m[0][1] + m[1][0]) / s,
            z: (m[0][2] + m[2][0]) / s,
        }
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        Quat {
            w: (m[0][2] - m[2][0]) / s,
            x: (m[0][1] + m[1][0]) / s,
            y: 0.25 * s,
            z: (m[1][2] + m[2][1]) / s,
        }
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        Quat {
            w: (m[1][0] - m[0][1]) / s,
            x: (m[0][2] + m[2][0]) / s,
            y: (m[1][2] + m[2][1]) / s,
            z: 0.25 * s,
        }
    };
    normalize_quat(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_matrix3_close(actual: Matrix3, expected: Matrix3) {
        for row in 0..3 {
            for col in 0..3 {
                assert!(
                    (actual[row][col] - expected[row][col]).abs() < 1.0e-9,
                    "matrix mismatch at [{row}][{col}]: actual={} expected={}",
                    actual[row][col],
                    expected[row][col]
                );
            }
        }
    }

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert!((actual.x - expected.x).abs() < 1.0e-9, "x mismatch");
        assert!((actual.y - expected.y).abs() < 1.0e-9, "y mismatch");
        assert!((actual.z - expected.z).abs() < 1.0e-9, "z mismatch");
    }

    fn assert_quat_equivalent(actual: Quat, expected: Quat) {
        let same = (actual.x - expected.x).abs() < 1.0e-9
            && (actual.y - expected.y).abs() < 1.0e-9
            && (actual.z - expected.z).abs() < 1.0e-9
            && (actual.w - expected.w).abs() < 1.0e-9;
        let negated = (actual.x + expected.x).abs() < 1.0e-9
            && (actual.y + expected.y).abs() < 1.0e-9
            && (actual.z + expected.z).abs() < 1.0e-9
            && (actual.w + expected.w).abs() < 1.0e-9;
        assert!(same || negated, "actual={actual:?} expected={expected:?}");
    }

    fn assert_transform_close(actual: SpatialTransform, expected: SpatialTransform) {
        assert_vec3_close(actual.translation.unwrap(), expected.translation.unwrap());
        assert_quat_equivalent(actual.orientation.unwrap(), expected.orientation.unwrap());
    }

    fn vec3(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn quat_z_90() -> Quat {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        Quat {
            x: 0.0,
            y: 0.0,
            z: half,
            w: half,
        }
    }

    fn quat_x_90() -> Quat {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        Quat {
            x: half,
            y: 0.0,
            z: 0.0,
            w: half,
        }
    }

    fn transform(translation: Vec3, orientation: Quat) -> SpatialTransform {
        SpatialTransform {
            translation: Some(translation),
            orientation: Some(orientation),
        }
    }

    fn identity_transform() -> SpatialTransform {
        transform(
            vec3(0.0, 0.0, 0.0),
            Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
        )
    }

    fn apply_transform_to_point(transform: &SpatialTransform, point: Vec3) -> Vec3 {
        let rotation = quat_to_matrix(transform.orientation.clone().unwrap());
        let rotated = apply_matrix3_to_vec3(rotation, point).unwrap();
        let translation = transform.translation.clone().unwrap();
        vec3(
            rotated.x + translation.x,
            rotated.y + translation.y,
            rotated.z + translation.z,
        )
    }

    fn apply_matrix(matrix: Matrix3, vector: Vec3) -> Vec3 {
        apply_matrix3_to_vec3(matrix, vector).unwrap()
    }

    #[test]
    fn meters_per_unit_is_locked() {
        assert_eq!(meters_per_unit(LengthUnit::Meters), 1.0);
        assert_eq!(meters_per_unit(LengthUnit::Centimeters), 0.01);
        assert_eq!(meters_per_unit(LengthUnit::Millimeters), 0.001);
    }

    #[test]
    fn ros_optical_to_opengl_axis_matrix_is_locked() {
        let from = FrameRegistryEntry::ros_optical("camera");
        let to = FrameRegistryEntry::opengl("world");
        assert_matrix3_close(
            axis_convention_matrix(&from.axes, &to.axes).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]],
        );
    }

    #[test]
    fn ros_body_to_opengl_axis_matrix_is_locked() {
        let from = FrameRegistryEntry::ros_body("body");
        let to = FrameRegistryEntry::opengl("world");
        assert_matrix3_close(
            axis_convention_matrix(&from.axes, &to.axes).unwrap(),
            [[0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0]],
        );
    }

    #[test]
    fn unity_to_opengl_axis_matrix_is_locked() {
        let from = FrameRegistryEntry::unity("unity");
        let to = FrameRegistryEntry::opengl("world");
        assert_matrix3_close(
            axis_convention_matrix(&from.axes, &to.axes).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
        );
    }

    #[test]
    fn point_conversion_applies_axes_and_units() {
        let from = FrameRegistryEntry {
            frame_id: "source".into(),
            handedness: Handedness::Right,
            axes: FrameRegistryEntry::ros_optical("source").axes,
            units: LengthUnit::Centimeters,
        };
        let to = FrameRegistryEntry::opengl("target");
        let converted = convert_point_convention(
            Vec3 {
                x: 100.0,
                y: 200.0,
                z: 300.0,
            },
            &from,
            &to,
        )
        .unwrap();
        assert_vec3_close(
            converted,
            Vec3 {
                x: 1.0,
                y: -2.0,
                z: -3.0,
            },
        );
    }

    #[test]
    fn direction_conversion_does_not_apply_units() {
        let from = FrameRegistryEntry {
            frame_id: "source".into(),
            handedness: Handedness::Right,
            axes: FrameRegistryEntry::ros_optical("source").axes,
            units: LengthUnit::Centimeters,
        };
        let to = FrameRegistryEntry::opengl("target");
        let converted = convert_direction_convention(
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            &from,
            &to,
        )
        .unwrap();
        assert_vec3_close(
            converted,
            Vec3 {
                x: 1.0,
                y: -2.0,
                z: -3.0,
            },
        );
    }

    #[test]
    fn convention_matrix_round_trips_to_identity() {
        let frames = [
            FrameRegistryEntry::ros_body("body"),
            FrameRegistryEntry::ros_optical("optical"),
            FrameRegistryEntry::opengl("opengl"),
            FrameRegistryEntry::unity("unity"),
        ];
        for a in &frames {
            for b in &frames {
                let ab = axis_convention_matrix(&a.axes, &b.axes).unwrap();
                let ba = axis_convention_matrix(&b.axes, &a.axes).unwrap();
                assert_matrix3_close(
                    mul3(ba, ab),
                    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                );
            }
        }
    }

    #[test]
    fn handedness_mismatch_is_rejected() {
        let bad = FrameRegistryEntry {
            frame_id: "bad".into(),
            handedness: Handedness::Right,
            axes: FrameRegistryEntry::unity("unity").axes,
            units: LengthUnit::Meters,
        };
        let err = convention_matrix(&bad, &FrameRegistryEntry::opengl("world")).unwrap_err();
        assert!(matches!(err, GeometryError::HandednessMismatch { .. }));
    }

    #[test]
    fn convert_pose_convention_reexpresses_translation_and_orientation() {
        let from = FrameRegistryEntry::ros_optical("camera");
        let to = FrameRegistryEntry::opengl("world");
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let pose = SpatialTransform {
            translation: Some(Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: half,
                w: half,
            }),
        };

        let converted = convert_pose_convention(&pose, &from, &to).unwrap();
        assert_vec3_close(
            converted.translation.unwrap(),
            Vec3 {
                x: 1.0,
                y: -2.0,
                z: -3.0,
            },
        );

        let basis = axis_convention_matrix(&from.axes, &to.axes).unwrap();
        let source_rotation = quat_to_matrix(pose.orientation.unwrap());
        let expected_rotation = mul3(mul3(basis, source_rotation), transpose3(basis));
        assert_quat_equivalent(
            converted.orientation.unwrap(),
            matrix_to_quat(expected_rotation).unwrap(),
        );
    }

    #[test]
    fn converted_orientation_preserves_rotated_vectors() {
        let from = FrameRegistryEntry::ros_body("body");
        let to = FrameRegistryEntry::opengl("world");
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let source_q = Quat {
            x: half,
            y: 0.0,
            z: 0.0,
            w: half,
        };
        let pose = SpatialTransform {
            translation: None,
            orientation: Some(source_q.clone()),
        };
        let converted = convert_pose_convention(&pose, &from, &to).unwrap();
        let source_vector = Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };

        let source_after = apply_matrix(quat_to_matrix(source_q), source_vector.clone());
        let expected_target_after = convert_direction_convention(source_after, &from, &to).unwrap();

        let target_vector = convert_direction_convention(source_vector, &from, &to).unwrap();
        let actual_target_after = apply_matrix(
            quat_to_matrix(converted.orientation.unwrap()),
            target_vector,
        );

        assert_vec3_close(actual_target_after, expected_target_after);
    }

    #[test]
    fn inverse_spatial_transform_inverts_identity() {
        let inverse = inverse_spatial_transform(&identity_transform()).unwrap();
        assert_transform_close(inverse, identity_transform());
    }

    #[test]
    fn inverse_spatial_transform_round_trips_point() {
        let camera_to_slam = transform(vec3(1.0, 2.0, 3.0), quat_z_90());
        let slam_to_camera = inverse_spatial_transform(&camera_to_slam).unwrap();

        let camera_point = vec3(4.0, -1.0, 2.0);
        let slam_point = apply_transform_to_point(&camera_to_slam, camera_point.clone());
        let round_tripped = apply_transform_to_point(&slam_to_camera, slam_point);

        assert_vec3_close(round_tripped, camera_point);
    }

    #[test]
    fn compose_spatial_transforms_maps_source_to_final_target() {
        let a_to_b = transform(vec3(1.0, 0.0, 0.0), quat_z_90());
        let b_to_c = transform(vec3(0.0, 2.0, 0.0), quat_x_90());

        let a_to_c = compose_spatial_transforms(&a_to_b, &b_to_c).unwrap();

        let point_in_a = vec3(3.0, 4.0, 5.0);
        let via_b = apply_transform_to_point(
            &b_to_c,
            apply_transform_to_point(&a_to_b, point_in_a.clone()),
        );
        let direct = apply_transform_to_point(&a_to_c, point_in_a);

        assert_vec3_close(direct, via_b);
    }

    #[test]
    fn compose_spatial_transforms_with_inverse_returns_identity() {
        let a_to_b = transform(vec3(1.0, 2.0, 3.0), quat_z_90());
        let b_to_a = inverse_spatial_transform(&a_to_b).unwrap();
        let composed = compose_spatial_transforms(&a_to_b, &b_to_a).unwrap();

        assert_transform_close(composed, identity_transform());
    }

    #[test]
    fn relative_spatial_transform_derives_target_to_target_from_common_source() {
        let camera_to_slam = transform(vec3(1.0, 0.0, 0.0), quat_z_90());
        let slam_to_domain = transform(vec3(0.0, 5.0, 0.0), quat_x_90());
        let camera_to_domain =
            compose_spatial_transforms(&camera_to_slam, &slam_to_domain).unwrap();

        let derived_slam_to_domain =
            relative_spatial_transform(&camera_to_slam, &camera_to_domain).unwrap();

        assert_transform_close(derived_slam_to_domain, slam_to_domain);
    }

    #[test]
    fn spatial_transform_helpers_treat_missing_parts_as_identity_components() {
        let only_translation = SpatialTransform {
            translation: Some(vec3(2.0, 3.0, 4.0)),
            orientation: None,
        };
        let only_rotation = SpatialTransform {
            translation: None,
            orientation: Some(quat_z_90()),
        };

        let composed = compose_spatial_transforms(&only_translation, &only_rotation).unwrap();

        assert_vec3_close(composed.translation.unwrap(), vec3(-3.0, 2.0, 4.0));
        assert_quat_equivalent(composed.orientation.unwrap(), quat_z_90());
    }

    #[test]
    fn spatial_transform_helpers_reject_zero_quaternion() {
        let bad = SpatialTransform {
            translation: None,
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            }),
        };

        assert!(matches!(
            inverse_spatial_transform(&bad),
            Err(GeometryError::ZeroQuaternion)
        ));
        assert!(matches!(
            compose_spatial_transforms(&bad, &identity_transform()),
            Err(GeometryError::ZeroQuaternion)
        ));
    }
}

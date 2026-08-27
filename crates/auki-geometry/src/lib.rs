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
    MixedHandednessConversion {
        from_frame_id: String,
        to_frame_id: String,
    },
    MixedUnitConversion {
        from_frame_id: String,
        to_frame_id: String,
    },
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
            GeometryError::MixedHandednessConversion {
                from_frame_id,
                to_frame_id,
            } => write!(
                f,
                "cannot convert only one side of a transform between frame {from_frame_id:?} and {to_frame_id:?}: they declare different handedness, so the result would not be a proper rotation"
            ),
            GeometryError::MixedUnitConversion {
                from_frame_id,
                to_frame_id,
            } => write!(
                f,
                "cannot convert only one side of a transform between frame {from_frame_id:?} and {to_frame_id:?}: they declare different length units, which would require a scale factor that a rotation quaternion cannot represent"
            ),
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
        .map(|t| convert_vector_convention(t, from, to))
        .transpose()?;

    let orientation = pose
        .orientation
        .map(|q| convert_orientation_convention(q, from, to))
        .transpose()?;

    Ok(SpatialTransform {
        translation,
        orientation,
    })
}

/// Re-express only the *source* side of a `from -> to` transform in a
/// new coordinate convention, leaving the target side untouched.
///
/// `transform` maps points expressed in `from`'s convention into
/// whatever convention its target side already uses. This returns the
/// equivalent transform that instead accepts points expressed in `to`'s
/// convention. Use this to reinterpret how a transform's input is read
/// (e.g. a producer switching axis convention) without touching the
/// convention its output lands in. For re-expressing both ends of a
/// pose together, use [`convert_pose_convention`] instead.
///
/// Returns [`GeometryError::MixedHandednessConversion`] if `from` and
/// `to` declare different handedness: converting only one side between
/// differently-handed conventions is not representable as a proper
/// rotation, and therefore not as a quaternion. Returns
/// [`GeometryError::MixedUnitConversion`] if `from` and `to` declare
/// different length units: the resulting map would be a scaled rotation
/// (a similarity transform), which a rotation quaternion cannot
/// represent either.
pub fn convert_transform_source_convention(
    transform: &SpatialTransform,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<SpatialTransform> {
    validate_frame(from)?;
    validate_frame(to)?;
    reject_mixed_handedness(from, to)?;
    reject_mixed_units(from, to)?;

    // Prepending the pure axis conversion `to -> from` is exactly
    // `T_source_target ∘ T_new_source_old_source`: it leaves the target
    // side alone and only changes what convention the input is read in.
    let new_source_to_old_source = axis_only_transform(&to.axes, &from.axes)?;
    compose_spatial_transforms(&new_source_to_old_source, transform)
}

/// Re-express only the *target* side of a `from -> to` transform in a
/// new coordinate convention, leaving the source side untouched.
///
/// `transform` maps points into `from`'s convention. This returns the
/// equivalent transform that instead produces points in `to`'s
/// convention, without touching the convention its input side already
/// expects. Use this to retarget where a transform's output lands (e.g.
/// a consumer switching render convention) without touching how its
/// input is interpreted. For re-expressing both ends of a pose
/// together, use [`convert_pose_convention`] instead.
///
/// Returns [`GeometryError::MixedHandednessConversion`] or
/// [`GeometryError::MixedUnitConversion`] for the same reasons as
/// [`convert_transform_source_convention`].
pub fn convert_transform_target_convention(
    transform: &SpatialTransform,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<SpatialTransform> {
    validate_frame(from)?;
    validate_frame(to)?;
    reject_mixed_handedness(from, to)?;
    reject_mixed_units(from, to)?;

    // Appending the pure axis conversion `from -> to` is exactly
    // `T_old_target_new_target ∘ T_source_target`: it leaves the source
    // side alone and only changes what convention the output lands in.
    let old_target_to_new_target = axis_only_transform(&from.axes, &to.axes)?;
    compose_spatial_transforms(transform, &old_target_to_new_target)
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

/// Build a 4×4 homogeneous transformation matrix from a `SpatialTransform`.
///
/// The matrix has the rotation in the upper-left 3×3, the translation in
/// the right column, and `[0, 0, 0, 1]` as the bottom row. Missing
/// translation is treated as zero; missing orientation is treated as
/// identity — matching the input contract of the PR #193 composition
/// helpers.
pub fn spatial_transform_to_matrix4(transform: &SpatialTransform) -> Result<Matrix4> {
    let rotation = spatial_transform_rotation(transform)?;
    let translation = spatial_transform_translation(transform);
    Ok([
        [
            rotation[0][0],
            rotation[0][1],
            rotation[0][2],
            translation.x,
        ],
        [
            rotation[1][0],
            rotation[1][1],
            rotation[1][2],
            translation.y,
        ],
        [
            rotation[2][0],
            rotation[2][1],
            rotation[2][2],
            translation.z,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

/// Decompose a 4×4 homogeneous transformation matrix into a
/// `SpatialTransform`.
///
/// Translation comes from the right column (`matrix[0..3][3]`).
/// Rotation comes from the upper-left 3×3 submatrix via `matrix_to_quat`,
/// which normalizes the result — small numerical drift in the rotation
/// submatrix is tolerated. The bottom row of the input is not validated;
/// callers are responsible for supplying a proper homogeneous transform.
pub fn spatial_transform_from_matrix4(matrix: Matrix4) -> Result<SpatialTransform> {
    let rotation: Matrix3 = [
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    ];
    let translation = Vec3 {
        x: matrix[0][3],
        y: matrix[1][3],
        z: matrix[2][3],
    };
    let orientation = matrix_to_quat(rotation)?;
    Ok(SpatialTransform {
        translation: Some(translation),
        orientation: Some(orientation),
    })
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

/// A zero-translation transform holding only the signed axis permutation
/// from `from` to `to`, suitable as an identity-translation leg to plug
/// into [`compose_spatial_transforms`].
fn axis_only_transform(from: &AxisConvention, to: &AxisConvention) -> Result<SpatialTransform> {
    Ok(SpatialTransform {
        translation: Some(Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }),
        orientation: Some(matrix_to_quat(axis_convention_matrix(from, to)?)?),
    })
}

fn reject_mixed_handedness(from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<()> {
    if from.handedness != to.handedness {
        return Err(GeometryError::MixedHandednessConversion {
            from_frame_id: from.frame_id.clone(),
            to_frame_id: to.frame_id.clone(),
        });
    }
    Ok(())
}

fn reject_mixed_units(from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<()> {
    if from.units != to.units {
        return Err(GeometryError::MixedUnitConversion {
            from_frame_id: from.frame_id.clone(),
            to_frame_id: to.frame_id.clone(),
        });
    }
    Ok(())
}

fn validate_axes(axes: &AxisConvention) -> Result<()> {
    let entry = FrameRegistryEntry {
        peer_id: String::new(),
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
    transform.translation.unwrap_or(Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
}

fn spatial_transform_rotation(transform: &SpatialTransform) -> Result<Matrix3> {
    let orientation = transform.orientation.unwrap_or(Quat {
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

    fn assert_matrix4_close(actual: Matrix4, expected: Matrix4) {
        for row in 0..4 {
            for col in 0..4 {
                assert!(
                    (actual[row][col] - expected[row][col]).abs() < 1.0e-9,
                    "matrix4 mismatch at [{row}][{col}]: actual={} expected={}",
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
        let rotation = quat_to_matrix(transform.orientation.unwrap());
        let rotated = apply_matrix3_to_vec3(rotation, point).unwrap();
        let translation = transform.translation.unwrap();
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
        let from = FrameRegistryEntry::ros_optical("", "camera");
        let to = FrameRegistryEntry::opengl("", "world");
        assert_matrix3_close(
            axis_convention_matrix(&from.axes, &to.axes).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]],
        );
    }

    #[test]
    fn ros_body_to_opengl_axis_matrix_is_locked() {
        let from = FrameRegistryEntry::ros_body("", "body");
        let to = FrameRegistryEntry::opengl("", "world");
        assert_matrix3_close(
            axis_convention_matrix(&from.axes, &to.axes).unwrap(),
            [[0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0]],
        );
    }

    #[test]
    fn unity_to_opengl_axis_matrix_is_locked() {
        let from = FrameRegistryEntry::unity("", "unity");
        let to = FrameRegistryEntry::opengl("", "world");
        assert_matrix3_close(
            axis_convention_matrix(&from.axes, &to.axes).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
        );
    }

    #[test]
    fn point_conversion_applies_axes_and_units() {
        let from = FrameRegistryEntry {
            peer_id: String::new(),
            frame_id: "source".into(),
            handedness: Handedness::Right,
            axes: FrameRegistryEntry::ros_optical("", "source").axes,
            units: LengthUnit::Centimeters,
        };
        let to = FrameRegistryEntry::opengl("", "target");
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
            peer_id: String::new(),
            frame_id: "source".into(),
            handedness: Handedness::Right,
            axes: FrameRegistryEntry::ros_optical("", "source").axes,
            units: LengthUnit::Centimeters,
        };
        let to = FrameRegistryEntry::opengl("", "target");
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
            FrameRegistryEntry::ros_body("", "body"),
            FrameRegistryEntry::ros_optical("", "optical"),
            FrameRegistryEntry::opengl("", "opengl"),
            FrameRegistryEntry::unity("", "unity"),
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
            peer_id: String::new(),
            frame_id: "bad".into(),
            handedness: Handedness::Right,
            axes: FrameRegistryEntry::unity("", "unity").axes,
            units: LengthUnit::Meters,
        };
        let err = convention_matrix(&bad, &FrameRegistryEntry::opengl("", "world")).unwrap_err();
        assert!(matches!(err, GeometryError::HandednessMismatch { .. }));
    }

    #[test]
    fn convert_pose_convention_reexpresses_translation_and_orientation() {
        let from = FrameRegistryEntry::ros_optical("", "camera");
        let to = FrameRegistryEntry::opengl("", "world");
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
        let from = FrameRegistryEntry::ros_body("", "body");
        let to = FrameRegistryEntry::opengl("", "world");
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let source_q = Quat {
            x: half,
            y: 0.0,
            z: 0.0,
            w: half,
        };
        let pose = SpatialTransform {
            translation: None,
            orientation: Some(source_q),
        };
        let converted = convert_pose_convention(&pose, &from, &to).unwrap();
        let source_vector = Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };

        let source_after = apply_matrix(quat_to_matrix(source_q), source_vector);
        let expected_target_after = convert_direction_convention(source_after, &from, &to).unwrap();

        let target_vector = convert_direction_convention(source_vector, &from, &to).unwrap();
        let actual_target_after = apply_matrix(
            quat_to_matrix(converted.orientation.unwrap()),
            target_vector,
        );

        assert_vec3_close(actual_target_after, expected_target_after);
    }

    #[test]
    fn source_and_target_conversion_compose_to_full_pose_conversion() {
        let from = FrameRegistryEntry::ros_optical("", "camera");
        let to = FrameRegistryEntry::opengl("", "world");
        let t = transform(vec3(1.0, 2.0, 3.0), quat_z_90());

        let source_then_target = convert_transform_target_convention(
            &convert_transform_source_convention(&t, &from, &to).unwrap(),
            &from,
            &to,
        )
        .unwrap();

        let full = convert_pose_convention(&t, &from, &to).unwrap();

        assert_transform_close(source_then_target, full);
    }

    #[test]
    fn convert_transform_source_convention_leaves_target_side_unchanged() {
        let from = FrameRegistryEntry::ros_optical("", "camera");
        let to = FrameRegistryEntry::opengl("", "camera_alt");
        let a_to_b = transform(vec3(1.0, 2.0, 3.0), quat_z_90());

        let converted = convert_transform_source_convention(&a_to_b, &from, &to).unwrap();

        // Feeding a point already expressed in `to`'s convention through the
        // converted transform should land on the same target-side point as
        // feeding the equivalent `from`-convention point through the
        // original transform.
        let point_in_to = vec3(4.0, -1.0, 2.0);
        let point_in_from = convert_point_convention(point_in_to, &to, &from).unwrap();

        let via_original = apply_transform_to_point(&a_to_b, point_in_from);
        let via_converted = apply_transform_to_point(&converted, point_in_to);

        assert_vec3_close(via_converted, via_original);
    }

    #[test]
    fn convert_transform_target_convention_leaves_source_side_unchanged() {
        let from = FrameRegistryEntry::ros_optical("", "world_a");
        let to = FrameRegistryEntry::opengl("", "world_b");
        let a_to_b = transform(vec3(1.0, 2.0, 3.0), quat_z_90());

        let converted = convert_transform_target_convention(&a_to_b, &from, &to).unwrap();

        // Feeding the same source-side point through both transforms should
        // produce target-side points that are the same physical point, just
        // re-expressed in the new target convention.
        let point_in_from = vec3(4.0, -1.0, 2.0);
        let via_original = apply_transform_to_point(&a_to_b, point_in_from);
        let via_converted = apply_transform_to_point(&converted, point_in_from);

        let expected = convert_point_convention(via_original, &from, &to).unwrap();
        assert_vec3_close(via_converted, expected);
    }

    #[test]
    fn one_sided_conversion_rejects_mixed_handedness() {
        let from = FrameRegistryEntry::opengl("", "world");
        let to = FrameRegistryEntry::unity("", "unity");
        let t = identity_transform();

        assert!(matches!(
            convert_transform_source_convention(&t, &from, &to),
            Err(GeometryError::MixedHandednessConversion { .. })
        ));
        assert!(matches!(
            convert_transform_target_convention(&t, &from, &to),
            Err(GeometryError::MixedHandednessConversion { .. })
        ));
    }

    #[test]
    fn one_sided_conversion_rejects_mixed_units() {
        let from = FrameRegistryEntry::opengl("", "world");
        let to = FrameRegistryEntry {
            peer_id: String::new(),
            frame_id: "world_cm".into(),
            handedness: Handedness::Right,
            axes: FrameRegistryEntry::opengl("", "world_cm").axes,
            units: LengthUnit::Centimeters,
        };
        let t = identity_transform();

        assert!(matches!(
            convert_transform_source_convention(&t, &from, &to),
            Err(GeometryError::MixedUnitConversion { .. })
        ));
        assert!(matches!(
            convert_transform_target_convention(&t, &from, &to),
            Err(GeometryError::MixedUnitConversion { .. })
        ));
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
        let slam_point = apply_transform_to_point(&camera_to_slam, camera_point);
        let round_tripped = apply_transform_to_point(&slam_to_camera, slam_point);

        assert_vec3_close(round_tripped, camera_point);
    }

    #[test]
    fn compose_spatial_transforms_maps_source_to_final_target() {
        let a_to_b = transform(vec3(1.0, 0.0, 0.0), quat_z_90());
        let b_to_c = transform(vec3(0.0, 2.0, 0.0), quat_x_90());

        let a_to_c = compose_spatial_transforms(&a_to_b, &b_to_c).unwrap();

        let point_in_a = vec3(3.0, 4.0, 5.0);
        let via_b =
            apply_transform_to_point(&b_to_c, apply_transform_to_point(&a_to_b, point_in_a));
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

    #[test]
    fn spatial_transform_to_matrix4_identity() {
        let identity = SpatialTransform {
            translation: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&identity).unwrap(),
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_to_matrix4_translation_only() {
        let t = SpatialTransform {
            translation: Some(Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&t).unwrap(),
            [
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 2.0],
                [0.0, 0.0, 1.0, 3.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_to_matrix4_rotation_only() {
        // 90° rotation around +Z: x→y, y→−x
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let t = SpatialTransform {
            translation: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: half,
                w: half,
            }),
        };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&t).unwrap(),
            [
                [0.0, -1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_to_matrix4_treats_missing_as_zero_identity() {
        // Both None: should produce 4x4 identity.
        let none = SpatialTransform {
            translation: None,
            orientation: None,
        };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&none).unwrap(),
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_from_matrix4_round_trip() {
        // Build a pose, send it to matrix4, decode it back. Should round-trip.
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let original = SpatialTransform {
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
        let matrix = spatial_transform_to_matrix4(&original).unwrap();
        let decoded = spatial_transform_from_matrix4(matrix).unwrap();
        assert_vec3_close(decoded.translation.unwrap(), original.translation.unwrap());
        assert_quat_equivalent(decoded.orientation.unwrap(), original.orientation.unwrap());
    }

    #[test]
    fn spatial_transform_from_matrix4_identity() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let decoded = spatial_transform_from_matrix4(identity).unwrap();
        assert_vec3_close(
            decoded.translation.unwrap(),
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        );
        assert_quat_equivalent(
            decoded.orientation.unwrap(),
            Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
        );
    }
}

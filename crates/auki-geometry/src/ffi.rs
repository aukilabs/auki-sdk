use crate::core;

uniffi::setup_scaffolding!();

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum GeometryBindingError {
    #[error("JSON is not valid: {message}")]
    InvalidJson { message: String },
    #[error("axes are invalid: {message}")]
    InvalidAxes { message: String },
    #[error(
        "frame {frame_id} declares {declared} handedness but axes have determinant {axes_determinant}"
    )]
    HandednessMismatch {
        frame_id: String,
        declared: String,
        axes_determinant: i8,
    },
    #[error("orientation quaternion has zero length")]
    ZeroQuaternion,
}

#[uniffi::export]
pub fn meters_per_unit_json(unit: String) -> Result<f64, GeometryBindingError> {
    core::meters_per_unit_json(&unit).map_err(Into::into)
}

#[uniffi::export]
pub fn axis_convention_matrix_json(
    from_axes_json: String,
    to_axes_json: String,
) -> Result<String, GeometryBindingError> {
    core::axis_convention_matrix_json(&from_axes_json, &to_axes_json).map_err(Into::into)
}

#[uniffi::export]
pub fn convention_matrix_json(
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, GeometryBindingError> {
    core::convention_matrix_json(&from_frame_json, &to_frame_json).map_err(Into::into)
}

#[uniffi::export]
pub fn convert_point_convention_json(
    point_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, GeometryBindingError> {
    core::convert_point_convention_json(&point_json, &from_frame_json, &to_frame_json)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn convert_vector_convention_json(
    vector_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, GeometryBindingError> {
    core::convert_vector_convention_json(&vector_json, &from_frame_json, &to_frame_json)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn convert_direction_convention_json(
    direction_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, GeometryBindingError> {
    core::convert_direction_convention_json(&direction_json, &from_frame_json, &to_frame_json)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn convert_pose_convention_json(
    pose_json: String,
    from_frame_json: String,
    to_frame_json: String,
) -> Result<String, GeometryBindingError> {
    core::convert_pose_convention_json(&pose_json, &from_frame_json, &to_frame_json)
        .map_err(Into::into)
}

impl From<core::GeometryError> for GeometryBindingError {
    fn from(err: core::GeometryError) -> Self {
        match err {
            core::GeometryError::InvalidJson(message) => Self::InvalidJson { message },
            core::GeometryError::InvalidAxes(message) => Self::InvalidAxes { message },
            core::GeometryError::HandednessMismatch {
                frame_id,
                declared,
                axes_determinant,
            } => Self::HandednessMismatch {
                frame_id,
                declared: format!("{declared:?}"),
                axes_determinant,
            },
            core::GeometryError::ZeroQuaternion => Self::ZeroQuaternion,
        }
    }
}

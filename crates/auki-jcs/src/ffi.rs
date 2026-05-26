use crate::core;

uniffi::setup_scaffolding!();

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum JcsError {
    #[error("JSON is not valid: {message}")]
    InvalidJson { message: String },
}

#[uniffi::export]
pub fn canonicalize_json(json: String) -> Result<Vec<u8>, JcsError> {
    core::canonicalize_json_str(&json).map_err(|err| JcsError::InvalidJson {
        message: err.to_string(),
    })
}

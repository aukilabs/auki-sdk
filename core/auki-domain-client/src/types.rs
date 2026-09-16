use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// Limits for buffered operations and each streaming request/callback.
#[derive(Clone, Copy, Debug)]
pub struct DataLimits {
    pub request_timeout: Duration,
    pub max_metadata_bytes: usize,
    pub max_data_bytes: usize,
}

impl Default for DataLimits {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            max_metadata_bytes: 1024 * 1024,
            max_data_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Server filters; this metadata endpoint does not support pagination yet.
#[derive(Clone, Debug, Default)]
pub struct DataListQuery {
    pub ids: Vec<Uuid>,
    pub name: Option<String>,
    pub data_type: Option<String>,
}

/// The existing Domain Server metadata contract, also used by Posemesh clients.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DataMetadata {
    pub id: Uuid,
    pub domain_id: Uuid,
    pub name: String,
    pub data_type: String,
    pub size: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Named writes use the simple upload endpoint, which returns HTTP 409 for an
/// existing name. Use `ById` to replace an existing item's contents.
/// Updating an ID requires an existing item and preserves its name/type.
#[derive(Clone, Copy, Debug)]
pub enum DataWrite<'a> {
    Named { name: &'a str, data_type: &'a str },
    ById(Uuid),
}

/// Streaming bounds are independent of the smaller buffered-operation limit.
#[derive(Clone, Copy, Debug)]
pub struct TransferOptions {
    pub max_bytes: u64,
    /// Maximum download callback chunk or upload part allocation (1–64 MiB).
    pub max_chunk_bytes: usize,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024 * 1024,
            max_chunk_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Existing pose fields, unmodified from the Domain Server contract.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PortalPose {
    pub id: Uuid,
    pub short_id: String,
    pub domain_id: Uuid,
    pub reported_size: f64,
    pub px: f64,
    pub py: f64,
    pub pz: f64,
    pub rx: f64,
    pub ry: f64,
    pub rz: f64,
    pub rw: f64,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub vertical_accuracy: Option<f64>,
    pub horizontal_accuracy: Option<f64>,
    pub gps_timestamp: Option<f64>,
    pub scanner_device_id: String,
    pub scanner_device_name: String,
    pub scanner_device_model: String,
    pub placed_at: DateTime<Utc>,
}

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::time::Duration;
use uuid::Uuid;

/// Limits for explicitly buffered operations. Streaming is a separate milestone.
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
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
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

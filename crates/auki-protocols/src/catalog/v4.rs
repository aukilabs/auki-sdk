//! `/auki/auth/1/resources/0.4.0` Map Log catalog payload codec.
//!
//! This is deliberately separate from v0.2/v0.3: their resource variants are
//! closed, so adding `map_log` in place would make older peers reject a whole
//! catalog response. v0.4 is map-focused; callers continue using older
//! protocols for sensor, pose, and message-channel discovery.

use auki_registry::RegistryRef;
use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Exact authenticated resource-catalog 0.4.0 protocol identifier.
pub const ID: &str = crate::ids::RESOURCES_V0_4_0;

pub const MAX_RESOURCES_FRAME_BYTES: u32 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesRequest {}

impl ResourcesRequest {
    pub fn all() -> Self {
        Self {}
    }
}

/// One Map Log row. The Map Registry ref pins its grid contract; the clock ref
/// defines the timestamp of every MapUpdate segment entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapLogResource {
    pub source_peer_id: String,
    pub writer_peer_id: String,
    pub resource_id: String,
    pub map: RegistryRef,
    pub clock: RegistryRef,
}

impl MapLogResource {
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        if self.source_peer_id.is_empty()
            || self.writer_peer_id.is_empty()
            || self.resource_id.is_empty()
            || self.map.peer_id.is_empty()
            || self.map.id.is_empty()
            || self.map.hash.is_empty()
            || self.clock.peer_id.is_empty()
            || self.clock.id.is_empty()
            || self.clock.hash.is_empty()
        {
            return Err(ResourcesProtocolError::Validation(
                "map log contains an empty identity field".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesResponse {
    pub resources: Vec<MapLogResource>,
}

impl ResourcesResponse {
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        self.resources.iter().try_for_each(MapLogResource::validate)
    }
}

#[derive(Debug, Error)]
pub enum ResourcesProtocolError {
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("decode: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("validation: {0}")]
    Validation(String),
    #[error("frame is empty")]
    EmptyFrame,
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

pub async fn write_resources_request<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    request: &ResourcesRequest,
) -> Result<(), ResourcesProtocolError> {
    write_json(stream, request).await
}
pub async fn read_resources_request<S: AsyncReadExt + Unpin>(
    stream: &mut S,
) -> Result<ResourcesRequest, ResourcesProtocolError> {
    read_json(stream).await
}
pub async fn write_resources_response<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    response: &ResourcesResponse,
) -> Result<(), ResourcesProtocolError> {
    response.validate()?;
    write_json(stream, response).await
}
pub async fn read_resources_response<S: AsyncReadExt + Unpin>(
    stream: &mut S,
) -> Result<ResourcesResponse, ResourcesProtocolError> {
    let r: ResourcesResponse = read_json(stream).await?;
    r.validate()?;
    Ok(r)
}

async fn write_json<S: AsyncWriteExt + Unpin, T: Serialize>(
    stream: &mut S,
    value: &T,
) -> Result<(), ResourcesProtocolError> {
    let bytes = serde_json::to_vec(value).map_err(ResourcesProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_RESOURCES_FRAME_BYTES as u64 {
        return Err(ResourcesProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_RESOURCES_FRAME_BYTES as u64,
        });
    }
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .map_err(ResourcesProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(ResourcesProtocolError::Io)?;
    stream.flush().await.map_err(ResourcesProtocolError::Io)
}
async fn read_json<S: AsyncReadExt + Unpin, T: for<'de> Deserialize<'de>>(
    stream: &mut S,
) -> Result<T, ResourcesProtocolError> {
    let mut len = [0; 4];
    stream
        .read_exact(&mut len)
        .await
        .map_err(ResourcesProtocolError::Io)?;
    let len = u32::from_be_bytes(len);
    if len == 0 {
        return Err(ResourcesProtocolError::EmptyFrame);
    }
    if len > MAX_RESOURCES_FRAME_BYTES {
        return Err(ResourcesProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_RESOURCES_FRAME_BYTES as u64,
        });
    }
    let mut bytes = vec![0; len as usize];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(ResourcesProtocolError::Io)?;
    serde_json::from_slice(&bytes).map_err(ResourcesProtocolError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;
    fn r(id: &str) -> RegistryRef {
        RegistryRef {
            peer_id: "galbot".into(),
            id: id.into(),
            hash: "hash".into(),
        }
    }
    #[tokio::test]
    async fn map_row_round_trips() {
        let response = ResourcesResponse {
            resources: vec![MapLogResource {
                source_peer_id: "galbot".into(),
                writer_peer_id: "galbot".into(),
                resource_id: "occupancy".into(),
                map: r("occupancy"),
                clock: r("clock"),
            }],
        };
        let mut cursor = Cursor::new(Vec::new());
        write_resources_response(&mut cursor, &response)
            .await
            .unwrap();
        cursor.set_position(0);
        assert_eq!(
            read_resources_response(&mut cursor).await.unwrap(),
            response
        );
    }

    #[tokio::test]
    async fn map_catalog_framed_bytes_are_locked() {
        let response = ResourcesResponse {
            resources: vec![MapLogResource {
                source_peer_id: "galbot".into(),
                writer_peer_id: "galbot".into(),
                resource_id: "occupancy".into(),
                map: r("occupancy"),
                clock: r("clock"),
            }],
        };
        let mut bytes = Vec::new();
        write_resources_response(&mut bytes, &response)
            .await
            .unwrap();

        const EXPECTED_JSON: &str = r#"{"resources":[{"source_peer_id":"galbot","writer_peer_id":"galbot","resource_id":"occupancy","map":{"peer_id":"galbot","id":"occupancy","hash":"hash"},"clock":{"peer_id":"galbot","id":"clock","hash":"hash"}}]}"#;
        let mut expected = (EXPECTED_JSON.len() as u32).to_be_bytes().to_vec();
        expected.extend_from_slice(EXPECTED_JSON.as_bytes());
        assert_eq!(bytes, expected);
    }
}

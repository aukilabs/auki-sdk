//! `/auki/resources/0.5.0` Device Model catalog protocol.

use auki_registry::RegistryRef;
use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const RESOURCES_PROTOCOL: StreamProtocol = StreamProtocol::new("/auki/resources/0.5.0");
pub const MAX_RESOURCES_FRAME_BYTES: u32 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesRequest {}
impl ResourcesRequest {
    pub fn all() -> Self { Self {} }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceModelResource {
    pub source_peer_id: String,
    pub writer_peer_id: String,
    pub resource_id: String,
    pub model: RegistryRef,
}

impl DeviceModelResource {
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        if self.source_peer_id.is_empty() || self.writer_peer_id.is_empty()
            || self.resource_id.is_empty() || self.model.peer_id.is_empty()
            || self.model.id.is_empty() || self.model.hash.is_empty()
        {
            return Err(ResourcesProtocolError::Validation("device model contains an empty identity field".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesResponse {
    pub resources: Vec<DeviceModelResource>,
}
impl ResourcesResponse {
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        self.resources.iter().try_for_each(DeviceModelResource::validate)
    }
}

#[derive(Debug, Error)]
pub enum ResourcesProtocolError {
    #[error("io: {0}")] Io(#[source] std::io::Error),
    #[error("encode: {0}")] Encode(#[source] serde_json::Error),
    #[error("decode: {0}")] Decode(#[source] serde_json::Error),
    #[error("validation: {0}")] Validation(String),
    #[error("frame is empty")] EmptyFrame,
    #[error("frame too large: {actual} bytes (max {max})")] FrameTooLarge { actual: u64, max: u64 },
}

pub async fn write_resources_request<S: AsyncWriteExt + Unpin>(stream: &mut S, value: &ResourcesRequest) -> Result<(), ResourcesProtocolError> { write_json(stream, value).await }
pub async fn read_resources_request<S: AsyncReadExt + Unpin>(stream: &mut S) -> Result<ResourcesRequest, ResourcesProtocolError> { read_json(stream).await }
pub async fn write_resources_response<S: AsyncWriteExt + Unpin>(stream: &mut S, value: &ResourcesResponse) -> Result<(), ResourcesProtocolError> { value.validate()?; write_json(stream, value).await }
pub async fn read_resources_response<S: AsyncReadExt + Unpin>(stream: &mut S) -> Result<ResourcesResponse, ResourcesProtocolError> { let value: ResourcesResponse = read_json(stream).await?; value.validate()?; Ok(value) }

async fn write_json<S: AsyncWriteExt + Unpin, T: Serialize>(stream: &mut S, value: &T) -> Result<(), ResourcesProtocolError> {
    let bytes = serde_json::to_vec(value).map_err(ResourcesProtocolError::Encode)?;
    if bytes.len() > MAX_RESOURCES_FRAME_BYTES as usize { return Err(ResourcesProtocolError::FrameTooLarge { actual: bytes.len() as u64, max: MAX_RESOURCES_FRAME_BYTES as u64 }); }
    stream.write_all(&(bytes.len() as u32).to_be_bytes()).await.map_err(ResourcesProtocolError::Io)?;
    stream.write_all(&bytes).await.map_err(ResourcesProtocolError::Io)?;
    stream.flush().await.map_err(ResourcesProtocolError::Io)
}
async fn read_json<S: AsyncReadExt + Unpin, T: for<'de> Deserialize<'de>>(stream: &mut S) -> Result<T, ResourcesProtocolError> {
    let mut len = [0; 4];
    stream.read_exact(&mut len).await.map_err(ResourcesProtocolError::Io)?;
    let len = u32::from_be_bytes(len);
    if len == 0 { return Err(ResourcesProtocolError::EmptyFrame); }
    if len > MAX_RESOURCES_FRAME_BYTES { return Err(ResourcesProtocolError::FrameTooLarge { actual: len as u64, max: MAX_RESOURCES_FRAME_BYTES as u64 }); }
    let mut bytes = vec![0; len as usize];
    stream.read_exact(&mut bytes).await.map_err(ResourcesProtocolError::Io)?;
    serde_json::from_slice(&bytes).map_err(ResourcesProtocolError::Decode)
}

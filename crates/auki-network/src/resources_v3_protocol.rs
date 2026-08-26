//! `/auki/auth/1/resources/0.3.0` additive Resource Catalog payload codec.

use crate::resources_protocol::{
    ResourceEntry as V2ResourceEntry, VariantContent as V2VariantContent,
};
use auki_registry::RegistryRef;
use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p_identity::PeerId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;
use thiserror::Error;

/// Maximum encoded request or response frame size.
pub const MAX_RESOURCES_FRAME_BYTES: u32 = 1024 * 1024;

/// Resource variants understood by the v0.3 catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceVariant {
    /// A v0.2 sensor log row.
    SensorLog,
    /// A v0.2 pose log row.
    PoseLog,
    /// A v0.2 time-transform log row.
    TimeTransformLog,
    /// A v0.2 detection log row.
    DetectionLog,
    /// A receiver-owned live message channel.
    MessageChannel,
}

/// Optional v0.3 catalog variant filter; empty means all variants.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesRequest {
    /// Variants to include, or empty for all variants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<ResourceVariant>,
}

impl ResourcesRequest {
    /// Request every advertised resource.
    pub fn all() -> Self {
        Self::default()
    }

    /// Validate the filter before putting it on the wire.
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        let mut seen = HashSet::new();
        if self.variants.iter().all(|variant| seen.insert(*variant)) {
            Ok(())
        } else {
            Err(ResourcesProtocolError::Validation(
                "duplicate resource variant filter".into(),
            ))
        }
    }

    /// Whether this request includes a specific variant.
    pub fn includes(&self, variant: ResourceVariant) -> bool {
        self.variants.is_empty() || self.variants.contains(&variant)
    }
}

/// Atomic catalog identity for a live, receiver-owned message channel.
///
/// Message transport has no storage path or persistence configuration:
/// registration binds only this identity and clock to a bounded live receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageChannelResource {
    /// Peer that owns and receives the channel.
    pub owner_peer_id: PeerId,
    /// Resource identifier scoped to `owner_peer_id`.
    pub resource_id: String,
    /// Clock declaration defining the meaning of message timestamps.
    pub clock: RegistryRef,
}

impl MessageChannelResource {
    /// Validate the resource identity and clock reference.
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        if self.resource_id.is_empty() {
            return Err(ResourcesProtocolError::Validation(
                "message channel resource_id is empty".into(),
            ));
        }
        if self.clock.peer_id.is_empty() || self.clock.id.is_empty() || self.clock.hash.is_empty() {
            return Err(ResourcesProtocolError::Validation(
                "message channel clock RegistryRef contains an empty field".into(),
            ));
        }
        self.clock.peer_id.parse::<PeerId>().map_err(|_| {
            ResourcesProtocolError::Validation(
                "message channel clock peer_id is not a valid PeerId".into(),
            )
        })?;
        Ok(())
    }
}

/// One v0.3 catalog row: either an unchanged v0.2 row or a message channel.
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceEntry {
    /// A byte/JSON-shape-compatible v0.2 reporting row.
    V2(Box<V2ResourceEntry>),
    /// A receiver-owned live message channel.
    MessageChannel(MessageChannelResource),
}

impl ResourceEntry {
    /// Return this row's filter variant.
    pub fn variant(&self) -> ResourceVariant {
        match self {
            Self::V2(row) => match row.variant_content {
                V2VariantContent::SensorLog { .. } => ResourceVariant::SensorLog,
                V2VariantContent::PoseLog { .. } => ResourceVariant::PoseLog,
                V2VariantContent::TimeTransformLog { .. } => ResourceVariant::TimeTransformLog,
                V2VariantContent::DetectionLog { .. } => ResourceVariant::DetectionLog,
            },
            Self::MessageChannel(_) => ResourceVariant::MessageChannel,
        }
    }

    /// Validate fields introduced by v0.3.
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        match self {
            Self::V2(_) => Ok(()),
            Self::MessageChannel(channel) => channel.validate(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageChannelResourceWire {
    variant: MessageChannelVariant,
    owner_peer_id: PeerId,
    resource_id: String,
    clock: RegistryRef,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MessageChannelVariant {
    MessageChannel,
}

impl Serialize for ResourceEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::V2(row) => row.serialize(serializer),
            Self::MessageChannel(row) => MessageChannelResourceWire {
                variant: MessageChannelVariant::MessageChannel,
                owner_peer_id: row.owner_peer_id,
                resource_id: row.resource_id.clone(),
                clock: row.clock.clone(),
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ResourceEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("variant").and_then(serde_json::Value::as_str) == Some("message_channel") {
            let wire: MessageChannelResourceWire =
                serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            Ok(Self::MessageChannel(MessageChannelResource {
                owner_peer_id: wire.owner_peer_id,
                resource_id: wire.resource_id,
                clock: wire.clock,
            }))
        } else {
            serde_json::from_value(value)
                .map(Box::new)
                .map(Self::V2)
                .map_err(serde::de::Error::custom)
        }
    }
}

/// Framed v0.3 catalog response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesResponse {
    /// Current resource rows.
    pub resources: Vec<ResourceEntry>,
}

impl ResourcesResponse {
    /// Validate every response row.
    pub fn validate(&self) -> Result<(), ResourcesProtocolError> {
        self.resources.iter().try_for_each(ResourceEntry::validate)
    }

    /// Return rows selected by `request`, preserving row order and shape.
    pub fn filtered(&self, request: &ResourcesRequest) -> Self {
        Self {
            resources: self
                .resources
                .iter()
                .filter(|row| request.includes(row.variant()))
                .cloned()
                .collect(),
        }
    }
}

/// Resource Catalog v0.3 framing, serialization, and validation errors.
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
    #[error("frame is empty (length prefix is zero)")]
    EmptyFrame,
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write one validated, length-delimited v0.3 request.
pub async fn write_resources_request<S>(
    stream: &mut S,
    request: &ResourcesRequest,
) -> Result<(), ResourcesProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    request.validate()?;
    write_json(stream, request).await
}

/// Read and validate one length-delimited v0.3 request.
pub async fn read_resources_request<S>(
    stream: &mut S,
) -> Result<ResourcesRequest, ResourcesProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let request: ResourcesRequest = read_json(stream).await?;
    request.validate()?;
    Ok(request)
}

/// Write one validated, length-delimited v0.3 response.
pub async fn write_resources_response<S>(
    stream: &mut S,
    response: &ResourcesResponse,
) -> Result<(), ResourcesProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    response.validate()?;
    write_json(stream, response).await
}

/// Read and validate one length-delimited v0.3 response.
pub async fn read_resources_response<S>(
    stream: &mut S,
) -> Result<ResourcesResponse, ResourcesProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let response: ResourcesResponse = read_json(stream).await?;
    response.validate()?;
    Ok(response)
}

async fn write_json<S, T>(stream: &mut S, value: &T) -> Result<(), ResourcesProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Serialize,
{
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

async fn read_json<S, T>(stream: &mut S) -> Result<T, ResourcesProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
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
    use super::{
        MessageChannelResource, ResourceEntry, ResourceVariant, ResourcesRequest,
        ResourcesResponse, read_resources_request, read_resources_response,
        write_resources_request, write_resources_response,
    };
    use crate::resources_protocol::{
        Available, Head, ResourceEntry as V2ResourceEntry,
        ResourcesResponse as V2ResourcesResponse, SensorBlock, SensorKind, SensorManifestPointer,
        VariantContent, write_resources_response as write_v2_resources_response,
    };
    use auki_registry::RegistryRef;
    use futures::io::Cursor;
    use libp2p_identity::PeerId;

    fn peer() -> PeerId {
        crate::PeerIdentity::from_seed(&[91; 32]).peer_id()
    }

    fn clock() -> RegistryRef {
        RegistryRef {
            peer_id: peer().to_string(),
            id: "session/monotonic".into(),
            hash: "clock-hash".into(),
        }
    }

    fn channel(resource_id: &str) -> ResourceEntry {
        ResourceEntry::MessageChannel(MessageChannelResource {
            owner_peer_id: peer(),
            resource_id: resource_id.into(),
            clock: clock(),
        })
    }

    fn v2_row() -> V2ResourceEntry {
        V2ResourceEntry {
            source_peer_id: peer().to_string(),
            writer_peer_id: peer().to_string(),
            resource_id: "camera".into(),
            state: "live".into(),
            head: Some(Head::Rolling { retention_ns: 1 }),
            extent: None,
            available: Available {
                bytes: 4096,
                entries: 23,
                duration_ns: 1_500_000,
            },
            sensor: Some(SensorBlock {
                kind: SensorKind::Camera,
                r#type: "rgb".into(),
                sensor_id: "front-camera".into(),
                sensor_hash: "sensor-hash".into(),
            }),
            pose: None,
            variant_content: VariantContent::SensorLog {
                manifest: SensorManifestPointer {
                    clock: clock(),
                    frame: Some(RegistryRef {
                        peer_id: peer().to_string(),
                        id: "frame/camera".into(),
                        hash: "frame-hash".into(),
                    }),
                },
            },
        }
    }

    #[test]
    fn message_channel_has_only_canonical_identity_and_clock_fields() {
        let canonical = auki_jcs::canonicalize(&serde_json::to_value(channel("events")).unwrap());
        assert_eq!(
            String::from_utf8(canonical).unwrap(),
            format!(
                r#"{{"clock":{{"hash":"clock-hash","id":"session/monotonic","peer_id":"{}"}},"owner_peer_id":"{}","resource_id":"events","variant":"message_channel"}}"#,
                peer(),
                peer()
            )
        );
    }

    #[test]
    fn v2_rows_keep_their_existing_json_shape() {
        let row = v2_row();
        let wrapped = ResourceEntry::V2(Box::new(row.clone()));
        assert_eq!(
            serde_json::to_value(wrapped).unwrap(),
            serde_json::to_value(row).unwrap()
        );
    }

    #[test]
    fn filter_selects_message_channels_without_dropping_requested_v2_rows() {
        let response = ResourcesResponse {
            resources: vec![ResourceEntry::V2(Box::new(v2_row())), channel("events")],
        };

        let only_messages = response.filtered(&ResourcesRequest {
            variants: vec![ResourceVariant::MessageChannel],
        });
        assert_eq!(only_messages.resources, vec![channel("events")]);

        let all = response.filtered(&ResourcesRequest::all());
        assert_eq!(all, response);
    }

    #[tokio::test]
    async fn request_and_response_round_trip_with_framing() {
        let request = ResourcesRequest {
            variants: vec![ResourceVariant::SensorLog, ResourceVariant::MessageChannel],
        };
        let response = ResourcesResponse {
            resources: vec![ResourceEntry::V2(Box::new(v2_row())), channel("events")],
        };

        let mut request_bytes = Vec::new();
        write_resources_request(&mut request_bytes, &request)
            .await
            .unwrap();
        assert_eq!(
            read_resources_request(&mut Cursor::new(request_bytes))
                .await
                .unwrap(),
            request
        );

        let mut response_bytes = Vec::new();
        write_resources_response(&mut response_bytes, &response)
            .await
            .unwrap();
        assert_eq!(
            read_resources_response(&mut Cursor::new(response_bytes))
                .await
                .unwrap(),
            response
        );
    }

    #[tokio::test]
    async fn nonempty_v2_row_is_preserved_through_v2_and_v3_response_framing() {
        let row = v2_row();
        let mut v2_bytes = Vec::new();
        write_v2_resources_response(
            &mut v2_bytes,
            &V2ResourcesResponse {
                resources: vec![row.clone()],
            },
        )
        .await
        .unwrap();
        let mut v3_bytes = Vec::new();
        write_resources_response(
            &mut v3_bytes,
            &ResourcesResponse {
                resources: vec![ResourceEntry::V2(Box::new(row.clone()))],
            },
        )
        .await
        .unwrap();

        let v2_json: serde_json::Value = serde_json::from_slice(&v2_bytes[4..]).unwrap();
        let v3_json: serde_json::Value = serde_json::from_slice(&v3_bytes[4..]).unwrap();
        assert_eq!(v2_json["resources"][0], serde_json::to_value(&row).unwrap());
        assert_eq!(v3_json["resources"][0], v2_json["resources"][0]);
        assert_eq!(v3_bytes, v2_bytes);
    }

    #[tokio::test]
    async fn mixed_v3_catalog_framed_bytes_are_locked() {
        let response = ResourcesResponse {
            resources: vec![ResourceEntry::V2(Box::new(v2_row())), channel("events")],
        };
        let mut bytes = Vec::new();
        write_resources_response(&mut bytes, &response)
            .await
            .unwrap();

        const EXPECTED_JSON: &str = concat!(
            r#"{"resources":[{"source_peer_id":"12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan","#,
            r#""writer_peer_id":"12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan","#,
            r#""resource_id":"camera","state":"live","head":{"kind":"rolling","retention_ns":1},"#,
            r#""available":{"bytes":4096,"entries":23,"duration_ns":1500000},"#,
            r#""sensor":{"kind":"camera","type":"rgb","sensor_id":"front-camera","sensor_hash":"sensor-hash"},"#,
            r#""variant":"sensor_log","manifest":{"clock":{"peer_id":"12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan","#,
            r#""id":"session/monotonic","hash":"clock-hash"},"#,
            r#""frame":{"peer_id":"12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan","#,
            r#""id":"frame/camera","hash":"frame-hash"}}},"#,
            r#"{"variant":"message_channel","owner_peer_id":"12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan","#,
            r#""resource_id":"events","clock":{"peer_id":"12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan","#,
            r#""id":"session/monotonic","hash":"clock-hash"}}]}"#,
        );
        let mut expected = (EXPECTED_JSON.len() as u32).to_be_bytes().to_vec();
        expected.extend_from_slice(EXPECTED_JSON.as_bytes());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn validation_rejects_empty_channel_identity_and_duplicate_filters() {
        let invalid = MessageChannelResource {
            owner_peer_id: peer(),
            resource_id: String::new(),
            clock: clock(),
        };
        assert!(invalid.validate().is_err());

        let duplicate = ResourcesRequest {
            variants: vec![
                ResourceVariant::MessageChannel,
                ResourceVariant::MessageChannel,
            ],
        };
        assert!(duplicate.validate().is_err());
    }
}

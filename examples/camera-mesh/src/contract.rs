//! Cross-runtime Camera Mesh values and JSON/protobuf-adjacent wire shapes.

use std::collections::HashSet;

use anyhow::{Context, Result, anyhow, bail, ensure};
use auki_protocols::{
    catalog::{
        v2::{
            Available, Head, ResourceEntry as V2ResourceEntry, SensorBlock, SensorKind,
            SensorManifestPointer, VariantContent,
        },
        v3::{self as catalog_v3, ResourceEntry},
        v4 as catalog_v4,
    },
    message::MessageChannelResource,
    registry::v3::{RegistryEntryEnvelope, RegistryKind},
    stream::v2::StreamManifest,
};
use auki_registry::{
    AxisConvention, AxisDirection, Camera, ClockBody, ClockMeta, ClockRegistryEntry,
    FrameRegistryEntry, Handedness, LengthUnit, RegistryRef, Scope, SensorBody,
    SensorRegistryEntry,
};
use auki_sdk::{Multiaddr, PeerId};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const APP: &str = "auki-camera-mesh";
pub const APP_VERSION: &str = "0.1.0";
pub const CAMERA_RESOURCE_ID: &str = "camera/main";
pub const CAMERA_CONTROL_RESOURCE_ID: &str = "camera/control";
pub const CAMERA_REPLY_RESOURCE_ID: &str = "camera/replies";
pub const CAMERA_CLOCK_ID: &str = "camera/utc";
pub const CAMERA_FRAME_ID: &str = "camera/optical";
pub const CAMERA_WIDTH: u32 = 480;
pub const CAMERA_HEIGHT: u32 = 270;
pub const CAMERA_RATE_HZ: u32 = 5;
pub const MAX_REPLY_ROUTES: usize = 4;
pub const MAX_BLOB_BYTES: usize = 20 * 1024 * 1024;

const DETERMINISTIC_JPEG_BASE64: &str = include_str!("../assets/deterministic-frame.jpg.base64");
const SYNTHETIC_JPEGS_BASE64: &str = include_str!("../assets/synthetic-frames.jpg.base64");

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraRole {
    Publisher,
    Viewer,
}

impl CameraRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::Viewer => "viewer",
        }
    }
}

impl std::str::FromStr for CameraRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "publisher" => Ok(Self::Publisher),
            "viewer" => Ok(Self::Viewer),
            other => bail!("AUKI_CAMERA_ROLE must be publisher or viewer, got {other:?}"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerCard {
    pub version: u8,
    pub runtime: String,
    pub domain_id: String,
    pub peer_id: String,
    pub protocols: Vec<String>,
    pub routes: PeerRoutes,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PeerRoutes {
    pub tcp: String,
    pub wss: String,
}

impl PeerCard {
    pub fn peer_id(&self) -> Result<PeerId> {
        self.peer_id.parse().context("invalid target Peer ID")
    }

    pub fn tcp_route(&self) -> Result<Multiaddr> {
        self.routes
            .tcp
            .parse()
            .context("invalid target native TCP route")
    }

    pub fn reply_routes(&self) -> Vec<String> {
        [self.routes.tcp.clone(), self.routes.wss.clone()]
            .into_iter()
            .filter(|route| !route.is_empty())
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct CameraMetadata {
    pub sensor: SensorRegistryEntry,
    pub clock: ClockRegistryEntry,
    pub frame: FrameRegistryEntry,
    pub sensor_ref: RegistryRef,
    pub clock_ref: RegistryRef,
    pub frame_ref: RegistryRef,
}

impl CameraMetadata {
    pub fn response(
        &self,
        request: &auki_protocols::registry::v3::RegistryRequest,
    ) -> auki_protocols::registry::v3::RegistryResponse {
        use auki_protocols::registry::v3::{RegistryListEntry, RegistryRequest, RegistryResponse};

        let entries = self.envelopes();
        match request {
            RegistryRequest::List { kind } => RegistryResponse::List {
                entries: entries
                    .iter()
                    .filter(|entry| entry.kind == *kind)
                    .map(|entry| RegistryListEntry {
                        id: entry.id.clone(),
                        hash: entry.hash.clone(),
                    })
                    .collect(),
            },
            RegistryRequest::Get { kind, id, hash } => RegistryResponse::Get {
                entry: entries
                    .into_iter()
                    .find(|entry| entry.kind == *kind && entry.id == *id && entry.hash == *hash),
            },
        }
    }

    pub fn envelopes(&self) -> Vec<RegistryEntryEnvelope> {
        vec![
            registry_envelope(
                RegistryKind::Sensor,
                &self.sensor.sensor_id,
                &self.sensor.hash(),
                &self.sensor.canonical_bytes(),
            ),
            registry_envelope(
                RegistryKind::Clock,
                &self.clock.clock_id,
                &self.clock.hash(),
                &self.clock.canonical_bytes(),
            ),
            registry_envelope(
                RegistryKind::Frame,
                &self.frame.frame_id,
                &self.frame.hash(),
                &self.frame.canonical_bytes(),
            ),
        ]
    }
}

pub fn metadata(peer_id: PeerId, session_id: impl Into<String>) -> CameraMetadata {
    let peer_id = peer_id.to_string();
    let frame = FrameRegistryEntry {
        peer_id: peer_id.clone(),
        frame_id: CAMERA_FRAME_ID.into(),
        handedness: Handedness::Right,
        axes: AxisConvention {
            x: AxisDirection::Right,
            y: AxisDirection::Down,
            z: AxisDirection::Forward,
        },
        units: LengthUnit::Meters,
    };
    let frame_ref = RegistryRef {
        peer_id: peer_id.clone(),
        id: frame.frame_id.clone(),
        hash: frame.hash(),
    };
    let clock = ClockRegistryEntry {
        peer_id: peer_id.clone(),
        session_id: session_id.into(),
        clock_id: CAMERA_CLOCK_ID.into(),
        body: ClockBody::UtcClock(ClockMeta {
            unit: "ns".into(),
            monotonic: false,
            epoch: Some("1970-01-01T00:00:00Z".into()),
            scope: Scope::Global,
        }),
    };
    let clock_ref = RegistryRef {
        peer_id: peer_id.clone(),
        id: clock.clock_id.clone(),
        hash: clock.hash(),
    };
    let sensor = SensorRegistryEntry {
        peer_id: peer_id.clone(),
        sensor_id: CAMERA_RESOURCE_ID.into(),
        body: SensorBody::Camera(Camera {
            r#type: "rgb".into(),
            width: CAMERA_WIDTH,
            height: CAMERA_HEIGHT,
            frame_rate_hz: CAMERA_RATE_HZ,
            image_encoding: "jpeg".into(),
            pixel_format: "rgb8".into(),
            row_stride_bytes: 0,
            color_space: "srgb".into(),
            intrinsics_model: "none".into(),
            distortion_model: "none".into(),
            calibration: None,
            frame: frame_ref.clone(),
        }),
    };
    let sensor_ref = RegistryRef {
        peer_id,
        id: sensor.sensor_id.clone(),
        hash: sensor.hash(),
    };
    CameraMetadata {
        sensor,
        clock,
        frame,
        sensor_ref,
        clock_ref,
        frame_ref,
    }
}

pub fn camera_catalog(peer_id: PeerId, metadata: &CameraMetadata) -> catalog_v3::ResourcesResponse {
    let peer_id = peer_id.to_string();
    catalog_v3::ResourcesResponse {
        resources: vec![
            ResourceEntry::V2(Box::new(V2ResourceEntry {
                source_peer_id: peer_id.clone(),
                writer_peer_id: peer_id,
                resource_id: CAMERA_RESOURCE_ID.into(),
                state: "live".into(),
                head: Some(Head::Rolling {
                    retention_ns: 1_000_000_000_i64 / i64::from(CAMERA_RATE_HZ),
                }),
                extent: None,
                available: Available {
                    bytes: 0,
                    entries: 0,
                    duration_ns: 0,
                },
                sensor: Some(SensorBlock {
                    kind: SensorKind::Camera,
                    r#type: "rgb".into(),
                    sensor_id: metadata.sensor_ref.id.clone(),
                    sensor_hash: metadata.sensor_ref.hash.clone(),
                }),
                pose: None,
                variant_content: VariantContent::SensorLog {
                    manifest: SensorManifestPointer {
                        clock: metadata.clock_ref.clone(),
                        frame: Some(metadata.frame_ref.clone()),
                    },
                },
            })),
            ResourceEntry::MessageChannel(control_channel(
                metadata
                    .sensor_ref
                    .peer_id
                    .parse()
                    .expect("metadata owner is a PeerId"),
                metadata,
            )),
        ],
    }
}

pub fn control_channel(peer_id: PeerId, metadata: &CameraMetadata) -> MessageChannelResource {
    MessageChannelResource {
        owner_peer_id: peer_id,
        resource_id: CAMERA_CONTROL_RESOURCE_ID.into(),
        clock: metadata.clock_ref.clone(),
    }
}

pub fn reply_channel(peer_id: PeerId, metadata: &CameraMetadata) -> MessageChannelResource {
    MessageChannelResource {
        owner_peer_id: peer_id,
        resource_id: CAMERA_REPLY_RESOURCE_ID.into(),
        clock: metadata.clock_ref.clone(),
    }
}

pub fn stream_manifest(metadata: &CameraMetadata) -> StreamManifest {
    StreamManifest {
        sensor_id: metadata.sensor_ref.id.clone(),
        sensor_hash: metadata.sensor_ref.hash.clone(),
        clock_peer_id: metadata.clock_ref.peer_id.clone(),
        clock_id: metadata.clock_ref.id.clone(),
        clock_hash: metadata.clock_ref.hash.clone(),
        frame_id: metadata.frame_ref.id.clone(),
        frame_hash: metadata.frame_ref.hash.clone(),
        resource_id: CAMERA_RESOURCE_ID.into(),
        payload: "camera_frame".into(),
        writer_mode: "live".into(),
        expected_rate_hz: CAMERA_RATE_HZ,
        ..Default::default()
    }
}

pub fn protocol_ids() -> Vec<String> {
    [
        auki_protocols::info::v1::ID,
        catalog_v3::ID,
        catalog_v4::ID,
        auki_protocols::registry::v3::ID,
        auki_protocols::blob::v1::ID,
        auki_protocols::message::v1::ID,
        auki_protocols::stream::v2::ID,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub fn protocol_ids_for_role(role: CameraRole) -> Vec<String> {
    let mut ids = protocol_ids();
    if role == CameraRole::Viewer {
        ids.retain(|id| id != auki_protocols::stream::v2::ID);
    }
    ids
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MessageChannelWire {
    pub variant: String,
    pub owner_peer_id: String,
    pub resource_id: String,
    pub clock: RegistryRef,
}

impl From<&MessageChannelResource> for MessageChannelWire {
    fn from(value: &MessageChannelResource) -> Self {
        Self {
            variant: "message_channel".into(),
            owner_peer_id: value.owner_peer_id.to_string(),
            resource_id: value.resource_id.clone(),
            clock: value.clock.clone(),
        }
    }
}

impl TryFrom<MessageChannelWire> for MessageChannelResource {
    type Error = anyhow::Error;

    fn try_from(value: MessageChannelWire) -> Result<Self> {
        ensure!(
            value.variant == "message_channel",
            "reply channel has an invalid variant"
        );
        let channel = Self {
            owner_peer_id: value
                .owner_peer_id
                .parse()
                .context("invalid reply channel owner")?,
            resource_id: value.resource_id,
            clock: value.clock,
        };
        channel.validate().context("invalid reply channel")?;
        Ok(channel)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotReplyTarget {
    pub peer_id: String,
    pub routes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotReplyAddress {
    pub target: SnapshotReplyTarget,
    pub channel: MessageChannelWire,
}

impl SnapshotReplyAddress {
    pub fn validate_for(
        &self,
        requester: PeerId,
    ) -> Result<(MessageChannelResource, Vec<Multiaddr>)> {
        ensure!(
            self.target.peer_id == requester.to_string(),
            "snapshot reply target is not the authenticated requester"
        );
        ensure!(
            !self.target.routes.is_empty(),
            "snapshot reply target has no routes"
        );
        ensure!(
            self.target.routes.len() <= MAX_REPLY_ROUTES,
            "snapshot reply target has too many routes"
        );

        let mut unique = HashSet::new();
        let routes = self
            .target
            .routes
            .iter()
            .map(|route| {
                ensure!(unique.insert(route), "snapshot reply routes must be unique");
                let parsed = route
                    .parse::<Multiaddr>()
                    .context("invalid snapshot reply route")?;
                ensure!(
                    route.ends_with(&format!("/p2p/{requester}")),
                    "snapshot reply route does not terminate at the requester"
                );
                Ok(parsed)
            })
            .collect::<Result<Vec<_>>>()?;
        let channel = MessageChannelResource::try_from(self.channel.clone())?;
        ensure!(
            channel.owner_peer_id == requester,
            "snapshot reply channel owner is not the requester"
        );
        ensure!(
            channel.resource_id == CAMERA_REPLY_RESOURCE_ID,
            "snapshot reply channel has the wrong resource ID"
        );
        ensure!(
            channel.clock.peer_id == requester.to_string(),
            "snapshot reply clock owner is not the requester"
        );
        Ok((channel, routes))
    }

    pub fn native_route_for(
        &self,
        requester: PeerId,
    ) -> Result<(MessageChannelResource, Multiaddr)> {
        let (channel, routes) = self.validate_for(requester)?;
        let route = routes
            .into_iter()
            .find(|route| !route.to_string().contains("/wss"))
            .ok_or_else(|| anyhow!("snapshot requester supplied no native TCP route"))?;
        Ok((channel, route))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRequestPayload {
    pub version: u8,
    pub request_id: String,
    pub reply: SnapshotReplyAddress,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotReadyPayload {
    pub version: u8,
    pub request_id: String,
    pub sha256: String,
    pub size: usize,
}

pub fn encode_snapshot_request(
    request_id: impl Into<String>,
    card: &PeerCard,
    channel: &MessageChannelResource,
) -> Result<Vec<u8>> {
    let request_id = request_id.into();
    validate_request_id(&request_id)?;
    let payload = SnapshotRequestPayload {
        version: 1,
        request_id,
        reply: SnapshotReplyAddress {
            target: SnapshotReplyTarget {
                peer_id: card.peer_id.clone(),
                routes: card.reply_routes(),
            },
            channel: channel.into(),
        },
    };
    payload.reply.validate_for(card.peer_id()?)?;
    serde_json::to_vec(&payload).context("encode snapshot request")
}

pub fn decode_snapshot_request(
    payload: &[u8],
    requester: PeerId,
) -> Result<SnapshotRequestPayload> {
    let payload: SnapshotRequestPayload =
        serde_json::from_slice(payload).context("decode snapshot request")?;
    ensure!(payload.version == 1, "unsupported snapshot request version");
    validate_request_id(&payload.request_id)?;
    payload.reply.validate_for(requester)?;
    Ok(payload)
}

pub fn encode_snapshot_ready(
    request_id: impl Into<String>,
    sha256: impl Into<String>,
    size: usize,
) -> Result<Vec<u8>> {
    let payload = SnapshotReadyPayload {
        version: 1,
        request_id: request_id.into(),
        sha256: sha256.into(),
        size,
    };
    validate_ready(&payload)?;
    serde_json::to_vec(&payload).context("encode snapshot ready")
}

pub fn decode_snapshot_ready(payload: &[u8]) -> Result<SnapshotReadyPayload> {
    let payload: SnapshotReadyPayload =
        serde_json::from_slice(payload).context("decode snapshot ready")?;
    validate_ready(&payload)?;
    Ok(payload)
}

pub fn deterministic_jpeg() -> Result<Vec<u8>> {
    let bytes = STANDARD
        .decode(DETERMINISTIC_JPEG_BASE64.trim())
        .context("decode checked-in deterministic JPEG")?;
    ensure!(
        bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9]),
        "deterministic fixture is not a JPEG"
    );
    Ok(bytes)
}

/// Decode the small checked-in animation used by headless demo publishers.
///
/// Keeping the JPEGs as fixtures means Rust and Python can publish a visibly
/// live feed without a camera, platform image APIs, or runtime codec packages.
pub fn synthetic_jpegs() -> Result<Vec<Vec<u8>>> {
    let frames = SYNTHETIC_JPEGS_BASE64
        .split_whitespace()
        .enumerate()
        .map(|(index, encoded)| {
            let bytes = STANDARD
                .decode(encoded)
                .with_context(|| format!("decode synthetic Camera Mesh JPEG {index}"))?;
            ensure!(
                bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9]),
                "synthetic Camera Mesh frame {index} is not a JPEG"
            );
            Ok(bytes)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        frames.len() >= 2,
        "synthetic Camera Mesh animation needs at least two frames"
    );
    Ok(frames)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn registry_envelope(
    kind: RegistryKind,
    id: &str,
    hash: &str,
    canonical_bytes: &[u8],
) -> RegistryEntryEnvelope {
    RegistryEntryEnvelope {
        kind,
        id: id.into(),
        hash: hash.into(),
        canonical_json: String::from_utf8(canonical_bytes.to_vec())
            .expect("registry canonical JSON is UTF-8"),
    }
}

fn validate_request_id(request_id: &str) -> Result<()> {
    ensure!(
        !request_id.is_empty() && request_id.len() <= 128,
        "snapshot requestId must contain 1..=128 bytes"
    );
    ensure!(
        request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')),
        "snapshot requestId contains unsupported characters"
    );
    Ok(())
}

fn validate_ready(payload: &SnapshotReadyPayload) -> Result<()> {
    ensure!(payload.version == 1, "unsupported snapshot reply version");
    validate_request_id(&payload.request_id)?;
    ensure!(
        payload.sha256.len() == 64
            && payload
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "snapshot SHA-256 must be 64 lowercase hexadecimal characters"
    );
    ensure!(payload.size > 0, "snapshot size must be positive");
    ensure!(
        payload.size <= MAX_BLOB_BYTES,
        "snapshot size exceeds the Camera Mesh limit"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }

    #[test]
    fn metadata_and_catalog_are_cross_runtime_stable() {
        let metadata = metadata(peer(), "camera-test-session");
        assert_eq!(
            (
                metadata.frame_ref.hash.as_str(),
                metadata.clock_ref.hash.as_str(),
                metadata.sensor_ref.hash.as_str(),
            ),
            (
                "917159c0c637e16c6227acd37a74ec63",
                "0dc82a1bfe9fa825e13a4b5d1d0de141",
                "66c133a3193e39d699fc436a0251785c",
            )
        );
        let catalog = camera_catalog(peer(), &metadata);
        assert_eq!(catalog.resources.len(), 2);
        assert_eq!(stream_manifest(&metadata).payload, "camera_frame");
    }

    #[test]
    fn snapshot_wire_is_camel_case_and_round_trips() {
        let metadata = metadata(peer(), "camera-test-session");
        let card = PeerCard {
            version: 1,
            runtime: "native".into(),
            domain_id: "4e990513-b110-467b-84ca-09a42d786f6d".into(),
            peer_id: peer().to_string(),
            protocols: protocol_ids(),
            routes: PeerRoutes {
                tcp: format!("/ip4/127.0.0.1/tcp/9000/p2p/{}", peer()),
                wss: format!("/dns4/relay.example.com/tcp/443/wss/p2p/{}", peer()),
            },
        };
        let encoded =
            encode_snapshot_request("locked-request", &card, &reply_channel(peer(), &metadata))
                .unwrap();
        let json = std::str::from_utf8(&encoded).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"version":1,"requestId":"locked-request","reply":{{"target":{{"peerId":"{}","routes":["/ip4/127.0.0.1/tcp/9000/p2p/{}","/dns4/relay.example.com/tcp/443/wss/p2p/{}"]}},"channel":{{"variant":"message_channel","owner_peer_id":"{}","resource_id":"camera/replies","clock":{{"peer_id":"{}","id":"camera/utc","hash":"0dc82a1bfe9fa825e13a4b5d1d0de141"}}}}}}}}"#,
                peer(),
                peer(),
                peer(),
                peer(),
                peer(),
            )
        );
        assert_eq!(
            decode_snapshot_request(&encoded, peer())
                .unwrap()
                .request_id,
            "locked-request"
        );
    }

    #[test]
    fn fixture_is_a_locked_jpeg() {
        let fixture = deterministic_jpeg().unwrap();
        assert!(fixture.len() > 1_000);
        assert_eq!(
            sha256_hex(&fixture),
            "9cb77ff8f8f6d6af10809750bba03a76a53d6b55c36515c20a688d8437689aa0"
        );
    }

    #[test]
    fn synthetic_animation_contains_distinct_jpegs() {
        let frames = synthetic_jpegs().unwrap();
        assert_eq!(frames.len(), 16);
        assert!(frames.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn viewer_advertises_consumable_protocols_without_stream_provider() {
        let publisher = protocol_ids_for_role(CameraRole::Publisher);
        let viewer = protocol_ids_for_role(CameraRole::Viewer);
        assert_eq!(publisher.len(), 7);
        assert_eq!(viewer.len(), 6);
        assert!(publisher.contains(&auki_protocols::stream::v2::ID.to_owned()));
        assert!(!viewer.contains(&auki_protocols::stream::v2::ID.to_owned()));
    }
}

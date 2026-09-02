//! Swift client and Rust-owned producer registry for typed Stream v2.
//!
//! Swift never implements a synchronous Rust callback. An application creates
//! bounded producers on a mounted endpoint and pushes protobuf payloads into
//! them. The native provider performs an exact in-memory lookup during
//! admission, then each asynchronous `push` waits for every active consumer's
//! one-item queue. This keeps memory bounded and propagates slow-consumer
//! backpressure without moving protocol framing or protobuf validation into
//! Swift.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

use async_channel::{Receiver, Sender};
use auki_datatypes::{
    audio, camera::CameraFrame, detection::DetectionFrame, joint_encoders, map::MapUpdate,
    point_cloud, pose, scalar,
};
use auki_protocols::stream::{
    SourceStream, StreamClient, StreamDispatch, StreamEndpoint, StreamEndpointError, StreamEntry,
    StreamError, StreamItem, StreamPayload, StreamProvider, StreamSubscription,
    SubscriptionEntries,
    v2::{
        DeclineReason, EndReason, ID, MAX_FRAME_BYTES, ReadFrom, StreamManifest, StreamRequest,
        end_reason,
    },
};
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use prost::Message;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{runtime::Handle, sync::watch};
use uuid::Uuid;

use crate::{
    AukiPeer, AukiPeerTarget, AukiSdkError, CleanupResult, DetachedCleanup, operation_error,
    parse_target, wait_cleanup,
};

const MAX_CONTROL_JSON_BYTES: usize = 64 * 1024;
const MAX_CONTROL_STRING_BYTES: usize = 8 * 1024;
const MAX_REMOTE_ERROR_BYTES: usize = 1024;
const MAX_ALLOWED_REQUESTERS: usize = 64;
const CAMERA_FRAME_CODEC_OVERHEAD_BYTES: usize = 1_024;

fn binding_error(message: impl Into<String>) -> AukiSdkError {
    AukiSdkError::Operation {
        message: message.into(),
    }
}

/// Encode opaque camera image bytes with the canonical Auki protobuf type.
///
/// Static camera metadata belongs in the Sensor Registry entry referenced by
/// the Stream manifest; this convenience codec deliberately omits per-frame
/// dynamic intrinsics.
#[uniffi::export]
pub fn encode_camera_frame_image(frame: Vec<u8>) -> Result<Vec<u8>, AukiSdkError> {
    let maximum = MAX_FRAME_BYTES as usize - CAMERA_FRAME_CODEC_OVERHEAD_BYTES;
    if frame.len() > maximum {
        return Err(binding_error(format!(
            "encode CameraFrame image: image is {} bytes; maximum is {maximum}",
            frame.len()
        )));
    }
    Ok(CameraFrame {
        dynamic_intrinsics: None,
        frame,
    }
    .encode_to_vec())
}

/// Decode canonical Auki camera-frame protobuf bytes into their opaque image
/// payload.
#[uniffi::export]
pub fn decode_camera_frame_image(payload: Vec<u8>) -> Result<Vec<u8>, AukiSdkError> {
    if payload.len() > MAX_FRAME_BYTES as usize {
        return Err(binding_error(format!(
            "decode CameraFrame image: payload is {} bytes; maximum is {MAX_FRAME_BYTES}",
            payload.len()
        )));
    }
    CameraFrame::decode(payload.as_slice())
        .map(|frame| frame.frame)
        .map_err(|error| operation_error("decode CameraFrame", error))
}

/// Closed protobuf family supported by Stream v2.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum AukiStreamPayloadKind {
    Camera,
    PointCloud,
    JointEncoders,
    Audio,
    Scalar,
    Pose,
    Detection,
    Map,
}

impl AukiStreamPayloadKind {
    fn manifest_payload(self) -> &'static str {
        match self {
            Self::Camera => "camera_frame",
            Self::PointCloud => "point_cloud_frame",
            Self::JointEncoders => "joint_encoders_frame",
            Self::Audio => "audio_frame",
            Self::Scalar => "scalar",
            Self::Pose => "spatial_transform",
            Self::Detection => "detection",
            Self::Map => "map_update",
        }
    }

    fn from_manifest_payload(payload: &str) -> Option<Self> {
        match payload {
            "camera_frame" => Some(Self::Camera),
            "point_cloud_frame" => Some(Self::PointCloud),
            "joint_encoders_frame" => Some(Self::JointEncoders),
            "audio_frame" => Some(Self::Audio),
            "scalar" => Some(Self::Scalar),
            "spatial_transform" => Some(Self::Pose),
            "detection" => Some(Self::Detection),
            "map_update" => Some(Self::Map),
            _ => None,
        }
    }

    fn validate_manifest(self, manifest: &StreamManifest) -> Result<(), AukiSdkError> {
        let expected = self.manifest_payload();
        if manifest.payload == expected {
            Ok(())
        } else {
            Err(binding_error(format!(
                "Stream payload kind {self:?} requires manifest payload {expected:?}, got {:?}",
                manifest.payload
            )))
        }
    }

    fn validate_payload(self, payload: &[u8]) -> Result<(), AukiSdkError> {
        if payload.len() > MAX_FRAME_BYTES as usize {
            return Err(binding_error(format!(
                "Stream protobuf payload is {} bytes; maximum is {MAX_FRAME_BYTES}",
                payload.len()
            )));
        }
        match self {
            Self::Camera => decode_payload::<CameraFrame>(payload),
            Self::PointCloud => decode_payload::<point_cloud::Data>(payload),
            Self::JointEncoders => decode_payload::<joint_encoders::Data>(payload),
            Self::Audio => decode_payload::<audio::Data>(payload),
            Self::Scalar => decode_payload::<scalar::Data>(payload),
            Self::Pose => decode_payload::<pose::SpatialTransform>(payload),
            Self::Detection => decode_payload::<DetectionFrame>(payload),
            Self::Map => decode_payload::<MapUpdate>(payload),
        }
    }
}

fn decode_payload<T>(payload: &[u8]) -> Result<(), AukiSdkError>
where
    T: Message + Default,
{
    T::decode(payload)
        .map(|_| ())
        .map_err(|error| operation_error("decode typed Stream protobuf payload", error))
}

/// Starting position requested from a Stream v2 producer.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AukiStreamReadFrom {
    #[default]
    Latest,
    FromStart,
    FromTimestamp {
        #[serde(rename = "timestampNs")]
        timestamp_ns: i64,
    },
}

impl From<AukiStreamReadFrom> for ReadFrom {
    fn from(value: AukiStreamReadFrom) -> Self {
        match value {
            AukiStreamReadFrom::Latest => Self::Latest,
            AukiStreamReadFrom::FromStart => Self::FromStart,
            AukiStreamReadFrom::FromTimestamp { timestamp_ns } => Self::FromTimestamp(timestamp_ns),
        }
    }
}

impl From<ReadFrom> for AukiStreamReadFrom {
    fn from(value: ReadFrom) -> Self {
        match value {
            ReadFrom::Latest => Self::Latest,
            ReadFrom::FromStart => Self::FromStart,
            ReadFrom::FromTimestamp(timestamp_ns) => Self::FromTimestamp { timestamp_ns },
        }
    }
}

/// Exact Stream v2 subscription request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AukiStreamRequest {
    pub source_peer_id: String,
    pub resource_id: String,
    pub read_from: AukiStreamReadFrom,
}

impl AukiStreamRequest {
    fn validate(&self) -> Result<(), AukiSdkError> {
        validate_control_string("Stream source Peer ID", &self.source_peer_id, true)?;
        if !self.source_peer_id.is_empty() {
            self.source_peer_id
                .parse::<auki_sdk_rs::PeerId>()
                .map_err(|error| operation_error("parse Stream source Peer ID", error))?;
        }
        validate_control_string("Stream resource ID", &self.resource_id, false)
    }
}

impl From<AukiStreamRequest> for StreamRequest {
    fn from(value: AukiStreamRequest) -> Self {
        Self {
            source_peer_id: value.source_peer_id,
            resource_id: value.resource_id,
            from: value.read_from.into(),
        }
    }
}

impl From<&StreamRequest> for AukiStreamRequest {
    fn from(value: &StreamRequest) -> Self {
        Self {
            source_peer_id: value.source_peer_id.clone(),
            resource_id: value.resource_id.clone(),
            read_from: value.from.into(),
        }
    }
}

/// Immutable Stream v2 handshake metadata.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, uniffi::Record)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct AukiStreamManifest {
    pub sensor_id: String,
    pub sensor_hash: String,
    pub clock_peer_id: String,
    pub clock_id: String,
    pub clock_hash: String,
    pub frame_id: String,
    pub frame_hash: String,
    pub resource_id: String,
    pub payload: String,
    pub from_frame_id: String,
    pub from_frame_hash: String,
    pub to_frame_id: String,
    pub to_frame_hash: String,
    pub writer_mode: String,
    pub expected_rate_hz: u32,
    pub map_peer_id: String,
    pub map_id: String,
    pub map_hash: String,
}

impl AukiStreamManifest {
    fn validate(&self) -> Result<(), AukiSdkError> {
        for (name, value, allow_empty) in [
            ("sensorId", self.sensor_id.as_str(), true),
            ("sensorHash", self.sensor_hash.as_str(), true),
            ("clockPeerId", self.clock_peer_id.as_str(), true),
            ("clockId", self.clock_id.as_str(), true),
            ("clockHash", self.clock_hash.as_str(), true),
            ("frameId", self.frame_id.as_str(), true),
            ("frameHash", self.frame_hash.as_str(), true),
            ("resourceId", self.resource_id.as_str(), false),
            ("payload", self.payload.as_str(), false),
            ("fromFrameId", self.from_frame_id.as_str(), true),
            ("fromFrameHash", self.from_frame_hash.as_str(), true),
            ("toFrameId", self.to_frame_id.as_str(), true),
            ("toFrameHash", self.to_frame_hash.as_str(), true),
            ("writerMode", self.writer_mode.as_str(), true),
            ("mapPeerId", self.map_peer_id.as_str(), true),
            ("mapId", self.map_id.as_str(), true),
            ("mapHash", self.map_hash.as_str(), true),
        ] {
            validate_control_string(name, value, allow_empty)?;
        }
        AukiStreamPayloadKind::from_manifest_payload(&self.payload).ok_or_else(|| {
            binding_error(format!(
                "unsupported Stream manifest payload {:?}",
                self.payload
            ))
        })?;
        Ok(())
    }
}

impl From<AukiStreamManifest> for StreamManifest {
    fn from(value: AukiStreamManifest) -> Self {
        Self {
            sensor_id: value.sensor_id,
            sensor_hash: value.sensor_hash,
            clock_peer_id: value.clock_peer_id,
            clock_id: value.clock_id,
            clock_hash: value.clock_hash,
            frame_id: value.frame_id,
            frame_hash: value.frame_hash,
            resource_id: value.resource_id,
            payload: value.payload,
            from_frame_id: value.from_frame_id,
            from_frame_hash: value.from_frame_hash,
            to_frame_id: value.to_frame_id,
            to_frame_hash: value.to_frame_hash,
            writer_mode: value.writer_mode,
            expected_rate_hz: value.expected_rate_hz,
            map_peer_id: value.map_peer_id,
            map_id: value.map_id,
            map_hash: value.map_hash,
        }
    }
}

impl From<&StreamManifest> for AukiStreamManifest {
    fn from(value: &StreamManifest) -> Self {
        Self {
            sensor_id: value.sensor_id.clone(),
            sensor_hash: value.sensor_hash.clone(),
            clock_peer_id: value.clock_peer_id.clone(),
            clock_id: value.clock_id.clone(),
            clock_hash: value.clock_hash.clone(),
            frame_id: value.frame_id.clone(),
            frame_hash: value.frame_hash.clone(),
            resource_id: value.resource_id.clone(),
            payload: value.payload.clone(),
            from_frame_id: value.from_frame_id.clone(),
            from_frame_hash: value.from_frame_hash.clone(),
            to_frame_id: value.to_frame_id.clone(),
            to_frame_hash: value.to_frame_hash.clone(),
            writer_mode: value.writer_mode.clone(),
            expected_rate_hz: value.expected_rate_hz,
            map_peer_id: value.map_peer_id.clone(),
            map_id: value.map_id.clone(),
            map_hash: value.map_hash.clone(),
        }
    }
}

fn validate_control_string(
    name: &'static str,
    value: &str,
    allow_empty: bool,
) -> Result<(), AukiSdkError> {
    if !allow_empty && value.is_empty() {
        return Err(binding_error(format!("{name} must not be empty")));
    }
    if value.len() > MAX_CONTROL_STRING_BYTES {
        return Err(binding_error(format!(
            "{name} is {} bytes; maximum is {MAX_CONTROL_STRING_BYTES}",
            value.len()
        )));
    }
    Ok(())
}

fn encode_control_json<T>(context: &'static str, value: &T) -> Result<String, AukiSdkError>
where
    T: Serialize,
{
    // Serializing through Value gives a normalized, stable key order with the
    // workspace's serde_json configuration. The control records contain only
    // strings and integers, so there is no cross-runtime float normalization.
    let value = serde_json::to_value(value).map_err(|error| operation_error(context, error))?;
    let json = serde_json::to_string(&value).map_err(|error| operation_error(context, error))?;
    if json.len() > MAX_CONTROL_JSON_BYTES {
        return Err(binding_error(format!(
            "{context}: JSON exceeds its {MAX_CONTROL_JSON_BYTES}-byte envelope"
        )));
    }
    Ok(json)
}

fn decode_control_json<T>(context: &'static str, json: &str) -> Result<T, AukiSdkError>
where
    T: DeserializeOwned,
{
    if json.len() > MAX_CONTROL_JSON_BYTES {
        return Err(binding_error(format!(
            "{context}: JSON exceeds its {MAX_CONTROL_JSON_BYTES}-byte envelope"
        )));
    }
    serde_json::from_str(json).map_err(|error| operation_error(context, error))
}

#[uniffi::export]
pub fn stream_request_to_json(request: AukiStreamRequest) -> Result<String, AukiSdkError> {
    request.validate()?;
    encode_control_json("encode Stream request", &request)
}

#[uniffi::export]
pub fn stream_request_from_json(json: String) -> Result<AukiStreamRequest, AukiSdkError> {
    let request = decode_control_json::<AukiStreamRequest>("decode Stream request", &json)?;
    request.validate()?;
    Ok(request)
}

#[uniffi::export]
pub fn stream_manifest_to_json(manifest: AukiStreamManifest) -> Result<String, AukiSdkError> {
    manifest.validate()?;
    encode_control_json("encode Stream manifest", &manifest)
}

#[uniffi::export]
pub fn stream_manifest_from_json(json: String) -> Result<AukiStreamManifest, AukiSdkError> {
    let manifest = decode_control_json::<AukiStreamManifest>("decode Stream manifest", &json)?;
    manifest.validate()?;
    Ok(manifest)
}

fn validate_target_domain(peer: &AukiPeer, target: &AukiPeerTarget) -> Result<(), AukiSdkError> {
    let local = Uuid::parse_str(&peer.domain_id())
        .map_err(|error| operation_error("parse local Domain ID", error))?;
    let remote = Uuid::parse_str(&target.domain_id)
        .map_err(|error| operation_error("parse remote Domain ID", error))?;
    if local == remote {
        Ok(())
    } else {
        Err(binding_error(format!(
            "remote peer Domain {} does not match local Domain {}",
            target.domain_id,
            peer.domain_id()
        )))
    }
}

/// Outbound Stream v2 client over one running peer.
#[derive(uniffi::Object)]
pub struct AukiStreamClient {
    inner: StreamClient,
    peer: Arc<AukiPeer>,
}

impl AukiStreamClient {
    fn from_inner(inner: StreamClient, peer: Arc<AukiPeer>) -> Arc<Self> {
        Arc::new(Self { inner, peer })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiStreamClient {
    #[uniffi::constructor]
    pub fn new(peer: Arc<AukiPeer>) -> Arc<Self> {
        Self::from_inner(StreamClient::new(peer.rust_protocols()), peer)
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    /// Open an exact authenticated subscription and validate its payload family.
    pub async fn subscribe(
        &self,
        target: AukiPeerTarget,
        payload_kind: AukiStreamPayloadKind,
        request: AukiStreamRequest,
    ) -> Result<Arc<AukiStreamSubscription>, AukiSdkError> {
        validate_target_domain(&self.peer, &target)?;
        request.validate()?;
        let (remote_peer_id, route) = parse_target(target)?;
        let subscription = subscribe_kind(
            &self.inner,
            remote_peer_id,
            route,
            payload_kind,
            request.into(),
        )
        .await
        .map_err(|error| operation_error("subscribe to Stream", error))?;
        payload_kind.validate_manifest(&subscription.manifest)?;
        Ok(Arc::new(AukiStreamSubscription::new(
            payload_kind,
            subscription.manifest,
            subscription.entries,
            Arc::clone(&self.peer),
        )))
    }
}

async fn subscribe_typed<T>(
    client: &StreamClient,
    remote_peer_id: auki_sdk_rs::PeerId,
    route: auki_sdk_rs::Multiaddr,
    request: StreamRequest,
) -> Result<StreamSubscription<T>, StreamEndpointError>
where
    T: StreamPayload,
{
    client
        .subscribe_exact::<T>(remote_peer_id, route, request)
        .await
}

async fn subscribe_kind(
    client: &StreamClient,
    remote_peer_id: auki_sdk_rs::PeerId,
    route: auki_sdk_rs::Multiaddr,
    payload_kind: AukiStreamPayloadKind,
    request: StreamRequest,
) -> Result<AnySubscription, StreamEndpointError> {
    match payload_kind {
        AukiStreamPayloadKind::Camera => {
            subscribe_typed::<CameraFrame>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::Camera))
        }
        AukiStreamPayloadKind::PointCloud => {
            subscribe_typed::<point_cloud::Data>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::PointCloud))
        }
        AukiStreamPayloadKind::JointEncoders => {
            subscribe_typed::<joint_encoders::Data>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::JointEncoders))
        }
        AukiStreamPayloadKind::Audio => {
            subscribe_typed::<audio::Data>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::Audio))
        }
        AukiStreamPayloadKind::Scalar => {
            subscribe_typed::<scalar::Data>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::Scalar))
        }
        AukiStreamPayloadKind::Pose => {
            subscribe_typed::<pose::SpatialTransform>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::Pose))
        }
        AukiStreamPayloadKind::Detection => {
            subscribe_typed::<DetectionFrame>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::Detection))
        }
        AukiStreamPayloadKind::Map => {
            subscribe_typed::<MapUpdate>(client, remote_peer_id, route, request)
                .await
                .map(|value| split_subscription(value, AnyEntries::Map))
        }
    }
}

struct AnySubscription {
    manifest: StreamManifest,
    entries: AnyEntries,
}

fn split_subscription<T>(
    subscription: StreamSubscription<T>,
    wrap: fn(SubscriptionEntries<T>) -> AnyEntries,
) -> AnySubscription {
    AnySubscription {
        manifest: subscription.manifest,
        entries: wrap(subscription.entries),
    }
}

enum AnyEntries {
    Camera(SubscriptionEntries<CameraFrame>),
    PointCloud(SubscriptionEntries<point_cloud::Data>),
    JointEncoders(SubscriptionEntries<joint_encoders::Data>),
    Audio(SubscriptionEntries<audio::Data>),
    Scalar(SubscriptionEntries<scalar::Data>),
    Pose(SubscriptionEntries<pose::SpatialTransform>),
    Detection(SubscriptionEntries<DetectionFrame>),
    Map(SubscriptionEntries<MapUpdate>),
}

impl AnyEntries {
    async fn next_encoded(&mut self) -> Option<Result<AukiStreamEntry, StreamError>> {
        match self {
            Self::Camera(entries) => next_typed(entries).await,
            Self::PointCloud(entries) => next_typed(entries).await,
            Self::JointEncoders(entries) => next_typed(entries).await,
            Self::Audio(entries) => next_typed(entries).await,
            Self::Scalar(entries) => next_typed(entries).await,
            Self::Pose(entries) => next_typed(entries).await,
            Self::Detection(entries) => next_typed(entries).await,
            Self::Map(entries) => next_typed(entries).await,
        }
    }
}

async fn next_typed<T>(
    entries: &mut SubscriptionEntries<T>,
) -> Option<Result<AukiStreamEntry, StreamError>>
where
    T: Message,
{
    entries.next().await.map(|entry| {
        entry.map(|entry: StreamEntry<T>| AukiStreamEntry {
            timestamp_ns: entry.timestamp_ns,
            sequence: entry.seq,
            payload: entry.payload.encode_to_vec(),
        })
    })
}

/// One validated Stream v2 item. Payload is the selected family's protobuf bytes.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiStreamEntry {
    pub timestamp_ns: i64,
    pub sequence: u64,
    pub payload: Vec<u8>,
}

/// Explicit producer terminal reason.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiStreamEndReason {
    SourceEnded,
    ProducerShuttingDown,
    SessionEnded,
    ProducerError { detail: String },
}

/// One demand-driven subscription result.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiStreamNext {
    Entry { entry: AukiStreamEntry },
    End { reason: AukiStreamEndReason },
}

fn end_reason_to_swift(reason: EndReason) -> Result<AukiStreamEndReason, AukiSdkError> {
    match reason.kind {
        Some(end_reason::Kind::SourceEnded(_)) => Ok(AukiStreamEndReason::SourceEnded),
        Some(end_reason::Kind::ProducerShuttingDown(_)) => {
            Ok(AukiStreamEndReason::ProducerShuttingDown)
        }
        Some(end_reason::Kind::SessionEnded(_)) => Ok(AukiStreamEndReason::SessionEnded),
        Some(end_reason::Kind::ProducerError(error)) => Ok(AukiStreamEndReason::ProducerError {
            detail: error.detail,
        }),
        None => Err(binding_error("Stream end reason has no kind")),
    }
}

struct SubscriptionSlot {
    entries: Option<AnyEntries>,
    closed: bool,
    next_pending: bool,
}

struct SubscriptionState {
    slot: Mutex<SubscriptionSlot>,
    cancel: watch::Sender<bool>,
    completed: watch::Sender<bool>,
    cleanup: DetachedCleanup,
}

impl SubscriptionState {
    fn new(entries: AnyEntries) -> Self {
        let (cancel, _) = watch::channel(false);
        let (completed, _) = watch::channel(false);
        Self {
            slot: Mutex::new(SubscriptionSlot {
                entries: Some(entries),
                closed: false,
                next_pending: false,
            }),
            cancel,
            completed,
            cleanup: DetachedCleanup::new(),
        }
    }

    fn begin_next(self: &Arc<Self>) -> Result<Option<PendingEntries>, AukiSdkError> {
        let mut slot = self.slot.lock();
        if slot.closed {
            return Ok(None);
        }
        if slot.next_pending {
            return Err(binding_error(
                "Stream subscription already has a pending next()",
            ));
        }
        let entries = slot
            .entries
            .take()
            .ok_or_else(|| binding_error("Stream subscription is unavailable"))?;
        slot.next_pending = true;
        Ok(Some(PendingEntries {
            state: Arc::clone(self),
            entries: Some(entries),
        }))
    }

    fn finish_next(&self, entries: AnyEntries, ended: bool) {
        let mut entries = Some(entries);
        let completed = {
            let mut slot = self.slot.lock();
            slot.next_pending = false;
            if slot.closed || ended {
                slot.closed = true;
                true
            } else {
                slot.entries = entries.take();
                false
            }
        };
        drop(entries);
        if completed {
            self.cancel.send_replace(true);
            self.completed.send_replace(true);
        }
    }

    fn begin_cancel(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.cancel.send_replace(true);
            let entries = {
                let mut slot = self.slot.lock();
                slot.closed = true;
                if slot.next_pending {
                    None
                } else {
                    slot.entries.take()
                }
            };
            if let Some(entries) = entries {
                drop(entries);
                self.completed.send_replace(true);
            }
            let completed = self.completed.subscribe();
            async move { wait_until_true(completed, "Stream subscription cleanup").await }
        })
    }
}

struct PendingEntries {
    state: Arc<SubscriptionState>,
    entries: Option<AnyEntries>,
}

impl PendingEntries {
    fn entries(&mut self) -> &mut AnyEntries {
        self.entries
            .as_mut()
            .expect("a pending Stream read owns its entries")
    }

    fn finish(mut self, ended: bool) {
        let entries = self
            .entries
            .take()
            .expect("a pending Stream read finishes only once");
        self.state.finish_next(entries, ended);
    }
}

impl Drop for PendingEntries {
    fn drop(&mut self) {
        if let Some(entries) = self.entries.take() {
            self.state.finish_next(entries, false);
        }
    }
}

async fn subscription_next(
    mut pending: PendingEntries,
) -> Option<Result<AukiStreamEntry, StreamError>> {
    let mut cancellation = pending.state.cancel.subscribe();
    let item = if *cancellation.borrow() {
        None
    } else {
        tokio::select! {
            biased;
            _ = cancellation.changed() => None,
            item = pending.entries().next_encoded() => item,
        }
    };
    let ended = !matches!(item, Some(Ok(_)));
    pending.finish(ended);
    item
}

async fn wait_until_true(
    mut completed: watch::Receiver<bool>,
    context: &'static str,
) -> Result<(), String> {
    loop {
        if *completed.borrow_and_update() {
            return Ok(());
        }
        if completed.changed().await.is_err() {
            return Err(format!("{context} ended without a completion signal"));
        }
    }
}

/// One accepted, demand-driven Stream v2 subscription.
#[derive(uniffi::Object)]
pub struct AukiStreamSubscription {
    payload_kind: AukiStreamPayloadKind,
    manifest: StreamManifest,
    state: Arc<SubscriptionState>,
    _peer: Arc<AukiPeer>,
}

impl AukiStreamSubscription {
    fn new(
        payload_kind: AukiStreamPayloadKind,
        manifest: StreamManifest,
        entries: AnyEntries,
        peer: Arc<AukiPeer>,
    ) -> Self {
        Self {
            payload_kind,
            manifest,
            state: Arc::new(SubscriptionState::new(entries)),
            _peer: peer,
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiStreamSubscription {
    pub fn payload_kind(&self) -> AukiStreamPayloadKind {
        self.payload_kind
    }

    pub fn manifest(&self) -> AukiStreamManifest {
        AukiStreamManifest::from(&self.manifest)
    }

    pub fn manifest_json(&self) -> Result<String, AukiSdkError> {
        stream_manifest_to_json(self.manifest())
    }

    /// Pull exactly one protobuf item or terminal reason.
    ///
    /// Only one `next()` call may be pending at a time. Cancelling the Swift
    /// task returns ownership to this subscription so a later call remains safe.
    pub async fn next(&self) -> Result<Option<AukiStreamNext>, AukiSdkError> {
        let Some(pending) = self.state.begin_next()? else {
            return Ok(None);
        };
        match subscription_next(pending).await {
            Some(Ok(entry)) => Ok(Some(AukiStreamNext::Entry { entry })),
            Some(Err(StreamError::EndOfStream { reason })) => Ok(Some(AukiStreamNext::End {
                reason: end_reason_to_swift(reason)?,
            })),
            Some(Err(error)) => Err(operation_error("read Stream entry", error)),
            None => Ok(None),
        }
    }

    /// Idempotently interrupt a pending read and release the authenticated route.
    pub async fn cancel(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.state.begin_cancel())
            .await
            .map_err(|error| operation_error("cancel Stream subscription", error))
    }
}

impl Drop for AukiStreamSubscription {
    fn drop(&mut self) {
        let _ = self.state.begin_cancel();
    }
}

/// Configuration for one Rust-owned live Stream source.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiStreamProducerConfig {
    /// Original source Peer ID. Empty means the mounted local peer.
    pub source_peer_id: String,
    pub payload_kind: AukiStreamPayloadKind,
    pub manifest: AukiStreamManifest,
    /// Empty means every mutually authenticated peer in the Domain.
    pub allowed_requester_peer_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SourceKey {
    source_peer_id: String,
    resource_id: String,
}

#[derive(Clone)]
enum SourceEvent {
    Item {
        timestamp_ns: i64,
        payload: Arc<[u8]>,
    },
    Failure(String),
}

struct ProducerSlot {
    closed: bool,
    published: u64,
    next_subscriber_id: u64,
    subscribers: HashMap<u64, Sender<SourceEvent>>,
    emit_pending: bool,
    broadcast_started: bool,
    pending_completion: Option<watch::Receiver<Option<CleanupResult>>>,
}

struct ProducerState {
    key: SourceKey,
    payload_kind: AukiStreamPayloadKind,
    manifest: StreamManifest,
    allowed_requesters: HashSet<String>,
    slot: Mutex<ProducerSlot>,
    changed: watch::Sender<u64>,
    cleanup: DetachedCleanup,
    runtime: Handle,
}

impl ProducerState {
    fn new(
        key: SourceKey,
        payload_kind: AukiStreamPayloadKind,
        manifest: StreamManifest,
        allowed_requesters: HashSet<String>,
    ) -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            key,
            payload_kind,
            manifest,
            allowed_requesters,
            slot: Mutex::new(ProducerSlot {
                closed: false,
                published: 0,
                next_subscriber_id: 0,
                subscribers: HashMap::new(),
                emit_pending: false,
                broadcast_started: false,
                pending_completion: None,
            }),
            changed,
            cleanup: DetachedCleanup::new(),
            runtime: Handle::current(),
        }
    }

    fn is_closed(&self) -> bool {
        self.slot.lock().closed
    }

    fn requester_allowed(&self, peer_id: &auki_sdk_rs::PeerId) -> bool {
        self.allowed_requesters.is_empty() || self.allowed_requesters.contains(&peer_id.to_string())
    }

    fn subscribe(
        self: &Arc<Self>,
        read_from: ReadFrom,
    ) -> Result<RegisteredSubscriber, SourceSubscribeError> {
        let mut slot = self.slot.lock();
        if slot.closed {
            return Err(SourceSubscribeError::Closed);
        }
        if !matches!(read_from, ReadFrom::Latest) && (slot.published != 0 || slot.broadcast_started)
        {
            return Err(SourceSubscribeError::HistoryUnavailable);
        }
        let subscriber_id = slot.next_subscriber_id;
        slot.next_subscriber_id = slot
            .next_subscriber_id
            .checked_add(1)
            .ok_or(SourceSubscribeError::Exhausted)?;
        let (sender, receiver) = async_channel::bounded(1);
        slot.subscribers.insert(subscriber_id, sender);
        drop(slot);
        self.changed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        Ok(RegisteredSubscriber {
            subscriber_id,
            receiver,
            from_timestamp: match read_from {
                ReadFrom::FromTimestamp(timestamp_ns) => Some(timestamp_ns),
                ReadFrom::Latest | ReadFrom::FromStart => None,
            },
            producer: Arc::downgrade(self),
        })
    }

    fn unregister(&self, subscriber_id: u64) {
        self.slot.lock().subscribers.remove(&subscriber_id);
        self.changed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    fn begin_emit(
        self: &Arc<Self>,
        event: SourceEvent,
        counts_as_publish: bool,
        close_after: bool,
    ) -> Result<watch::Receiver<Option<CleanupResult>>, AukiSdkError> {
        let (completion, receiver) = watch::channel(None);
        {
            let mut slot = self.slot.lock();
            if slot.closed {
                return Err(binding_error("Stream producer is closed"));
            }
            if slot.emit_pending {
                return Err(binding_error(
                    "Stream producer already has a pending push or failure",
                ));
            }
            slot.emit_pending = true;
            slot.broadcast_started = false;
            slot.pending_completion = Some(receiver.clone());
        }

        let state = Arc::clone(self);
        self.runtime.spawn(async move {
            let result = broadcast_event(&state, event).await;
            state.finish_emit(result.is_ok() && counts_as_publish, close_after);
            completion.send_replace(Some(
                result.map_err(|error| Arc::<str>::from(error.as_str())),
            ));
        });
        Ok(receiver)
    }

    fn finish_emit(&self, published: bool, close_after: bool) {
        let senders = {
            let mut slot = self.slot.lock();
            slot.emit_pending = false;
            slot.broadcast_started = false;
            if published {
                slot.published = slot.published.saturating_add(1);
            }
            if close_after {
                slot.closed = true;
                Some(
                    slot.subscribers
                        .drain()
                        .map(|(_, sender)| sender)
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        };
        if let Some(senders) = senders {
            for sender in senders {
                sender.close();
            }
        }
        self.changed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    fn close_now(&self) -> Option<watch::Receiver<Option<CleanupResult>>> {
        let (senders, pending) = {
            let mut slot = self.slot.lock();
            slot.closed = true;
            (
                slot.subscribers
                    .drain()
                    .map(|(_, sender)| sender)
                    .collect::<Vec<_>>(),
                slot.pending_completion.clone(),
            )
        };
        for sender in senders {
            sender.close();
        }
        self.changed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        pending
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let pending = self.close_now();
            async move {
                if let Some(pending) = pending {
                    // Closing intentionally interrupts an item that is waiting
                    // for a consumer. Preserve that error for the observing
                    // `push`, but do not turn expected producer cleanup into a
                    // failed endpoint shutdown.
                    let _ = wait_cleanup(pending).await;
                }
                Ok::<_, String>(())
            }
        })
    }
}

async fn broadcast_event(state: &Arc<ProducerState>, event: SourceEvent) -> Result<(), String> {
    let mut changed = state.changed.subscribe();
    loop {
        let subscribers = {
            let mut slot = state.slot.lock();
            slot.subscribers.retain(|_, sender| !sender.is_closed());
            if slot.closed {
                return Err("Stream producer closed before the pending item was delivered".into());
            }
            if slot.subscribers.is_empty() {
                None
            } else {
                slot.broadcast_started = true;
                Some(
                    slot.subscribers
                        .iter()
                        .map(|(id, sender)| (*id, sender.clone()))
                        .collect::<Vec<_>>(),
                )
            }
        };

        let Some(subscribers) = subscribers else {
            if changed.changed().await.is_err() {
                return Err("Stream producer notification channel closed".into());
            }
            continue;
        };

        let mut delivered = false;
        let mut closed = Vec::new();
        for (subscriber_id, sender) in subscribers {
            match sender.send(event.clone()).await {
                Ok(()) => delivered = true,
                Err(_) => closed.push(subscriber_id),
            }
        }
        if !closed.is_empty() {
            let mut slot = state.slot.lock();
            for subscriber_id in closed {
                slot.subscribers.remove(&subscriber_id);
            }
        }
        if delivered {
            return Ok(());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceSubscribeError {
    Closed,
    HistoryUnavailable,
    Exhausted,
}

struct RegisteredSubscriber {
    subscriber_id: u64,
    receiver: Receiver<SourceEvent>,
    from_timestamp: Option<i64>,
    producer: Weak<ProducerState>,
}

impl Drop for RegisteredSubscriber {
    fn drop(&mut self) {
        if let Some(producer) = self.producer.upgrade() {
            producer.unregister(self.subscriber_id);
        }
    }
}

fn typed_source<T>(subscriber: RegisteredSubscriber) -> SourceStream<T>
where
    T: Message + Default + Send + 'static,
{
    Box::pin(futures::stream::unfold(
        subscriber,
        |subscriber| async move {
            let subscriber = subscriber;
            loop {
                let event = subscriber.receiver.recv().await.ok()?;
                match event {
                    SourceEvent::Item {
                        timestamp_ns,
                        payload,
                    } => {
                        if subscriber
                            .from_timestamp
                            .is_some_and(|minimum| timestamp_ns < minimum)
                        {
                            continue;
                        }
                        let item = T::decode(payload.as_ref())
                            .map(|payload| StreamItem {
                                timestamp_ns,
                                payload,
                            })
                            .map_err(|_| {
                                "Rust-owned Stream source contained invalid protobuf bytes".into()
                            });
                        return Some((item, subscriber));
                    }
                    SourceEvent::Failure(detail) => return Some((Err(detail), subscriber)),
                }
            }
        },
    ))
}

fn accepted_dispatch(
    state: Arc<ProducerState>,
    request: &StreamRequest,
) -> Result<StreamDispatch, SourceSubscribeError> {
    let subscriber = state.subscribe(request.from)?;
    let manifest = state.manifest.clone();
    Ok(match state.payload_kind {
        AukiStreamPayloadKind::Camera => StreamDispatch::AcceptCamera {
            manifest,
            source: typed_source(subscriber),
        },
        AukiStreamPayloadKind::PointCloud => StreamDispatch::AcceptPointCloud {
            manifest,
            source: typed_source(subscriber),
        },
        AukiStreamPayloadKind::JointEncoders => StreamDispatch::AcceptJointEncoders {
            manifest,
            source: typed_source(subscriber),
        },
        AukiStreamPayloadKind::Audio => StreamDispatch::AcceptAudio {
            manifest,
            source: typed_source(subscriber),
        },
        AukiStreamPayloadKind::Scalar => StreamDispatch::AcceptScalar {
            manifest,
            source: typed_source(subscriber),
        },
        AukiStreamPayloadKind::Pose => StreamDispatch::AcceptPose {
            manifest,
            source: typed_source(subscriber),
        },
        AukiStreamPayloadKind::Detection => StreamDispatch::AcceptDetection {
            manifest,
            source: typed_source(subscriber),
        },
        AukiStreamPayloadKind::Map => StreamDispatch::AcceptMap {
            manifest,
            source: typed_source(subscriber),
        },
    })
}

struct SourceRegistrySlot {
    closing: bool,
    sources: HashMap<SourceKey, Weak<ProducerState>>,
}

struct StreamSourceRegistry {
    local_peer_id: String,
    slot: RwLock<SourceRegistrySlot>,
}

impl StreamSourceRegistry {
    fn new(local_peer_id: String) -> Self {
        Self {
            local_peer_id,
            slot: RwLock::new(SourceRegistrySlot {
                closing: false,
                sources: HashMap::new(),
            }),
        }
    }

    fn register(
        &self,
        config: AukiStreamProducerConfig,
    ) -> Result<Arc<ProducerState>, AukiSdkError> {
        config.manifest.validate()?;
        let manifest: StreamManifest = config.manifest.into();
        config.payload_kind.validate_manifest(&manifest)?;

        let source_peer_id = if config.source_peer_id.is_empty() {
            self.local_peer_id.clone()
        } else {
            config
                .source_peer_id
                .parse::<auki_sdk_rs::PeerId>()
                .map_err(|error| operation_error("parse producer source Peer ID", error))?
                .to_string()
        };
        if config.allowed_requester_peer_ids.len() > MAX_ALLOWED_REQUESTERS {
            return Err(binding_error(format!(
                "Stream producer allows at most {MAX_ALLOWED_REQUESTERS} requester Peer IDs"
            )));
        }
        let allowed_requesters = config
            .allowed_requester_peer_ids
            .into_iter()
            .map(|value| {
                value
                    .parse::<auki_sdk_rs::PeerId>()
                    .map(|peer_id| peer_id.to_string())
                    .map_err(|error| {
                        operation_error("parse allowed Stream requester Peer ID", error)
                    })
            })
            .collect::<Result<HashSet<_>, _>>()?;
        let key = SourceKey {
            source_peer_id,
            resource_id: manifest.resource_id.clone(),
        };
        let state = Arc::new(ProducerState::new(
            key.clone(),
            config.payload_kind,
            manifest,
            allowed_requesters,
        ));

        let mut slot = self.slot.write();
        if slot.closing {
            return Err(binding_error("Stream endpoint is closing"));
        }
        slot.sources.retain(|_, source| source.strong_count() != 0);
        if slot
            .sources
            .get(&key)
            .and_then(Weak::upgrade)
            .is_some_and(|existing| !existing.is_closed())
        {
            return Err(binding_error(format!(
                "Stream producer already exists for {}/{}",
                key.source_peer_id, key.resource_id
            )));
        }
        slot.sources.insert(key, Arc::downgrade(&state));
        Ok(state)
    }

    fn dispatch(
        &self,
        requester: &auki_sdk_rs::AuthenticatedPeer,
        mut request: StreamRequest,
    ) -> StreamDispatch {
        let state = {
            let slot = self.slot.read();
            if slot.closing {
                return StreamDispatch::Decline {
                    reason: DeclineReason::producer_shutting_down(),
                };
            }
            if request.source_peer_id.is_empty() {
                request.source_peer_id = self.local_peer_id.clone();
            }
            slot.sources
                .get(&SourceKey {
                    source_peer_id: request.source_peer_id.clone(),
                    resource_id: request.resource_id.clone(),
                })
                .and_then(Weak::upgrade)
        };
        let Some(state) = state else {
            return StreamDispatch::Decline {
                reason: DeclineReason::sensor_not_found(),
            };
        };
        if !state.requester_allowed(&requester.peer_id) {
            // Hide source existence from an authenticated but unauthorized peer.
            return StreamDispatch::Decline {
                reason: DeclineReason::sensor_not_found(),
            };
        }
        match accepted_dispatch(state, &request) {
            Ok(dispatch) => dispatch,
            Err(SourceSubscribeError::Closed) => StreamDispatch::Decline {
                reason: DeclineReason::producer_shutting_down(),
            },
            Err(SourceSubscribeError::HistoryUnavailable) => StreamDispatch::Decline {
                reason: DeclineReason::sensor_unavailable(),
            },
            Err(SourceSubscribeError::Exhausted) => StreamDispatch::Decline {
                reason: DeclineReason::other("Stream subscriber identity space exhausted"),
            },
        }
    }

    fn begin_shutdown(&self) -> Vec<watch::Receiver<Option<CleanupResult>>> {
        let sources = {
            let mut slot = self.slot.write();
            slot.closing = true;
            let sources = slot
                .sources
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>();
            slot.sources.clear();
            sources
        };
        sources
            .into_iter()
            .map(|source| source.begin_close())
            .collect()
    }
}

#[derive(Clone)]
struct RegistryStreamProvider {
    sources: Arc<StreamSourceRegistry>,
}

impl StreamProvider for RegistryStreamProvider {
    fn dispatch(
        &self,
        remote_peer: &auki_sdk_rs::AuthenticatedPeer,
        request: StreamRequest,
    ) -> StreamDispatch {
        self.sources.dispatch(remote_peer, request)
    }
}

/// Rust-owned producer for one exact source/resource and protobuf family.
#[derive(uniffi::Object)]
pub struct AukiStreamProducer {
    state: Arc<ProducerState>,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiStreamProducer {
    pub fn source_peer_id(&self) -> String {
        self.state.key.source_peer_id.clone()
    }

    pub fn resource_id(&self) -> String {
        self.state.key.resource_id.clone()
    }

    pub fn payload_kind(&self) -> AukiStreamPayloadKind {
        self.state.payload_kind
    }

    pub fn manifest(&self) -> AukiStreamManifest {
        AukiStreamManifest::from(&self.state.manifest)
    }

    /// Push one protobuf value to every active consumer.
    ///
    /// The call waits when there is no consumer and when any consumer's
    /// one-item queue is full. Only one push or failure may be pending.
    pub async fn push(&self, timestamp_ns: i64, payload: Vec<u8>) -> Result<(), AukiSdkError> {
        self.state.payload_kind.validate_payload(&payload)?;
        let completion = self.state.begin_emit(
            SourceEvent::Item {
                timestamp_ns,
                payload: Arc::<[u8]>::from(payload),
            },
            true,
            false,
        )?;
        wait_cleanup(completion)
            .await
            .map_err(|error| operation_error("push Stream item", error))
    }

    /// Deliver one bounded producer error, then close this source.
    pub async fn fail(&self, detail: String) -> Result<(), AukiSdkError> {
        validate_control_string("Stream producer error", &detail, false)?;
        if detail.len() > MAX_REMOTE_ERROR_BYTES {
            return Err(binding_error(format!(
                "Stream producer error is {} bytes; maximum is {MAX_REMOTE_ERROR_BYTES}",
                detail.len()
            )));
        }
        let completion = self
            .state
            .begin_emit(SourceEvent::Failure(detail), false, true)?;
        wait_cleanup(completion)
            .await
            .map_err(|error| operation_error("fail Stream producer", error))
    }

    /// Idempotently finish the source and unblock all consumers and pending push.
    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.state.begin_close())
            .await
            .map_err(|error| operation_error("close Stream producer", error))
    }
}

impl Drop for AukiStreamProducer {
    fn drop(&mut self) {
        let _ = self.state.begin_close();
    }
}

struct StreamEndpointOwner {
    endpoint: Mutex<Option<StreamEndpoint>>,
    sources: Arc<StreamSourceRegistry>,
    cleanup: DetachedCleanup,
}

impl StreamEndpointOwner {
    fn new(endpoint: StreamEndpoint, sources: Arc<StreamSourceRegistry>) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            sources,
            cleanup: DetachedCleanup::new(),
        }
    }

    fn ensure_open(&self) -> Result<(), AukiSdkError> {
        if self.endpoint.lock().is_some() {
            Ok(())
        } else {
            Err(binding_error("Stream endpoint is stopped"))
        }
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            // Fence source registration and dispatch before the observing Swift
            // Task can be cancelled.
            let producer_cleanup = self.sources.begin_shutdown();
            let close = self.endpoint.lock().take().map(StreamEndpoint::close);
            async move {
                let mut first_error = None;
                for cleanup in producer_cleanup {
                    if let Err(error) = wait_cleanup(cleanup).await
                        && first_error.is_none()
                    {
                        first_error = Some(error.to_string());
                    }
                }
                if let Some(close) = close
                    && let Err(error) = close.await
                    && first_error.is_none()
                {
                    first_error = Some(error.to_string());
                }
                match first_error {
                    Some(error) => Err(error),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for StreamEndpointOwner {
    fn drop(&mut self) {
        if self.endpoint.get_mut().is_some() {
            let _ = self.begin_close();
        }
    }
}

/// Mounted Stream v2 endpoint backed by explicit Rust-owned producers.
#[derive(uniffi::Object)]
pub struct AukiStreamEndpoint {
    owner: StreamEndpointOwner,
    client: StreamClient,
    peer: Arc<AukiPeer>,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiStreamEndpoint {
    /// Mount Stream v2 without a Swift callback.
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiSdkError> {
        let sources = Arc::new(StreamSourceRegistry::new(peer.peer_id()));
        let endpoint = StreamEndpoint::mount(
            peer.rust_protocols(),
            RegistryStreamProvider {
                sources: Arc::clone(&sources),
            },
        )
        .map_err(|error| operation_error("mount Stream endpoint", error))?;
        let client = endpoint.client();
        Ok(Arc::new(Self {
            owner: StreamEndpointOwner::new(endpoint, sources),
            client,
            peer,
        }))
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    pub fn client(&self) -> Arc<AukiStreamClient> {
        AukiStreamClient::from_inner(self.client.clone(), Arc::clone(&self.peer))
    }

    /// Register one live source. Creating producers is async so their detached
    /// cleanup always captures the UniFFI Tokio runtime rather than a Swift thread.
    pub async fn create_producer(
        &self,
        config: AukiStreamProducerConfig,
    ) -> Result<Arc<AukiStreamProducer>, AukiSdkError> {
        self.owner.ensure_open()?;
        let state = self.owner.sources.register(config)?;
        Ok(Arc::new(AukiStreamProducer { state }))
    }

    /// Fence admission, close every producer, then await admitted handlers.
    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Stream endpoint", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(kind: AukiStreamPayloadKind) -> AukiStreamManifest {
        AukiStreamManifest {
            resource_id: "sensor/front".into(),
            payload: kind.manifest_payload().into(),
            ..Default::default()
        }
    }

    fn source_state(kind: AukiStreamPayloadKind) -> Arc<ProducerState> {
        Arc::new(ProducerState::new(
            SourceKey {
                source_peer_id: "source".into(),
                resource_id: "sensor/front".into(),
            },
            kind,
            manifest(kind).into(),
            HashSet::new(),
        ))
    }

    fn run_test(future: impl std::future::Future<Output = ()>) {
        tokio::runtime::Runtime::new().unwrap().block_on(future);
    }

    #[test]
    fn payload_kinds_lock_all_eight_manifest_and_protobuf_families() {
        let cases = [
            (
                AukiStreamPayloadKind::Camera,
                CameraFrame::default().encode_to_vec(),
            ),
            (
                AukiStreamPayloadKind::PointCloud,
                point_cloud::Data::default().encode_to_vec(),
            ),
            (
                AukiStreamPayloadKind::JointEncoders,
                joint_encoders::Data::default().encode_to_vec(),
            ),
            (
                AukiStreamPayloadKind::Audio,
                audio::Data::default().encode_to_vec(),
            ),
            (
                AukiStreamPayloadKind::Scalar,
                scalar::Data::default().encode_to_vec(),
            ),
            (
                AukiStreamPayloadKind::Pose,
                pose::SpatialTransform::default().encode_to_vec(),
            ),
            (
                AukiStreamPayloadKind::Detection,
                DetectionFrame::default().encode_to_vec(),
            ),
            (
                AukiStreamPayloadKind::Map,
                MapUpdate::default().encode_to_vec(),
            ),
        ];
        for (kind, payload) in cases {
            let core: StreamManifest = manifest(kind).into();
            assert!(kind.validate_manifest(&core).is_ok());
            assert!(kind.validate_payload(&payload).is_ok());
            assert_eq!(
                AukiStreamPayloadKind::from_manifest_payload(kind.manifest_payload()),
                Some(kind)
            );
        }
        assert!(
            AukiStreamPayloadKind::Camera
                .validate_payload(b"not protobuf")
                .is_err()
        );
    }

    #[test]
    fn camera_image_codec_matches_the_locked_protobuf_shape() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];
        let encoded = encode_camera_frame_image(jpeg.clone()).unwrap();
        assert_eq!(encoded, [0x12, 0x04, 0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(decode_camera_frame_image(encoded).unwrap(), jpeg);

        let error = decode_camera_frame_image(vec![0xff]).unwrap_err();
        assert!(error.to_string().starts_with("decode CameraFrame:"));
    }

    #[test]
    fn camera_image_codec_enforces_the_stream_frame_bound() {
        let maximum = MAX_FRAME_BYTES as usize - CAMERA_FRAME_CODEC_OVERHEAD_BYTES;
        let error = encode_camera_frame_image(vec![0_u8; maximum + 1]).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "encode CameraFrame image: image is {} bytes; maximum is {maximum}",
                maximum + 1
            )
        );

        let oversized = MAX_FRAME_BYTES as usize + 1;
        let error = decode_camera_frame_image(vec![0_u8; oversized]).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "decode CameraFrame image: payload is {oversized} bytes; maximum is {MAX_FRAME_BYTES}"
            )
        );
    }

    #[test]
    fn control_json_is_bounded_normalized_and_round_trips() {
        let request = AukiStreamRequest {
            source_peer_id: String::new(),
            resource_id: "sensor/front".into(),
            read_from: AukiStreamReadFrom::FromTimestamp { timestamp_ns: 42 },
        };
        let json = stream_request_to_json(request.clone()).unwrap();
        assert_eq!(stream_request_from_json(json).unwrap(), request);

        let stream_manifest = manifest(AukiStreamPayloadKind::Camera);
        let json = stream_manifest_to_json(stream_manifest.clone()).unwrap();
        assert_eq!(stream_manifest_from_json(json).unwrap(), stream_manifest);
        let minimal = stream_manifest_from_json(
            r#"{"resourceId":"playground/scalar","payload":"scalar"}"#.into(),
        )
        .unwrap();
        assert_eq!(minimal.resource_id, "playground/scalar");
        assert_eq!(minimal.payload, "scalar");
        assert!(stream_manifest_from_json("x".repeat(MAX_CONTROL_JSON_BYTES + 1)).is_err());
    }

    #[test]
    fn producer_uses_a_one_item_backpressure_queue() {
        run_test(async {
            let state = source_state(AukiStreamPayloadKind::Camera);
            let subscriber = state.subscribe(ReadFrom::Latest).unwrap();
            let first = state
                .begin_emit(
                    SourceEvent::Item {
                        timestamp_ns: 1,
                        payload: Arc::from(CameraFrame::default().encode_to_vec()),
                    },
                    true,
                    false,
                )
                .unwrap();
            wait_cleanup(first).await.unwrap();

            let second = state
                .begin_emit(
                    SourceEvent::Item {
                        timestamp_ns: 2,
                        payload: Arc::from(CameraFrame::default().encode_to_vec()),
                    },
                    true,
                    false,
                )
                .unwrap();
            tokio::task::yield_now().await;
            assert!(
                second.borrow().is_none(),
                "second item bypassed backpressure"
            );

            let first_item = subscriber.receiver.recv().await.unwrap();
            assert!(matches!(
                first_item,
                SourceEvent::Item {
                    timestamp_ns: 1,
                    ..
                }
            ));
            wait_cleanup(second).await.unwrap();
            state.begin_close();
        });
    }

    #[test]
    fn producer_rejects_two_pending_pushes_and_close_interrupts_the_first() {
        run_test(async {
            let state = source_state(AukiStreamPayloadKind::Camera);
            let first = state
                .begin_emit(
                    SourceEvent::Item {
                        timestamp_ns: 1,
                        payload: Arc::from(CameraFrame::default().encode_to_vec()),
                    },
                    true,
                    false,
                )
                .unwrap();
            assert!(
                state
                    .begin_emit(
                        SourceEvent::Item {
                            timestamp_ns: 2,
                            payload: Arc::from(CameraFrame::default().encode_to_vec()),
                        },
                        true,
                        false,
                    )
                    .is_err()
            );
            let closed = state.begin_close();
            assert!(wait_cleanup(first).await.is_err());
            assert!(wait_cleanup(closed).await.is_ok());
        });
    }

    #[test]
    fn subscription_enforces_one_pending_next_and_cancel_releases_it() {
        run_test(async {
            let entries = AnyEntries::Camera(Box::pin(futures::stream::pending()));
            let state = Arc::new(SubscriptionState::new(entries));
            let pending = state.begin_next().unwrap().unwrap();
            assert!(state.begin_next().is_err());
            let cleanup = state.begin_cancel();
            assert!(subscription_next(pending).await.is_none());
            assert!(wait_cleanup(cleanup).await.is_ok());
        });
    }

    #[test]
    fn historical_requests_are_declined_after_the_first_published_item() {
        run_test(async {
            let state = source_state(AukiStreamPayloadKind::Camera);
            let subscriber = state.subscribe(ReadFrom::Latest).unwrap();
            let pushed = state
                .begin_emit(
                    SourceEvent::Item {
                        timestamp_ns: 1,
                        payload: Arc::from(CameraFrame::default().encode_to_vec()),
                    },
                    true,
                    false,
                )
                .unwrap();
            wait_cleanup(pushed).await.unwrap();
            let _ = subscriber.receiver.recv().await.unwrap();
            assert!(matches!(
                state.subscribe(ReadFrom::FromStart),
                Err(SourceSubscribeError::HistoryUnavailable)
            ));
            state.begin_close();
        });
    }
}

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use auki_datatypes::{
    audio, camera::CameraFrame, detection::DetectionFrame, joint_encoders, map::MapUpdate,
    point_cloud, pose, scalar,
};
#[cfg(test)]
use auki_protocols::stream::v2::decline_reason;
use auki_protocols::stream::{
    SourceStream, StreamClient, StreamDispatch, StreamEndpoint, StreamEntry, StreamError,
    StreamItem, StreamProvider, StreamSubscription, SubscriptionEntries,
    v2::{DeclineReason, EndReason, ID, ReadFrom, StreamManifest, StreamRequest, end_reason},
};
use auki_sdk::AuthenticatedPeer;
use futures::{FutureExt, StreamExt, pin_mut};
use js_sys::{AsyncIterator, Function, Promise, Reflect, Symbol, Uint8Array};
use prost::Message;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::{
    AukiPeer,
    protocol_support::{
        CloseBarrier, authenticated_peer_to_js, javascript_error_reason, js_context, js_error,
        parse_exact_target, peer_protocols, to_js_value,
    },
};

const PAYLOAD_KIND_NAMES: &str =
    "camera, point_cloud, joint_encoders, audio, scalar, pose, detection, or map";
const REMOTE_PROVIDER_FAILURE_DETAIL: &str = "Stream provider failed";
const REMOTE_SOURCE_FAILURE_DETAIL: &str = "Stream provider source failed";

#[cfg(not(test))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(value: &JsValue);
}

#[cfg(not(test))]
fn report_local_stream_error(context: &str, detail: &str) {
    console_error(&js_error(format!("{context}: {detail}")));
}

#[cfg(test)]
fn report_local_stream_error(_context: &str, _detail: &str) {}

/// Outbound Stream v2 client backed by the portable Rust protocol.
#[wasm_bindgen]
pub struct AukiStreamClient {
    inner: StreamClient,
}

#[wasm_bindgen]
impl AukiStreamClient {
    /// Bind an outbound Stream client to one running browser peer.
    #[wasm_bindgen(constructor)]
    pub fn new(peer: &AukiPeer) -> Result<AukiStreamClient, JsValue> {
        Ok(Self {
            inner: StreamClient::new(peer_protocols(peer, "Stream")?),
        })
    }

    /// Immutable authenticated protocol identifier implemented by this client.
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        ID.to_owned()
    }

    /// Open one typed, demand-driven subscription through an exact advertised route.
    ///
    /// The caller resolves `payloadKind` from the resource's Catalog/Registry
    /// contract. The accepted manifest must name that exact payload family.
    #[wasm_bindgen(js_name = subscribeExact)]
    pub async fn subscribe_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
        #[wasm_bindgen(
            js_name = payloadKind,
            unchecked_param_type = "AukiStreamPayloadKind"
        )]
        payload_kind: String,
        #[wasm_bindgen(unchecked_param_type = "AukiStreamRequest")] request: JsValue,
    ) -> Result<AukiStreamSubscription, JsValue> {
        let payload_kind = StreamPayloadKind::parse(&payload_kind)?;
        let request = stream_request_from_js(request)?;
        let (peer_id, route) = parse_exact_target(target)?;

        let subscription = match payload_kind {
            StreamPayloadKind::Camera => self
                .inner
                .subscribe_exact::<CameraFrame>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::Camera)),
            StreamPayloadKind::PointCloud => self
                .inner
                .subscribe_exact::<point_cloud::Data>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::PointCloud)),
            StreamPayloadKind::JointEncoders => self
                .inner
                .subscribe_exact::<joint_encoders::Data>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::JointEncoders)),
            StreamPayloadKind::Audio => self
                .inner
                .subscribe_exact::<audio::Data>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::Audio)),
            StreamPayloadKind::Scalar => self
                .inner
                .subscribe_exact::<scalar::Data>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::Scalar)),
            StreamPayloadKind::Pose => self
                .inner
                .subscribe_exact::<pose::SpatialTransform>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::Pose)),
            StreamPayloadKind::Detection => self
                .inner
                .subscribe_exact::<DetectionFrame>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::Detection)),
            StreamPayloadKind::Map => self
                .inner
                .subscribe_exact::<MapUpdate>(peer_id, route, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::Map)),
        }
        .map_err(|error| js_context("subscribe to Stream", error))?;
        payload_kind.validate_manifest(&subscription.manifest)?;

        Ok(AukiStreamSubscription::new(
            payload_kind,
            subscription.manifest,
            subscription.entries,
        ))
    }
}

/// Mounted inbound Stream v2 service backed by a JavaScript admission callback.
#[wasm_bindgen]
pub struct AukiStreamEndpoint {
    inner: RefCell<Option<StreamEndpoint>>,
    closing: CloseBarrier,
}

#[wasm_bindgen]
impl AukiStreamEndpoint {
    /// Mount Stream v2 on one running browser peer.
    ///
    /// `provider` is invoked synchronously for each authenticated request. It
    /// returns either a decline reason or an accepted manifest plus an async
    /// iterable of typed protobuf payloads.
    #[wasm_bindgen(js_name = mount)]
    pub fn mount(
        peer: &AukiPeer,
        #[wasm_bindgen(unchecked_param_type = "AukiStreamProvider")] provider: Function,
    ) -> Result<AukiStreamEndpoint, JsValue> {
        let endpoint = StreamEndpoint::mount(
            peer_protocols(peer, "Stream endpoint")?,
            BrowserStreamProvider { provider },
        )
        .map_err(|error| js_context("mount Stream endpoint", error))?;
        Ok(Self {
            inner: RefCell::new(Some(endpoint)),
            closing: CloseBarrier::default(),
        })
    }

    /// Idempotently stop accepting subscriptions and await admitted handlers.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let endpoint = self.inner.borrow_mut().take();
            future_to_promise(async move {
                if let Some(endpoint) = endpoint {
                    endpoint
                        .close()
                        .await
                        .map_err(|error| js_context("close Stream endpoint", error))?;
                }
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

struct BrowserStreamProvider {
    provider: Function,
}

impl StreamProvider for BrowserStreamProvider {
    fn dispatch(&self, remote_peer: &AuthenticatedPeer, request: StreamRequest) -> StreamDispatch {
        browser_stream_dispatch(&self.provider, remote_peer, &request).unwrap_or_else(|error| {
            report_local_stream_error("Stream provider dispatch failed", &js_error_message(error));
            StreamDispatch::Decline {
                reason: DeclineReason::other(REMOTE_PROVIDER_FAILURE_DETAIL),
            }
        })
    }
}

fn browser_stream_dispatch(
    provider: &Function,
    remote_peer: &AuthenticatedPeer,
    request: &StreamRequest,
) -> Result<StreamDispatch, JsValue> {
    let remote_peer =
        authenticated_peer_to_js("convert authenticated Stream requester", remote_peer)?;
    let request = stream_request_to_js(request)?;
    let dispatch = provider
        .call2(&JsValue::UNDEFINED, &remote_peer, &request)
        .map_err(|error| js_context("call Stream provider", js_error_message(error)))?;
    let kind = string_property(&dispatch, "kind", "read Stream provider result")?;
    match kind.as_str() {
        "decline" => {
            let reason =
                Reflect::get(&dispatch, &JsValue::from_str("reason")).map_err(|error| {
                    js_context("read Stream decline reason", js_error_message(error))
                })?;
            Ok(StreamDispatch::Decline {
                reason: decline_reason_from_js(reason)?,
            })
        }
        "accept" => accepted_stream_dispatch(dispatch),
        _ => Err(js_error(format!(
            "Stream provider result kind must be \"accept\" or \"decline\", got {kind:?}"
        ))),
    }
}

fn accepted_stream_dispatch(dispatch: JsValue) -> Result<StreamDispatch, JsValue> {
    let payload_kind = StreamPayloadKind::parse(&string_property(
        &dispatch,
        "payloadKind",
        "read Stream provider payload kind",
    )?)?;
    let manifest_value = Reflect::get(&dispatch, &JsValue::from_str("manifest"))
        .map_err(|error| js_context("read Stream provider manifest", js_error_message(error)))?;
    let manifest = stream_manifest_from_js(manifest_value)?;
    payload_kind.validate_manifest(&manifest)?;
    let source = Reflect::get(&dispatch, &JsValue::from_str("source"))
        .map_err(|error| js_context("read Stream provider source", js_error_message(error)))?;
    let source = async_iterator_from_js(source)?;

    Ok(match payload_kind {
        StreamPayloadKind::Camera => StreamDispatch::AcceptCamera {
            manifest,
            source: typed_source_stream(source),
        },
        StreamPayloadKind::PointCloud => StreamDispatch::AcceptPointCloud {
            manifest,
            source: typed_source_stream(source),
        },
        StreamPayloadKind::JointEncoders => StreamDispatch::AcceptJointEncoders {
            manifest,
            source: typed_source_stream(source),
        },
        StreamPayloadKind::Audio => StreamDispatch::AcceptAudio {
            manifest,
            source: typed_source_stream(source),
        },
        StreamPayloadKind::Scalar => StreamDispatch::AcceptScalar {
            manifest,
            source: typed_source_stream(source),
        },
        StreamPayloadKind::Pose => StreamDispatch::AcceptPose {
            manifest,
            source: typed_source_stream(source),
        },
        StreamPayloadKind::Detection => StreamDispatch::AcceptDetection {
            manifest,
            source: typed_source_stream(source),
        },
        StreamPayloadKind::Map => StreamDispatch::AcceptMap {
            manifest,
            source: typed_source_stream(source),
        },
    })
}

fn async_iterator_from_js(source: JsValue) -> Result<AsyncIterator<JsValue>, JsValue> {
    if source.is_instance_of::<AsyncIterator<JsValue>>() {
        return Ok(source.unchecked_into());
    }
    let method = Reflect::get(&source, &Symbol::async_iterator().into())
        .map_err(|error| js_context("read Stream async iterator", js_error_message(error)))?;
    let method = method
        .dyn_into::<Function>()
        .map_err(|_| js_error("Stream provider source must be an AsyncIterable"))?;
    let iterator = method
        .call0(&source)
        .map_err(|error| js_context("create Stream async iterator", js_error_message(error)))?;
    iterator
        .dyn_into::<AsyncIterator<JsValue>>()
        .map_err(|_| js_error("Stream provider source returned an invalid AsyncIterator"))
}

fn typed_source_stream<T>(iterator: AsyncIterator<JsValue>) -> SourceStream<T>
where
    T: Message + Default + 'static,
{
    Box::pin(futures::stream::unfold(iterator, |iterator| async move {
        let next = match iterator.next() {
            Ok(next) => next,
            Err(error) => {
                report_local_stream_error(
                    "Stream provider source next() failed",
                    &js_error_message(error),
                );
                return Some((Err(REMOTE_SOURCE_FAILURE_DETAIL.to_owned()), iterator));
            }
        };
        let next = match JsFuture::from(next).await {
            Ok(next) => next,
            Err(error) => {
                report_local_stream_error(
                    "Stream provider source rejected",
                    &js_error_message(error),
                );
                return Some((Err(REMOTE_SOURCE_FAILURE_DETAIL.to_owned()), iterator));
            }
        };
        let done = match Reflect::get(&next, &JsValue::from_str("done")) {
            Ok(done) => done.as_bool().unwrap_or(false),
            Err(error) => {
                report_local_stream_error(
                    "Stream provider source result is invalid",
                    &js_error_message(error),
                );
                return Some((Err(REMOTE_SOURCE_FAILURE_DETAIL.to_owned()), iterator));
            }
        };
        if done {
            return None;
        }
        let item = Reflect::get(&next, &JsValue::from_str("value"))
            .map_err(|error| {
                report_local_stream_error(
                    "Stream provider source result has no value",
                    &js_error_message(error),
                );
                REMOTE_SOURCE_FAILURE_DETAIL.to_owned()
            })
            .and_then(|value| {
                source_item_from_js::<T>(value).map_err(|error| {
                    report_local_stream_error("Stream provider source item is invalid", &error);
                    REMOTE_SOURCE_FAILURE_DETAIL.to_owned()
                })
            });
        Some((item, iterator))
    }))
}

fn source_item_from_js<T>(value: JsValue) -> Result<StreamItem<T>, String>
where
    T: Message + Default,
{
    let metadata: StreamSourceItemMetadata = serde_wasm_bindgen::from_value(value.clone())
        .map_err(|error| format!("read Stream source item: {error}"))?;
    let payload = Reflect::get(&value, &JsValue::from_str("payload"))
        .map_err(|error| format!("read Stream source payload: {}", js_error_message(error)))?;
    if !payload.is_instance_of::<Uint8Array>() {
        return Err("Stream source payload must be a Uint8Array".into());
    }
    let payload = Uint8Array::new(&payload).to_vec();
    let payload = T::decode(payload.as_slice())
        .map_err(|error| format!("decode Stream source payload: {error}"))?;
    Ok(StreamItem {
        timestamp_ns: metadata.timestamp_ns,
        payload,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamSourceItemMetadata {
    timestamp_ns: i64,
}

fn string_property(value: &JsValue, name: &str, context: &'static str) -> Result<String, JsValue> {
    Reflect::get(value, &JsValue::from_str(name))
        .map_err(|error| js_context(context, js_error_message(error)))?
        .as_string()
        .ok_or_else(|| js_context(context, format!("{name} must be a string")))
}

fn js_error_message(error: JsValue) -> String {
    javascript_error_reason(&error)
}

/// One accepted, typed Stream v2 subscription.
#[wasm_bindgen]
pub struct AukiStreamSubscription {
    payload_kind: StreamPayloadKind,
    manifest: StreamManifestRecord,
    state: Rc<StreamSubscriptionState>,
    closing: CloseBarrier,
}

impl AukiStreamSubscription {
    fn new(payload_kind: StreamPayloadKind, manifest: StreamManifest, entries: AnyEntries) -> Self {
        Self {
            payload_kind,
            manifest: StreamManifestRecord::from(manifest),
            state: Rc::new(StreamSubscriptionState::new(entries)),
            closing: CloseBarrier::default(),
        }
    }
}

#[wasm_bindgen]
impl AukiStreamSubscription {
    /// Exact protobuf payload family validated against the accepted manifest.
    #[wasm_bindgen(
        getter,
        js_name = payloadKind,
        unchecked_return_type = "AukiStreamPayloadKind"
    )]
    pub fn payload_kind(&self) -> String {
        self.payload_kind.as_str().to_owned()
    }

    /// Immutable producer manifest accepted during the Stream handshake.
    #[wasm_bindgen(getter, unchecked_return_type = "AukiStreamManifest")]
    pub fn manifest(&self) -> Result<JsValue, JsValue> {
        to_js_value("convert Stream manifest", &self.manifest)
    }

    /// Pull one entry, one explicit terminal reason, or `undefined` after cancellation.
    ///
    /// Only one pending call is permitted. The payload is a normalized protobuf
    /// encoding of the concrete payload kind after Rust has decoded it.
    #[wasm_bindgen(unchecked_return_type = "Promise<AukiStreamNext | undefined>")]
    pub fn next(&self) -> Result<Promise, JsValue> {
        match self.state.begin_next()? {
            Some(entries) => {
                let state = Rc::clone(&self.state);
                Ok(future_to_promise(async move {
                    subscription_next(state, entries).await
                }))
            }
            None => Ok(Promise::resolve(&JsValue::UNDEFINED)),
        }
    }

    /// Idempotently cancel this subscription and await local route release.
    ///
    /// Resolution confirms that the Rust subscription handle was dropped; it is
    /// not a remote close acknowledgement.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn cancel(&self) -> Promise {
        self.closing.get_or_start(|| {
            self.state.cancel();
            let cleaned = self.state.cleaned_receiver.clone();
            future_to_promise(async move {
                let _ = cleaned.recv().await;
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

impl Drop for AukiStreamSubscription {
    fn drop(&mut self) {
        self.state.cancel();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamPayloadKind {
    Camera,
    PointCloud,
    JointEncoders,
    Audio,
    Scalar,
    Pose,
    Detection,
    Map,
}

impl StreamPayloadKind {
    fn parse(value: &str) -> Result<Self, JsValue> {
        match value {
            "camera" => Ok(Self::Camera),
            "point_cloud" => Ok(Self::PointCloud),
            "joint_encoders" => Ok(Self::JointEncoders),
            "audio" => Ok(Self::Audio),
            "scalar" => Ok(Self::Scalar),
            "pose" => Ok(Self::Pose),
            "detection" => Ok(Self::Detection),
            "map" => Ok(Self::Map),
            _ => Err(js_context(
                "parse Stream payload kind",
                format!("expected {PAYLOAD_KIND_NAMES}, got {value:?}"),
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::PointCloud => "point_cloud",
            Self::JointEncoders => "joint_encoders",
            Self::Audio => "audio",
            Self::Scalar => "scalar",
            Self::Pose => "pose",
            Self::Detection => "detection",
            Self::Map => "map",
        }
    }

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

    fn validate_manifest(self, manifest: &StreamManifest) -> Result<(), JsValue> {
        let expected = self.manifest_payload();
        if manifest.payload == expected {
            return Ok(());
        }
        Err(js_context(
            "validate Stream payload kind",
            format!(
                "payloadKind {:?} requires manifest payload {expected:?}, got {:?}",
                self.as_str(),
                manifest.payload
            ),
        ))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StreamReadFromRecord {
    #[default]
    Latest,
    FromStart,
    FromTimestamp {
        #[serde(rename = "timestampNs")]
        timestamp_ns: i64,
    },
}

impl From<ReadFrom> for StreamReadFromRecord {
    fn from(value: ReadFrom) -> Self {
        match value {
            ReadFrom::Latest => Self::Latest,
            ReadFrom::FromStart => Self::FromStart,
            ReadFrom::FromTimestamp(timestamp_ns) => Self::FromTimestamp { timestamp_ns },
        }
    }
}

impl From<StreamReadFromRecord> for ReadFrom {
    fn from(value: StreamReadFromRecord) -> Self {
        match value {
            StreamReadFromRecord::Latest => Self::Latest,
            StreamReadFromRecord::FromStart => Self::FromStart,
            StreamReadFromRecord::FromTimestamp { timestamp_ns } => {
                Self::FromTimestamp(timestamp_ns)
            }
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StreamRequestRecord {
    source_peer_id: String,
    resource_id: String,
    #[serde(default)]
    from: StreamReadFromRecord,
}

impl From<&StreamRequest> for StreamRequestRecord {
    fn from(request: &StreamRequest) -> Self {
        Self {
            source_peer_id: request.source_peer_id.clone(),
            resource_id: request.resource_id.clone(),
            from: request.from.into(),
        }
    }
}

fn stream_request_from_js(value: JsValue) -> Result<StreamRequest, JsValue> {
    let request: StreamRequestRecord = serde_wasm_bindgen::from_value(value)
        .map_err(|error| js_context("read Stream request", error))?;
    Ok(StreamRequest {
        source_peer_id: request.source_peer_id,
        resource_id: request.resource_id,
        from: request.from.into(),
    })
}

fn stream_request_to_js(request: &StreamRequest) -> Result<JsValue, JsValue> {
    to_js_value(
        "convert Stream provider request",
        &StreamRequestRecord::from(request),
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamManifestRecord {
    sensor_id: String,
    sensor_hash: String,
    clock_peer_id: String,
    clock_id: String,
    clock_hash: String,
    frame_id: String,
    frame_hash: String,
    resource_id: String,
    payload: String,
    from_frame_id: String,
    from_frame_hash: String,
    to_frame_id: String,
    to_frame_hash: String,
    writer_mode: String,
    expected_rate_hz: u32,
    map_peer_id: String,
    map_id: String,
    map_hash: String,
}

impl From<StreamManifestRecord> for StreamManifest {
    fn from(manifest: StreamManifestRecord) -> Self {
        Self {
            sensor_id: manifest.sensor_id,
            sensor_hash: manifest.sensor_hash,
            clock_peer_id: manifest.clock_peer_id,
            clock_id: manifest.clock_id,
            clock_hash: manifest.clock_hash,
            frame_id: manifest.frame_id,
            frame_hash: manifest.frame_hash,
            resource_id: manifest.resource_id,
            payload: manifest.payload,
            from_frame_id: manifest.from_frame_id,
            from_frame_hash: manifest.from_frame_hash,
            to_frame_id: manifest.to_frame_id,
            to_frame_hash: manifest.to_frame_hash,
            writer_mode: manifest.writer_mode,
            expected_rate_hz: manifest.expected_rate_hz,
            map_peer_id: manifest.map_peer_id,
            map_id: manifest.map_id,
            map_hash: manifest.map_hash,
        }
    }
}

impl From<StreamManifest> for StreamManifestRecord {
    fn from(manifest: StreamManifest) -> Self {
        Self {
            sensor_id: manifest.sensor_id,
            sensor_hash: manifest.sensor_hash,
            clock_peer_id: manifest.clock_peer_id,
            clock_id: manifest.clock_id,
            clock_hash: manifest.clock_hash,
            frame_id: manifest.frame_id,
            frame_hash: manifest.frame_hash,
            resource_id: manifest.resource_id,
            payload: manifest.payload,
            from_frame_id: manifest.from_frame_id,
            from_frame_hash: manifest.from_frame_hash,
            to_frame_id: manifest.to_frame_id,
            to_frame_hash: manifest.to_frame_hash,
            writer_mode: manifest.writer_mode,
            expected_rate_hz: manifest.expected_rate_hz,
            map_peer_id: manifest.map_peer_id,
            map_id: manifest.map_id,
            map_hash: manifest.map_hash,
        }
    }
}

fn stream_manifest_from_js(value: JsValue) -> Result<StreamManifest, JsValue> {
    serde_wasm_bindgen::from_value::<StreamManifestRecord>(value)
        .map(StreamManifest::from)
        .map_err(|error| js_context("read Stream provider manifest", error))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StreamDeclineReasonRecord {
    SensorNotFound,
    SensorUnavailable,
    ProducerShuttingDown,
    Other { detail: String },
}

fn decline_reason_from_js(value: JsValue) -> Result<DeclineReason, JsValue> {
    let reason: StreamDeclineReasonRecord = serde_wasm_bindgen::from_value(value)
        .map_err(|error| js_context("read Stream decline reason", error))?;
    Ok(match reason {
        StreamDeclineReasonRecord::SensorNotFound => DeclineReason::sensor_not_found(),
        StreamDeclineReasonRecord::SensorUnavailable => DeclineReason::sensor_unavailable(),
        StreamDeclineReasonRecord::ProducerShuttingDown => DeclineReason::producer_shutting_down(),
        StreamDeclineReasonRecord::Other { detail } => DeclineReason::other(detail),
    })
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
    async fn next_encoded(&mut self) -> Option<Result<EncodedStreamEntry, StreamError>> {
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
) -> Option<Result<EncodedStreamEntry, StreamError>>
where
    T: Message,
{
    entries
        .next()
        .await
        .map(|entry| entry.map(encode_stream_entry))
}

struct EncodedStreamEntry {
    timestamp_ns: i64,
    sequence: u64,
    payload: Vec<u8>,
}

fn encode_stream_entry<T>(entry: StreamEntry<T>) -> EncodedStreamEntry
where
    T: Message,
{
    EncodedStreamEntry {
        timestamp_ns: entry.timestamp_ns,
        sequence: entry.seq,
        payload: entry.payload.encode_to_vec(),
    }
}

struct StreamSubscriptionState {
    entries: RefCell<Option<AnyEntries>>,
    closed: Cell<bool>,
    next_pending: Cell<bool>,
    cancel_sender: async_channel::Sender<()>,
    cancel_receiver: async_channel::Receiver<()>,
    cleaned_sender: async_channel::Sender<()>,
    cleaned_receiver: async_channel::Receiver<()>,
}

impl StreamSubscriptionState {
    fn new(entries: AnyEntries) -> Self {
        let (cancel_sender, cancel_receiver) = async_channel::bounded(1);
        let (cleaned_sender, cleaned_receiver) = async_channel::bounded(1);
        Self {
            entries: RefCell::new(Some(entries)),
            closed: Cell::new(false),
            next_pending: Cell::new(false),
            cancel_sender,
            cancel_receiver,
            cleaned_sender,
            cleaned_receiver,
        }
    }

    fn begin_next(&self) -> Result<Option<AnyEntries>, JsValue> {
        if self.closed.get() {
            return Ok(None);
        }
        if self.next_pending.replace(true) {
            return Err(js_error("Stream subscription already has a pending next()"));
        }
        match self.entries.borrow_mut().take() {
            Some(entries) => Ok(Some(entries)),
            None => {
                self.next_pending.set(false);
                Err(js_error("Stream subscription is unavailable"))
            }
        }
    }

    fn finish_next(&self, entries: AnyEntries, ended: bool) {
        self.next_pending.set(false);
        if self.closed.get() || ended {
            self.closed.set(true);
            self.cancel_sender.close();
            drop(entries);
            self.cleaned_sender.close();
        } else {
            let previous = self.entries.borrow_mut().replace(entries);
            debug_assert!(previous.is_none());
        }
    }

    fn cancel(&self) {
        self.closed.set(true);
        self.cancel_sender.close();
        if !self.next_pending.get() {
            drop(self.entries.borrow_mut().take());
            self.cleaned_sender.close();
        }
    }
}

enum PendingNext {
    Cancelled,
    Item(Option<Result<EncodedStreamEntry, StreamError>>),
}

async fn subscription_next(
    state: Rc<StreamSubscriptionState>,
    mut entries: AnyEntries,
) -> Result<JsValue, JsValue> {
    let outcome = {
        let cancelled = state.cancel_receiver.recv().fuse();
        let received = entries.next_encoded().fuse();
        pin_mut!(cancelled, received);
        futures::select_biased! {
            _ = cancelled => PendingNext::Cancelled,
            entry = received => PendingNext::Item(entry),
        }
    };

    match outcome {
        PendingNext::Cancelled | PendingNext::Item(None) => {
            state.finish_next(entries, true);
            Ok(JsValue::UNDEFINED)
        }
        PendingNext::Item(Some(Ok(entry))) => match stream_entry_to_js(entry) {
            Ok(value) => {
                state.finish_next(entries, false);
                Ok(value)
            }
            Err(error) => {
                state.finish_next(entries, true);
                Err(error)
            }
        },
        PendingNext::Item(Some(Err(StreamError::EndOfStream { reason }))) => {
            state.finish_next(entries, true);
            stream_end_to_js(reason)
        }
        PendingNext::Item(Some(Err(error))) => {
            state.finish_next(entries, true);
            Err(js_context("read Stream entry", error))
        }
    }
}

#[derive(Serialize)]
struct StreamEntryNextRecord {
    kind: &'static str,
    entry: StreamEntryRecord,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamEntryRecord {
    timestamp_ns: i64,
    sequence: u64,
}

fn stream_entry_to_js(entry: EncodedStreamEntry) -> Result<JsValue, JsValue> {
    let value = to_js_value(
        "convert Stream entry",
        &StreamEntryNextRecord {
            kind: "entry",
            entry: StreamEntryRecord {
                timestamp_ns: entry.timestamp_ns,
                sequence: entry.sequence,
            },
        },
    )?;
    let entry_value = Reflect::get(&value, &JsValue::from_str("entry"))
        .map_err(|error| js_context("read converted Stream entry", format!("{error:?}")))?;
    let attached = Reflect::set(
        &entry_value,
        &JsValue::from_str("payload"),
        &Uint8Array::from(entry.payload.as_slice()),
    )
    .map_err(|error| js_context("attach Stream payload", format!("{error:?}")))?;
    if !attached {
        return Err(js_error("attach Stream payload"));
    }
    Ok(value)
}

#[derive(Serialize)]
struct StreamEndNextRecord {
    kind: &'static str,
    reason: StreamEndReasonRecord,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StreamEndReasonRecord {
    SourceEnded,
    ProducerShuttingDown,
    SessionEnded,
    ProducerError { detail: String },
}

fn stream_end_to_js(reason: EndReason) -> Result<JsValue, JsValue> {
    let reason = match reason.kind {
        Some(end_reason::Kind::SourceEnded(_)) => StreamEndReasonRecord::SourceEnded,
        Some(end_reason::Kind::ProducerShuttingDown(_)) => {
            StreamEndReasonRecord::ProducerShuttingDown
        }
        Some(end_reason::Kind::SessionEnded(_)) => StreamEndReasonRecord::SessionEnded,
        Some(end_reason::Kind::ProducerError(error)) => StreamEndReasonRecord::ProducerError {
            detail: error.detail,
        },
        None => return Err(js_error("Stream end reason has no kind")),
    };
    to_js_value(
        "convert Stream end reason",
        &StreamEndNextRecord {
            kind: "end",
            reason,
        },
    )
}

#[wasm_bindgen(typescript_custom_section)]
const STREAM_TYPESCRIPT: &str = r#"
/** Exact protobuf family selected from Catalog/Registry knowledge and checked against the manifest. */
export type AukiStreamPayloadKind =
    | "camera"
    | "point_cloud"
    | "joint_encoders"
    | "audio"
    | "scalar"
    | "pose"
    | "detection"
    | "map";

export type AukiStreamReadFrom =
    | { readonly kind: "latest" }
    | { readonly kind: "from_start" }
    | { readonly kind: "from_timestamp"; readonly timestampNs: bigint };

/** One exact Stream v2 subscription request. */
export interface AukiStreamRequest {
    readonly sourcePeerId: string;
    readonly resourceId: string;
    readonly from?: AukiStreamReadFrom;
}

/** Immutable control metadata accepted from the producer handshake. */
export interface AukiStreamManifest {
    readonly sensorId: string;
    readonly sensorHash: string;
    readonly clockPeerId: string;
    readonly clockId: string;
    readonly clockHash: string;
    readonly frameId: string;
    readonly frameHash: string;
    readonly resourceId: string;
    readonly payload: string;
    readonly fromFrameId: string;
    readonly fromFrameHash: string;
    readonly toFrameId: string;
    readonly toFrameHash: string;
    readonly writerMode: string;
    readonly expectedRateHz: number;
    readonly mapPeerId: string;
    readonly mapId: string;
    readonly mapHash: string;
}

/** One producer item. `payload` is the protobuf encoding for `payloadKind`. */
export interface AukiStreamSourceItem {
    readonly timestampNs: bigint;
    readonly payload: Uint8Array;
}

export type AukiStreamDeclineReason =
    | { readonly kind: "sensor_not_found" }
    | { readonly kind: "sensor_unavailable" }
    | { readonly kind: "producer_shutting_down" }
    | { readonly kind: "other"; readonly detail: string };

/** Synchronous admission decision with a demand-driven asynchronous source. */
export type AukiStreamDispatch =
    | { readonly kind: "decline"; readonly reason: AukiStreamDeclineReason }
    | {
          readonly kind: "accept";
          readonly payloadKind: AukiStreamPayloadKind;
          readonly manifest: AukiStreamManifest;
          readonly source: AsyncIterable<AukiStreamSourceItem>;
      };

export type AukiStreamProvider = (
    requester: AukiAuthenticatedPeer,
    request: AukiStreamRequest,
) => AukiStreamDispatch;

/** One sequence-checked, Rust-decoded-and-re-encoded protobuf entry. */
export interface AukiStreamEntry {
    readonly timestampNs: bigint;
    readonly sequence: bigint;
    readonly payload: Uint8Array;
}

export type AukiStreamEndReason =
    | { readonly kind: "source_ended" }
    | { readonly kind: "producer_shutting_down" }
    | { readonly kind: "session_ended" }
    | { readonly kind: "producer_error"; readonly detail: string };

/** An explicit producer end is a normal result; transport/protocol failures reject. */
export type AukiStreamNext =
    | { readonly kind: "entry"; readonly entry: AukiStreamEntry }
    | { readonly kind: "end"; readonly reason: AukiStreamEndReason };
"#;

#[cfg(test)]
mod tests {
    use futures::stream;
    use js_sys::{Object, Reflect};
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn property(value: &JsValue, name: &str) -> JsValue {
        Reflect::get(value, &JsValue::from_str(name)).unwrap()
    }

    fn peer_id() -> auki_sdk::PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }

    fn authenticated_peer() -> AuthenticatedPeer {
        AuthenticatedPeer {
            peer_id: peer_id(),
            subject: "b03a67cb-45d4-4f60-a8b8-d9687e91d018".parse().unwrap(),
            peer_type: Some("robot".into()),
            domain_ids: vec!["4e990513-b110-467b-84ca-09a42d786f6d".parse().unwrap()],
            scopes: vec!["stream:serve".into()],
            application: None,
            verified_until: "2030-01-01T00:00:00Z".parse().unwrap(),
        }
    }

    fn pending_subscription() -> AukiStreamSubscription {
        AukiStreamSubscription::new(
            StreamPayloadKind::Camera,
            StreamManifest {
                payload: "camera_frame".into(),
                ..Default::default()
            },
            AnyEntries::Camera(Box::pin(stream::pending())),
        )
    }

    #[wasm_bindgen_test]
    fn payload_kind_parser_accepts_only_the_eight_exact_names() {
        let expected = [
            ("camera", StreamPayloadKind::Camera, "camera_frame"),
            (
                "point_cloud",
                StreamPayloadKind::PointCloud,
                "point_cloud_frame",
            ),
            (
                "joint_encoders",
                StreamPayloadKind::JointEncoders,
                "joint_encoders_frame",
            ),
            ("audio", StreamPayloadKind::Audio, "audio_frame"),
            ("scalar", StreamPayloadKind::Scalar, "scalar"),
            ("pose", StreamPayloadKind::Pose, "spatial_transform"),
            ("detection", StreamPayloadKind::Detection, "detection"),
            ("map", StreamPayloadKind::Map, "map_update"),
        ];
        for (name, kind, manifest_payload) in expected {
            assert_eq!(StreamPayloadKind::parse(name).unwrap(), kind);
            assert_eq!(kind.as_str(), name);
            assert_eq!(kind.manifest_payload(), manifest_payload);
            assert!(
                kind.validate_manifest(&StreamManifest {
                    payload: manifest_payload.into(),
                    ..Default::default()
                })
                .is_ok()
            );
        }

        let error = StreamPayloadKind::parse("pointcloud").unwrap_err();
        assert!(
            js_sys::Error::from(error)
                .message()
                .as_string()
                .unwrap()
                .contains("point_cloud")
        );
        assert!(
            StreamPayloadKind::Camera
                .validate_manifest(&StreamManifest {
                    payload: "audio_frame".into(),
                    ..Default::default()
                })
                .is_err()
        );
        assert!(
            StreamPayloadKind::Camera
                .validate_manifest(&StreamManifest::default())
                .is_err()
        );
    }

    #[wasm_bindgen_test]
    fn request_boundary_reads_camel_case_bigint_and_defaults_to_latest() {
        let latest = Object::new();
        Reflect::set(
            &latest,
            &JsValue::from_str("sourcePeerId"),
            &JsValue::from_str("source"),
        )
        .unwrap();
        Reflect::set(
            &latest,
            &JsValue::from_str("resourceId"),
            &JsValue::from_str("camera/front"),
        )
        .unwrap();
        let parsed = stream_request_from_js(latest.into()).unwrap();
        assert_eq!(parsed.from, ReadFrom::Latest);

        let from = Object::new();
        Reflect::set(
            &from,
            &JsValue::from_str("kind"),
            &JsValue::from_str("from_timestamp"),
        )
        .unwrap();
        let timestamp = to_js_value("convert test timestamp", &i64::MIN).unwrap();
        assert!(timestamp.is_bigint());
        Reflect::set(&from, &JsValue::from_str("timestampNs"), &timestamp).unwrap();

        let request = Object::new();
        Reflect::set(
            &request,
            &JsValue::from_str("sourcePeerId"),
            &JsValue::from_str("source"),
        )
        .unwrap();
        Reflect::set(
            &request,
            &JsValue::from_str("resourceId"),
            &JsValue::from_str("camera/front"),
        )
        .unwrap();
        Reflect::set(&request, &JsValue::from_str("from"), &from).unwrap();
        let parsed = stream_request_from_js(request.into()).unwrap();
        assert_eq!(parsed.from, ReadFrom::FromTimestamp(i64::MIN));
    }

    #[wasm_bindgen_test]
    fn provider_request_and_manifest_round_trip_without_new_wire_models() {
        let request = StreamRequest {
            source_peer_id: "source".into(),
            resource_id: "camera/front".into(),
            from: ReadFrom::FromTimestamp(i64::MIN),
        };
        let request_value = stream_request_to_js(&request).unwrap();
        assert_eq!(stream_request_from_js(request_value).unwrap(), request);

        let manifest = StreamManifest {
            resource_id: "camera/front".into(),
            payload: "camera_frame".into(),
            expected_rate_hz: 30,
            ..Default::default()
        };
        let manifest_value = to_js_value(
            "convert provider test manifest",
            &StreamManifestRecord::from(manifest.clone()),
        )
        .unwrap();
        assert_eq!(stream_manifest_from_js(manifest_value).unwrap(), manifest);
    }

    #[wasm_bindgen_test]
    fn provider_declines_are_a_closed_validated_set() {
        let reason = Object::new();
        Reflect::set(
            &reason,
            &JsValue::from_str("kind"),
            &JsValue::from_str("sensor_unavailable"),
        )
        .unwrap();
        assert!(matches!(
            decline_reason_from_js(reason.into()).unwrap().kind,
            Some(decline_reason::Kind::SensorUnavailable(_))
        ));

        let invalid = Object::new();
        Reflect::set(
            &invalid,
            &JsValue::from_str("kind"),
            &JsValue::from_str("retry_later"),
        )
        .unwrap();
        assert!(decline_reason_from_js(invalid.into()).is_err());

        let explicit = Object::new();
        Reflect::set(
            &explicit,
            &JsValue::from_str("kind"),
            &JsValue::from_str("other"),
        )
        .unwrap();
        Reflect::set(
            &explicit,
            &JsValue::from_str("detail"),
            &JsValue::from_str("camera is warming up"),
        )
        .unwrap();
        let explicit = decline_reason_from_js(explicit.into()).unwrap();
        let Some(decline_reason::Kind::Other(explicit)) = explicit.kind else {
            panic!("expected explicit Other decline")
        };
        assert_eq!(explicit.detail, "camera is warming up");
    }

    #[wasm_bindgen_test]
    fn thrown_provider_errors_are_not_exposed_in_decline_details() {
        const SECRET: &str = "provider-secret-do-not-send";
        let provider = Function::new_no_args("throw new Error('provider-secret-do-not-send')");
        let dispatch = BrowserStreamProvider { provider }.dispatch(
            &authenticated_peer(),
            StreamRequest {
                source_peer_id: peer_id().to_string(),
                resource_id: "camera/front".into(),
                from: ReadFrom::Latest,
            },
        );
        let StreamDispatch::Decline { reason } = dispatch else {
            panic!("expected provider failure to decline")
        };
        let Some(decline_reason::Kind::Other(other)) = reason.kind else {
            panic!("expected generic Other decline")
        };
        assert_eq!(other.detail, REMOTE_PROVIDER_FAILURE_DETAIL);
        assert!(!other.detail.contains(SECRET));
    }

    #[wasm_bindgen_test(async)]
    async fn provider_async_iterable_is_demand_driven_and_typed() {
        let factory = Function::new_no_args(
            "return (async function* () { yield { timestampNs: 7n, payload: new Uint8Array([]) }; })();",
        );
        let source = factory.call0(&JsValue::UNDEFINED).unwrap();
        let iterator = async_iterator_from_js(source).unwrap();
        let mut source = typed_source_stream::<CameraFrame>(iterator);
        let item = source.next().await.unwrap().unwrap();
        assert_eq!(item.timestamp_ns, 7);
        assert_eq!(item.payload, CameraFrame::default());
        assert!(source.next().await.is_none());
    }

    #[wasm_bindgen_test(async)]
    async fn source_exceptions_and_decode_errors_are_not_exposed_remotely() {
        const SECRET: &str = "source-secret-do-not-send";
        let factory = Function::new_no_args(
            "return (async function* () { throw new Error('source-secret-do-not-send'); })();",
        );
        let source = factory.call0(&JsValue::UNDEFINED).unwrap();
        let iterator = async_iterator_from_js(source).unwrap();
        let mut source = typed_source_stream::<CameraFrame>(iterator);
        let error = source.next().await.unwrap().unwrap_err();
        assert_eq!(error, REMOTE_SOURCE_FAILURE_DETAIL);
        assert!(!error.contains(SECRET));

        let factory = Function::new_no_args(
            "return (async function* () { yield { timestampNs: 7n, payload: new Uint8Array([255]) }; })();",
        );
        let source = factory.call0(&JsValue::UNDEFINED).unwrap();
        let iterator = async_iterator_from_js(source).unwrap();
        let mut source = typed_source_stream::<CameraFrame>(iterator);
        let error = source.next().await.unwrap().unwrap_err();
        assert_eq!(error, REMOTE_SOURCE_FAILURE_DETAIL);
    }

    #[wasm_bindgen_test]
    fn manifest_and_entries_are_plain_records_with_bigints_and_typed_bytes() {
        let manifest = StreamManifestRecord::from(StreamManifest {
            resource_id: "camera/front".into(),
            clock_peer_id: "clock-owner".into(),
            expected_rate_hz: 30,
            ..Default::default()
        });
        let manifest = to_js_value("convert test manifest", &manifest).unwrap();
        assert_eq!(
            property(&manifest, "resourceId").as_string().as_deref(),
            Some("camera/front")
        );
        assert_eq!(
            property(&manifest, "clockPeerId").as_string().as_deref(),
            Some("clock-owner")
        );
        assert_eq!(property(&manifest, "expectedRateHz").as_f64(), Some(30.0));

        let entry = stream_entry_to_js(EncodedStreamEntry {
            timestamp_ns: i64::MIN,
            sequence: u64::MAX,
            payload: vec![0, 1, 127, 255],
        })
        .unwrap();
        assert_eq!(
            property(&entry, "kind").as_string().as_deref(),
            Some("entry")
        );
        let entry = property(&entry, "entry");
        assert!(property(&entry, "timestampNs").is_bigint());
        assert!(property(&entry, "sequence").is_bigint());
        let payload = property(&entry, "payload");
        assert!(payload.is_instance_of::<Uint8Array>());
        assert_eq!(Uint8Array::new(&payload).to_vec(), vec![0, 1, 127, 255]);
    }

    #[wasm_bindgen_test]
    fn all_eight_payload_types_are_reencoded_after_typed_decode() {
        fn assert_encoded<T>(payload: T)
        where
            T: Message + Default,
        {
            let expected = payload.encode_to_vec();
            let validated = T::decode(expected.as_slice()).unwrap();
            let encoded = encode_stream_entry(StreamEntry {
                timestamp_ns: 1,
                seq: 2,
                payload: validated,
            });
            assert_eq!(encoded.payload, expected);
        }

        assert_encoded(CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![1],
        });
        assert_encoded(point_cloud::Data { data: vec![2] });
        assert_encoded(joint_encoders::Data {
            angles_rad: vec![3.0],
        });
        assert_encoded(audio::Data { data: vec![4] });
        assert_encoded(scalar::Data { value: 5.0 });
        assert_encoded(pose::SpatialTransform::default());
        assert_encoded(DetectionFrame {
            data: vec![6],
            sensor_hash: "sensor".into(),
            r#type: "example".into(),
        });
        assert_encoded(MapUpdate::default());
    }

    #[wasm_bindgen_test]
    fn explicit_end_reasons_are_normal_terminal_records() {
        let cases = [
            (EndReason::source_ended(), "source_ended", None),
            (
                EndReason::producer_shutting_down(),
                "producer_shutting_down",
                None,
            ),
            (EndReason::session_ended(), "session_ended", None),
            (
                EndReason::producer_error("camera disconnected"),
                "producer_error",
                Some("camera disconnected"),
            ),
        ];
        for (reason, expected_kind, expected_detail) in cases {
            let value = stream_end_to_js(reason).unwrap();
            assert_eq!(property(&value, "kind").as_string().as_deref(), Some("end"));
            let reason = property(&value, "reason");
            assert_eq!(
                property(&reason, "kind").as_string().as_deref(),
                Some(expected_kind)
            );
            assert_eq!(
                property(&reason, "detail").as_string().as_deref(),
                expected_detail
            );
        }
        assert!(stream_end_to_js(EndReason { kind: None }).is_err());
    }

    #[wasm_bindgen_test(async)]
    async fn one_pending_next_is_enforced_and_cancel_wakes_it() {
        let subscription = pending_subscription();
        let pending = subscription.next().unwrap();
        assert!(subscription.next().is_err());

        let cancelled = subscription.cancel();
        let cancelled_again = subscription.cancel();
        assert!(Object::is(cancelled.as_ref(), cancelled_again.as_ref()));
        let result = JsFuture::from(pending).await.unwrap();
        assert!(result.is_undefined());
        assert!(JsFuture::from(cancelled).await.unwrap().is_undefined());
        assert!(
            JsFuture::from(subscription.next().unwrap())
                .await
                .unwrap()
                .is_undefined()
        );
    }

    #[wasm_bindgen_test(async)]
    async fn dropping_subscription_cancels_a_pending_next() {
        let subscription = pending_subscription();
        let pending = subscription.next().unwrap();
        drop(subscription);
        assert!(JsFuture::from(pending).await.unwrap().is_undefined());
    }
}

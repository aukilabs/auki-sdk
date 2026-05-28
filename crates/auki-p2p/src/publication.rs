//! Generic local offer publication helpers.

use auki_protocol::v1::{
    base64url,
    message::{SPATIAL_MESSAGE_TYPE, SpatialMessage, SpatialMessageError},
    offer::{
        Offer, OfferAccessMode, OfferError, OfferStatus, PayloadDescriptor, RegistryReference,
    },
    subscribe::SubscribeEndReason,
};
use futures::{Stream, StreamExt as _, stream};
use libp2p::PeerId;
use serde_json::{Map, Value};
use std::{
    fmt,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

/// Default per-subscription source queue capacity.
pub const DEFAULT_SUBSCRIPTION_QUEUE_CAPACITY: usize = 1024;

/// One producer frame exposed by a published byte source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedByteFrame {
    /// Raw payload bytes.
    pub bytes: Vec<u8>,
    /// Optional producer-assigned frame sequence.
    pub sequence: Option<u64>,
    /// Optional producer generation timestamp.
    pub generated_at: Option<String>,
}

/// Boxed byte stream opened for one Subscribe request.
pub type PublishedByteSource = Pin<Box<dyn Stream<Item = PublishedByteFrame> + Send>>;

/// Shared latest-frame source for live producers that should fan out one stream
/// of truth to Get and all Subscribe consumers.
#[derive(Debug, Clone)]
pub struct LatestPublishedByteSource {
    state: Arc<Mutex<LatestPublishedByteSourceState>>,
    signal_tx: watch::Sender<u64>,
}

#[derive(Debug, Default)]
struct LatestPublishedByteSourceState {
    latest: Option<PublishedByteFrame>,
    frame_version: u64,
    signal_version: u64,
    closed: bool,
}

enum LatestSourceRead {
    Frame(PublishedByteFrame, u64),
    Pending,
    Closed,
}

/// Factory that opens a byte stream for one Subscribe request.
pub trait PublishedByteSourceFactory: Send {
    /// Open a new byte stream for one accepted Subscribe request.
    fn open(&mut self) -> PublishedByteSource;
}

impl<F, S> PublishedByteSourceFactory for F
where
    F: FnMut() -> S + Send,
    S: Stream + Send + 'static,
    S::Item: Into<PublishedByteFrame> + Send + 'static,
{
    fn open(&mut self) -> PublishedByteSource {
        Box::pin(self().map(Into::into))
    }
}

impl PublishedByteSourceFactory for LatestPublishedByteSource {
    fn open(&mut self) -> PublishedByteSource {
        self.stream()
    }
}

impl PublishedByteFrame {
    /// Create a producer frame from raw payload bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            sequence: None,
            generated_at: None,
        }
    }

    /// Set the producer sequence for this frame.
    pub fn with_sequence(mut self, sequence: u64) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Set the producer generation timestamp for this frame.
    pub fn with_generated_at(mut self, generated_at: impl Into<String>) -> Self {
        self.generated_at = Some(generated_at.into());
        self
    }
}

impl From<Vec<u8>> for PublishedByteFrame {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl Default for LatestPublishedByteSource {
    fn default() -> Self {
        Self::new()
    }
}

impl LatestPublishedByteSource {
    /// Create an empty shared latest-frame source.
    pub fn new() -> Self {
        let (signal_tx, _) = watch::channel(0);
        Self {
            state: Arc::new(Mutex::new(LatestPublishedByteSourceState::default())),
            signal_tx,
        }
    }

    /// Publish a new latest frame and notify active Subscribe streams.
    ///
    /// Returns `false` when the source has already been closed.
    pub fn publish(&self, frame: impl Into<PublishedByteFrame>) -> bool {
        let signal = {
            let mut state = self.state.lock().expect("latest source mutex poisoned");
            if state.closed {
                return false;
            }
            state.latest = Some(frame.into());
            state.frame_version = state.frame_version.saturating_add(1);
            state.signal_version = state.signal_version.saturating_add(1);
            state.signal_version
        };
        let _ = self.signal_tx.send(signal);
        true
    }

    /// Return the current latest frame, if any.
    pub fn latest(&self) -> Option<PublishedByteFrame> {
        self.state
            .lock()
            .expect("latest source mutex poisoned")
            .latest
            .clone()
    }

    /// Return the current latest payload bytes, if any.
    pub fn latest_bytes(&self) -> Option<Vec<u8>> {
        self.latest().map(|frame| frame.bytes)
    }

    /// Close the source and let active/future streams complete after their
    /// latest pending frame has been delivered.
    pub fn close(&self) {
        let signal = {
            let mut state = self.state.lock().expect("latest source mutex poisoned");
            if state.closed {
                return;
            }
            state.closed = true;
            state.signal_version = state.signal_version.saturating_add(1);
            state.signal_version
        };
        let _ = self.signal_tx.send(signal);
    }

    /// Whether the source has been closed.
    pub fn is_closed(&self) -> bool {
        self.state
            .lock()
            .expect("latest source mutex poisoned")
            .closed
    }

    /// Open a stream that first yields the current latest frame, when present,
    /// and then yields subsequent source updates.
    pub fn stream(&self) -> PublishedByteSource {
        let state = Arc::clone(&self.state);
        let signal_rx = self.signal_tx.subscribe();

        Box::pin(stream::unfold(
            (state, signal_rx, 0_u64),
            |(state, mut signal_rx, mut last_frame_version)| async move {
                loop {
                    match read_latest_source_state(&state, last_frame_version) {
                        LatestSourceRead::Frame(frame, frame_version) => {
                            last_frame_version = frame_version;
                            return Some((frame, (state, signal_rx, last_frame_version)));
                        }
                        LatestSourceRead::Closed => return None,
                        LatestSourceRead::Pending => {}
                    }

                    if signal_rx.changed().await.is_err() {
                        return None;
                    }
                }
            },
        ))
    }
}

fn read_latest_source_state(
    state: &Arc<Mutex<LatestPublishedByteSourceState>>,
    last_frame_version: u64,
) -> LatestSourceRead {
    let state = state.lock().expect("latest source mutex poisoned");
    if state.frame_version > last_frame_version {
        let Some(frame) = state.latest.clone() else {
            return LatestSourceRead::Pending;
        };
        return LatestSourceRead::Frame(frame, state.frame_version);
    }
    if state.closed {
        return LatestSourceRead::Closed;
    }
    LatestSourceRead::Pending
}

/// Backpressure policy for runtime-managed published Subscribe streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AukiSubscriptionBackpressurePolicy {
    /// Keep only the newest queued source frame and drop older queued frames.
    LatestOnly,
    /// Backpressure the source task once this subscription queue is full.
    Bounded {
        /// Maximum queued source frames or control events.
        capacity: usize,
    },
    /// Close the subscription with a backpressure error once the queue is full.
    CloseOnFull {
        /// Maximum queued source frames or control events.
        capacity: usize,
    },
}

impl Default for AukiSubscriptionBackpressurePolicy {
    fn default() -> Self {
        Self::Bounded {
            capacity: DEFAULT_SUBSCRIPTION_QUEUE_CAPACITY,
        }
    }
}

/// High-level local offer publication input.
pub struct PublishOfferInput {
    /// Offer domain id.
    pub domain_id: String,
    /// Producer-scoped offer id.
    pub offer_id: String,
    /// Open offer-kind string.
    pub kind: String,
    /// Payload descriptor advertised in the offer and Subscribe accept.
    pub payload: PayloadDescriptor,
    /// Optional human-readable display name.
    pub display_name: Option<String>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
    /// Registry references needed to interpret the offer.
    pub registry_refs: Vec<RegistryReference>,
    /// Access modes advertised by the offer.
    pub access_modes: Vec<OfferAccessMode>,
    /// Per-subscription backpressure policy used by the SDK serve runtime.
    pub backpressure_policy: AukiSubscriptionBackpressurePolicy,
    source_factory: Box<dyn PublishedByteSourceFactory>,
}

/// Handle identifying one published local offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedOfferHandle {
    domain_id: String,
    offer_id: String,
}

/// Result of serving one generic published-offer Subscribe stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedPublishedSubscription {
    /// Requesting peer id.
    pub peer_id: PeerId,
    /// Requested domain id, when the request parsed.
    pub domain_id: Option<String>,
    /// Requested offer id, when the request parsed.
    pub offer_id: Option<String>,
    /// Whether a Subscribe accept was served.
    pub accepted: bool,
    /// Stable failure code when a reject was served.
    pub failure_code: Option<String>,
    /// Number of spatial messages sent before the stream ended.
    pub messages_sent: u64,
    /// End reason written for accepted published streams.
    pub end_reason: Option<SubscribeEndReason>,
}

/// Failure while creating a published offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOfferError {
    /// The RFC offer object failed validation.
    Offer(OfferError),
}

/// Failure while creating a published spatial message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationMessageError {
    /// The publication sequence would exceed `u64`.
    SequenceOverflow,
    /// The RFC spatial-message object failed validation.
    SpatialMessage(SpatialMessageError),
}

pub(crate) struct LocalOfferPublication {
    offer: Offer,
    source_factory: Box<dyn PublishedByteSourceFactory>,
    backpressure_policy: AukiSubscriptionBackpressurePolicy,
    next_sequence: u64,
}

impl PublishOfferInput {
    /// Create high-level publication input from a byte-source factory.
    pub fn new<F>(
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        kind: impl Into<String>,
        payload: PayloadDescriptor,
        source_factory: F,
    ) -> Self
    where
        F: PublishedByteSourceFactory + 'static,
    {
        Self {
            domain_id: domain_id.into(),
            offer_id: offer_id.into(),
            kind: kind.into(),
            payload,
            display_name: None,
            metadata: None,
            registry_refs: Vec::new(),
            access_modes: vec![OfferAccessMode::Subscribe],
            backpressure_policy: AukiSubscriptionBackpressurePolicy::default(),
            source_factory: Box::new(source_factory),
        }
    }

    /// Set an optional human-readable display name.
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    /// Set optional non-authoritative metadata.
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set registry references needed to interpret this offer.
    pub fn with_registry_refs(mut self, registry_refs: Vec<RegistryReference>) -> Self {
        self.registry_refs = registry_refs;
        self
    }

    /// Set advertised access modes for this offer.
    pub fn with_access_modes(mut self, access_modes: Vec<OfferAccessMode>) -> Self {
        self.access_modes = access_modes;
        self
    }

    /// Set the runtime backpressure policy for Subscribe streams.
    pub fn with_backpressure_policy(mut self, policy: AukiSubscriptionBackpressurePolicy) -> Self {
        self.backpressure_policy = policy;
        self
    }

    pub(crate) fn key(&self) -> (String, String) {
        (self.domain_id.clone(), self.offer_id.clone())
    }

    pub(crate) fn into_publication(self) -> Result<LocalOfferPublication, PublishOfferError> {
        let offer = create_published_offer(&self)?;
        Ok(LocalOfferPublication {
            offer,
            source_factory: self.source_factory,
            backpressure_policy: self.backpressure_policy,
            next_sequence: 0,
        })
    }
}

impl fmt::Debug for PublishOfferInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublishOfferInput")
            .field("domain_id", &self.domain_id)
            .field("offer_id", &self.offer_id)
            .field("kind", &self.kind)
            .field("payload", &self.payload)
            .field("display_name", &self.display_name)
            .field("metadata", &self.metadata)
            .field("registry_refs", &self.registry_refs)
            .field("access_modes", &self.access_modes)
            .field("backpressure_policy", &self.backpressure_policy)
            .finish_non_exhaustive()
    }
}

impl PublishedOfferHandle {
    pub(crate) fn new(domain_id: impl Into<String>, offer_id: impl Into<String>) -> Self {
        Self {
            domain_id: domain_id.into(),
            offer_id: offer_id.into(),
        }
    }

    /// Published offer domain id.
    pub fn domain_id(&self) -> &str {
        &self.domain_id
    }

    /// Published offer id.
    pub fn offer_id(&self) -> &str {
        &self.offer_id
    }

    pub(crate) fn key(&self) -> (String, String) {
        (self.domain_id.clone(), self.offer_id.clone())
    }
}

impl LocalOfferPublication {
    pub(crate) fn offer(&self) -> &Offer {
        &self.offer
    }

    pub(crate) fn open_source(&mut self) -> PublishedByteSource {
        self.source_factory.open()
    }

    pub(crate) fn backpressure_policy(&self) -> AukiSubscriptionBackpressurePolicy {
        self.backpressure_policy
    }

    pub(crate) fn next_message(
        &mut self,
        frame: PublishedByteFrame,
        fallback_generated_at: Option<&str>,
    ) -> Result<SpatialMessage, PublicationMessageError> {
        let sequence = match frame.sequence {
            Some(sequence) => {
                if let Some(next_sequence) = sequence.checked_add(1) {
                    self.next_sequence = self.next_sequence.max(next_sequence);
                }
                sequence
            }
            None => {
                let sequence = self.next_sequence;
                self.next_sequence = sequence
                    .checked_add(1)
                    .ok_or(PublicationMessageError::SequenceOverflow)?;
                sequence
            }
        };
        let generated_at = frame.generated_at.as_deref().or(fallback_generated_at);
        let message = create_publication_spatial_message(
            &self.offer,
            sequence,
            frame.bytes.as_slice(),
            generated_at,
        )?;
        Ok(message)
    }
}

impl ServedPublishedSubscription {
    pub(crate) fn rejected(
        peer_id: PeerId,
        domain_id: Option<String>,
        offer_id: Option<String>,
        failure_code: Option<String>,
    ) -> Self {
        Self {
            peer_id,
            domain_id,
            offer_id,
            accepted: false,
            failure_code,
            messages_sent: 0,
            end_reason: None,
        }
    }

    pub(crate) fn accepted(
        peer_id: PeerId,
        domain_id: String,
        offer_id: String,
        messages_sent: u64,
        end_reason: SubscribeEndReason,
    ) -> Self {
        Self {
            peer_id,
            domain_id: Some(domain_id),
            offer_id: Some(offer_id),
            accepted: true,
            failure_code: None,
            messages_sent,
            end_reason: Some(end_reason),
        }
    }
}

impl From<OfferError> for PublishOfferError {
    fn from(error: OfferError) -> Self {
        Self::Offer(error)
    }
}

impl From<SpatialMessageError> for PublicationMessageError {
    fn from(error: SpatialMessageError) -> Self {
        Self::SpatialMessage(error)
    }
}

impl fmt::Display for PublishOfferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offer(error) => write!(f, "published offer failed validation: {error}"),
        }
    }
}

impl std::error::Error for PublishOfferError {}

impl fmt::Display for PublicationMessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceOverflow => write!(f, "published offer sequence overflow"),
            Self::SpatialMessage(error) => {
                write!(f, "published spatial message failed validation: {error}")
            }
        }
    }
}

impl std::error::Error for PublicationMessageError {}

fn create_published_offer(input: &PublishOfferInput) -> Result<Offer, PublishOfferError> {
    let mut object = Map::new();
    object.insert("offer_id".to_owned(), Value::String(input.offer_id.clone()));
    object.insert(
        "domain_id".to_owned(),
        Value::String(input.domain_id.clone()),
    );
    object.insert("kind".to_owned(), Value::String(input.kind.clone()));
    object.insert(
        "status".to_owned(),
        Value::String(OfferStatus::Available.as_str().to_owned()),
    );
    object.insert(
        "access_modes".to_owned(),
        Value::Array(
            input
                .access_modes
                .iter()
                .map(|mode| Value::String(mode.as_str().to_owned()))
                .collect(),
        ),
    );
    object.insert("payload".to_owned(), input.payload.value().clone());
    object.insert(
        "registry_refs".to_owned(),
        Value::Array(
            input
                .registry_refs
                .iter()
                .map(|reference| reference.value().clone())
                .collect(),
        ),
    );
    if let Some(display_name) = &input.display_name {
        object.insert(
            "display_name".to_owned(),
            Value::String(display_name.clone()),
        );
    }
    if let Some(metadata) = &input.metadata {
        object.insert("metadata".to_owned(), metadata.clone());
    }

    Offer::from_value(Value::Object(object)).map_err(PublishOfferError::from)
}

fn create_publication_spatial_message(
    offer: &Offer,
    sequence: u64,
    chunk: &[u8],
    generated_at: Option<&str>,
) -> Result<SpatialMessage, PublicationMessageError> {
    let mut payload = offer
        .payload
        .value()
        .as_object()
        .expect("payload descriptor is validated as a JSON object")
        .clone();
    payload.insert("bytes".to_owned(), Value::String(base64url::encode(chunk)));

    let mut object = Map::new();
    object.insert(
        "type".to_owned(),
        Value::String(SPATIAL_MESSAGE_TYPE.to_owned()),
    );
    object.insert(
        "domain_id".to_owned(),
        Value::String(offer.domain_id.clone()),
    );
    object.insert("offer_id".to_owned(), Value::String(offer.offer_id.clone()));
    object.insert("payload".to_owned(), Value::Object(payload));
    object.insert("sequence".to_owned(), Value::String(sequence.to_string()));
    if let Some(generated_at) = generated_at {
        object.insert(
            "generated_at".to_owned(),
            Value::String(generated_at.to_owned()),
        );
    }

    SpatialMessage::from_value(Value::Object(object)).map_err(PublicationMessageError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_protocol::v1::offer::PayloadDescriptor;
    use futures::stream;
    use serde_json::json;

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";

    #[test]
    fn creates_subscribe_only_offer_from_publication_input() {
        let input = PublishOfferInput::new(
            DOMAIN_ID,
            "bytes",
            "example.bytes",
            PayloadDescriptor::create("example.bytes.v1"),
            || stream::iter([vec![1, 2, 3]]),
        )
        .with_display_name("Bytes")
        .with_metadata(json!({"source": "test"}));

        let publication = input.into_publication().expect("publication");
        let offer = publication.offer();
        assert_eq!(offer.domain_id, DOMAIN_ID);
        assert_eq!(offer.offer_id, "bytes");
        assert_eq!(offer.kind, "example.bytes");
        assert_eq!(offer.access_modes, vec![OfferAccessMode::Subscribe]);
        assert_eq!(offer.payload.payload_type, "example.bytes.v1");
        assert_eq!(offer.display_name.as_deref(), Some("Bytes"));
        assert_eq!(offer.metadata, Some(json!({"source": "test"})));
        assert_eq!(
            publication.backpressure_policy(),
            AukiSubscriptionBackpressurePolicy::default()
        );
    }

    #[test]
    fn creates_offer_with_explicit_access_modes() {
        let input = PublishOfferInput::new(
            DOMAIN_ID,
            "bytes",
            "example.bytes",
            PayloadDescriptor::create("example.bytes.v1"),
            || stream::iter([vec![1, 2, 3]]),
        )
        .with_access_modes(vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]);

        let publication = input.into_publication().expect("publication");

        assert_eq!(
            publication.offer().access_modes,
            vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]
        );
    }

    #[test]
    fn published_offer_can_select_backpressure_policy() {
        let input = PublishOfferInput::new(
            DOMAIN_ID,
            "bytes",
            "example.bytes",
            PayloadDescriptor::create("example.bytes.v1"),
            || stream::iter([vec![1, 2, 3]]),
        )
        .with_backpressure_policy(AukiSubscriptionBackpressurePolicy::CloseOnFull { capacity: 2 });

        let publication = input.into_publication().expect("publication");

        assert_eq!(
            publication.backpressure_policy(),
            AukiSubscriptionBackpressurePolicy::CloseOnFull { capacity: 2 }
        );
    }

    #[test]
    fn creates_spatial_messages_from_byte_chunks() {
        let input = PublishOfferInput::new(
            DOMAIN_ID,
            "bytes",
            "example.bytes",
            PayloadDescriptor::create("example.bytes.v1"),
            || stream::iter([vec![1, 2, 3]]),
        );
        let mut publication = input.into_publication().expect("publication");

        let message = publication
            .next_message(
                PublishedByteFrame::new(vec![1, 2, 3]),
                Some("2026-05-26T12:00:00Z"),
            )
            .expect("message");

        assert_eq!(message.domain_id, DOMAIN_ID);
        assert_eq!(message.offer_id, "bytes");
        assert_eq!(message.payload.payload_type, "example.bytes.v1");
        assert_eq!(message.payload.bytes, Some(vec![1, 2, 3]));
        assert_eq!(message.sequence, Some(0));
        assert_eq!(
            message.generated_at.as_deref(),
            Some("2026-05-26T12:00:00Z")
        );
    }

    #[test]
    fn creates_spatial_messages_from_producer_frame_metadata() {
        let input = PublishOfferInput::new(
            DOMAIN_ID,
            "bytes",
            "example.bytes",
            PayloadDescriptor::create("example.bytes.v1"),
            || stream::iter([vec![1, 2, 3]]),
        );
        let mut publication = input.into_publication().expect("publication");

        let message = publication
            .next_message(
                PublishedByteFrame::new(vec![4, 5, 6])
                    .with_sequence(42)
                    .with_generated_at("2026-05-26T12:01:00Z"),
                Some("2026-05-26T12:00:00Z"),
            )
            .expect("message");

        assert_eq!(message.payload.bytes, Some(vec![4, 5, 6]));
        assert_eq!(message.sequence, Some(42));
        assert_eq!(
            message.generated_at.as_deref(),
            Some("2026-05-26T12:01:00Z")
        );
    }

    #[tokio::test]
    async fn latest_source_replays_latest_and_streams_updates() {
        let source = LatestPublishedByteSource::new();
        assert_eq!(source.latest(), None);

        assert!(source.publish(PublishedByteFrame::new(vec![1]).with_sequence(4)));
        let mut stream = source.stream();

        let first = stream.next().await.expect("initial latest frame");
        assert_eq!(first.bytes, vec![1]);
        assert_eq!(first.sequence, Some(4));

        assert!(source.publish(PublishedByteFrame::new(vec![2]).with_sequence(5)));
        let second = stream.next().await.expect("updated frame");
        assert_eq!(second.bytes, vec![2]);
        assert_eq!(second.sequence, Some(5));

        source.close();
        assert!(stream.next().await.is_none());
    }
}

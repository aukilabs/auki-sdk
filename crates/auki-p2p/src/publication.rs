//! Generic local offer publication helpers.

use auki_protocol::v1::{
    base64url,
    message::{SPATIAL_MESSAGE_TYPE, SpatialMessage, SpatialMessageError},
    offer::{
        Offer, OfferAccessMode, OfferError, OfferStatus, PayloadDescriptor, RegistryReference,
    },
    subscribe::SubscribeEndReason,
};
use futures::Stream;
use libp2p::PeerId;
use serde_json::{Map, Value};
use std::{fmt, pin::Pin};

/// Boxed byte stream opened for one Subscribe request.
pub type PublishedByteSource = Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;

/// Factory that opens a byte stream for one Subscribe request.
pub trait PublishedByteSourceFactory: Send {
    /// Open a new byte stream for one accepted Subscribe request.
    fn open(&mut self) -> PublishedByteSource;
}

impl<F, S> PublishedByteSourceFactory for F
where
    F: FnMut() -> S + Send,
    S: Stream<Item = Vec<u8>> + Send + 'static,
{
    fn open(&mut self) -> PublishedByteSource {
        Box::pin(self())
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
    next_sequence: u64,
}

impl PublishOfferInput {
    /// Create high-level publication input from a byte-source factory.
    pub fn new<F, S>(
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        kind: impl Into<String>,
        payload: PayloadDescriptor,
        source_factory: F,
    ) -> Self
    where
        F: FnMut() -> S + Send + 'static,
        S: Stream<Item = Vec<u8>> + Send + 'static,
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

    pub(crate) fn key(&self) -> (String, String) {
        (self.domain_id.clone(), self.offer_id.clone())
    }

    pub(crate) fn into_publication(self) -> Result<LocalOfferPublication, PublishOfferError> {
        let offer = create_published_offer(&self)?;
        Ok(LocalOfferPublication {
            offer,
            source_factory: self.source_factory,
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

    pub(crate) fn next_message(
        &mut self,
        chunk: Vec<u8>,
        generated_at: Option<&str>,
    ) -> Result<SpatialMessage, PublicationMessageError> {
        let sequence = self.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(PublicationMessageError::SequenceOverflow)?;
        let message = create_publication_spatial_message(
            &self.offer,
            sequence,
            chunk.as_slice(),
            generated_at,
        )?;
        self.next_sequence = next_sequence;
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
            .next_message(vec![1, 2, 3], Some("2026-05-26T12:00:00Z"))
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
}

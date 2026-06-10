//! Preview offer profile helpers over generic local offer publication.

use crate::{
    api::{AukiGetProviderError, AukiNode, AukiNodeError},
    publication::{
        AukiSubscriptionBackpressurePolicy, LatestPublishedByteSource, PublishOfferInput,
        PublishedByteFrame, PublishedByteSourceFactory, PublishedOfferHandle,
    },
};
use auki_protocol::v1::{
    base64url, error,
    get::GetRequest,
    message::{SPATIAL_MESSAGE_TYPE, SpatialMessage, SpatialMessageError},
    offer::{OfferAccessMode, PayloadDescriptor},
};
use serde_json::{Map, Value, json};

/// Offer kind for live RGB camera preview frames.
pub const PREVIEW_OFFER_KIND: &str = "auki.sensor.rgb_camera.preview";
/// Payload type for JPEG preview frames.
pub const PREVIEW_PAYLOAD_TYPE: &str = "auki.camera.jpeg_frame.v1";
/// Payload encoding for preview frame bytes.
pub const PREVIEW_PAYLOAD_ENCODING: &str = "binary";
/// Payload media type for preview frame bytes.
pub const PREVIEW_PAYLOAD_MEDIA_TYPE: &str = "image/jpeg";
/// Payload schema version for the initial preview profile.
pub const PREVIEW_PAYLOAD_SCHEMA_VERSION: &str = "1";

/// Options for publishing one preview offer.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewOfferOptions {
    /// Offer domain id.
    pub domain_id: String,
    /// Producer-scoped offer id.
    pub offer_id: String,
    /// Optional human-readable display name.
    pub display_name: Option<String>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
    /// Access modes advertised by the preview offer.
    pub access_modes: Vec<OfferAccessMode>,
    /// Runtime backpressure policy used by preview Subscribe streams.
    pub backpressure_policy: AukiSubscriptionBackpressurePolicy,
}

impl PreviewOfferOptions {
    /// Create preview offer options.
    pub fn new(domain_id: impl Into<String>, offer_id: impl Into<String>) -> Self {
        Self {
            domain_id: domain_id.into(),
            offer_id: offer_id.into(),
            display_name: None,
            metadata: None,
            access_modes: vec![OfferAccessMode::Subscribe],
            backpressure_policy: AukiSubscriptionBackpressurePolicy::LatestOnly,
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

    /// Set advertised access modes for this preview offer.
    pub fn with_access_modes(mut self, access_modes: Vec<OfferAccessMode>) -> Self {
        self.access_modes = access_modes;
        self
    }

    /// Advertise one-shot Get snapshots and Subscribe streams.
    pub fn with_snapshot_and_stream_access(self) -> Self {
        self.with_access_modes(preview_snapshot_and_stream_access_modes())
    }

    /// Set the runtime backpressure policy for preview Subscribe streams.
    pub fn with_backpressure_policy(mut self, policy: AukiSubscriptionBackpressurePolicy) -> Self {
        self.backpressure_policy = policy;
        self
    }
}

/// Build the shared preview payload descriptor.
pub fn preview_payload_descriptor() -> PayloadDescriptor {
    PayloadDescriptor::from_value(json!({
        "type": PREVIEW_PAYLOAD_TYPE,
        "encoding": PREVIEW_PAYLOAD_ENCODING,
        "media_type": PREVIEW_PAYLOAD_MEDIA_TYPE,
        "schema_version": PREVIEW_PAYLOAD_SCHEMA_VERSION,
    }))
    .expect("static preview payload descriptor is valid")
}

/// Access modes for a preview publisher that serves both snapshots and streams.
pub fn preview_snapshot_and_stream_access_modes() -> Vec<OfferAccessMode> {
    vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]
}

/// Build generic publication input for the preview profile.
pub fn preview_offer_input<F>(source_factory: F, options: PreviewOfferOptions) -> PublishOfferInput
where
    F: PublishedByteSourceFactory + 'static,
{
    let mut input = PublishOfferInput::new(
        options.domain_id,
        options.offer_id,
        PREVIEW_OFFER_KIND,
        preview_payload_descriptor(),
        source_factory,
    )
    .with_access_modes(options.access_modes);
    input = input.with_backpressure_policy(options.backpressure_policy);
    if let Some(display_name) = options.display_name {
        input = input.with_display_name(display_name);
    }
    if let Some(metadata) = options.metadata {
        input = input.with_metadata(metadata);
    }
    input
}

/// Build one preview spatial message from JPEG bytes.
pub fn preview_spatial_message(
    domain_id: impl Into<String>,
    offer_id: impl Into<String>,
    sequence: u64,
    bytes: &[u8],
    generated_at: Option<&str>,
) -> Result<SpatialMessage, SpatialMessageError> {
    preview_spatial_message_with_optional_sequence(
        domain_id,
        offer_id,
        Some(sequence),
        bytes,
        generated_at,
    )
}

fn preview_spatial_message_with_optional_sequence(
    domain_id: impl Into<String>,
    offer_id: impl Into<String>,
    sequence: Option<u64>,
    bytes: &[u8],
    generated_at: Option<&str>,
) -> Result<SpatialMessage, SpatialMessageError> {
    let mut payload = preview_payload_descriptor()
        .value()
        .as_object()
        .expect("static preview payload descriptor is a JSON object")
        .clone();
    payload.insert("bytes".to_owned(), Value::String(base64url::encode(bytes)));

    let mut object = Map::new();
    object.insert(
        "type".to_owned(),
        Value::String(SPATIAL_MESSAGE_TYPE.to_owned()),
    );
    object.insert("domain_id".to_owned(), Value::String(domain_id.into()));
    object.insert("offer_id".to_owned(), Value::String(offer_id.into()));
    object.insert("payload".to_owned(), Value::Object(payload));
    if let Some(sequence) = sequence {
        object.insert("sequence".to_owned(), Value::String(sequence.to_string()));
    }
    if let Some(generated_at) = generated_at {
        object.insert(
            "generated_at".to_owned(),
            Value::String(generated_at.to_owned()),
        );
    }

    SpatialMessage::from_value(Value::Object(object))
}

/// Build one preview spatial message from a producer frame.
pub fn preview_spatial_message_from_frame(
    domain_id: impl Into<String>,
    offer_id: impl Into<String>,
    frame: &PublishedByteFrame,
    fallback_generated_at: Option<&str>,
) -> Result<SpatialMessage, SpatialMessageError> {
    preview_spatial_message_with_optional_sequence(
        domain_id,
        offer_id,
        frame.sequence,
        frame.bytes.as_slice(),
        frame.generated_at.as_deref().or(fallback_generated_at),
    )
}

/// Publish a preview offer through the generic native publication API.
pub fn publish_preview_offer<F>(
    node: &mut AukiNode,
    source_factory: F,
    options: PreviewOfferOptions,
) -> Result<PublishedOfferHandle, AukiNodeError>
where
    F: PublishedByteSourceFactory + 'static,
{
    node.publish_offer(preview_offer_input(source_factory, options))
}

/// Publish a preview offer with one-shot Get snapshots and Subscribe streams.
pub fn publish_preview_offer_with_snapshot<F, G>(
    node: &mut AukiNode,
    source_factory: F,
    mut snapshot_factory: G,
    options: PreviewOfferOptions,
) -> Result<PublishedOfferHandle, AukiNodeError>
where
    F: PublishedByteSourceFactory + 'static,
    G: FnMut(&GetRequest, &str) -> Result<Vec<u8>, AukiGetProviderError> + Send + 'static,
{
    let domain_id = options.domain_id.clone();
    let offer_id = options.offer_id.clone();
    let handle = node.publish_offer(preview_offer_input(
        source_factory,
        options.with_snapshot_and_stream_access(),
    ))?;

    let provider_domain_id = domain_id.clone();
    let provider_offer_id = offer_id.clone();
    let mut next_sequence = 0_u64;
    node.upsert_get_provider(
        domain_id,
        offer_id,
        move |request: &GetRequest, now: &str| {
            let sequence = next_sequence;
            next_sequence = next_sequence
                .checked_add(1)
                .ok_or_else(|| AukiGetProviderError::new(error::MESSAGE_INVALID_ENVELOPE))?;
            let bytes = snapshot_factory(request, now)?;
            let frame = PublishedByteFrame::new(bytes).with_sequence(sequence);
            preview_spatial_message_from_frame(
                provider_domain_id.clone(),
                provider_offer_id.clone(),
                &frame,
                Some(now),
            )
            .map_err(|_| AukiGetProviderError::new(error::MESSAGE_INVALID_ENVELOPE))
        },
    )?;

    Ok(handle)
}

/// Publish a preview offer backed by one shared latest-frame source.
///
/// Get returns the current latest frame. Each Subscribe receives the same
/// producer frame stream instead of opening an independent source instance.
pub fn publish_preview_offer_with_latest_source(
    node: &mut AukiNode,
    source: LatestPublishedByteSource,
    options: PreviewOfferOptions,
) -> Result<PublishedOfferHandle, AukiNodeError> {
    let domain_id = options.domain_id.clone();
    let offer_id = options.offer_id.clone();
    let handle = node.publish_offer(preview_offer_input(
        source.clone(),
        options.with_snapshot_and_stream_access(),
    ))?;

    let provider_domain_id = domain_id.clone();
    let provider_offer_id = offer_id.clone();
    node.upsert_get_provider(
        domain_id,
        offer_id,
        move |_request: &GetRequest, now: &str| {
            let frame = source
                .latest()
                .ok_or_else(|| AukiGetProviderError::new(error::OFFER_TEMPORARILY_UNAVAILABLE))?;
            preview_spatial_message_from_frame(
                provider_domain_id.clone(),
                provider_offer_id.clone(),
                &frame,
                Some(now),
            )
            .map_err(|_| AukiGetProviderError::new(error::MESSAGE_INVALID_ENVELOPE))
        },
    )?;

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AukiP2pNodeConfig, LocalDomainRegistration, LocalPeerIdentity};
    use auki_identity::Wallet;
    use auki_protocol::v1::domain::{DOMAIN_NONCE_LEN, DomainDeclaration};
    use futures::stream;
    use std::sync::Arc;

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";
    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";

    fn wallet(seed: u8) -> Arc<Wallet> {
        Wallet::from_seed(vec![seed; 32]).expect("32-byte seed")
    }

    fn identity_from_wallet(wallet: Arc<Wallet>) -> LocalPeerIdentity {
        LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, Some("preview-test"))
            .expect("local peer identity")
    }

    #[test]
    fn payload_descriptor_matches_shared_preview_profile() {
        let descriptor = preview_payload_descriptor();

        assert_eq!(descriptor.payload_type, PREVIEW_PAYLOAD_TYPE);
        assert_eq!(
            descriptor.encoding.as_deref(),
            Some(PREVIEW_PAYLOAD_ENCODING)
        );
        assert_eq!(
            descriptor.media_type.as_deref(),
            Some(PREVIEW_PAYLOAD_MEDIA_TYPE)
        );
        assert_eq!(
            descriptor.schema_version.as_deref(),
            Some(PREVIEW_PAYLOAD_SCHEMA_VERSION)
        );
    }

    #[test]
    fn publish_preview_offer_registers_profile_offer() {
        let local_wallet = wallet(21);
        let declaration = DomainDeclaration::create(
            &local_wallet,
            &[21; DOMAIN_NONCE_LEN],
            Some("preview-profile"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut node = AukiNode::new(
            identity_from_wallet(local_wallet),
            AukiP2pNodeConfig::dial_only_development(),
        )
        .expect("node");
        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");

        let handle = publish_preview_offer(
            &mut node,
            || stream::iter([vec![0xff, 0xd8, 0xff, 0xd9]]),
            PreviewOfferOptions::new(domain_id.clone(), "preview-main")
                .with_display_name("Preview Main")
                .with_metadata(json!({"source": "generated"})),
        )
        .expect("publish preview");

        assert_eq!(handle.domain_id(), domain_id);
        assert_eq!(handle.offer_id(), "preview-main");

        let offers = node.local_offers(&domain_id);
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].kind, PREVIEW_OFFER_KIND);
        assert_eq!(offers[0].payload.payload_type, PREVIEW_PAYLOAD_TYPE);
        assert_eq!(
            offers[0].payload.encoding.as_deref(),
            Some(PREVIEW_PAYLOAD_ENCODING)
        );
        assert_eq!(
            offers[0].payload.media_type.as_deref(),
            Some(PREVIEW_PAYLOAD_MEDIA_TYPE)
        );
        assert_eq!(
            offers[0].payload.schema_version.as_deref(),
            Some(PREVIEW_PAYLOAD_SCHEMA_VERSION)
        );
        assert_eq!(offers[0].access_modes, vec![OfferAccessMode::Subscribe]);
        assert_eq!(offers[0].display_name.as_deref(), Some("Preview Main"));
        assert_eq!(offers[0].metadata, Some(json!({"source": "generated"})));
    }

    #[test]
    fn preview_snapshot_and_stream_offer_advertises_both_access_modes() {
        let input = preview_offer_input(
            || stream::iter([vec![0xff, 0xd8, 0xff, 0xd9]]),
            PreviewOfferOptions::new(DOMAIN_ID, "preview-main").with_snapshot_and_stream_access(),
        );
        let publication = input.into_publication().expect("publication");

        assert_eq!(
            publication.offer().access_modes,
            vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]
        );
        assert_eq!(
            publication.backpressure_policy(),
            AukiSubscriptionBackpressurePolicy::LatestOnly
        );
    }

    #[test]
    fn preview_offer_can_override_backpressure_policy() {
        let input = preview_offer_input(
            || stream::iter([vec![0xff, 0xd8, 0xff, 0xd9]]),
            PreviewOfferOptions::new(DOMAIN_ID, "preview-main").with_backpressure_policy(
                AukiSubscriptionBackpressurePolicy::Bounded { capacity: 2 },
            ),
        );
        let publication = input.into_publication().expect("publication");

        assert_eq!(
            publication.backpressure_policy(),
            AukiSubscriptionBackpressurePolicy::Bounded { capacity: 2 }
        );
    }

    #[test]
    fn preview_spatial_message_wraps_jpeg_bytes() {
        let message = preview_spatial_message(
            DOMAIN_ID,
            "preview-main",
            7,
            &[0xff, 0xd8, 0xff, 0xd9],
            Some(ISSUED_AT),
        )
        .expect("preview message");

        assert_eq!(message.domain_id, DOMAIN_ID);
        assert_eq!(message.offer_id, "preview-main");
        assert_eq!(message.sequence, Some(7));
        assert_eq!(message.generated_at.as_deref(), Some(ISSUED_AT));
        assert_eq!(message.payload.payload_type, PREVIEW_PAYLOAD_TYPE);
        assert_eq!(message.payload.bytes, Some(vec![0xff, 0xd8, 0xff, 0xd9]));
    }

    #[test]
    fn preview_spatial_message_from_frame_uses_source_metadata() {
        let frame = PublishedByteFrame::new(vec![0xff, 0xd8, 0xff, 0xd9])
            .with_sequence(12)
            .with_generated_at("2026-05-26T12:01:00Z");

        let message =
            preview_spatial_message_from_frame(DOMAIN_ID, "preview-main", &frame, Some(ISSUED_AT))
                .expect("preview message");

        assert_eq!(message.sequence, Some(12));
        assert_eq!(
            message.generated_at.as_deref(),
            Some("2026-05-26T12:01:00Z")
        );
        assert_eq!(message.payload.bytes, Some(vec![0xff, 0xd8, 0xff, 0xd9]));
    }

    #[test]
    fn publish_preview_offer_with_latest_source_registers_get_and_subscribe() {
        let local_wallet = wallet(22);
        let declaration = DomainDeclaration::create(
            &local_wallet,
            &[22; DOMAIN_NONCE_LEN],
            Some("preview-latest"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut node = AukiNode::new(
            identity_from_wallet(local_wallet),
            AukiP2pNodeConfig::dial_only_development(),
        )
        .expect("node");
        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        let source = LatestPublishedByteSource::new();
        assert!(
            source.publish(PublishedByteFrame::new(vec![0xff, 0xd8, 0xff, 0xd9]).with_sequence(3))
        );

        let handle = publish_preview_offer_with_latest_source(
            &mut node,
            source,
            PreviewOfferOptions::new(domain_id.clone(), "preview-main"),
        )
        .expect("publish preview");

        assert_eq!(handle.domain_id(), domain_id);
        assert_eq!(handle.offer_id(), "preview-main");
        let offers = node.local_offers(&domain_id);
        assert_eq!(
            offers[0].access_modes,
            vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]
        );
    }
}

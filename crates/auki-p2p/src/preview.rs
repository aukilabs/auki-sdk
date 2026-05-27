//! Preview offer profile helpers over generic local offer publication.

use crate::{
    api::{AukiNode, AukiNodeError},
    publication::{PublishOfferInput, PublishedOfferHandle},
};
use auki_protocol::v1::offer::PayloadDescriptor;
use futures::Stream;
use serde_json::{Value, json};

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
}

impl PreviewOfferOptions {
    /// Create preview offer options.
    pub fn new(domain_id: impl Into<String>, offer_id: impl Into<String>) -> Self {
        Self {
            domain_id: domain_id.into(),
            offer_id: offer_id.into(),
            display_name: None,
            metadata: None,
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

/// Build generic publication input for the preview profile.
pub fn preview_offer_input<F, S>(
    source_factory: F,
    options: PreviewOfferOptions,
) -> PublishOfferInput
where
    F: FnMut() -> S + Send + 'static,
    S: Stream<Item = Vec<u8>> + Send + 'static,
{
    let mut input = PublishOfferInput::new(
        options.domain_id,
        options.offer_id,
        PREVIEW_OFFER_KIND,
        preview_payload_descriptor(),
        source_factory,
    );
    if let Some(display_name) = options.display_name {
        input = input.with_display_name(display_name);
    }
    if let Some(metadata) = options.metadata {
        input = input.with_metadata(metadata);
    }
    input
}

/// Publish a preview offer through the generic native publication API.
pub fn publish_preview_offer<F, S>(
    node: &mut AukiNode,
    source_factory: F,
    options: PreviewOfferOptions,
) -> Result<PublishedOfferHandle, AukiNodeError>
where
    F: FnMut() -> S + Send + 'static,
    S: Stream<Item = Vec<u8>> + Send + 'static,
{
    node.publish_offer(preview_offer_input(source_factory, options))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AukiP2pNodeConfig, LocalDomainRegistration, LocalPeerIdentity};
    use auki_identity::Wallet;
    use auki_protocol::v1::domain::{DOMAIN_NONCE_LEN, DomainDeclaration};
    use futures::stream;
    use std::sync::Arc;

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
        assert_eq!(offers[0].display_name.as_deref(), Some("Preview Main"));
        assert_eq!(offers[0].metadata, Some(json!({"source": "generated"})));
    }
}

//! libp2p stream clients for Get and Subscribe path operations.

use crate::{
    GetClient, GetInput, GetOutcome, OfferLoadReport, PathClientError, PathContext,
    PathOrchestrationError, PeerRelationship, RuntimeLimits, SubscribeClient, SubscribeInput,
    SubscriptionHandle,
    protocols::{get_protocol, subscribe_protocol},
};
use auki_protocol::v1::{
    error,
    frame::{self, FrameError},
    get::GetRequest,
    subscribe::SubscribeRequest,
};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::PeerId;
use std::{future::Future, io};

/// libp2p-stream-backed client for outgoing Get and Subscribe starts.
pub struct Libp2pPathClient {
    control: libp2p_stream::Control,
    limits: RuntimeLimits,
    subscription_stream: Option<libp2p::Stream>,
}

/// Accepted Subscribe stream retained after the start result has validated.
pub struct Libp2pSubscription {
    handle: SubscriptionHandle,
    stream: libp2p::Stream,
}

impl Libp2pPathClient {
    /// Create a path client from a raw libp2p stream control handle.
    pub fn new(control: libp2p_stream::Control, limits: RuntimeLimits) -> Self {
        Self {
            control,
            limits,
            subscription_stream: None,
        }
    }

    /// Borrow the configured runtime limits.
    pub fn limits(&self) -> RuntimeLimits {
        self.limits
    }

    /// Take the stream retained by the last Subscribe start operation.
    pub fn take_subscription_stream(&mut self) -> Option<libp2p::Stream> {
        self.subscription_stream.take()
    }

    async fn run_get(
        &mut self,
        peer_id: PeerId,
        request: GetRequest,
    ) -> Result<Vec<u8>, PathClientError> {
        let mut stream = self
            .control
            .open_stream(peer_id, get_protocol())
            .await
            .map_err(|error| open_stream_error("get", error))?;

        write_json_frame(
            &mut stream,
            request.value(),
            self.limits.get_response_frame_body_bytes,
            error::GET_INVALID_REQUEST,
            "get request",
        )
        .await?;

        let response = read_frame_bytes(
            &mut stream,
            self.limits.get_response_frame_body_bytes,
            "get response",
        )
        .await?;
        Ok(response)
    }

    async fn run_subscribe(
        &mut self,
        peer_id: PeerId,
        request: SubscribeRequest,
    ) -> Result<Vec<u8>, PathClientError> {
        if self.subscription_stream.is_some() {
            return Err(PathClientError::new(
                error::TRANSPORT_FAILED,
                "previous subscribe stream has not been taken",
                false,
            ));
        }

        let mut stream = self
            .control
            .open_stream(peer_id, subscribe_protocol())
            .await
            .map_err(|error| open_stream_error("subscribe", error))?;

        write_json_frame(
            &mut stream,
            request.value(),
            self.limits.subscribe_message_frame_body_bytes,
            error::SUBSCRIBE_INVALID_REQUEST,
            "subscribe request",
        )
        .await?;

        let start = read_frame_bytes(
            &mut stream,
            self.limits.subscribe_message_frame_body_bytes,
            "subscribe start",
        )
        .await?;
        self.subscription_stream = Some(stream);
        Ok(start)
    }
}

impl Libp2pSubscription {
    /// Borrow the validated Subscribe handle.
    pub fn handle(&self) -> &SubscriptionHandle {
        &self.handle
    }

    /// Mutably borrow the validated Subscribe handle.
    pub fn handle_mut(&mut self) -> &mut SubscriptionHandle {
        &mut self.handle
    }

    /// Mutably borrow the underlying libp2p stream.
    pub fn stream_mut(&mut self) -> &mut libp2p::Stream {
        &mut self.stream
    }

    /// Read the next raw Subscribe data or end frame from this stream.
    pub async fn read_next_frame(&mut self, max_body_len: u64) -> Result<Vec<u8>, PathClientError> {
        read_frame_bytes(&mut self.stream, max_body_len, "subscribe message").await
    }
}

impl GetClient for Libp2pPathClient {
    fn get(
        &mut self,
        peer_id: PeerId,
        request: GetRequest,
    ) -> impl Future<Output = Result<Vec<u8>, PathClientError>> {
        self.run_get(peer_id, request)
    }
}

impl SubscribeClient for Libp2pPathClient {
    fn subscribe(
        &mut self,
        peer_id: PeerId,
        request: SubscribeRequest,
    ) -> impl Future<Output = Result<Vec<u8>, PathClientError>> {
        self.run_subscribe(peer_id, request)
    }
}

/// Run Get through a libp2p stream client.
pub async fn get_over_libp2p(
    relationship: &mut PeerRelationship,
    offers: &OfferLoadReport,
    client: &mut Libp2pPathClient,
    input: GetInput,
    context: PathContext<'_>,
) -> Result<GetOutcome, PathOrchestrationError> {
    crate::paths::get(relationship, offers, client, input, context).await
}

/// Start Subscribe through a libp2p stream client and retain the stream.
pub async fn subscribe_over_libp2p(
    relationship: &mut PeerRelationship,
    offers: &OfferLoadReport,
    client: &mut Libp2pPathClient,
    input: SubscribeInput,
    context: PathContext<'_>,
) -> Result<Libp2pSubscription, PathOrchestrationError> {
    let handle = crate::paths::subscribe(relationship, offers, client, input, context).await?;
    let stream = client.take_subscription_stream().ok_or_else(|| {
        PathOrchestrationError::SubscribeClient(PathClientError::new(
            error::TRANSPORT_FAILED,
            "subscribe stream was not retained",
            true,
        ))
    })?;
    Ok(Libp2pSubscription { handle, stream })
}

async fn write_json_frame<S>(
    stream: &mut S,
    value: &serde_json::Value,
    max_body_len: u64,
    invalid_request_code: &'static str,
    label: &'static str,
) -> Result<(), PathClientError>
where
    S: AsyncWrite + Unpin,
{
    let frame = frame::encode_json_frame(value, max_body_len).map_err(|error| {
        PathClientError::new(
            invalid_request_code,
            format!("{label} frame encode failed: {error}"),
            false,
        )
    })?;
    stream
        .write_all(&frame)
        .await
        .map_err(|error| io_error(label, error))?;
    stream.flush().await.map_err(|error| io_error(label, error))
}

async fn read_frame_bytes<S>(
    stream: &mut S,
    max_body_len: u64,
    label: &'static str,
) -> Result<Vec<u8>, PathClientError>
where
    S: AsyncRead + Unpin,
{
    let mut prefix = Vec::with_capacity(frame::MAX_LEB128_U64_BYTES);

    loop {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|error| io_error(label, error))?;
        prefix.push(byte[0]);

        match frame::decode_length(&prefix, max_body_len) {
            Ok((body_len, prefix_len)) => {
                debug_assert_eq!(prefix_len, prefix.len());
                let body_len = usize::try_from(body_len).map_err(|_| {
                    frame_error(label, FrameError::LengthOverflow, "frame length overflow")
                })?;
                let mut body = vec![0u8; body_len];
                stream
                    .read_exact(&mut body)
                    .await
                    .map_err(|error| io_error(label, error))?;

                let mut complete = prefix;
                complete.extend_from_slice(&body);
                return Ok(complete);
            }
            Err(FrameError::UnexpectedEof) if prefix.len() < frame::MAX_LEB128_U64_BYTES => {}
            Err(error) => return Err(frame_error(label, error, "frame decode failed")),
        }
    }
}

fn open_stream_error(
    label: &'static str,
    error: libp2p_stream::OpenStreamError,
) -> PathClientError {
    PathClientError::new(
        error::TRANSPORT_FAILED,
        format!("{label} stream open failed: {error}"),
        true,
    )
}

fn io_error(label: &'static str, error: io::Error) -> PathClientError {
    PathClientError::new(
        error::TRANSPORT_FAILED,
        format!("{label} stream io failed: {error}"),
        true,
    )
}

fn frame_error(label: &'static str, error: FrameError, message: &'static str) -> PathClientError {
    let code = match error {
        FrameError::BodyTooLarge { .. } => error::MESSAGE_PAYLOAD_TOO_LARGE,
        _ => error::TRANSPORT_FAILED,
    };
    PathClientError::new(code, format!("{label} {message}: {error}"), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AukiP2pEvent, AukiP2pNode, AukiP2pNodeConfig, LoadedRemoteOffer, OfferCatalogLoadState,
        OfferLoadReport, PeerRelationshipState, accept_subscribe_data_frame,
    };
    use auki_identity::Wallet;
    use auki_protocol::v1::{
        frame::{decode_json_frame, encode_json_frame},
        get::{GetRequest, GetResponse},
        message::SpatialMessage,
        offer::{Offer, OfferAccessMode, OfferStatus, PayloadDescriptor, RegistryReference},
        subscribe::SubscribeAccept,
    };
    use futures::StreamExt as _;
    use libp2p::Multiaddr;
    use serde_json::{Value, json};
    use tokio::time::{Duration, timeout};

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";
    const VALID_HASH: &str = "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const NOW: &str = "2026-05-26T12:30:00Z";

    fn identity(seed: u8) -> crate::LocalPeerIdentity {
        let wallet = Wallet::from_seed(vec![seed; 32]).expect("32-byte seed");
        crate::LocalPeerIdentity::from_wallet(wallet, NOW, Some("path-streams-test"))
            .expect("local peer identity")
    }

    fn relationship(peer_id: PeerId) -> PeerRelationship {
        let mut relationship = PeerRelationship::new(peer_id);
        relationship.state = PeerRelationshipState::Ready;
        relationship.connected = true;
        relationship.authorized = true;
        relationship.accepted_served_domains = vec![DOMAIN_ID.to_owned()];
        relationship.offer_catalog_state = OfferCatalogLoadState::Loaded;
        relationship
    }

    fn offer_report(peer_id: PeerId) -> OfferLoadReport {
        OfferLoadReport {
            peer_id,
            offers: vec![LoadedRemoteOffer {
                offer: offer("camera-main", "auki.frame"),
                usable: true,
                unusable_reason: None,
            }],
            diagnostics: Vec::new(),
            generated_at: Some(NOW.to_owned()),
        }
    }

    fn offer(offer_id: &str, payload_type: &str) -> Offer {
        let reference = RegistryReference::create("clock", "clock", "clock-main", VALID_HASH, None)
            .expect("registry ref");
        Offer::create(
            offer_id,
            DOMAIN_ID,
            "sensor.frame",
            OfferStatus::Available,
            vec![OfferAccessMode::Get, OfferAccessMode::Subscribe],
            PayloadDescriptor::create(payload_type),
            vec![reference],
        )
        .expect("offer")
    }

    fn message_value(sequence: u64) -> Value {
        json!({
            "type": auki_protocol::v1::message::SPATIAL_MESSAGE_TYPE,
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
            "payload": {
                "type": "auki.frame",
                "bytes": "AQID",
                "json": {"ok": true},
            },
            "sequence": sequence.to_string(),
            "generated_at": NOW,
        })
    }

    fn message(sequence: u64) -> SpatialMessage {
        SpatialMessage::from_value(message_value(sequence)).expect("message")
    }

    async fn wait_for_listen_addr(node: &mut AukiP2pNode) -> Multiaddr {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(AukiP2pEvent::Listening { address }) = node.next_event().await {
                    return address;
                }
            }
        })
        .await
        .expect("listen address should be emitted")
    }

    async fn drain_node(mut node: AukiP2pNode) {
        while node.next_event().await.is_some() {}
    }

    fn decode_request_frame(frame: &[u8], max_body_len: u64) -> Value {
        let (value, consumed) = decode_json_frame(frame, max_body_len).expect("request frame");
        assert_eq!(consumed, frame.len());
        value
    }

    #[tokio::test]
    async fn stream_frame_reader_rejects_body_over_limit_before_json_parse() {
        let frame = encode_json_frame(&json!({"type": "too.large"}), 1024).expect("frame");
        let mut cursor = futures::io::Cursor::new(frame);

        let error = read_frame_bytes(&mut cursor, 8, "test frame")
            .await
            .expect_err("oversized frame should fail");

        assert_eq!(error.code, error::MESSAGE_PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn get_over_libp2p_opens_one_stream_and_validates_response() {
        let mut dialer =
            AukiP2pNode::new(identity(41), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiP2pNode::new(identity(42), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let p2p_config = dialer.config().p2p.clone();
        let limits = p2p_config.limits;
        let mut incoming = listener
            .stream_control()
            .accept(get_protocol())
            .expect("accept get streams");
        let mut client = Libp2pPathClient::new(dialer.stream_control(), limits);

        dialer
            .dial_peer(listener_peer_id, vec![listener_addr])
            .expect("dial should be accepted");
        let dialer_task = tokio::spawn(drain_node(dialer));
        let listener_task = tokio::spawn(drain_node(listener));

        let server = tokio::spawn(async move {
            let (peer_id, mut stream) = incoming.next().await.expect("get stream");
            assert_eq!(peer_id, dialer_peer_id);
            let request_frame = read_frame_bytes(
                &mut stream,
                limits.get_response_frame_body_bytes,
                "get request",
            )
            .await
            .expect("read get request");
            let request = GetRequest::from_value(decode_request_frame(
                &request_frame,
                limits.get_response_frame_body_bytes,
            ))
            .expect("get request");
            assert_eq!(request.domain_id, DOMAIN_ID);
            assert_eq!(request.offer_id, "camera-main");

            let response = GetResponse::success(message(7));
            let response_frame =
                encode_json_frame(response.value(), limits.get_response_frame_body_bytes)
                    .expect("get response frame");
            stream
                .write_all(&response_frame)
                .await
                .expect("write get response");
            stream.close().await.expect("close get stream");
        });

        let mut relationship = relationship(listener_peer_id);
        let outcome = get_over_libp2p(
            &mut relationship,
            &offer_report(listener_peer_id),
            &mut client,
            GetInput::new(DOMAIN_ID, "camera-main"),
            PathContext::new(&p2p_config, NOW),
        )
        .await
        .expect("get over libp2p");

        assert_eq!(outcome.message.sequence, Some(7));
        assert_eq!(relationship.paths[0].state.as_deref(), Some("succeeded"));
        server.await.expect("server task");
        dialer_task.abort();
        listener_task.abort();
    }

    #[tokio::test]
    async fn subscribe_over_libp2p_retains_stream_for_data_frames() {
        let mut dialer =
            AukiP2pNode::new(identity(43), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiP2pNode::new(identity(44), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let p2p_config = dialer.config().p2p.clone();
        let limits = p2p_config.limits;
        let mut incoming = listener
            .stream_control()
            .accept(subscribe_protocol())
            .expect("accept subscribe streams");
        let mut client = Libp2pPathClient::new(dialer.stream_control(), limits);

        dialer
            .dial_peer(listener_peer_id, vec![listener_addr])
            .expect("dial should be accepted");
        let dialer_task = tokio::spawn(drain_node(dialer));
        let listener_task = tokio::spawn(drain_node(listener));

        let server = tokio::spawn(async move {
            let (peer_id, mut stream) = incoming.next().await.expect("subscribe stream");
            assert_eq!(peer_id, dialer_peer_id);
            let request_frame = read_frame_bytes(
                &mut stream,
                limits.subscribe_message_frame_body_bytes,
                "subscribe request",
            )
            .await
            .expect("read subscribe request");
            let request = SubscribeRequest::from_value(decode_request_frame(
                &request_frame,
                limits.subscribe_message_frame_body_bytes,
            ))
            .expect("subscribe request");
            assert_eq!(request.domain_id, DOMAIN_ID);
            assert_eq!(request.offer_id, "camera-main");

            let accept = SubscribeAccept::create(
                DOMAIN_ID,
                "camera-main",
                PayloadDescriptor::create("auki.frame"),
                Vec::new(),
                Some(1),
                Some(NOW.to_owned()),
                None,
            )
            .expect("accept");
            let accept_frame =
                encode_json_frame(accept.value(), limits.subscribe_message_frame_body_bytes)
                    .expect("accept frame");
            stream.write_all(&accept_frame).await.expect("write accept");

            let data_frame =
                encode_json_frame(&message_value(1), limits.subscribe_message_frame_body_bytes)
                    .expect("data frame");
            stream.write_all(&data_frame).await.expect("write data");
            stream.close().await.expect("close subscribe stream");
        });

        let mut relationship = relationship(listener_peer_id);
        let mut subscription = subscribe_over_libp2p(
            &mut relationship,
            &offer_report(listener_peer_id),
            &mut client,
            SubscribeInput::new(DOMAIN_ID, "camera-main"),
            PathContext::new(&p2p_config, NOW),
        )
        .await
        .expect("subscribe over libp2p");

        assert_eq!(subscription.handle().payload_type(), "auki.frame");
        let data_frame = subscription
            .read_next_frame(limits.subscribe_message_frame_body_bytes)
            .await
            .expect("data frame");
        let message = accept_subscribe_data_frame(
            &mut relationship,
            subscription.handle_mut(),
            &data_frame,
            PathContext::new(&p2p_config, "2026-05-26T12:31:00Z"),
        )
        .expect("accepted data frame");

        assert_eq!(message.sequence, Some(1));
        assert_eq!(relationship.paths[0].last_sequence, Some(1));
        server.await.expect("server task");
        dialer_task.abort();
        listener_task.abort();
    }
}

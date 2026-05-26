//! Lifecycle stream helpers for the RFC-first peer handshake.

use crate::{LocalPeerIdentity, protocols::cluster_lifecycle_protocol};
use auki_protocol::v1::{
    frame::{self, FrameError},
    handshake::{HandshakeError, PeerHandshake},
};
use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::PeerId;
use std::{fmt, io};

/// Result of a lifecycle handshake exchange over one libp2p stream.
#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleHandshakeExchange {
    /// Transport-authenticated remote peer id for the stream.
    pub authenticated_peer_id: PeerId,
    /// Remote handshake decoded from the stream.
    pub handshake: PeerHandshake,
}

/// Errors produced while exchanging lifecycle handshake frames.
#[derive(Debug)]
pub enum LifecycleProtocolError {
    /// Underlying stream I/O failed.
    Io(io::Error),
    /// RFC JSON frame encoding or decoding failed.
    Frame(FrameError),
    /// Decoded frame was not a valid peer handshake.
    Handshake(HandshakeError),
}

/// Build the local peer-handshake body for this identity.
pub fn build_local_peer_handshake(identity: &LocalPeerIdentity) -> PeerHandshake {
    PeerHandshake::create(identity.peer_binding().clone(), Vec::new())
}

/// Accept inbound lifecycle streams on a libp2p-stream control.
pub fn accept_lifecycle_streams(
    control: &mut libp2p_stream::Control,
) -> Result<libp2p_stream::IncomingStreams, libp2p_stream::AlreadyRegistered> {
    control.accept(cluster_lifecycle_protocol())
}

/// Open an outbound lifecycle stream to a remote peer.
pub async fn open_lifecycle_stream(
    control: &mut libp2p_stream::Control,
    peer_id: PeerId,
) -> Result<libp2p::Stream, libp2p_stream::OpenStreamError> {
    control
        .open_stream(peer_id, cluster_lifecycle_protocol())
        .await
}

/// Write one peer-handshake JSON frame.
pub async fn write_peer_handshake<S>(
    stream: &mut S,
    handshake: &PeerHandshake,
    max_body_len: u64,
) -> Result<(), LifecycleProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let frame = frame::encode_json_frame(handshake.value(), max_body_len)
        .map_err(LifecycleProtocolError::Frame)?;
    stream
        .write_all(&frame)
        .await
        .map_err(LifecycleProtocolError::Io)?;
    stream.flush().await.map_err(LifecycleProtocolError::Io)
}

/// Read one peer-handshake JSON frame.
pub async fn read_peer_handshake<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<PeerHandshake, LifecycleProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let value = read_json_frame(stream, max_body_len).await?;
    PeerHandshake::from_value(value).map_err(LifecycleProtocolError::Handshake)
}

/// Send the local handshake frame first, then read the remote handshake frame.
///
/// Both sides can call this concurrently without deadlocking. The protocol
/// validation step is deliberately separate and belongs to the policy kernel.
pub async fn exchange_peer_handshake<S>(
    stream: &mut S,
    authenticated_peer_id: PeerId,
    local_handshake: &PeerHandshake,
    max_body_len: u64,
) -> Result<LifecycleHandshakeExchange, LifecycleProtocolError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_peer_handshake(stream, local_handshake, max_body_len).await?;
    let handshake = read_peer_handshake(stream, max_body_len).await?;
    Ok(LifecycleHandshakeExchange {
        authenticated_peer_id,
        handshake,
    })
}

async fn read_json_frame<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<serde_json::Value, LifecycleProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let mut prefix = Vec::with_capacity(frame::MAX_LEB128_U64_BYTES);

    loop {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(LifecycleProtocolError::Io)?;
        prefix.push(byte[0]);

        match frame::decode_length(&prefix, max_body_len) {
            Ok((body_len, prefix_len)) => {
                debug_assert_eq!(prefix_len, prefix.len());
                let mut body = vec![0u8; body_len as usize];
                stream
                    .read_exact(&mut body)
                    .await
                    .map_err(LifecycleProtocolError::Io)?;

                let mut complete = prefix;
                complete.extend_from_slice(&body);
                let (value, consumed) = frame::decode_json_frame(&complete, max_body_len)
                    .map_err(LifecycleProtocolError::Frame)?;
                debug_assert_eq!(consumed, complete.len());
                return Ok(value);
            }
            Err(FrameError::UnexpectedEof) if prefix.len() < frame::MAX_LEB128_U64_BYTES => {}
            Err(error) => return Err(LifecycleProtocolError::Frame(error)),
        }
    }
}

impl fmt::Display for LifecycleProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "lifecycle stream io: {error}"),
            Self::Frame(error) => write!(f, "lifecycle frame: {error}"),
            Self::Handshake(error) => write!(f, "lifecycle handshake: {error}"),
        }
    }
}

impl std::error::Error for LifecycleProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Frame(error) => Some(error),
            Self::Handshake(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AukiP2pEvent, AukiP2pNode, AukiP2pNodeConfig};
    use auki_identity::Wallet;
    use auki_protocol::v1::handshake::CLUSTER_LIFECYCLE_V1;
    use futures::StreamExt as _;
    use libp2p::Multiaddr;
    use tokio::time::{Duration, timeout};

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";

    fn identity(seed: u8) -> LocalPeerIdentity {
        let wallet = Wallet::from_seed(vec![seed; 32]).expect("32-byte seed");
        LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, Some("lifecycle-test"))
            .expect("local peer identity")
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

    async fn wait_for_connection(
        dialer: &mut AukiP2pNode,
        listener: &mut AukiP2pNode,
        dialer_peer_id: PeerId,
        listener_peer_id: PeerId,
    ) {
        timeout(Duration::from_secs(10), async {
            let mut dialer_observed_listener = false;
            let mut listener_observed_dialer = false;

            loop {
                tokio::select! {
                    event = dialer.next_event() => {
                        if let Some(AukiP2pEvent::ConnectionEstablished { peer_id }) = event {
                            dialer_observed_listener |= peer_id == listener_peer_id;
                        }
                    }
                    event = listener.next_event() => {
                        if let Some(AukiP2pEvent::ConnectionEstablished { peer_id }) = event {
                            listener_observed_dialer |= peer_id == dialer_peer_id;
                        }
                    }
                }

                if dialer_observed_listener && listener_observed_dialer {
                    break;
                }
            }
        })
        .await
        .expect("both peers should observe an authenticated connection");
    }

    #[tokio::test]
    async fn lifecycle_stream_exchanges_peer_handshake_frames_both_ways() {
        let mut dialer =
            AukiP2pNode::new(identity(31), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiP2pNode::new(identity(32), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;

        let mut listener_control = listener.stream_control();
        let mut incoming =
            accept_lifecycle_streams(&mut listener_control).expect("accept lifecycle streams");

        dialer
            .dial_peer(listener_peer_id, vec![listener_addr])
            .expect("dial should be accepted");
        wait_for_connection(&mut dialer, &mut listener, dialer_peer_id, listener_peer_id).await;

        let mut dialer_control = dialer.stream_control();
        let open = open_lifecycle_stream(&mut dialer_control, listener_peer_id);
        tokio::pin!(open);
        let mut outbound = None;
        let mut inbound = None;

        timeout(Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    result = &mut open, if outbound.is_none() => {
                        outbound = Some(result.expect("open lifecycle stream"));
                    }
                    accepted = incoming.next(), if inbound.is_none() => {
                        inbound = Some(accepted.expect("inbound lifecycle stream"));
                    }
                    _ = dialer.next_event() => {}
                    _ = listener.next_event() => {}
                }

                if outbound.is_some() && inbound.is_some() {
                    break;
                }
            }
        })
        .await
        .expect("lifecycle stream should open on both peers");

        let mut outbound = outbound.expect("outbound stream");
        let (accepted_peer_id, mut inbound) = inbound.expect("inbound stream");
        assert_eq!(accepted_peer_id, dialer_peer_id);

        let dialer_handshake = build_local_peer_handshake(dialer.identity());
        let listener_handshake = build_local_peer_handshake(listener.identity());
        let dialer_limit = dialer.config().p2p.limits.handshake_frame_body_bytes;
        let listener_limit = listener.config().p2p.limits.handshake_frame_body_bytes;

        let (dialer_exchange, listener_exchange) = tokio::join!(
            exchange_peer_handshake(
                &mut outbound,
                listener_peer_id,
                &dialer_handshake,
                dialer_limit,
            ),
            exchange_peer_handshake(
                &mut inbound,
                accepted_peer_id,
                &listener_handshake,
                listener_limit,
            )
        );
        let dialer_exchange = dialer_exchange.expect("dialer exchange");
        let listener_exchange = listener_exchange.expect("listener exchange");

        assert_eq!(dialer_exchange.authenticated_peer_id, listener_peer_id);
        assert_eq!(listener_exchange.authenticated_peer_id, dialer_peer_id);
        assert_eq!(
            dialer_exchange.handshake.supported_lifecycle_versions,
            vec![CLUSTER_LIFECYCLE_V1.to_owned()]
        );
        assert_eq!(
            listener_exchange.handshake.supported_lifecycle_versions,
            vec![CLUSTER_LIFECYCLE_V1.to_owned()]
        );
        assert_eq!(
            dialer_exchange
                .handshake
                .peer_binding
                .verify_for_peer_id(&listener_peer_id)
                .expect("listener peer binding")
                .peer_id,
            listener_peer_id
        );
        assert_eq!(
            listener_exchange
                .handshake
                .peer_binding
                .verify_for_peer_id(&dialer_peer_id)
                .expect("dialer peer binding")
                .peer_id,
            dialer_peer_id
        );
    }

    #[tokio::test]
    async fn read_peer_handshake_rejects_frames_over_configured_limit() {
        let handshake = build_local_peer_handshake(&identity(33));
        let mut bytes = Vec::new();
        write_peer_handshake(&mut bytes, &handshake, 64 * 1024)
            .await
            .unwrap();
        let mut cursor = futures::io::Cursor::new(bytes);

        let error = read_peer_handshake(&mut cursor, 8).await.unwrap_err();

        assert!(matches!(
            error,
            LifecycleProtocolError::Frame(FrameError::BodyTooLarge { .. })
        ));
    }
}

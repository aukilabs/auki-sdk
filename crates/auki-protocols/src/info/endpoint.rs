//! Portable [`auki_sdk::AukiPeer`] endpoint for participant information v1.

use std::{fmt, future::Future, time::Duration};

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AuthenticatedPeer, AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::{AsyncWriteExt, FutureExt, pin_mut};
use futures_timer::Delay;

use super::v1::{
    AuthenticatedParticipantInfo, ID, InfoProtocolError, MAX_INFO_FRAME_BYTES,
    read_authenticated_info_response, read_info_request, write_authenticated_info_response,
    write_info_request,
};

/// Maximum number of concurrently served participant-info requests.
pub const INFO_MAX_CONCURRENCY: usize = 16;

/// Fixed deadline for each open, exchange, and close operation.
pub const INFO_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(not(target_arch = "wasm32"))]
type ProviderHandle = Arc<dyn InfoProvider>;
#[cfg(target_arch = "wasm32")]
type ProviderHandle = Rc<dyn InfoProvider>;

/// Application-owned source of participant metadata.
///
/// The requester has already completed mutual DDS authentication. Returning
/// `None` declines the request by closing the stream without a response.
#[cfg(not(target_arch = "wasm32"))]
pub trait InfoProvider: Send + Sync + 'static {
    /// Sample the metadata visible to one authenticated requester.
    fn participant_info(
        &self,
        requester: &AuthenticatedPeer,
    ) -> Option<AuthenticatedParticipantInfo>;
}

/// Application-owned source of participant metadata.
///
/// Browser providers are local to the Wasm executor. The requester has already
/// completed mutual DDS authentication. Returning `None` declines the request.
#[cfg(target_arch = "wasm32")]
pub trait InfoProvider: 'static {
    /// Sample the metadata visible to one authenticated requester.
    fn participant_info(
        &self,
        requester: &AuthenticatedPeer,
    ) -> Option<AuthenticatedParticipantInfo>;
}

#[cfg(not(target_arch = "wasm32"))]
impl<F> InfoProvider for F
where
    F: Fn(&AuthenticatedPeer) -> Option<AuthenticatedParticipantInfo> + Send + Sync + 'static,
{
    fn participant_info(
        &self,
        requester: &AuthenticatedPeer,
    ) -> Option<AuthenticatedParticipantInfo> {
        self(requester)
    }
}

#[cfg(target_arch = "wasm32")]
impl<F> InfoProvider for F
where
    F: Fn(&AuthenticatedPeer) -> Option<AuthenticatedParticipantInfo> + 'static,
{
    fn participant_info(
        &self,
        requester: &AuthenticatedPeer,
    ) -> Option<AuthenticatedParticipantInfo> {
        self(requester)
    }
}

/// Build the exact bounded participant-info registration.
pub fn info_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(ID, INFO_MAX_CONCURRENCY, MAX_INFO_FRAME_BYTES)
}

/// Cloneable outbound participant-info client.
#[derive(Clone)]
pub struct InfoClient {
    protocols: AukiPeerProtocols,
}

impl InfoClient {
    /// Construct a client over one running peer's protocol surface.
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    /// Fetch metadata using routes configured on the owning native peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn fetch(
        &self,
        remote_peer_id: PeerId,
    ) -> Result<AuthenticatedParticipantInfo, InfoEndpointError> {
        fetch_opened(remote_peer_id, self.protocols.open(remote_peer_id, ID)).await
    }

    /// Fetch metadata through one exact advertised route.
    pub async fn fetch_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
    ) -> Result<AuthenticatedParticipantInfo, InfoEndpointError> {
        fetch_opened(
            remote_peer_id,
            self.protocols.open_exact(remote_peer_id, route, ID),
        )
        .await
    }
}

/// Mounted participant-info service plus its outbound client.
pub struct InfoEndpoint {
    client: InfoClient,
    registration: AukiProtocolRegistration,
}

impl InfoEndpoint {
    /// Mount participant-info v1 on one running Auki peer.
    pub fn mount<P>(protocols: AukiPeerProtocols, provider: P) -> Result<Self, InfoEndpointError>
    where
        P: InfoProvider,
    {
        let local_peer_id = protocols.peer_id();
        let provider = share_provider(provider);
        let registration = protocols.register(info_protocol_spec()?, move |mut stream| {
            let provider = clone_provider(&provider);
            async move {
                let requester = stream.remote_peer().clone();
                let exchange = deadline(InfoOperation::Exchange, async {
                    read_info_request(&mut stream).await?;
                    let info = provider
                        .participant_info(&requester)
                        .ok_or(InfoEndpointError::Declined)?;
                    ensure_peer_id(local_peer_id, &info)?;
                    write_authenticated_info_response(&mut stream, &info).await?;
                    Ok::<_, InfoEndpointError>(())
                })
                .await
                .and_then(|result| result);
                let cleanup = close_stream(&mut stream).await;
                let _ = prefer_primary(exchange, cleanup);
            }
        })?;

        Ok(Self {
            client: InfoClient::new(protocols),
            registration,
        })
    }

    /// Clone the outbound client without cloning registration ownership.
    pub fn client(&self) -> InfoClient {
        self.client.clone()
    }

    /// Stop accepting requests and await all admitted handlers.
    pub async fn close(self) -> Result<(), InfoEndpointError> {
        self.registration
            .close()
            .await
            .map_err(InfoEndpointError::Sdk)
    }
}

async fn fetch_opened<F>(
    expected_peer: PeerId,
    opening: F,
) -> Result<AuthenticatedParticipantInfo, InfoEndpointError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(InfoOperation::Open, opening)
        .await?
        .map_err(InfoEndpointError::Sdk)?;
    let exchange = deadline(InfoOperation::Exchange, async {
        write_info_request(&mut stream, &Default::default()).await?;
        let info = read_authenticated_info_response(&mut stream).await?;
        ensure_peer_id(expected_peer, &info)?;
        Ok::<_, InfoEndpointError>(info)
    })
    .await
    .and_then(|result| result);
    let cleanup = close_stream(&mut stream).await;
    prefer_primary(exchange, cleanup)
}

async fn close_stream<S>(stream: &mut S) -> Result<(), InfoEndpointError>
where
    S: AsyncWriteExt + Unpin,
{
    deadline(InfoOperation::Close, stream.close())
        .await?
        .map_err(|error| InfoEndpointError::Close(error.to_string()))
}

async fn deadline<T>(
    operation: InfoOperation,
    future: impl Future<Output = T>,
) -> Result<T, InfoEndpointError> {
    let work = future.fuse();
    let timer = Delay::new(INFO_OPERATION_TIMEOUT).fuse();
    pin_mut!(work, timer);
    futures::select_biased! {
        result = work => Ok(result),
        () = timer => Err(InfoEndpointError::Timeout(operation)),
    }
}

fn ensure_peer_id(
    expected_peer: PeerId,
    info: &AuthenticatedParticipantInfo,
) -> Result<(), InfoEndpointError> {
    if info.peer_id == expected_peer {
        Ok(())
    } else {
        Err(InfoEndpointError::PeerMismatch {
            expected: Box::new(expected_peer),
            actual: Box::new(info.peer_id),
        })
    }
}

fn prefer_primary<T, E>(primary: Result<T, E>, cleanup: Result<(), E>) -> Result<T, E> {
    match primary {
        Err(error) => Err(error),
        Ok(value) => cleanup.map(|()| value),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn share_provider<P: InfoProvider>(provider: P) -> ProviderHandle {
    Arc::new(provider)
}

#[cfg(target_arch = "wasm32")]
fn share_provider<P: InfoProvider>(provider: P) -> ProviderHandle {
    Rc::new(provider)
}

fn clone_provider(provider: &ProviderHandle) -> ProviderHandle {
    provider.clone()
}

/// One bounded participant-info endpoint operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InfoOperation {
    /// Open and authenticate a protocol stream.
    Open,
    /// Exchange the bounded request and response.
    Exchange,
    /// Close the authenticated stream.
    Close,
}

impl fmt::Display for InfoOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::Exchange => "exchange",
            Self::Close => "close",
        })
    }
}

/// Failure from the portable participant-info endpoint.
#[derive(Debug, thiserror::Error)]
pub enum InfoEndpointError {
    /// The SDK rejected protocol registration or stream opening.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    /// The participant-info wire conversation failed.
    #[error("participant-info codec failed: {0}")]
    Codec(#[from] InfoProtocolError),
    /// The provider declined to reveal metadata to this requester.
    #[error("participant-info provider declined the authenticated requester")]
    Declined,
    /// Served or received metadata did not match the authenticated identity.
    #[error("participant info Peer ID {actual} does not match authenticated peer {expected}")]
    PeerMismatch {
        /// Expected authenticated identity.
        expected: Box<PeerId>,
        /// Identity repeated by the metadata document.
        actual: Box<PeerId>,
    },
    /// One endpoint phase exceeded its fixed deadline.
    #[error("participant-info {0} timed out after 5 seconds")]
    Timeout(InfoOperation),
    /// Stream cleanup failed after the exchange.
    #[error("close participant-info stream: {0}")]
    Close(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_sdk::Identity;

    #[test]
    fn spec_mounts_the_exact_wire_contract() {
        let spec = info_protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), ID);
        assert_eq!(spec.max_concurrency(), INFO_MAX_CONCURRENCY);
        assert_eq!(spec.max_frame_bytes(), MAX_INFO_FRAME_BYTES);
    }

    #[test]
    fn repeated_identity_must_match_the_authenticated_peer() {
        let expected = Identity::generate().peer_id();
        let actual = Identity::generate().peer_id();
        let info = AuthenticatedParticipantInfo {
            app: "example".into(),
            app_version: "1.0.0".into(),
            name: "robot".into(),
            session_id: "session".into(),
            session_clock_id: "clock".into(),
            session_clock_hash: "hash".into(),
            session_now_ns: 1,
            peer_id: actual,
            app_instance: "instance".into(),
        };
        assert!(matches!(
            ensure_peer_id(expected, &info),
            Err(InfoEndpointError::PeerMismatch { .. })
        ));
    }
}

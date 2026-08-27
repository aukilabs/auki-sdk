use std::{sync::Arc, time::Duration};

use auki_p2p::PeerId;
use auki_protocols::info::v1::{
    AuthenticatedParticipantInfo, ID as INFO_V1_0_0, InfoProtocolError, MAX_INFO_FRAME_BYTES,
    read_authenticated_info_response, read_info_request, write_authenticated_info_response,
    write_info_request,
};
use parking_lot::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::{
    peers::{DomainPeerInfoError, DomainPeers},
    protocols::{
        DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
        DomainProtocols,
    },
};

const INFO_V1_MAX_CONCURRENCY: usize = 16;
const INFO_V1_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

pub trait ParticipantInfoProvider: Send + Sync + 'static {
    fn participant_info(&self) -> AuthenticatedParticipantInfo;
}

impl<F> ParticipantInfoProvider for F
where
    F: Fn() -> AuthenticatedParticipantInfo + Send + Sync + 'static,
{
    fn participant_info(&self) -> AuthenticatedParticipantInfo {
        self()
    }
}

#[derive(Clone)]
pub(crate) struct InfoV1 {
    local_peer_id: PeerId,
    protocols: DomainProtocols,
    peers: DomainPeers,
    lifecycle: CancellationToken,
    provider: Arc<Mutex<Option<Arc<dyn ParticipantInfoProvider>>>>,
}

impl InfoV1 {
    pub(super) fn new(
        local_peer_id: PeerId,
        protocols: DomainProtocols,
        peers: DomainPeers,
        lifecycle: CancellationToken,
    ) -> Self {
        Self {
            local_peer_id,
            protocols,
            peers,
            lifecycle,
            provider: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn register(&self) -> Result<DomainProtocolRegistration, InfoV1Error> {
        let spec =
            DomainProtocolSpec::new(INFO_V1_0_0, INFO_V1_MAX_CONCURRENCY, MAX_INFO_FRAME_BYTES)?;
        let info = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let info = info.clone();
                async move {
                    if let Err(error) = info.handle(stream).await {
                        tracing::warn!(%error, "authenticated participant info request failed");
                    }
                }
            })
            .map_err(InfoV1Error::Protocol)
    }

    pub(crate) fn set_provider(
        &self,
        provider: Arc<dyn ParticipantInfoProvider>,
    ) -> Result<(), InfoV1Error> {
        self.ensure_running()?;
        let mut current = self.provider.lock();
        self.ensure_running()?;
        *current = Some(provider);
        Ok(())
    }

    pub(crate) fn local(&self) -> Result<AuthenticatedParticipantInfo, InfoV1Error> {
        self.ensure_running()?;
        let provider = {
            let current = self.provider.lock();
            self.ensure_running()?;
            current.clone().ok_or(InfoV1Error::ProviderUnavailable)?
        };
        let info = provider.participant_info();
        ensure_peer_id(self.local_peer_id, &info)?;
        Ok(info)
    }

    pub(crate) async fn fetch(
        &self,
        expected_peer: PeerId,
    ) -> Result<AuthenticatedParticipantInfo, InfoV1Error> {
        self.ensure_running()?;
        let (info, authenticated_peer) = timeout(INFO_V1_EXCHANGE_TIMEOUT, async {
            let mut stream = self.protocols.open(expected_peer, INFO_V1_0_0).await?;
            let authenticated_peer = stream.remote_peer().clone();
            write_info_request(&mut stream, &Default::default()).await?;
            let info = read_authenticated_info_response(&mut stream).await?;
            ensure_peer_id(expected_peer, &info)?;
            Ok::<_, InfoV1Error>((info, authenticated_peer))
        })
        .await
        .map_err(|_| InfoV1Error::Timeout(INFO_V1_EXCHANGE_TIMEOUT))??;
        self.peers
            .refresh_participant_info(expected_peer, &authenticated_peer, info.clone())?;
        Ok(info)
    }

    async fn handle(&self, mut stream: DomainProtocolStream) -> Result<(), InfoV1Error> {
        timeout(INFO_V1_EXCHANGE_TIMEOUT, async {
            read_info_request(&mut stream).await?;
            let info = self.local()?;
            write_authenticated_info_response(&mut stream, &info).await?;
            Ok(())
        })
        .await
        .map_err(|_| InfoV1Error::Timeout(INFO_V1_EXCHANGE_TIMEOUT))?
    }

    fn ensure_running(&self) -> Result<(), InfoV1Error> {
        if self.lifecycle.is_cancelled() {
            Err(InfoV1Error::Stopped)
        } else {
            Ok(())
        }
    }
}

fn ensure_peer_id(
    expected_peer: PeerId,
    info: &AuthenticatedParticipantInfo,
) -> Result<(), InfoV1Error> {
    if info.peer_id == expected_peer {
        Ok(())
    } else {
        Err(InfoV1Error::PeerMismatch {
            expected: Box::new(expected_peer),
            actual: Box::new(info.peer_id),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InfoV1Error {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("no participant info provider is installed")]
    ProviderUnavailable,
    #[error("participant info Peer ID {actual} does not match authenticated peer {expected}")]
    PeerMismatch {
        expected: Box<PeerId>,
        actual: Box<PeerId>,
    },
    #[error("participant info protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("participant info codec failed: {0}")]
    Codec(#[from] InfoProtocolError),
    #[error("participant observation update failed: {0}")]
    Observation(#[from] DomainPeerInfoError),
    #[error("participant info exchange exceeded {0:?}")]
    Timeout(Duration),
}

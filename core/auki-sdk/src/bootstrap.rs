//! User/App authentication and peer startup composition.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use auki_auth::{
    AuthClient, AuthEnvironment, AuthSession, Credentials, DomainChoice, DomainSelection,
};

use crate::{
    AukiPeer, AukiPeerConfig, AukiPeerStartError, DdsTrackerConfig, DdsTrackerMode, Identity,
};

/// Authenticated User/App session paired with one peer runtime configuration.
///
/// This is the ordinary application bootstrap path. It keeps authentication,
/// explicit Domain selection, identity proof, authority preparation, and
/// [`AukiPeer`] startup in Rust so platform bindings only adapt values and
/// ownership. It does not choose a Domain, mount protocols, or discover peers.
#[derive(Clone)]
pub struct AukiPeerBootstrap {
    auth: AuthSession,
    peer_config: AukiPeerConfig,
}

impl AukiPeerBootstrap {
    /// Compose an already-owned session without network work. Keep a clone of
    /// the session for persistence recovery and close-before-clear logout.
    pub fn from_session(auth: AuthSession, peer_config: AukiPeerConfig) -> Self {
        Self { auth, peer_config }
    }

    pub fn session(&self) -> &AuthSession {
        &self.auth
    }

    /// Authenticate a User or trusted native App and retain the configuration
    /// used for every peer started from this session.
    pub async fn authenticate(
        client: AuthClient,
        credentials: Credentials,
        peer_config: AukiPeerConfig,
    ) -> Result<Self, AukiPeerBootstrapError> {
        let auth = client
            .authenticate(credentials)
            .await
            .map_err(AukiPeerBootstrapError::Authenticate)?;
        Ok(Self { auth, peer_config })
    }

    /// Authenticate against the shared development API, DDS, and DMS.
    pub async fn dev(credentials: Credentials) -> Result<Self, AukiPeerBootstrapError> {
        let client = AuthClient::new(AuthEnvironment::dev())
            .map_err(AukiPeerBootstrapError::ConfigureAuthentication)?;
        Self::authenticate(client, credentials, AukiPeerConfig::dev()).await
    }

    /// Enable DDS discovery using the same validated DDS origin as this auth session.
    ///
    /// The mode is explicit because observing other peers and publishing this
    /// peer's reachability are separate product choices.
    pub fn with_dds_tracker(mut self, mode: DdsTrackerMode) -> Self {
        let tracker = DdsTrackerConfig::new(self.auth.dds_base_url(), mode)
            .expect("an authenticated session always retains a validated DDS origin");
        self.peer_config = self.peer_config.with_dds_tracker(tracker);
        self
    }

    /// Disable the local DMS relay booking for peers started from this clone.
    ///
    /// The authenticated session remains reusable through other clones. A
    /// browser peer started from this configuration can dial relay-backed
    /// peers but has no public route of its own, so it may discover but cannot
    /// advertise through DDS.
    pub fn without_relay(mut self) -> Self {
        self.peer_config = self.peer_config.without_relay();
        self
    }

    /// List the Domains the authenticated principal may explicitly select.
    ///
    /// Applications only need this operation when presenting a choice. Peer
    /// authorization checks the selected Domain again during every start.
    pub async fn accessible_domains(&self) -> Result<Vec<DomainChoice>, AukiPeerBootstrapError> {
        self.auth
            .accessible_domains()
            .await
            .map_err(AukiPeerBootstrapError::ListDomains)
    }

    /// Authorize `identity` for the explicitly selected Domain and start it.
    ///
    /// The caller owns identity persistence policy. Native applications can use
    /// `start_persistent_peer`; use [`Self::start_ephemeral_peer`] when a
    /// deliberately session-scoped Peer ID is appropriate.
    pub async fn start_peer(
        &self,
        selection: DomainSelection,
        identity: Identity,
    ) -> Result<AukiPeer, AukiPeerBootstrapError> {
        let prepared = self
            .auth
            .authorize_peer(selection, &identity.proof())
            .await
            .map_err(AukiPeerBootstrapError::AuthorizePeer)?;
        AukiPeer::start(identity, prepared, self.peer_config.clone())
            .await
            .map_err(AukiPeerBootstrapError::StartPeer)
    }

    /// Generate a new in-memory identity and start it in the selected Domain.
    ///
    /// Browser peers use this intentionally. Native production applications
    /// normally use `start_persistent_peer` instead.
    pub async fn start_ephemeral_peer(
        &self,
        selection: DomainSelection,
    ) -> Result<AukiPeer, AukiPeerBootstrapError> {
        self.start_peer(selection, Identity::generate()).await
    }

    /// Load or atomically create one persistent native identity, then start it
    /// in the selected Domain.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn start_persistent_peer(
        &self,
        selection: DomainSelection,
        identity_file: impl AsRef<Path>,
    ) -> Result<AukiPeer, AukiPeerBootstrapError> {
        let identity = Identity::load_or_create(identity_file)
            .map_err(AukiPeerBootstrapError::LoadIdentity)?;
        self.start_peer(selection, identity).await
    }
}

/// Failure while composing User/App authentication and peer startup.
#[derive(Debug, thiserror::Error)]
pub enum AukiPeerBootstrapError {
    /// Development authentication client construction failed.
    #[error("configure Auki authentication: {0}")]
    ConfigureAuthentication(#[source] auki_auth::Error),
    /// User/App authentication failed.
    #[error("authenticate Auki principal: {0}")]
    Authenticate(#[source] auki_auth::Error),
    /// Accessible-Domain lookup failed.
    #[error("list accessible Domains: {0}")]
    ListDomains(#[source] auki_auth::Error),
    /// Selected-Domain and identity authorization failed.
    #[error("authorize Auki peer: {0}")]
    AuthorizePeer(#[source] auki_auth::Error),
    /// Native persistent identity loading or creation failed.
    #[cfg(not(target_arch = "wasm32"))]
    #[error("load or create Auki peer identity: {0}")]
    LoadIdentity(#[source] auki_p2p::Error),
    /// Authenticated peer runtime startup failed.
    #[error("start Auki peer: {0}")]
    StartPeer(#[source] AukiPeerStartError),
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod zitadel_tests {
    use super::*;
    use auki_auth::{Error, ZitadelSessionCredentials, ZitadelSessionStore};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Store;
    #[async_trait::async_trait]
    impl ZitadelSessionStore for Store {
        async fn save(&self, _: &ZitadelSessionCredentials) -> auki_auth::Result<()> {
            panic!("unknown-expiry service failure must not rotate or save");
        }
    }

    #[tokio::test]
    async fn imported_handle_survives_bootstrap_failure_and_closes_all_clones() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).await.unwrap();
            let request = std::str::from_utf8(&request[..n]).unwrap();
            assert!(request.starts_with("POST /api/v1/domains/"));
            assert!(request.contains("/p2p/zitadel/challenge HTTP/1.1"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer opaque")
            );
            stream
                .write_all(
                    b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let client = AuthClient::new(AuthEnvironment::new(&base, &base).unwrap()).unwrap();
        let credentials = ZitadelSessionCredentials::new(
            "opaque",
            "refresh",
            "client",
            base.parse().unwrap(),
            None,
        )
        .unwrap();
        let session = client
            .import_zitadel_session(credentials, Arc::new(Store))
            .unwrap();
        let bootstrap =
            AukiPeerBootstrap::from_session(session.clone(), AukiPeerConfig::dev().without_relay());
        let result = bootstrap
            .start_peer(uuid::Uuid::new_v4().into(), Identity::generate())
            .await;
        assert!(matches!(
            result,
            Err(AukiPeerBootstrapError::AuthorizePeer(Error::HttpStatus {
                status: 503,
                ..
            }))
        ));
        server.await.unwrap();
        session.close().await;
        assert!(matches!(
            bootstrap.session().accessible_domains().await,
            Err(Error::SessionClosed)
        ));
    }
}

use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    net::IpAddr,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use auki_p2p::{
    ApplicationProtocol, CANDIDATE_ROUTE_MAX_BYTES, Multiaddr, PeerId, canonicalize_candidate_route,
};
use chrono::{DateTime, Utc};
use futures::{FutureExt, StreamExt, future::Either, pin_mut};
use futures_timer::Delay;
use reqwest::{
    Client, Method, StatusCode, Url,
    header::{ACCEPT, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use auki_p2p::RouteCatalog;
#[cfg(not(target_arch = "wasm32"))]
use tokio::{sync::watch, task::JoinHandle};
#[cfg(not(target_arch = "wasm32"))]
use tokio_util::sync::CancellationToken;
#[cfg(not(target_arch = "wasm32"))]
use tracing::warn;
#[cfg(target_arch = "wasm32")]
use {
    futures::{channel::oneshot, future::Shared},
    tokio_util::sync::CancellationToken,
    wasm_bindgen_futures::spawn_local,
};

#[cfg(not(target_arch = "wasm32"))]
use crate::{authority::AuthorityStatus, protocols::AukiPeerProtocols};

/// DDS HTTP base used by [`DdsTrackerConfig::dev`].
pub const DEV_DDS_BASE_URL: &str = "https://dds.dev.aukiverse.com/";

pub(crate) const DDS_ADVERTISEMENT_RENEWAL_INTERVAL: Duration = Duration::from_secs(60);
const DDS_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
// RouteCatalog counts one relay slot as one logical route, while a tracker
// advertisement expands that slot to its required TCP/WSS pair. Keep this
// wire bound above the maximum current 13-direct + 3-pair (19) payload.
const MAX_ROUTES: usize = 32;
const MAX_ROUTE_BYTES: usize = CANDIDATE_ROUTE_MAX_BYTES;
const MAX_PROTOCOLS: usize = 64;
const MAX_PROTOCOL_BYTES: usize = 255;
const PAGE_LIMIT: usize = 100;
const MAX_PAGES: usize = 100;
const MAX_CANDIDATES: usize = PAGE_LIMIT * MAX_PAGES;
const MAX_CURSOR_BYTES: usize = 2_048;

/// Explicit DDS tracker behavior for one peer.
///
/// There is intentionally no default: applications must decide whether the
/// peer only observes advertisements or also publishes its own reachability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DdsTrackerMode {
    /// Perform fresh lookups only. Startup and shutdown make no tracker calls.
    DiscoverOnly,
    /// Perform lookups and maintain one short-lived self advertisement.
    DiscoverAndAdvertise,
}

/// Validated configuration for the DDS discovery tracker.
#[derive(Clone, Debug)]
pub struct DdsTrackerConfig {
    base_url: Url,
    mode: DdsTrackerMode,
}

impl DdsTrackerConfig {
    /// Select an exact, explicitly trusted DDS base and tracker mode.
    ///
    /// The peer's renewable DDS P2P bearer is sent to this origin. Prefer
    /// [`AukiPeerBootstrap::with_dds_tracker`](crate::AukiPeerBootstrap::with_dds_tracker),
    /// which pins discovery to the same DDS used for authentication. Use this
    /// lower-level constructor only when the application already trusts and
    /// controls the custom DDS endpoint. Production endpoints require HTTPS;
    /// HTTP is accepted only for localhost or a literal loopback address.
    pub fn for_trusted_dds(
        base_url: impl AsRef<str>,
        mode: DdsTrackerMode,
    ) -> Result<Self, AukiDiscoveryError> {
        Self::new(base_url.as_ref(), mode)
    }

    pub(crate) fn new(
        base_url: impl AsRef<str>,
        mode: DdsTrackerMode,
    ) -> Result<Self, AukiDiscoveryError> {
        Ok(Self {
            base_url: parse_dds_base_url(base_url.as_ref())?,
            mode,
        })
    }

    /// Select the shared development DDS and one explicit mode.
    pub fn dev(mode: DdsTrackerMode) -> Self {
        Self::new(DEV_DDS_BASE_URL, mode).expect("the built-in development DDS URL is valid")
    }

    /// Validated DDS base, including an optional caller-supplied path prefix.
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// Explicit lookup/publication behavior.
    pub fn mode(&self) -> DdsTrackerMode {
        self.mode
    }

    pub(crate) fn base(&self) -> &Url {
        &self.base_url
    }
}

/// Local provenance attached to one discovery observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AukiDiscoverySource {
    /// Short-lived advertisement read from the configured DDS tracker.
    DdsTracker,
}

/// One bounded, untrusted candidate returned by discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AukiDiscoveryCandidate {
    peer_id: PeerId,
    routes: Vec<Multiaddr>,
    served_protocols: Vec<String>,
    expires_at: DateTime<Utc>,
    source: AukiDiscoverySource,
}

/// Cloneable Rust-owned discovery capability for one configured peer.
///
/// Platform bindings retain this handle so a lookup never has to borrow the
/// peer owner across an async boundary. It exposes no credentials or
/// publication policy; those remain private to the peer runtime.
#[derive(Clone)]
pub struct AukiDiscovery {
    inner: DdsDiscovery,
}

impl AukiDiscovery {
    pub(crate) fn new(inner: DdsDiscovery) -> Self {
        Self { inner }
    }

    /// Fetch a fresh bounded list of untrusted same-Domain dial candidates.
    pub async fn discover(&self) -> Result<Vec<AukiDiscoveryCandidate>, AukiDiscoveryError> {
        self.inner.discover().await
    }

    /// Fetch fresh candidates advertising one exact inbound protocol ID.
    pub async fn discover_protocol(
        &self,
        protocol_id: impl AsRef<str>,
    ) -> Result<Vec<AukiDiscoveryCandidate>, AukiDiscoveryError> {
        self.inner.discover_protocol(protocol_id.as_ref()).await
    }
}

impl AukiDiscoveryCandidate {
    /// Expected remote Peer ID. Exact dialing still verifies it cryptographically.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Canonical, untrusted direct and relay route hints.
    pub fn routes(&self) -> &[Multiaddr] {
        &self.routes
    }

    /// Exact inbound protocol IDs self-advertised by the remote peer.
    pub fn served_protocols(&self) -> &[String] {
        &self.served_protocols
    }

    /// DDS-assigned lease expiration.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Provider that produced this process-local observation.
    pub fn source(&self) -> AukiDiscoverySource {
        self.source
    }
}

/// Bounded discovery configuration, authentication, transport, or response failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AukiDiscoveryError {
    /// No provider was explicitly selected for the peer.
    #[error("DDS discovery is not configured for this Auki peer")]
    Disabled,
    /// The configured DDS origin violates the SDK transport policy.
    #[error(
        "DDS base must be an HTTPS URL (or HTTP with localhost/a literal loopback host) without credentials, query, or fragment"
    )]
    InvalidConfiguration,
    /// The requested filter is not one exact bounded application protocol ID.
    #[error("discovery protocol filter is invalid")]
    InvalidProtocol,
    /// No current renewable P2P bearer was available.
    #[error("DDS discovery authentication is unavailable")]
    Authentication,
    /// The bounded DDS request did not complete in time.
    #[error("DDS discovery request timed out")]
    RequestTimedOut,
    /// The DDS request could not be sent or read.
    #[error("DDS discovery transport failed")]
    Transport,
    /// The browser transport or its owner stopped before initial publication
    /// reached the startup barrier.
    #[error("browser peer runtime stopped while its DDS advertisement was starting")]
    RuntimeStoppedDuringAdvertisementStartup,
    /// DDS returned a status outside the frozen tracker contract.
    #[error("DDS discovery {operation} returned HTTP {status}")]
    HttpStatus {
        /// Redacted operation name.
        operation: &'static str,
        /// Numeric HTTP response status.
        status: u16,
    },
    /// DDS returned a response outside the frozen bounded contract.
    #[error("DDS discovery {operation} returned an invalid response: {reason}")]
    InvalidResponse {
        /// Redacted operation name.
        operation: &'static str,
        /// Bounded static diagnostic without response content.
        reason: &'static str,
    },
}

#[derive(Clone)]
pub(crate) struct DdsAuthorizationSnapshot {
    header: HeaderValue,
    revision: u64,
}

impl DdsAuthorizationSnapshot {
    pub(crate) fn new(header: HeaderValue, revision: u64) -> Self {
        Self { header, revision }
    }

    fn revision(&self) -> u64 {
        self.revision
    }
}

impl fmt::Debug for DdsAuthorizationSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DdsAuthorizationSnapshot")
            .field("header", &"[redacted]")
            .field("revision", &self.revision)
            .finish()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub(crate) trait DdsAuthorizationProvider: Send + Sync {
    async fn authorization(&self) -> Result<DdsAuthorizationSnapshot, AukiDiscoveryError>;

    async fn refresh_after_unauthorized(
        &self,
        rejected_revision: u64,
    ) -> Result<(), AukiDiscoveryError>;
}

#[derive(Clone)]
pub(crate) struct DdsTrackerClient {
    base: Url,
    http: Client,
    auth: Arc<dyn DdsAuthorizationProvider>,
    request_timeout: Duration,
}

impl DdsTrackerClient {
    pub(crate) fn new(
        config: &DdsTrackerConfig,
        auth: Arc<dyn DdsAuthorizationProvider>,
    ) -> Result<Self, AukiDiscoveryError> {
        let builder = Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder
            .no_proxy()
            .connect_timeout(DDS_REQUEST_TIMEOUT)
            .timeout(DDS_REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none());
        let http = builder
            .build()
            .map_err(|_| AukiDiscoveryError::InvalidConfiguration)?;
        Ok(Self {
            base: config.base().clone(),
            http,
            auth,
            request_timeout: DDS_REQUEST_TIMEOUT,
        })
    }

    pub(crate) async fn publish(
        &self,
        domain_id: Uuid,
        peer_id: PeerId,
        routes: Vec<Multiaddr>,
        protocols: Vec<String>,
    ) -> Result<AukiDiscoveryCandidate, AukiDiscoveryError> {
        let routes = canonicalize_routes(peer_id, routes, "publish")?;
        if routes.is_empty() {
            return Err(invalid_response(
                "publish",
                "advertisement must contain at least one route",
            ));
        }
        let protocols = canonicalize_protocols(protocols, "publish")?;
        let body = serde_json::to_vec(&PublishRequest {
            routes: routes.iter().map(ToString::to_string).collect(),
            protocols: protocols.clone(),
        })
        .map_err(|_| invalid_response("publish", "request encoding failed"))?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(invalid_response(
                "publish",
                "encoded advertisement exceeds the request limit",
            ));
        }
        let url = self.self_endpoint(domain_id)?;
        let raw = self
            .send("publish", Method::PUT, url, Some(body), StatusCode::OK)
            .await?;
        let wire: WireAdvertisement = decode_json("publish", &raw)?;
        let candidate = validate_advertisement(wire, "publish")?
            .ok_or_else(|| invalid_response("publish", "DDS returned an expired lease"))?;
        if candidate.peer_id != peer_id
            || candidate.routes != routes
            || candidate.served_protocols != protocols
        {
            return Err(invalid_response(
                "publish",
                "DDS changed the self advertisement identity or values",
            ));
        }
        Ok(candidate)
    }

    pub(crate) async fn discover(
        &self,
        domain_id: Uuid,
        local_peer_id: PeerId,
        protocol: Option<&str>,
    ) -> Result<Vec<AukiDiscoveryCandidate>, AukiDiscoveryError> {
        let protocol = protocol.map(validate_filter).transpose()?;
        let mut candidates = BTreeMap::<String, AukiDiscoveryCandidate>::new();
        let mut cursors = HashSet::new();
        let mut cursor: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let url =
                self.collection_endpoint(domain_id, protocol.as_deref(), cursor.as_deref())?;
            let raw = self
                .send("list", Method::GET, url, None, StatusCode::OK)
                .await?;
            let page: ListResponse = decode_json("list", &raw)?;
            if page.advertisements.len() > PAGE_LIMIT {
                return Err(invalid_response(
                    "list",
                    "page exceeds the requested candidate limit",
                ));
            }
            for wire in page.advertisements {
                let Some(candidate) = validate_advertisement(wire, "list")? else {
                    continue;
                };
                if candidate.peer_id == local_peer_id {
                    continue;
                }
                if protocol.as_ref().is_some_and(|expected| {
                    !candidate
                        .served_protocols
                        .iter()
                        .any(|actual| actual == expected)
                }) {
                    return Err(invalid_response(
                        "list",
                        "protocol-filtered page contains a non-matching candidate",
                    ));
                }
                if candidates
                    .insert(candidate.peer_id.to_string(), candidate)
                    .is_some()
                {
                    return Err(invalid_response(
                        "list",
                        "candidate Peer ID is repeated across pages",
                    ));
                }
                if candidates.len() > MAX_CANDIDATES {
                    return Err(invalid_response(
                        "list",
                        "aggregate candidate limit exceeded",
                    ));
                }
            }

            let Some(next) = page.next_cursor else {
                return Ok(candidates.into_values().collect());
            };
            if next.is_empty() || next.len() > MAX_CURSOR_BYTES || !cursors.insert(next.clone()) {
                return Err(invalid_response("list", "invalid or repeated page cursor"));
            }
            cursor = Some(next);
        }
        Err(invalid_response("list", "page limit exceeded"))
    }

    pub(crate) async fn withdraw(&self, domain_id: Uuid) -> Result<(), AukiDiscoveryError> {
        let url = self.self_endpoint(domain_id)?;
        let raw = self
            .send(
                "withdraw",
                Method::DELETE,
                url,
                None,
                StatusCode::NO_CONTENT,
            )
            .await?;
        if !raw.is_empty() {
            return Err(invalid_response(
                "withdraw",
                "204 response must not contain a body",
            ));
        }
        Ok(())
    }

    fn self_endpoint(&self, domain_id: Uuid) -> Result<Url, AukiDiscoveryError> {
        self.endpoint(&[
            "api",
            "v1",
            "domains",
            &domain_id.to_string(),
            "p2p",
            "advertisements",
            "self",
        ])
    }

    fn collection_endpoint(
        &self,
        domain_id: Uuid,
        protocol: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Url, AukiDiscoveryError> {
        let mut url = self.endpoint(&[
            "api",
            "v1",
            "domains",
            &domain_id.to_string(),
            "p2p",
            "advertisements",
        ])?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &PAGE_LIMIT.to_string());
            if let Some(protocol) = protocol {
                query.append_pair("protocol", protocol);
            }
            if let Some(cursor) = cursor {
                query.append_pair("cursor", cursor);
            }
        }
        Ok(url)
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, AukiDiscoveryError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| AukiDiscoveryError::InvalidConfiguration)?
            .pop_if_empty()
            .extend(segments.iter().copied());
        if url.query().is_some() || url.fragment().is_some() {
            return Err(AukiDiscoveryError::InvalidConfiguration);
        }
        Ok(url)
    }

    async fn send(
        &self,
        operation: &'static str,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        expected_status: StatusCode,
    ) -> Result<Vec<u8>, AukiDiscoveryError> {
        let operation_future = async {
            for attempt in 0..=1 {
                let authorization = self.auth.authorization().await?;
                if !valid_sensitive_bearer_header(&authorization.header) {
                    return Err(AukiDiscoveryError::Authentication);
                }
                let mut request = self
                    .http
                    .request(method.clone(), url.clone())
                    .header(ACCEPT, "application/json")
                    .header(CACHE_CONTROL, "no-store")
                    .header(AUTHORIZATION, authorization.header.clone());
                if let Some(body) = body.clone() {
                    request = request.header(CONTENT_TYPE, "application/json").body(body);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|_| AukiDiscoveryError::Transport)?;
                if response.url() != &url {
                    return Err(invalid_response(operation, "response URL changed"));
                }
                if response.status() == StatusCode::UNAUTHORIZED && attempt == 0 {
                    self.auth
                        .refresh_after_unauthorized(authorization.revision())
                        .await?;
                    continue;
                }
                if response.status() != expected_status {
                    return Err(AukiDiscoveryError::HttpStatus {
                        operation,
                        status: response.status().as_u16(),
                    });
                }
                if !response_is_non_cacheable(&response) {
                    return Err(invalid_response(
                        operation,
                        "Cache-Control must contain no-store",
                    ));
                }
                if expected_status != StatusCode::NO_CONTENT && !response_is_json(&response) {
                    return Err(invalid_response(
                        operation,
                        "Content-Type must be application/json",
                    ));
                }
                if response
                    .content_length()
                    .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
                {
                    return Err(invalid_response(operation, "response body is too large"));
                }
                let mut bytes = Vec::new();
                let mut chunks = response.bytes_stream();
                while let Some(chunk) = chunks.next().await {
                    let chunk = chunk.map_err(|_| AukiDiscoveryError::Transport)?;
                    if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                        return Err(invalid_response(operation, "response body is too large"));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                return Ok(bytes);
            }
            unreachable!("the bounded authorization retry loop always returns")
        }
        .fuse();
        let timeout = Delay::new(self.request_timeout).fuse();
        pin_mut!(operation_future, timeout);
        match futures::future::select(operation_future, timeout).await {
            Either::Left((result, _)) => result,
            Either::Right(((), _)) => Err(AukiDiscoveryError::RequestTimedOut),
        }
    }
}

/// Cloneable lookup capability retained by one configured peer runtime.
#[derive(Clone)]
pub(crate) struct DdsDiscovery {
    client: DdsTrackerClient,
    domain_id: Uuid,
    local_peer_id: PeerId,
}

impl DdsDiscovery {
    pub(crate) fn new(
        config: &DdsTrackerConfig,
        domain_id: Uuid,
        local_peer_id: PeerId,
        auth: Arc<dyn DdsAuthorizationProvider>,
    ) -> Result<Self, AukiDiscoveryError> {
        Ok(Self {
            client: DdsTrackerClient::new(config, auth)?,
            domain_id,
            local_peer_id,
        })
    }

    pub(crate) async fn discover(&self) -> Result<Vec<AukiDiscoveryCandidate>, AukiDiscoveryError> {
        self.client
            .discover(self.domain_id, self.local_peer_id, None)
            .await
    }

    pub(crate) async fn discover_protocol(
        &self,
        protocol_id: &str,
    ) -> Result<Vec<AukiDiscoveryCandidate>, AukiDiscoveryError> {
        self.client
            .discover(self.domain_id, self.local_peer_id, Some(protocol_id))
            .await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn start_native_publisher(
        &self,
        routes: RouteCatalog,
        protocols: AukiPeerProtocols,
        authority: watch::Receiver<AuthorityStatus>,
    ) -> Result<NativeDdsPublisher, AukiDiscoveryError> {
        NativeDdsPublisher::start(self.clone(), routes, protocols, authority).await
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn start_browser_publisher(
        &self,
        routes: Vec<Multiaddr>,
        protocols: crate::browser_protocols::AukiPeerProtocols,
        cancellation: CancellationToken,
        cleanup_complete: CancellationToken,
    ) -> Result<BrowserDdsPublisher, AukiDiscoveryError> {
        BrowserDdsPublisher::start(
            self.clone(),
            routes,
            protocols,
            cancellation,
            cleanup_complete,
        )
        .await
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct NativeDdsPublisher {
    cancellation: CancellationToken,
    task: Option<JoinHandle<Result<(), AukiDiscoveryError>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeDdsPublisher {
    async fn start(
        discovery: DdsDiscovery,
        routes: RouteCatalog,
        protocols: AukiPeerProtocols,
        authority: watch::Receiver<AuthorityStatus>,
    ) -> Result<Self, AukiDiscoveryError> {
        let route_updates = routes.subscribe();
        let protocol_updates = protocols.subscribe_served_protocols();
        let initial_routes = native_routes(&routes)?;
        let initial_protocols = protocol_updates.borrow().protocol_ids.clone();
        if let Err(error) =
            publish_with_retry(&discovery, initial_routes, initial_protocols, None).await
        {
            // A timed-out PUT can still have reached DDS. Withdraw defensively
            // before returning startup ownership to the caller.
            if let Err(withdrawal) = discovery.client.withdraw(discovery.domain_id).await {
                warn!(error = %withdrawal, "native DDS startup compensation failed; lease will expire");
            }
            return Err(error);
        }

        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            maintain_native_advertisement(
                discovery,
                routes,
                route_updates,
                protocol_updates,
                authority,
                task_cancellation,
            )
            .await
        });
        Ok(Self {
            cancellation,
            task: Some(task),
        })
    }

    /// Stop future publication, attempt one bounded withdrawal, and join the task.
    pub(crate) async fn shutdown(mut self) -> Result<(), AukiDiscoveryError> {
        self.cancellation.cancel();
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.await.map_err(|_| {
            invalid_response(
                "withdraw",
                "publisher task stopped before its cleanup barrier",
            )
        })?
    }

    /// Stop background publication when the owning runtime fails or is dropped.
    pub(crate) fn abort(&self) {
        self.cancellation.cancel();
    }

    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for NativeDdsPublisher {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn maintain_native_advertisement(
    discovery: DdsDiscovery,
    routes: RouteCatalog,
    mut route_updates: watch::Receiver<auki_p2p::RouteCatalogStatus>,
    mut protocol_updates: watch::Receiver<crate::served_protocols::ServedProtocolSnapshot>,
    mut authority: watch::Receiver<AuthorityStatus>,
    cancellation: CancellationToken,
) -> Result<(), AukiDiscoveryError> {
    let mut renewal = tokio::time::interval(DDS_ADVERTISEMENT_RENEWAL_INTERVAL);
    renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The initial advertisement already fulfilled the immediate first tick.
    renewal.tick().await;

    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return discovery.client.withdraw(discovery.domain_id).await;
            }
            _ = renewal.tick() => {}
            changed = route_updates.changed() => {
                if changed.is_err() {
                    return discovery.client.withdraw(discovery.domain_id).await;
                }
            }
            changed = protocol_updates.changed() => {
                if changed.is_err() {
                    return discovery.client.withdraw(discovery.domain_id).await;
                }
            }
            changed = authority.changed() => {
                if changed.is_err() || matches!(*authority.borrow(), AuthorityStatus::Stopped) {
                    return discovery.client.withdraw(discovery.domain_id).await;
                }
            }
        }

        let current_routes = match native_routes(&routes) {
            Ok(routes) => routes,
            Err(error) => {
                warn!(error = %error, "DDS advertisement route snapshot failed");
                continue;
            }
        };
        if current_routes.is_empty() {
            if let Err(error) = discovery.client.withdraw(discovery.domain_id).await {
                warn!(error = %error, "DDS advertisement withdrawal after route loss failed");
            }
            continue;
        }
        let current_protocols = protocol_updates.borrow().protocol_ids.clone();
        if let Err(error) = publish_with_retry(
            &discovery,
            current_routes,
            current_protocols,
            Some(&cancellation),
        )
        .await
        {
            if cancellation.is_cancelled() {
                return discovery.client.withdraw(discovery.domain_id).await;
            }
            warn!(error = %error, "DDS advertisement update failed; lease will retry or expire");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn native_routes(routes: &RouteCatalog) -> Result<Vec<Multiaddr>, AukiDiscoveryError> {
    let snapshot = routes
        .snapshot()
        .map_err(|_| invalid_response("publish", "local route snapshot is invalid"))?;
    let mut values = snapshot.direct_routes;
    for relay in snapshot.relay_routes {
        values.push(relay.routes.tcp().clone());
        values.push(relay.routes.wss().clone());
    }
    values.sort_unstable_by_key(ToString::to_string);
    values.dedup();
    Ok(values)
}

async fn publish_with_retry(
    discovery: &DdsDiscovery,
    routes: Vec<Multiaddr>,
    protocols: Vec<String>,
    cancellation: Option<&CancellationToken>,
) -> Result<(), AukiDiscoveryError> {
    let mut delay = Duration::from_secs(1);
    for attempt in 0..3 {
        match discovery
            .client
            .publish(
                discovery.domain_id,
                discovery.local_peer_id,
                routes.clone(),
                protocols.clone(),
            )
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) if attempt < 2 && retryable(&error) => {
                match cancellation {
                    Some(cancellation) => {
                        let cancelled = cancellation.cancelled().fuse();
                        let sleep = Delay::new(delay).fuse();
                        pin_mut!(cancelled, sleep);
                        futures::select_biased! {
                            () = cancelled => return Err(error),
                            () = sleep => {}
                        }
                    }
                    None => Delay::new(delay).await,
                }
                delay = delay.saturating_mul(2);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded publication retry loop always returns")
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct BrowserDdsPublisher {
    cancellation: CancellationToken,
    stopped: Shared<oneshot::Receiver<Result<(), AukiDiscoveryError>>>,
}

#[cfg(target_arch = "wasm32")]
impl BrowserDdsPublisher {
    async fn start(
        discovery: DdsDiscovery,
        routes: Vec<Multiaddr>,
        protocols: crate::browser_protocols::AukiPeerProtocols,
        cancellation: CancellationToken,
        cleanup_complete: CancellationToken,
    ) -> Result<Self, AukiDiscoveryError> {
        let protocol_updates = protocols.subscribe_served_protocols();
        let task_cancellation = cancellation.clone();
        let start_cancellation = cancellation.clone();
        let (stopped_sender, stopped_receiver) = oneshot::channel();
        let (started_sender, started_receiver) = oneshot::channel();
        spawn_local(async move {
            let result = start_and_maintain_browser_advertisement(
                discovery,
                routes,
                protocol_updates,
                task_cancellation,
                started_sender,
            )
            .await;
            cleanup_complete.cancel();
            let _ = stopped_sender.send(result);
        });

        // Initial publication runs in its own local task. If the owner drops
        // this startup future, cancellation still makes that task finish the
        // bounded request and then issue the compensating DELETE.
        let mut startup_guard = BrowserPublisherStartupGuard::new(start_cancellation);
        let started = started_receiver.await.map_err(|_| {
            invalid_response(
                "publish",
                "browser publisher stopped before its startup barrier",
            )
        })?;
        started?;
        startup_guard.disarm();

        Ok(Self {
            cancellation,
            stopped: stopped_receiver.shared(),
        })
    }

    pub(crate) async fn shutdown(&self) -> Result<(), AukiDiscoveryError> {
        self.cancellation.cancel();
        self.stopped.clone().await.map_err(|_| {
            invalid_response(
                "withdraw",
                "browser publisher stopped before its cleanup barrier",
            )
        })?
    }

    pub(crate) fn abort(&self) {
        self.cancellation.cancel();
    }
}

#[cfg(target_arch = "wasm32")]
struct BrowserPublisherStartupGuard {
    cancellation: CancellationToken,
    armed: bool,
}

#[cfg(target_arch = "wasm32")]
impl BrowserPublisherStartupGuard {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for BrowserPublisherStartupGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for BrowserDdsPublisher {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[cfg(target_arch = "wasm32")]
async fn maintain_browser_advertisement(
    discovery: DdsDiscovery,
    routes: Vec<Multiaddr>,
    mut protocols: tokio::sync::watch::Receiver<crate::served_protocols::ServedProtocolSnapshot>,
    cancellation: CancellationToken,
) -> Result<(), AukiDiscoveryError> {
    loop {
        let should_stop = {
            let cancelled = cancellation.cancelled().fuse();
            let changed = protocols.changed().fuse();
            let renewal = Delay::new(DDS_ADVERTISEMENT_RENEWAL_INTERVAL).fuse();
            pin_mut!(cancelled, changed, renewal);
            futures::select_biased! {
                () = cancelled => true,
                result = changed => result.is_err(),
                () = renewal => false,
            }
        };
        if should_stop {
            return discovery.client.withdraw(discovery.domain_id).await;
        }

        if let Err(error) = publish_with_retry(
            &discovery,
            routes.clone(),
            protocols.borrow().protocol_ids.clone(),
            Some(&cancellation),
        )
        .await
        {
            if cancellation.is_cancelled() {
                return discovery.client.withdraw(discovery.domain_id).await;
            }
            tracing::warn!(error = %error, "browser DDS advertisement update failed; lease will retry or expire");
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn start_and_maintain_browser_advertisement(
    discovery: DdsDiscovery,
    routes: Vec<Multiaddr>,
    protocols: tokio::sync::watch::Receiver<crate::served_protocols::ServedProtocolSnapshot>,
    cancellation: CancellationToken,
    started: oneshot::Sender<Result<(), AukiDiscoveryError>>,
) -> Result<(), AukiDiscoveryError> {
    if cancellation.is_cancelled() {
        let error = AukiDiscoveryError::RuntimeStoppedDuringAdvertisementStartup;
        let _ = started.send(Err(error.clone()));
        return Err(error);
    }

    let initial = publish_with_retry(
        &discovery,
        routes.clone(),
        protocols.borrow().protocol_ids.clone(),
        Some(&cancellation),
    )
    .await;

    if let Err(error) = initial {
        // A timed-out PUT can still have reached DDS. Make one bounded
        // compensating DELETE while relay supervision keeps authority alive.
        if let Err(withdrawal) = discovery.client.withdraw(discovery.domain_id).await {
            tracing::warn!(error = %withdrawal, "browser DDS startup compensation failed; lease will expire");
        }
        let _ = started.send(Err(error.clone()));
        return Err(error);
    }

    if cancellation.is_cancelled() {
        let cleanup = discovery.client.withdraw(discovery.domain_id).await;
        let error = AukiDiscoveryError::RuntimeStoppedDuringAdvertisementStartup;
        let _ = started.send(Err(error.clone()));
        return cleanup.and(Err(error));
    }

    // If the startup owner vanished between publication and this barrier,
    // there is nobody to own the advertisement. Withdraw it immediately.
    if started.send(Ok(())).is_err() {
        cancellation.cancel();
        return discovery.client.withdraw(discovery.domain_id).await;
    }

    maintain_browser_advertisement(discovery, routes, protocols, cancellation).await
}

fn retryable(error: &AukiDiscoveryError) -> bool {
    matches!(
        error,
        AukiDiscoveryError::Authentication
            | AukiDiscoveryError::RequestTimedOut
            | AukiDiscoveryError::Transport
            | AukiDiscoveryError::HttpStatus {
                status: 429 | 500..=599,
                ..
            }
    )
}

#[derive(Serialize)]
struct PublishRequest {
    routes: Vec<String>,
    protocols: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAdvertisement {
    peer_id: String,
    routes: Vec<String>,
    protocols: Vec<String>,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListResponse {
    advertisements: Vec<WireAdvertisement>,
    #[serde(default)]
    next_cursor: Option<String>,
}

fn validate_advertisement(
    wire: WireAdvertisement,
    operation: &'static str,
) -> Result<Option<AukiDiscoveryCandidate>, AukiDiscoveryError> {
    let peer_id = PeerId::from_str(&wire.peer_id)
        .map_err(|_| invalid_response(operation, "candidate Peer ID is invalid"))?;
    let routes = wire
        .routes
        .into_iter()
        .map(|value| {
            if value.len() > MAX_ROUTE_BYTES {
                return Err(invalid_response(operation, "candidate route is too large"));
            }
            Multiaddr::from_str(&value)
                .map_err(|_| invalid_response(operation, "candidate route is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let routes = canonicalize_routes(peer_id, routes, operation)?;
    if routes.is_empty() {
        return Err(invalid_response(
            operation,
            "candidate must contain at least one route",
        ));
    }
    let served_protocols = canonicalize_protocols(wire.protocols, operation)?;
    if wire.expires_at <= Utc::now() {
        return Ok(None);
    }
    Ok(Some(AukiDiscoveryCandidate {
        peer_id,
        routes,
        served_protocols,
        expires_at: wire.expires_at,
        source: AukiDiscoverySource::DdsTracker,
    }))
}

fn canonicalize_routes(
    expected_peer: PeerId,
    routes: Vec<Multiaddr>,
    operation: &'static str,
) -> Result<Vec<Multiaddr>, AukiDiscoveryError> {
    if routes.len() > MAX_ROUTES {
        return Err(invalid_response(operation, "route limit exceeded"));
    }
    let mut canonical = routes
        .into_iter()
        .map(|route| {
            if route.len() > MAX_ROUTE_BYTES {
                return Err(invalid_response(operation, "route is too large"));
            }
            canonicalize_candidate_route(&route, expected_peer)
                .map(|candidate| candidate.into_route())
                .map_err(|_| invalid_response(operation, "route grammar is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    canonical.sort_unstable_by_key(ToString::to_string);
    canonical.dedup();
    Ok(canonical)
}

fn canonicalize_protocols(
    protocols: Vec<String>,
    operation: &'static str,
) -> Result<Vec<String>, AukiDiscoveryError> {
    if protocols.len() > MAX_PROTOCOLS {
        return Err(invalid_response(operation, "protocol limit exceeded"));
    }
    let mut protocols = protocols
        .into_iter()
        .map(|value| validate_protocol(&value, operation))
        .collect::<Result<Vec<_>, _>>()?;
    protocols.sort_unstable();
    protocols.dedup();
    Ok(protocols)
}

fn validate_protocol(value: &str, operation: &'static str) -> Result<String, AukiDiscoveryError> {
    if value.len() > MAX_PROTOCOL_BYTES || ApplicationProtocol::new(value.to_owned()).is_err() {
        return Err(invalid_response(operation, "protocol ID is invalid"));
    }
    Ok(value.to_owned())
}

fn validate_filter(value: &str) -> Result<String, AukiDiscoveryError> {
    if value.len() > MAX_PROTOCOL_BYTES || ApplicationProtocol::new(value.to_owned()).is_err() {
        return Err(AukiDiscoveryError::InvalidProtocol);
    }
    Ok(value.to_owned())
}

fn decode_json<T: DeserializeOwned>(
    operation: &'static str,
    bytes: &[u8],
) -> Result<T, AukiDiscoveryError> {
    serde_json::from_slice(bytes)
        .map_err(|_| invalid_response(operation, "JSON does not match the tracker contract"))
}

fn response_is_json(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn response_is_non_cacheable(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get_all(CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|directive| directive.trim().eq_ignore_ascii_case("no-store"))
}

fn valid_sensitive_bearer_header(header: &HeaderValue) -> bool {
    header.is_sensitive()
        && header
            .to_str()
            .is_ok_and(|value| value.starts_with("Bearer ") && value.len() > "Bearer ".len())
}

fn parse_dds_base_url(value: &str) -> Result<Url, AukiDiscoveryError> {
    let url = Url::parse(value).map_err(|_| AukiDiscoveryError::InvalidConfiguration)?;
    let authority = value
        .split_once("//")
        .map(|(_, remainder)| remainder)
        .and_then(|remainder| remainder.split(['/', '?', '#']).next())
        .unwrap_or_default();
    let http_loopback = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    if url.cannot_be_a_base()
        || url.host_str().is_none()
        || !(url.scheme() == "https" || http_loopback)
        || authority.contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AukiDiscoveryError::InvalidConfiguration);
    }
    Ok(url)
}

fn invalid_response(operation: &'static str, reason: &'static str) -> AukiDiscoveryError {
    AukiDiscoveryError::InvalidResponse { operation, reason }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use httpmock::{Method::DELETE, Method::GET, Method::PUT, MockServer};
    use reqwest::header::HeaderValue;
    use serde_json::json;

    use super::*;

    const INFO: &str = "/auki/auth/1/info/1.0.0";
    const MESSAGE: &str = "/auki/auth/1/message/0.1.0";
    const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

    struct RotatingAuth {
        state: Mutex<(String, u64, usize)>,
    }

    impl RotatingAuth {
        fn new(token: &str) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new((token.to_owned(), 1, 0)),
            })
        }

        fn refreshes(&self) -> usize {
            self.state.lock().unwrap().2
        }
    }

    #[async_trait]
    impl DdsAuthorizationProvider for RotatingAuth {
        async fn authorization(&self) -> Result<DdsAuthorizationSnapshot, AukiDiscoveryError> {
            let state = self.state.lock().unwrap();
            let mut header = HeaderValue::from_str(&format!("Bearer {}", state.0)).unwrap();
            header.set_sensitive(true);
            Ok(DdsAuthorizationSnapshot::new(header, state.1))
        }

        async fn refresh_after_unauthorized(
            &self,
            rejected_revision: u64,
        ) -> Result<(), AukiDiscoveryError> {
            let mut state = self.state.lock().unwrap();
            if state.1 == rejected_revision {
                state.0 = "fresh-token".into();
                state.1 += 1;
                state.2 += 1;
            }
            Ok(())
        }
    }

    struct HangingAuth;

    #[async_trait]
    impl DdsAuthorizationProvider for HangingAuth {
        async fn authorization(&self) -> Result<DdsAuthorizationSnapshot, AukiDiscoveryError> {
            std::future::pending().await
        }

        async fn refresh_after_unauthorized(
            &self,
            _rejected_revision: u64,
        ) -> Result<(), AukiDiscoveryError> {
            std::future::pending().await
        }
    }

    fn peer() -> PeerId {
        auki_p2p::Identity::generate().peer_id()
    }

    fn direct(port: u16) -> Multiaddr {
        format!("/ip4/127.0.0.1/tcp/{port}").parse().unwrap()
    }

    fn client(server: &MockServer, auth: Arc<RotatingAuth>) -> DdsTrackerClient {
        DdsTrackerClient::new(
            &DdsTrackerConfig::new(server.base_url(), DdsTrackerMode::DiscoverOnly).unwrap(),
            auth,
        )
        .unwrap()
    }

    fn advertisement(peer_id: PeerId, port: u16, protocols: &[&str]) -> serde_json::Value {
        json!({
            "peer_id": peer_id.to_string(),
            "routes": [direct(port).to_string()],
            "protocols": protocols,
            "expires_at": Utc::now() + chrono::Duration::minutes(2),
        })
    }

    #[test]
    fn tracker_configuration_is_explicit_and_bounded() {
        assert_eq!(
            DdsTrackerConfig::dev(DdsTrackerMode::DiscoverOnly).mode(),
            DdsTrackerMode::DiscoverOnly
        );
        assert!(
            DdsTrackerConfig::for_trusted_dds(
                "http://127.0.0.1:8080/prefix/",
                DdsTrackerMode::DiscoverAndAdvertise
            )
            .is_ok()
        );
        assert!(
            DdsTrackerConfig::for_trusted_dds(
                "http://localhost:8080/prefix/",
                DdsTrackerMode::DiscoverOnly
            )
            .is_ok()
        );
        for invalid in [
            "not a URL",
            "http://dds.example/",
            "https://user@dds.example/",
            "https://dds.example/?secret=yes",
        ] {
            assert_eq!(
                DdsTrackerConfig::for_trusted_dds(invalid, DdsTrackerMode::DiscoverOnly)
                    .unwrap_err(),
                AukiDiscoveryError::InvalidConfiguration,
                "{invalid}"
            );
        }
    }

    #[tokio::test]
    async fn publish_and_withdraw_use_the_self_owned_contract() {
        let server = MockServer::start();
        let domain_id = Uuid::new_v4();
        let peer_id = peer();
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements/self");
        let publish = server.mock(|when, then| {
            when.method(PUT)
                .path(path.clone())
                .header("authorization", "Bearer original-token")
                .header("cache-control", "no-store")
                .json_body(json!({
                    "routes": [direct(4001).to_string()],
                    "protocols": [INFO, MESSAGE],
                }));
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(advertisement(peer_id, 4001, &[INFO, MESSAGE]));
        });
        let withdraw = server.mock(|when, then| {
            when.method(DELETE)
                .path(path)
                .header("authorization", "Bearer original-token");
            then.status(204).header("cache-control", "no-store");
        });
        let tracker = client(&server, RotatingAuth::new("original-token"));

        let published = tracker
            .publish(
                domain_id,
                peer_id,
                vec![direct(4001)],
                vec![MESSAGE.into(), INFO.into()],
            )
            .await
            .unwrap();
        assert_eq!(published.peer_id(), peer_id);
        assert_eq!(published.served_protocols(), [INFO, MESSAGE]);
        tracker.withdraw(domain_id).await.unwrap();
        publish.assert_calls(1);
        withdraw.assert_calls(1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn route_change_during_initial_publish_is_republished_without_waiting_for_renewal() {
        let server = MockServer::start();
        let domain_id = Uuid::new_v4();
        let identity = auki_p2p::Identity::generate();
        let peer_id = identity.peer_id();
        let initial_route = direct(4001);
        let replacement_route = direct(4002);
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements/self");
        let initial = server.mock(|when, then| {
            when.method(PUT).path(path.clone()).json_body(json!({
                "routes": [initial_route.to_string()],
                "protocols": [],
            }));
            then.delay(Duration::from_millis(100))
                .status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(advertisement(peer_id, 4001, &[]));
        });
        let replacement = server.mock(|when, then| {
            when.method(PUT).path(path.clone()).json_body(json!({
                "routes": [replacement_route.to_string()],
                "protocols": [],
            }));
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(advertisement(peer_id, 4002, &[]));
        });
        let withdraw = server.mock(|when, then| {
            when.method(DELETE).path(path);
            then.status(204).header("cache-control", "no-store");
        });

        let tracker =
            DdsTrackerConfig::new(server.base_url(), DdsTrackerMode::DiscoverAndAdvertise).unwrap();
        let discovery = DdsDiscovery::new(
            &tracker,
            domain_id,
            peer_id,
            RotatingAuth::new("original-token"),
        )
        .unwrap();
        let routes = RouteCatalog::new(
            peer_id,
            vec![initial_route],
            auki_p2p::RouteCatalogLimits::new(16, 3),
        )
        .unwrap();
        let verifier = auki_p2p::DdsTokenVerifier::from_es256_pem(TEST_DDS_PUBLIC_KEY).unwrap();
        let node =
            auki_p2p::Node::start(identity, verifier, std::iter::empty::<Multiaddr>()).unwrap();
        let protocols = crate::AukiPeerProtocols::new(
            node.clone(),
            domain_id,
            std::iter::empty(),
            crate::context::ContextLifecycle::new(),
        );
        let (_, authority) = watch::channel(AuthorityStatus::Ready {
            credential_revision: 1,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        });

        let starting = tokio::spawn(NativeDdsPublisher::start(
            discovery,
            routes.clone(),
            protocols,
            authority,
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while initial.calls() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("initial publication must enter the server");
        routes
            .replace_direct_routes(vec![replacement_route])
            .unwrap();

        let publisher = starting.await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while replacement.calls() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the pre-subscribed route update must publish immediately");

        publisher.shutdown().await.unwrap();
        node.shutdown().await.unwrap();
        initial.assert_calls(1);
        replacement.assert_calls(1);
        withdraw.assert_calls(1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publication_recovers_after_a_transient_tracker_failure() {
        let server = MockServer::start();
        let domain_id = Uuid::new_v4();
        let peer_id = peer();
        let route = direct(4001);
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements/self");
        let mut unavailable = server.mock(|when, then| {
            when.method(PUT).path(path.clone());
            then.status(503).header("cache-control", "no-store");
        });
        let tracker =
            DdsTrackerConfig::new(server.base_url(), DdsTrackerMode::DiscoverAndAdvertise).unwrap();
        let discovery = DdsDiscovery::new(
            &tracker,
            domain_id,
            peer_id,
            RotatingAuth::new("original-token"),
        )
        .unwrap();
        let publishing = tokio::spawn({
            let discovery = discovery.clone();
            let route = route.clone();
            async move { publish_with_retry(&discovery, vec![route], Vec::new(), None).await }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while unavailable.calls() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first publication attempt must reach DDS");
        unavailable.assert_calls(1);
        unavailable.delete();
        let recovered = server.mock(|when, then| {
            when.method(PUT).path(path).json_body(json!({
                "routes": [route.to_string()],
                "protocols": [],
            }));
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(advertisement(peer_id, 4001, &[]));
        });

        publishing.await.unwrap().unwrap();
        recovered.assert_calls(1);
    }

    #[tokio::test]
    async fn paged_exact_filter_is_fresh_sorted_and_self_excluding() {
        let server = MockServer::start();
        let domain_id = Uuid::new_v4();
        let local = peer();
        let remote_a = peer();
        let remote_b = peer();
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements");
        let first = server.mock(|when, then| {
            when.method(GET)
                .path(path.clone())
                .query_param("limit", "100")
                .query_param("protocol", INFO)
                .query_param_missing("cursor")
                .header("authorization", "Bearer token");
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "advertisements": [
                        advertisement(local, 4000, &[INFO]),
                        advertisement(remote_b, 4002, &[INFO]),
                    ],
                    "next_cursor": "opaque-next",
                }));
        });
        let second = server.mock(|when, then| {
            when.method(GET)
                .path(path)
                .query_param("limit", "100")
                .query_param("protocol", INFO)
                .query_param("cursor", "opaque-next")
                .header("authorization", "Bearer token");
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "advertisements": [advertisement(remote_a, 4001, &[INFO])]
                }));
        });

        let found = client(&server, RotatingAuth::new("token"))
            .discover(domain_id, local, Some(INFO))
            .await
            .unwrap();
        assert_eq!(found.len(), 2);
        assert!(
            found
                .windows(2)
                .all(|pair| { pair[0].peer_id().to_string() < pair[1].peer_id().to_string() })
        );
        assert!(found.iter().all(|candidate| candidate.peer_id() != local));
        assert!(found.iter().all(|candidate| {
            candidate.served_protocols().iter().any(|id| id == INFO)
                && candidate.source() == AukiDiscoverySource::DdsTracker
        }));
        first.assert_calls(1);
        second.assert_calls(1);
    }

    #[tokio::test]
    async fn one_unauthorized_response_refreshes_without_exposing_the_token() {
        let server = MockServer::start();
        let domain_id = Uuid::new_v4();
        let local = peer();
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements");
        let rejected = server.mock(|when, then| {
            when.method(GET)
                .path(path.clone())
                .header("authorization", "Bearer stale-token");
            then.status(401).header("cache-control", "no-store");
        });
        let accepted = server.mock(|when, then| {
            when.method(GET)
                .path(path)
                .header("authorization", "Bearer fresh-token");
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({"advertisements": []}));
        });
        let auth = RotatingAuth::new("stale-token");
        let tracker = client(&server, Arc::clone(&auth));

        assert!(
            tracker
                .discover(domain_id, local, None)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(auth.refreshes(), 1);
        rejected.assert_calls(1);
        accepted.assert_calls(1);
    }

    #[tokio::test]
    async fn request_deadline_includes_authorization_work() {
        let server = MockServer::start();
        let request = server.mock(|when, then| {
            when.method(GET);
            then.status(500);
        });
        let mut tracker = DdsTrackerClient::new(
            &DdsTrackerConfig::new(server.base_url(), DdsTrackerMode::DiscoverOnly).unwrap(),
            Arc::new(HangingAuth),
        )
        .unwrap();
        tracker.request_timeout = Duration::from_millis(10);

        assert_eq!(
            tracker
                .discover(Uuid::new_v4(), peer(), None)
                .await
                .unwrap_err(),
            AukiDiscoveryError::RequestTimedOut
        );
        request.assert_calls(0);
    }

    #[tokio::test]
    async fn repeated_peer_across_pages_fails_closed() {
        let server = MockServer::start();
        let domain_id = Uuid::new_v4();
        let local = peer();
        let repeated = peer();
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements");
        server.mock(|when, then| {
            when.method(GET)
                .path(path.clone())
                .query_param_missing("cursor");
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "advertisements": [advertisement(repeated, 4001, &[INFO])],
                    "next_cursor": "next",
                }));
        });
        server.mock(|when, then| {
            when.method(GET).path(path).query_param("cursor", "next");
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "advertisements": [advertisement(repeated, 4001, &[INFO])]
                }));
        });

        assert!(matches!(
            client(&server, RotatingAuth::new("token"))
                .discover(domain_id, local, None)
                .await
                .unwrap_err(),
            AukiDiscoveryError::InvalidResponse { .. }
        ));
    }

    #[tokio::test]
    async fn malformed_or_non_matching_candidates_fail_the_complete_lookup() {
        let server = MockServer::start();
        let domain_id = Uuid::new_v4();
        let local = peer();
        let remote = peer();
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements");
        server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "advertisements": [advertisement(remote, 4001, &[MESSAGE])]
                }));
        });

        let error = client(&server, RotatingAuth::new("token"))
            .discover(domain_id, local, Some(INFO))
            .await
            .unwrap_err();
        assert!(matches!(error, AukiDiscoveryError::InvalidResponse { .. }));
        assert_eq!(
            client(&server, RotatingAuth::new("token"))
                .discover(domain_id, local, Some("not/a/protocol"))
                .await
                .unwrap_err(),
            AukiDiscoveryError::InvalidProtocol
        );
    }

    #[test]
    fn route_validation_accepts_the_native_and_browser_relay_pair() {
        let target = peer();
        let relay = peer();
        let tcp: Multiaddr =
            format!("/dns4/relay.example.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}")
                .parse()
                .unwrap();
        let wss: Multiaddr =
            format!("/dns4/relay.example.com/tcp/4443/wss/p2p/{relay}/p2p-circuit/p2p/{target}")
                .parse()
                .unwrap();
        assert_eq!(
            canonicalize_candidate_route(&tcp, target)
                .unwrap()
                .into_route(),
            tcp
        );
        assert_eq!(
            canonicalize_candidate_route(&wss, target)
                .unwrap()
                .into_route(),
            wss
        );
        assert!(canonicalize_candidate_route(&wss, peer()).is_err());
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn abandoned_startup_cancels_the_background_publisher() {
        let cancellation = CancellationToken::new();
        {
            let _guard = BrowserPublisherStartupGuard::new(cancellation.clone());
        }
        assert!(cancellation.is_cancelled());
    }

    #[wasm_bindgen_test]
    fn completed_startup_disarms_its_cancellation_guard() {
        let cancellation = CancellationToken::new();
        {
            let mut guard = BrowserPublisherStartupGuard::new(cancellation.clone());
            guard.disarm();
        }
        assert!(!cancellation.is_cancelled());
    }
}

use std::{
    collections::HashSet,
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use auki_domain::ServedProtocols;
use auki_p2p::{Multiaddr, PeerId, Protocol, canonicalize_circuit_route, validate_direct_route};
use reqwest::Url;

/// DMS HTTP base used by [`AukiPeerConfig::dev`].
pub const DEV_DMS_BASE_URL: &str = "https://dms.dev.aukiverse.com/v1/";

const MAX_LISTEN_ADDRESSES: usize = 16;
const MAX_ADVERTISED_DIRECT_ROUTES: usize = 16;
const MAX_LOCAL_ROUTES: usize = 16;
const MAX_INITIAL_ROUTE_PEERS: usize = 1_024;
const MAX_INITIAL_ROUTES_PER_PEER: usize = 16;
const MAX_INITIAL_ROUTE_INPUT_PER_PEER: usize = 4_096;
const MAX_INITIAL_ROUTES: usize = 4_096;
const MAX_MULTIADDR_BYTES: usize = 1_024;
const MIN_RELAY_COUNT: u8 = 1;
const MAX_RELAY_COUNT: u8 = 3;
const MIN_RELAY_DURATION: Duration = Duration::from_secs(300);
const MAX_RELAY_DURATION: Duration = Duration::from_secs(86_400);
const MIN_RELAY_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RELAY_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// DMS relay pool selected for this peer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AukiRelayMode {
    /// Shared relay capacity suitable for ordinary peers.
    #[default]
    Public,
    /// Dedicated relay capacity provisioned for the requester.
    Dedicated,
}

/// Small public relay-allocation policy.
///
/// Retry budgets, HTTP timeouts, renewal margins, and reconciliation details
/// remain SDK-owned mechanics. These four values preserve the deployment
/// choices applications already need to make.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AukiRelayConfig {
    /// Shared or dedicated relay allocation.
    pub mode: AukiRelayMode,
    /// Number of relay slots requested from DMS, in `1..=3`.
    pub relay_count: u8,
    /// Requested booking duration, in whole seconds from 300 through 86,400.
    pub requested_duration: Duration,
    /// DMS booking-status poll interval, from one through 60 seconds.
    pub status_poll_interval: Duration,
}

impl Default for AukiRelayConfig {
    fn default() -> Self {
        Self {
            mode: AukiRelayMode::Public,
            relay_count: 1,
            requested_duration: MAX_RELAY_DURATION,
            status_poll_interval: Duration::from_secs(5),
        }
    }
}

impl AukiRelayConfig {
    /// Construct and validate one relay-allocation policy.
    pub fn new(
        mode: AukiRelayMode,
        relay_count: u8,
        requested_duration: Duration,
        status_poll_interval: Duration,
    ) -> Result<Self, AukiRelayConfigError> {
        let config = Self {
            mode,
            relay_count,
            requested_duration,
            status_poll_interval,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(self) -> Result<(), AukiRelayConfigError> {
        if !(MIN_RELAY_COUNT..=MAX_RELAY_COUNT).contains(&self.relay_count) {
            return Err(AukiRelayConfigError::RelayCount {
                minimum: MIN_RELAY_COUNT,
                maximum: MAX_RELAY_COUNT,
            });
        }
        if self.requested_duration.subsec_nanos() != 0
            || !(MIN_RELAY_DURATION..=MAX_RELAY_DURATION).contains(&self.requested_duration)
        {
            return Err(AukiRelayConfigError::RequestedDuration {
                minimum_seconds: MIN_RELAY_DURATION.as_secs(),
                maximum_seconds: MAX_RELAY_DURATION.as_secs(),
            });
        }
        if self.status_poll_interval.subsec_nanos() != 0
            || !(MIN_RELAY_POLL_INTERVAL..=MAX_RELAY_POLL_INTERVAL)
                .contains(&self.status_poll_interval)
        {
            return Err(AukiRelayConfigError::StatusPollInterval {
                minimum_seconds: MIN_RELAY_POLL_INTERVAL.as_secs(),
                maximum_seconds: MAX_RELAY_POLL_INTERVAL.as_secs(),
            });
        }
        Ok(())
    }
}

/// Rejected public relay-allocation policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AukiRelayConfigError {
    /// Relay count is outside the DMS contract.
    #[error("relay count must be in {minimum}..={maximum}")]
    RelayCount {
        /// Inclusive lower bound.
        minimum: u8,
        /// Inclusive upper bound.
        maximum: u8,
    },
    /// Booking duration is not a whole number of seconds inside its bounds.
    #[error(
        "relay booking duration must be a whole number of seconds in {minimum_seconds}..={maximum_seconds}"
    )]
    RequestedDuration {
        /// Inclusive lower bound in seconds.
        minimum_seconds: u64,
        /// Inclusive upper bound in seconds.
        maximum_seconds: u64,
    },
    /// Status polling interval is not a whole number of seconds inside its bounds.
    #[error(
        "relay status poll interval must be a whole number of seconds in {minimum_seconds}..={maximum_seconds}"
    )]
    StatusPollInterval {
        /// Inclusive lower bound in seconds.
        minimum_seconds: u64,
        /// Inclusive upper bound in seconds.
        maximum_seconds: u64,
    },
}

/// One remote peer and its initial, untrusted route hints.
///
/// Route hints only tell the transport where to dial. The expected Peer ID and
/// selected Domain are still verified by the authenticated protocol layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialPeerRoutes {
    peer_id: PeerId,
    routes: Vec<Multiaddr>,
}

impl InitialPeerRoutes {
    /// Peer that every route is required to reach.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Canonical routes, with direct routes ordered before relay routes.
    pub fn routes(&self) -> &[Multiaddr] {
        &self.routes
    }
}

/// Mechanical configuration for one authenticated peer runtime.
///
/// The runtime creates and owns its `auki-session` Peer and Session from
/// `app_id` and `storage_root`. It exposes those data APIs without exposing the
/// underlying networking Domain or node.
#[derive(Clone, Debug)]
pub struct AukiPeerConfig {
    dms_base_url: Url,
    app_id: String,
    storage_root: PathBuf,
    listen_addresses: Vec<Multiaddr>,
    advertised_direct_routes: Vec<Multiaddr>,
    initial_peer_routes: Vec<InitialPeerRoutes>,
    served_protocols: ServedProtocols,
    relay: Option<AukiRelayConfig>,
}

impl AukiPeerConfig {
    /// Configure one peer against an exact DMS HTTP base.
    ///
    /// Production endpoints must use HTTPS. Plain HTTP is accepted only when
    /// the URL host is a literal loopback IP address. Credentials, queries,
    /// and fragments are never accepted.
    pub fn new(
        dms_base_url: impl AsRef<str>,
        app_id: impl Into<String>,
        storage_root: impl Into<PathBuf>,
    ) -> Result<Self, AukiPeerConfigError> {
        let dms_base_url = parse_dms_base_url(dms_base_url.as_ref())?;
        Ok(Self {
            dms_base_url,
            app_id: app_id.into(),
            storage_root: storage_root.into(),
            listen_addresses: Vec::new(),
            advertised_direct_routes: Vec::new(),
            initial_peer_routes: Vec::new(),
            served_protocols: ServedProtocols::none(),
            relay: Some(AukiRelayConfig::default()),
        })
    }

    /// Configure one peer against the shared development DMS.
    pub fn dev(app_id: impl Into<String>, storage_root: impl Into<PathBuf>) -> Self {
        Self::new(DEV_DMS_BASE_URL, app_id, storage_root)
            .expect("the built-in development DMS URL is valid")
    }

    /// Replace the addresses on which the local P2P transport will listen.
    ///
    /// Zero listeners is valid. The runtime reports the concrete addresses
    /// that actually bound; a requested TCP port may therefore be zero.
    pub fn with_listen_addresses(
        mut self,
        addresses: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<Self, AukiPeerConfigError> {
        self.listen_addresses =
            bounded_sorted_addresses(addresses, MAX_LISTEN_ADDRESSES, AddressSet::Listeners)?;
        Ok(self)
    }

    /// Replace local direct routes advertised to other peers.
    ///
    /// These are publication addresses, not listeners. They must use the
    /// exact `ip|dns/tcp[/p2p]` grammar and a non-zero port. If a terminal Peer
    /// ID is supplied, the runtime also verifies it matches its local identity.
    pub fn with_advertised_direct_routes(
        mut self,
        routes: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<Self, AukiPeerConfigError> {
        let routes = canonicalize_advertised_routes(routes)?;
        validate_local_route_capacity(routes.len(), self.relay)?;
        self.advertised_direct_routes = routes;
        Ok(self)
    }

    /// Replace the initial route hints for one expected remote peer.
    ///
    /// Supplying no routes removes that peer. Repeating the method for the
    /// same peer replaces its prior hints atomically in the configuration.
    pub fn with_peer_routes(
        mut self,
        peer_id: PeerId,
        routes: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<Self, AukiPeerConfigError> {
        let routes = canonicalize_peer_routes(peer_id, routes)?;
        let existing = self
            .initial_peer_routes
            .iter()
            .position(|entry| entry.peer_id == peer_id);
        let old_count = existing.map_or(0, |index| self.initial_peer_routes[index].routes.len());
        let resulting_peer_count = self.initial_peer_routes.len()
            + usize::from(existing.is_none() && !routes.is_empty())
            - usize::from(existing.is_some() && routes.is_empty());
        if resulting_peer_count > MAX_INITIAL_ROUTE_PEERS {
            return Err(AukiPeerConfigError::InitialPeerLimit {
                maximum: MAX_INITIAL_ROUTE_PEERS,
            });
        }
        let resulting_route_count = self
            .initial_peer_routes
            .iter()
            .map(|entry| entry.routes.len())
            .sum::<usize>()
            .saturating_sub(old_count)
            .saturating_add(routes.len());
        if resulting_route_count > MAX_INITIAL_ROUTES {
            return Err(AukiPeerConfigError::InitialRouteLimit {
                maximum: MAX_INITIAL_ROUTES,
            });
        }

        match (existing, routes.is_empty()) {
            (Some(index), true) => {
                self.initial_peer_routes.remove(index);
            }
            (Some(index), false) => self.initial_peer_routes[index].routes = routes,
            (None, false) => self
                .initial_peer_routes
                .push(InitialPeerRoutes { peer_id, routes }),
            (None, true) => {}
        }
        self.initial_peer_routes
            .sort_unstable_by_key(|entry| entry.peer_id.to_string());
        Ok(self)
    }

    /// Select the exact application protocol versions served inbound.
    ///
    /// The default is [`ServedProtocols::none`]. Client-side protocol opens do
    /// not require enabling the corresponding inbound server.
    pub fn with_served_protocols(mut self, protocols: ServedProtocols) -> Self {
        self.served_protocols = protocols;
        self
    }

    /// Disable DMS relay allocation for this peer.
    ///
    /// Relay-backed reachability is required by default. This is the sole
    /// opt-out and guarantees the runtime makes no relay-booking DMS calls.
    /// Call it before configuring a direct-route set that uses capacity
    /// otherwise reserved for the default relay.
    pub fn direct_only(mut self) -> Self {
        self.relay = None;
        self
    }

    /// Require relay-backed reachability using an explicit validated policy.
    ///
    /// This also re-enables relay allocation after [`Self::direct_only`].
    pub fn with_relay(mut self, relay: AukiRelayConfig) -> Result<Self, AukiPeerConfigError> {
        relay.validate()?;
        validate_local_route_capacity(self.advertised_direct_routes.len(), Some(relay))?;
        self.relay = Some(relay);
        Ok(self)
    }

    /// Validated DMS base, including any caller-supplied path prefix.
    pub fn dms_base_url(&self) -> &str {
        self.dms_base_url.as_str()
    }

    /// Application identifier used by the runtime-owned SDK Peer.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// Storage root used by the runtime-owned SDK Peer and Session.
    pub fn storage_root(&self) -> &Path {
        &self.storage_root
    }

    /// Requested local listener addresses.
    pub fn listen_addresses(&self) -> &[Multiaddr] {
        &self.listen_addresses
    }

    /// Local direct routes eligible for advertisement.
    pub fn advertised_direct_routes(&self) -> &[Multiaddr] {
        &self.advertised_direct_routes
    }

    /// Initial remote route hints in stable Peer-ID order.
    pub fn initial_peer_routes(&self) -> &[InitialPeerRoutes] {
        &self.initial_peer_routes
    }

    /// Exact inbound application protocols selected by the caller.
    pub fn served_protocols(&self) -> ServedProtocols {
        self.served_protocols
    }

    /// Whether startup requires at least one confirmed relay route.
    pub fn relay_required(&self) -> bool {
        self.relay.is_some()
    }

    /// Relay policy required at startup, or `None` for direct-only operation.
    pub fn relay(&self) -> Option<AukiRelayConfig> {
        self.relay
    }

    #[allow(dead_code)] // Consumed by the AukiPeer runtime in the next slice.
    pub(crate) fn dms_base(&self) -> &Url {
        &self.dms_base_url
    }
}

/// Rejected peer runtime configuration.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AukiPeerConfigError {
    /// DMS URL is malformed or violates the transport policy.
    #[error(
        "DMS base must be an HTTPS URL (or HTTP with a literal loopback host) without credentials, query, or fragment"
    )]
    InvalidDmsBaseUrl,
    /// The configured direct and required relay routes cannot coexist.
    #[error(
        "{direct_routes} direct routes plus {relay_routes} relay routes exceed the {maximum}-route local publication limit"
    )]
    LocalRouteLimit {
        /// Canonical direct routes selected by the caller.
        direct_routes: usize,
        /// Relay slots required by the selected policy.
        relay_routes: u8,
        /// Fixed total publication limit.
        maximum: usize,
    },
    /// The supplied relay policy violates the DMS contract.
    #[error(transparent)]
    Relay(#[from] AukiRelayConfigError),
    /// A bounded address set exceeded its maximum size.
    #[error("{kind} contains more than {maximum} addresses")]
    AddressLimit {
        /// Address-set name.
        kind: &'static str,
        /// Fixed maximum accepted by the runtime.
        maximum: usize,
    },
    /// One encoded multiaddr exceeded its fixed pre-validation bound.
    #[error("{kind} contains a multiaddr larger than {maximum} encoded bytes")]
    AddressTooLong {
        /// Address-set name.
        kind: &'static str,
        /// Fixed maximum encoded size.
        maximum: usize,
    },
    /// A local advertised route is not a direct TCP route.
    #[error("advertised direct route is invalid: {reason}")]
    InvalidAdvertisedDirectRoute {
        /// Bounded static diagnostic.
        reason: &'static str,
    },
    /// One remote peer supplied an invalid route hint.
    #[error("initial route for Peer {peer_id} is invalid")]
    InvalidInitialPeerRoute {
        /// Expected remote peer.
        peer_id: PeerId,
    },
    /// One remote peer exceeded the per-peer route bound.
    #[error("initial routes for Peer {peer_id} exceed the {maximum}-route limit")]
    InitialPeerRouteLimit {
        /// Expected remote peer.
        peer_id: PeerId,
        /// Fixed per-peer maximum.
        maximum: usize,
    },
    /// The configuration exceeded the remote peer bound.
    #[error("initial route hints exceed the {maximum}-peer limit")]
    InitialPeerLimit {
        /// Fixed peer maximum.
        maximum: usize,
    },
    /// The configuration exceeded the aggregate remote route bound.
    #[error("initial route hints exceed the {maximum}-route aggregate limit")]
    InitialRouteLimit {
        /// Fixed aggregate maximum.
        maximum: usize,
    },
}

fn validate_local_route_capacity(
    direct_routes: usize,
    relay: Option<AukiRelayConfig>,
) -> Result<(), AukiPeerConfigError> {
    let relay_routes = relay.map_or(0, |relay| relay.relay_count);
    if direct_routes.saturating_add(usize::from(relay_routes)) > MAX_LOCAL_ROUTES {
        return Err(AukiPeerConfigError::LocalRouteLimit {
            direct_routes,
            relay_routes,
            maximum: MAX_LOCAL_ROUTES,
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum AddressSet {
    Listeners,
}

impl AddressSet {
    const fn label(self) -> &'static str {
        match self {
            Self::Listeners => "listener set",
        }
    }
}

fn parse_dms_base_url(value: &str) -> Result<Url, AukiPeerConfigError> {
    let url = Url::parse(value).map_err(|_| AukiPeerConfigError::InvalidDmsBaseUrl)?;
    let authority = value
        .split_once("//")
        .map(|(_, remainder)| remainder)
        .and_then(|remainder| remainder.split(['/', '?', '#']).next())
        .unwrap_or_default();
    let http_loopback = url.scheme() == "http"
        && url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    if url.cannot_be_a_base()
        || url.host_str().is_none()
        || !(url.scheme() == "https" || http_loopback)
        || authority.contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AukiPeerConfigError::InvalidDmsBaseUrl);
    }
    Ok(url)
}

fn bounded_sorted_addresses(
    addresses: impl IntoIterator<Item = Multiaddr>,
    maximum: usize,
    kind: AddressSet,
) -> Result<Vec<Multiaddr>, AukiPeerConfigError> {
    let mut addresses = addresses.into_iter().collect::<Vec<_>>();
    if addresses.len() > maximum {
        return Err(AukiPeerConfigError::AddressLimit {
            kind: kind.label(),
            maximum,
        });
    }
    if addresses
        .iter()
        .any(|address| address.len() > MAX_MULTIADDR_BYTES)
    {
        return Err(AukiPeerConfigError::AddressTooLong {
            kind: kind.label(),
            maximum: MAX_MULTIADDR_BYTES,
        });
    }
    addresses.sort_unstable_by_key(ToString::to_string);
    addresses.dedup();
    Ok(addresses)
}

fn canonicalize_advertised_routes(
    routes: impl IntoIterator<Item = Multiaddr>,
) -> Result<Vec<Multiaddr>, AukiPeerConfigError> {
    let mut canonical = Vec::new();
    for route in routes.into_iter() {
        if canonical.len() == MAX_ADVERTISED_DIRECT_ROUTES {
            return Err(AukiPeerConfigError::AddressLimit {
                kind: "advertised direct route set",
                maximum: MAX_ADVERTISED_DIRECT_ROUTES,
            });
        }
        if route.len() > MAX_MULTIADDR_BYTES {
            return Err(AukiPeerConfigError::AddressTooLong {
                kind: "advertised direct route set",
                maximum: MAX_MULTIADDR_BYTES,
            });
        }
        validate_identity_independent_direct_route(&route)?;
        canonical.push(route);
    }
    canonical.sort_unstable_by_key(ToString::to_string);
    canonical.dedup();
    Ok(canonical)
}

fn validate_identity_independent_direct_route(
    route: &Multiaddr,
) -> Result<(), AukiPeerConfigError> {
    let protocols = route.iter().collect::<Vec<_>>();
    let (network, port) = match protocols.as_slice() {
        [network, Protocol::Tcp(port)] | [network, Protocol::Tcp(port), Protocol::P2p(_)] => {
            (network, *port)
        }
        _ => {
            return Err(AukiPeerConfigError::InvalidAdvertisedDirectRoute {
                reason: "expected exact ip|dns/tcp[/p2p] grammar",
            });
        }
    };
    if !matches!(
        network,
        Protocol::Ip4(_)
            | Protocol::Ip6(_)
            | Protocol::Dns(_)
            | Protocol::Dns4(_)
            | Protocol::Dns6(_)
    ) {
        return Err(AukiPeerConfigError::InvalidAdvertisedDirectRoute {
            reason: "host must be ip4, ip6, dns, dns4, or dns6",
        });
    }
    if matches!(network, Protocol::Ip4(ip) if ip.is_unspecified())
        || matches!(network, Protocol::Ip6(ip) if ip.is_unspecified())
    {
        return Err(AukiPeerConfigError::InvalidAdvertisedDirectRoute {
            reason: "advertised IP host must not be unspecified",
        });
    }
    if port == 0 {
        return Err(AukiPeerConfigError::InvalidAdvertisedDirectRoute {
            reason: "TCP port must be non-zero",
        });
    }
    Ok(())
}

fn canonicalize_peer_routes(
    peer_id: PeerId,
    routes: impl IntoIterator<Item = Multiaddr>,
) -> Result<Vec<Multiaddr>, AukiPeerConfigError> {
    let mut direct = Vec::new();
    let mut relay = Vec::new();
    let mut seen = HashSet::new();
    for (index, route) in routes.into_iter().enumerate() {
        if index >= MAX_INITIAL_ROUTE_INPUT_PER_PEER {
            return Err(AukiPeerConfigError::InitialPeerRouteLimit {
                peer_id,
                maximum: MAX_INITIAL_ROUTE_INPUT_PER_PEER,
            });
        }
        if route.len() > MAX_MULTIADDR_BYTES {
            return Err(AukiPeerConfigError::AddressTooLong {
                kind: "initial peer route set",
                maximum: MAX_MULTIADDR_BYTES,
            });
        }
        let (route, routes) = match validate_direct_route(&route, peer_id) {
            Ok(route) => (route, &mut direct),
            Err(_) => match canonicalize_circuit_route(&route, peer_id) {
                Ok(route) => (route.route, &mut relay),
                Err(_) => {
                    return Err(AukiPeerConfigError::InvalidInitialPeerRoute { peer_id });
                }
            },
        };
        if seen.insert(route.clone()) {
            routes.push(route);
            if direct.len() + relay.len() > MAX_INITIAL_ROUTES_PER_PEER {
                return Err(AukiPeerConfigError::InitialPeerRouteLimit {
                    peer_id,
                    maximum: MAX_INITIAL_ROUTES_PER_PEER,
                });
            }
        }
    }
    direct.sort_unstable_by_key(ToString::to_string);
    relay.sort_unstable_by_key(ToString::to_string);
    direct.extend(relay);
    Ok(direct)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn addr(value: impl AsRef<str>) -> Multiaddr {
        Multiaddr::from_str(value.as_ref()).expect("test multiaddr must parse")
    }

    fn peer(seed: u64) -> PeerId {
        let mut encoded = [0_u8; 34];
        encoded[1] = 32;
        encoded[2..10].copy_from_slice(&seed.to_be_bytes());
        PeerId::from_bytes(&encoded).expect("test Peer ID must parse")
    }

    #[test]
    fn defaults_are_relay_required_and_serve_no_protocols() {
        let config = AukiPeerConfig::dev("robot-app", "/tmp/robot-app");
        assert!(config.relay_required());
        assert_eq!(config.relay(), Some(AukiRelayConfig::default()));
        assert_eq!(config.served_protocols(), ServedProtocols::none());
        assert!(config.listen_addresses().is_empty());
        assert!(config.advertised_direct_routes().is_empty());
        assert!(config.initial_peer_routes().is_empty());
        assert_eq!(config.app_id(), "robot-app");
        assert_eq!(config.storage_root(), Path::new("/tmp/robot-app"));
        assert_eq!(config.dms_base_url(), DEV_DMS_BASE_URL);
    }

    #[test]
    fn direct_only_is_the_explicit_relay_opt_out() {
        let config = AukiPeerConfig::dev("app", ".").direct_only();
        assert!(!config.relay_required());
        assert_eq!(config.relay(), None);
    }

    #[test]
    fn dms_base_preserves_path_prefix_and_accepts_literal_loopback_http() {
        let prefixed = AukiPeerConfig::new("https://dms.example/v1", "app", ".").unwrap();
        assert_eq!(prefixed.dms_base_url(), "https://dms.example/v1");

        for base in [
            "http://127.0.0.1:8080/v1/",
            "http://127.42.0.7/v1/",
            "http://[::1]:8080/v1/",
        ] {
            assert!(AukiPeerConfig::new(base, "app", ".").is_ok(), "{base}");
        }
    }

    #[test]
    fn dms_base_rejects_unsafe_or_ambiguous_urls() {
        for base in [
            "not a URL",
            "ftp://dms.example/v1/",
            "http://dms.example/v1/",
            "http://localhost:8080/v1/",
            "http://192.168.1.1/v1/",
            "https://user@dms.example/v1/",
            "https://:secret@dms.example/v1/",
            "https://@dms.example/v1/",
            "HTTPS://@dms.example/v1/",
            "https://dms.example/v1/?domain=x",
            "https://dms.example/v1/#fragment",
        ] {
            assert_eq!(
                AukiPeerConfig::new(base, "app", ".").unwrap_err(),
                AukiPeerConfigError::InvalidDmsBaseUrl,
                "{base}"
            );
        }
    }

    #[test]
    fn listener_addresses_are_deduplicated_and_stably_sorted() {
        let first = addr("/ip4/127.0.0.1/tcp/4002");
        let second = addr("/ip4/127.0.0.1/tcp/4001");
        let config = AukiPeerConfig::dev("app", ".")
            .with_listen_addresses([first.clone(), second.clone(), first])
            .unwrap();
        assert_eq!(
            config.listen_addresses(),
            [second, addr("/ip4/127.0.0.1/tcp/4002")]
        );
    }

    #[test]
    fn listener_address_count_is_bounded_before_deduplication() {
        let addresses =
            (0..=MAX_LISTEN_ADDRESSES).map(|port| addr(format!("/ip4/127.0.0.1/tcp/{port}")));
        assert!(matches!(
            AukiPeerConfig::dev("app", ".").with_listen_addresses(addresses),
            Err(AukiPeerConfigError::AddressLimit {
                kind: "listener set",
                maximum: MAX_LISTEN_ADDRESSES,
            })
        ));
    }

    #[test]
    fn advertised_routes_require_nonzero_direct_tcp_addresses() {
        let expected_peer = peer(7);
        let valid = addr(format!("/dns4/robot.example/tcp/4001/p2p/{expected_peer}"));
        let config = AukiPeerConfig::dev("app", ".")
            .with_advertised_direct_routes([valid.clone()])
            .unwrap();
        assert_eq!(config.advertised_direct_routes(), [valid]);

        for invalid in [
            addr("/ip4/127.0.0.1/tcp/0"),
            addr("/ip4/0.0.0.0/tcp/4001"),
            addr("/ip6/::/tcp/4001"),
            addr("/ip4/127.0.0.1/udp/4001"),
            addr("/tcp/4001"),
        ] {
            assert!(matches!(
                AukiPeerConfig::dev("app", ".").with_advertised_direct_routes([invalid]),
                Err(AukiPeerConfigError::InvalidAdvertisedDirectRoute { .. })
            ));
        }
    }

    #[test]
    fn initial_routes_are_canonical_direct_first_and_replaceable() {
        let target = peer(11);
        let relay = peer(12);
        let direct = addr("/dns4/robot.example/tcp/4001");
        let direct_with_peer = addr(format!("{direct}/p2p/{target}"));
        let circuit = addr(format!(
            "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}"
        ));
        let config = AukiPeerConfig::dev("app", ".")
            .with_peer_routes(target, [circuit.clone(), direct_with_peer, direct.clone()])
            .unwrap();
        assert_eq!(config.initial_peer_routes().len(), 1);
        assert_eq!(config.initial_peer_routes()[0].routes(), [direct, circuit]);

        let replacement = addr("/ip4/127.0.0.1/tcp/5000");
        let config = config
            .with_peer_routes(target, [replacement.clone()])
            .unwrap();
        assert_eq!(config.initial_peer_routes()[0].routes(), [replacement]);

        let config = config.with_peer_routes(target, []).unwrap();
        assert!(config.initial_peer_routes().is_empty());
    }

    #[test]
    fn initial_routes_reject_wrong_target_and_bound_raw_input() {
        let target = peer(21);
        let other = peer(22);
        let wrong = addr(format!("/ip4/127.0.0.1/tcp/4001/p2p/{other}"));
        assert!(matches!(
            AukiPeerConfig::dev("app", ".").with_peer_routes(target, [wrong]),
            Err(AukiPeerConfigError::InvalidInitialPeerRoute { .. })
        ));

        let too_many = (1..=17).map(|port| addr(format!("/ip4/127.0.0.1/tcp/{port}")));
        assert!(matches!(
            AukiPeerConfig::dev("app", ".").with_peer_routes(target, too_many),
            Err(AukiPeerConfigError::InitialPeerRouteLimit {
                maximum: MAX_INITIAL_ROUTES_PER_PEER,
                ..
            })
        ));
    }

    #[test]
    fn served_protocols_are_an_explicit_opt_in() {
        let selected = ServedProtocols::none()
            .with_resources_v3()
            .with_streams_v2();
        let config = AukiPeerConfig::dev("app", ".").with_served_protocols(selected);
        assert_eq!(config.served_protocols(), selected);
    }

    #[test]
    fn relay_policy_preserves_supported_robot_deployment_choices() {
        let relay = AukiRelayConfig::new(
            AukiRelayMode::Dedicated,
            3,
            Duration::from_secs(300),
            Duration::from_secs(60),
        )
        .unwrap();
        let config = AukiPeerConfig::dev("app", ".")
            .direct_only()
            .with_relay(relay)
            .unwrap();
        assert_eq!(config.relay(), Some(relay));
        assert!(config.relay_required());
    }

    #[test]
    fn relay_policy_rejects_every_value_outside_the_dms_contract() {
        for relay_count in [0, 4] {
            assert!(matches!(
                AukiRelayConfig::new(
                    AukiRelayMode::Public,
                    relay_count,
                    Duration::from_secs(300),
                    Duration::from_secs(5),
                ),
                Err(AukiRelayConfigError::RelayCount { .. })
            ));
        }
        for duration in [
            Duration::from_secs(299),
            Duration::from_millis(300_001),
            Duration::from_secs(86_401),
        ] {
            assert!(matches!(
                AukiRelayConfig::new(AukiRelayMode::Public, 1, duration, Duration::from_secs(5),),
                Err(AukiRelayConfigError::RequestedDuration { .. })
            ));
        }
        for poll in [
            Duration::from_millis(999),
            Duration::from_millis(1_500),
            Duration::from_secs(61),
        ] {
            assert!(matches!(
                AukiRelayConfig::new(AukiRelayMode::Public, 1, Duration::from_secs(300), poll,),
                Err(AukiRelayConfigError::StatusPollInterval { .. })
            ));
        }

        let invalid_literal = AukiRelayConfig {
            relay_count: 0,
            ..AukiRelayConfig::default()
        };
        assert!(matches!(
            AukiPeerConfig::dev("app", ".").with_relay(invalid_literal),
            Err(AukiPeerConfigError::Relay(
                AukiRelayConfigError::RelayCount { .. }
            ))
        ));
    }

    #[test]
    fn local_direct_and_required_relay_routes_share_one_capacity() {
        let sixteen_direct = (1..=16)
            .map(|port| addr(format!("/ip4/127.0.0.1/tcp/{port}")))
            .collect::<Vec<_>>();
        assert!(matches!(
            AukiPeerConfig::dev("app", ".").with_advertised_direct_routes(sixteen_direct.clone()),
            Err(AukiPeerConfigError::LocalRouteLimit {
                direct_routes: 16,
                relay_routes: 1,
                maximum: MAX_LOCAL_ROUTES,
            })
        ));

        let direct_only = AukiPeerConfig::dev("app", ".")
            .direct_only()
            .with_advertised_direct_routes(sixteen_direct)
            .unwrap();
        assert!(matches!(
            direct_only.with_relay(AukiRelayConfig::default()),
            Err(AukiPeerConfigError::LocalRouteLimit {
                direct_routes: 16,
                relay_routes: 1,
                maximum: MAX_LOCAL_ROUTES,
            })
        ));

        let three_relays = AukiRelayConfig::new(
            AukiRelayMode::Public,
            3,
            Duration::from_secs(300),
            Duration::from_secs(5),
        )
        .unwrap();
        let thirteen_direct = (1..=13)
            .map(|port| addr(format!("/ip4/127.0.0.1/tcp/{port}")))
            .collect::<Vec<_>>();
        let exact_capacity = AukiPeerConfig::dev("app", ".")
            .direct_only()
            .with_advertised_direct_routes(thirteen_direct)
            .unwrap()
            .with_relay(three_relays)
            .unwrap();
        assert_eq!(exact_capacity.relay(), Some(three_relays));
    }
}

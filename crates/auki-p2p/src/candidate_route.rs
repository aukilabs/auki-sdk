//! Target-neutral validation of untrusted route hints.

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};

use crate::relay::canonicalize_provider_base;

/// Maximum encoded size of one candidate multiaddr.
pub const CANDIDATE_ROUTE_MAX_BYTES: usize = 1_024;

/// Transport shape of one validated candidate route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateRouteKind {
    /// A direct IP or DNS address over TCP.
    DirectTcp,
    /// A Circuit Relay v2 route reached over TCP.
    RelayTcp,
    /// A Circuit Relay v2 route reached over secure WebSockets.
    RelayWss,
}

/// One bounded candidate route in the SDK's canonical grammar.
///
/// This only validates an untrusted dialing hint. It does not prove that the
/// route reaches the expected peer or grant any capability; the connection
/// must still authenticate the expected [`PeerId`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalCandidateRoute {
    route: Multiaddr,
    kind: CandidateRouteKind,
}

impl CanonicalCandidateRoute {
    /// Canonical multiaddr to use as the exact dialing hint.
    pub fn route(&self) -> &Multiaddr {
        &self.route
    }

    /// Direct or relay transport shape represented by this route.
    pub fn kind(&self) -> CandidateRouteKind {
        self.kind
    }

    /// Consume this validation result and return its canonical multiaddr.
    pub fn into_route(self) -> Multiaddr {
        self.route
    }
}

/// Why an untrusted candidate route was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CandidateRouteError {
    /// The encoded multiaddr exceeds the shared SDK safety limit.
    #[error("candidate route is {actual} bytes, exceeding the {maximum}-byte safety limit")]
    TooLong { actual: usize, maximum: usize },
    /// The multiaddr is outside the exact direct and relay route grammars.
    #[error("candidate route does not match an exact supported route grammar")]
    InvalidGrammar,
    /// An encoded destination differs from the expected remote peer.
    #[error("candidate route target Peer ID does not match the expected peer")]
    TargetPeerMismatch,
}

pub type CandidateRouteResult<T> = std::result::Result<T, CandidateRouteError>;

/// Canonicalize one bounded, untrusted dialing hint for an expected peer.
///
/// Direct routes accept exact `ip|dns/tcp[/p2p]` grammar. A terminal Peer ID,
/// when present, must match `expected_target_peer_id` and is removed from the
/// canonical direct route. Relay routes require exact
/// `dns4/tcp[/wss]/p2p/<relay>/p2p-circuit/p2p/<target>` grammar and retain the
/// matching target Peer ID.
pub fn canonicalize_candidate_route(
    route: &Multiaddr,
    expected_target_peer_id: PeerId,
) -> CandidateRouteResult<CanonicalCandidateRoute> {
    if route.len() > CANDIDATE_ROUTE_MAX_BYTES {
        return Err(CandidateRouteError::TooLong {
            actual: route.len(),
            maximum: CANDIDATE_ROUTE_MAX_BYTES,
        });
    }

    match canonicalize_direct_route(route, expected_target_peer_id) {
        Ok(route) => Ok(CanonicalCandidateRoute {
            route,
            kind: CandidateRouteKind::DirectTcp,
        }),
        Err(CandidateRouteError::TargetPeerMismatch) => {
            Err(CandidateRouteError::TargetPeerMismatch)
        }
        Err(CandidateRouteError::InvalidGrammar) => {
            canonicalize_relay_route(route, expected_target_peer_id)
        }
        Err(CandidateRouteError::TooLong { .. }) => {
            unreachable!("candidate size is checked before grammar validation")
        }
    }
}

fn canonicalize_direct_route(
    route: &Multiaddr,
    expected_target_peer_id: PeerId,
) -> CandidateRouteResult<Multiaddr> {
    let protocols = route.iter().collect::<Vec<_>>();
    let (network, port, target_peer_id) = match protocols.as_slice() {
        [network, Protocol::Tcp(port)] => (network, *port, None),
        [network, Protocol::Tcp(port), Protocol::P2p(peer_id)] => (network, *port, Some(*peer_id)),
        _ => return Err(CandidateRouteError::InvalidGrammar),
    };
    if !matches!(
        network,
        Protocol::Ip4(_)
            | Protocol::Ip6(_)
            | Protocol::Dns(_)
            | Protocol::Dns4(_)
            | Protocol::Dns6(_)
    ) || port == 0
    {
        return Err(CandidateRouteError::InvalidGrammar);
    }
    if target_peer_id.is_some_and(|peer_id| peer_id != expected_target_peer_id) {
        return Err(CandidateRouteError::TargetPeerMismatch);
    }

    let mut canonical = route.clone();
    if target_peer_id.is_some() {
        canonical.pop();
    }
    Ok(canonical)
}

fn canonicalize_relay_route(
    route: &Multiaddr,
    expected_target_peer_id: PeerId,
) -> CandidateRouteResult<CanonicalCandidateRoute> {
    let mut provider_base = route.clone();
    let target_peer_id = match provider_base.pop() {
        Some(Protocol::P2p(peer_id)) => peer_id,
        _ => return Err(CandidateRouteError::InvalidGrammar),
    };
    if target_peer_id != expected_target_peer_id {
        return Err(CandidateRouteError::TargetPeerMismatch);
    }
    if !matches!(provider_base.pop(), Some(Protocol::P2pCircuit)) {
        return Err(CandidateRouteError::InvalidGrammar);
    }
    let relay_peer_id = match provider_base.iter().last() {
        Some(Protocol::P2p(peer_id)) => peer_id,
        _ => return Err(CandidateRouteError::InvalidGrammar),
    };
    let canonical_base = canonicalize_provider_base(&provider_base.to_string(), relay_peer_id)
        .map_err(|_| CandidateRouteError::InvalidGrammar)?;
    let kind = match canonical_base.iter().collect::<Vec<_>>().as_slice() {
        [Protocol::Dns4(_), Protocol::Tcp(_), Protocol::P2p(_)] => CandidateRouteKind::RelayTcp,
        [Protocol::Dns4(_), Protocol::Tcp(_), Protocol::Wss(path), Protocol::P2p(_)]
            if path.as_ref() == "/" =>
        {
            CandidateRouteKind::RelayWss
        }
        _ => return Err(CandidateRouteError::InvalidGrammar),
    };
    Ok(CanonicalCandidateRoute {
        route: canonical_base
            .with(Protocol::P2pCircuit)
            .with(Protocol::P2p(target_peer_id)),
        kind,
    })
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, str::FromStr};

    use libp2p::identity::Keypair;

    use super::*;

    fn peer_id() -> PeerId {
        Keypair::generate_ed25519().public().to_peer_id()
    }

    fn route(value: impl AsRef<str>) -> Multiaddr {
        Multiaddr::from_str(value.as_ref()).expect("test route must parse")
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn direct_tcp_route_strips_matching_target_suffix() {
        let target = peer_id();
        let candidate = route(format!("/dns6/peer.example.com/tcp/9443/p2p/{target}"));

        let canonical = canonicalize_candidate_route(&candidate, target).unwrap();

        assert_eq!(canonical.kind(), CandidateRouteKind::DirectTcp);
        assert_eq!(
            canonical.route().to_string(),
            "/dns6/peer.example.com/tcp/9443"
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn relay_tcp_and_wss_routes_are_canonicalized_and_classified() {
        let relay = peer_id();
        let target = peer_id();
        let tcp = route(format!(
            "/dns4/RELAY.AUKIVERSE.COM./tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}"
        ));
        let wss = route(format!(
            "/dns4/RELAY.AUKIVERSE.COM./tcp/4443/wss/p2p/{relay}/p2p-circuit/p2p/{target}"
        ));

        let canonical_tcp = canonicalize_candidate_route(&tcp, target).unwrap();
        let canonical_wss = canonicalize_candidate_route(&wss, target).unwrap();

        assert_eq!(canonical_tcp.kind(), CandidateRouteKind::RelayTcp);
        assert_eq!(canonical_wss.kind(), CandidateRouteKind::RelayWss);
        assert_eq!(
            canonical_tcp.route().to_string(),
            format!("/dns4/relay.aukiverse.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}")
        );
        assert_eq!(
            canonical_wss.route().to_string(),
            format!("/dns4/relay.aukiverse.com/tcp/4443/wss/p2p/{relay}/p2p-circuit/p2p/{target}")
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn encoded_target_must_match_for_direct_and_relay_routes() {
        let relay = peer_id();
        let target = peer_id();
        let other = peer_id();
        let direct = route(format!("/ip4/192.0.2.1/tcp/443/p2p/{other}"));
        let circuit = route(format!(
            "/dns4/relay.aukiverse.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{other}"
        ));

        assert_eq!(
            canonicalize_candidate_route(&direct, target),
            Err(CandidateRouteError::TargetPeerMismatch)
        );
        assert_eq!(
            canonicalize_candidate_route(&circuit, target),
            Err(CandidateRouteError::TargetPeerMismatch)
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn rejects_zero_ports_and_non_exact_relay_grammars() {
        let relay = peer_id();
        let target = peer_id();
        let zero_port = route("/ip6/2001:db8::1/tcp/0");
        let ws_relay = route(format!(
            "/dns4/relay.aukiverse.com/tcp/443/ws/p2p/{relay}/p2p-circuit/p2p/{target}"
        ));
        let ip_relay = route(format!(
            "/ip4/192.0.2.1/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}"
        ));

        for candidate in [zero_port, ws_relay, ip_relay] {
            assert_eq!(
                canonicalize_candidate_route(&candidate, target),
                Err(CandidateRouteError::InvalidGrammar)
            );
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn rejects_routes_over_the_shared_sdk_byte_limit() {
        let target = peer_id();
        let oversized = Multiaddr::empty()
            .with(Protocol::Dns(Cow::Owned(
                "a".repeat(CANDIDATE_ROUTE_MAX_BYTES),
            )))
            .with(Protocol::Tcp(443));
        assert!(oversized.len() > CANDIDATE_ROUTE_MAX_BYTES);

        assert_eq!(
            canonicalize_candidate_route(&oversized, target),
            Err(CandidateRouteError::TooLong {
                actual: oversized.len(),
                maximum: CANDIDATE_ROUTE_MAX_BYTES,
            })
        );
    }
}

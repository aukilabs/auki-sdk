//! Exact WSS relay route validation shared by the browser runtime and native tests.

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};

use crate::{
    relay::{canonicalize_provider_base, RelayBaseTransport, RelayProvider},
    Error, Result,
};

pub(crate) struct ParsedBrowserRelayRoute {
    pub(crate) relay_peer_id: PeerId,
    pub(crate) target_peer_id: PeerId,
    pub(crate) direct_relay_address: Multiaddr,
    pub(crate) circuit_dial_address: Multiaddr,
}

pub(crate) fn browser_direct_address(provider: &RelayProvider) -> Result<Multiaddr> {
    if provider.selected_transport() != RelayBaseTransport::Wss {
        return Err(Error::InvalidRelayRoute {
            address: provider.selected_base().to_string(),
            reason: "browser relay reservations require a selected WSS base".into(),
        });
    }
    let mut direct = provider.selected_base().clone();
    match direct.pop() {
        Some(Protocol::P2p(peer_id)) if peer_id == provider.relay_peer_id() => Ok(direct),
        _ => Err(Error::InvalidRelayRoute {
            address: provider.selected_base().to_string(),
            reason: "selected relay base is missing its terminal Peer ID".into(),
        }),
    }
}

pub(crate) fn parse_browser_relay_route(route: &Multiaddr) -> Result<ParsedBrowserRelayRoute> {
    let mut base = route.clone();
    let target_peer_id = match base.pop() {
        Some(Protocol::P2p(peer_id)) => peer_id,
        _ => return invalid_browser_route(route, "route is missing its target Peer ID"),
    };
    if !matches!(base.pop(), Some(Protocol::P2pCircuit)) {
        return invalid_browser_route(route, "route is missing p2p-circuit");
    }
    let relay_peer_id = match base.iter().last() {
        Some(Protocol::P2p(peer_id)) => peer_id,
        _ => return invalid_browser_route(route, "relay base is missing its Peer ID"),
    };
    let canonical =
        canonicalize_provider_base(&base.to_string(), relay_peer_id).map_err(|error| {
            Error::InvalidRelayRoute {
                address: route.to_string(),
                reason: error.to_string(),
            }
        })?;
    let has_wss = canonical
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Wss(path) if path.as_ref() == "/"));
    if canonical != base || !has_wss {
        return invalid_browser_route(route, "expected one exact canonical WSS relay route");
    }
    let canonical_route = canonical
        .clone()
        .with(Protocol::P2pCircuit)
        .with(Protocol::P2p(target_peer_id));
    if canonical_route != *route {
        return invalid_browser_route(route, "route is not in canonical form");
    }

    let mut direct_relay_address = canonical;
    direct_relay_address.pop();
    let mut circuit_dial_address = route.clone();
    circuit_dial_address.pop();
    Ok(ParsedBrowserRelayRoute {
        relay_peer_id,
        target_peer_id,
        direct_relay_address,
        circuit_dial_address,
    })
}

pub(crate) fn parse_browser_relay_route_for_peer(
    route: &Multiaddr,
    expected_peer_id: PeerId,
) -> Result<ParsedBrowserRelayRoute> {
    let parsed = parse_browser_relay_route(route)?;
    if parsed.target_peer_id != expected_peer_id {
        return Err(Error::UnexpectedRemotePeer {
            expected: expected_peer_id.to_string(),
            actual: parsed.target_peer_id.to_string(),
        });
    }
    Ok(parsed)
}

fn invalid_browser_route<T>(route: &Multiaddr, reason: &str) -> Result<T> {
    Err(Error::InvalidRelayRoute {
        address: route.to_string(),
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use libp2p::identity::Keypair;

    use super::*;
    use crate::relay::ExpectedRelayLimits;

    fn peer_id() -> PeerId {
        Keypair::generate_ed25519().public().to_peer_id()
    }

    fn limits() -> ExpectedRelayLimits {
        ExpectedRelayLimits::new(Duration::from_secs(900), 10_737_418_240).unwrap()
    }

    #[test]
    fn browser_provider_selects_exact_wss_dial_address() {
        let relay = peer_id();
        let provider = RelayProvider::new_for_transport(
            relay,
            [
                format!("/dns4/relay.dev.aukiverse.com/tcp/443/p2p/{relay}"),
                format!("/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/{relay}"),
            ],
            RelayBaseTransport::Wss,
            limits(),
        )
        .unwrap();

        assert_eq!(
            browser_direct_address(&provider).unwrap().to_string(),
            "/dns4/relay.dev.aukiverse.com/tcp/4443/wss"
        );
    }

    #[test]
    fn browser_provider_rejects_native_transport_selection() {
        let relay = peer_id();
        let provider = RelayProvider::new(
            relay,
            [format!("/dns4/relay.dev.aukiverse.com/tcp/443/p2p/{relay}")],
            limits(),
        )
        .unwrap();

        assert!(matches!(
            browser_direct_address(&provider),
            Err(Error::InvalidRelayRoute { .. })
        ));
    }

    #[test]
    fn exact_wss_circuit_route_yields_pinned_dial_addresses() {
        let relay = peer_id();
        let target = peer_id();
        let route: Multiaddr = format!(
            "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/{relay}/p2p-circuit/p2p/{target}"
        )
        .parse()
        .unwrap();

        let parsed = parse_browser_relay_route(&route).unwrap();
        assert_eq!(parsed.relay_peer_id, relay);
        assert_eq!(parsed.target_peer_id, target);
        assert_eq!(
            parsed.direct_relay_address.to_string(),
            "/dns4/relay.dev.aukiverse.com/tcp/4443/wss"
        );
        assert_eq!(
            parsed.circuit_dial_address.to_string(),
            format!("/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/{relay}/p2p-circuit")
        );
    }

    #[test]
    fn browser_route_rejects_tcp_and_noncanonical_shapes() {
        let relay = peer_id();
        let target = peer_id();
        let invalid = [
            format!(
                "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}"
            ),
            format!("/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/{relay}"),
            format!(
                "/dns4/RELAY.dev.aukiverse.com/tcp/4443/wss/p2p/{relay}/p2p-circuit/p2p/{target}"
            ),
            format!(
                "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/{relay}/p2p-circuit/p2p/{target}/p2p-circuit"
            ),
        ];

        for raw in invalid {
            let route: Multiaddr = raw.parse().unwrap();
            assert!(
                parse_browser_relay_route(&route).is_err(),
                "accepted invalid route {raw}"
            );
        }
    }

    #[test]
    fn expected_target_is_rejected_before_the_route_can_be_dialed() {
        let relay = peer_id();
        let advertised_target = peer_id();
        let expected_target = peer_id();
        let route: Multiaddr = format!(
            "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/{relay}/p2p-circuit/p2p/{advertised_target}"
        )
        .parse()
        .unwrap();

        assert!(matches!(
            parse_browser_relay_route_for_peer(&route, expected_target),
            Err(Error::UnexpectedRemotePeer { expected, actual })
                if expected == expected_target.to_string()
                    && actual == advertised_target.to_string()
        ));
    }
}

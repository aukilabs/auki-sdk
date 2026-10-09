//! Full native facade against disposable DMS/NCS/Hagall. API/DDS issuance,
//! isolated DNS and an ephemeral TLS CA are fixtures; validation is not bypassed.
use std::{path::PathBuf, time::Duration};

use auki_p2p::{DdsTokenVerifier, Identity, Multiaddr, Node, PublishedRoute, RelayBaseTransport};
use futures::io::{AsyncReadExt, AsyncWriteExt};
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    AukiPeer, AukiPeerBootstrap, AukiPeerConfig, AukiPeerStatus, AukiProtocolRegistration,
    AukiProtocolSpec, AukiRelayConfig, AukiRelayMode, AuthClient, AuthEnvironment, Credentials,
    DomainSelection, RelayBillingAcceptance,
};

use crate::circuit_handover_fixture as fixture;

const PROTOCOL: &str = "/auki-p2p/paid-identity-e2e/1";

#[derive(Clone, Debug)]
pub(crate) struct TestTransport {
    resolver: ResolverConfig,
    options: ResolverOpts,
    ca: Option<Vec<u8>>,
}

impl TestTransport {
    pub(crate) fn start(
        &self,
        identity: Identity,
        verifier: DdsTokenVerifier,
        listeners: &[Multiaddr],
    ) -> auki_p2p::Result<Node> {
        Node::start_with_test_transport(
            identity,
            verifier,
            listeners.iter().cloned(),
            self.resolver.clone(),
            self.options.clone(),
            self.ca.clone(),
        )
    }
}

#[derive(Deserialize)]
struct FixtureInfo {
    domain_id: Uuid,
}

async fn bootstrap(endpoint: &str, mode: &str, config: AukiPeerConfig) -> AukiPeerBootstrap {
    AukiPeerBootstrap::authenticate(
        AuthClient::new(AuthEnvironment::new(endpoint, endpoint).unwrap()).unwrap(),
        Credentials::app(mode, "local-fixture-only"),
        config,
    )
    .await
    .unwrap()
}

fn route(peer: &AukiPeer) -> PublishedRoute {
    let snapshot = peer.protocol_context().routes().snapshot().unwrap();
    assert!(snapshot.direct_routes.is_empty());
    assert_eq!(snapshot.relay_routes.len(), 1);
    snapshot.relay_routes[0].clone()
}

fn serve(peer: &AukiPeer) -> AukiProtocolRegistration {
    peer.protocols()
        .register(
            AukiProtocolSpec::new(PROTOCOL, 16, 8192).unwrap(),
            |mut stream| async move {
                let mut bytes = vec![0; 8192];
                // Expiry/shutdown may close a handshake or stream; successful echoes
                // are asserted by the caller, never inferred from handler startup.
                if stream.read_exact(&mut bytes).await.is_ok() {
                    let _ = stream.write_all(&bytes).await;
                    let _ = stream.flush().await;
                }
            },
        )
        .unwrap()
}

async fn echo(source: &AukiPeer, target: &AukiPeer, route: &PublishedRoute, marker: u8) {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut stream = source
            .protocols()
            .open_exact(target.peer_id(), route.routes.wss().clone(), PROTOCOL)
            .await
            .unwrap();
        assert!(stream.is_relayed());
        assert_eq!(stream.remote_peer().peer_id, target.peer_id());
        let bytes = vec![marker; 8192];
        stream.write_all(&bytes).await.unwrap();
        stream.flush().await.unwrap();
        let mut received = vec![0; bytes.len()];
        stream.read_exact(&mut received).await.unwrap();
        assert_eq!(bytes, received);
        stream.close().await.unwrap();
    })
    .await
    .expect("authenticated WSS echo deadline");
}

async fn reject_tls(source: &AukiPeer, target: &AukiPeer, address: Multiaddr, expected: &str) {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        source
            .protocols()
            .open_exact(target.peer_id(), address, PROTOCOL),
    )
    .await
    .unwrap();
    let error = result
        .err()
        .expect("invalid TLS certificate must reject the circuit");
    let detail = format!("{error:?}");
    assert!(
        detail.contains(expected),
        "expected {expected}, got {detail}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the NCS disposable cross-repository runner; no shared services"]
async fn paid_wss_refresh_expiry_and_persistent_restart() {
    let endpoint = std::env::var("RELAY_TRANSPORT_FIXTURE_URL").unwrap();
    let dms = std::env::var("RELAY_TRANSPORT_DMS_URL").unwrap();
    for address in [&endpoint, &dms] {
        assert_eq!(
            reqwest::Url::parse(address).unwrap().host_str(),
            Some("127.0.0.1")
        );
    }
    let ready = std::env::var("RELAY_TRANSPORT_HAGALL_READY").unwrap();
    let identity_file = PathBuf::from(std::env::var("RELAY_TRANSPORT_IDENTITY_FILE").unwrap());
    assert!(
        !identity_file.exists(),
        "runner must supply a disposable new identity path"
    );
    let info: FixtureInfo = reqwest::get(format!("{endpoint}/fixture/info"))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let selection = DomainSelection::new(info.domain_id);
    let dns = fixture::TestDns::start();
    let (resolver, options) = dns.resolver();
    let transport = TestTransport {
        resolver,
        options,
        ca: Some(std::fs::read(format!("{ready}.ca.der")).unwrap()),
    };
    let mut config = AukiPeerConfig::new(&dms)
        .unwrap()
        .with_relay_transport(RelayBaseTransport::Wss)
        .with_relay(
            AukiRelayConfig::new(
                AukiRelayMode::Public,
                1,
                Duration::from_secs(300),
                Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap()
        .with_relay_billing(RelayBillingAcceptance::new("local-v1", "1", "1").unwrap())
        .unwrap();
    config.test_transport = Some(transport.clone());
    let outbound = config.clone().without_relay();
    let target_bootstrap = bootstrap(&endpoint, "refresh", config.clone()).await;
    let target = tokio::time::timeout(
        Duration::from_secs(60),
        target_bootstrap.start_persistent_peer(selection, &identity_file),
    )
    .await
    .unwrap()
    .unwrap();
    let peer_id = target.peer_id();
    let original = route(&target);
    let authorization = target.protocol_context().authorization();
    let initial = authorization.current().unwrap();
    let endpoint_registration = serve(&target);

    let mut untrusted_config = outbound.clone();
    untrusted_config.test_transport.as_mut().unwrap().ca = None;
    let untrusted_bootstrap = bootstrap(&endpoint, "source", untrusted_config).await;
    let untrusted = untrusted_bootstrap
        .start_ephemeral_peer(selection)
        .await
        .unwrap();
    reject_tls(
        &untrusted,
        &target,
        original.routes.wss().clone(),
        "UnknownIssuer",
    )
    .await;
    untrusted.shutdown().await.unwrap();
    untrusted_bootstrap.session().close().await;

    let source_bootstrap = bootstrap(&endpoint, "source", outbound).await;
    let source = source_bootstrap
        .start_ephemeral_peer(selection)
        .await
        .unwrap();
    let wrong_host = original
        .routes
        .wss()
        .to_string()
        .replace(
            "relay-0.interop.aukiverse.com",
            "wrong.interop.aukiverse.com",
        )
        .replace(
            "relay-1.interop.aukiverse.com",
            "wrong.interop.aukiverse.com",
        );
    reject_tls(
        &source,
        &target,
        wrong_host.parse().unwrap(),
        "certificate not valid for name",
    )
    .await;
    echo(&source, &target, &original, 1).await;

    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let current = authorization.current().unwrap();
            if current.credential_revision() >= 2 {
                if initial.credential_revision() == 1 {
                    assert!(current.expires_at() > initial.expires_at());
                }
                assert_eq!(current.claims().sub, initial.claims().sub);
                assert_eq!(current.claims().peer_id, initial.claims().peer_id);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("real SDK scheduled renewal, 401 exchange and signed credential replacement");
    assert_eq!(
        route(&target).fence,
        original.fence,
        "refresh preserves the paid reservation"
    );
    assert_eq!(target.status(), AukiPeerStatus::Ready);
    for batch in 0..3 {
        futures::future::join_all(
            (0..8).map(|i| echo(&source, &target, &original, 10 + batch * 8 + i)),
        )
        .await;
    }
    endpoint_registration.close().await.unwrap();
    target.shutdown().await.unwrap();
    assert!(
        authorization.current().is_err(),
        "retained handles are fenced on shutdown"
    );

    // Restart is an explicit application action with a fresh per-booking ceiling.
    // The persisted key must survive, and shutdown must have released its booking.
    let restarted = tokio::time::timeout(
        Duration::from_secs(60),
        target_bootstrap.start_persistent_peer(selection, &identity_file),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(restarted.peer_id(), peer_id);
    let replacement = route(&restarted);
    assert_ne!(replacement.fence.route_id, original.fence.route_id);
    let restarted_registration = serve(&restarted);
    echo(&source, &restarted, &replacement, 100).await;
    restarted_registration.close().await.unwrap();
    restarted.shutdown().await.unwrap();
    target_bootstrap.session().close().await;

    let expiry_bootstrap = bootstrap(&endpoint, "expiry", config).await;
    let expiring = tokio::time::timeout(
        Duration::from_secs(30),
        expiry_bootstrap.start_ephemeral_peer(selection),
    )
    .await
    .unwrap()
    .unwrap();
    let expiry_route = route(&expiring);
    let expiry_registration = serve(&expiring);
    let expired_authorization = expiring.protocol_context().authorization();
    let deadline = expired_authorization.current().unwrap().expires_at();
    echo(&source, &expiring, &expiry_route, 200).await;
    tokio::time::timeout(Duration::from_secs(75), async {
        while chrono::Utc::now() <= deadline + chrono::Duration::seconds(1) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        while expiring.status().is_ready() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("literal credential expiry fences the real facade");
    assert!(expired_authorization.current().is_err());
    let expired_open = tokio::time::timeout(
        Duration::from_secs(15),
        source.protocols().open_exact(
            expiring.peer_id(),
            expiry_route.routes.wss().clone(),
            PROTOCOL,
        ),
    )
    .await;
    assert!(
        expired_open.is_ok_and(|result| result.is_err()),
        "expired authority must reject fresh application streams"
    );
    expiry_registration.close().await.unwrap();
    // With DDS still unavailable and no usable bearer, authenticated DMS DELETE
    // can fail. The NCS harness separately proves expiry closes the ledger and
    // releases every hold without another purchase or an administrative edit.
    let cleanup = expiring.shutdown().await;
    if let Err(error) = cleanup {
        assert!(error.discovery().is_none());
        assert!(error.routes().is_empty());
        assert!(error.supervisor().is_none());
        assert!(error.transport().is_none());
        assert!(error.protocols().is_none());
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(
            error
                .relay()
                .expect("only expired relay authority may prevent cleanup"),
        );
        let mut expected = false;
        while let Some(current) = cause {
            expected |= matches!(
                current.downcast_ref::<crate::relay::coordinator::RelayCoordinatorError>(),
                Some(crate::relay::coordinator::RelayCoordinatorError::AuthorityEnded)
                    | Some(crate::relay::coordinator::RelayCoordinatorError::Dms(
                        auki_relay_booking::RelayBookingClientError::Authentication { .. }
                    ))
            );
            cause = current.source();
        }
        assert!(expected, "unexpected expired-peer cleanup: {error:?}");
        println!("expired peer transport stopped; DMS expiry owns the remaining release: {error}");
    }
    expiry_bootstrap.session().close().await;
    source.shutdown().await.unwrap();
    source_bootstrap.session().close().await;
    println!(
        "27 authenticated WSS 8-KiB echoes; TLS issuer/hostname rejection; scheduled refresh with 401; persistent same-Peer-ID restart; literal expiry with DDS unavailable passed"
    );
}

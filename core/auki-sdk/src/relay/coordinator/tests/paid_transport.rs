//! Explicitly opt-in real DMS/NCS + Hagall test. Only DDS issuance and DNS are
//! fixtures. Keep this inside the coordinator so no production DNS/test hook is
//! added to AukiPeer. The facade's identity supervisor is covered separately.
use super::*;
use auki_p2p::{
    ApplicationProtocol, DdsTokenVerifier, ExactRoute, Identity, PublishedRoute, RouteCatalog,
    RouteCatalogLimits, SessionRequirements, SignedP2pCredential,
};
use auki_relay_booking::{
    RelayAuthorizationError, RelayAuthorizationProvider, RelayAuthorizationSnapshot,
    RelayBillingAcceptance, RelayBookingClient,
};
use futures::io::{AsyncReadExt, AsyncWriteExt};
use serde::Deserialize;
use serde_json::json;

struct Authorization(RelayAuthorizationSnapshot);

#[async_trait]
impl RelayAuthorizationProvider for Authorization {
    async fn authorization(&self) -> Result<RelayAuthorizationSnapshot, RelayAuthorizationError> {
        Ok(self.0.clone())
    }

    async fn refresh_after_unauthorized(&self, _: u64) -> Result<(), RelayAuthorizationError> {
        Err(RelayAuthorizationError)
    }
}

#[derive(Deserialize)]
struct FixtureInfo {
    domain_id: Uuid,
    public_key: String,
}

async fn install_identity(node: &Node, endpoint: &str, role: &str) -> String {
    let response: serde_json::Value = reqwest::Client::new()
        .post(format!("{endpoint}/fixture/peer"))
        .json(&json!({"peer_id":node.peer_id().to_string(), "role":role}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = response["token"].as_str().unwrap().to_string();
    node.authority()
        .install_credential(SignedP2pCredential::new(token.clone()).unwrap())
        .await
        .unwrap();
    token
}

async fn current_route(
    catalog: &RouteCatalog,
    other_than: Option<PeerId>,
    timeout: Duration,
) -> PublishedRoute {
    tokio::time::timeout(timeout, async {
        loop {
            if let Some(route) = catalog
                .snapshot()
                .unwrap()
                .relay_routes
                .into_iter()
                .find(|route| Some(route.relay_peer_id) != other_than)
            {
                return route;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("real Hagall route becomes usable within the DMS recovery grace")
}

async fn echo(source: &Node, target: PeerId, domain: Uuid, route: &PublishedRoute, marker: u8) {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut stream = source
            .open_exact_route(
                target,
                ExactRoute::Circuit(route.routes.tcp().clone()),
                ApplicationProtocol::new("/auki-p2p/paid-e2e/1").unwrap(),
                SessionRequirements::new(domain.to_string())
                    .unwrap()
                    .with_expected_remote_peer_id(target),
            )
            .await
            .unwrap();
        assert!(
            stream.is_relayed(),
            "a direct route must not mask a broken relay"
        );
        let payload = vec![marker; 8192];
        stream.write_all(&payload).await.unwrap();
        stream.flush().await.unwrap();
        let mut received = vec![0; payload.len()];
        stream.read_exact(&mut received).await.unwrap();
        assert_eq!(received, payload);
        stream.close().await.unwrap();
    })
    .await
    .expect("relay round trip deadline");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the NCS disposable cross-repository runner; no shared services"]
async fn paid_booking_renews_and_recovers_real_provider_loss() {
    let endpoint = std::env::var("RELAY_TRANSPORT_FIXTURE_URL").unwrap();
    let dms = std::env::var("RELAY_TRANSPORT_DMS_URL").unwrap();
    for address in [&endpoint, &dms] {
        assert_eq!(
            reqwest::Url::parse(address).unwrap().host_str(),
            Some("127.0.0.1")
        );
    }
    let http = reqwest::Client::new();
    let info: FixtureInfo = http
        .get(format!("{endpoint}/fixture/info"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let dns = fixture::TestDns::start();
    let make_node = || {
        let (dns_config, dns_options) = dns.resolver();
        Node::start_with_dns_config(
            Identity::generate(),
            DdsTokenVerifier::from_es256_pem(info.public_key.as_bytes()).unwrap(),
            ["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            dns_config,
            dns_options,
        )
        .unwrap()
    };
    let target = make_node();
    let source = make_node();
    let token = install_identity(&target, &endpoint, "robot").await;
    install_identity(&source, &endpoint, "compute").await;
    let mut header = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
    header.set_sensitive(true);
    let client = Arc::new(
        RelayBookingClient::new(
            dms.parse().unwrap(),
            Arc::new(Authorization(RelayAuthorizationSnapshot::new(header, 1))),
        )
        .unwrap(),
    );
    let catalog =
        RouteCatalog::new(target.peer_id(), vec![], RouteCatalogLimits::new(2, 1)).unwrap();
    let lifecycle = CancellationToken::new();
    let mut config = coordinator_config("local-real-paid-transport");
    config.billing = Some(RelayBillingAcceptance::new("local-v1", "1", "1").unwrap());
    config.status_poll_interval = Duration::from_secs(1);
    config.http_timeout = Duration::from_secs(10);
    config.retry_min = Duration::from_secs(1);
    config.retry_max = Duration::from_secs(2);
    let coordinator = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            match RelayBookingCoordinator::start(
                client.clone(),
                PeerRelayReservations::new(target.clone(), lifecycle.clone()),
                catalog.clone(),
                config.clone(),
            )
            .await
            {
                Ok(coordinator) => break coordinator,
                Err(error) if error.startup_retry_after(Duration::from_secs(1)).is_some() => {
                    tokio::time::sleep(Duration::from_secs(1)).await
                }
                Err(error) => panic!("paid coordinator startup: {error:?}"),
            }
        }
    })
    .await
    .unwrap();
    let initial = client.active().await.unwrap().unwrap();
    let first = current_route(&catalog, None, Duration::from_secs(45)).await;
    let mut incoming = target
        .accept(
            ApplicationProtocol::new("/auki-p2p/paid-e2e/1").unwrap(),
            SessionRequirements::new(info.domain_id.to_string()).unwrap(),
        )
        .unwrap();
    let server = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..58 {
            let stream = incoming.accept().await.unwrap().unwrap();
            tasks.spawn(async move {
                let mut stream = stream;
                let mut bytes = vec![0; 8192];
                stream.read_exact(&mut bytes).await.unwrap();
                stream.write_all(&bytes).await.unwrap();
                stream.flush().await.unwrap();
            });
            while let Some(result) = tasks.try_join_next() {
                result.unwrap();
            }
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    });
    echo(&source, target.peer_id(), info.domain_id, &first, 1).await;
    http.post(format!("{endpoint}/fixture/outage"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    for marker in 2..27 {
        echo(&source, target.peer_id(), info.domain_id, &first, marker).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert!(!coordinator.health().is_failed());
    http.post(format!("{endpoint}/fixture/fail-provider"))
        .json(&json!({"peer_id":first.relay_peer_id.to_string()}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let recovery_deadline = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = client.active().await.unwrap().unwrap();
            if let Some(slot) = snapshot
                .slots
                .iter()
                .find(|slot| slot.state == RelaySlotState::Recovering)
            {
                break slot.recovery_expires_at.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the dead provider enters the actual bounded DMS recovery window");
    http.post(format!("{endpoint}/fixture/recovery-observed"))
        .json(&json!({"expires_at":recovery_deadline}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let second = current_route(
        &catalog,
        Some(first.relay_peer_id),
        Duration::from_secs(390),
    )
    .await;
    assert_eq!(
        second.fence.route_id, first.fence.route_id,
        "one logical slot survives failover"
    );
    assert_ne!(second.fence.authority_id, first.fence.authority_id);
    assert!(
        !catalog
            .snapshot()
            .unwrap()
            .relay_routes
            .iter()
            .any(|route| route.fence == first.fence)
    );
    let after = client.active().await.unwrap().unwrap();
    assert_eq!(after.booking_id, initial.booking_id);
    assert!(
        after.authority_expires_at > initial.authority_expires_at,
        "automatic SDK renewal must extend funded authority"
    );
    for batch in 0..4 {
        futures::future::join_all((0..8).map(|i| {
            echo(
                &source,
                target.peer_id(),
                info.domain_id,
                &second,
                100 + batch * 8 + i,
            )
        }))
        .await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    assert!(!coordinator.health().is_failed());
    assert_eq!(
        coordinator
            .shutdown(true, Duration::from_secs(30))
            .await
            .unwrap(),
        RelayCoordinatorShutdownOutcome::Graceful
    );
    assert!(catalog.snapshot().unwrap().relay_routes.is_empty());
    assert!(client.active().await.unwrap().is_none());
    tokio::time::timeout(Duration::from_secs(15), server)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), source.shutdown())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), target.shutdown())
        .await
        .unwrap()
        .unwrap();
    println!(
        "58 verified 8-KiB relayed round trips, 45-second NCS outage, actual provider loss/reassignment and automatic paid renewal passed"
    );
}

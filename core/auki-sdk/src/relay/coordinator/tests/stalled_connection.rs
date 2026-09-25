//! Actual Node + booking coordinator with a scripted DMS boundary and Go relay.
use super::*;
use auki_p2p::{ApplicationProtocol, ExactRoute, PeerRole, Protocol};
use futures::io::{AsyncReadExt, AsyncWriteExt};

#[path = "../../../../../../test-support/circuit-handover/fixtures.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;

struct RecoveringDms {
    snapshot: Mutex<RelayBookingSnapshot>,
    failures: Mutex<Vec<ReservationFailedRequest>>,
    creates: AtomicUsize,
    deletes: AtomicUsize,
}

#[async_trait]
impl RelayBookingApi for RecoveringDms {
    async fn active(&self) -> ApiResult<Option<RelayBookingSnapshot>> {
        Ok(Some(self.snapshot.lock().clone()))
    }

    async fn create(
        &self,
        _: &RelayIdempotencyKey,
        _: &CreateRelayBookingRequest,
    ) -> ApiResult<CreateRelayBookingResponse> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        panic!("recovery must preserve the existing booking")
    }

    async fn renew(&self, id: Uuid) -> ApiResult<RelayBookingSnapshot> {
        let snapshot = self.snapshot.lock();
        assert_eq!(id, snapshot.booking_id);
        Ok(snapshot.clone())
    }

    async fn report_reservation_failed(
        &self,
        id: Uuid,
        request: &ReservationFailedRequest,
    ) -> ApiResult<RelayBookingSnapshot> {
        let mut snapshot = self.snapshot.lock();
        assert_eq!(id, snapshot.booking_id);
        assert_eq!(request.slot_id, snapshot.slots[0].slot_id);
        assert_eq!(Some(request.assignment_id), snapshot.slots[0].assignment_id);
        assert_eq!(
            Some(request.reservation_epoch),
            snapshot.slots[0].reservation_epoch
        );
        assert_eq!(request.reason, ReservationFailureReason::ReservationLost);
        self.failures.lock().push(request.clone());
        snapshot.provider_ready_count = 0;
        snapshot.slots[0].state = RelaySlotState::Recovering;
        snapshot.slots[0].reservation_epoch = Some(Uuid::new_v4());
        snapshot.slots[0].provider_lease_expires_at = None;
        snapshot.slots[0].recovery_expires_at =
            Some(chrono::Utc::now() + chrono::Duration::minutes(2));
        Ok(snapshot.clone())
    }

    async fn delete(&self, id: Uuid) -> ApiResult<()> {
        assert_eq!(id, self.snapshot.lock().booking_id);
        self.deletes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("recovery condition reached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go relay fixture; no live DMS calls"]
async fn caller_timeouts_recover_through_dms_without_creating_a_new_booking() {
    tokio::time::timeout(Duration::from_secs(60), exercise_recovery())
        .await
        .expect("the complete booking recovery and shutdown remain bounded");
}

async fn exercise_recovery() {
    let relay = GoRelay::start(60);
    let provider = relay.provider();
    let relay_peer = provider.relay_peer_id();
    let relay_port = provider
        .selected_base()
        .iter()
        .find_map(|part| match part {
            Protocol::Tcp(port) => Some(port),
            _ => None,
        })
        .unwrap();
    let mut proxy = FaultProxy::start(relay_port).await;
    let dns = TestDns::start();
    let source = node(&dns);
    let target = node(&dns);
    let domain = Uuid::new_v4().to_string();
    for peer in [&source, &target] {
        install_current_token(peer, PeerRole::Robot, vec![domain.clone()]).await;
    }
    let target_reservation = target.start_relay_reservation(provider).await.unwrap();
    target
        .wait_relay_reservation(target_reservation)
        .await
        .unwrap();
    let base = format!(
        "/dns4/proxy.relay.auki-p2p.dev/tcp/{}/p2p/{relay_peer}",
        proxy.port
    );
    let wss = format!(
        "/dns4/proxy.relay.auki-p2p.dev/tcp/{}/wss/p2p/{relay_peer}",
        proxy.port
    );
    let booking_id = Uuid::new_v4();
    let mut snapshot = ready_snapshot(
        booking_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        relay_peer,
    );
    snapshot.slots[0].provider_base_addresses = Some(vec![base.clone(), wss]);
    snapshot.slots[0].limits = Some(RelayLimits {
        duration_seconds: 60,
        data_bytes_per_direction: 64 * 1024 * 1024,
    });
    let original_epoch = snapshot.slots[0].reservation_epoch.unwrap();
    let api = Arc::new(RecoveringDms {
        snapshot: Mutex::new(snapshot),
        failures: Mutex::new(Vec::new()),
        creates: AtomicUsize::new(0),
        deletes: AtomicUsize::new(0),
    });
    let routes = local_route_catalog(source.peer_id());
    let backend = Arc::new(PeerRelayReservations::new(
        source.clone(),
        CancellationToken::new(),
    ));
    let mut config = coordinator_config("stalled-full-booking-cycle");
    config.status_poll_interval = Duration::from_millis(100);
    let coordinator = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend,
        Arc::new(routes.clone()),
        config,
    )
    .await
    .unwrap();
    until(|| routes.snapshot().unwrap().relay_routes.len() == 1).await;
    let fault = proxy.accepted.recv().await.unwrap();
    fault.send(TunnelState::Stalled).unwrap();
    let route: auki_p2p::Multiaddr = format!("{base}/p2p-circuit/p2p/{}", target.peer_id())
        .parse()
        .unwrap();
    let protocol = ApplicationProtocol::new("/auki-p2p/coordinator-recovery/1").unwrap();
    let requirements = auki_p2p::SessionRequirements::new(&domain)
        .unwrap()
        .with_expected_remote_peer_id(target.peer_id());
    // All callers abandon their opens before the libp2p negotiation timer fires.
    for _ in 0..3 {
        assert!(
            tokio::time::timeout(
                Duration::from_secs(9),
                source.open_exact_route(
                    target.peer_id(),
                    ExactRoute::Circuit(route.clone()),
                    protocol.clone(),
                    requirements.clone(),
                )
            )
            .await
            .is_err()
        );
        tokio::time::sleep(Duration::from_millis(1200)).await;
    }
    until(|| api.failures.lock().len() == 1).await;
    until(|| routes.snapshot().unwrap().relay_routes.is_empty()).await;
    assert!(
        *fault.borrow() == TunnelState::Stalled,
        "SDK, not the test, closes the tunnel"
    );
    assert_eq!(api.failures.lock()[0].reservation_epoch, original_epoch);
    // Simulate the provider completing DMS Recover + Ready, preserving booking ID.
    let recovered_epoch = {
        let mut snapshot = api.snapshot.lock();
        snapshot.provider_ready_count = 1;
        snapshot.slots[0].state = RelaySlotState::Ready;
        snapshot.slots[0].provider_lease_expires_at =
            Some(chrono::Utc::now() + chrono::Duration::minutes(4));
        snapshot.slots[0].recovery_expires_at = None;
        snapshot.slots[0].reservation_epoch.unwrap()
    };
    assert_ne!(original_epoch, recovered_epoch);
    until(|| routes.snapshot().unwrap().relay_routes.len() == 1).await;
    let _new_tunnel = proxy.accepted.recv().await.unwrap();
    assert_eq!(api.snapshot.lock().booking_id, booking_id);
    assert_eq!(api.creates.load(Ordering::SeqCst), 0);
    assert_eq!(
        api.failures.lock().len(),
        1,
        "Unpublished/Canceled must not duplicate failure reports"
    );
    let mut incoming = target
        .accept(
            protocol.clone(),
            auki_p2p::SessionRequirements::new(&domain).unwrap(),
        )
        .unwrap();
    let echo = tokio::spawn(async move {
        let mut stream = incoming.accept().await.unwrap().unwrap();
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        stream.write_all(&byte).await.unwrap();
        stream.flush().await.unwrap();
    });
    let mut stream = tokio::time::timeout(
        Duration::from_secs(5),
        source.open_exact_route(
            target.peer_id(),
            ExactRoute::Circuit(route),
            protocol,
            requirements,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        stream.write_all(b"R").await.unwrap();
        stream.flush().await.unwrap();
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        assert_eq!(&byte, b"R");
    })
    .await
    .unwrap();
    stream.close().await.unwrap();
    echo.await.unwrap();
    coordinator
        .shutdown(true, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(api.deletes.load(Ordering::SeqCst), 1);
    target
        .cancel_relay_reservation(target_reservation)
        .await
        .unwrap();
    source.shutdown().await.unwrap();
    target.shutdown().await.unwrap();
}

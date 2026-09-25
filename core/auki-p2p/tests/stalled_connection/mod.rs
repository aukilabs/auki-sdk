//! Local fault injection against the SDK used in the 1,000-peer attempt.
//! The proxy pauses one established TCP tunnel; siblings bypass that proxy.
//! No DMS, DDS, AWS, public DNS or real credentials are used.

use super::*;
use tokio::{sync::watch, task::JoinSet};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_connection_is_retired_automatically() {
    // Exclude the fixture's usual four-second reservation renewal from the fault.
    let relay =
        RelayHarness::start_with_reservation_duration("stalled-source", Duration::from_secs(180))
            .await;
    exercise_stalled_connection("rust", relay.provider()).await;
    relay.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_stalled_connection_is_retired_automatically() {
    let relay = GoRelay::start(60);
    exercise_stalled_connection("go", relay.provider()).await;
}

async fn exercise_stalled_connection(backend: &str, provider: RelayProvider) {
    let dns = TestDns::start();
    let relay_port = provider
        .selected_base()
        .iter()
        .find_map(|component| match component {
            Protocol::Tcp(port) => Some(port),
            _ => None,
        })
        .unwrap();
    let relay_peer = provider.relay_peer_id();
    let mut proxy = FaultProxy::start(relay_port).await;
    let domain = Uuid::new_v4().to_string();
    let target = node(&dns);
    let affected = node(&dns);
    let healthy = node(&dns);
    for peer in [&target, &affected, &healthy] {
        install_current_token(peer, PeerRole::Robot, vec![domain.clone()]).await;
    }
    let target_reservation = must_succeed(target.start_relay_reservation(provider.clone())).await;
    let target_snapshot = must_succeed(target.wait_relay_reservation(target_reservation)).await;
    let direct_route = target_snapshot.publishable_route().unwrap().clone();
    let proxy_base = format!(
        "/dns4/proxy.relay.auki-p2p.dev/tcp/{}/p2p/{}",
        proxy.port, relay_peer
    );
    let proxy_provider =
        RelayProvider::new(relay_peer, [proxy_base.clone()], provider.expected_limits()).unwrap();
    let proxy_route: Multiaddr = format!("{proxy_base}/p2p-circuit/p2p/{}", target.peer_id())
        .parse()
        .unwrap();
    let affected_reservation =
        must_succeed(affected.start_relay_reservation(proxy_provider.clone())).await;
    let affected_snapshot =
        must_succeed(affected.wait_relay_reservation(affected_reservation)).await;
    let old_connection = affected_snapshot.direct_connection().unwrap();
    let fault = timeout(proxy.accepted.recv()).await.unwrap();
    let protocol = ApplicationProtocol::new("/auki-p2p/stalled-connection-test/1").unwrap();
    let requirements = SessionRequirements::new(&domain)
        .unwrap()
        .with_expected_remote_peer_id(target.peer_id());
    let mut incoming = target
        .accept(protocol.clone(), SessionRequirements::new(&domain).unwrap())
        .unwrap();
    let server = tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                incoming = incoming.accept() => {
                    let Some(Ok(mut stream)) = incoming else { break; };
                    handlers.spawn(async move {
                        let mut byte = [0];
                        while stream.read_exact(&mut byte).await.is_ok() {
                            if stream.write_all(&byte).await.is_err() || stream.flush().await.is_err() { break; }
                        }
                    });
                },
                _ = handlers.join_next(), if !handlers.is_empty() => {},
            }
        }
    });
    let mut sibling = must_succeed(healthy.open_exact_route(
        target.peer_id(),
        ExactRoute::Circuit(direct_route),
        protocol.clone(),
        requirements.clone(),
    ))
    .await;
    let delivered = Arc::new(AtomicU64::new(0));
    let progress = delivered.clone();
    let (stop, mut stopped) = watch::channel(false);
    let traffic = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stopped.changed() => break,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                sibling.write_all(b"H").await.unwrap();
                sibling.flush().await.unwrap();
                let mut byte = [0];
                sibling.read_exact(&mut byte).await.unwrap();
                assert_eq!(&byte, b"H");
            })
            .await
            .expect("healthy sibling stalled");
            progress.fetch_add(1, Ordering::SeqCst);
        }
        sibling.close().await.unwrap();
    });
    let mut relay_events = affected.subscribe_relay_events();
    fault.send(TunnelState::Stalled).unwrap();
    for attempt in 1..=3 {
        let before = delivered.load(Ordering::SeqCst);
        let began = tokio::time::Instant::now();
        let result = timeout(affected.open_exact_route(
            target.peer_id(),
            ExactRoute::Circuit(proxy_route.clone()),
            protocol.clone(),
            requirements.clone(),
        ))
        .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("stalled tunnel unexpectedly opened"),
        };
        match error {
            auki_p2p::Error::TargetedStream(
                auki_p2p::TargetedStreamError::NegotiationTimeout {
                    connection_id,
                    protocol,
                    ..
                },
            ) => {
                assert_eq!(connection_id, old_connection);
                assert_eq!(protocol, SOURCE_ADMISSION_PROTOCOL);
            }
            error => panic!("unexpected failure: {error}"),
        }
        assert!(delivered.load(Ordering::SeqCst) > before + 20);
        assert!(
            matches!(
                proxy.accepted.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ),
            "SDK unexpectedly opened a replacement tunnel"
        );
        println!("backend={backend} attempt={attempt} outcome=negotiation_timeout connection={old_connection} elapsed_ms={} healthy_frames={}", began.elapsed().as_millis(), delivered.load(Ordering::SeqCst));
    }
    // The SDK must retire the connection without the proxy or test closing it.
    timeout(async {
        loop {
            if matches!(relay_events.recv().await.unwrap(),
                auki_p2p::RelayTransportEvent::Canceled { handle } if handle == affected_reservation
            ) {
                break;
            }
        }
    })
    .await;
    assert!(*fault.borrow() == TunnelState::Stalled);
    // Closure may already have retired the generation before explicit cleanup.
    assert!(matches!(
        timeout(affected.cancel_relay_reservation(affected_reservation)).await,
        Ok(())
            | Err(auki_p2p::Error::RelayReservation(
                auki_p2p::RelayReservationError::StaleHandle
            ))
    ));
    assert!(
        matches!(
            timeout(affected.connect_relayed(proxy_route.clone(), &requirements)).await,
            Err(auki_p2p::Error::RelayReservationClosed(_))
        ),
        "retired reservation must fence source circuits until a replacement is confirmed"
    );
    assert!(
        proxy.accepted.try_recv().is_err(),
        "no premature direct dial during recovery"
    );
    let replacement = must_succeed(affected.start_relay_reservation(proxy_provider)).await;
    let snapshot = must_succeed(affected.wait_relay_reservation(replacement)).await;
    let new_connection = snapshot.direct_connection().unwrap();
    assert_ne!(old_connection, new_connection);
    let _replacement_control = timeout(proxy.accepted.recv()).await.unwrap();
    let mut recovered = must_succeed(affected.open_exact_route(
        target.peer_id(),
        ExactRoute::Circuit(proxy_route),
        protocol,
        requirements,
    ))
    .await;
    timeout(async {
        recovered.write_all(b"R").await.unwrap();
        recovered.flush().await.unwrap();
        let mut byte = [0];
        recovered.read_exact(&mut byte).await.unwrap();
        assert_eq!(&byte, b"R");
    })
    .await;
    println!("backend={backend} outcome=recovered old_connection={old_connection} new_connection={new_connection} healthy_frames={}", delivered.load(Ordering::SeqCst));
    recovered.close().await.unwrap();
    stop.send(true).unwrap();
    timeout(traffic).await.unwrap();
    must_succeed(affected.cancel_relay_reservation(replacement)).await;
    must_succeed(target.cancel_relay_reservation(target_reservation)).await;
    for peer in [affected, healthy, target] {
        must_succeed(peer.shutdown()).await;
    }
    server.abort();
    let _ = server.await;
}

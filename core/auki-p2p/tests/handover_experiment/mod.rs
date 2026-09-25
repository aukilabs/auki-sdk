//! Opt-in, loopback-only native experiment. The Go process mocks admission;
//! peers still perform the SDK's mutual DDS authentication with fixture keys.
use super::*;
use std::sync::Mutex;
use tokio::time::Instant;

const PAYLOAD: usize = 3200;

#[derive(Default)]
struct Received {
    sequences: Vec<u64>,
    drains: usize,
}

struct Peers {
    source: Node,
    target: Node,
    route: Multiaddr,
    protocol: ApplicationProtocol,
    requirements: SessionRequirements,
    reservation: auki_p2p::RelayReservationHandle,
    received: Arc<Mutex<Received>>,
    server: auki_p2p::ApplicationProtocolServer,
    _dns: TestDns,
}

impl Peers {
    async fn start(relay: &GoRelay) -> Self {
        let dns = TestDns::start();
        let source = node(&dns);
        let target = node(&dns);
        let domain = Uuid::new_v4().to_string();
        install_current_token(&source, PeerRole::Compute, vec![domain.clone()]).await;
        install_current_token(&target, PeerRole::Robot, vec![domain.clone()]).await;
        let reservation = must_succeed(target.start_relay_reservation(relay.provider())).await;
        let snapshot = must_succeed(target.wait_relay_reservation(reservation)).await;
        let route = snapshot.publishable_route().unwrap().clone();
        let protocol = ApplicationProtocol::new("/auki-p2p/handover-experiment/1").unwrap();
        let requirements = SessionRequirements::new(&domain)
            .unwrap()
            .with_expected_remote_peer_id(target.peer_id());
        let received = Arc::new(Mutex::new(Received::default()));
        let results = received.clone();
        let spec =
            auki_p2p::ApplicationProtocolSpec::new(protocol.clone(), 18, PAYLOAD as u32).unwrap();
        let server = target
            .serve(
                spec,
                SessionRequirements::new(&domain).unwrap(),
                &tokio_util::sync::CancellationToken::new(),
                move |stream| receive(stream, results.clone()),
            )
            .unwrap();
        Self {
            source,
            target,
            route,
            protocol,
            requirements,
            reservation,
            received,
            server,
            _dns: dns,
        }
    }

    async fn open(&self) -> auki_p2p::Result<AuthenticatedRouteStream> {
        self.source
            .open_exact_route(
                self.target.peer_id(),
                ExactRoute::Circuit(self.route.clone()),
                self.protocol.clone(),
                self.requirements.clone(),
            )
            .await
    }

    fn replace(
        &self,
        old: &AuthenticatedRouteStream,
    ) -> impl std::future::Future<Output = auki_p2p::Result<AuthenticatedRouteStream>> + Send + 'static
    {
        self.source
            .prepare_route_replacement(old, self.protocol.clone(), self.requirements.clone())
    }

    async fn finish(self, relay: &mut GoRelay) -> serde_json::Value {
        must_succeed(self.source.shutdown()).await;
        self.finish_after_source_shutdown(relay).await
    }

    async fn finish_after_source_shutdown(self, relay: &mut GoRelay) -> serde_json::Value {
        must_succeed(self.target.cancel_relay_reservation(self.reservation)).await;
        must_succeed(self.target.shutdown()).await;
        must_succeed(self.server.shutdown()).await;
        let stats = relay.wait_active(0).await;
        assert_eq!(
            stats["reservations"], 1,
            "handover created another reservation"
        );
        assert_eq!(stats["opened"], stats["closed"]);
        stats
    }
}

async fn receive<S: futures::AsyncRead + futures::AsyncWrite + Unpin>(
    mut stream: S,
    received: Arc<Mutex<Received>>,
) {
    let mut count = 0u64;
    let mut last = u64::MAX;
    loop {
        let mut header = [0u8; 20];
        if stream.read_exact(&mut header).await.is_err() {
            break;
        }
        let sequence = u64::from_be_bytes(header[..8].try_into().unwrap());
        let length = u32::from_be_bytes(header[16..].try_into().unwrap()) as usize;
        if length == 0 {
            let mut ack = [0; 16];
            ack[..8].copy_from_slice(&last.to_be_bytes());
            ack[8..].copy_from_slice(&count.to_be_bytes());
            if stream.write_all(&ack).await.is_ok() && stream.flush().await.is_ok() {
                received.lock().unwrap().drains += 1;
            }
            break;
        }
        assert_eq!(length, PAYLOAD);
        let mut payload = [0; PAYLOAD];
        if stream.read_exact(&mut payload).await.is_err() {
            break;
        }
        assert!(payload.iter().all(|b| *b == (sequence % 251) as u8));
        received.lock().unwrap().sequences.push(sequence);
        count += 1;
        last = sequence;
    }
}

async fn send(stream: &mut AuthenticatedRouteStream, sequence: u64) -> std::io::Result<()> {
    let mut header = [0; 20];
    header[..8].copy_from_slice(&sequence.to_be_bytes());
    header[16..].copy_from_slice(&(PAYLOAD as u32).to_be_bytes());
    stream.write_all(&header).await?;
    stream.write_all(&[(sequence % 251) as u8; PAYLOAD]).await?;
    stream.flush().await
}

async fn drain(stream: &mut AuthenticatedRouteStream, last: Option<u64>, count: u64) {
    must_succeed(async {
        stream.write_all(&[0; 20]).await?;
        stream.flush().await?;
        let mut ack = [0; 16];
        stream.read_exact(&mut ack).await?;
        assert_eq!(
            u64::from_be_bytes(ack[..8].try_into().unwrap()),
            last.unwrap_or(u64::MAX)
        );
        assert_eq!(u64::from_be_bytes(ack[8..].try_into().unwrap()), count);
        Ok::<_, std::io::Error>(())
    })
    .await;
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    ForcedExpiry,
    Sequential,
    Overlap,
}

async fn streaming(mode: Mode) {
    let mut relay = GoRelay::start(6);
    let peers = Peers::start(&relay).await;
    let mut stream = must_succeed(peers.open()).await;
    let started = Instant::now();
    let mut next_rotation = started + Duration::from_secs(2);
    let mut sequence = 0u64;
    let mut sent = Vec::new();
    let mut circuit_count = 0;
    let mut last = None;
    let mut failures = 0;
    let mut rotations = 0;
    let mut rotation_ms = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while started.elapsed() < Duration::from_secs(8) {
        tick.tick().await;
        if mode != Mode::ForcedExpiry && Instant::now() >= next_rotation {
            let t = Instant::now();
            if mode == Mode::Overlap {
                let replacement = must_succeed(peers.replace(&stream)).await;
                let stats = relay.wait_active(2).await;
                assert_eq!(stats["opened"].as_u64(), Some(rotations + 2));
                drain(&mut stream, last, circuit_count).await;
                must_succeed(stream.close()).await;
                stream = replacement;
                // The old generation's close must not remove the replacement.
                let sibling = must_succeed(peers.open()).await;
                must_succeed(sibling.close()).await;
                assert_eq!(relay.command("stats")["opened"], stats["opened"]);
            } else {
                drain(&mut stream, last, circuit_count).await;
                must_succeed(stream.close()).await;
                relay.wait_active(0).await;
                stream = must_succeed(peers.open()).await;
            }
            relay.wait_active(1).await;
            rotation_ms.push(t.elapsed().as_secs_f64() * 1000.0);
            rotations += 1;
            next_rotation = Instant::now() + Duration::from_secs(2);
            circuit_count = 0;
        }
        match send(&mut stream, sequence).await {
            Ok(()) => {
                sent.push(sequence);
                circuit_count += 1;
                last = Some(sequence);
            }
            Err(_) => {
                assert_eq!(mode, Mode::ForcedExpiry, "planned rotation lost its stream");
                failures += 1;
                must_succeed(stream.close()).await;
                stream = must_succeed(peers.open()).await;
                circuit_count = 0;
                last = None;
            }
        }
        sequence += 1;
    }
    drain(&mut stream, last, circuit_count).await;
    must_succeed(stream.close()).await;
    let (received, drains) = {
        let r = peers.received.lock().unwrap();
        (r.sequences.clone(), r.drains)
    };
    assert!(
        received.windows(2).all(|w| w[0] < w[1]),
        "frames reordered or duplicated"
    );
    if mode == Mode::ForcedExpiry {
        assert_eq!(failures, 1);
    } else {
        assert_eq!(received, sent, "planned handover lost payload");
        assert!(rotations >= 3);
        assert_eq!(drains as u64, rotations + 1);
    }
    let missing = sent.iter().filter(|s| !received.contains(s)).count();
    let stats = peers.finish(&mut relay).await;
    assert_eq!(stats["peak"], if mode == Mode::Overlap { 2 } else { 1 });
    println!(
        "HANDOVER_RESULT {}",
        serde_json::json!({"mode":format!("{mode:?}"),"duration_seconds":8,"sent_frames":sent.len(),"received_frames":received.len(),"successful_writes_missing":missing,"write_failures":failures,"rotations":rotations,"rotation_wall_ms":rotation_ms,"drain_acks":drains,"relay":stats})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_forced_expiry_baseline() {
    streaming(Mode::ForcedExpiry).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_planned_drain_close_reopen() {
    streaming(Mode::Sequential).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_overlap_preserves_frames_and_replacement_cache() {
    streaming(Mode::Overlap).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_failed_dial_leaves_old_stream_usable() {
    let mut relay = GoRelay::start(30);
    let peers = Peers::start(&relay).await;
    let mut old = must_succeed(peers.open()).await;
    must_succeed(send(&mut old, 0)).await;
    relay.command("deny");
    assert!(timeout(peers.replace(&old)).await.is_err());
    must_succeed(send(&mut old, 1)).await;
    drain(&mut old, Some(1), 2).await;
    must_succeed(old.close()).await;
    assert_eq!(peers.received.lock().unwrap().sequences, [0, 1]);
    let stats = peers.finish(&mut relay).await;
    assert_eq!(stats["opened"], 1);
    assert!(stats["rejected"].as_u64().unwrap() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_failed_auth_closes_candidate_and_preserves_old_stream() {
    let mut relay = GoRelay::start(30);
    let peers = Peers::start(&relay).await;
    let mut old = must_succeed(peers.open()).await;
    must_succeed(send(&mut old, 0)).await;
    // Valid signature and correct target Peer ID, but no authority for our Domain.
    install_current_token(
        &peers.target,
        PeerRole::Robot,
        vec![Uuid::new_v4().to_string()],
    )
    .await;
    assert!(timeout(peers.replace(&old)).await.is_err());
    relay.wait_active(1).await;
    must_succeed(send(&mut old, 1)).await;
    drain(&mut old, Some(1), 2).await;
    must_succeed(old.close()).await;
    assert_eq!(peers.received.lock().unwrap().sequences, [0, 1]);
    let stats = peers.finish(&mut relay).await;
    assert_eq!(stats["opened"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_cancelled_dial_closes_candidate_and_preserves_old_stream() {
    let mut relay = GoRelay::start(30);
    let peers = Peers::start(&relay).await;
    let mut old = must_succeed(peers.open()).await;
    must_succeed(send(&mut old, 0)).await;
    relay.command("delay");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), peers.replace(&old))
            .await
            .is_err()
    );
    // Wait for the relay's delayed CONNECT to finish, then for cancellation cleanup.
    timeout(async {
        loop {
            let stats = relay.command("stats");
            if stats["opened"] == 2 && stats["active"] == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    must_succeed(send(&mut old, 1)).await;
    drain(&mut old, Some(1), 2).await;
    must_succeed(old.close()).await;
    assert_eq!(peers.received.lock().unwrap().sequences, [0, 1]);
    let stats = peers.finish(&mut relay).await;
    assert_eq!(stats["opened"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_concurrent_replacement_shares_new_circuit_and_keeps_old_sibling() {
    let mut relay = GoRelay::start(30);
    let peers = Peers::start(&relay).await;
    let old = must_succeed(peers.open()).await;
    let mut old_sibling = must_succeed(peers.open()).await;
    let (new, sibling) = tokio::join!(peers.replace(&old), peers.replace(&old));
    let mut new = new.unwrap();
    let sibling = sibling.unwrap();
    let stats = relay.wait_active(2).await;
    assert_eq!(stats["opened"], 2);
    // A repeated request against the old generation must not retire the new one.
    must_succeed(must_succeed(peers.replace(&old)).await.close()).await;
    must_succeed(old.close()).await;
    assert_eq!(relay.command("stats")["active"], 2);
    must_succeed(send(&mut old_sibling, 0)).await;
    drain(&mut old_sibling, Some(0), 1).await;
    must_succeed(old_sibling.close()).await;
    relay.wait_active(1).await;
    must_succeed(sibling.close()).await;
    must_succeed(send(&mut new, 1)).await;
    drain(&mut new, Some(1), 1).await;
    must_succeed(new.close()).await;
    assert_eq!(peers.received.lock().unwrap().sequences, [0, 1]);
    assert_eq!(peers.finish(&mut relay).await["opened"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_old_expiry_does_not_invalidate_new_circuit() {
    let mut relay = GoRelay::start(4);
    let peers = Peers::start(&relay).await;
    let old = must_succeed(peers.open()).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut new = must_succeed(peers.replace(&old)).await;
    relay.wait_active(2).await;
    relay.wait_active(1).await; // The Go deadline has reset the old circuit.
    must_succeed(old.close()).await;
    let sibling = must_succeed(peers.open()).await;
    assert_eq!(relay.command("stats")["opened"], 2);
    must_succeed(sibling.close()).await;
    must_succeed(send(&mut new, 0)).await;
    drain(&mut new, Some(0), 1).await;
    must_succeed(new.close()).await;
    assert_eq!(peers.received.lock().unwrap().sequences, [0]);
    assert_eq!(peers.finish(&mut relay).await["closed"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_shutdown_releases_both_live_generations() {
    let mut relay = GoRelay::start(30);
    let peers = Peers::start(&relay).await;
    let old = must_succeed(peers.open()).await;
    let new = must_succeed(peers.replace(&old)).await;
    relay.wait_active(2).await;
    must_succeed(peers.source.shutdown()).await;
    relay.wait_active(0).await;
    drop((old, new));
    let stats = peers.finish_after_source_shutdown(&mut relay).await;
    assert_eq!(stats["opened"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_old_stream_continues_while_preparing_replacement() {
    let mut relay = GoRelay::start(30);
    let peers = Peers::start(&relay).await;
    let mut old = must_succeed(peers.open()).await;
    relay.command("delay");
    let mut candidate = Box::pin(peers.replace(&old));
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    let mut count = 0;
    let mut new = timeout(async {
        loop {
            tokio::select! {
                new = &mut candidate => break new.unwrap(),
                _ = tick.tick() => {
                    must_succeed(send(&mut old, count)).await;
                    count += 1;
                }
            }
        }
    })
    .await;
    drop(candidate);
    assert!(
        count >= 10,
        "old stream did not continue during the delayed dial"
    );
    drain(&mut old, Some(count - 1), count).await;
    must_succeed(old.close()).await;
    relay.wait_active(1).await;
    must_succeed(send(&mut new, count)).await;
    drain(&mut new, Some(count), 1).await;
    must_succeed(new.close()).await;
    assert_eq!(
        peers.received.lock().unwrap().sequences,
        (0..=count).collect::<Vec<_>>()
    );
    let stats = peers.finish(&mut relay).await;
    assert_eq!(stats["opened"], 2);
    assert_eq!(stats["admissions"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the loopback Go fixture"]
async fn go_replacement_rejects_foreign_node_and_bounds_generations() {
    let mut relay = GoRelay::start(30);
    let peers = Peers::start(&relay).await;
    let old = must_succeed(peers.open()).await;
    assert!(matches!(
        peers
            .target
            .prepare_route_replacement(&old, peers.protocol.clone(), peers.requirements.clone())
            .await,
        Err(auki_p2p::Error::ForeignRelayRoute)
    ));
    let new = must_succeed(peers.replace(&old)).await;
    assert!(timeout(peers.replace(&new)).await.is_err());
    assert_eq!(relay.wait_active(2).await["opened"], 2);
    must_succeed(old.close()).await;
    relay.wait_active(1).await;
    let mut third = must_succeed(peers.replace(&new)).await;
    relay.wait_active(2).await;
    must_succeed(new.close()).await;
    must_succeed(send(&mut third, 0)).await;
    drain(&mut third, Some(0), 1).await;
    must_succeed(third.close()).await;
    let stats = peers.finish(&mut relay).await;
    assert_eq!(stats["opened"], 3);
    assert_eq!(stats["peak"], 2);
}

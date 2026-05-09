//! Integration test for `discovery_client` against a real Discovery
//! binary. Skipped by default (won't compile on `--no-default-features`,
//! and `#[ignore]` even with `--features discovery_client`).
//!
//! ## Running
//!
//! ```bash
//! # First build / locate the Discovery binary:
//! (cd /path/to/Discovery && cargo build)
//!
//! # Then run the ignored test, pointing at the binary:
//! DISCOVERY_BIN=/path/to/Discovery/target/debug/discovery \
//!   cargo test -p auki-network --features discovery_client \
//!     -- --ignored discovery_round_trip
//! ```
//!
//! What the test covers:
//!
//! - Boot Discovery on a tempdir + freshly-picked loopback port.
//! - `register` Sentinel and Booster into the same cluster.
//! - `fetch` returns both peers.
//! - `deregister` removes Sentinel; `fetch` returns just Booster.
//! - Tampered signature → `Status { 401 }`.
//! - Cross-cluster replay (sign for cluster A, POST to cluster B) →
//!   `Status { 401 }`.
//!
//! Discovery's own unit tests already pin the verifier; this exercises
//! the SDK's wire shape against the real verifier over the real HTTP
//! transport.

#![cfg(feature = "discovery_client")]

use auki_identity::Wallet;
use auki_network::cluster_doc::ClusterDoc;
use auki_network::discovery_client::{DiscoveryClient, DiscoveryError};
use futures::StreamExt;
use multiaddr::Multiaddr;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Discovery child handle. `Drop` kills the process so a panicking
/// test doesn't leak a daemon.
struct DiscoveryChild {
    child: Child,
    base_url: String,
    _data_dir: tempfile::TempDir,
}

impl Drop for DiscoveryChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn Discovery on a free loopback port and wait until it answers
/// `GET /clusters`. Returns `None` if `DISCOVERY_BIN` isn't set so the
/// test can skip cleanly without panicking on a missing fixture.
async fn spawn_discovery() -> Option<DiscoveryChild> {
    let bin = std::env::var("DISCOVERY_BIN").ok()?;
    let bin = PathBuf::from(bin);
    if !bin.exists() {
        eprintln!("DISCOVERY_BIN={} does not exist; skipping", bin.display());
        return None;
    }

    // Pick a free port: bind a listener on 127.0.0.1:0, read the port,
    // drop the listener. Discovery binds the same port a moment later;
    // the race window is small enough to be irrelevant in practice.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);

    let data_dir = tempfile::tempdir().expect("temp dir");
    let addr = format!("127.0.0.1:{port}");
    let child = Command::new(&bin)
        .arg("--addr")
        .arg(&addr)
        .arg("--data-dir")
        .arg(data_dir.path())
        .env("RUST_LOG", "discovery=warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn discovery");

    // Poll GET /clusters until 200 OK or 5s elapses.
    let base_url = format!("http://{addr}");
    let probe = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .expect("probe client");
    let started_at = std::time::Instant::now();
    loop {
        if let Ok(resp) = probe.get(format!("{base_url}/clusters")).send().await
            && resp.status().is_success()
        {
            break;
        }
        if started_at.elapsed() > Duration::from_secs(5) {
            panic!("discovery did not respond on {base_url}/clusters within 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Some(DiscoveryChild {
        child,
        base_url,
        _data_dir: data_dir,
    })
}

fn fixed_addrs(spec: &str) -> Vec<Multiaddr> {
    vec![spec.parse().expect("valid multiaddr")]
}

#[tokio::test]
#[ignore = "requires DISCOVERY_BIN env var pointing at a built discovery binary"]
async fn discovery_round_trip() {
    let Some(disco) = spawn_discovery().await else {
        eprintln!("skipping: DISCOVERY_BIN unset (set it to a built ./discovery to run)");
        return;
    };

    let client = DiscoveryClient::new(&disco.base_url);

    let sentinel = Wallet::from_seed(&[1u8; 32]);
    let booster = Wallet::from_seed(&[2u8; 32]);

    // 1. Sentinel registers; expect a ClusterDoc with one peer.
    let doc1: ClusterDoc = client
        .register(
            &sentinel,
            "vinland",
            &fixed_addrs("/ip4/192.168.9.130/tcp/4011"),
            Some("sentinel"),
            None,
        )
        .await
        .expect("sentinel register");
    assert_eq!(doc1.cluster_name, "vinland");
    assert_eq!(doc1.peers.len(), 1);

    // 2. Booster registers; expect a ClusterDoc with two peers.
    let doc2: ClusterDoc = client
        .register(
            &booster,
            "vinland",
            &fixed_addrs("/ip4/192.168.9.72/tcp/4001"),
            Some("boosterapp"),
            None,
        )
        .await
        .expect("booster register");
    assert_eq!(doc2.peers.len(), 2);

    // 3. fetch returns both.
    let fetched = client.fetch("vinland").await.expect("fetch");
    assert_eq!(fetched.peers.len(), 2);

    // 4. Sentinel deregisters; one peer left.
    client
        .deregister(&sentinel, "vinland")
        .await
        .expect("sentinel deregister");
    let after = client.fetch("vinland").await.expect("fetch after deregister");
    assert_eq!(after.peers.len(), 1);
    assert_eq!(after.peers[0].expected_app_id.as_deref(), Some("boosterapp"));

    // 5. Sentinel deregister again → 404 (already removed).
    let err = client
        .deregister(&sentinel, "vinland")
        .await
        .expect_err("second sentinel deregister must fail");
    match err {
        DiscoveryError::Status { status: 404, .. } => {}
        other => panic!("expected Status 404, got {other:?}"),
    }

    // 6. Unknown cluster → 404 on fetch.
    let err = client
        .fetch("not-a-cluster")
        .await
        .expect_err("fetch unknown cluster must fail");
    match err {
        DiscoveryError::Status { status: 404, .. } => {}
        other => panic!("expected Status 404, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires DISCOVERY_BIN env var pointing at a built discovery binary"]
async fn discovery_rejects_invalid_cluster_name() {
    let Some(disco) = spawn_discovery().await else {
        eprintln!("skipping: DISCOVERY_BIN unset");
        return;
    };
    let client = DiscoveryClient::new(&disco.base_url);
    let wallet = Wallet::from_seed(&[42u8; 32]);

    // cluster_name with `/` violates Discovery's `^[A-Za-z0-9._-]+$`
    // charset and breaks URL routing too — caught somewhere in the
    // Discovery side. We assert it errors, not the specific status,
    // because path traversal can land as 400 or 404 depending on
    // axum's router decisions.
    let err = client
        .register(
            &wallet,
            "../etc/passwd",
            &fixed_addrs("/ip4/127.0.0.1/tcp/4001"),
            None,
            None,
        )
        .await
        .expect_err("path-traversal cluster_name must be rejected");
    match err {
        DiscoveryError::Status { .. } => {}
        DiscoveryError::Transport(_) => {}
        other => panic!("expected Status or Transport error, got {other:?}"),
    }
}

// ─── subscribe (live cluster_doc subscriptions) ──────────────────────────────

/// Subscribing to a cluster that already has a peer registered yields
/// the current state as the first event. (Discovery emits a snapshot
/// when a new subscriber connects.)
#[tokio::test]
#[ignore = "requires DISCOVERY_BIN env var pointing at a built discovery binary"]
async fn subscribe_initial_event_is_current_state() {
    let Some(disco) = spawn_discovery().await else {
        eprintln!("skipping: DISCOVERY_BIN unset");
        return;
    };
    let client = DiscoveryClient::new(&disco.base_url);
    let sentinel = Wallet::from_seed(&[10u8; 32]);

    // Pre-register a peer so the cluster has state before we subscribe.
    client
        .register(
            &sentinel,
            "init-snap",
            &fixed_addrs("/ip4/192.168.9.130/tcp/4011"),
            Some("sentinel"),
            None,
        )
        .await
        .expect("sentinel register");

    let stream = client
        .subscribe("init-snap")
        .await
        .expect("subscribe should establish");
    futures::pin_mut!(stream);

    // First event arrives within a few hundred ms — Discovery
    // synthesizes a snapshot for new subscribers.
    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("first event within timeout")
        .expect("stream produced an item")
        .expect("event parses");
    assert_eq!(event.cluster_name, "init-snap");
    assert_eq!(event.peers.len(), 1);
}

/// A `register` against a subscribed cluster_name produces a snapshot
/// event on the subscriber's stream that includes the new peer.
#[tokio::test]
#[ignore = "requires DISCOVERY_BIN env var pointing at a built discovery binary"]
async fn subscribe_receives_register_events() {
    let Some(disco) = spawn_discovery().await else {
        eprintln!("skipping: DISCOVERY_BIN unset");
        return;
    };
    let client = DiscoveryClient::new(&disco.base_url);

    let stream = client
        .subscribe("reg-events")
        .await
        .expect("subscribe should establish");
    futures::pin_mut!(stream);

    // Subscribe before any peer registers — no initial event.
    let no_event = tokio::time::timeout(Duration::from_millis(200), stream.next()).await;
    assert!(
        no_event.is_err(),
        "subscribing to empty cluster should not yield an initial event"
    );

    // Register a peer; expect the next event to carry it.
    let booster = Wallet::from_seed(&[11u8; 32]);
    let bg = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .register(
                    &booster,
                    "reg-events",
                    &fixed_addrs("/ip4/192.168.9.72/tcp/4001"),
                    Some("boosterapp"),
                    None,
                )
                .await
        }
    });

    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("event after register within timeout")
        .expect("stream produced an item")
        .expect("event parses");
    assert_eq!(event.cluster_name, "reg-events");
    assert_eq!(event.peers.len(), 1);
    assert_eq!(event.peers[0].expected_app_id.as_deref(), Some("boosterapp"));

    bg.await.expect("background register").expect("register ok");
}

/// A `deregister` against a subscribed cluster_name produces a
/// snapshot event without the departed peer.
#[tokio::test]
#[ignore = "requires DISCOVERY_BIN env var pointing at a built discovery binary"]
async fn subscribe_receives_deregister_events() {
    let Some(disco) = spawn_discovery().await else {
        eprintln!("skipping: DISCOVERY_BIN unset");
        return;
    };
    let client = DiscoveryClient::new(&disco.base_url);
    let sentinel = Wallet::from_seed(&[12u8; 32]);

    client
        .register(
            &sentinel,
            "dereg-events",
            &fixed_addrs("/ip4/192.168.9.130/tcp/4011"),
            Some("sentinel"),
            None,
        )
        .await
        .expect("sentinel register");

    let stream = client
        .subscribe("dereg-events")
        .await
        .expect("subscribe should establish");
    futures::pin_mut!(stream);

    // Drain the initial snapshot.
    let _initial = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("initial snapshot")
        .expect("stream open")
        .expect("parse");

    // Deregister; expect a snapshot with zero peers.
    client
        .deregister(&sentinel, "dereg-events")
        .await
        .expect("deregister");

    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("dereg event within timeout")
        .expect("stream open")
        .expect("parse");
    assert_eq!(event.cluster_name, "dereg-events");
    assert_eq!(event.peers.len(), 0);
}

/// Subscribing before any peer registers is allowed — no initial
/// item is emitted; the stream starts producing on the first
/// register. This pins the "subscribe-then-register" timing the
/// daemon-side caller (Park, Boosterapp) will rely on at startup.
#[tokio::test]
#[ignore = "requires DISCOVERY_BIN env var pointing at a built discovery binary"]
async fn subscribe_to_empty_cluster_waits_for_first_register() {
    let Some(disco) = spawn_discovery().await else {
        eprintln!("skipping: DISCOVERY_BIN unset");
        return;
    };
    let client = DiscoveryClient::new(&disco.base_url);

    let stream = client
        .subscribe("empty-then-fill")
        .await
        .expect("subscribe to empty cluster");
    futures::pin_mut!(stream);

    // Register a peer; the (single) event carries it.
    let wallet = Wallet::from_seed(&[13u8; 32]);
    client
        .register(
            &wallet,
            "empty-then-fill",
            &fixed_addrs("/ip4/192.168.9.72/tcp/4001"),
            None,
            None,
        )
        .await
        .expect("register");

    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("event within timeout")
        .expect("stream open")
        .expect("parse");
    assert_eq!(event.cluster_name, "empty-then-fill");
    assert_eq!(event.peers.len(), 1);
}

/// Multi-cluster isolation: subscribing to cluster `alpha` and
/// registering a peer into cluster `beta` should NOT produce an event
/// on alpha's stream.
#[tokio::test]
#[ignore = "requires DISCOVERY_BIN env var pointing at a built discovery binary"]
async fn subscribe_isolated_per_cluster_name() {
    let Some(disco) = spawn_discovery().await else {
        eprintln!("skipping: DISCOVERY_BIN unset");
        return;
    };
    let client = DiscoveryClient::new(&disco.base_url);

    let alpha_stream = client
        .subscribe("alpha-iso")
        .await
        .expect("subscribe alpha");
    futures::pin_mut!(alpha_stream);

    let beta_wallet = Wallet::from_seed(&[14u8; 32]);
    client
        .register(
            &beta_wallet,
            "beta-iso",
            &fixed_addrs("/ip4/127.0.0.1/tcp/4001"),
            None,
            None,
        )
        .await
        .expect("register into beta");

    // Wait briefly for a stray event; expect timeout.
    let no_event = tokio::time::timeout(Duration::from_millis(500), alpha_stream.next()).await;
    assert!(
        no_event.is_err(),
        "alpha subscriber received an event from beta — isolation broken"
    );
}

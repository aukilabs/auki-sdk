//! Integration test for `discovery_client` against a real running
//! Discovery service. `#[ignore]` by default — CI doesn't depend on
//! the live deployment.
//!
//! ## Running
//!
//! ```bash
//! DISCOVERY_URL=http://192.168.9.130:8080 \
//!   cargo test -p auki-network --features discovery_client \
//!     --test discovery_integration -- --ignored --nocapture
//! ```
//!
//! What the test covers, against a real Discovery:
//!
//! 1. `put_peer_manager` with a uniquely-named domain registers the hint.
//! 2. A second `put_peer_manager` upserts the same row.
//! 3. `heartbeat` updates `peer_count` + bumps `last_seen`.
//! 4. `rotate_manager` (compat PUT alias) swaps the Manager hint.
//! 5. `deregister` removes the row; a follow-up `get_peer_manager`
//!    returns 404.

#![cfg(feature = "discovery_client")]

use auki_network::PeerIdentity;
use auki_network::discovery_client::{DiscoveryClient, DiscoveryError};
use multiaddr::Multiaddr;
use std::time::{SystemTime, UNIX_EPOCH};

fn discovery_url() -> String {
    std::env::var("DISCOVERY_URL").unwrap_or_else(|_| "http://192.168.9.130:8080".to_string())
}

fn unique_cluster_name(prefix: &str) -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{ns}")
}

#[tokio::test]
#[ignore]
async fn roundtrip_against_live_discovery() {
    let client = DiscoveryClient::new(discovery_url());

    let name = unique_cluster_name("sdk-it");
    let identity = PeerIdentity::from_seed(&[42u8; 32]);
    let manager_peer_id = identity.peer_id();
    let manager_multiaddrs: Vec<Multiaddr> = vec!["/ip4/127.0.0.1/tcp/40010".parse().unwrap()];

    let entry = client
        .put_peer_manager(name.clone(), manager_peer_id, manager_multiaddrs.clone())
        .await
        .expect("put_peer_manager succeeds");
    assert_eq!(entry.name, name);
    assert_eq!(entry.manager_peer_id, manager_peer_id);
    assert!(entry.created_ns > 0, "Discovery stamps registered_at");
    assert!(
        entry.last_liveness_check_ns >= entry.created_ns,
        "Discovery stamps last_seen at register time"
    );
    eprintln!("Registered domain {name}, created_ns={}", entry.created_ns);

    let upserted = client
        .put_peer_manager(name.clone(), manager_peer_id, manager_multiaddrs.clone())
        .await
        .expect("put_peer_manager upsert succeeds");
    assert_eq!(upserted.name, name);

    let beat = client
        .heartbeat(name.clone(), 3)
        .await
        .expect("heartbeat succeeds");
    assert_eq!(beat.peer_count, 3, "heartbeat updates peer_count");
    assert!(
        beat.last_liveness_check_ns > 0,
        "heartbeat stamps last_seen"
    );

    let new_identity = PeerIdentity::from_seed(&[43u8; 32]);
    let new_peer_id = new_identity.peer_id();
    let new_multiaddrs: Vec<Multiaddr> = vec!["/ip4/127.0.0.1/tcp/40011".parse().unwrap()];
    let rotated = client
        .rotate_manager(name.clone(), new_peer_id, new_multiaddrs.clone())
        .await
        .expect("rotate_manager succeeds");
    assert_eq!(rotated.manager_peer_id, new_peer_id);
    assert_eq!(rotated.manager_multiaddrs, new_multiaddrs);

    client
        .deregister(name.clone())
        .await
        .expect("deregister succeeds");
    let err = client
        .get_peer_manager(name.clone())
        .await
        .expect_err("get_peer_manager after deregister");
    assert!(
        matches!(err, DiscoveryError::Status { status: 404, .. }),
        "expected 404 after deregister, got {err:?}"
    );

    eprintln!("Roundtrip OK against {}", discovery_url());
}

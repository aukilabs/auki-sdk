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
//! 1. `list_clusters` returns a snapshot (empty or otherwise).
//! 2. `create_cluster` with a uniquely-named cluster returns `Created`.
//! 3. A second `create_cluster` with the same name returns
//!    `AlreadyExists`.
//! 4. `liveness_check` updates `peer_count` + bumps `last_liveness_check_ns`.
//! 5. `rotate_manager` swaps the Manager hint.
//! 6. `deregister` removes the cluster; a follow-up `list_clusters`
//!    confirms it's gone.

#![cfg(feature = "discovery_client")]

use auki_network::PeerIdentity;
use auki_network::discovery_client::{CreateClusterOutcome, DiscoveryClient};
use multiaddr::Multiaddr;
use std::time::{SystemTime, UNIX_EPOCH};

fn discovery_url() -> String {
    std::env::var("DISCOVERY_URL").unwrap_or_else(|_| "http://192.168.9.130:8080".to_string())
}

fn unique_cluster_name(prefix: &str) -> String {
    // Discovery accepts ^[A-Za-z0-9_-]{1,64}$. Combine a prefix + epoch
    // nanos (replacing ':') so two concurrent test runs don't collide.
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

    // 1. List — works (any result OK, snapshot can be anything).
    let before = client.list_clusters().await.expect("list_clusters succeeds");
    eprintln!("Before: {} cluster(s) in directory", before.len());

    // 2. Create.
    let name = unique_cluster_name("sdk-it");
    let identity = PeerIdentity::from_seed(&[42u8; 32]);
    let manager_peer_id = identity.peer_id();
    let manager_multiaddrs: Vec<Multiaddr> = vec![
        "/ip4/127.0.0.1/tcp/40010".parse().unwrap(),
    ];

    let outcome = client
        .create_cluster(&name, &manager_peer_id, &manager_multiaddrs)
        .await
        .expect("create_cluster succeeds");
    let entry = match outcome {
        CreateClusterOutcome::Created(e) => e,
        CreateClusterOutcome::AlreadyExists => {
            panic!("a unique-named cluster came back as AlreadyExists — name collision?")
        }
    };
    assert_eq!(entry.name, name);
    assert_eq!(entry.manager_peer_id, manager_peer_id);
    assert_eq!(
        entry.peer_count, 1,
        "Discovery counts the creator as the first peer on create"
    );
    assert!(entry.created_ns > 0, "Discovery stamps created_ns");
    assert!(
        entry.last_liveness_check_ns >= entry.created_ns,
        "Discovery stamps last_liveness_check_ns at create time"
    );
    eprintln!("Created cluster {name}, created_ns={}", entry.created_ns);

    // 3. Second create → AlreadyExists.
    let dup = client
        .create_cluster(&name, &manager_peer_id, &manager_multiaddrs)
        .await
        .expect("create_cluster (duplicate) returns Ok with AlreadyExists");
    assert!(
        matches!(dup, CreateClusterOutcome::AlreadyExists),
        "second create on same name returns AlreadyExists"
    );

    // 4. Liveness check — bumps peer_count + last_liveness_check_ns.
    let beat = client
        .liveness_check(&name, 3)
        .await
        .expect("liveness_check succeeds");
    assert_eq!(beat.peer_count, 3, "liveness_check updates peer_count");
    assert!(
        beat.last_liveness_check_ns > 0,
        "liveness_check stamps last_liveness_check_ns"
    );

    // 5. Rotate Manager — swap to a different peer.
    let new_identity = PeerIdentity::from_seed(&[43u8; 32]);
    let new_peer_id = new_identity.peer_id();
    let new_multiaddrs: Vec<Multiaddr> = vec!["/ip4/127.0.0.1/tcp/40011".parse().unwrap()];
    let rotated = client
        .rotate_manager(&name, &new_peer_id, &new_multiaddrs)
        .await
        .expect("rotate_manager succeeds");
    assert_eq!(
        rotated.manager_peer_id, new_peer_id,
        "Manager peer-id rotated"
    );
    assert_eq!(
        rotated.manager_multiaddrs, new_multiaddrs,
        "Manager multiaddrs rotated"
    );

    // 6. Deregister + verify gone.
    client.deregister(&name).await.expect("deregister succeeds");
    let after = client.list_clusters().await.expect("list_clusters after");
    assert!(
        !after.iter().any(|c| c.name == name),
        "cluster {name} still in directory after deregister"
    );

    eprintln!("Roundtrip OK against {}", discovery_url());
}

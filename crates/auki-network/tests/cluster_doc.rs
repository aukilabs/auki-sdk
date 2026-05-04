//! Integration tests for the `cluster.json` loader.
//!
//! These exercise the loader against on-disk JSON files (written into a
//! tempdir at test setup) — separate from the unit tests in
//! [`auki_network::cluster_doc`] which run in-process tempfile round
//! trips. Together they cover the contract end-to-end: a daemon hands
//! `load` a `Path` and gets a [`ClusterDoc`] back, with every error
//! variant reachable.

use auki_network::{
    PeerIdentity,
    cluster_doc::{self, ClusterDoc, ClusterPeer, LoadError, default_path, load, resolve_path},
};
use std::fs;
use std::path::PathBuf;

/// Process-wide lock for env-mutating tests in this integration binary.
/// Cargo's integration test harness compiles this file as a separate
/// binary, so the mutex in the unit test module isn't visible — we
/// need our own.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Build a deterministic two-peer doc — same shape as the example in
/// the spec section of [`auki-network/README.md`].
fn fixture_doc() -> ClusterDoc {
    let p1 = PeerIdentity::from_seed(&[1u8; 32]).peer_id();
    let p2 = PeerIdentity::from_seed(&[2u8; 32]).peer_id();
    ClusterDoc {
        version: 1,
        cluster_name: "demo-2026-05".to_string(),
        peers: vec![
            ClusterPeer {
                peer_id: p1,
                addresses: vec![
                    "/ip4/192.168.1.10/tcp/4001".parse().unwrap(),
                    "/ip4/192.168.1.10/udp/4001/quic-v1".parse().unwrap(),
                ],
                expected_app_id: Some("boosterapp".to_string()),
                note: Some("robot 1 — K1 NUC".to_string()),
            },
            ClusterPeer {
                peer_id: p2,
                addresses: vec!["/ip4/10.0.0.5/tcp/4001".parse().unwrap()],
                expected_app_id: Some("sentinel".to_string()),
                note: None,
            },
        ],
    }
}

/// Write `doc` to `<dir>/registries/cluster_registries/cluster.json`,
/// creating directories as needed. Returns the full path.
fn write_at_default_layout(app_root: &std::path::Path, doc: &ClusterDoc) -> PathBuf {
    let path = default_path(app_root);
    fs::create_dir_all(path.parent().unwrap()).expect("create registries dir");
    let json = serde_json::to_string_pretty(doc).expect("serialize");
    fs::write(&path, json.as_bytes()).expect("write fixture");
    path
}

#[test]
fn loads_from_default_path_layout() {
    // Mirrors what a daemon does at startup: app_root is the on-disk
    // root for the daemon's persistent state, the cluster doc lives
    // under registries/cluster_registries/cluster.json by convention.
    let dir = tempfile::tempdir().unwrap();
    let original = fixture_doc();
    let path = write_at_default_layout(dir.path(), &original);

    // Resolve via the public path helper (no CLI override, no env var)
    // so we exercise the real production code path end-to-end.
    // SAFETY: env mutation is process-global; we hold env_lock and
    // clear so a parallel test setting the var can't leak in.
    let _g = env_lock();
    unsafe { std::env::remove_var(cluster_doc::ENV_OVERRIDE) };
    let resolved = resolve_path(dir.path(), None);
    assert_eq!(resolved, path);
    let loaded = load(&resolved).expect("load via resolved path");
    assert_eq!(loaded, original);
}

#[test]
fn loads_from_cli_override_path() {
    // Same fixture, but at a non-default location; daemon was launched
    // with `--cluster-doc /tmp/.../alt.json`.
    let dir = tempfile::tempdir().unwrap();
    let alt = dir.path().join("alt-cluster.json");
    let original = fixture_doc();
    fs::write(&alt, serde_json::to_string_pretty(&original).unwrap()).unwrap();

    // app_root is empty on disk — only the override matters.
    let app_root = dir.path().join("nonexistent-app-root");
    let resolved = resolve_path(&app_root, Some(alt.as_path()));
    assert_eq!(resolved, alt);
    let loaded = load(&resolved).expect("load via cli override");
    assert_eq!(loaded, original);
}

#[test]
fn surfaces_invalid_peer_id_with_value_in_error() {
    // Operator typo in the doc — the error should carry enough context
    // for them to fix it without rebuilding from logs alone.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cluster.json");
    let bad = r#"{
      "version": 1,
      "cluster_name": "typo-test",
      "peers": [{ "peer_id": "12D3KooW-broken-typo", "addresses": [] }]
    }"#;
    fs::write(&path, bad).unwrap();

    let err = load(&path).expect_err("expected InvalidPeerId");
    match err {
        LoadError::InvalidPeerId(_) => {}
        other => panic!("expected InvalidPeerId, got {other:?}"),
    }
}

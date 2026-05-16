"""Python-side tests for ``auki_network``.

Run via::

    cd crates/auki-network-py
    python -m venv .venv && source .venv/bin/activate
    pip install maturin pytest
    maturin develop --release
    pytest python_tests/

Two-tier coverage:

1. **Surface tests** — module shape, type construction, getters, error
   mapping. Fast.
2. **Spawn tests** — the real cluster runtime. Includes the
   two-runtime discovery test (cross-language analog of
   ``auki_network::cluster_runtime::tests::two_runtimes_discover_each_other_via_cluster_doc``).
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest

import auki_network
from auki_network import cluster


# ─── Module surface ──────────────────────────────────────────────────────────


def test_module_exposes_only_documented_apis() -> None:
    """Pin the cluster sub-module surface — anything new requires a
    deliberate decision and a changelog bump."""
    public = {name for name in dir(cluster) if not name.startswith("_")}
    expected = {
        # Ansuz surface (v0.0.14).
        "ClusterDoc",
        "ClusterRuntime",
        "ParticipantInfo",
        "PeerSnapshot",
        "load_doc",
        "spawn",
        # Grimsby Stream<T> surface (deliverable #4 / v0.0.17).
        "StreamRequest",
        "StreamDescriptor",
        "JpegFrame",
        # Dagaz Batch 2 (v0.0.21) — pointcloud `T`.
        "PointCloudFrame",
        "DeclineReason",
        "EndReason",
        "ProducerFrame",
        "ConsumerFrame",
        "StreamDecision",
        "StreamSubscription",
        "FrameIterator",
        "StreamEndOfStream",
        "StreamConnectionLost",
        "StreamProtocolError",
        "StreamDeclined",
        "StreamUnreachable",
    }
    assert expected.issubset(public), f"missing expected API: {expected - public}"


def test_top_level_module_exposes_cluster() -> None:
    assert hasattr(auki_network, "cluster")
    # `from auki_network import cluster` works because the wrapper
    # registers the submodule in `sys.modules`.
    from auki_network import cluster as cluster_via_from  # noqa: F401


# ─── ParticipantInfo ─────────────────────────────────────────────────────────


# Seed [0x10; 32] under PeerIdentity::from_seed (direct ed25519, not
# wallet-derived). Computed from the Rust source via
# `cargo test print_python_e2e_peer_ids -- --nocapture` (see
# crates/auki-network-py/src/lib.rs::tests::print_python_e2e_peer_ids).
PEER_ID_SEED_10 = "12D3KooWG3t2M63pjiZP7UHsWruK1tQomm9kMsTm4FS3YMTfE6ao"
PEER_ID_SEED_11 = "12D3KooWPqT2nMDSiXUSx5D7fasaxhxKigVhcqfkKqrLghCq9jxz"


def make_participant_info(
    *,
    app: str = "boosterapp",
    name: str = "k1-walker",
    session_id: str = "11111111-2222-4333-8444-555555555555",
    session_clock_id: str = "K1-AABBCCDDEEFF/session-monotonic",
    session_clock_hash: str = "abc123",
    session_now_ns: int = 12_345_678_900,
    cluster_joined_at_ns: int | None = 1_745_000_000,
    peer_id: str = PEER_ID_SEED_10,
    app_instance: str = "aabbccddeeff",
) -> cluster.ParticipantInfo:
    """Test fixture — builds a ParticipantInfo with reasonable defaults
    so individual tests can override only the field under test."""
    return cluster.ParticipantInfo(
        app=app,
        name=name,
        session_id=session_id,
        session_clock_id=session_clock_id,
        session_clock_hash=session_clock_hash,
        session_now_ns=session_now_ns,
        cluster_joined_at_ns=cluster_joined_at_ns,
        peer_id=peer_id,
        app_instance=app_instance,
    )


def test_participant_info_round_trips_through_constructor_and_getters() -> None:
    p = make_participant_info()
    assert p.app == "boosterapp"
    assert p.name == "k1-walker"
    assert p.session_id == "11111111-2222-4333-8444-555555555555"
    assert p.session_clock_id == "K1-AABBCCDDEEFF/session-monotonic"
    assert p.session_clock_hash == "abc123"
    assert p.session_now_ns == 12_345_678_900
    assert p.cluster_joined_at_ns == 1_745_000_000
    assert p.peer_id == PEER_ID_SEED_10
    assert p.app_instance == "aabbccddeeff"


def test_participant_info_accepts_none_cluster_joined_at_ns() -> None:
    p = make_participant_info(cluster_joined_at_ns=None)
    assert p.cluster_joined_at_ns is None


def test_participant_info_rejects_invalid_peer_id() -> None:
    with pytest.raises(ValueError, match="invalid peer_id"):
        make_participant_info(peer_id="not-a-peer-id")


def test_participant_info_eq_compares_all_fields() -> None:
    a = make_participant_info()
    b = make_participant_info()
    assert a == b
    c = make_participant_info(app="sentinel")
    assert a != c


def test_participant_info_repr_is_informative() -> None:
    p = make_participant_info()
    r = repr(p)
    assert "boosterapp" in r
    assert "k1-walker" in r
    assert PEER_ID_SEED_10 in r


# ─── load_doc ────────────────────────────────────────────────────────────────


def write_cluster_json(path: Path, peers: list[dict] | None = None) -> Path:
    """Write a minimal cluster.json at `path` and return it."""
    doc = {
        "version": 1,
        "cluster_name": "test",
        "peers": peers if peers is not None else [],
    }
    path.write_text(json.dumps(doc))
    return path


def test_load_doc_round_trips_minimal(tmp_path: Path) -> None:
    path = write_cluster_json(tmp_path / "cluster.json")
    doc = cluster.load_doc(str(path))
    assert doc.cluster_name == "test"
    assert doc.peer_count == 0


def test_load_doc_with_peers(tmp_path: Path) -> None:
    path = write_cluster_json(
        tmp_path / "cluster.json",
        peers=[{"peer_id": PEER_ID_SEED_10, "addresses": []}],
    )
    doc = cluster.load_doc(str(path))
    assert doc.peer_count == 1


def test_load_doc_missing_file_raises_oserror(tmp_path: Path) -> None:
    with pytest.raises(OSError):
        cluster.load_doc(str(tmp_path / "does-not-exist.json"))


def test_load_doc_invalid_json_raises_value_error(tmp_path: Path) -> None:
    path = tmp_path / "cluster.json"
    path.write_text("not valid json {")
    with pytest.raises(ValueError):
        cluster.load_doc(str(path))


def test_load_doc_unsupported_version_raises_value_error(tmp_path: Path) -> None:
    path = tmp_path / "cluster.json"
    path.write_text(json.dumps({"version": 99, "cluster_name": "x", "peers": []}))
    with pytest.raises(ValueError, match="unsupported version 99"):
        cluster.load_doc(str(path))


def test_load_doc_invalid_peer_id_raises_value_error(tmp_path: Path) -> None:
    path = write_cluster_json(
        tmp_path / "cluster.json",
        peers=[{"peer_id": "not-a-peer-id", "addresses": []}],
    )
    with pytest.raises(ValueError, match="invalid peer_id"):
        cluster.load_doc(str(path))


# ─── cluster.spawn — argument validation ─────────────────────────────────────


def test_spawn_rejects_wrong_seed_length(tmp_path: Path) -> None:
    path = write_cluster_json(tmp_path / "cluster.json")
    doc = cluster.load_doc(str(path))
    with pytest.raises(ValueError, match="32 bytes"):
        cluster.spawn(
            seed=b"\x00" * 16,
            doc=doc,
            participant_provider=lambda: None,
        )


def test_spawn_rejects_invalid_listen_multiaddr(tmp_path: Path) -> None:
    path = write_cluster_json(tmp_path / "cluster.json")
    doc = cluster.load_doc(str(path))
    with pytest.raises(ValueError, match="invalid multiaddr"):
        cluster.spawn(
            seed=b"\x00" * 32,
            doc=doc,
            participant_provider=lambda: None,
            listen_addresses=["not a multiaddr"],
        )


# ─── cluster.spawn — happy path ──────────────────────────────────────────────


def test_spawn_then_shutdown_round_trip(tmp_path: Path) -> None:
    """Spawn an empty cluster (no peers), verify peers() returns [],
    shutdown succeeds, second shutdown raises, post-shutdown peers()
    raises."""
    path = write_cluster_json(tmp_path / "cluster.json")
    doc = cluster.load_doc(str(path))

    runtime = cluster.spawn(
        seed=b"\x42" * 32,
        doc=doc,
        participant_provider=lambda: None,
        listen_addresses=["/ip4/127.0.0.1/tcp/0"],
        enable_mdns=False,
    )

    # Empty cluster — no peers visible, peers() is callable.
    assert runtime.peers() == []

    runtime.shutdown()  # consumes; first shutdown succeeds

    with pytest.raises(RuntimeError, match="shut down"):
        runtime.shutdown()
    with pytest.raises(RuntimeError, match="shut down"):
        runtime.peers()


# ─── cluster.spawn — two-runtime discovery (the big one) ─────────────────────


# Fixed loopback ports for the two-runtime test. If these conflict with
# something on the host, adjust. Two ports because each runtime needs its
# own address; can't use OS-chosen because the cluster.json must list the
# address each peer is reachable at, and the wrapper has no introspection
# API for the bound address (consumer would normally hand-write
# cluster.json with known ports).
TEST_PORT_A = 45051
TEST_PORT_B = 45052


def _build_two_peer_doc(tmp_path: Path) -> cluster.ClusterDoc:
    """Build a cluster.json listing both fixed-port peers and load it."""
    path = write_cluster_json(
        tmp_path / "cluster.json",
        peers=[
            {
                "peer_id": PEER_ID_SEED_10,
                "addresses": [f"/ip4/127.0.0.1/tcp/{TEST_PORT_A}"],
            },
            {
                "peer_id": PEER_ID_SEED_11,
                "addresses": [f"/ip4/127.0.0.1/tcp/{TEST_PORT_B}"],
            },
        ],
    )
    return cluster.load_doc(str(path))


def _provider_for(peer_id: str, app: str, name: str):
    """Build a participant_provider closure that returns a fresh
    ParticipantInfo on each call (with `session_now_ns` updated to the
    current monotonic time so the cluster runtime's per-request fresh-
    ness assertion holds)."""

    def provider() -> cluster.ParticipantInfo:
        return cluster.ParticipantInfo(
            app=app,
            name=name,
            session_id=f"session-{name}",
            session_clock_id=f"{name}/clock",
            session_clock_hash="deadbeef",
            session_now_ns=time.monotonic_ns(),
            cluster_joined_at_ns=None,
            peer_id=peer_id,
            app_instance="00163eabcdef",
        )

    return provider


def test_two_runtimes_discover_each_other_via_cluster_doc(tmp_path: Path) -> None:
    """Cross-language analog of the Rust `two_runtimes_discover_each_other_via_cluster_doc`.

    Two `cluster.spawn` instances against the same cluster.json — both peers
    listed with fixed loopback addresses — should discover each other and
    converge on each other's `ParticipantInfo` within a few seconds.
    """
    doc = _build_two_peer_doc(tmp_path)

    rt_a = cluster.spawn(
        seed=b"\x10" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_10, "boosterapp", "robot-a"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{TEST_PORT_A}"],
        enable_mdns=False,
    )
    rt_b = cluster.spawn(
        seed=b"\x11" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_11, "sentinel", "sentinel-b"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{TEST_PORT_B}"],
        enable_mdns=False,
    )

    try:
        # Poll for convergence — each side sees exactly one peer (the
        # other). 10s budget mirrors the Rust test's deadline.
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if len(rt_a.peers()) == 1 and len(rt_b.peers()) == 1:
                break
            time.sleep(0.1)

        a_peers = rt_a.peers()
        b_peers = rt_b.peers()
        assert len(a_peers) == 1, (
            f"rt_a sees {len(a_peers)} peers, expected 1; b sees {len(b_peers)}"
        )
        assert len(b_peers) == 1, f"rt_b sees {len(b_peers)} peers, expected 1"

        # Each side sees the other, not itself.
        assert a_peers[0].peer_id == PEER_ID_SEED_11
        assert a_peers[0].info.app == "sentinel"
        assert a_peers[0].info.name == "sentinel-b"
        assert b_peers[0].peer_id == PEER_ID_SEED_10
        assert b_peers[0].info.app == "boosterapp"
        assert b_peers[0].info.name == "robot-a"

        # `first_seen_ns` is the peer's `session_now_ns` at first response —
        # set, monotonic, non-zero.
        assert a_peers[0].first_seen_ns > 0
        assert b_peers[0].first_seen_ns > 0
    finally:
        rt_a.shutdown()
        rt_b.shutdown()


# ─── Provider error path ─────────────────────────────────────────────────────


def test_spawn_with_raising_provider_does_not_panic(tmp_path: Path) -> None:
    """A provider that raises on every call should be caught + logged
    by the wrapper, returning `None` to the runtime. With no peers in
    the doc the provider is never actually invoked, but spawn itself
    must accept the closure without panicking on the (synchronous)
    setup."""

    def bad_provider() -> cluster.ParticipantInfo:
        raise RuntimeError("boom")

    doc = cluster.load_doc(str(write_cluster_json(tmp_path / "cluster.json")))
    rt = cluster.spawn(
        seed=b"\x99" * 32,
        doc=doc,
        participant_provider=bad_provider,
        listen_addresses=["/ip4/127.0.0.1/tcp/0"],
        enable_mdns=False,
    )
    # Runtime is alive — provider is never invoked because the doc has
    # no peers. peers() returns [] without touching the provider.
    assert rt.peers() == []
    rt.shutdown()

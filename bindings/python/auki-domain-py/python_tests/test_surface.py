"""Smoke tests for the post-#216 `auki_domain` Python module surface.

Run after building the wheel:

    maturin develop -m bindings/python/auki-domain-py/Cargo.toml
    pytest bindings/python/auki-domain-py/python_tests/
"""

from __future__ import annotations

import inspect
import json
import pathlib

import pytest


def test_module_imports_post_216_surface() -> None:
    """Verify the module exposes the post-#216 public surface."""
    import auki_domain

    # Core cluster types — unchanged from v0.0.52
    assert hasattr(auki_domain, "ClusterMember")
    assert hasattr(auki_domain, "ClusterMembership")
    assert hasattr(auki_domain, "DaemonInfo")
    assert hasattr(auki_domain, "ParticipantInfo")
    # Post-#216 resource catalog type (replaces SensorStreamResource etc.)
    assert hasattr(auki_domain, "ResourceEntry")
    assert hasattr(auki_domain, "MessageEvent")
    assert hasattr(auki_domain, "MessageChannelResource")
    assert hasattr(auki_domain, "MessageChannelReceiver")
    assert hasattr(auki_domain, "MessageChannelSender")
    for attr in ("resource_id", "sender_peer_id", "type", "timestamp_ns", "payload"):
        assert hasattr(auki_domain.MessageEvent, attr)

    # Post-#216 stream request types
    assert hasattr(auki_domain, "ReadFrom")
    assert hasattr(auki_domain, "StreamRequest")

    # Manifest builder + cluster control types
    assert hasattr(auki_domain, "StreamManifestBuilder")
    assert hasattr(auki_domain, "ClusterTarget")
    assert hasattr(auki_domain, "ClusterManager")

    # ClusterManager has the expected stream-open methods
    assert hasattr(auki_domain.ClusterManager, "open_stream")
    assert hasattr(auki_domain.ClusterManager, "open_pose_stream")
    assert hasattr(auki_domain.ClusterManager, "open_detection_stream")
    assert hasattr(auki_domain.ClusterManager, "open_camera_stream")
    assert hasattr(auki_domain.ClusterManager, "open_stream_with_request")
    assert hasattr(auki_domain.ClusterManager, "fetch_detector_entry")
    assert hasattr(auki_domain.ClusterManager, "register_message_channel")
    assert hasattr(auki_domain.ClusterManager, "fetch_message_channels")
    assert hasattr(auki_domain.ClusterManager, "open_message_channel")
    signature = inspect.signature(auki_domain.ClusterManager.register_message_channel)
    assert list(signature.parameters) == ["self", "resource_id", "capacity"]
    assert signature.parameters["capacity"].default == 64
    open_signature = inspect.signature(auki_domain.ClusterManager.open_message_channel)
    assert list(open_signature.parameters) == ["self", "resource"]
    send_signature = inspect.signature(auki_domain.MessageChannelSender.send)
    assert list(send_signature.parameters) == [
        "self",
        "message_type",
        "timestamp_ns",
        "payload",
    ]

    # Old deleted types must NOT be present
    assert not hasattr(auki_domain, "SensorEntry")
    assert not hasattr(auki_domain, "SensorStreamResource")
    assert not hasattr(auki_domain, "TransformEdgeResource")
    assert not hasattr(auki_domain, "PoseStreamResource")
    assert not hasattr(auki_domain, "ResourcePinholeIntrinsics")
    assert not hasattr(auki_domain, "ResourceVec3")
    assert not hasattr(auki_domain, "ResourceQuat")
    assert not hasattr(auki_domain, "ResourceSpatialTransform")


def test_cluster_target_factories() -> None:
    import auki_domain

    target = auki_domain.ClusterTarget.most_recent_or_create("hagall")
    assert target.kind == "most_recent_or_create"
    assert target.name == "hagall"
    assert "hagall" in repr(target)

    create_t = auki_domain.ClusterTarget.create("my-cluster")
    assert create_t.kind == "create"
    assert create_t.name == "my-cluster"

    join_t = auki_domain.ClusterTarget.join("other-cluster")
    assert join_t.kind == "join"

    joc_t = auki_domain.ClusterTarget.join_or_create("flex-cluster")
    assert joc_t.kind == "join_or_create"


def test_stream_manifest_builder_missing_sensor_is_file_not_found(
    tmp_path: pathlib.Path,
) -> None:
    import auki_domain

    with pytest.raises(FileNotFoundError):
        auki_domain.StreamManifestBuilder.from_registry(
            tmp_path,
            "missing-peer",
            "missing/sensor",
            "missing-hash",
            "clock",
            "clock-hash",
        )


# ─── ReadFrom tests ──────────────────────────────────────────────────────────


def test_read_from_factories() -> None:
    import auki_domain

    latest = auki_domain.ReadFrom.latest()
    assert latest.kind == "latest"
    assert latest.timestamp_ns is None

    from_start = auki_domain.ReadFrom.from_start()
    assert from_start.kind == "from_start"
    assert from_start.timestamp_ns is None

    ts = auki_domain.ReadFrom.from_timestamp(1_733_836_800_000_000_000)
    assert ts.kind == "from_timestamp"
    assert ts.timestamp_ns == 1_733_836_800_000_000_000


def test_read_from_repr() -> None:
    import auki_domain

    assert "latest" in repr(auki_domain.ReadFrom.latest())
    assert "from_start" in repr(auki_domain.ReadFrom.from_start())
    assert "42" in repr(auki_domain.ReadFrom.from_timestamp(42))


def test_read_from_equality() -> None:
    import auki_domain

    assert auki_domain.ReadFrom.latest() == auki_domain.ReadFrom.latest()
    assert auki_domain.ReadFrom.from_start() == auki_domain.ReadFrom.from_start()
    assert auki_domain.ReadFrom.from_timestamp(100) == auki_domain.ReadFrom.from_timestamp(100)
    assert auki_domain.ReadFrom.latest() != auki_domain.ReadFrom.from_start()
    assert auki_domain.ReadFrom.from_timestamp(1) != auki_domain.ReadFrom.from_timestamp(2)


# ─── StreamRequest tests ─────────────────────────────────────────────────────


def test_stream_request_construction() -> None:
    import auki_domain

    req = auki_domain.StreamRequest(
        resource_id="head_left_rgb",
        source_peer_id="galbot",
        from_=auki_domain.ReadFrom.latest(),
    )
    assert req.resource_id == "head_left_rgb"
    assert req.source_peer_id == "galbot"
    assert req.from_.kind == "latest"


def test_stream_request_defaults() -> None:
    import auki_domain

    req = auki_domain.StreamRequest(resource_id="my_resource")
    assert req.source_peer_id == ""
    assert req.from_.kind == "latest"


def test_stream_request_from_timestamp() -> None:
    import auki_domain

    req = auki_domain.StreamRequest(
        resource_id="some_log",
        from_=auki_domain.ReadFrom.from_timestamp(1_234_567_890),
    )
    assert req.from_.kind == "from_timestamp"
    assert req.from_.timestamp_ns == 1_234_567_890


# ─── ResourceEntry class surface test ────────────────────────────────────────


def test_resource_entry_class_is_exported() -> None:
    """ResourceEntry is on the module and has the expected attribute names."""
    import auki_domain

    cls = auki_domain.ResourceEntry
    assert cls is not None
    for attr in (
        "source_peer_id",
        "writer_peer_id",
        "resource_id",
        "variant",
        "state",
        "head",
        "extent",
        "available",
        "sensor",
        "pose",
        "manifest",
        "to_json",
    ):
        assert hasattr(cls, attr), f"ResourceEntry missing attribute {attr!r}"


# ─── ClusterMembership / ClusterMember tests ──────────────────────────────────


def test_cluster_membership_json_round_trip() -> None:
    import auki_domain

    membership = auki_domain.ClusterMembership("test-cluster")
    json_str = membership.to_json()
    parsed = json.loads(json_str)
    assert parsed["cluster_name"] == "test-cluster"
    assert parsed["peers"] == []

    restored = auki_domain.ClusterMembership.from_json(json_str)
    assert restored.cluster_name == "test-cluster"
    assert restored.peers == []
    assert restored == membership


def test_cluster_member_value_type() -> None:
    import auki_domain

    # Use a real libp2p peer id for parsing test
    peer_id = "12D3KooWJfVjn3XAFv5XnuACSMsPB3Uh8nCqC7zkKMNpJkgjKZBW"
    m = auki_domain.ClusterMember(
        peer_id=peer_id,
        multiaddrs=["/ip4/127.0.0.1/tcp/4001"],
        join_ts_ns=1_000_000,
    )
    assert m.peer_id == peer_id
    assert m.multiaddrs == ["/ip4/127.0.0.1/tcp/4001"]
    assert m.join_ts_ns == 1_000_000
    assert m.successor_token is None
    assert peer_id in repr(m)


def test_daemon_info_construction() -> None:
    import auki_domain

    info = auki_domain.DaemonInfo(
        app="boosterapp",
        name="K1-daemon",
        session_id="session-123",
        session_clock_id="K1/sdk_clock",
        session_clock_hash="clock-hash",
        app_instance="instance-456",
    )
    # DaemonInfo is an opaque constructor (no getters in Python surface);
    # just verify it constructs without error.
    assert info is not None

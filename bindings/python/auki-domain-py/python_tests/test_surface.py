"""The Stage 1 Python API is the authenticated Rust Domain, not a cluster facade."""

from __future__ import annotations

import pytest


REQUIRED_CLASSES = {
    "Identity",
    "DdsVerificationKeys",
    "SignedP2pCredential",
    "DomainConfig",
    "DomainBuilder",
    "Domain",
    "DomainAuthority",
    "DomainStatus",
    "DomainStatusSubscription",
    "DomainRoutes",
    "DomainRouteSnapshot",
    "KnownPeers",
    "KnownPeer",
    "KnownPeerEvent",
    "ResourceEntry",
    "MapLogResource",
    "MessageChannelResource",
    "MessageChannelReceiver",
    "MessageChannelSender",
    "StreamRequest",
    "StreamSubscription",
    "StreamManifest",
    "StreamManifestBuilder",
    "StreamEntry",
    "StreamItem",
    "StreamDecision",
    "DeclineReason",
    "DynamicIntrinsics",
    "CameraFrame",
    "PointCloudFrame",
    "JointEncodersFrame",
    "AudioFrame",
    "ScalarFrame",
    "DetectionFrame",
    "SpatialTransformFrame",
    "MapUpdateFrame",
    "ReadFrom",
}

REMOVED_CLASSES = {
    "ClusterManager",
    "ClusterTarget",
    "ClusterMembership",
    "ClusterMember",
    "DiscoveryClient",
    "NetworkRuntime",
}

REMOVED_DOMAIN_METHODS = {
    "bootstrap",
    "create_cluster",
    "join_cluster",
    "list_clusters",
    "membership",
    "admit_peer",
    "is_manager",
    "manager_peer_id",
    "heartbeat",
    "domain_time",
    "book_relay",
    "shutdown",
}


def test_authenticated_domain_surface_and_removed_control_plane() -> None:
    import auki_domain

    for name in REQUIRED_CLASSES:
        assert hasattr(auki_domain, name), name
    for name in REMOVED_CLASSES:
        assert not hasattr(auki_domain, name), name
    for name in REMOVED_DOMAIN_METHODS:
        assert not hasattr(auki_domain.Domain, name), name

    for name in (
        "builder",
        "authority",
        "status",
        "subscribe_status",
        "routes",
        "known_peers",
        "catalog",
        "fetch_resources_catalog",
        "list_registry_entries",
        "fetch_blob",
        "open_message_channel",
        "send_message",
        "open_stream",
        "leave",
    ):
        assert hasattr(auki_domain.Domain, name), name

    for name in (
        "authority",
        "participant_info",
        "participant_info_provider",
        "resource_catalog_provider",
        "map_catalog_provider",
        "stream_provider",
        "registry_app_root",
        "message_channel",
        "join",
    ):
        assert hasattr(auki_domain.DomainBuilder, name), name


def test_credentials_are_redacted_and_identity_round_trips() -> None:
    import auki_domain

    identity = auki_domain.Identity.from_ed25519_seed(bytes([7]) * 32)
    restored = auki_domain.Identity.from_protobuf_encoding(identity.to_protobuf_encoding())
    assert restored.peer_id == identity.peer_id

    credential = auki_domain.SignedP2pCredential("header.payload.signature")
    assert "header" not in repr(credential)
    assert "payload" not in repr(credential)
    assert "signature" not in repr(credential)


def test_read_from_and_map_resource_compatibility() -> None:
    import auki_domain

    assert auki_domain.ReadFrom.latest() == auki_domain.ReadFrom.latest()
    assert auki_domain.ReadFrom.from_start() != auki_domain.ReadFrom.latest()
    assert auki_domain.ReadFrom.from_timestamp(42).timestamp_ns == 42

    row = auki_domain.MapLogResource.from_dict(
        {
            "source_peer_id": "source",
            "writer_peer_id": "writer",
            "resource_id": "map-log",
            "map": {"peer_id": "source", "id": "map", "hash": "map-hash"},
            "clock": {"peer_id": "writer", "id": "clock", "hash": "clock-hash"},
        }
    )
    assert row.map == {"peer_id": "source", "id": "map", "hash": "map-hash"}
    assert row.clock == {"peer_id": "writer", "id": "clock", "hash": "clock-hash"}


def test_domain_builder_rejects_unversioned_or_wrong_session_handles() -> None:
    import auki_domain

    identity = auki_domain.Identity.from_ed25519_seed(bytes([8]) * 32)
    config = auki_domain.DomainConfig("11111111-1111-4111-8111-111111111111", identity)
    with pytest.raises(TypeError, match="same SDK release"):
        auki_domain.Domain.builder(object(), object(), config)

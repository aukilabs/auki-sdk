from __future__ import annotations

import json

import pytest


def test_module_exposes_the_small_peer_facade() -> None:
    import auki_sdk

    assert auki_sdk.AukiSession.__name__ == "AukiSession"
    assert auki_sdk.AukiDomain.__name__ == "AukiDomain"
    assert auki_sdk.AukiPeerRoutes.__name__ == "AukiPeerRoutes"
    assert auki_sdk.AukiDiscoveryCandidate.__name__ == "AukiDiscoveryCandidate"
    assert auki_sdk.AukiPeer.__name__ == "AukiPeer"
    assert hasattr(auki_sdk.AukiSession, "login_dev")
    assert hasattr(auki_sdk.AukiSession, "login_app_dev")
    assert hasattr(auki_sdk.AukiPeerRoutes, "tcp")
    assert hasattr(auki_sdk.AukiPeerRoutes, "wss")
    assert hasattr(auki_sdk.AukiPeer, "routes")
    assert hasattr(auki_sdk.AukiPeer, "discover")
    assert hasattr(auki_sdk.AukiPeer, "discover_protocol")
    assert hasattr(auki_sdk.AukiPeer, "shutdown")


def test_module_exposes_every_standard_protocol_role() -> None:
    import auki_sdk

    expected = {
        "AukiInfoClient": ("protocol", "fetch", "fetch_exact"),
        "AukiInfoEndpoint": ("mount", "protocol", "client", "close"),
        "AukiCatalogClient": (
            "fetch_resources",
            "fetch_resources_exact",
            "fetch_maps",
            "fetch_maps_exact",
        ),
        "AukiCatalogEndpoint": (
            "mount",
            "resource_protocol",
            "maps_protocol",
            "client",
            "close",
        ),
        "AukiRegistryClient": (
            "protocol",
            "list",
            "list_exact",
            "fetch",
            "fetch_exact",
        ),
        "AukiRegistryEndpoint": ("mount", "protocol", "client", "close"),
        "AukiBlobClient": ("protocol", "fetch", "fetch_exact"),
        "AukiBlobEndpoint": ("mount", "protocol", "client", "close"),
        "AukiMessageClient": ("protocol", "open", "open_exact"),
        "AukiMessageEndpoint": (
            "mount",
            "protocol",
            "client",
            "declare",
            "catalog",
            "close",
        ),
        "AukiMessageSender": (
            "remote_peer",
            "channel",
            "relayed",
            "send",
            "close",
        ),
        "AukiMessageReceiver": ("channel", "next", "close"),
        "AukiStreamClient": ("protocol", "subscribe", "subscribe_exact"),
        "AukiStreamEndpoint": ("mount", "protocol", "client", "close"),
        "AukiStreamSubscription": (
            "payload_kind",
            "manifest",
            "next",
            "cancel",
        ),
    }

    for class_name, members in expected.items():
        protocol_class = getattr(auki_sdk, class_name)
        for member in members:
            assert hasattr(protocol_class, member), f"{class_name}.{member} is missing"


def test_protocol_preparation_helpers_use_canonical_rust_codecs() -> None:
    import auki_sdk

    for name in (
        "prepare_catalog_resources",
        "prepare_registry_entry",
        "encode_camera_frame_image",
        "decode_camera_frame_image",
    ):
        assert callable(getattr(auki_sdk, name))

    jpeg = bytes.fromhex("ffd8ffe000104a464946")
    encoded = auki_sdk.encode_camera_frame_image(jpeg)
    assert encoded.hex() == "120affd8ffe000104a464946"
    assert auki_sdk.decode_camera_frame_image(encoded) == jpeg
    with pytest.raises(ValueError, match="decode CameraFrame"):
        auki_sdk.decode_camera_frame_image(b"not protobuf")

    assert auki_sdk.prepare_catalog_resources({"resources": []}) == {"resources": []}
    with pytest.raises(ValueError, match="Catalog resources snapshot"):
        auki_sdk.prepare_catalog_resources({"resources": [], "unexpected": True})

    entry = {
        "peer_id": "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan",
        "frame_id": "camera/optical",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "meters",
    }
    envelope = auki_sdk.prepare_registry_entry("frame", entry)
    assert envelope["kind"] == "frame"
    assert envelope["id"] == "camera/optical"
    assert len(envelope["hash"]) == 32
    assert json.loads(envelope["canonical_json"]) == entry

    invalid = {**entry, "frame_id": "camera optical"}
    with pytest.raises(ValueError, match="invalid frame id"):
        auki_sdk.prepare_registry_entry("frame", invalid)

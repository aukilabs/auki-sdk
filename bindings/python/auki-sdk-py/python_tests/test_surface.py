from __future__ import annotations


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

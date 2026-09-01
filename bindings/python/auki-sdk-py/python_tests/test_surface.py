from __future__ import annotations


def test_module_exposes_the_small_peer_facade() -> None:
    import auki_sdk

    assert auki_sdk.AukiSession.__name__ == "AukiSession"
    assert auki_sdk.AukiDomain.__name__ == "AukiDomain"
    assert auki_sdk.AukiPeerRoutes.__name__ == "AukiPeerRoutes"
    assert auki_sdk.AukiPeer.__name__ == "AukiPeer"
    assert hasattr(auki_sdk.AukiSession, "login_dev")
    assert hasattr(auki_sdk.AukiSession, "login_app_dev")
    assert hasattr(auki_sdk.AukiPeerRoutes, "tcp")
    assert hasattr(auki_sdk.AukiPeerRoutes, "wss")
    assert hasattr(auki_sdk.AukiPeer, "routes")
    assert hasattr(auki_sdk.AukiPeer, "shutdown")

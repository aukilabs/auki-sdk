from __future__ import annotations


def test_one_module_exposes_the_peer_and_echo_facades() -> None:
    import auki_portable_echo

    assert auki_portable_echo.AukiSession.__name__ == "AukiSession"
    assert auki_portable_echo.AukiDomain.__name__ == "AukiDomain"
    assert auki_portable_echo.AukiPeerRoutes.__name__ == "AukiPeerRoutes"
    assert auki_portable_echo.AukiPeer.__name__ == "AukiPeer"
    assert auki_portable_echo.AukiEcho.__name__ == "AukiEcho"
    assert auki_portable_echo.EchoReceipt.__name__ == "EchoReceipt"


def test_session_surface_keeps_authentication_and_startup_in_rust() -> None:
    from auki_portable_echo import AukiSession

    assert hasattr(AukiSession, "login_dev")
    assert hasattr(AukiSession, "login_app_dev")
    assert hasattr(AukiSession, "accessible_domains")
    assert hasattr(AukiSession, "start_peer")


def test_peer_surface_exposes_routes_and_ordered_lifecycle() -> None:
    from auki_portable_echo import AukiPeer

    for member in (
        "peer_id",
        "domain_id",
        "routes",
        "wait_stopped",
        "shutdown",
    ):
        assert hasattr(AukiPeer, member)


def test_echo_surface_uses_exact_routes_and_explicit_cleanup() -> None:
    from auki_portable_echo import AukiEcho, EchoReceipt

    for member in ("mount", "protocol", "send_exact", "next_served", "close"):
        assert hasattr(AukiEcho, member)
    for member in ("remote_peer_id", "payload"):
        assert hasattr(EchoReceipt, member)

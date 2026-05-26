#!/usr/bin/env python3
from __future__ import annotations

import concurrent.futures
import json
import os
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "bindings" / "python" / "auki-network"))

import auki_network  # noqa: E402


def runtime(
    wallet_seed: bytes,
    listen_multiaddrs: list[str],
    allowed_peers: list[auki_network.BindingAllowedPeer],
) -> auki_network.AukiNetworkRuntime:
    return auki_network.AukiNetworkRuntime.spawn(
        auki_network.BindingSwarmConfig(
            wallet_seed=wallet_seed,
            listen_multiaddrs=listen_multiaddrs,
            agent_version="auki-network-python-smoke/0.1",
            allowed_peers=allowed_peers,
            heartbeat_clock_id=None,
            heartbeat_clock_hash_hex=None,
        )
    )


def wait_for(label: str, ready, timeout_s: float = 60.0):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        value = ready()
        if value:
            return value
        time.sleep(0.02)
    raise TimeoutError(f"timed out waiting for {label}")


def first_tcp_listen_addr(runtime: auki_network.AukiNetworkRuntime) -> str | None:
    for addr in runtime.listen_multiaddrs():
        if "/tcp/" in addr and not addr.endswith("/tcp/0"):
            return addr
    return None


def main() -> None:
    seed = bytes([3]) * 32
    expected_peer = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
    assert auki_network.peer_id_from_wallet_seed(seed) == expected_peer

    a_seed = os.urandom(32)
    b_seed = os.urandom(32)
    a_peer = auki_network.peer_id_from_wallet_seed(a_seed)
    b_peer = auki_network.peer_id_from_wallet_seed(b_seed)

    a = runtime(a_seed, ["/ip4/127.0.0.1/tcp/0"], [])
    b = runtime(b_seed, ["/ip4/127.0.0.1/tcp/0"], [])
    try:
        assert a.local_peer_id() == a_peer
        assert b.local_peer_id() == b_peer
        a_addr = wait_for("runtime A listen address", lambda: first_tcp_listen_addr(a))
        b_addr = wait_for("runtime B listen address", lambda: first_tcp_listen_addr(b))

        a_allowed = [auki_network.BindingAllowedPeer(peer_id=b_peer, multiaddrs=[b_addr])]
        b_allowed = [auki_network.BindingAllowedPeer(peer_id=a_peer, multiaddrs=[a_addr])]
        a.set_allowed_peers(a_allowed)
        b.set_allowed_peers(b_allowed)

        def peers_connected() -> bool:
            connected = b_peer in a.connected_peers() and a_peer in b.connected_peers()
            if not connected:
                a.set_allowed_peers(a_allowed)
                b.set_allowed_peers(b_allowed)
            return connected

        wait_for("runtime peers connected", peers_connected)

        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(
                a.send_join_request_json,
                b_peer,
                json.dumps({"multiaddrs": []}, separators=(",", ":")),
                5_000,
            )
            event = wait_for(
                "join request event",
                lambda: next(iter(b.drain_join_requests(10)), None),
            )
            assert event.kind == "join_request"
            assert event.peer_id == a_peer
            assert json.loads(event.payload_json) == {"multiaddrs": []}
            b.respond_join_json(
                event.responder_id,
                json.dumps(
                    {"kind": "reject", "reason": "python-smoke"},
                    separators=(",", ":"),
                ),
            )
            response = pending.result(timeout=5)

        assert response.peer_id == b_peer
        assert json.loads(response.payload_json) == {
            "kind": "reject",
            "reason": "python-smoke",
        }
    finally:
        a.shutdown()
        b.shutdown()


if __name__ == "__main__":
    main()

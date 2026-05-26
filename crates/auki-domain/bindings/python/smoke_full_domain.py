#!/usr/bin/env python3
from __future__ import annotations

import asyncio
import contextlib
import json
import socket
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "bindings" / "python" / "auki-network"))
sys.path.insert(0, str(ROOT / "bindings" / "python" / "auki-domain"))

import auki_domain  # noqa: E402
import auki_network  # noqa: E402


class MockDiscoveryServer:
    def __init__(self, cluster_name: str, manager_peer_id: str):
        self.cluster_name = cluster_name
        self.manager_peer_id = manager_peer_id

        server_ref = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                if self.path == "/clusters":
                    self._json(
                        200,
                        {"clusters": [server_ref.entry()]},
                    )
                elif self.path == f"/clusters/{server_ref.cluster_name}/liveness":
                    self._json(200, server_ref.entry())
                else:
                    self._json(404, {"error": "unexpected path"})

            def do_POST(self) -> None:
                if self.path == f"/clusters/{server_ref.cluster_name}":
                    self._json(201, server_ref.entry())
                else:
                    self._json(404, {"error": "unexpected path"})

            def do_DELETE(self) -> None:
                if self.path == f"/clusters/{server_ref.cluster_name}":
                    self.send_response(204)
                    self.send_header("content-length", "0")
                    self.end_headers()
                else:
                    self._json(404, {"error": "unexpected path"})

            def log_message(self, _format: str, *_args) -> None:
                return

            def _json(self, status: int, body: dict) -> None:
                encoded = json.dumps(body, separators=(",", ":")).encode()
                self.send_response(status)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def entry(self) -> dict:
        return {
            "name": self.cluster_name,
            "manager_peer_id": self.manager_peer_id,
            "manager_multiaddrs": ["/ip4/127.0.0.1/tcp/48000"],
            "peer_count": 1,
            "created_ns": 1,
            "last_liveness_check_ns": 1,
        }

    @property
    def base_url(self) -> str:
        host, port = self._server.server_address
        return f"http://{host}:{port}"

    def shutdown(self) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=2)


def reserve_local_multiaddr() -> str:
    with contextlib.closing(socket.socket(socket.AF_INET, socket.SOCK_STREAM)) as sock:
        sock.bind(("127.0.0.1", 0))
        return f"/ip4/127.0.0.1/tcp/{sock.getsockname()[1]}"


async def bootstrap_manager(cluster_name: str, seed_byte: int):
    seed = bytes([seed_byte]) * 32
    server = MockDiscoveryServer(
        cluster_name,
        auki_network.peer_id_from_wallet_seed(seed),
    )
    multiaddr = reserve_local_multiaddr()
    manager = await auki_domain.bootstrap_domain_cluster_manager(
        auki_domain.ClusterTargetMode.CREATE,
        cluster_name,
        seed,
        [multiaddr],
        [multiaddr],
        server.base_url,
        auki_domain.DaemonInfo(
            app="python-smoke",
            name=f"peer-{seed_byte}",
            session_id=f"session-{seed_byte}",
            session_clock_id="legacy-clock",
            session_clock_hash="legacy-clock-hash",
            app_instance="00163eabcdef",
        ),
        "auki-domain-python-smoke/0.1",
    )
    return server, manager


async def wait_for_json(label: str, operation, timeout_s: float = 10.0) -> str:
    deadline = time.monotonic() + timeout_s
    last_error = None
    while time.monotonic() < deadline:
        try:
            return await operation()
        except Exception as exc:  # noqa: BLE001 - smoke retries transient peer setup.
            last_error = exc
            await asyncio.sleep(0.05)
    raise TimeoutError(f"timed out waiting for {label}: {last_error!r}")


async def main() -> None:
    servers = []
    managers = []
    try:
        server_a, a = await bootstrap_manager("python-smoke-a", 41)
        server_b, b = await bootstrap_manager("python-smoke-b", 42)
        servers.extend([server_a, server_b])
        managers.extend([a, b])

        assert a.cluster_name() == "python-smoke-a"
        assert a.local_peer_id()
        membership = json.loads(a.membership_json())
        assert membership["cluster_name"] == "python-smoke-a"
        assert len(membership["peers"]) == 1

        await a.admit_peer(b.local_peer_id(), b.local_multiaddrs())
        await b.admit_peer(a.local_peer_id(), a.local_multiaddrs())

        info = json.loads(
            await wait_for_json(
                "participant info",
                lambda: a.fetch_participant_info_json(b.local_peer_id()),
            )
        )
        assert info["peer_id"] == b.local_peer_id()
        assert info["app"] == "python-smoke"

        b.set_static_sensor_catalog_json(
            json.dumps(
                {
                    "sensors": [
                        {
                            "sensor_id": "python-camera",
                            "sensor_hash": "sensor-hash",
                            "kind": "camera",
                        }
                    ]
                },
                separators=(",", ":"),
            )
        )
        b.set_static_resource_catalog_json(
            json.dumps(
                {
                    "resources": [
                        {
                            "kind": "sensor_stream",
                            "id": "python-camera",
                            "sensor_id": "python-camera",
                            "sensor_hash": "sensor-hash",
                            "sensor_kind": "camera",
                            "stream_protocol": "/auki/stream/0.1.0",
                            "payload": "camera_frame",
                        }
                    ]
                },
                separators=(",", ":"),
            )
        )

        sensors = json.loads(
            await wait_for_json(
                "sensor catalog",
                lambda: a.fetch_sensor_catalog_json(b.local_peer_id(), 5_000),
            )
        )
        assert sensors["sensors"][0]["sensor_id"] == "python-camera"

        resources = json.loads(
            await wait_for_json(
                "resource catalog",
                lambda: a.fetch_resource_catalog_json(b.local_peer_id(), 5_000),
            )
        )
        assert resources["resources"][0]["id"] == "python-camera"
    finally:
        for manager in managers:
            await manager.shutdown()
        for server in servers:
            server.shutdown()


if __name__ == "__main__":
    asyncio.run(main())

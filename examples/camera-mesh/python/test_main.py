from __future__ import annotations

from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import types
import unittest


def prepare_registry_entry(kind, entry):
    id_field = {"sensor": "sensor_id", "clock": "clock_id", "frame": "frame_id"}[kind]
    canonical = json.dumps(entry, sort_keys=True, separators=(",", ":"))
    return {
        "kind": kind,
        "id": entry[id_field],
        "hash": hashlib.sha256(canonical.encode()).hexdigest()[:32],
        "canonical_json": canonical,
    }


fake_sdk = types.SimpleNamespace(
    prepare_registry_entry=prepare_registry_entry,
    prepare_catalog_resources=lambda response: response,
    encode_camera_frame_image=lambda image: b"camera-frame:" + image,
    decode_camera_frame_image=lambda payload: payload.removeprefix(b"camera-frame:"),
)
sys.modules["auki_sdk"] = fake_sdk
spec = importlib.util.spec_from_file_location(
    "camera_mesh_python", Path(__file__).with_name("main.py")
)
assert spec is not None and spec.loader is not None
app = importlib.util.module_from_spec(spec)
spec.loader.exec_module(app)


class Routes:
    tcp = "/dns4/relay.example/tcp/4001/p2p-circuit"
    wss = "/dns4/relay.example/tcp/443/wss/p2p-circuit"


class Peer:
    peer_id = "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
    domain_id = "domain"
    routes = Routes()


class Protocol:
    def __init__(self, value):
        self.protocol = value


class CatalogProtocols:
    resource_protocol = "/catalog/resources"
    maps_protocol = "/catalog/maps"


class CameraMeshTests(unittest.TestCase):
    def mesh(self, role="publisher"):
        mesh = app.CameraMesh(Peer(), role)
        mesh.info = Protocol("/info")
        mesh.catalog = CatalogProtocols()
        mesh.registry = Protocol("/registry")
        mesh.blob = Protocol("/blob")
        mesh.message = Protocol("/message")
        mesh.stream = Protocol("/stream")
        return mesh

    def test_shared_jpeg_and_camera_frame_are_deterministic(self):
        self.assertEqual(
            app.JPEG_SHA256,
            "9cb77ff8f8f6d6af10809750bba03a76a53d6b55c36515c20a688d8437689aa0",
        )
        self.assertTrue(app.is_jpeg(app.JPEG))
        self.assertEqual(self.mesh().frame_payload, b"camera-frame:" + app.JPEG)

    def test_access_policy_is_session_scoped_and_fail_closed(self):
        mesh = self.mesh()
        requester = {
            "peer_id": "viewer",
            "domain_ids": ["domain"],
            "peer_type": "python",
            "subject": "test",
        }
        with redirect_stdout(io.StringIO()):
            self.assertEqual(
                mesh.catalog_resources_provider(requester, {"variants": []}),
                {"resources": []},
            )
        self.assertEqual(mesh.pending, {"viewer"})
        self.assertEqual(
            mesh.registry_provider(requester, {"op": "list", "kind": "sensor"}),
            {"op": "error", "reason": "access_denied"},
        )
        mesh.approve("viewer")
        self.assertEqual(
            mesh.catalog_resources_provider(requester, {"variants": []}),
            mesh.catalog_snapshot,
        )
        self.assertTrue(mesh.is_allowed(requester))

    def test_role_controls_stream_advertisement(self):
        publisher = self.mesh("publisher")
        viewer = self.mesh("viewer")
        self.assertIn("/stream", publisher.card()["protocols"])
        self.assertNotIn("/stream", viewer.card()["protocols"])
        self.assertEqual(len(viewer.card()["protocols"]), 6)
        requester = {"peer_id": "viewer", "domain_ids": ["domain"]}
        self.assertEqual(publisher.info_provider(requester)["app_instance"], "python/publisher")

    def test_discovery_routes_select_native_tcp_and_prefer_relay(self):
        self.assertEqual(
            app.choose_tcp(
                [
                    "/ip4/127.0.0.1/tcp/9000",
                    "/dns4/relay/tcp/443/wss/p2p-circuit",
                    "/dns4/relay/tcp/4001/p2p-circuit",
                ]
            ),
            "/dns4/relay/tcp/4001/p2p-circuit",
        )


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import asyncio
import copy
from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import types
import unittest
from unittest import mock
import uuid


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

    def test_shared_still_and_animation_are_valid(self):
        self.assertEqual(
            app.JPEG_SHA256,
            "775f2d96e68916cc9958fa78e7fcf6ab457b4629543fbd890d772baf78cc16dc",
        )
        self.assertTrue(app.is_jpeg(app.JPEG))
        for profile in app.CAMERA_PROFILES:
            frames = app.SYNTHETIC_JPEGS_BY_RESOURCE[profile.resource_id]
            self.assertEqual(len(frames), 16)
            self.assertEqual(len(set(frames)), 16)
            self.assertTrue(all(app.is_jpeg(frame) for frame in frames))
        self.assertEqual(
            self.mesh().frame_payload,
            b"camera-frame:" + app.SYNTHETIC_JPEGS[0],
        )

    def test_publisher_exposes_all_three_renditions(self):
        mesh = self.mesh()
        sensor_rows = [
            row
            for row in mesh.catalog_snapshot["resources"]
            if row.get("variant") == "sensor_log"
        ]
        self.assertEqual(
            [row["resource_id"] for row in sensor_rows],
            [profile.resource_id for profile in app.CAMERA_PROFILES],
        )
        self.assertEqual(
            [row["head"]["retention_ns"] for row in sensor_rows],
            [1_000_000_000 // profile.rate_hz for profile in app.CAMERA_PROFILES],
        )

        requester = {"peer_id": "viewer", "domain_ids": ["domain"]}
        mesh.allowed.add("viewer")
        listed = mesh.registry_provider(requester, {"op": "list", "kind": "sensor"})
        self.assertEqual(len(listed["entries"]), 3)
        for profile in app.CAMERA_PROFILES:
            resource_id = profile.resource_id
            dispatch = mesh.stream_provider(
                requester,
                {
                    "source_peer_id": mesh.peer_id,
                    "resource_id": resource_id,
                    "from": {"kind": "latest"},
                },
            )
            self.assertEqual(dispatch["kind"], "accept")
            self.assertEqual(dispatch["manifest"]["resource_id"], resource_id)
            self.assertEqual(
                dispatch["manifest"]["expected_rate_hz"], profile.rate_hz
            )

    def test_synthetic_frame_advances_with_stream_age(self):
        mesh = self.mesh()
        mesh.synthetic_started_ns = 1_000_000_000
        with mock.patch.object(app.time, "monotonic_ns", return_value=1_200_000_000):
            first, first_payload = mesh.current_synthetic_frame()
        with mock.patch.object(app.time, "monotonic_ns", return_value=1_400_000_000):
            second, second_payload = mesh.current_synthetic_frame()
        self.assertEqual(first, app.SYNTHETIC_JPEGS[1])
        self.assertEqual(second, app.SYNTHETIC_JPEGS[2])
        self.assertNotEqual(first, second)
        self.assertEqual(first_payload, b"camera-frame:" + first)
        self.assertEqual(second_payload, b"camera-frame:" + second)

    def test_each_mount_mints_a_fresh_uuid_session(self):
        first = self.mesh()
        second = self.mesh()
        self.assertNotEqual(first.session_id, second.session_id)
        for mesh in (first, second):
            parsed = uuid.UUID(mesh.session_id)
            self.assertEqual(parsed.version, 4)
            self.assertEqual(str(parsed), mesh.session_id)
            self.assertEqual(mesh.entries["clock"][0]["session_id"], mesh.session_id)
            requester = {"peer_id": "viewer", "domain_ids": ["domain"]}
            self.assertEqual(mesh.info_provider(requester)["session_id"], mesh.session_id)

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

    def test_automatic_approval_is_opt_in_and_domain_scoped(self):
        mesh = app.CameraMesh(Peer(), "publisher", auto_approve_same_domain=True)
        same_domain = {"peer_id": "viewer", "domain_ids": ["domain"]}
        other_domain = {"peer_id": "outsider", "domain_ids": ["another-domain"]}

        self.assertTrue(mesh.is_allowed(same_domain))
        self.assertEqual(mesh.allowed, {"viewer"})
        self.assertFalse(mesh.is_allowed(other_domain))
        self.assertNotIn("outsider", mesh.allowed)

    def test_automatic_approval_environment_is_strict(self):
        self.assertFalse(app.env_flag("AUKI_CAMERA_TEST_MISSING"))
        with mock.patch.dict(app.os.environ, {"AUKI_CAMERA_TEST_FLAG": "true"}):
            self.assertTrue(app.env_flag("AUKI_CAMERA_TEST_FLAG"))
        with mock.patch.dict(app.os.environ, {"AUKI_CAMERA_TEST_FLAG": "yes"}):
            with self.assertRaisesRegex(RuntimeError, "must be 1, true, 0, or false"):
                app.env_flag("AUKI_CAMERA_TEST_FLAG")

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

    def test_locked_remote_metadata_and_manifest_are_enforced(self):
        mesh = self.mesh()
        info = {"session_id": mesh.session_id}
        refs = {kind: copy.deepcopy(value[2]) for kind, value in mesh.entries.items()}
        entries = {kind: copy.deepcopy(value[0]) for kind, value in mesh.entries.items()}
        app.validate_remote_entries(mesh.peer_id, info, refs, entries)

        invalid_entries = copy.deepcopy(entries)
        invalid_entries["sensor"]["pixel_format"] = "bgr8"
        with self.assertRaisesRegex(RuntimeError, "Sensor image contract"):
            app.validate_remote_entries(mesh.peer_id, info, refs, invalid_entries)

        invalid_entries = copy.deepcopy(entries)
        invalid_entries["clock"]["scope"] = "domain-local"
        with self.assertRaisesRegex(RuntimeError, "Clock contract"):
            app.validate_remote_entries(mesh.peer_id, info, refs, invalid_entries)

        invalid_entries = copy.deepcopy(entries)
        invalid_entries["frame"]["axes"]["z"] = "backward"
        with self.assertRaisesRegex(RuntimeError, "ROS optical"):
            app.validate_remote_entries(mesh.peer_id, info, refs, invalid_entries)

        metadata = {"peerId": mesh.peer_id, "refs": refs}
        mesh.validate_stream_manifest(copy.deepcopy(mesh.stream_manifest), metadata)
        invalid_manifest = copy.deepcopy(mesh.stream_manifest)
        invalid_manifest["writer_mode"] = "retained"
        with self.assertRaisesRegex(RuntimeError, "writer_mode"):
            mesh.validate_stream_manifest(invalid_manifest, metadata)

    def test_snapshot_reply_routes_are_bounded_unique_and_requester_bound(self):
        requester = "12D3KooWRequester"
        tcp = f"/dns4/relay.example/tcp/4001/p2p-circuit/p2p/{requester}"
        wss = f"/dns4/relay.example/tcp/443/wss/p2p-circuit/p2p/{requester}"
        self.assertEqual(
            app.snapshot_reply_route(
                {"peerId": requester, "routes": [tcp, wss]}, requester
            ),
            tcp,
        )
        with self.assertRaisesRegex(RuntimeError, "unique"):
            app.snapshot_reply_route(
                {"peerId": requester, "routes": [tcp, tcp]}, requester
            )
        with self.assertRaisesRegex(RuntimeError, "1..4"):
            app.snapshot_reply_route(
                {"peerId": requester, "routes": [tcp] * 5}, requester
            )
        with self.assertRaisesRegex(RuntimeError, "terminate"):
            app.snapshot_reply_route(
                {
                    "peerId": requester,
                    "routes": ["/dns4/relay.example/tcp/4001/p2p/another-peer"],
                },
                requester,
            )

    def test_snapshot_request_requires_reply_address(self):
        mesh = self.mesh()
        requester = {"peer_id": "viewer", "domain_ids": ["domain"]}
        mesh.allowed.add("viewer")
        payload = json.dumps({"version": 1, "requestId": "required-reply"}).encode()
        with self.assertRaisesRegex(RuntimeError, "reply address is missing"):
            asyncio.run(mesh.handle_snapshot_request(requester, payload))


class CameraMeshAsyncTests(unittest.IsolatedAsyncioTestCase):
    async def test_view_bounds_malformed_target_and_frame_bytes(self):
        class Subscription:
            def __init__(self, manifest, payload):
                self.payload_kind = "camera"
                self.manifest = manifest
                self.payload = payload
                self.cancelled = False

            async def next(self):
                return {"kind": "entry", "entry": {"payload": self.payload}}

            async def cancel(self):
                self.cancelled = True

        class StreamClient:
            def __init__(self, subscription):
                self.subscription = subscription
                self.subscribe_calls = 0

            async def subscribe_exact(self, *_args):
                self.subscribe_calls += 1
                return self.subscription

        mesh = app.CameraMesh(Peer(), "viewer")
        refs = {kind: copy.deepcopy(value[2]) for kind, value in mesh.entries.items()}
        metadata = {"peerId": mesh.peer_id, "refs": refs}

        async def resolve_remote_metadata(_target):
            return metadata

        subscription = Subscription(copy.deepcopy(mesh.stream_manifest), mesh.frame_payload)
        stream = StreamClient(subscription)
        mesh.resolve_remote_metadata = resolve_remote_metadata
        mesh.stream = stream
        target = {
            "peerId": mesh.peer_id,
            "routes": {"tcp": "/dns4/relay.example/tcp/4001/p2p-circuit"},
        }

        report = await mesh.view(target, 1)
        self.assertTrue(report["ok"])
        self.assertEqual(report["frameBytes"], len(app.SYNTHETIC_JPEGS[0]))
        self.assertEqual(report["frames"], 1)
        self.assertTrue(subscription.cancelled)

        too_many = await mesh.view(target, 65)
        self.assertFalse(too_many["ok"])
        self.assertIn("between 1 and 64", too_many["error"])
        self.assertEqual(stream.subscribe_calls, 1)

        boolean_count = await mesh.view(target, True)
        self.assertFalse(boolean_count["ok"])
        self.assertIn("integer between 1 and 64", boolean_count["error"])
        self.assertEqual(stream.subscribe_calls, 1)

        malformed = await mesh.view(None, 3)
        self.assertFalse(malformed["ok"])
        self.assertIsNone(malformed["targetPeerId"])
        self.assertEqual(
            malformed["checks"],
            {"info": False, "catalog": False, "registry": False, "stream": False},
        )

    async def test_duplicate_inflight_snapshot_id_preserves_existing_waiter(self):
        mesh = app.CameraMesh(Peer(), "viewer")
        resolving = asyncio.Event()
        release = asyncio.Event()

        async def resolve_remote_metadata(_target):
            resolving.set()
            await release.wait()
            raise RuntimeError("stop after reservation test")

        mesh.resolve_remote_metadata = resolve_remote_metadata
        target = {
            "peerId": "publisher",
            "routes": {"tcp": "/dns4/relay.example/tcp/4001/p2p-circuit"},
        }

        first = asyncio.create_task(mesh.request_snapshot(target, "duplicate"))
        await resolving.wait()
        original = mesh.snapshot_waiters["duplicate"]
        with self.assertRaisesRegex(RuntimeError, "already pending"):
            await mesh.request_snapshot(target, "duplicate")
        self.assertIs(mesh.snapshot_waiters["duplicate"], original)
        release.set()
        result = await first
        self.assertFalse(result["ok"])
        self.assertNotIn("duplicate", mesh.snapshot_waiters)

    async def test_malformed_view_command_still_emits_view_result(self):
        mesh = app.CameraMesh(Peer(), "viewer")
        reader = asyncio.StreamReader()
        reader.feed_data(b'{"command":"view","id":"bad-view","frames":3}\n')
        reader.feed_eof()
        output = io.StringIO()
        with redirect_stdout(output):
            await app.read_commands(mesh, reader)
        event = json.loads(output.getvalue())
        self.assertEqual(event["event"], "view_result")
        self.assertEqual(event["id"], "bad-view")
        self.assertFalse(event["ok"])
        self.assertIsNone(event["targetPeerId"])

    async def test_failed_message_send_evicts_and_closes_without_replay(self):
        class Sender:
            def __init__(self, fail):
                self.fail = fail
                self.send_calls = 0
                self.close_calls = 0
                self.remote_peer = {
                    "peer_id": "publisher",
                    "domain_ids": ["domain"],
                }

            async def send(self, _kind, _timestamp_ns, _payload):
                self.send_calls += 1
                if self.fail:
                    raise RuntimeError("indeterminate send")

            async def close(self):
                self.close_calls += 1

        class MessageClient:
            def __init__(self, senders):
                self.senders = iter(senders)
                self.open_calls = 0

            async def open_exact(self, _peer_id, _route, _channel):
                self.open_calls += 1
                return next(self.senders)

        failed = Sender(True)
        replacement = Sender(False)
        mesh = app.CameraMesh(Peer(), "viewer")
        mesh.message = MessageClient([failed, replacement])
        channel = {
            "resource_id": app.CONTROL_RESOURCE_ID,
            "clock": {"id": app.CLOCK_ID, "hash": "0" * 32},
        }

        with self.assertRaisesRegex(RuntimeError, "indeterminate send"):
            await mesh.send_message(
                "publisher", "relay-route", channel, "camera.pause", b""
            )
        self.assertEqual(mesh.message.open_calls, 1)
        self.assertEqual(failed.send_calls, 1)
        self.assertEqual(failed.close_calls, 1)
        self.assertEqual(mesh.senders, {})

        await mesh.send_message(
            "publisher", "relay-route", channel, "camera.resume", b""
        )
        self.assertEqual(mesh.message.open_calls, 2)
        self.assertEqual(replacement.send_calls, 1)

    async def test_concurrent_close_callers_share_the_cleanup_barrier(self):
        class BlockingPeer:
            def __init__(self):
                self.started = asyncio.Event()
                self.release = asyncio.Event()
                self.shutdown_calls = 0

            async def shutdown(self):
                self.shutdown_calls += 1
                self.started.set()
                await self.release.wait()

        mesh = app.CameraMesh(Peer(), "viewer")
        peer = BlockingPeer()
        mesh.peer = peer
        first = asyncio.create_task(mesh.close())
        await peer.started.wait()
        second = asyncio.create_task(mesh.close())
        await asyncio.sleep(0)
        self.assertFalse(second.done())
        peer.release.set()
        await asyncio.gather(first, second)
        await mesh.close()
        self.assertEqual(peer.shutdown_calls, 1)

    async def test_snapshot_failure_reports_generated_request_id(self):
        class FailingMesh:
            def __init__(self):
                self.request_id = None

            async def request_snapshot(self, _target, request_id):
                self.request_id = request_id
                raise RuntimeError("snapshot failed")

        mesh = FailingMesh()
        reader = asyncio.StreamReader()
        reader.feed_data(
            b'{"command":"snapshot","id":"snapshot","target":{"peerId":"publisher"}}\n'
        )
        reader.feed_eof()
        output = io.StringIO()
        with redirect_stdout(output):
            await app.read_commands(mesh, reader)
        event = json.loads(output.getvalue())
        self.assertEqual(event["event"], "snapshot_result")
        self.assertEqual(event["requestId"], mesh.request_id)
        self.assertRegex(event["requestId"], app.REQUEST_ID)


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import asyncio
import base64
from collections import OrderedDict
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import threading
import time
from typing import Any
import uuid

import auki_sdk


APP = "auki-camera-mesh"
APP_VERSION = "0.1.0"
CAMERA_RESOURCE_ID = "camera/main"
CONTROL_RESOURCE_ID = "camera/control"
REPLY_RESOURCE_ID = "camera/replies"
CLOCK_ID = "camera/utc"
FRAME_ID = "camera/optical"
WIDTH = 480
HEIGHT = 270
RATE_HZ = 5
MAX_STAGED_BLOBS = 8
MAX_PENDING_SNAPSHOTS = 16
MAX_REPLY_ROUTES = 4
REQUEST_ID = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
REGISTRY_HASH = re.compile(r"^[0-9a-f]{32}$")
DISCOVERY_MODES = ("discover_only", "discover_and_advertise")

# The locked still exercises validation tests. Headless publishers cycle the
# shared animation below so a wall makes live progress obvious without Pillow,
# OpenCV, a camera, or platform codecs.
JPEG = base64.b64decode(
    (Path(__file__).resolve().parents[1] / "assets/deterministic-frame.jpg.base64").read_text()
)
JPEG_SHA256 = hashlib.sha256(JPEG).hexdigest()
SYNTHETIC_JPEGS = tuple(
    base64.b64decode(frame)
    for frame in (
        Path(__file__).resolve().parents[1] / "assets/synthetic-frames.jpg.base64"
    ).read_text().split()
)
_OUTPUT_LOCK = threading.Lock()


def emit(event: dict[str, Any]) -> None:
    with _OUTPUT_LOCK:
        print(json.dumps(event, separators=(",", ":")), flush=True)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def error_text(error: BaseException) -> str:
    return f"{type(error).__name__}: {error}"


def env_flag(name: str) -> bool:
    value = os.environ.get(name)
    if value is None:
        return False
    if value in ("1", "true"):
        return True
    if value in ("0", "false"):
        return False
    raise RuntimeError(f"{name} must be 1, true, 0, or false; got {value!r}")


def registry_ref(peer_id: str, envelope: dict[str, Any]) -> dict[str, str]:
    return {"peer_id": peer_id, "id": envelope["id"], "hash": envelope["hash"]}


def message_channel(peer_id: str, resource_id: str, clock: dict[str, str]) -> dict[str, Any]:
    return {
        "variant": "message_channel",
        "owner_peer_id": peer_id,
        "resource_id": resource_id,
        "clock": dict(clock),
    }


def choose_tcp(routes: Any) -> str:
    if isinstance(routes, dict):
        route = routes.get("tcp")
        require(isinstance(route, str) and route, "target has no TCP route")
        return route
    require(isinstance(routes, list), "target routes must be an object or array")
    native = [route for route in routes if isinstance(route, str) and "/tcp/" in route and "/wss/" not in route]
    route = min(native, key=lambda item: "p2p-circuit" not in item, default=None)
    require(route is not None, "target has no native TCP route")
    return route


def target_parts(target: Any) -> tuple[str, str]:
    require(isinstance(target, dict), "target must be an object")
    peer_id = target.get("peerId")
    require(isinstance(peer_id, str) and peer_id, "target peerId is missing")
    return peer_id, choose_tcp(target.get("routes"))


def snapshot_reply_route(target: Any, requester_peer_id: str) -> str:
    require(isinstance(target, dict), "snapshot reply target is missing")
    require(target.get("peerId") == requester_peer_id, "snapshot reply target is not the authenticated requester")
    routes = target.get("routes")
    require(isinstance(routes, list), "snapshot reply routes must be an array")
    require(0 < len(routes) <= MAX_REPLY_ROUTES, "snapshot reply target must have 1..4 routes")
    require(all(isinstance(route, str) and route for route in routes), "snapshot reply route is empty")
    require(len(set(routes)) == len(routes), "snapshot reply routes must be unique")
    suffix = f"/p2p/{requester_peer_id}"
    require(all(route.endswith(suffix) for route in routes), "snapshot reply route does not terminate at the requester")
    return choose_tcp(routes)


def is_jpeg(data: bytes) -> bool:
    return len(data) > 4 and data[:2] == b"\xff\xd8" and data[-2:] == b"\xff\xd9"


def validate_remote_entries(
    peer_id: str,
    info: dict[str, Any],
    refs: dict[str, dict[str, str]],
    entries: dict[str, dict[str, Any]],
) -> None:
    sensor = entries["sensor"]
    require(sensor.get("peer_id") == peer_id and sensor.get("sensor_id") == CAMERA_RESOURCE_ID, "Sensor identity mismatch")
    require(sensor.get("kind") == "camera" and sensor.get("type") == "rgb", "Sensor type mismatch")
    require(sensor.get("width") == WIDTH and sensor.get("height") == HEIGHT, "Sensor dimensions mismatch")
    require(sensor.get("frame_rate_hz") == RATE_HZ, "Sensor frame rate mismatch")
    require(
        sensor.get("image_encoding") == "jpeg"
        and sensor.get("pixel_format") == "rgb8"
        and sensor.get("row_stride_bytes") == 0
        and sensor.get("color_space") == "srgb",
        "Sensor image contract mismatch",
    )
    require(
        sensor.get("intrinsics_model") == "none"
        and sensor.get("distortion_model") == "none"
        and sensor.get("calibration") is None,
        "Sensor calibration contract mismatch",
    )
    require(sensor.get("frame") == refs["frame"], "Sensor frame reference mismatch")

    clock = entries["clock"]
    require(clock.get("peer_id") == peer_id and clock.get("clock_id") == CLOCK_ID, "Clock identity mismatch")
    require(clock.get("session_id") == info.get("session_id"), "Clock and Info session IDs differ")
    require(
        clock.get("type") == "utc_clock"
        and clock.get("unit") == "ns"
        and clock.get("monotonic") is False
        and clock.get("epoch") == "1970-01-01T00:00:00Z"
        and clock.get("scope") == "global",
        "Clock contract mismatch",
    )

    frame = entries["frame"]
    require(frame.get("peer_id") == peer_id and frame.get("frame_id") == FRAME_ID, "Frame identity mismatch")
    require(frame.get("handedness") == "right" and frame.get("units") == "meters", "Frame convention mismatch")
    require(frame.get("axes") == {"x": "right", "y": "down", "z": "forward"}, "Frame is not ROS optical")


async def authenticate() -> Any:
    user = (os.environ.get("AUKI_EMAIL"), os.environ.get("AUKI_PASSWORD"))
    app = (os.environ.get("AUKI_APP_ACCESS_KEY"), os.environ.get("AUKI_APP_SECRET"))
    has_user = all(user)
    has_app = all(app)
    if has_user == has_app:
        raise RuntimeError(
            "set either AUKI_EMAIL/AUKI_PASSWORD or "
            "AUKI_APP_ACCESS_KEY/AUKI_APP_SECRET"
        )
    if has_user:
        return await auki_sdk.AukiSession.login_dev(*user)
    return await auki_sdk.AukiSession.login_app_dev(*app)


class CameraMesh:
    def __init__(
        self,
        peer: Any,
        role: str,
        auto_approve_same_domain: bool = False,
    ) -> None:
        self.peer = peer
        self.peer_id = peer.peer_id
        self.role = role
        self.auto_approve_same_domain = auto_approve_same_domain
        self.node_name = os.environ.get("AUKI_NODE_NAME", f"python-camera-{role}")
        self.session_id = str(uuid.uuid4())
        self.allowed: set[str] = set()
        self.pending: set[str] = set()
        self.endpoints: list[Any] = []
        self.receiver: Any | None = None
        self.receiver_task: asyncio.Task[None] | None = None
        self.senders: dict[tuple[str, str, str, str, str], Any] = {}
        self.staged: OrderedDict[str, bytes] = OrderedDict()
        self.snapshot_waiters: dict[str, tuple[str, asyncio.Future[dict[str, Any]]]] = {}
        self.remote_metadata: dict[tuple[str, str], dict[str, Any]] = {}
        self.running = asyncio.Event()
        self.running.set()
        self.closing = False
        self.close_task: asyncio.Task[None] | None = None
        self.synthetic_started_ns = time.monotonic_ns()
        self.frame_mode = os.environ.get("AUKI_CAMERA_FRAME_MODE", "animated")
        require(self.frame_mode in ("animated", "still"), "AUKI_CAMERA_FRAME_MODE must be animated or still")

        frame_entry = {
            "peer_id": self.peer_id,
            "frame_id": FRAME_ID,
            "handedness": "right",
            "axes": {"x": "right", "y": "down", "z": "forward"},
            "units": "meters",
        }
        frame = auki_sdk.prepare_registry_entry("frame", frame_entry)
        frame_ref = registry_ref(self.peer_id, frame)
        clock_entry = {
            "peer_id": self.peer_id,
            "session_id": self.session_id,
            "clock_id": CLOCK_ID,
            "type": "utc_clock",
            "unit": "ns",
            "monotonic": False,
            "epoch": "1970-01-01T00:00:00Z",
            "scope": "global",
        }
        clock = auki_sdk.prepare_registry_entry("clock", clock_entry)
        clock_ref = registry_ref(self.peer_id, clock)
        sensor_entry = {
            "peer_id": self.peer_id,
            "sensor_id": CAMERA_RESOURCE_ID,
            "kind": "camera",
            "type": "rgb",
            "width": WIDTH,
            "height": HEIGHT,
            "frame_rate_hz": RATE_HZ,
            "image_encoding": "jpeg",
            "pixel_format": "rgb8",
            "row_stride_bytes": 0,
            "color_space": "srgb",
            "intrinsics_model": "none",
            "distortion_model": "none",
            "frame": frame_ref,
        }
        sensor = auki_sdk.prepare_registry_entry("sensor", sensor_entry)
        sensor_ref = registry_ref(self.peer_id, sensor)
        self.entries = {
            "sensor": (sensor_entry, sensor, sensor_ref),
            "clock": (clock_entry, clock, clock_ref),
            "frame": (frame_entry, frame, frame_ref),
        }
        self.clock_ref = clock_ref
        self.control_channel = message_channel(self.peer_id, CONTROL_RESOURCE_ID, clock_ref)
        self.reply_channel = message_channel(self.peer_id, REPLY_RESOURCE_ID, clock_ref)
        self.stream_manifest = {
            "sensor_id": sensor["id"],
            "sensor_hash": sensor["hash"],
            "clock_peer_id": self.peer_id,
            "clock_id": clock["id"],
            "clock_hash": clock["hash"],
            "frame_id": frame["id"],
            "frame_hash": frame["hash"],
            "resource_id": CAMERA_RESOURCE_ID,
            "payload": "camera_frame",
            "from_frame_id": "",
            "from_frame_hash": "",
            "to_frame_id": "",
            "to_frame_hash": "",
            "writer_mode": "live",
            "expected_rate_hz": RATE_HZ,
            "map_peer_id": "",
            "map_id": "",
            "map_hash": "",
        }
        require(len(SYNTHETIC_JPEGS) >= 2, "synthetic camera animation needs at least two frames")
        require(all(is_jpeg(frame) for frame in SYNTHETIC_JPEGS), "synthetic camera frame is not a JPEG")
        require(len(set(SYNTHETIC_JPEGS)) == len(SYNTHETIC_JPEGS), "synthetic camera frames must be distinct")
        self.source_jpegs = (JPEG,) if self.frame_mode == "still" else SYNTHETIC_JPEGS
        self.frame_payloads = tuple(
            bytes(auki_sdk.encode_camera_frame_image(frame))
            for frame in self.source_jpegs
        )
        require(
            all(
                bytes(auki_sdk.decode_camera_frame_image(payload)) == frame
                for frame, payload in zip(self.source_jpegs, self.frame_payloads)
            ),
            "CameraFrame codec did not round-trip",
        )
        self.frame_payload = self.frame_payloads[0]
        self.catalog_snapshot = auki_sdk.prepare_catalog_resources(
            {
                "resources": [
                    {
                        "variant": "sensor_log",
                        "source_peer_id": self.peer_id,
                        "writer_peer_id": self.peer_id,
                        "resource_id": CAMERA_RESOURCE_ID,
                        "state": "live",
                        "head": {"kind": "rolling", "retention_ns": 1_000_000_000 // RATE_HZ},
                        "available": {"bytes": 0, "entries": 0, "duration_ns": 0},
                        "sensor": {
                            "kind": "camera",
                            "type": "rgb",
                            "sensor_id": sensor["id"],
                            "sensor_hash": sensor["hash"],
                        },
                        "manifest": {"clock": clock_ref, "frame": frame_ref},
                    },
                    self.control_channel,
                ]
            }
        )

    @classmethod
    async def start(cls) -> "CameraMesh":
        role = os.environ.get("AUKI_CAMERA_ROLE", "publisher")
        require(role in ("publisher", "viewer"), "AUKI_CAMERA_ROLE must be publisher or viewer")
        domain_id = os.environ["AUKI_DOMAIN_ID"]
        identity_file = Path(os.environ["AUKI_IDENTITY_FILE"])
        identity_file.parent.mkdir(parents=True, exist_ok=True)
        default_mode = "discover_and_advertise" if role == "publisher" else "discover_only"
        discovery_mode = os.environ.get("AUKI_DISCOVERY_MODE", default_mode)
        require(discovery_mode in DISCOVERY_MODES, "invalid AUKI_DISCOVERY_MODE")
        session = await authenticate()
        peer = await session.start_peer(domain_id, identity_file, discovery_mode=discovery_mode)
        mesh = cls(peer, role, env_flag("AUKI_CAMERA_AUTO_APPROVE"))
        try:
            mesh.mount()
        except BaseException:
            await mesh.close()
            raise
        return mesh

    def mount(self) -> None:
        info = auki_sdk.AukiInfoEndpoint.mount(self.peer, self.info_provider)
        self.endpoints.append(info)
        message = auki_sdk.AukiMessageEndpoint.mount(self.peer)
        self.endpoints.append(message)
        channel = self.control_channel if self.role == "publisher" else self.reply_channel
        self.receiver = message.declare(channel, 16)
        self.receiver_task = asyncio.create_task(self.drain_messages())
        catalog = auki_sdk.AukiCatalogEndpoint.mount(
            self.peer, self.catalog_resources_provider, self.catalog_maps_provider
        )
        self.endpoints.append(catalog)
        registry = auki_sdk.AukiRegistryEndpoint.mount(self.peer, self.registry_provider)
        self.endpoints.append(registry)
        blob = auki_sdk.AukiBlobEndpoint.mount(self.peer, self.blob_provider)
        self.endpoints.append(blob)
        if self.role == "publisher":
            stream = auki_sdk.AukiStreamEndpoint.mount(self.peer, self.stream_provider)
            self.endpoints.append(stream)

        self.info = auki_sdk.AukiInfoClient(self.peer)
        self.catalog = auki_sdk.AukiCatalogClient(self.peer)
        self.registry = auki_sdk.AukiRegistryClient(self.peer)
        self.blob = auki_sdk.AukiBlobClient(self.peer)
        self.message = auki_sdk.AukiMessageClient(self.peer)
        self.stream = auki_sdk.AukiStreamClient(self.peer)

    def same_domain(self, requester: dict[str, Any]) -> bool:
        return self.peer.domain_id in requester.get("domain_ids", [])

    def is_allowed(self, requester: dict[str, Any]) -> bool:
        if not self.same_domain(requester):
            return False
        peer_id = requester.get("peer_id")
        if not isinstance(peer_id, str) or not peer_id:
            return False
        if peer_id in self.allowed:
            return True
        if not self.auto_approve_same_domain:
            return False
        self.pending.discard(peer_id)
        self.allowed.add(peer_id)
        return True

    def request_approval(self, requester: dict[str, Any]) -> None:
        peer_id = requester.get("peer_id")
        if not self.same_domain(requester) or not isinstance(peer_id, str) or not peer_id:
            return
        if peer_id in self.allowed or peer_id in self.pending:
            return
        self.pending.add(peer_id)
        emit(
            {
                "event": "approval_required",
                "peerId": peer_id,
                "peerType": requester.get("peer_type"),
                "subject": requester.get("subject"),
            }
        )

    def info_provider(self, requester: dict[str, Any]) -> dict[str, Any] | None:
        if not self.same_domain(requester):
            return None
        return {
            "app": APP,
            "app_version": APP_VERSION,
            "name": self.node_name,
            "session_id": self.session_id,
            "session_clock_id": self.entries["clock"][1]["id"],
            "session_clock_hash": self.entries["clock"][1]["hash"],
            "session_now_ns": time.time_ns(),
            "peer_id": self.peer_id,
            "app_instance": f"python/{self.role}",
        }

    def catalog_resources_provider(
        self, requester: dict[str, Any], _request: dict[str, Any]
    ) -> dict[str, Any]:
        if self.role != "publisher" or not self.is_allowed(requester):
            if self.role == "publisher":
                self.request_approval(requester)
            return {"resources": []}
        return self.catalog_snapshot

    def catalog_maps_provider(self, _requester: dict[str, Any]) -> dict[str, Any]:
        return {"resources": []}

    def registry_provider(
        self, requester: dict[str, Any], request: dict[str, Any]
    ) -> dict[str, Any]:
        if not self.is_allowed(requester):
            return {"op": "error", "reason": "access_denied"}
        selected = self.entries.get(request.get("kind"))
        if request.get("op") == "list":
            return {
                "op": "list",
                "entries": []
                if selected is None
                else [{"id": selected[1]["id"], "hash": selected[1]["hash"]}],
            }
        found = (
            selected is not None
            and request.get("id") == selected[1]["id"]
            and request.get("hash") == selected[1]["hash"]
        )
        return {"op": "get", "entry": selected[1] if found else None}

    async def blob_provider(
        self, requester: dict[str, Any], request: dict[str, Any]
    ) -> dict[str, Any] | None:
        if not self.is_allowed(requester):
            return None
        data = self.staged.get(request.get("sha256"))
        if data is None:
            return None
        start = request["offset"]
        if start > len(data):
            return None
        end = min(len(data), start + request["max_len"])
        return {"total_size": len(data), "bytes": data[start:end]}

    def stream_provider(
        self, requester: dict[str, Any], request: dict[str, Any]
    ) -> dict[str, Any]:
        if self.role != "publisher":
            return {"kind": "decline", "reason": {"kind": "sensor_not_found"}}
        if not self.is_allowed(requester):
            self.request_approval(requester)
            return {
                "kind": "decline",
                "reason": {"kind": "other", "detail": "approval_required"},
            }
        if request != {
            "source_peer_id": self.peer_id,
            "resource_id": CAMERA_RESOURCE_ID,
            "from": {"kind": "latest"},
        }:
            return {"kind": "decline", "reason": {"kind": "sensor_not_found"}}
        return {
            "kind": "accept",
            "payload_kind": "camera",
            "manifest": self.stream_manifest,
            "source": self.camera_source(),
        }

    async def camera_source(self):
        while not self.closing:
            await self.running.wait()
            if self.closing:
                return
            _jpeg, payload = self.current_synthetic_frame()
            yield {"timestamp_ns": time.time_ns(), "payload": payload}
            await asyncio.sleep(1 / RATE_HZ)

    def current_synthetic_frame(self) -> tuple[bytes, bytes]:
        elapsed_ns = max(0, time.monotonic_ns() - self.synthetic_started_ns)
        frame_index = (elapsed_ns * RATE_HZ // 1_000_000_000) % len(self.source_jpegs)
        return self.source_jpegs[frame_index], self.frame_payloads[frame_index]

    def card(self) -> dict[str, Any]:
        routes = self.peer.routes
        protocols = [
            self.info.protocol,
            self.catalog.resource_protocol,
            self.catalog.maps_protocol,
            self.registry.protocol,
            self.blob.protocol,
            self.message.protocol,
        ]
        if self.role == "publisher":
            protocols.append(self.stream.protocol)
        return {
            "version": 1,
            "runtime": "python",
            "domainId": self.peer.domain_id,
            "peerId": self.peer_id,
            "protocols": protocols,
            "routes": {"tcp": routes.tcp, "wss": routes.wss},
        }

    async def discover(self, protocol: str | None) -> list[dict[str, Any]]:
        protocol = protocol or self.stream.protocol
        candidates = await self.peer.discover_protocol(protocol)
        return [
            {
                "peerId": item.peer_id,
                "routes": item.routes,
                "servedProtocols": item.served_protocols,
                "expiresAt": item.expires_at,
                "source": item.source,
            }
            for item in candidates
        ]

    def approve(self, peer_id: str) -> None:
        require(isinstance(peer_id, str) and peer_id, "peerId is missing")
        require(peer_id in self.pending, "peer is not pending approval")
        self.pending.remove(peer_id)
        self.allowed.add(peer_id)

    async def resolve_remote_metadata(self, target: Any) -> dict[str, Any]:
        peer_id, route = target_parts(target)
        info = await self.info.fetch_exact(peer_id, route)
        require(info["peer_id"] == peer_id, "Info returned the wrong Peer ID")
        require(info["app"] == APP and info["app_version"] == APP_VERSION, "Info app mismatch")
        cache_key = (peer_id, route)
        cached = self.remote_metadata.get(cache_key)
        if (
            cached is not None
            and cached["info"]["session_id"] == info["session_id"]
            and cached["info"]["session_clock_id"] == info["session_clock_id"]
            and cached["info"]["session_clock_hash"] == info["session_clock_hash"]
        ):
            cached["info"] = info
            return cached
        self.remote_metadata.pop(cache_key, None)
        catalog = await self.catalog.fetch_resources_exact(
            peer_id, route, ["sensor_log", "message_channel"]
        )
        rows = catalog["resources"]
        cameras = [row for row in rows if row.get("variant") == "sensor_log" and row.get("resource_id") == CAMERA_RESOURCE_ID]
        require(len(cameras) == 1, "approval_required: camera Catalog row is missing or duplicated")
        camera = cameras[0]
        require(camera.get("source_peer_id") == peer_id, "Catalog source Peer ID mismatch")
        require(camera.get("writer_peer_id") == peer_id, "Catalog writer Peer ID mismatch")
        require(camera.get("state") == "live", "camera Catalog resource is not live")
        sensor_block = camera.get("sensor", {})
        require(sensor_block.get("kind") == "camera" and sensor_block.get("type") == "rgb", "Catalog sensor is not an RGB camera")
        manifest = camera.get("manifest", {})
        refs = {
            "sensor": {"peer_id": peer_id, "id": sensor_block.get("sensor_id"), "hash": sensor_block.get("sensor_hash")},
            "clock": manifest.get("clock"),
            "frame": manifest.get("frame"),
        }
        for kind, ref in refs.items():
            require(isinstance(ref, dict), f"Catalog {kind} Registry reference is missing")
            require(ref.get("peer_id") == peer_id, f"Catalog {kind} Registry owner mismatch")
            require(isinstance(ref.get("id"), str) and ref["id"], f"Catalog {kind} Registry ID is missing")
            require(isinstance(ref.get("hash"), str) and REGISTRY_HASH.fullmatch(ref["hash"]) is not None, f"Catalog {kind} Registry hash is invalid")
        require(info["session_clock_id"] == refs["clock"]["id"], "Info and Catalog clock IDs differ")
        require(info["session_clock_hash"] == refs["clock"]["hash"], "Info and Catalog clock hashes differ")
        controls = [row for row in rows if row.get("variant") == "message_channel" and row.get("resource_id") == CONTROL_RESOURCE_ID]
        require(len(controls) == 1, "camera control channel is missing or duplicated")
        control = controls[0]
        require(control.get("owner_peer_id") == peer_id, "control channel owner mismatch")
        require(control.get("clock") == refs["clock"], "control channel clock mismatch")

        entries: dict[str, dict[str, Any]] = {}
        for kind in ("sensor", "clock", "frame"):
            ref = refs[kind]
            entry = await self.registry.fetch_exact(peer_id, route, kind, ref["id"], ref["hash"])
            envelope = auki_sdk.prepare_registry_entry(kind, entry)
            require(envelope["id"] == ref["id"], f"{kind} Registry ID mismatch")
            require(envelope["hash"] == ref["hash"], f"{kind} Registry hash mismatch")
            entries[kind] = entry
        validate_remote_entries(peer_id, info, refs, entries)
        metadata = {"peerId": peer_id, "route": route, "info": info, "catalog": camera, "control": control, "refs": refs, "entries": entries}
        self.remote_metadata[cache_key] = metadata
        return metadata

    def validate_stream_manifest(self, manifest: dict[str, Any], metadata: dict[str, Any]) -> None:
        refs = metadata["refs"]
        expected = {
            "sensor_id": refs["sensor"]["id"],
            "sensor_hash": refs["sensor"]["hash"],
            "clock_peer_id": metadata["peerId"],
            "clock_id": refs["clock"]["id"],
            "clock_hash": refs["clock"]["hash"],
            "frame_id": refs["frame"]["id"],
            "frame_hash": refs["frame"]["hash"],
            "resource_id": CAMERA_RESOURCE_ID,
            "payload": "camera_frame",
            "from_frame_id": "",
            "from_frame_hash": "",
            "to_frame_id": "",
            "to_frame_hash": "",
            "writer_mode": "live",
            "expected_rate_hz": RATE_HZ,
            "map_peer_id": "",
            "map_id": "",
            "map_hash": "",
        }
        for key, value in expected.items():
            require(manifest.get(key) == value, f"Stream {key} does not match verified metadata")

    async def view(self, target: Any, frame_limit: int) -> dict[str, Any]:
        candidate_peer_id = target.get("peerId") if isinstance(target, dict) else None
        target_peer_id = candidate_peer_id if isinstance(candidate_peer_id, str) else None
        checks = {"info": False, "catalog": False, "registry": False, "stream": False}
        received = 0
        last_sha: str | None = None
        last_bytes = 0
        try:
            require(
                isinstance(frame_limit, int)
                and not isinstance(frame_limit, bool)
                and 1 <= frame_limit <= 64,
                "frames must be an integer between 1 and 64",
            )
            peer_id, route = target_parts(target)
            target_peer_id = peer_id
            metadata = await self.resolve_remote_metadata(target)
            checks.update({"info": True, "catalog": True, "registry": True})
            subscription = await self.stream.subscribe_exact(
                peer_id,
                route,
                "camera",
                {"source_peer_id": peer_id, "resource_id": CAMERA_RESOURCE_ID, "from": {"kind": "latest"}},
            )
            try:
                require(subscription.payload_kind == "camera", "Stream payload kind mismatch")
                self.validate_stream_manifest(subscription.manifest, metadata)
                checks["stream"] = True
                while received < frame_limit:
                    item = await asyncio.wait_for(subscription.next(), timeout=30)
                    require(item is not None and item.get("kind") == "entry", "Stream ended before requested frames arrived")
                    image = bytes(auki_sdk.decode_camera_frame_image(item["entry"]["payload"]))
                    require(is_jpeg(image), "CameraFrame payload is not a JPEG")
                    received += 1
                    last_bytes = len(image)
                    last_sha = hashlib.sha256(image).hexdigest()
            finally:
                await subscription.cancel()
            return {
                "ok": True,
                "targetPeerId": peer_id,
                "checks": checks,
                "frames": received,
                "frameSha256": last_sha,
                "frameBytes": last_bytes,
            }
        except Exception as error:
            return {
                "ok": False,
                "targetPeerId": target_peer_id,
                "checks": checks,
                "frames": received,
                "error": error_text(error),
            }

    async def sender_for(self, peer_id: str, route: str, channel: dict[str, Any]) -> Any:
        clock = channel["clock"]
        key = (
            peer_id,
            route,
            channel["resource_id"],
            clock["id"],
            clock["hash"],
        )
        sender = self.senders.get(key)
        if sender is None:
            sender = await self.message.open_exact(peer_id, route, channel)
            require(sender.remote_peer["peer_id"] == peer_id, "Message authenticated the wrong peer")
            require(self.peer.domain_id in sender.remote_peer["domain_ids"], "Message peer is outside the selected Domain")
            self.senders[key] = sender
        return sender

    async def send_message(
        self,
        peer_id: str,
        route: str,
        channel: dict[str, Any],
        kind: str,
        payload: bytes,
    ) -> None:
        clock = channel["clock"]
        key = (
            peer_id,
            route,
            channel["resource_id"],
            clock["id"],
            clock["hash"],
        )
        sender = await self.sender_for(peer_id, route, channel)
        try:
            await sender.send(kind, time.time_ns(), payload)
        except BaseException:
            if self.senders.get(key) is sender:
                self.senders.pop(key, None)
            try:
                await sender.close()
            except BaseException:
                pass
            raise

    async def send_control(self, target: Any, kind: str) -> str:
        require(self.role == "viewer", "only a viewer can send camera controls")
        peer_id, route = target_parts(target)
        metadata = await self.resolve_remote_metadata(target)
        await self.send_message(peer_id, route, metadata["control"], kind, b"")
        return peer_id

    async def request_snapshot(self, target: Any, request_id: str | None) -> dict[str, Any]:
        require(self.role == "viewer", "only a viewer can request snapshots")
        peer_id, route = target_parts(target)
        request_id = request_id or str(uuid.uuid4())
        require(REQUEST_ID.fullmatch(request_id) is not None, "invalid requestId")
        require(request_id not in self.snapshot_waiters, "requestId is already pending")
        require(len(self.snapshot_waiters) < MAX_PENDING_SNAPSHOTS, "too many pending snapshots")
        loop = asyncio.get_running_loop()
        future: asyncio.Future[dict[str, Any]] = loop.create_future()
        waiting = (peer_id, future)
        self.snapshot_waiters[request_id] = waiting
        try:
            routes = self.peer.routes
            payload = json.dumps(
                {
                    "version": 1,
                    "requestId": request_id,
                    "reply": {
                        "target": {"peerId": self.peer_id, "routes": [routes.tcp, routes.wss]},
                        "channel": self.reply_channel,
                    },
                },
                separators=(",", ":"),
            ).encode()
            metadata = await self.resolve_remote_metadata(target)
            await self.send_message(
                peer_id,
                route,
                metadata["control"],
                "camera.request_snapshot",
                payload,
            )
            announcement = await asyncio.wait_for(future, timeout=30)
            receipt = await self.blob.fetch_exact(peer_id, route, announcement["sha256"])
            data = bytes(receipt["bytes"])
            require(receipt["remote_peer_id"] == peer_id, "Blob authenticated the wrong publisher")
            require(receipt["sha256"] == announcement["sha256"], "Blob receipt hash mismatch")
            require(len(data) == announcement["size"], "snapshot size mismatch")
            require(hashlib.sha256(data).hexdigest() == announcement["sha256"], "snapshot SHA-256 mismatch")
            require(is_jpeg(data), "snapshot is not a JPEG")
            return {"ok": True, "requestId": request_id, "targetPeerId": peer_id, "sha256": announcement["sha256"], "size": len(data)}
        except Exception as error:
            return {"ok": False, "requestId": request_id, "targetPeerId": peer_id, "error": error_text(error)}
        finally:
            if self.snapshot_waiters.get(request_id) is waiting:
                self.snapshot_waiters.pop(request_id, None)

    async def drain_messages(self) -> None:
        require(self.receiver is not None, "Message receiver is not mounted")
        while True:
            event = await self.receiver.next()
            if event is None:
                return
            try:
                if self.role == "publisher":
                    await self.handle_control(event)
                else:
                    self.handle_snapshot_ready(event)
            except Exception as error:
                print(f"Message ignored: {error_text(error)}", file=sys.stderr, flush=True)

    async def handle_control(self, event: dict[str, Any]) -> None:
        requester = event["sender"]
        kind = event["type"]
        if not self.is_allowed(requester):
            return
        if kind == "camera.pause":
            require(event["payload"] == b"", "pause payload must be empty")
            self.running.clear()
        elif kind == "camera.resume":
            require(event["payload"] == b"", "resume payload must be empty")
            self.running.set()
        elif kind == "camera.request_snapshot":
            await self.handle_snapshot_request(requester, event["payload"])
        else:
            return
        emit({"event": "control_received", "peerId": requester["peer_id"], "control": kind, "applied": True})

    async def handle_snapshot_request(self, requester: dict[str, Any], payload: bytes) -> None:
        require(len(payload) <= 16_384, "snapshot request is too large")
        request = json.loads(payload.decode())
        request_id = request.get("requestId")
        require(request.get("version") == 1 and isinstance(request_id, str), "invalid snapshot request")
        require(REQUEST_ID.fullmatch(request_id) is not None, "invalid snapshot requestId")
        reply = request.get("reply")
        require(isinstance(reply, dict), "snapshot reply address is missing")
        target = reply.get("target")
        route = snapshot_reply_route(target, requester["peer_id"])
        channel = reply.get("channel")
        require(isinstance(channel, dict), "snapshot reply channel is missing")
        require(channel.get("variant") == "message_channel", "snapshot reply channel variant is invalid")
        require(channel.get("owner_peer_id") == requester["peer_id"], "snapshot reply channel owner mismatch")
        require(channel.get("resource_id") == REPLY_RESOURCE_ID, "snapshot reply channel resource mismatch")
        clock = channel.get("clock", {})
        require(clock.get("peer_id") == requester["peer_id"], "snapshot reply clock owner mismatch")
        require(isinstance(clock.get("hash"), str) and REGISTRY_HASH.fullmatch(clock["hash"]) is not None, "snapshot reply clock hash is invalid")
        jpeg, _payload = self.current_synthetic_frame()
        sha256 = hashlib.sha256(jpeg).hexdigest()
        self.staged[sha256] = jpeg
        self.staged.move_to_end(sha256)
        while len(self.staged) > MAX_STAGED_BLOBS:
            self.staged.popitem(last=False)
        emit({"event": "snapshot_staged", "peerId": requester["peer_id"], "requestId": request_id, "sha256": sha256, "size": len(jpeg)})
        response = json.dumps(
            {"version": 1, "requestId": request_id, "sha256": sha256, "size": len(jpeg)},
            separators=(",", ":"),
        ).encode()
        await self.send_message(
            requester["peer_id"], route, channel, "camera.snapshot_ready", response
        )

    def handle_snapshot_ready(self, event: dict[str, Any]) -> None:
        sender = event["sender"]
        if not self.same_domain(sender) or event["type"] != "camera.snapshot_ready":
            return
        require(len(event["payload"]) <= 4096, "snapshot announcement is too large")
        response = json.loads(event["payload"].decode())
        request_id = response.get("requestId")
        waiting = self.snapshot_waiters.get(request_id)
        if waiting is None or waiting[0] != sender.get("peer_id"):
            return
        require(response.get("version") == 1, "snapshot announcement version mismatch")
        sha256 = response.get("sha256")
        size = response.get("size")
        require(isinstance(sha256, str) and SHA256.fullmatch(sha256) is not None, "invalid snapshot SHA-256")
        require(isinstance(size, int) and 0 < size <= 20 * 1024 * 1024, "invalid snapshot size")
        future = waiting[1]
        if not future.done():
            future.set_result({"sha256": sha256, "size": size})

    async def close(self) -> None:
        if self.close_task is None:
            self.closing = True
            self.running.set()
            self.close_task = asyncio.create_task(self.close_owned())
        await asyncio.shield(self.close_task)

    async def close_owned(self) -> None:
        errors: list[str] = []
        for _key, sender in list(self.senders.items())[::-1]:
            try:
                await sender.close()
            except BaseException as error:
                errors.append(f"Message sender: {error}")
        self.senders.clear()
        if self.receiver is not None:
            try:
                await self.receiver.close()
            except BaseException as error:
                errors.append(f"Message receiver: {error}")
        if self.receiver_task is not None:
            result = await asyncio.gather(self.receiver_task, return_exceptions=True)
            if isinstance(result[0], BaseException):
                errors.append(f"Message drain: {result[0]}")
        for endpoint in reversed(self.endpoints):
            try:
                await endpoint.close()
            except BaseException as error:
                errors.append(f"{type(endpoint).__name__}: {error}")
        try:
            await self.peer.shutdown()
        except BaseException as error:
            errors.append(f"Auki peer: {error}")
        if errors:
            raise RuntimeError("ordered shutdown failed: " + "; ".join(errors))


async def read_commands(mesh: CameraMesh, reader: asyncio.StreamReader) -> None:
    while line := await reader.readline():
        command: dict[str, Any] = {}
        try:
            command = json.loads(line.decode())
            name = command["command"]
            command_id = command["id"]
            if name == "discover":
                protocol = command.get("protocol")
                try:
                    candidates = await mesh.discover(protocol)
                    emit({"event": "discovery_result", "id": command_id, "ok": True, "protocol": protocol, "candidates": candidates})
                except Exception as error:
                    emit({"event": "discovery_result", "id": command_id, "ok": False, "protocol": protocol, "candidates": [], "error": error_text(error)})
            elif name == "approve":
                try:
                    mesh.approve(command["peerId"])
                    emit({"event": "approve_result", "id": command_id, "ok": True, "peerId": command["peerId"]})
                except Exception as error:
                    emit({"event": "approve_result", "id": command_id, "ok": False, "peerId": command.get("peerId"), "error": error_text(error)})
            elif name == "view":
                target = command.get("target")
                try:
                    frames = command.get("frames", 3)
                    require(
                        isinstance(frames, int) and not isinstance(frames, bool),
                        "frames must be an integer",
                    )
                    result = await mesh.view(target, frames)
                except Exception as error:
                    candidate_peer_id = target.get("peerId") if isinstance(target, dict) else None
                    target_peer_id = candidate_peer_id if isinstance(candidate_peer_id, str) else None
                    result = {
                        "ok": False,
                        "targetPeerId": target_peer_id,
                        "checks": {
                            "info": False,
                            "catalog": False,
                            "registry": False,
                            "stream": False,
                        },
                        "frames": 0,
                        "error": error_text(error),
                    }
                emit({"event": "view_result", "id": command_id, **result})
            elif name in ("pause", "resume"):
                try:
                    peer_id = await mesh.send_control(command["target"], f"camera.{name}")
                    emit({"event": "control_result", "id": command_id, "ok": True, "control": f"camera.{name}", "targetPeerId": peer_id})
                except Exception as error:
                    peer_id = command.get("target", {}).get("peerId")
                    emit({"event": "control_result", "id": command_id, "ok": False, "control": f"camera.{name}", "targetPeerId": peer_id, "error": error_text(error)})
            elif name == "snapshot":
                request_id = command.get("requestId") or str(uuid.uuid4())
                try:
                    result = await mesh.request_snapshot(command["target"], request_id)
                except Exception as error:
                    result = {"ok": False, "requestId": request_id, "targetPeerId": command.get("target", {}).get("peerId"), "error": error_text(error)}
                emit({"event": "snapshot_result", "id": command_id, **result})
            elif name == "shutdown":
                emit({"event": "shutdown_ack", "id": command_id})
                return
            else:
                raise ValueError(f"unknown command {name!r}")
        except Exception as error:
            emit({"event": "command_error", "id": command.get("id"), "error": f"invalid command: {error}"})


async def command_loop(mesh: CameraMesh) -> None:
    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader()
    protocol = asyncio.StreamReaderProtocol(reader)
    transport, _ = await loop.connect_read_pipe(lambda: protocol, sys.stdin)
    try:
        await read_commands(mesh, reader)
    finally:
        transport.close()


async def main() -> None:
    mesh = await CameraMesh.start()
    emit({"event": "ready", "runtime": "python", "role": mesh.role, "card": mesh.card()})
    operation: BaseException | None = None
    try:
        await command_loop(mesh)
    except BaseException as error:
        operation = error
    try:
        await mesh.close()
    except BaseException as error:
        operation = error if operation is None else RuntimeError(f"{operation}; cleanup also failed: {error}")
    if operation is not None:
        raise operation


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass

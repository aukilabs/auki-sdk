from __future__ import annotations

import asyncio
import hashlib
import json
import os
from pathlib import Path
import struct
import sys
from typing import Any, Awaitable, Callable

import auki_sdk


APP = "standard-protocols"
APP_VERSION = "0.1.0"
BLOB_BYTES = b"auki-standard-protocols-v1"
BLOB_SHA256 = hashlib.sha256(BLOB_BYTES).hexdigest()
MESSAGE_RESOURCE_ID = "playground/events"
MESSAGE_CLOCK_ID = "playground/clock"
MESSAGE_CLOCK_HASH = "playground-clock-v1"
MESSAGE_TYPE = "playground.message"
MESSAGE_TIMESTAMP_NS = 42
MESSAGE_BYTES = b"hello from the standard protocol playground"
STREAM_RESOURCE_ID = "playground/scalar"
STREAM_TIMESTAMP_NS = 99
STREAM_VALUE = 12.5
SCALAR_BYTES = b"\x09" + struct.pack("<d", STREAM_VALUE)
REGISTRY_ID = "playground/base"
REGISTRY_HASH = "0" * 32
CHECK_NAMES = ("info", "catalog", "registry", "blob", "message", "stream")


def emit(event: dict[str, Any]) -> None:
    print(json.dumps(event, separators=(",", ":")), flush=True)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def message_channel(peer_id: str) -> dict[str, Any]:
    return {
        "variant": "message_channel",
        "owner_peer_id": peer_id,
        "resource_id": MESSAGE_RESOURCE_ID,
        "clock": {
            "peer_id": peer_id,
            "id": MESSAGE_CLOCK_ID,
            "hash": MESSAGE_CLOCK_HASH,
        },
    }


async def authenticate() -> Any:
    user = (os.environ.get("AUKI_EMAIL"), os.environ.get("AUKI_PASSWORD"))
    app = (
        os.environ.get("AUKI_APP_ACCESS_KEY"),
        os.environ.get("AUKI_APP_SECRET"),
    )
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


class Playground:
    def __init__(self, peer: Any) -> None:
        self.peer = peer
        self.peer_id = peer.peer_id
        self.node_name = os.environ.get("AUKI_NODE_NAME", "python-playground")
        self.endpoints: list[Any] = []
        self.receiver: Any | None = None
        self.receiver_task: asyncio.Task[None] | None = None

    @classmethod
    async def start(cls) -> "Playground":
        domain_id = os.environ["AUKI_DOMAIN_ID"]
        identity_file = Path(os.environ["AUKI_IDENTITY_FILE"])
        identity_file.parent.mkdir(parents=True, exist_ok=True)
        session = await authenticate()
        peer = await session.start_peer(domain_id, identity_file)
        playground = cls(peer)
        try:
            playground.mount()
        except BaseException as operation:
            try:
                await playground.close()
            except BaseException as cleanup:
                raise RuntimeError(
                    f"mount protocols: {operation}; cleanup also failed: {cleanup}"
                ) from operation
            raise
        return playground

    def mount(self) -> None:
        info = auki_sdk.AukiInfoEndpoint.mount(self.peer, self.info_provider)
        self.endpoints.append(info)
        message = auki_sdk.AukiMessageEndpoint.mount(self.peer)
        self.endpoints.append(message)
        self.receiver = message.declare(message_channel(self.peer_id), 16)
        self.receiver_task = asyncio.create_task(self.drain_messages())
        catalog = auki_sdk.AukiCatalogEndpoint.mount(
            self.peer, self.catalog_resources_provider, self.catalog_maps_provider
        )
        self.endpoints.append(catalog)
        registry = auki_sdk.AukiRegistryEndpoint.mount(
            self.peer, self.registry_provider
        )
        self.endpoints.append(registry)
        blob = auki_sdk.AukiBlobEndpoint.mount(self.peer, self.blob_provider)
        self.endpoints.append(blob)
        stream = auki_sdk.AukiStreamEndpoint.mount(self.peer, self.stream_provider)
        self.endpoints.append(stream)

        self.info = auki_sdk.AukiInfoClient(self.peer)
        self.catalog = auki_sdk.AukiCatalogClient(self.peer)
        self.registry = auki_sdk.AukiRegistryClient(self.peer)
        self.blob = auki_sdk.AukiBlobClient(self.peer)
        self.message = auki_sdk.AukiMessageClient(self.peer)
        self.stream = auki_sdk.AukiStreamClient(self.peer)

    def info_provider(self, _requester: dict[str, Any]) -> dict[str, Any]:
        self.validate_requester(_requester)
        return {
            "app": APP,
            "app_version": APP_VERSION,
            "name": self.node_name,
            "session_id": "playground-session",
            "session_clock_id": MESSAGE_CLOCK_ID,
            "session_clock_hash": MESSAGE_CLOCK_HASH,
            "session_now_ns": 0,
            "peer_id": self.peer_id,
            "app_instance": "python",
        }

    def catalog_resources_provider(
        self, _requester: dict[str, Any], _request: dict[str, Any]
    ) -> dict[str, Any]:
        self.validate_requester(_requester)
        return {"resources": [message_channel(self.peer_id)]}

    def catalog_maps_provider(self, _requester: dict[str, Any]) -> dict[str, Any]:
        self.validate_requester(_requester)
        return {"resources": []}

    def registry_provider(
        self, _requester: dict[str, Any], request: dict[str, Any]
    ) -> dict[str, Any]:
        self.validate_requester(_requester)
        if request["op"] == "list":
            return {
                "op": "list",
                "entries": [{"id": REGISTRY_ID, "hash": REGISTRY_HASH}],
            }
        return {"op": "get", "entry": None}

    async def blob_provider(
        self, _requester: dict[str, Any], request: dict[str, Any]
    ) -> dict[str, Any] | None:
        self.validate_requester(_requester)
        if request["sha256"] != BLOB_SHA256:
            return None
        start = request["offset"]
        end = min(len(BLOB_BYTES), start + request["max_len"])
        return {"total_size": len(BLOB_BYTES), "bytes": BLOB_BYTES[start:end]}

    def stream_provider(
        self, _requester: dict[str, Any], request: dict[str, Any]
    ) -> dict[str, Any]:
        self.validate_requester(_requester)
        if request != {
            "source_peer_id": self.peer_id,
            "resource_id": STREAM_RESOURCE_ID,
            "from": {"kind": "latest"},
        }:
            return {"kind": "decline", "reason": {"kind": "sensor_not_found"}}
        return {
            "kind": "accept",
            "payload_kind": "scalar",
            "manifest": {"resource_id": STREAM_RESOURCE_ID, "payload": "scalar"},
            "source": self.scalar_source(),
        }

    def validate_requester(self, requester: dict[str, Any]) -> None:
        require(bool(requester["peer_id"]), "authenticated requester Peer ID is missing")
        require(
            self.peer.domain_id in requester["domain_ids"],
            "authenticated requester does not share the selected Domain",
        )

    async def scalar_source(self):
        yield {"timestamp_ns": STREAM_TIMESTAMP_NS, "payload": SCALAR_BYTES}

    async def drain_messages(self) -> None:
        require(self.receiver is not None, "Message receiver is not mounted")
        while True:
            event = await self.receiver.next()
            if event is None:
                return
            require(event["type"] == MESSAGE_TYPE, "received Message type mismatch")
            require(
                event["timestamp_ns"] == MESSAGE_TIMESTAMP_NS,
                "received Message timestamp mismatch",
            )
            require(event["payload"] == MESSAGE_BYTES, "received Message payload mismatch")
            require(bool(event["sender"]["peer_id"]), "received Message sender is missing")
            print(
                f"message received from {event['sender']['peer_id']} "
                f"type={event['type']} bytes={len(event['payload'])}",
                file=sys.stderr,
                flush=True,
            )

    def card(self) -> dict[str, Any]:
        routes = self.peer.routes
        return {
            "version": 1,
            "runtime": "python",
            "domainId": self.peer.domain_id,
            "peerId": self.peer_id,
            "protocols": [
                self.info.protocol,
                self.catalog.resource_protocol,
                self.catalog.maps_protocol,
                self.registry.protocol,
                self.blob.protocol,
                self.message.protocol,
                self.stream.protocol,
            ],
            "routes": {"tcp": routes.tcp, "wss": routes.wss},
        }

    async def probe_all(self, target: dict[str, Any]) -> dict[str, Any]:
        async def run(name: str, operation: Callable[[], Awaitable[None]]) -> None:
            try:
                await asyncio.wait_for(operation(), timeout=60)
                checks[name] = True
            except Exception as error:
                checks[name] = False
                errors[name] = f"{type(error).__name__}: {error}"

        checks: dict[str, bool] = {}
        errors: dict[str, str] = {}
        for name in CHECK_NAMES:
            await run(name, lambda name=name: getattr(self, f"probe_{name}")(target))
        return {"checks": checks, "errors": errors, "ok": not errors}

    async def probe_info(self, target: dict[str, Any]) -> None:
        info = await self.info.fetch_exact(target["peerId"], target["routes"]["tcp"])
        require(info["peer_id"] == target["peerId"], "Info returned the wrong Peer ID")
        require(
            info["app"] == APP and info["app_version"] == APP_VERSION,
            "Info fixture mismatch",
        )

    async def probe_catalog(self, target: dict[str, Any]) -> None:
        resources = await self.catalog.fetch_resources_exact(
            target["peerId"], target["routes"]["tcp"]
        )
        require(
            resources == {"resources": [message_channel(target["peerId"])]},
            "Catalog v3 fixture mismatch",
        )
        maps = await self.catalog.fetch_maps_exact(
            target["peerId"], target["routes"]["tcp"]
        )
        require(maps == {"resources": []}, "Catalog v4 fixture mismatch")

    async def probe_registry(self, target: dict[str, Any]) -> None:
        entries = await self.registry.list_exact(
            target["peerId"], target["routes"]["tcp"], "frame"
        )
        require(
            entries == [{"id": REGISTRY_ID, "hash": REGISTRY_HASH}],
            "Registry fixture mismatch",
        )

    async def probe_blob(self, target: dict[str, Any]) -> None:
        receipt = await self.blob.fetch_exact(
            target["peerId"], target["routes"]["tcp"], BLOB_SHA256
        )
        require(receipt["remote_peer_id"] == target["peerId"], "Blob Peer ID mismatch")
        require(receipt["sha256"] == BLOB_SHA256, "Blob hash mismatch")
        require(receipt["bytes"] == BLOB_BYTES, "Blob bytes mismatch")
        require(receipt["relayed"] is True, "Blob did not use the relay circuit")

    async def probe_message(self, target: dict[str, Any]) -> None:
        sender = await self.message.open_exact(
            target["peerId"],
            target["routes"]["tcp"],
            message_channel(target["peerId"]),
        )
        try:
            require(sender.remote_peer["peer_id"] == target["peerId"], "Message Peer ID mismatch")
            require(sender.relayed is True, "Message did not use the relay circuit")
            await sender.send(MESSAGE_TYPE, MESSAGE_TIMESTAMP_NS, MESSAGE_BYTES)
        finally:
            await sender.close()

    async def probe_stream(self, target: dict[str, Any]) -> None:
        subscription = await self.stream.subscribe_exact(
            target["peerId"],
            target["routes"]["tcp"],
            "scalar",
            {
                "source_peer_id": target["peerId"],
                "resource_id": STREAM_RESOURCE_ID,
                "from": {"kind": "latest"},
            },
        )
        try:
            require(subscription.payload_kind == "scalar", "Stream payload kind mismatch")
            require(
                subscription.manifest["resource_id"] == STREAM_RESOURCE_ID,
                "Stream resource mismatch",
            )
            require(subscription.manifest["payload"] == "scalar", "Stream manifest mismatch")
            entry = await subscription.next()
            require(
                entry
                == {
                    "kind": "entry",
                    "entry": {
                        "timestamp_ns": STREAM_TIMESTAMP_NS,
                        "sequence": 0,
                        "payload": SCALAR_BYTES,
                    },
                },
                "Stream fixture entry mismatch",
            )
            terminal = await subscription.next()
            require(
                terminal == {"kind": "end", "reason": {"kind": "source_ended"}},
                "Stream terminal mismatch",
            )
        finally:
            await subscription.cancel()

    async def close(self) -> None:
        errors: list[str] = []
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


async def command_loop(playground: Playground) -> None:
    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader()
    protocol = asyncio.StreamReaderProtocol(reader)
    transport, _ = await loop.connect_read_pipe(lambda: protocol, sys.stdin)
    try:
        await read_commands(playground, reader)
    finally:
        transport.close()


async def read_commands(playground: Playground, reader: asyncio.StreamReader) -> None:
    while line := await reader.readline():
        try:
            command = json.loads(line.decode())
            if command["command"] == "probe_all":
                result = await playground.probe_all(command["target"])
                emit(
                    {
                        "event": "probe_result",
                        "id": command["id"],
                        "targetPeerId": command["target"]["peerId"],
                        **result,
                    }
                )
            elif command["command"] == "shutdown":
                emit({"event": "shutdown_ack", "id": command["id"]})
                return
            else:
                raise ValueError(f"unknown command {command['command']!r}")
        except Exception as error:
            emit({"event": "command_error", "error": f"invalid command: {error}"})


async def main() -> None:
    playground = await Playground.start()
    emit({"event": "ready", "runtime": "python", "card": playground.card()})
    operation: BaseException | None = None
    try:
        await command_loop(playground)
    except BaseException as error:
        operation = error
    try:
        await playground.close()
    except BaseException as error:
        if operation is None:
            operation = error
        else:
            operation = RuntimeError(f"{operation}; cleanup also failed: {error}")
    if operation is not None:
        raise operation
    emit({"event": "stopped", "runtime": "python"})


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass

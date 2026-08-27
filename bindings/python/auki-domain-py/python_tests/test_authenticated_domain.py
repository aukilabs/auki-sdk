"""Real Python vertical tests for the authenticated Rust Domain owner."""

from __future__ import annotations

import asyncio
import gc
import json
from pathlib import Path
import re
import socket
import tempfile

import pytest


SDK_ROOT = Path(__file__).resolve().parents[4]
AUTH_VECTORS = (
    SDK_ROOT
    / "crates"
    / "auki-domain"
    / "tests"
    / "fixtures"
    / "authenticated_domain_vectors.json"
)
LISTENER = "/ip4/127.0.0.1/tcp/0"


def identity(seed: int):
    import auki_domain

    return auki_domain.Identity.from_ed25519_seed(bytes([seed]) * 32)


def resource(owner: str, resource_id: str) -> dict:
    return {
        "variant": "sensor_log",
        "source_peer_id": owner,
        "writer_peer_id": owner,
        "resource_id": resource_id,
        "state": "live",
        "head": {"kind": "rolling", "retention_ns": 5_000_000_000},
        "available": {"bytes": 1024, "entries": 10, "duration_ns": 5_000_000_000},
        "sensor": {
            "kind": "camera",
            "type": "rgb",
            "sensor_id": resource_id,
            "sensor_hash": "sensor-hash",
        },
        "manifest": {
            "clock": {"peer_id": owner, "id": "clock", "hash": "clock-hash"},
            "frame": None,
        },
    }


class CountingCatalog:
    def __init__(self, rows: list[object]) -> None:
        self.rows = rows
        self.calls = 0

    def __call__(self) -> list[object]:
        self.calls += 1
        return self.rows


def peer_and_session(identity_value, root: str):
    import auki_session

    peer = auki_session.Peer(identity_value.peer_id, "python-domain-test")
    peer = peer.with_storage_root(root)
    return peer, peer.start_session()


async def join_domain(
    domain_id: str,
    identity_value,
    *,
    provider=None,
    participant_provider=None,
    stream_provider=None,
    listen: bool = True,
    credential_identity=None,
    credential_domain: str | None = None,
    expired: bool = False,
    serve_all: bool = True,
):
    import auki_domain

    root = tempfile.TemporaryDirectory()
    peer, session = peer_and_session(identity_value, root.name)
    config = auki_domain.DomainConfig(domain_id, identity_value)
    if listen:
        config.with_listen_addresses([LISTENER])
    builder = auki_domain.Domain.builder(peer, session, config)
    keys, credential = auki_domain._test_authority(
        credential_identity or identity_value,
        credential_domain or domain_id,
        expired=expired,
    )
    builder.authority(keys, credential)
    # Test profile: explicitly serve every retained protocol family. Production
    # applications should call only the exact version methods they host.
    if serve_all:
        for enable in (
            builder.serve_info_v1,
            builder.serve_resources_v2,
            builder.serve_resources_v3,
            builder.serve_resources_v4,
            builder.serve_registries_v2,
            builder.serve_registries_v3,
            builder.serve_blobs_v1,
            builder.serve_messages_v1,
            builder.serve_streams_v2,
        ):
            enable()
    if provider is not None:
        builder.resource_catalog_provider(provider)
    if participant_provider is not None:
        builder.participant_info_provider(participant_provider)
    if stream_provider is not None:
        builder.stream_provider(stream_provider)
    domain = await asyncio.wait_for(builder.join(), 10)
    expected_ids = (
        [
            "/auki/auth/1/info/1.0.0",
            "/auki/auth/1/resources/0.2.0",
            "/auki/auth/1/resources/0.3.0",
            "/auki/auth/1/resources/0.4.0",
            "/auki/auth/1/registries/0.2.0",
            "/auki/auth/1/registries/0.3.0",
            "/auki/auth/1/blobs/0.1.0",
            "/auki/auth/1/message/0.1.0",
            "/auki/auth/1/stream/0.2.0",
        ]
        if serve_all
        else []
    )
    assert domain.served_protocol_ids == expected_ids
    return domain, root, peer, session


def tcp_port(address: str) -> int:
    match = re.search(r"/tcp/(\d+)(?:/|$)", address)
    assert match is not None, address
    return int(match.group(1))


def unused_tcp_port() -> int:
    candidate = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        candidate.bind(("127.0.0.1", 0))
        return int(candidate.getsockname()[1])
    finally:
        candidate.close()


async def wait_stopped(subscription, timeout: float = 5.0) -> None:
    async def wait() -> None:
        while subscription.current().state != "stopped":
            await subscription.changed()

    await asyncio.wait_for(wait(), timeout)


async def wait_rebind(port: int, timeout: float = 5.0) -> None:
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        probe.settimeout(0.05)
        still_listening = probe.connect_ex(("127.0.0.1", port)) == 0
        probe.close()
        if still_listening:
            if asyncio.get_running_loop().time() >= deadline:
                raise AssertionError(f"listener on TCP port {port} is still accepting connections")
            await asyncio.sleep(0.02)
            continue
        candidate = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            # Accepted connections can leave the listener port in TIME_WAIT on
            # macOS. Reuse is the same honest listener-release probe used by
            # Tokio/libp2p; a still-open listener would still make bind fail.
            candidate.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            candidate.bind(("127.0.0.1", port))
            return
        except OSError:
            if asyncio.get_running_loop().time() >= deadline:
                raise
            await asyncio.sleep(0.02)
        finally:
            candidate.close()


def test_shared_authority_vectors_match_rust() -> None:
    vectors = json.loads(AUTH_VECTORS.read_text())

    async def scenario() -> None:
        for vector in vectors["cases"]:
            local_identity = identity(vector["identity_seed_byte"])
            credential_identity = identity(vector["credential_peer_seed_byte"])
            join = join_domain(
                vectors["domain_id"],
                local_identity,
                listen=False,
                credential_identity=credential_identity,
                credential_domain=vector["credential_domain_id"],
                expired=vector["expired"],
            )
            if vector["expected_join"] == "ready":
                domain, root, _peer, _session = await join
                assert domain.status().state == "ready"
                await asyncio.wait_for(domain.leave(), 5)
                root.cleanup()
            else:
                assert vector["expected_join"] == "rejected"
                with pytest.raises(RuntimeError):
                    await asyncio.wait_for(join, 5)

    asyncio.run(asyncio.wait_for(scenario(), 30))


def test_bidirectional_resources_auth_negatives_and_ordered_leave() -> None:
    vectors = json.loads(AUTH_VECTORS.read_text())
    domain_id = vectors["domain_id"]

    async def scenario() -> None:
        import auki_domain

        a_identity = identity(101)
        b_identity = identity(102)
        a_catalog = CountingCatalog(
            [auki_domain.ResourceEntry.from_dict(resource(a_identity.peer_id, "alpha-camera"))]
        )
        b_catalog = CountingCatalog(
            [auki_domain.ResourceEntry.from_dict(resource(b_identity.peer_id, "bravo-camera"))]
        )
        a, a_root, _a_peer, _a_session = await join_domain(
            domain_id, a_identity, provider=a_catalog
        )
        b, b_root, _b_peer, _b_session = await join_domain(
            domain_id, b_identity, provider=b_catalog
        )
        a_port = tcp_port(a.listen_addresses[0])
        b_port = tcp_port(b.listen_addresses[0])
        a.routes().replace(b.peer_id, b.listen_addresses)
        b.routes().replace(a.peer_id, a.listen_addresses)

        from_a, from_b = await asyncio.gather(
            b.fetch_resources_catalog(a.peer_id),
            a.fetch_resources_catalog(b.peer_id),
        )
        assert [row.resource_id for row in from_a] == ["alpha-camera"]
        assert [row.resource_id for row in from_b] == ["bravo-camera"]
        assert a_catalog.calls == 1
        assert b_catalog.calls == 1
        assert [peer.peer_id for peer in a.known_peers().snapshot()] == [b.peer_id]
        assert [peer.peer_id for peer in b.known_peers().snapshot()] == [a.peer_id]

        # A route hint aimed at the wrong expected Noise/token peer must fail
        # before A's Python provider is sampled.
        wrong_peer = identity(103).peer_id
        wrong_target, wrong_root, _peer, _session = await join_domain(
            domain_id, identity(104), listen=False
        )
        wrong_target.routes().replace(wrong_peer, a.listen_addresses)
        before = a_catalog.calls
        with pytest.raises(RuntimeError):
            await asyncio.wait_for(wrong_target.fetch_resources_catalog(wrong_peer), 5)
        assert a_catalog.calls == before
        await wrong_target.leave()
        wrong_root.cleanup()

        wrong_domain, wrong_domain_root, _peer, _session = await join_domain(
            vectors["wrong_domain_id"], identity(105), listen=False
        )
        wrong_domain.routes().replace(a.peer_id, a.listen_addresses)
        before = a_catalog.calls
        with pytest.raises(RuntimeError):
            await asyncio.wait_for(wrong_domain.fetch_resources_catalog(a.peer_id), 5)
        assert a_catalog.calls == before
        await wrong_domain.leave()
        wrong_domain_root.cleanup()

        a_routes = a.routes()
        b_routes = b.routes()
        await asyncio.wait_for(asyncio.gather(b.leave(), a.leave()), 10)
        assert a.status().state == "stopped"
        assert b.status().state == "stopped"
        with pytest.raises(RuntimeError):
            a_routes.snapshot()
        with pytest.raises(RuntimeError):
            b_routes.snapshot()
        await asyncio.gather(wait_rebind(a_port), wait_rebind(b_port))
        a_root.cleanup()
        b_root.cleanup()

    asyncio.run(asyncio.wait_for(scenario(), 45))


def test_client_only_domain_does_not_serve_its_configured_provider() -> None:
    vectors = json.loads(AUTH_VECTORS.read_text())

    async def scenario() -> None:
        import auki_domain

        server_identity = identity(106)
        client_identity = identity(107)
        server_catalog = CountingCatalog(
            [
                auki_domain.ResourceEntry.from_dict(
                    resource(server_identity.peer_id, "selected-server-camera")
                )
            ]
        )
        client_catalog = CountingCatalog(
            [
                auki_domain.ResourceEntry.from_dict(
                    resource(client_identity.peer_id, "configured-but-unserved-camera")
                )
            ]
        )
        server, server_root, _peer, _session = await join_domain(
            vectors["domain_id"], server_identity, provider=server_catalog
        )
        client, client_root, _peer, _session = await join_domain(
            vectors["domain_id"],
            client_identity,
            provider=client_catalog,
            serve_all=False,
        )
        client.routes().replace(server.peer_id, server.listen_addresses)
        server.routes().replace(client.peer_id, client.listen_addresses)

        rows = await client.fetch_resources_catalog(server.peer_id)
        assert [row.resource_id for row in rows] == ["selected-server-camera"]
        assert server_catalog.calls == 1

        with pytest.raises(RuntimeError):
            await server.fetch_resources_catalog(client.peer_id)
        assert client_catalog.calls == 0

        await asyncio.gather(client.leave(), server.leave())
        client_root.cleanup()
        server_root.cleanup()

    asyncio.run(asyncio.wait_for(scenario(), 30))


def test_cancelled_leave_and_provider_cycle_gc_finish_native_cleanup() -> None:
    vectors = json.loads(AUTH_VECTORS.read_text())

    async def scenario() -> None:
        # leave() starts one independently owned cleanup barrier before it
        # hands Python an awaitable. Cancelling the await cannot cancel cleanup.
        domain, root, _peer, _session = await join_domain(
            vectors["domain_id"], identity(111)
        )
        status = domain.subscribe_status()
        port = tcp_port(domain.listen_addresses[0])
        leaving = domain.leave()
        leaving.cancel()
        with pytest.raises(asyncio.CancelledError):
            await leaving
        await wait_stopped(status)
        await wait_rebind(port)
        root.cleanup()

        # Exercise the Python GC cycle: Domain -> provider callback -> Domain.
        class CyclicProvider(CountingCatalog):
            domain = None

        provider = CyclicProvider([])
        domain, root, _peer, _session = await join_domain(
            vectors["domain_id"], identity(112), provider=provider
        )
        status = domain.subscribe_status()
        port = tcp_port(domain.listen_addresses[0])
        provider.domain = domain
        del domain
        del provider
        # The completed PyO3 join bridge can retain its result until the next
        # event-loop turn. Collect repeatedly so this proves cleanup once the
        # object graph is actually unreachable, rather than depending on that
        # bridge's callback scheduling detail.
        for _ in range(10):
            await asyncio.sleep(0)
            gc.collect()
            if status.current().state == "stopped":
                break
        await wait_stopped(status)
        await wait_rebind(port)
        root.cleanup()

    asyncio.run(asyncio.wait_for(scenario(), 30))


def test_python_stream_provider_and_live_participant_sampling() -> None:
    vectors = json.loads(AUTH_VECTORS.read_text())

    async def scenario() -> None:
        import auki_domain

        producer_identity = identity(114)
        consumer_identity = identity(115)
        source_finished = asyncio.Event()
        source_never_finishes = asyncio.Event()
        stream_calls = 0
        participant_calls = 0

        def participant_provider():
            nonlocal participant_calls
            participant_calls += 1
            return auki_domain.ParticipantInfo(
                "python-stream-test",
                "1.0.0",
                "producer",
                "session",
                "clock",
                "clock-hash",
                participant_calls,
                producer_identity.peer_id,
            )

        async def camera_source():
            try:
                yield auki_domain.StreamItem(
                    timestamp_ns=123_456,
                    payload=auki_domain.CameraFrame(b"camera-frame"),
                )
                await source_never_finishes.wait()
            finally:
                source_finished.set()

        def stream_provider(requester_peer_id, request):
            nonlocal stream_calls
            stream_calls += 1
            assert requester_peer_id == consumer_identity.peer_id
            assert request.resource_id == "camera"
            assert request.source_peer_id == producer_identity.peer_id
            return auki_domain.StreamDecision.accept_camera(
                manifest=auki_domain.StreamManifest(
                    sensor_id="camera",
                    sensor_hash="sensor-hash",
                    clock_id="clock",
                    clock_hash="clock-hash",
                ),
                source=camera_source(),
            )

        producer, producer_root, _peer, _session = await join_domain(
            vectors["domain_id"],
            producer_identity,
            participant_provider=participant_provider,
            stream_provider=stream_provider,
        )
        consumer, consumer_root, _peer, _session = await join_domain(
            vectors["domain_id"], consumer_identity
        )
        consumer.routes().replace(producer.peer_id, producer.listen_addresses)
        producer.routes().replace(consumer.peer_id, consumer.listen_addresses)

        first_info = await consumer.fetch_participant_info(producer.peer_id)
        second_info = await consumer.fetch_participant_info(producer.peer_id)
        assert first_info.session_now_ns < second_info.session_now_ns
        assert participant_calls >= 3  # installation sample plus both fetches

        request = auki_domain.StreamRequest(
            "camera", producer.peer_id, auki_domain.ReadFrom.latest()
        )
        subscription = await consumer.open_stream(
            producer.peer_id, request, "camera"
        )
        assert subscription.manifest.sensor_id == "camera"
        entry = await asyncio.wait_for(subscription.recv(), 5)
        assert entry.seq == 0
        assert entry.timestamp_ns == 123_456
        assert entry.payload == auki_domain.CameraFrame(b"camera-frame")
        assert stream_calls == 1

        # Dropping the consumer must close the authenticated stream, cancel
        # the suspended Python source, and execute its finally block.
        del subscription
        gc.collect()
        await asyncio.wait_for(source_finished.wait(), 5)

        await asyncio.wait_for(asyncio.gather(consumer.leave(), producer.leave()), 10)
        consumer_root.cleanup()
        producer_root.cleanup()

    asyncio.run(asyncio.wait_for(scenario(), 30))


def test_cancelled_join_eventually_releases_fixed_listener() -> None:
    vectors = json.loads(AUTH_VECTORS.read_text())

    async def scenario() -> None:
        import auki_domain

        # join() starts a binding-owned native task before returning its Python
        # Future. Cancelling immediately closes the result receiver, but must
        # not cancel native join halfway through listener startup. If native
        # join succeeds, its unclaimed-result guard performs ordered leave().
        port = unused_tcp_port()
        local_identity = identity(113)
        root = tempfile.TemporaryDirectory()
        peer, session = peer_and_session(local_identity, root.name)
        config = auki_domain.DomainConfig(vectors["domain_id"], local_identity)
        config.with_listen_addresses([f"/ip4/127.0.0.1/tcp/{port}"])
        builder = auki_domain.Domain.builder(peer, session, config)
        keys, credential = auki_domain._test_authority(
            local_identity, vectors["domain_id"]
        )
        builder.authority(keys, credential)

        joining = builder.join()
        joining.cancel()
        with pytest.raises(asyncio.CancelledError):
            await joining

        # Allow the independently owned native task to finish either join or
        # its cleanup before testing the fixed listener. A leaked successful
        # join remains accepting and cannot be rebound, which wait_rebind
        # detects independently of Python object lifetime.
        await asyncio.sleep(0.5)
        await wait_rebind(port, timeout=10)
        root.cleanup()

    asyncio.run(asyncio.wait_for(scenario(), 15))

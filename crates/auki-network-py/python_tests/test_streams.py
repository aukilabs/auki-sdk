"""Python-side tests for grimsby's `Stream<T>` surface (deliverable #4).

Run via::

    cd crates/auki-network-py
    maturin develop --release
    pytest python_tests/test_streams.py

Two-tier coverage:

1. **Surface tests** — type construction, getters, tagged-enum factories.
   Fast; no SDK runtime.
2. **Cross-language conformance** — two `cluster.spawn` instances in the
   same process: a Python producer with `stream_provider` accepting and
   yielding three `JpegFrame`s, a Python consumer calling `open_stream`
   and iterating the frames. Mirrors the Rust
   `producer_accepts_and_streams_jpeg_frames` test shape.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest

from auki_network import cluster


# Reuse the seed → peer_id constants from `test_basic.py`. Computed via
# `cargo test print_python_e2e_peer_ids -- --nocapture`.
PEER_ID_SEED_10 = "12D3KooWG3t2M63pjiZP7UHsWruK1tQomm9kMsTm4FS3YMTfE6ao"
PEER_ID_SEED_11 = "12D3KooWPqT2nMDSiXUSx5D7fasaxhxKigVhcqfkKqrLghCq9jxz"


def _port_pair(offset: int) -> tuple[int, int]:
    """Return a fresh `(port_a, port_b)` pair for each test that spawns
    a two-runtime cluster. Sequential tests on the same loopback port pair
    can collide on macOS `TIME_WAIT` states; staggering by test gets us
    clear of that. Range 45070-45099 is reserved for streams tests; basic
    tests use 45050-45069."""
    return (45070 + offset * 2, 45071 + offset * 2)


# ─── Surface tests ───────────────────────────────────────────────────────────


def test_stream_request_round_trips() -> None:
    r = cluster.StreamRequest(sensor_id="K1-AABB/head_left_cam")
    assert r.sensor_id == "K1-AABB/head_left_cam"
    assert "head_left_cam" in repr(r)


def test_accept_info_round_trips_and_compares() -> None:
    a = cluster.AcceptInfo(sensor_hash="h", clock_id="c", clock_hash="ch")
    b = cluster.AcceptInfo(sensor_hash="h", clock_id="c", clock_hash="ch")
    assert a == b
    assert a.sensor_hash == "h"
    assert a.clock_id == "c"
    assert a.clock_hash == "ch"
    c = cluster.AcceptInfo(sensor_hash="other", clock_id="c", clock_hash="ch")
    assert a != c


def test_jpeg_frame_carries_bytes() -> None:
    payload = b"\xff\xd8\x01\x02\x03"
    f = cluster.JpegFrame(payload)
    assert f.bytes == payload
    assert len(f) == len(payload)


def test_pointcloud_frame_carries_bytes() -> None:
    """Dagaz Batch 2 — `cluster.PointCloudFrame` is the analog of
    `JpegFrame` for raw CDR-encoded `PointCloud2` ROS payloads. Same
    shape, same accessors; the SDK doesn't decode either."""
    payload = b"\x00\x01\x02\x03\xff"
    f = cluster.PointCloudFrame(payload)
    assert f.bytes == payload
    assert len(f) == len(payload)
    assert "PointCloudFrame" in repr(f)


def test_decline_reason_factories() -> None:
    nf = cluster.DeclineReason.sensor_not_found()
    assert nf.kind == "sensor_not_found"
    assert nf.detail is None

    other = cluster.DeclineReason.other(detail="custom reason")
    assert other.kind == "other"
    assert other.detail == "custom reason"

    # Equality follows the inner Rust enum.
    assert nf == cluster.DeclineReason.sensor_not_found()
    assert nf != cluster.DeclineReason.sensor_unavailable()


def test_end_reason_factories() -> None:
    assert cluster.EndReason.source_ended().kind == "source_ended"
    perr = cluster.EndReason.producer_error(detail="encoder died")
    assert perr.kind == "producer_error"
    assert perr.detail == "encoder died"


def test_producer_frame_round_trips() -> None:
    pf = cluster.ProducerFrame(timestamp_ns=12345, payload=cluster.JpegFrame(b"abc"))
    assert pf.timestamp_ns == 12345
    assert isinstance(pf.payload, cluster.JpegFrame)
    assert pf.payload.bytes == b"abc"


def test_producer_frame_accepts_pointcloud_payload() -> None:
    """Dagaz Batch 2 — `ProducerFrame.payload` accepts either
    `JpegFrame` or `PointCloudFrame`. The substream-typed dispatch
    happens later, when the source iterator is paired with a
    `StreamDecision.accept_pointcloud(...)` Accept variant."""
    pf = cluster.ProducerFrame(
        timestamp_ns=42_000, payload=cluster.PointCloudFrame(b"\x01\x02")
    )
    assert pf.timestamp_ns == 42_000
    assert isinstance(pf.payload, cluster.PointCloudFrame)
    assert pf.payload.bytes == b"\x01\x02"


def test_producer_frame_rejects_unknown_payload_type() -> None:
    """Anything other than JpegFrame / PointCloudFrame is a
    `ValueError` at construction time."""
    with pytest.raises(ValueError, match="JpegFrame"):
        cluster.ProducerFrame(timestamp_ns=0, payload="not-a-frame")


def test_stream_decision_factory_tags() -> None:
    info = cluster.AcceptInfo(sensor_hash="h", clock_id="c", clock_hash="ch")

    async def _empty():
        return
        yield  # unreachable; makes this an async generator function

    acc = cluster.StreamDecision.accept(info=info, source=_empty())
    assert acc.kind == "accept"

    acc_pc = cluster.StreamDecision.accept_pointcloud(info=info, source=_empty())
    assert acc_pc.kind == "accept_pointcloud"

    dec = cluster.StreamDecision.decline(cluster.DeclineReason.sensor_not_found())
    assert dec.kind == "decline"


# ─── Cross-`.so` bridge for sibling PyO3 wrapper crates ─────────────────────


def test_build_stream_provider_helper_returns_named_capsule() -> None:
    """`auki_network.cluster._build_stream_provider(callable)` returns a
    `PyCapsule` whose name pins the bridge contract with sibling wrapper
    crates (`auki-domain-py` consumes it via `PyCapsule::reference`).

    The bridge exists because each PyO3 `#[pyclass]` gets a distinct
    type-id per cdylib that registers it — so a `PyStreamDecision`
    constructed in `auki_network.so` cannot be `PyRef::extract`'d in
    `auki_domain.so`. Routing the closure construction through this
    helper keeps the extract local to `auki_network.so`; the capsule
    payload is type-id-free.

    Regression catch: rename / delete the helper or rev the capsule
    name without coordinating with `auki-domain-py`'s consumer and this
    test fails loudly before the runtime mismatch can ship.
    """
    import ctypes

    def _cb(_req):
        # Never invoked — the bridge construction doesn't run the
        # callable, it just wraps it in a Rust closure that becomes
        # the capsule payload.
        raise AssertionError("callback unexpectedly invoked")

    capsule = cluster._build_stream_provider(_cb)

    # PyCapsule is a built-in type with no public Python class; the
    # cleanest cross-version check is the type name.
    assert type(capsule).__name__ == "PyCapsule", (
        f"expected PyCapsule, got {type(capsule).__name__}"
    )

    # Pin the capsule's name to the canonical bridge string. Renaming
    # this string is a wire break with `auki-domain-py`'s consumer
    # (the `stream_provider_from_python` helper there validates this
    # exact name before unboxing the Arc).
    ctypes.pythonapi.PyCapsule_GetName.restype = ctypes.c_char_p
    ctypes.pythonapi.PyCapsule_GetName.argtypes = [ctypes.py_object]
    name = ctypes.pythonapi.PyCapsule_GetName(capsule)
    assert name == b"auki_network_py::stream_provider::v1", (
        f"capsule name drifted: {name!r}"
    )


# ─── Cross-language conformance ─────────────────────────────────────────────


def write_cluster_json(path: Path, peers: list[dict]) -> Path:
    doc = {"version": 1, "cluster_name": "stream-conformance", "peers": peers}
    path.write_text(json.dumps(doc))
    return path


def _provider_for(peer_id: str, app: str, name: str):
    """Mirror of test_basic._provider_for — builds a ParticipantInfo with
    a fresh `session_now_ns` on each call."""

    def provider() -> cluster.ParticipantInfo:
        return cluster.ParticipantInfo(
            app=app,
            name=name,
            session_id=f"session-{name}",
            session_clock_id=f"{name}/clock",
            session_clock_hash="deadbeef",
            session_now_ns=time.monotonic_ns(),
            cluster_joined_at_ns=None,
            peer_id=peer_id,
            app_instance="00163eabcdef",
        )

    return provider


def _two_peer_doc(tmp_path: Path, port_a: int, port_b: int) -> cluster.ClusterDoc:
    path = write_cluster_json(
        tmp_path / "cluster.json",
        peers=[
            {
                "peer_id": PEER_ID_SEED_10,
                "addresses": [f"/ip4/127.0.0.1/tcp/{port_a}"],
            },
            {
                "peer_id": PEER_ID_SEED_11,
                "addresses": [f"/ip4/127.0.0.1/tcp/{port_b}"],
            },
        ],
    )
    return cluster.load_doc(str(path))


def test_python_producer_python_consumer_round_trip_jpeg(tmp_path: Path) -> None:
    """Cross-language conformance vector (per the grimsby task queue):
    two `cluster.spawn` instances in one process — Python producer +
    Python consumer — round-trip `JpegFrame`s end-to-end.

    Mirrors `auki_network::stream_runtime::tests::producer_accepts_and_streams_jpeg_frames`.
    """
    port_a, port_b = _port_pair(0)
    doc = _two_peer_doc(tmp_path, port_a, port_b)

    # Producer side.
    accepted_count = 0

    def producer_stream(req: cluster.StreamRequest) -> cluster.StreamDecision:
        nonlocal accepted_count
        if req.sensor_id != "test/cam":
            return cluster.StreamDecision.decline(
                cluster.DeclineReason.sensor_not_found()
            )
        accepted_count += 1

        async def gen():
            yield cluster.ProducerFrame(
                timestamp_ns=1_000, payload=cluster.JpegFrame(b"\xff\xd8\x01")
            )
            yield cluster.ProducerFrame(
                timestamp_ns=2_000, payload=cluster.JpegFrame(b"\xff\xd8\x02")
            )
            yield cluster.ProducerFrame(
                timestamp_ns=3_000, payload=cluster.JpegFrame(b"\xff\xd8\x03")
            )

        return cluster.StreamDecision.accept(
            info=cluster.AcceptInfo(
                sensor_hash="sensor-hash-3",
                clock_id="test/session-monotonic",
                clock_hash="clock-hash-3",
            ),
            source=gen(),
        )

    rt_producer = cluster.spawn(
        seed=b"\x10" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_10, "boosterapp", "robot-a"),
        stream_provider=producer_stream,
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_a}"],
        enable_mdns=False,
    )

    # Consumer side. No stream_provider → consumer-only (decline_all_streams
    # behind the scenes).
    rt_consumer = cluster.spawn(
        seed=b"\x11" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_11, "park", "consumer"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_b}"],
        enable_mdns=False,
    )

    try:
        # Wait for cluster connect so open_stream routes through the
        # existing connection rather than a fresh dial.
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if rt_consumer.peers() and rt_producer.peers():
                break
            time.sleep(0.1)
        assert rt_consumer.peers(), "consumer did not see producer within 15s"

        sub = rt_consumer.open_stream(
            peer_id=PEER_ID_SEED_10,
            sensor_id="test/cam",
        )
        assert sub.info.sensor_hash == "sensor-hash-3"
        assert sub.info.clock_id == "test/session-monotonic"
        assert sub.info.clock_hash == "clock-hash-3"

        frames = sub.frames()

        # Drain three frames — order, seq, timestamp, payload all locked.
        f0 = next(frames)
        assert f0.seq == 0
        assert f0.timestamp_ns == 1_000
        assert f0.payload.bytes == b"\xff\xd8\x01"

        f1 = next(frames)
        assert f1.seq == 1
        assert f1.timestamp_ns == 2_000

        f2 = next(frames)
        assert f2.seq == 2
        assert f2.payload.bytes == b"\xff\xd8\x03"

        # Producer's generator returned → SDK writes
        # `EndOfStream { SourceEnded }` → Python sees `StreamEndOfStream`.
        with pytest.raises(cluster.StreamEndOfStream) as excinfo:
            next(frames)
        reason = excinfo.value.args[0]
        assert isinstance(reason, cluster.EndReason)
        assert reason.kind == "source_ended"

        # After the terminator, the iterator is exhausted.
        with pytest.raises(StopIteration):
            next(frames)

        assert accepted_count == 1, "provider should have been invoked exactly once"
    finally:
        rt_producer.shutdown()
        rt_consumer.shutdown()


def test_open_stream_against_unknown_sensor_raises_declined(tmp_path: Path) -> None:
    """Producer's `stream_provider` declines the request → consumer's
    `open_stream` raises `StreamDeclined` carrying a typed
    `DeclineReason`. No frames flow."""
    port_a, port_b = _port_pair(1)
    doc = _two_peer_doc(tmp_path, port_a, port_b)

    def producer_stream(req: cluster.StreamRequest) -> cluster.StreamDecision:
        return cluster.StreamDecision.decline(cluster.DeclineReason.sensor_not_found())

    rt_producer = cluster.spawn(
        seed=b"\x10" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_10, "boosterapp", "robot-a"),
        stream_provider=producer_stream,
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_a}"],
        enable_mdns=False,
    )
    rt_consumer = cluster.spawn(
        seed=b"\x11" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_11, "park", "consumer"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_b}"],
        enable_mdns=False,
    )

    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if rt_consumer.peers():
                break
            time.sleep(0.1)
        assert rt_consumer.peers(), "consumer did not see producer within 15s"

        with pytest.raises(cluster.StreamDeclined) as excinfo:
            rt_consumer.open_stream(peer_id=PEER_ID_SEED_10, sensor_id="anything")
        reason = excinfo.value.args[0]
        assert isinstance(reason, cluster.DeclineReason)
        assert reason.kind == "sensor_not_found"
    finally:
        rt_producer.shutdown()
        rt_consumer.shutdown()


def test_python_producer_python_consumer_round_trip_pointcloud(
    tmp_path: Path,
) -> None:
    """Dagaz Batch 2 cross-language conformance vector — same shape as
    the JPEG round-trip but with `PointCloudFrame` end-to-end. Producer
    accepts via `accept_pointcloud(...)`; consumer opens via
    `open_pointcloud_stream(...)`. Verifies the SDK routes both `T`s
    through one shared swarm without crosstalk.

    Mirrors `auki_network::stream_runtime::tests::producer_accepts_and_streams_pointcloud_frames`
    on the Rust side.
    """
    port_a, port_b = _port_pair(2)
    doc = _two_peer_doc(tmp_path, port_a, port_b)

    # Producer side — sensor_id="lidar/points" → accept_pointcloud.
    accepted_count = 0

    def producer_stream(req: cluster.StreamRequest) -> cluster.StreamDecision:
        nonlocal accepted_count
        if req.sensor_id != "lidar/points":
            return cluster.StreamDecision.decline(
                cluster.DeclineReason.sensor_not_found()
            )
        accepted_count += 1

        async def gen():
            yield cluster.ProducerFrame(
                timestamp_ns=10_000,
                payload=cluster.PointCloudFrame(b"\x00\x01\x02"),
            )
            yield cluster.ProducerFrame(
                timestamp_ns=20_000,
                payload=cluster.PointCloudFrame(b"\x10\x11"),
            )
            yield cluster.ProducerFrame(
                timestamp_ns=30_000,
                payload=cluster.PointCloudFrame(b"\xa0\xa1\xa2\xa3"),
            )

        return cluster.StreamDecision.accept_pointcloud(
            info=cluster.AcceptInfo(
                sensor_hash="pc-sensor",
                clock_id="lidar/clock",
                clock_hash="pc-clock-hash",
            ),
            source=gen(),
        )

    rt_producer = cluster.spawn(
        seed=b"\x10" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_10, "boosterapp", "robot-a"),
        stream_provider=producer_stream,
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_a}"],
        enable_mdns=False,
    )
    rt_consumer = cluster.spawn(
        seed=b"\x11" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_11, "park", "consumer"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_b}"],
        enable_mdns=False,
    )

    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if rt_consumer.peers() and rt_producer.peers():
                break
            time.sleep(0.1)
        assert rt_consumer.peers(), "consumer did not see producer within 15s"

        sub = rt_consumer.open_pointcloud_stream(
            peer_id=PEER_ID_SEED_10,
            sensor_id="lidar/points",
        )
        assert sub.info.sensor_hash == "pc-sensor"
        assert sub.info.clock_id == "lidar/clock"

        frames = sub.frames()

        f0 = next(frames)
        assert f0.seq == 0
        assert f0.timestamp_ns == 10_000
        assert isinstance(f0.payload, cluster.PointCloudFrame)
        assert f0.payload.bytes == b"\x00\x01\x02"

        f1 = next(frames)
        assert f1.seq == 1
        assert f1.timestamp_ns == 20_000
        assert f1.payload.bytes == b"\x10\x11"

        f2 = next(frames)
        assert f2.seq == 2
        assert f2.payload.bytes == b"\xa0\xa1\xa2\xa3"

        with pytest.raises(cluster.StreamEndOfStream) as excinfo:
            next(frames)
        reason = excinfo.value.args[0]
        assert reason.kind == "source_ended"

        assert accepted_count == 1
    finally:
        rt_producer.shutdown()
        rt_consumer.shutdown()


def test_payload_mismatch_ends_stream_with_producer_error(tmp_path: Path) -> None:
    """A producer that says `accept_pointcloud(...)` but yields a
    `JpegFrame` ends the stream with `EndReason::ProducerError` —
    each substream is mono-`T`, the SDK rejects the wrong payload
    variant rather than coercing it."""
    port_a, port_b = _port_pair(3)
    doc = _two_peer_doc(tmp_path, port_a, port_b)

    def producer_stream(req: cluster.StreamRequest) -> cluster.StreamDecision:
        async def gen():
            # Wrong T — substream is PointCloud, but we yield a JPEG frame.
            yield cluster.ProducerFrame(
                timestamp_ns=1, payload=cluster.JpegFrame(b"jpeg-bytes")
            )

        return cluster.StreamDecision.accept_pointcloud(
            info=cluster.AcceptInfo(
                sensor_hash="pc", clock_id="c", clock_hash="ch"
            ),
            source=gen(),
        )

    rt_producer = cluster.spawn(
        seed=b"\x10" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_10, "boosterapp", "robot-a"),
        stream_provider=producer_stream,
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_a}"],
        enable_mdns=False,
    )
    rt_consumer = cluster.spawn(
        seed=b"\x11" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_11, "park", "consumer"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_b}"],
        enable_mdns=False,
    )

    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if rt_consumer.peers():
                break
            time.sleep(0.1)
        assert rt_consumer.peers(), "consumer did not see producer within 15s"

        sub = rt_consumer.open_pointcloud_stream(
            peer_id=PEER_ID_SEED_10, sensor_id="any"
        )
        frames = sub.frames()

        with pytest.raises(cluster.StreamEndOfStream) as excinfo:
            next(frames)
        reason = excinfo.value.args[0]
        assert reason.kind == "producer_error"
        # Detail names which T it was expecting.
        assert "PointCloudFrame" in (reason.detail or "")
    finally:
        rt_producer.shutdown()
        rt_consumer.shutdown()


def test_open_stream_after_shutdown_raises_runtime_error(tmp_path: Path) -> None:
    """Trying to open a stream after `runtime.shutdown()` raises
    `RuntimeError` rather than hanging or panicking. Same shape for the
    pointcloud method."""
    doc = cluster.load_doc(
        str(write_cluster_json(tmp_path / "cluster.json", peers=[]))
    )
    rt = cluster.spawn(
        seed=b"\x42" * 32,
        doc=doc,
        participant_provider=lambda: None,
        listen_addresses=["/ip4/127.0.0.1/tcp/0"],
        enable_mdns=False,
    )
    rt.shutdown()
    with pytest.raises(RuntimeError, match="shut down"):
        rt.open_stream(peer_id=PEER_ID_SEED_10, sensor_id="any")
    with pytest.raises(RuntimeError, match="shut down"):
        rt.open_pointcloud_stream(peer_id=PEER_ID_SEED_10, sensor_id="any")


def test_open_stream_with_invalid_peer_id_raises_value_error(tmp_path: Path) -> None:
    doc = cluster.load_doc(
        str(write_cluster_json(tmp_path / "cluster.json", peers=[]))
    )
    rt = cluster.spawn(
        seed=b"\x42" * 32,
        doc=doc,
        participant_provider=lambda: None,
        listen_addresses=["/ip4/127.0.0.1/tcp/0"],
        enable_mdns=False,
    )
    try:
        with pytest.raises(ValueError, match="invalid peer_id"):
            rt.open_stream(peer_id="not-a-peer-id", sensor_id="any")
    finally:
        rt.shutdown()


# ─── JointEncoders (sawslin Phase B) ────────────────────────────────────────


def test_joint_encoders_frame_carries_angles() -> None:
    """sawslin Phase B — `cluster.JointEncodersFrame` is the third
    payload `T`. Differs from `JpegFrame` / `PointCloudFrame` in
    payload shape: a `list[float]` of joint angles (radians) instead
    of opaque `bytes`. Length is producer-defined and pinned by the
    `JointEncoders { joint_count }` registry body."""
    angles = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]
    f = cluster.JointEncodersFrame(angles)
    assert f.angles_rad == angles
    assert len(f) == len(angles)
    assert "JointEncodersFrame" in repr(f)
    # Equality follows the inner Rust struct.
    assert f == cluster.JointEncodersFrame(angles)
    assert f != cluster.JointEncodersFrame(angles[:5])


def test_joint_encoders_frame_empty_vector() -> None:
    """Empty `angles_rad` is a valid wire shape — proto3 default-elision
    means an empty vector encodes to zero bytes. The SDK surfaces it
    verbatim; `joint_count` enforcement is the consumer's job."""
    f = cluster.JointEncodersFrame([])
    assert f.angles_rad == []
    assert len(f) == 0


def test_producer_frame_accepts_joint_encoders_payload() -> None:
    pf = cluster.ProducerFrame(
        timestamp_ns=99_000, payload=cluster.JointEncodersFrame([0.1, 0.2, 0.3])
    )
    assert pf.timestamp_ns == 99_000
    assert isinstance(pf.payload, cluster.JointEncodersFrame)
    assert pf.payload.angles_rad == [
        pytest.approx(0.1),
        pytest.approx(0.2),
        pytest.approx(0.3),
    ]


def test_stream_decision_accept_joint_encoders_factory() -> None:
    info = cluster.AcceptInfo(sensor_hash="h", clock_id="c", clock_hash="ch")

    async def _empty():
        return
        yield  # unreachable; makes this an async generator function

    acc_je = cluster.StreamDecision.accept_joint_encoders(info=info, source=_empty())
    assert acc_je.kind == "accept_joint_encoders"


def test_python_producer_python_consumer_round_trip_joint_encoders(
    tmp_path: Path,
) -> None:
    """sawslin Phase B cross-language conformance vector — same shape
    as the JPEG/PointCloud round-trips but with `JointEncodersFrame`
    end-to-end. Producer accepts via `accept_joint_encoders(...)`;
    consumer opens via `open_joint_encoders_stream(...)`. Verifies the
    SDK routes the third `T` through the same shared swarm without
    crosstalk against the other two."""
    port_a, port_b = _port_pair(4)
    doc = _two_peer_doc(tmp_path, port_a, port_b)

    # 6-DOF arm fixture — matches the SDK's locked
    # `joint_encoders_disk_wire_byte_identical` test vector.
    SAMPLES = [
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        [1.1, 2.1, 3.1, 4.1, 5.1, 6.1],
        [1.2, 2.2, 3.2, 4.2, 5.2, 6.2],
    ]
    accepted_count = 0

    def producer_stream(req: cluster.StreamRequest) -> cluster.StreamDecision:
        nonlocal accepted_count
        if req.sensor_id != "arm/joints":
            return cluster.StreamDecision.decline(
                cluster.DeclineReason.sensor_not_found()
            )
        accepted_count += 1

        async def gen():
            for i, angles in enumerate(SAMPLES):
                yield cluster.ProducerFrame(
                    timestamp_ns=10_000 * (i + 1),
                    payload=cluster.JointEncodersFrame(angles),
                )

        return cluster.StreamDecision.accept_joint_encoders(
            info=cluster.AcceptInfo(
                sensor_hash="je-sensor",
                clock_id="arm/clock",
                clock_hash="je-clock-hash",
            ),
            source=gen(),
        )

    rt_producer = cluster.spawn(
        seed=b"\x10" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_10, "boosterapp", "robot-a"),
        stream_provider=producer_stream,
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_a}"],
        enable_mdns=False,
    )
    rt_consumer = cluster.spawn(
        seed=b"\x11" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_11, "park", "consumer"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_b}"],
        enable_mdns=False,
    )

    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if rt_consumer.peers() and rt_producer.peers():
                break
            time.sleep(0.1)
        assert rt_consumer.peers(), "consumer did not see producer within 15s"

        sub = rt_consumer.open_joint_encoders_stream(
            peer_id=PEER_ID_SEED_10,
            sensor_id="arm/joints",
        )
        assert sub.info.sensor_hash == "je-sensor"
        assert sub.info.clock_id == "arm/clock"

        frames = sub.frames()

        for i, expected in enumerate(SAMPLES):
            f = next(frames)
            assert f.seq == i
            assert f.timestamp_ns == 10_000 * (i + 1)
            assert isinstance(f.payload, cluster.JointEncodersFrame)
            assert f.payload.angles_rad == [pytest.approx(a) for a in expected]

        with pytest.raises(cluster.StreamEndOfStream) as excinfo:
            next(frames)
        reason = excinfo.value.args[0]
        assert reason.kind == "source_ended"

        assert accepted_count == 1
    finally:
        rt_producer.shutdown()
        rt_consumer.shutdown()


def test_joint_encoders_payload_mismatch_ends_with_producer_error(
    tmp_path: Path,
) -> None:
    """A producer that says `accept_joint_encoders(...)` but yields a
    `JpegFrame` ends the stream with `EndReason::ProducerError` — same
    mono-T enforcement as the JPEG/PointCloud variants."""
    port_a, port_b = _port_pair(5)
    doc = _two_peer_doc(tmp_path, port_a, port_b)

    def producer_stream(req: cluster.StreamRequest) -> cluster.StreamDecision:
        async def gen():
            # Wrong T — substream is JointEncoders, but we yield a JPEG frame.
            yield cluster.ProducerFrame(
                timestamp_ns=1, payload=cluster.JpegFrame(b"jpeg-bytes")
            )

        return cluster.StreamDecision.accept_joint_encoders(
            info=cluster.AcceptInfo(
                sensor_hash="je", clock_id="c", clock_hash="ch"
            ),
            source=gen(),
        )

    rt_producer = cluster.spawn(
        seed=b"\x10" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_10, "boosterapp", "robot-a"),
        stream_provider=producer_stream,
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_a}"],
        enable_mdns=False,
    )
    rt_consumer = cluster.spawn(
        seed=b"\x11" * 32,
        doc=doc,
        participant_provider=_provider_for(PEER_ID_SEED_11, "park", "consumer"),
        listen_addresses=[f"/ip4/127.0.0.1/tcp/{port_b}"],
        enable_mdns=False,
    )

    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if rt_consumer.peers():
                break
            time.sleep(0.1)
        assert rt_consumer.peers(), "consumer did not see producer within 15s"

        sub = rt_consumer.open_joint_encoders_stream(
            peer_id=PEER_ID_SEED_10, sensor_id="any"
        )
        frames = sub.frames()

        with pytest.raises(cluster.StreamEndOfStream) as excinfo:
            next(frames)
        reason = excinfo.value.args[0]
        assert reason.kind == "producer_error"
        # Detail names which T it was expecting.
        assert "JointEncodersFrame" in (reason.detail or "")
    finally:
        rt_producer.shutdown()
        rt_consumer.shutdown()

"""Python-side tests for grimsby's `Stream<T>` surface (deliverable #4).

Run via::

    cd bindings/python/auki-network-py
    maturin develop --release
    pytest python_tests/test_streams.py

Two-tier coverage:

1. **Surface tests** — type construction, getters, tagged-enum factories.
   Fast; no SDK runtime.
2. **Provider bridge tests** — the SDK-internal PyCapsule bridge exists
   for `auki-domain-py`, without reintroducing network runtime ownership
   into this package.
"""

from __future__ import annotations

import struct

import pytest

import auki_logs
from auki_network import cluster


# ─── Surface tests ───────────────────────────────────────────────────────────


def test_stream_request_round_trips() -> None:
    r = cluster.StreamRequest(sensor_id="K1-AABB/head_left_cam")
    assert r.sensor_id == "K1-AABB/head_left_cam"
    assert "head_left_cam" in repr(r)


def test_stream_manifest_round_trips_and_compares() -> None:
    a = cluster.StreamManifest(
        sensor_id="sensor", sensor_hash="h", clock_id="c", clock_hash="ch",
        frame_id="frame", frame_hash="fh"
    )
    b = cluster.StreamManifest(
        sensor_id="sensor", sensor_hash="h", clock_id="c", clock_hash="ch",
        frame_id="frame", frame_hash="fh"
    )
    assert a == b
    assert a.sensor_id == "sensor"
    assert a.sensor_hash == "h"
    assert a.clock_id == "c"
    assert a.clock_hash == "ch"
    assert a.frame_id == "frame"
    assert a.frame_hash == "fh"
    c = cluster.StreamManifest(
        sensor_id="sensor", sensor_hash="other", clock_id="c", clock_hash="ch",
        frame_id="frame", frame_hash="fh"
    )
    assert a != c


def test_camera_frame_carries_frame_and_intrinsics() -> None:
    payload = b"\xff\xd8\x01\x02\x03"
    intrinsics = cluster.DynamicIntrinsics(
        fx=400.0, fy=401.0, cx=272.5, cy=244.5,
        distortion_coefficients=[0.1, -0.2],
    )
    f = cluster.CameraFrame(payload, dynamic_intrinsics=intrinsics)
    assert f.frame == payload
    assert f.dynamic_intrinsics == intrinsics
    assert len(f) == len(payload)
    assert "CameraFrame" in repr(f)


def test_no_pinhole_camera_log_entry_python_alias() -> None:
    assert not hasattr(cluster, "PinholeCameraLogEntry")


def test_pointcloud_frame_carries_bytes() -> None:
    """Dagaz Batch 2 — `cluster.PointCloudFrame` is the analog of
    `CameraFrame` for raw CDR-encoded `PointCloud2` ROS payloads.
    It remains an opaque stream-specific wrapper; the SDK doesn't decode it."""
    payload = b"\x00\x01\x02\x03\xff"
    f = cluster.PointCloudFrame(payload)
    assert f.bytes == payload
    assert len(f) == len(payload)
    assert "PointCloudFrame" in repr(f)


def test_audio_frame_carries_bytes() -> None:
    """Dialogue Batch 1 — `cluster.AudioFrame` is the audio analog of
    `CameraFrame` / `PointCloudFrame` for interleaved PCM bytes. Same
    opaque-bytes pattern, but the getter is named `.data` to match
    the underlying `bytes data = 1` proto field (not `bytes bytes
    = 1`)."""
    payload = b"\x00\x80\xff\x7f\x40\x40\xc0\xbf"
    f = cluster.AudioFrame(payload)
    assert f.data == payload
    assert len(f) == len(payload)
    assert "AudioFrame" in repr(f)


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


def test_stream_item_round_trips() -> None:
    pf = cluster.StreamItem(timestamp_ns=12345, payload=cluster.CameraFrame(b"abc"))
    assert pf.timestamp_ns == 12345
    assert isinstance(pf.payload, cluster.CameraFrame)
    assert pf.payload.frame == b"abc"


def test_stream_item_accepts_pointcloud_payload() -> None:
    """Dagaz Batch 2 — `StreamItem.payload` accepts either
    `CameraFrame` or `PointCloudFrame`. The substream-typed dispatch
    happens later, when the source iterator is paired with a
    `StreamDecision.accept_pointcloud(...)` Accept variant."""
    pf = cluster.StreamItem(
        timestamp_ns=42_000, payload=cluster.PointCloudFrame(b"\x01\x02")
    )
    assert pf.timestamp_ns == 42_000
    assert isinstance(pf.payload, cluster.PointCloudFrame)
    assert pf.payload.bytes == b"\x01\x02"


def test_stream_item_accepts_audio_payload() -> None:
    """Dialogue Batch 1 — `StreamItem.payload` accepts `AudioFrame`
    alongside `CameraFrame` / `PointCloudFrame` / `JointEncodersFrame`.
    The substream-typed dispatch happens later, when the source
    iterator is paired with a `StreamDecision.accept_audio(...)`
    Accept variant."""
    pf = cluster.StreamItem(
        timestamp_ns=20_000, payload=cluster.AudioFrame(b"\xab\xcd")
    )
    assert pf.timestamp_ns == 20_000
    assert isinstance(pf.payload, cluster.AudioFrame)
    assert pf.payload.data == b"\xab\xcd"


def test_stream_item_rejects_unknown_payload_type() -> None:
    """Anything other than the supported frame types is a `ValueError`
    at construction time."""
    with pytest.raises(ValueError, match="AudioFrame"):
        cluster.StreamItem(timestamp_ns=0, payload="not-a-frame")


def test_stream_decision_factory_tags() -> None:
    manifest = cluster.StreamManifest(
        sensor_id="sensor", sensor_hash="h", clock_id="c", clock_hash="ch"
    )

    async def _empty():
        return
        yield  # unreachable; makes this an async generator function

    acc = cluster.StreamDecision.accept_camera(manifest=manifest, source=_empty())
    assert acc.kind == "accept_camera"

    acc_pc = cluster.StreamDecision.accept_pointcloud(manifest=manifest, source=_empty())
    assert acc_pc.kind == "accept_pointcloud"

    acc_audio = cluster.StreamDecision.accept_audio(manifest=manifest, source=_empty())
    assert acc_audio.kind == "accept_audio"

    dec = cluster.StreamDecision.decline(cluster.DeclineReason.sensor_not_found())
    assert dec.kind == "decline"


def _prost_varint(value: int) -> bytes:
    out = bytearray()
    while value >= 0x80:
        out.append((value & 0x7F) | 0x80)
        value >>= 7
    out.append(value)
    return bytes(out)


def _prost_bytes_field(field_number: int, payload: bytes) -> bytes:
    return bytes([(field_number << 3) | 2]) + _prost_varint(len(payload)) + payload


def _camera_log_payload(frame: bytes) -> bytes:
    return _prost_bytes_field(2, frame)


def _pointcloud_log_payload(data: bytes) -> bytes:
    return _prost_bytes_field(1, data)


def _joint_encoders_log_payload(angles: list[float]) -> bytes:
    packed = b"".join(struct.pack("<f", angle) for angle in angles)
    return _prost_bytes_field(1, packed)


def _audio_log_payload(data: bytes) -> bytes:
    return _prost_bytes_field(1, data)


def _retained_source(tmp_path, payload_kind: str, payload: bytes | None = None):
    log = auki_logs.Log.open(
        str(tmp_path),
        {
            "segment_duration_ns": 1_000_000_000,
            "retention_ns": 10_000_000_000,
            "kind": "test",
        },
    )
    try:
        if payload is not None:
            log.append(123_000, payload)
            log.flush()
        return log.stream_source(
            sensor_id=f"robot/{payload_kind}",
            sensor_hash="sensor-hash",
            clock_id="robot/clock",
            clock_hash="clock-hash",
            payload_kind=payload_kind,
            frame_id="robot/base",
            frame_hash="frame-hash",
        )
    finally:
        log.close()


@pytest.mark.parametrize(
    ("payload_kind", "decision_kind"),
    [
        ("camera", "accept_camera"),
        ("pointcloud", "accept_pointcloud"),
        ("joint_encoders", "accept_joint_encoders"),
        ("audio", "accept_audio"),
    ],
)
def test_stream_decision_accept_source_resolves_payload_kind(
    tmp_path, payload_kind: str, decision_kind: str
) -> None:
    source = _retained_source(tmp_path, payload_kind)

    decision = cluster.StreamDecision.accept_source(source)

    assert decision.kind == decision_kind


@pytest.mark.parametrize(
    ("payload_kind", "payload", "decision_kind"),
    [
        ("camera", _camera_log_payload(b"jpeg"), "accept_camera"),
        ("pointcloud", _pointcloud_log_payload(b"cdr"), "accept_pointcloud"),
        (
            "joint_encoders",
            _joint_encoders_log_payload([0.1, 0.2, 0.3]),
            "accept_joint_encoders",
        ),
        ("audio", _audio_log_payload(b"pcm"), "accept_audio"),
    ],
)
def test_stream_decision_accept_source_uses_retained_log_payloads(
    tmp_path, payload_kind: str, payload: bytes, decision_kind: str
) -> None:
    source = _retained_source(tmp_path, payload_kind, payload=payload)

    decision = cluster.StreamDecision.accept_source(source)

    assert decision.kind == decision_kind


def test_stream_decision_accept_source_keeps_camera_name_clean(tmp_path) -> None:
    source = _retained_source(tmp_path, "camera")

    decision = cluster.StreamDecision.accept_source(source)

    assert decision.kind == "accept_camera"
    assert not hasattr(cluster, "PinholeCameraLogEntry")


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


# ─── JointEncoders (sawslin Phase B) ────────────────────────────────────────


def test_joint_encoders_frame_carries_angles() -> None:
    """sawslin Phase B — `cluster.JointEncodersFrame` is the third
    payload `T`. Differs from `CameraFrame` / `PointCloudFrame` in
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


def test_stream_item_accepts_joint_encoders_payload() -> None:
    pf = cluster.StreamItem(
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
    manifest = cluster.StreamManifest(
        sensor_id="sensor", sensor_hash="h", clock_id="c", clock_hash="ch"
    )

    async def _empty():
        return
        yield  # unreachable; makes this an async generator function

    acc_je = cluster.StreamDecision.accept_joint_encoders(manifest=manifest, source=_empty())
    assert acc_je.kind == "accept_joint_encoders"

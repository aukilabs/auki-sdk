"""Smoke tests for the `auki_session` Python module.

Run after building the wheel:

    maturin develop -m bindings/python/auki-session-py/Cargo.toml
    pytest bindings/python/auki-session-py/python_tests/

These tests verify the Python binding surface for the declarative Peer and
Session APIs. A Peer owns device-level registries; ``start_session()`` creates
one session timeline and its clocks/logs.

Test inventory
--------------
- test_session_construction                    — Peer.start_session(), accessors
- test_with_storage_root                       — Peer builder, inherited session root
- test_session_id_is_ulid                      — 26-char ULID, unique per session
- test_register_frame_returns_registry_ref     — register_frame + RegistryRef shape
- test_register_clock_returns_registry_ref     — register_clock + RegistryRef shape
- test_register_sensor_returns_registry_ref    — register_sensor + RegistryRef shape
- test_register_sensor_log_end_to_end          — full log registration + catalog check
- test_register_duplicate_log_raises           — duplicate registration raises ValueError
- test_register_sensor_bad_id_raises           — invalid sensor_id raises ValueError
- test_materialize_remote_log_raises           — NotImplementedError with Phase-5 message
- test_resolve_static_transform_raises         — NotImplementedError with Phase-5 message
- test_frame_def_classmethods                  — FrameDef.ros_body/optical/opengl/unity repr
- test_head_spec_classmethods                  — HeadSpec.rolling/fixed repr

Note: test_log_ref_class / test_registry_ref_class were removed in #236.
RegistryRef and LogRef now come from auki-registry-py; their own tests live
in that package's test suite.
"""

from __future__ import annotations

import pathlib
import pytest

# ─── Constants ──────────────────────────────────────────────────────────────

PEER_ID = "galbot"
APP_ID = "galbot-ctrl"


# ─── Fixtures ───────────────────────────────────────────────────────────────


def make_peer_and_session(tmp_path: pathlib.Path):
    """Return one storage-backed Peer and a fresh Session."""
    import auki_session

    peer = auki_session.Peer(PEER_ID, APP_ID).with_storage_root(str(tmp_path))
    return peer, peer.start_session()


def register_optical_frame(peer, frame_id: str = "head_left_camera_optical"):
    """Register a ROS-optical frame and return the RegistryRef."""
    import auki_session

    return peer.register_frame(frame_id, auki_session.FrameDef.ros_optical())


def register_camera_sensor(peer, sensor_id: str = "head_left_rgb", frame_ref=None):
    """Register an RGB camera sensor and return the RegistryRef."""
    body = {
        "kind": "camera",
        "type": "rgb",
        "width": 1920,
        "height": 1200,
        "frame_rate_hz": 30,
        "image_encoding": "raw",
        "pixel_format": "rgb8",
        "row_stride_bytes": 5760,
        "color_space": "srgb",
        "intrinsics_model": "pinhole",
        "distortion_model": "brown_conrady",
        "frame": {
            "peer_id": frame_ref.peer_id,
            "id": frame_ref.id,
            "hash": frame_ref.hash,
        },
    }
    return peer.register_sensor(sensor_id, body)


def register_monotonic_clock(session, clock_id: str = "session/sdk_clock"):
    """Register a monotonic clock and return the RegistryRef."""
    body = {
        "type": "monotonic_clock",
        "unit": "ns",
        "monotonic": True,
        "scope": "device-local",
    }
    return session.register_clock(clock_id, body)


# ─── test_session_construction ──────────────────────────────────────────────


def test_session_construction(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer = auki_session.Peer(PEER_ID, APP_ID)
    assert peer.peer_id == PEER_ID
    assert peer.app_id == APP_ID
    assert peer.storage_root == "."
    peer.with_storage_root(str(tmp_path))
    s = peer.start_session()
    assert s.peer_id == PEER_ID
    assert s.app_id == APP_ID
    assert len(s.session_id) == 26  # ULID
    assert pathlib.Path(s.storage_root) == tmp_path


# ─── test_with_storage_root ─────────────────────────────────────────────────


def test_with_storage_root(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer = auki_session.Peer(PEER_ID, APP_ID).with_storage_root(str(tmp_path))
    s = peer.start_session()
    assert pathlib.Path(s.storage_root) == tmp_path


def test_with_storage_root_is_inherited_by_new_sessions(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer = auki_session.Peer(PEER_ID, APP_ID).with_storage_root(str(tmp_path))
    first = peer.start_session()
    second = peer.start_session()
    assert pathlib.Path(first.storage_root) == tmp_path
    assert pathlib.Path(second.storage_root) == tmp_path
    assert first.session_id != second.session_id


# ─── test_session_id_is_ulid ────────────────────────────────────────────────


def test_session_id_is_ulid(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer = auki_session.Peer(PEER_ID, APP_ID).with_storage_root(str(tmp_path))
    a = peer.start_session()
    b = peer.start_session()
    assert len(a.session_id) == 26
    assert len(b.session_id) == 26
    assert a.session_id != b.session_id


# ─── test_register_frame_returns_registry_ref ───────────────────────────────


def test_register_frame_returns_registry_ref(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer, _session = make_peer_and_session(tmp_path)
    ref = peer.register_frame("head_left_camera_optical", auki_session.FrameDef.ros_optical())

    assert isinstance(ref, auki_session.RegistryRef)
    assert ref.peer_id == PEER_ID
    assert ref.id == "head_left_camera_optical"
    assert len(ref.hash) == 32  # XXH3-128 → 32 hex chars


def test_register_frame_all_presets(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer, _session = make_peer_and_session(tmp_path)
    rb = peer.register_frame("a", auki_session.FrameDef.ros_body())
    ro = peer.register_frame("b", auki_session.FrameDef.ros_optical())
    gl = peer.register_frame("c", auki_session.FrameDef.opengl())
    u = peer.register_frame("d", auki_session.FrameDef.unity())
    # All four presets have distinct hashes
    hashes = {rb.hash, ro.hash, gl.hash, u.hash}
    assert len(hashes) == 4


# ─── test_register_clock_returns_registry_ref ───────────────────────────────


def test_register_clock_returns_registry_ref(tmp_path: pathlib.Path) -> None:
    import auki_session

    _peer, session = make_peer_and_session(tmp_path)
    clock_ref = register_monotonic_clock(session)

    assert isinstance(clock_ref, auki_session.RegistryRef)
    assert clock_ref.peer_id == PEER_ID
    assert clock_ref.id == "session/sdk_clock"
    assert clock_ref.hash  # non-empty


# ─── test_register_sensor_returns_registry_ref ──────────────────────────────


def test_register_sensor_returns_registry_ref(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer, _session = make_peer_and_session(tmp_path)
    frame_ref = register_optical_frame(peer)
    sensor_ref = register_camera_sensor(peer, frame_ref=frame_ref)

    assert isinstance(sensor_ref, auki_session.RegistryRef)
    assert sensor_ref.peer_id == PEER_ID
    assert sensor_ref.id == "head_left_rgb"
    assert sensor_ref.hash  # non-empty


# ─── test_register_sensor_log_end_to_end ─────────────────────────────────────


def test_register_sensor_log_end_to_end(tmp_path: pathlib.Path) -> None:
    """Full pipeline: frame + sensor + clock + sensor_log; verify handle and catalog."""
    import auki_session

    peer, session = make_peer_and_session(tmp_path)
    frame_ref = register_optical_frame(peer)
    sensor_ref = register_camera_sensor(peer, frame_ref=frame_ref)
    clock_ref = register_monotonic_clock(session)

    spec = auki_session.SensorLogSpec(
        sensor=sensor_ref,
        clock=clock_ref,
        frame=frame_ref,
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    handle = session.register_sensor_log(spec)

    assert isinstance(handle, auki_session.SensorLogHandle)
    assert handle.resource_id == "head_left_rgb"
    assert pathlib.Path(handle.root) == (
        tmp_path / session.session_id / "logs" / PEER_ID / "head_left_rgb"
    )

    lr = handle.log_ref
    assert isinstance(lr, auki_session.LogRef)
    assert lr.source_peer_id == PEER_ID
    assert lr.resource_id == "head_left_rgb"

    # manifest.json should be on disk
    manifest_path = pathlib.Path(handle.root) / "manifest.json"
    assert manifest_path.exists(), f"manifest.json missing at {manifest_path}"


# ─── test_register_duplicate_log_raises ──────────────────────────────────────


def test_register_duplicate_log_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer, session = make_peer_and_session(tmp_path)
    frame_ref = register_optical_frame(peer)
    sensor_ref = register_camera_sensor(peer, frame_ref=frame_ref)
    clock_ref = register_monotonic_clock(session)

    spec = auki_session.SensorLogSpec(
        sensor=sensor_ref,
        clock=clock_ref,
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    session.register_sensor_log(spec)

    with pytest.raises(ValueError, match="duplicate log"):
        session.register_sensor_log(spec)


# ─── test_register_sensor_bad_id_raises ─────────────────────────────────────


def test_register_sensor_bad_id_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    peer, _session = make_peer_and_session(tmp_path)
    frame_ref = register_optical_frame(peer)
    body = {
        "kind": "camera",
        "type": "rgb",
        "width": 1920,
        "height": 1200,
        "frame_rate_hz": 30,
        "image_encoding": "raw",
        "pixel_format": "rgb8",
        "row_stride_bytes": 5760,
        "color_space": "srgb",
        "intrinsics_model": "pinhole",
        "distortion_model": "brown_conrady",
        "frame": {"peer_id": frame_ref.peer_id, "id": frame_ref.id, "hash": frame_ref.hash},
    }
    with pytest.raises(ValueError):
        peer.register_sensor("bad>id", body)


# ─── test_materialize_remote_log_raises ─────────────────────────────────────


def test_materialize_remote_log_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    _peer, session = make_peer_and_session(tmp_path)
    log_ref = {"source_peer_id": PEER_ID, "resource_id": "head_left_rgb"}

    with pytest.raises(NotImplementedError, match="not implemented"):
        session.materialize_remote_log(
            log_ref,
            retention_ns=300_000_000_000,
            segment_duration_ns=10_000_000_000,
        )


def test_materialize_remote_log_raises_with_log_ref_object(tmp_path: pathlib.Path) -> None:
    """LogRef pyclass instance is also accepted."""
    import auki_session

    _peer, session = make_peer_and_session(tmp_path)
    log_ref = auki_session.LogRef(source_peer_id=PEER_ID, resource_id="head_left_rgb")

    with pytest.raises(NotImplementedError):
        session.materialize_remote_log(
            log_ref,
            retention_ns=300_000_000_000,
            segment_duration_ns=10_000_000_000,
        )


# ─── test_resolve_static_transform_raises ───────────────────────────────────


def test_resolve_static_transform_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    _peer, session = make_peer_and_session(tmp_path)
    log_ref = {"source_peer_id": "park", "resource_id": "world->base_link"}

    with pytest.raises(NotImplementedError, match="not implemented"):
        session.resolve_static_transform(log_ref)


# ─── test_frame_def_classmethods ─────────────────────────────────────────────


def test_frame_def_classmethods() -> None:
    import auki_session

    rb = auki_session.FrameDef.ros_body()
    ro = auki_session.FrameDef.ros_optical()
    gl = auki_session.FrameDef.opengl()
    u = auki_session.FrameDef.unity()
    assert "ros_body" in repr(rb)
    assert "ros_optical" in repr(ro)
    assert "opengl" in repr(gl)
    assert "unity" in repr(u)


# ─── test_head_spec_classmethods ─────────────────────────────────────────────


def test_head_spec_classmethods() -> None:
    import auki_session

    rolling = auki_session.HeadSpec.rolling(5_000_000_000)
    fixed = auki_session.HeadSpec.fixed()
    assert "rolling" in repr(rolling)
    assert "5000000000" in repr(rolling)
    assert "fixed" in repr(fixed)


# ─── Additional log type tests ───────────────────────────────────────────────


def test_register_pose_log_resource_id(tmp_path: pathlib.Path) -> None:
    """Pose log resource_id is `{from_frame.id}->{to_frame.id}`."""
    import auki_session

    peer, session = make_peer_and_session(tmp_path)
    world = peer.register_frame("world", auki_session.FrameDef.ros_body())
    base_link = peer.register_frame("base_link", auki_session.FrameDef.ros_body())
    clock = register_monotonic_clock(session)

    spec = auki_session.PoseLogSpec(
        from_frame=world,
        to_frame=base_link,
        clock=clock,
        source={"kind": "manual"},
        writer_mode="movable",
        expected_rate_hz=30,
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    handle = session.register_pose_log(spec)

    assert isinstance(handle, auki_session.PoseLogHandle)
    assert handle.resource_id == "world->base_link"
    assert handle.log_ref.source_peer_id == PEER_ID


def test_register_time_transform_log_resource_id(tmp_path: pathlib.Path) -> None:
    """TimeTransform log resource_id is `{from_clock.id}->{to_clock.id}`."""
    import auki_session

    _peer, session = make_peer_and_session(tmp_path)
    sdk_clock = register_monotonic_clock(session)
    wall_clock = session.register_clock("wall_clock", {
        "type": "utc_clock",
        "unit": "ns",
        "monotonic": False,
        "scope": "global",
        "epoch": "1970-01-01T00:00:00Z",
    })

    spec = auki_session.TimeTransformLogSpec(
        from_clock=sdk_clock,
        to_clock=wall_clock,
        source={"kind": "local_clock_read"},
        head=auki_session.HeadSpec.rolling(60_000_000_000),
        segment_duration_ns=60_000_000_000,
        retention_ns=3_600_000_000_000,
    )
    handle = session.register_time_transform_log(spec)

    assert isinstance(handle, auki_session.TimeTransformLogHandle)
    assert handle.resource_id == "session/sdk_clock->wall_clock"


def test_register_detection_log_resource_id(tmp_path: pathlib.Path) -> None:
    """Detection log resource_id is `{detector.id}@{input_sensor.id}`."""
    import auki_session

    peer, session = make_peer_and_session(tmp_path)
    frame_ref = register_optical_frame(peer)
    sensor_ref = register_camera_sensor(peer, frame_ref=frame_ref)
    clock_ref = register_monotonic_clock(session)

    detector_ref = peer.register_detector(
        "yolo_v8",
        {"type": "object_detection", "model": "yolo_v8n"},
        ["bounding_box"],
    )
    input_log_ref = auki_session.LogRef(
        source_peer_id=PEER_ID,
        resource_id="head_left_rgb",
    )

    spec = auki_session.DetectionLogSpec(
        instance_id="yolo-head-left",
        detector=detector_ref,
        input_log=input_log_ref,
        input_sensor=sensor_ref,
        clock=clock_ref,
        cadence={"kind": "every_frame"},
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    # We need to register the sensor_log first to match the full chain expectation
    # but the session only needs the sensor registered for detection log.
    handle = session.register_detection_log(spec)

    assert isinstance(handle, auki_session.DetectionLogHandle)
    assert handle.resource_id == "yolo-head-left"

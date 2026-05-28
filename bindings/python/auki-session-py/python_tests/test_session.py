"""Smoke tests for the `auki_session` Python module.

Run after building the wheel:

    maturin develop -m bindings/python/auki-session-py/Cargo.toml
    pytest bindings/python/auki-session-py/python_tests/

These tests verify the Python binding surface for the declarative Session API
introduced in #216 / card #223.

Test inventory
--------------
- test_session_construction                    — Session(peer_id, app_id), accessors
- test_with_storage_root                       — builder pattern, storage_root accessor
- test_session_id_is_ulid                      — 26-char ULID, unique per instance
- test_register_frame_returns_registry_ref     — register_frame + RegistryRef shape
- test_register_clock_returns_registry_ref     — register_clock + RegistryRef shape
- test_register_sensor_returns_registry_ref    — register_sensor + RegistryRef shape
- test_register_sensor_log_end_to_end          — full log registration + catalog check
- test_catalog_resource_id_and_shape           — catalog() returns correct dict shape
- test_register_duplicate_log_raises           — duplicate registration raises ValueError
- test_register_sensor_bad_id_raises           — invalid sensor_id raises ValueError
- test_materialize_remote_log_raises           — NotImplementedError with Phase-5 message
- test_resolve_static_transform_raises         — NotImplementedError with Phase-5 message
- test_join_domain_raises                      — NotImplementedError (requires libp2p swarm)
- test_leave_domain_raises                     — NotImplementedError (requires libp2p swarm)
- test_frame_def_classmethods                  — FrameDef.ros_body/optical/opengl/unity repr
- test_head_spec_classmethods                  — HeadSpec.rolling/fixed repr
- test_log_ref_class                           — LogRef pyclass accessors
- test_registry_ref_class                      — RegistryRef pyclass accessors
"""

from __future__ import annotations

import pathlib
import pytest

# ─── Constants ──────────────────────────────────────────────────────────────

PEER_ID = "galbot"
APP_ID = "galbot-ctrl"


# ─── Fixtures ───────────────────────────────────────────────────────────────


def make_session(tmp_path: pathlib.Path):
    """Return a Session with storage root at tmp_path."""
    import auki_session

    return auki_session.Session(PEER_ID, APP_ID).with_storage_root(str(tmp_path))


def register_optical_frame(session, frame_id: str = "head_left_camera_optical"):
    """Register a ROS-optical frame and return the RegistryRef."""
    import auki_session

    return session.register_frame(frame_id, auki_session.FrameDef.ros_optical())


def register_camera_sensor(session, sensor_id: str = "head_left_rgb", frame_ref=None):
    """Register an RGB camera sensor and return the RegistryRef."""
    body = {
        "kind": "camera",
        "type": "rgb",
        "width": 1920,
        "height": 1200,
        "frame_rate_hz": 30,
        "pixel_format": "rgb8",
        "color_space": "srgb",
        "intrinsics_model": "pinhole",
        "distortion_model": "brown_conrady",
        "frame": {
            "peer_id": frame_ref.peer_id,
            "id": frame_ref.id,
            "hash": frame_ref.hash,
        },
    }
    return session.register_sensor(sensor_id, body)


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


def test_session_construction() -> None:
    import auki_session

    s = auki_session.Session(PEER_ID, APP_ID)
    assert s.peer_id == PEER_ID
    assert s.app_id == APP_ID
    assert len(s.session_id) == 26  # ULID
    # Default storage root is "."
    assert s.storage_root == "."


# ─── test_with_storage_root ─────────────────────────────────────────────────


def test_with_storage_root(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = auki_session.Session(PEER_ID, APP_ID).with_storage_root(str(tmp_path))
    assert pathlib.Path(s.storage_root) == tmp_path


def test_with_storage_root_preserves_session_id(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = auki_session.Session(PEER_ID, APP_ID)
    original_session_id = s.session_id
    s.with_storage_root(str(tmp_path))
    # session_id is stable across the call — with_storage_root mutates
    # the underlying SessionInner in place via Rust's set_storage_root.
    assert s.session_id == original_session_id
    assert pathlib.Path(s.storage_root) == tmp_path


# ─── test_session_id_is_ulid ────────────────────────────────────────────────


def test_session_id_is_ulid() -> None:
    import auki_session

    a = auki_session.Session(PEER_ID, APP_ID)
    b = auki_session.Session(PEER_ID, APP_ID)
    assert len(a.session_id) == 26
    assert len(b.session_id) == 26
    assert a.session_id != b.session_id


# ─── test_register_frame_returns_registry_ref ───────────────────────────────


def test_register_frame_returns_registry_ref(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    ref = s.register_frame("head_left_camera_optical", auki_session.FrameDef.ros_optical())

    assert isinstance(ref, auki_session.RegistryRef)
    assert ref.peer_id == PEER_ID
    assert ref.id == "head_left_camera_optical"
    assert len(ref.hash) == 32  # XXH3-128 → 32 hex chars


def test_register_frame_all_presets(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    rb = s.register_frame("a", auki_session.FrameDef.ros_body())
    ro = s.register_frame("b", auki_session.FrameDef.ros_optical())
    gl = s.register_frame("c", auki_session.FrameDef.opengl())
    u = s.register_frame("d", auki_session.FrameDef.unity())
    # All four presets have distinct hashes
    hashes = {rb.hash, ro.hash, gl.hash, u.hash}
    assert len(hashes) == 4


# ─── test_register_clock_returns_registry_ref ───────────────────────────────


def test_register_clock_returns_registry_ref(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    clock_ref = register_monotonic_clock(s)

    assert isinstance(clock_ref, auki_session.RegistryRef)
    assert clock_ref.peer_id == PEER_ID
    assert clock_ref.id == "session/sdk_clock"
    assert clock_ref.hash  # non-empty


# ─── test_register_sensor_returns_registry_ref ──────────────────────────────


def test_register_sensor_returns_registry_ref(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    frame_ref = register_optical_frame(s)
    sensor_ref = register_camera_sensor(s, frame_ref=frame_ref)

    assert isinstance(sensor_ref, auki_session.RegistryRef)
    assert sensor_ref.peer_id == PEER_ID
    assert sensor_ref.id == "head_left_rgb"
    assert sensor_ref.hash  # non-empty


# ─── test_register_sensor_log_end_to_end ─────────────────────────────────────


def test_register_sensor_log_end_to_end(tmp_path: pathlib.Path) -> None:
    """Full pipeline: frame + sensor + clock + sensor_log; verify handle and catalog."""
    import auki_session

    s = make_session(tmp_path)
    frame_ref = register_optical_frame(s)
    sensor_ref = register_camera_sensor(s, frame_ref=frame_ref)
    clock_ref = register_monotonic_clock(s)

    spec = auki_session.SensorLogSpec(
        sensor=sensor_ref,
        clock=clock_ref,
        frame=frame_ref,
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    handle = s.register_sensor_log(spec)

    assert isinstance(handle, auki_session.SensorLogHandle)
    assert handle.resource_id == "head_left_rgb"

    lr = handle.log_ref
    assert isinstance(lr, auki_session.LogRef)
    assert lr.source_peer_id == PEER_ID
    assert lr.resource_id == "head_left_rgb"

    # manifest.json should be on disk
    manifest_path = tmp_path / "logs" / PEER_ID / "head_left_rgb" / "manifest.json"
    assert manifest_path.exists(), f"manifest.json missing at {manifest_path}"


# ─── test_catalog_resource_id_and_shape ─────────────────────────────────────


def test_catalog_resource_id_and_shape(tmp_path: pathlib.Path) -> None:
    """catalog() returns a list[dict] with canonical ResourceEntry shape."""
    import auki_session

    s = make_session(tmp_path)
    frame_ref = register_optical_frame(s)
    sensor_ref = register_camera_sensor(s, frame_ref=frame_ref)
    clock_ref = register_monotonic_clock(s)

    spec = auki_session.SensorLogSpec(
        sensor=sensor_ref,
        clock=clock_ref,
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    s.register_sensor_log(spec)

    rows = s.catalog()
    assert isinstance(rows, list)
    assert len(rows) == 1

    row = rows[0]
    assert isinstance(row, dict)
    assert row["source_peer_id"] == PEER_ID
    assert row["writer_peer_id"] == PEER_ID
    assert row["resource_id"] == "head_left_rgb"
    assert row["state"] == "live"
    # Head block present (rolling)
    assert row["head"] is not None
    # Sensor block present with kind=camera, type=rgb
    assert row["sensor"]["kind"] == "camera"
    assert row["sensor"]["type"] == "rgb"
    # variant tag present (ResourceEntry.variant_content is flattened; the tag key is "variant")
    assert row.get("variant") is not None


# ─── test_register_duplicate_log_raises ──────────────────────────────────────


def test_register_duplicate_log_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    frame_ref = register_optical_frame(s)
    sensor_ref = register_camera_sensor(s, frame_ref=frame_ref)
    clock_ref = register_monotonic_clock(s)

    spec = auki_session.SensorLogSpec(
        sensor=sensor_ref,
        clock=clock_ref,
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    s.register_sensor_log(spec)

    with pytest.raises(ValueError, match="duplicate log"):
        s.register_sensor_log(spec)


# ─── test_register_sensor_bad_id_raises ─────────────────────────────────────


def test_register_sensor_bad_id_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    frame_ref = register_optical_frame(s)
    body = {
        "kind": "camera",
        "type": "rgb",
        "width": 1920,
        "height": 1200,
        "frame_rate_hz": 30,
        "pixel_format": "rgb8",
        "color_space": "srgb",
        "intrinsics_model": "pinhole",
        "distortion_model": "brown_conrady",
        "frame": {"peer_id": frame_ref.peer_id, "id": frame_ref.id, "hash": frame_ref.hash},
    }
    with pytest.raises(ValueError):
        s.register_sensor("bad>id", body)


# ─── test_materialize_remote_log_raises ─────────────────────────────────────


def test_materialize_remote_log_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    log_ref = {"source_peer_id": PEER_ID, "resource_id": "head_left_rgb"}

    with pytest.raises(NotImplementedError, match="not implemented"):
        s.materialize_remote_log(log_ref, retention_ns=300_000_000_000, segment_duration_ns=10_000_000_000)


def test_materialize_remote_log_raises_with_log_ref_object(tmp_path: pathlib.Path) -> None:
    """LogRef pyclass instance is also accepted."""
    import auki_session

    s = make_session(tmp_path)
    log_ref = auki_session.LogRef(source_peer_id=PEER_ID, resource_id="head_left_rgb")

    with pytest.raises(NotImplementedError):
        s.materialize_remote_log(log_ref, retention_ns=300_000_000_000, segment_duration_ns=10_000_000_000)


# ─── test_resolve_static_transform_raises ───────────────────────────────────


def test_resolve_static_transform_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    log_ref = {"source_peer_id": "park", "resource_id": "world->base_link"}

    with pytest.raises(NotImplementedError, match="not implemented"):
        s.resolve_static_transform(log_ref)


# ─── test_join_domain_raises ─────────────────────────────────────────────────


def test_join_domain_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    with pytest.raises(NotImplementedError, match="libp2p"):
        s.join_domain({})


# ─── test_leave_domain_raises ────────────────────────────────────────────────


def test_leave_domain_raises(tmp_path: pathlib.Path) -> None:
    import auki_session

    s = make_session(tmp_path)
    with pytest.raises(NotImplementedError, match="libp2p"):
        s.leave_domain()


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


# ─── test_log_ref_class ──────────────────────────────────────────────────────


def test_log_ref_class() -> None:
    import auki_session

    lr = auki_session.LogRef(source_peer_id=PEER_ID, resource_id="head_left_rgb")
    assert lr.source_peer_id == PEER_ID
    assert lr.resource_id == "head_left_rgb"
    assert "LogRef" in repr(lr)


# ─── test_registry_ref_class ─────────────────────────────────────────────────


def test_registry_ref_class() -> None:
    import auki_session

    ref = auki_session.RegistryRef(peer_id=PEER_ID, id="head_left_rgb", hash="abc123")
    assert ref.peer_id == PEER_ID
    assert ref.id == "head_left_rgb"
    assert ref.hash == "abc123"
    assert "RegistryRef" in repr(ref)


# ─── Additional log type tests ───────────────────────────────────────────────


def test_register_pose_log_resource_id(tmp_path: pathlib.Path) -> None:
    """Pose log resource_id is `{from_frame.id}->{to_frame.id}`."""
    import auki_session

    s = make_session(tmp_path)
    world = s.register_frame("world", auki_session.FrameDef.ros_body())
    base_link = s.register_frame("base_link", auki_session.FrameDef.ros_body())
    clock = register_monotonic_clock(s)

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
    handle = s.register_pose_log(spec)

    assert isinstance(handle, auki_session.PoseLogHandle)
    assert handle.resource_id == "world->base_link"
    assert handle.log_ref.source_peer_id == PEER_ID


def test_register_time_transform_log_resource_id(tmp_path: pathlib.Path) -> None:
    """TimeTransform log resource_id is `{from_clock.id}->{to_clock.id}`."""
    import auki_session

    s = make_session(tmp_path)
    sdk_clock = register_monotonic_clock(s)
    wall_clock = s.register_clock("wall_clock", {
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
    handle = s.register_time_transform_log(spec)

    assert isinstance(handle, auki_session.TimeTransformLogHandle)
    assert handle.resource_id == "session/sdk_clock->wall_clock"


def test_register_detection_log_resource_id(tmp_path: pathlib.Path) -> None:
    """Detection log resource_id is `{detector.id}@{input_sensor.id}`."""
    import auki_session

    s = make_session(tmp_path)
    frame_ref = register_optical_frame(s)
    sensor_ref = register_camera_sensor(s, frame_ref=frame_ref)
    clock_ref = register_monotonic_clock(s)

    detector_ref = s.register_detector(
        "yolo_v8",
        {"type": "object_detection", "model": "yolo_v8n"},
        ["bounding_box"],
    )
    input_log_ref = auki_session.LogRef(
        source_peer_id=PEER_ID,
        resource_id="head_left_rgb",
    )

    spec = auki_session.DetectionLogSpec(
        detector=detector_ref,
        input_log=input_log_ref,
        input_sensor=sensor_ref,
        clock=clock_ref,
        head=auki_session.HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    # We need to register the sensor_log first to match the full chain expectation
    # but the session only needs the sensor registered for detection log.
    handle = s.register_detection_log(spec)

    assert isinstance(handle, auki_session.DetectionLogHandle)
    assert handle.resource_id == "yolo_v8@head_left_rgb"

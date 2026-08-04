"""Smoke tests for the `auki_registry` Python module.

Run after building the wheel:

    maturin develop -m bindings/python/auki-registry-py/Cargo.toml
    pytest bindings/python/auki-registry-py/python_tests/

Tests reflect the #216 restructure:
- All entry constructors take ``peer_id`` as first positional argument.
- Frame references are nested ``{"peer_id": ..., "id": ..., "hash": ...}``
  objects (was flat ``frame_id`` + ``frame_hash``).
- ``SensorBody`` discriminator is now ``"kind"`` (was ``"type"``).
- Each body has an open-string ``"type"`` field alongside ``"kind"``.
- New sensor variants: Rangefinder (renamed from PointCloud), Rf.
- ``RegistryRef`` and ``LogRef`` Python classes exposed.
"""

from __future__ import annotations

import json
import pathlib

import pytest

PEER_ID = "galbot"
FRAME_ID = "K1-AABBCCDDEEFF/head_left_cam_optical"


# ─── RegistryRef / LogRef classes ────────────────────────────────────────────


def test_registry_ref_class() -> None:
    import auki_registry

    r = auki_registry.RegistryRef(peer_id=PEER_ID, id=FRAME_ID, hash="abc123")
    assert r.peer_id == PEER_ID
    assert r.id == FRAME_ID
    assert r.hash == "abc123"
    assert "RegistryRef" in repr(r)


def test_log_ref_class() -> None:
    import auki_registry

    lr = auki_registry.LogRef(source_peer_id=PEER_ID, resource_id="head_left_rgb")
    assert lr.source_peer_id == PEER_ID
    assert lr.resource_id == "head_left_rgb"
    assert "LogRef" in repr(lr)


# ─── Frame presets carry peer_id ─────────────────────────────────────────────


def test_frame_ros_optical_carries_peer_id(tmp_path: pathlib.Path) -> None:
    import auki_registry

    frame = auki_registry.frame_ros_optical(PEER_ID, FRAME_ID)
    assert frame["peer_id"] == PEER_ID
    assert frame["frame_id"] == FRAME_ID
    assert frame["handedness"] == "right"
    assert frame["axes"] == {"x": "right", "y": "down", "z": "forward"}
    assert frame["units"] == "meters"


def test_frame_ros_body_carries_peer_id() -> None:
    import auki_registry

    frame = auki_registry.frame_ros_body(PEER_ID, "base_link")
    assert frame["peer_id"] == PEER_ID
    assert frame["frame_id"] == "base_link"
    assert frame["axes"] == {"x": "forward", "y": "left", "z": "up"}


def test_frame_write_read_round_trip(tmp_path: pathlib.Path) -> None:
    import auki_registry

    frame = auki_registry.frame_ros_optical(PEER_ID, FRAME_ID)
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    assert frame_hash == auki_registry.hash_frame(frame)
    # read_frame now takes (app_root, peer_id, frame_id, hash)
    read = auki_registry.read_frame(tmp_path, PEER_ID, FRAME_ID, frame_hash)
    assert read == frame
    assert auki_registry.read_frame(tmp_path, PEER_ID, FRAME_ID, "missing") is None


# ─── Sensor entry constructors ────────────────────────────────────────────────


def test_camera_sensor_entry_kind_and_type(tmp_path: pathlib.Path) -> None:
    """Camera body now uses kind='camera' and has an open-string 'type' field."""
    import auki_registry

    frame = auki_registry.frame_ros_optical(PEER_ID, FRAME_ID)
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    frame_ref = {"peer_id": PEER_ID, "id": FRAME_ID, "hash": frame_hash}

    sensor = auki_registry.camera_sensor_entry(
        peer_id=PEER_ID,
        sensor_id="K1-AABBCCDDEEFF/head_left_cam",
        sensor_type="rgb",
        width=544,
        height=488,
        frame_rate_hz=20,
        image_encoding="raw",
        pixel_format="YUV_NV12",
        row_stride_bytes=544,
        color_space="BT.709",
        intrinsics_model="pinhole",
        distortion_model="plumb_bob",
        frame=frame_ref,
    )

    assert sensor["kind"] == "camera"
    assert sensor["type"] == "rgb"
    assert sensor["peer_id"] == PEER_ID
    assert sensor["frame"]["peer_id"] == PEER_ID
    assert sensor["frame"]["id"] == FRAME_ID
    assert sensor["frame"]["hash"] == frame_hash

    sensor_hash = auki_registry.write_sensor(tmp_path, sensor)
    read = auki_registry.read_sensor(tmp_path, PEER_ID, "K1-AABBCCDDEEFF/head_left_cam", sensor_hash)
    assert read == sensor


def test_camera_sensor_entry_with_registry_ref_object(tmp_path: pathlib.Path) -> None:
    """RegistryRef pyclass instance accepted wherever frame dict is expected."""
    import auki_registry

    frame = auki_registry.frame_ros_optical(PEER_ID, FRAME_ID)
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    frame_ref = auki_registry.RegistryRef(peer_id=PEER_ID, id=FRAME_ID, hash=frame_hash)

    sensor = auki_registry.camera_sensor_entry(
        peer_id=PEER_ID,
        sensor_id="K1-AABBCCDDEEFF/head_left_cam",
        sensor_type="depth",
        width=640,
        height=480,
        frame_rate_hz=30,
        image_encoding="raw",
        pixel_format="Z16",
        row_stride_bytes=1280,
        color_space="linear",
        intrinsics_model="pinhole",
        distortion_model="brown_conrady",
        frame=frame_ref,
    )
    assert sensor["kind"] == "camera"
    assert sensor["type"] == "depth"
    assert sensor["frame"]["hash"] == frame_hash


def test_rangefinder_sensor_entry(tmp_path: pathlib.Path) -> None:
    """Rangefinder (renamed from PointCloud) has kind='rangefinder' + type field."""
    import auki_registry

    frame = auki_registry.frame_ros_optical(PEER_ID, FRAME_ID)
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    frame_ref = {"peer_id": PEER_ID, "id": FRAME_ID, "hash": frame_hash}

    sensor = auki_registry.rangefinder_sensor_entry(
        peer_id=PEER_ID,
        sensor_id="K1-AABBCCDDEEFF/head_depth_points",
        sensor_type="point_cloud",
        fields=[
            auki_registry.point_field("x", 0, "float32"),
            auki_registry.point_field("y", 4, "float32"),
            auki_registry.point_field("z", 8, "float32"),
        ],
        point_step=12,
        is_bigendian=False,
        frame_rate_hz=10,
        frame=frame_ref,
    )

    assert sensor["kind"] == "rangefinder"
    assert sensor["type"] == "point_cloud"
    assert sensor["peer_id"] == PEER_ID
    assert sensor["frame"]["hash"] == frame_hash

    sensor_hash = auki_registry.write_sensor(tmp_path, sensor)
    read = auki_registry.read_sensor(tmp_path, PEER_ID, "K1-AABBCCDDEEFF/head_depth_points", sensor_hash)
    assert read == sensor


def test_rangefinder_sensor_rejects_missing_frame(tmp_path: pathlib.Path) -> None:
    import auki_registry

    frame_ref = {"peer_id": PEER_ID, "id": FRAME_ID, "hash": "missing"}
    sensor = auki_registry.rangefinder_sensor_entry(
        peer_id=PEER_ID,
        sensor_id="K1-AABBCCDDEEFF/head_depth_points",
        sensor_type="point_cloud",
        fields=[auki_registry.point_field("x", 0, "float32")],
        point_step=4,
        is_bigendian=False,
        frame_rate_hz=10,
        frame=frame_ref,
    )
    with pytest.raises(ValueError, match="references missing frame"):
        auki_registry.write_sensor(tmp_path, sensor)


def test_audio_sensor_entry(tmp_path: pathlib.Path) -> None:
    """Audio body now has kind='audio', open-string type, and frame ref."""
    import auki_registry

    frame = auki_registry.frame_ros_body(PEER_ID, "base_link")
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    frame_ref = {"peer_id": PEER_ID, "id": "base_link", "hash": frame_hash}

    sensor = auki_registry.audio_sensor_entry(
        peer_id=PEER_ID,
        sensor_id="K1-AABBCCDDEEFF/head_array_4mic",
        sensor_type="pcm",
        sample_rate_hz=48_000,
        channels=4,
        sample_format="pcm_s16le",
        channel_layout="n_channel",
        frame=frame_ref,
    )

    assert sensor["kind"] == "audio"
    assert sensor["type"] == "pcm"
    assert sensor["peer_id"] == PEER_ID
    assert sensor["frame"]["hash"] == frame_hash


def test_joint_encoders_sensor_entry(tmp_path: pathlib.Path) -> None:
    """JointEncoders body now has frame ref and open-string type."""
    import auki_registry

    frame = auki_registry.frame_ros_body(PEER_ID, "base_link")
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    frame_ref = {"peer_id": PEER_ID, "id": "base_link", "hash": frame_hash}

    sensor = auki_registry.joint_encoders_sensor_entry(
        peer_id=PEER_ID,
        sensor_id="K1-AABBCCDDEEFF/right_arm_joints",
        sensor_type="absolute",
        joint_count=6,
        frame_rate_hz=100,
        frame=frame_ref,
    )

    assert sensor["kind"] == "joint_encoders"
    assert sensor["type"] == "absolute"
    assert sensor["frame"]["hash"] == frame_hash


def test_rf_sensor_entry(tmp_path: pathlib.Path) -> None:
    """New Rf variant: kind='rf' with open-string type and frame ref."""
    import auki_registry

    frame = auki_registry.frame_ros_body(PEER_ID, "base_link")
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    frame_ref = {"peer_id": PEER_ID, "id": "base_link", "hash": frame_hash}

    sensor = auki_registry.rf_sensor_entry(
        peer_id=PEER_ID,
        sensor_id="K1-AABBCCDDEEFF/ble_beacon",
        sensor_type="bluetooth",
        frame=frame_ref,
    )

    assert sensor["kind"] == "rf"
    assert sensor["type"] == "bluetooth"
    assert sensor["peer_id"] == PEER_ID
    assert sensor["frame"]["hash"] == frame_hash


# ─── Clock entries carry peer_id ──────────────────────────────────────────────


def test_clock_write_read_round_trip(tmp_path: pathlib.Path) -> None:
    import auki_registry

    clock = auki_registry.monotonic_clock_entry(
        peer_id=PEER_ID,
        clock_id="K1-AABBCCDDEEFF/monotonic",
        unit="milliseconds",
        scope="device-local",
    )
    assert clock["peer_id"] == PEER_ID

    clock_hash = auki_registry.write_clock(tmp_path, clock)
    assert auki_registry.hash_clock(clock) == clock_hash
    # read_clock now takes (app_root, peer_id, clock_id, hash)
    read = auki_registry.read_clock(tmp_path, PEER_ID, "K1-AABBCCDDEEFF/monotonic", clock_hash)
    assert read == clock


def test_utc_clock_entry_carries_peer_id() -> None:
    import auki_registry

    clock = auki_registry.utc_clock_entry(
        peer_id=PEER_ID,
        clock_id="K1-AABBCCDDEEFF/utc",
        unit="milliseconds",
        scope="global",
        epoch="1970-01-01T00:00:00Z",
    )
    assert clock["peer_id"] == PEER_ID
    assert clock["type"] == "utc_clock"


# ─── Validation helpers ───────────────────────────────────────────────────────


def test_validate_sensor_id_accepts_valid() -> None:
    import auki_registry

    auki_registry.validate_sensor_id("head_left_rgb")
    auki_registry.validate_sensor_id("K1-AABBCCDDEEFF/head_left_cam")


def test_validate_sensor_id_rejects_disallowed_chars() -> None:
    import auki_registry

    with pytest.raises(ValueError):
        auki_registry.validate_sensor_id("bad>id")
    with pytest.raises(ValueError):
        auki_registry.validate_sensor_id("bad@id")
    with pytest.raises(ValueError):
        auki_registry.validate_sensor_id("")


def test_invalid_enum_string_raises_value_error() -> None:
    import auki_registry

    with pytest.raises(ValueError, match="handedness"):
        auki_registry.frame_entry(
            peer_id=PEER_ID,
            frame_id="frame",
            handedness="sideways",
            x="right",
            y="down",
            z="forward",
            units="meters",
        )


# ─── Cross-language parity: Python construction == locked Rust fixture bytes ──
#
# Each test constructs the same value as the on-disk locked fixture via the
# Python API, then compares the canonical JSON bytes produced by
# canonical_json_* against the fixture content.  This proves the Python
# binding generates byte-identical JCS canonical JSON to the Rust side.
#
# Fixtures live at:
#   crates/auki-registry/tests/locked/<name>.json

_FIXTURES_ROOT = pathlib.Path(__file__).parent.parent.parent.parent.parent / "crates" / "auki-registry" / "tests" / "locked"


def _load_fixture(name: str) -> str:
    """Load a locked fixture and return its content stripped of trailing whitespace."""
    return (_FIXTURES_ROOT / name).read_text(encoding="utf-8").rstrip()


def test_parity_frame_ros_optical() -> None:
    """Python frame_ros_optical + canonical_json_frame must match locked fixture."""
    import auki_registry

    fixture = _load_fixture("frame_ros_optical.json")
    fixture_data = json.loads(fixture)
    # Fixture peer_id is "galbot", frame_id is "head_left_camera_optical"
    frame = auki_registry.frame_ros_optical(
        fixture_data["peer_id"],
        fixture_data["frame_id"],
    )
    canonical = auki_registry.canonical_json_frame(frame)
    assert canonical == fixture, f"\nExpected: {fixture}\n  Actual: {canonical}"


def test_parity_clock_monotonic() -> None:
    """Python monotonic_clock_entry + canonical_json_clock must match locked fixture."""
    import auki_registry

    fixture = _load_fixture("clock_monotonic.json")
    fixture_data = json.loads(fixture)
    clock = auki_registry.monotonic_clock_entry(
        peer_id=fixture_data["peer_id"],
        clock_id=fixture_data["clock_id"],
        unit=fixture_data["unit"],
        scope=fixture_data["scope"],
        epoch=fixture_data.get("epoch"),
    )
    canonical = auki_registry.canonical_json_clock(clock)
    assert canonical == fixture, f"\nExpected: {fixture}\n  Actual: {canonical}"


def test_parity_clock_utc() -> None:
    """Python utc_clock_entry + canonical_json_clock must match locked fixture."""
    import auki_registry

    fixture = _load_fixture("clock_utc.json")
    fixture_data = json.loads(fixture)
    clock = auki_registry.utc_clock_entry(
        peer_id=fixture_data["peer_id"],
        clock_id=fixture_data["clock_id"],
        unit=fixture_data["unit"],
        scope=fixture_data["scope"],
        epoch=fixture_data["epoch"],
    )
    canonical = auki_registry.canonical_json_clock(clock)
    assert canonical == fixture, f"\nExpected: {fixture}\n  Actual: {canonical}"


def test_parity_sensor_camera_rgb() -> None:
    """Python camera_sensor_entry + canonical_json_sensor must match locked fixture."""
    import auki_registry

    fixture = _load_fixture("sensor_camera_rgb.json")
    fixture_data = json.loads(fixture)
    sensor = auki_registry.camera_sensor_entry(
        peer_id=fixture_data["peer_id"],
        sensor_id=fixture_data["sensor_id"],
        sensor_type=fixture_data["type"],
        width=fixture_data["width"],
        height=fixture_data["height"],
        frame_rate_hz=fixture_data["frame_rate_hz"],
        image_encoding=fixture_data["image_encoding"],
        pixel_format=fixture_data["pixel_format"],
        row_stride_bytes=fixture_data["row_stride_bytes"],
        color_space=fixture_data["color_space"],
        intrinsics_model=fixture_data["intrinsics_model"],
        distortion_model=fixture_data["distortion_model"],
        frame=fixture_data["frame"],
    )
    canonical = auki_registry.canonical_json_sensor(sensor)
    assert canonical == fixture, f"\nExpected: {fixture}\n  Actual: {canonical}"


def test_parity_sensor_rangefinder_point_cloud() -> None:
    """Python rangefinder_sensor_entry + canonical_json_sensor must match locked fixture."""
    import auki_registry

    fixture = _load_fixture("sensor_rangefinder_point_cloud.json")
    fixture_data = json.loads(fixture)
    sensor = auki_registry.rangefinder_sensor_entry(
        peer_id=fixture_data["peer_id"],
        sensor_id=fixture_data["sensor_id"],
        sensor_type=fixture_data["type"],
        fields=fixture_data["fields"],
        point_step=fixture_data["point_step"],
        is_bigendian=fixture_data["is_bigendian"],
        frame_rate_hz=fixture_data["frame_rate_hz"],
        frame=fixture_data["frame"],
    )
    canonical = auki_registry.canonical_json_sensor(sensor)
    assert canonical == fixture, f"\nExpected: {fixture}\n  Actual: {canonical}"


def test_parity_sensor_audio_pcm() -> None:
    """Python audio_sensor_entry + canonical_json_sensor must match locked fixture."""
    import auki_registry

    fixture = _load_fixture("sensor_audio_pcm.json")
    fixture_data = json.loads(fixture)
    sensor = auki_registry.audio_sensor_entry(
        peer_id=fixture_data["peer_id"],
        sensor_id=fixture_data["sensor_id"],
        sensor_type=fixture_data["type"],
        sample_rate_hz=fixture_data["sample_rate_hz"],
        channels=fixture_data["channels"],
        sample_format=fixture_data["sample_format"],
        channel_layout=fixture_data["channel_layout"],
        frame=fixture_data["frame"],
    )
    canonical = auki_registry.canonical_json_sensor(sensor)
    assert canonical == fixture, f"\nExpected: {fixture}\n  Actual: {canonical}"

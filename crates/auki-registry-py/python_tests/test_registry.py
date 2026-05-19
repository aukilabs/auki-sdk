"""Smoke tests for the `auki_registry` Python module.

Run after building the wheel:

    maturin develop -m crates/auki-registry-py/Cargo.toml
    pytest crates/auki-registry-py/python_tests/
"""

from __future__ import annotations

import pathlib

import pytest


FRAME_ID = "K1-AABBCCDDEEFF/head_left_cam_optical"
FRAME_HASH = "e0d40e7b526e04f15f83f75897f53825"


def test_frame_write_read_round_trip(tmp_path: pathlib.Path) -> None:
    import auki_registry

    frame = auki_registry.frame_ros_optical(FRAME_ID)

    assert frame == {
        "frame_id": FRAME_ID,
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "meters",
    }
    assert auki_registry.hash_frame(frame) == FRAME_HASH
    assert auki_registry.write_frame(tmp_path, frame) == FRAME_HASH
    assert auki_registry.read_frame(tmp_path, FRAME_ID, FRAME_HASH) == frame
    assert auki_registry.read_frame(tmp_path, FRAME_ID, "missing") is None


def test_point_cloud_sensor_write_requires_frame(tmp_path: pathlib.Path) -> None:
    import auki_registry

    frame = auki_registry.frame_ros_optical(FRAME_ID)
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    sensor = auki_registry.point_cloud_sensor_entry(
        sensor_id="K1-AABBCCDDEEFF/head_depth_points",
        fields=[
            auki_registry.point_field("x", 0, "float32"),
            auki_registry.point_field("y", 4, "float32"),
            auki_registry.point_field("z", 8, "float32"),
        ],
        point_step=12,
        frame_rate_hz=10,
        frame_id=FRAME_ID,
        frame_hash=frame_hash,
    )

    sensor_hash = auki_registry.write_sensor(tmp_path, sensor)
    read = auki_registry.read_sensor(
        tmp_path,
        "K1-AABBCCDDEEFF/head_depth_points",
        sensor_hash,
    )

    assert read == sensor
    assert read["frame_hash"] == FRAME_HASH


def test_spatial_sensor_rejects_missing_frame(tmp_path: pathlib.Path) -> None:
    import auki_registry

    sensor = auki_registry.point_cloud_sensor_entry(
        sensor_id="K1-AABBCCDDEEFF/head_depth_points",
        fields=[
            auki_registry.point_field("x", 0, "float32"),
            auki_registry.point_field("y", 4, "float32"),
            auki_registry.point_field("z", 8, "float32"),
        ],
        point_step=12,
        frame_rate_hz=10,
        frame_id=FRAME_ID,
        frame_hash="missing",
    )

    with pytest.raises(ValueError, match="references missing frame"):
        auki_registry.write_sensor(tmp_path, sensor)


def test_point_cloud_sensor_rejects_non_xyz_layout(tmp_path: pathlib.Path) -> None:
    import auki_registry

    frame = auki_registry.frame_ros_optical(FRAME_ID)
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    sensor = auki_registry.point_cloud_sensor_entry(
        sensor_id="K1-AABBCCDDEEFF/head_depth_points",
        fields=[auki_registry.point_field("x", 0, "float32")],
        point_step=4,
        frame_rate_hz=10,
        frame_id=FRAME_ID,
        frame_hash=frame_hash,
    )

    with pytest.raises(ValueError, match="invalid pointcloud layout"):
        auki_registry.write_sensor(tmp_path, sensor)


def test_clock_write_read_round_trip(tmp_path: pathlib.Path) -> None:
    import auki_registry

    clock = auki_registry.monotonic_clock_entry(
        clock_id="K1-AABBCCDDEEFF/monotonic",
        unit="milliseconds",
        scope="device-local",
    )

    clock_hash = auki_registry.write_clock(tmp_path, clock)

    assert auki_registry.hash_clock(clock) == clock_hash
    assert auki_registry.read_clock(
        tmp_path,
        "K1-AABBCCDDEEFF/monotonic",
        clock_hash,
    ) == clock


def test_invalid_enum_string_raises_value_error() -> None:
    import auki_registry

    with pytest.raises(ValueError, match="handedness"):
        auki_registry.frame_entry(
            frame_id="frame",
            handedness="sideways",
            x="right",
            y="down",
            z="forward",
            units="meters",
        )

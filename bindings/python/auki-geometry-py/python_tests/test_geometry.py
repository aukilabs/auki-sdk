"""Smoke tests for the `auki_geometry` Python module.

Run after building the wheel:

    maturin develop -m bindings/python/auki-geometry-py/Cargo.toml
    pytest bindings/python/auki-geometry-py/python_tests/
"""

from __future__ import annotations

import pytest


def test_meters_per_unit_locked_values() -> None:
    import auki_geometry

    assert auki_geometry.meters_per_unit("meters") == 1.0
    assert auki_geometry.meters_per_unit("centimeters") == 0.01
    assert auki_geometry.meters_per_unit("millimeters") == 0.001


def test_meters_per_unit_rejects_unknown_unit() -> None:
    import auki_geometry

    with pytest.raises(ValueError):
        auki_geometry.meters_per_unit("furlongs")


def test_axis_convention_matrix_ros_optical_to_opengl() -> None:
    import auki_geometry

    ros_optical = {"x": "right", "y": "down", "z": "forward"}
    opengl = {"x": "right", "y": "up", "z": "backward"}
    assert auki_geometry.axis_convention_matrix(ros_optical, opengl) == [
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
    ]


def test_convention_matrix_round_trips_to_identity() -> None:
    import auki_geometry
    import auki_registry

    presets = [
        auki_registry.frame_ros_body("body"),
        auki_registry.frame_ros_optical("optical"),
        auki_registry.frame_opengl("opengl"),
        auki_registry.frame_unity("unity"),
    ]

    def matmul4(a: list[list[float]], b: list[list[float]]) -> list[list[float]]:
        return [
            [sum(a[r][k] * b[k][c] for k in range(4)) for c in range(4)]
            for r in range(4)
        ]

    identity4 = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]

    for a in presets:
        for b in presets:
            ab = auki_geometry.convention_matrix(a, b)
            ba = auki_geometry.convention_matrix(b, a)
            product = matmul4(ba, ab)
            for r in range(4):
                for c in range(4):
                    assert abs(product[r][c] - identity4[r][c]) < 1e-9


def test_convert_point_convention_applies_axes_and_units() -> None:
    import auki_geometry
    import auki_registry

    source = {
        "frame_id": "source",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "centimeters",
    }
    target = auki_registry.frame_opengl("target")
    converted = auki_geometry.convert_point_convention([100.0, 200.0, 300.0], source, target)
    assert converted == pytest.approx([1.0, -2.0, -3.0])


def test_convert_direction_convention_skips_unit_scale() -> None:
    import auki_geometry
    import auki_registry

    source = {
        "frame_id": "source",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "centimeters",
    }
    target = auki_registry.frame_opengl("target")
    converted = auki_geometry.convert_direction_convention([1.0, 2.0, 3.0], source, target)
    assert converted == pytest.approx([1.0, -2.0, -3.0])


def test_convert_pose_convention_translates_and_rotates() -> None:
    import math

    import auki_geometry
    import auki_registry

    half = 1.0 / math.sqrt(2)
    pose = [1.0, 2.0, 3.0, 0.0, 0.0, half, half]
    from_entry = auki_registry.frame_ros_optical("camera")
    to_entry = auki_registry.frame_opengl("world")

    converted = auki_geometry.convert_pose_convention(pose, from_entry, to_entry)

    # Translation: ROS-optical (x=right, y=down, z=forward) in meters →
    # OpenGL (x=right, y=up, z=backward) in meters. Same axis flips as
    # convert_point_convention without unit scale.
    assert converted[0] == pytest.approx(1.0)
    assert converted[1] == pytest.approx(-2.0)
    assert converted[2] == pytest.approx(-3.0)

    # Orientation should be a unit quaternion.
    qx, qy, qz, qw = converted[3:]
    assert abs(qx * qx + qy * qy + qz * qz + qw * qw - 1.0) < 1e-9


def test_convert_pose_convention_rejects_short_array() -> None:
    import auki_geometry
    import auki_registry

    pose = [1.0, 2.0, 3.0, 0.0, 0.0, 0.0]  # 6 elements, not 7
    with pytest.raises(ValueError, match="pose: expected 7 floats"):
        auki_geometry.convert_pose_convention(
            pose,
            auki_registry.frame_ros_optical("camera"),
            auki_registry.frame_opengl("world"),
        )

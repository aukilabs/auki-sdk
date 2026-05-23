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

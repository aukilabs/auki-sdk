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
        auki_registry.frame_ros_body("test-peer", "body"),
        auki_registry.frame_ros_optical("test-peer", "optical"),
        auki_registry.frame_opengl("test-peer", "opengl"),
        auki_registry.frame_unity("test-peer", "unity"),
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
        "peer_id": "test-peer",
        "frame_id": "source",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "centimeters",
    }
    target = auki_registry.frame_opengl("test-peer", "target")
    converted = auki_geometry.convert_point_convention([100.0, 200.0, 300.0], source, target)
    assert converted == pytest.approx([1.0, -2.0, -3.0])


def test_convert_vector_convention_applies_axes_and_units() -> None:
    import auki_geometry
    import auki_registry

    source = {
        "peer_id": "test-peer",
        "frame_id": "source",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "centimeters",
    }
    target = auki_registry.frame_opengl("test-peer", "target")
    # Same axis flip + unit scale as the point conversion; binding seam
    # is the same shape — both go through length_scaled_axis_matrix in Rust.
    converted = auki_geometry.convert_vector_convention([100.0, 200.0, 300.0], source, target)
    assert converted == pytest.approx([1.0, -2.0, -3.0])


def test_convert_direction_convention_skips_unit_scale() -> None:
    import auki_geometry
    import auki_registry

    source = {
        "peer_id": "test-peer",
        "frame_id": "source",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "centimeters",
    }
    target = auki_registry.frame_opengl("test-peer", "target")
    converted = auki_geometry.convert_direction_convention([1.0, 2.0, 3.0], source, target)
    assert converted == pytest.approx([1.0, -2.0, -3.0])


def test_convert_pose_convention_translates_and_rotates() -> None:
    import math

    import auki_geometry
    import auki_registry

    half = 1.0 / math.sqrt(2)
    pose = [1.0, 2.0, 3.0, 0.0, 0.0, half, half]
    from_entry = auki_registry.frame_ros_optical("test-peer", "camera")
    to_entry = auki_registry.frame_opengl("test-peer", "world")

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
            auki_registry.frame_ros_optical("test-peer", "camera"),
            auki_registry.frame_opengl("test-peer", "world"),
        )


def test_inverse_then_compose_yields_identity() -> None:
    import math

    import auki_geometry

    half = 1.0 / math.sqrt(2)
    pose = [1.0, 2.0, 3.0, 0.0, 0.0, half, half]

    inverse = auki_geometry.inverse_spatial_transform(pose)
    composed = auki_geometry.compose_spatial_transforms(pose, inverse)

    # Identity: zero translation, identity quaternion (sign may flip).
    for i in range(3):
        assert composed[i] == pytest.approx(0.0, abs=1e-9)
    qx, qy, qz, qw = composed[3:]
    # Identity quaternion is (0, 0, 0, ±1).
    assert abs(qx) < 1e-9 and abs(qy) < 1e-9 and abs(qz) < 1e-9
    assert abs(abs(qw) - 1.0) < 1e-9


def test_compose_spatial_transforms_order() -> None:
    """Compose (A → B) with (B → C); apply to origin → translation of A → C."""
    import math

    import auki_geometry

    half = 1.0 / math.sqrt(2)
    # A → B: rotate 90° around +Z, no translation.
    a_to_b = [0.0, 0.0, 0.0, 0.0, 0.0, half, half]
    # B → C: translate +1 along x (post-rotation), no rotation.
    b_to_c = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]

    a_to_c = auki_geometry.compose_spatial_transforms(a_to_b, b_to_c)

    # Origin of A is (0, 0, 0). Apply A → B (rotation only): origin stays.
    # Apply B → C: now translation (1, 0, 0) in C. So composed translation = (1, 0, 0).
    assert a_to_c[0] == pytest.approx(1.0)
    assert a_to_c[1] == pytest.approx(0.0)
    assert a_to_c[2] == pytest.approx(0.0)


def test_relative_spatial_transform_derives_from_to() -> None:
    """Given common→A and common→B, derive A→B."""
    import auki_geometry

    # common → A: pure translation along +x.
    common_to_a = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    # common → B: pure translation along +y.
    common_to_b = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]

    # A → B = inverse(common → A) ∘ (common → B) = translate (-1, 1, 0).
    a_to_b = auki_geometry.relative_spatial_transform(common_to_a, common_to_b)
    assert a_to_b[0] == pytest.approx(-1.0)
    assert a_to_b[1] == pytest.approx(1.0)
    assert a_to_b[2] == pytest.approx(0.0)


def test_inverse_spatial_transform_rejects_zero_quaternion() -> None:
    import auki_geometry

    # Zero quaternion is invalid; Rust's normalize_quat surfaces ZeroQuaternion.
    pose = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    with pytest.raises(auki_geometry.GeometryError):
        auki_geometry.inverse_spatial_transform(pose)


def test_geometry_error_is_value_error_subclass() -> None:
    import auki_geometry

    assert issubclass(auki_geometry.GeometryError, ValueError)

    # Catchable as ValueError too.
    with pytest.raises(ValueError):
        auki_geometry.inverse_spatial_transform([0.0] * 7)


def test_spatial_transform_to_matrix4_identity() -> None:
    import auki_geometry

    identity = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    matrix = auki_geometry.spatial_transform_to_matrix4(identity)
    assert matrix == [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]


def test_spatial_transform_matrix4_round_trip() -> None:
    import math

    import auki_geometry

    half = 1.0 / math.sqrt(2)
    poses = [
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, half, 0.0, 0.0, half],
        [0.0, 0.0, 0.0, 0.0, half, 0.0, half],
        [1.0, 2.0, 3.0, 0.0, 0.0, half, half],
    ]
    for original in poses:
        matrix = auki_geometry.spatial_transform_to_matrix4(original)
        decoded = auki_geometry.spatial_transform_from_matrix4(matrix)
        # Translation round-trips exactly.
        for i in range(3):
            assert decoded[i] == pytest.approx(original[i])
        # Quaternion can equal ±original_quaternion (Hamilton sign).
        same = all(abs(decoded[3 + i] - original[3 + i]) < 1e-9 for i in range(4))
        negated = all(abs(decoded[3 + i] + original[3 + i]) < 1e-9 for i in range(4))
        assert same or negated, f"pose {original} did not round-trip: {decoded}"


def test_spatial_transform_from_matrix4_rejects_wrong_shape() -> None:
    import auki_geometry

    # 3x3 instead of 4x4.
    with pytest.raises(ValueError, match="matrix: expected 4 rows"):
        auki_geometry.spatial_transform_from_matrix4([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ])

    # 4 rows but inner row only 3 wide.
    with pytest.raises(ValueError, match="matrix\\[0\\]: expected 4 floats"):
        auki_geometry.spatial_transform_from_matrix4([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])


def test_convention_matrix_raises_geometry_error_on_handedness_mismatch() -> None:
    import auki_geometry

    # Right-handed declaration but axes have determinant -1 (left-handed
    # in fact — unity preset shape). Rust's validate_frame rejects this
    # as GeometryError::HandednessMismatch.
    inconsistent = {
        "peer_id": "test-peer",
        "frame_id": "inconsistent",
        "handedness": "right",
        "axes": {"x": "right", "y": "up", "z": "forward"},
        "units": "meters",
    }
    target = {
        "peer_id": "test-peer",
        "frame_id": "target",
        "handedness": "right",
        "axes": {"x": "right", "y": "up", "z": "backward"},
        "units": "meters",
    }
    with pytest.raises(auki_geometry.GeometryError):
        auki_geometry.convention_matrix(inconsistent, target)


def test_axis_convention_matrix_raises_geometry_error_on_invalid_axes() -> None:
    import auki_geometry

    # Duplicate axis direction — x and y both "right". Rust's validate_axes
    # rejects this as GeometryError::InvalidAxes.
    duplicate = {"x": "right", "y": "right", "z": "forward"}
    valid = {"x": "right", "y": "up", "z": "backward"}
    with pytest.raises(auki_geometry.GeometryError):
        auki_geometry.axis_convention_matrix(duplicate, valid)

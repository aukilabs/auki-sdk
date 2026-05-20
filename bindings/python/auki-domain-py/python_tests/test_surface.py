"""Smoke tests for the current `auki_domain` Python module.

Run after building the wheel:

    maturin develop -m bindings/python/auki-domain-py/Cargo.toml
    pytest bindings/python/auki-domain-py/python_tests/
"""

from __future__ import annotations

import pathlib

import pytest


def test_module_imports_current_surface() -> None:
    import auki_domain

    assert hasattr(auki_domain, "ClusterMember")
    assert hasattr(auki_domain, "ClusterMembership")
    assert hasattr(auki_domain, "DaemonInfo")
    assert hasattr(auki_domain, "ParticipantInfo")
    assert hasattr(auki_domain, "SensorEntry")
    assert hasattr(auki_domain, "ResourcePinholeIntrinsics")
    assert hasattr(auki_domain, "ResourceVec3")
    assert hasattr(auki_domain, "ResourceQuat")
    assert hasattr(auki_domain, "ResourceSpatialTransform")
    assert hasattr(auki_domain, "SensorStreamResource")
    assert hasattr(auki_domain, "TransformEdgeResource")
    assert hasattr(auki_domain, "StreamManifestBuilder")
    assert hasattr(auki_domain, "ClusterTarget")
    assert hasattr(auki_domain, "ClusterManager")


def test_cluster_target_factories() -> None:
    import auki_domain

    target = auki_domain.ClusterTarget.most_recent_or_create("hagall")

    assert target.kind == "most_recent_or_create"
    assert target.name == "hagall"
    assert "hagall" in repr(target)


def test_stream_manifest_builder_missing_sensor_is_file_not_found(
    tmp_path: pathlib.Path,
) -> None:
    import auki_domain

    with pytest.raises(FileNotFoundError, match="sensor registry entry missing"):
        auki_domain.StreamManifestBuilder.from_registry(
            tmp_path,
            "missing/sensor",
            "missing-hash",
            "clock",
            "clock-hash",
        )


def test_sensor_entry_value_type() -> None:
    import auki_domain

    entry = auki_domain.SensorEntry(
        "K1-AABBCCDDEEFF/head_depth_points",
        "sensor-hash",
        "point_cloud",
    )

    assert entry.sensor_id == "K1-AABBCCDDEEFF/head_depth_points"
    assert entry.sensor_hash == "sensor-hash"
    assert entry.kind == "point_cloud"
    assert entry == auki_domain.SensorEntry(
        "K1-AABBCCDDEEFF/head_depth_points",
        "sensor-hash",
        "point_cloud",
    )


def test_resource_value_types() -> None:
    import auki_domain

    intrinsics = auki_domain.ResourcePinholeIntrinsics(
        fx=400.0,
        fy=401.0,
        cx=272.5,
        cy=244.5,
    )
    stream = auki_domain.SensorStreamResource(
        id="K1-LIVE01/head_left_cam",
        sensor_id="K1-LIVE01/head_left_cam",
        sensor_hash="sensor-hash",
        sensor_kind="rgb_camera",
        stream_protocol="/auki/stream/0.1.0",
        payload="pinhole_camera_log_entry",
        pinhole_intrinsics=intrinsics,
    )

    assert stream.kind == "sensor_stream"
    assert stream.pinhole_intrinsics == intrinsics

    transform = auki_domain.ResourceSpatialTransform(
        translation=auki_domain.ResourceVec3(0.0, 0.0, 0.0),
        orientation=auki_domain.ResourceQuat(0.5, -0.5, 0.5, -0.5),
    )
    edge = auki_domain.TransformEdgeResource(
        id="K1-LIVE01/camera_link->K1-LIVE01/head_left_cam_optical",
        from_frame_id="K1-LIVE01/camera_link",
        from_frame_hash="from-hash",
        to_frame_id="K1-LIVE01/head_left_cam_optical",
        to_frame_hash="to-hash",
        writer_mode="rigid",
        source_json='{"kind":"ros2_tf"}',
        transform=transform,
    )

    assert edge.kind == "transform_edge"
    assert edge.transform == transform
    assert edge.source_json == '{"kind":"ros2_tf"}'

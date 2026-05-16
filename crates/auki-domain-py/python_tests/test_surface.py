"""Smoke tests for the current `auki_domain` Python module.

Run after building the wheel:

    maturin develop -m crates/auki-domain-py/Cargo.toml
    pytest crates/auki-domain-py/python_tests/
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

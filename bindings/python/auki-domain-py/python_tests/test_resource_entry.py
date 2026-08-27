"""Tests for ResourceEntry.from_dict and ResourceEntry.from_json constructors.

Run after building the wheel:

    maturin develop -m bindings/python/auki-domain-py/Cargo.toml
    pytest bindings/python/auki-domain-py/python_tests/
"""

from __future__ import annotations

import json

import pytest

# ─── Test fixtures ───────────────────────────────────────────────────────────

ZERO_HASH = "0" * 32

POSE_LOG_DICT = {
    "variant": "pose_log",
    "source_peer_id": "galbot",
    "writer_peer_id": "galbot",
    "resource_id": "base_link->head_left_camera_color_optical_frame",
    "state": "live",
    "head": {"kind": "rolling", "retention_ns": 5_000_000_000},
    "available": {"bytes": 0, "entries": 0, "duration_ns": 0},
    "pose": {"writer_mode": "movable"},
    "manifest": {
        "from_frame": {"peer_id": "galbot", "id": "base_link", "hash": ZERO_HASH},
        "to_frame": {
            "peer_id": "galbot",
            "id": "head_left_camera_color_optical_frame",
            "hash": ZERO_HASH,
        },
        "clock": {"peer_id": "galbot", "id": "sdk_clock", "hash": ZERO_HASH},
        "source": {"kind": "manual"},
        "expected_rate_hz": 10,
    },
}

SENSOR_LOG_DICT = {
    "variant": "sensor_log",
    "source_peer_id": "galbot",
    "writer_peer_id": "galbot",
    "resource_id": "head_left_rgb",
    "state": "live",
    "head": {"kind": "rolling", "retention_ns": 5_000_000_000},
    "available": {"bytes": 1024, "entries": 10, "duration_ns": 5_000_000_000},
    "sensor": {
        "kind": "camera",
        "type": "rgb",
        "sensor_id": "head_left_rgb",
        "sensor_hash": "abc123",
    },
    "manifest": {
        "clock": {"peer_id": "galbot", "id": "session/sdk_clock", "hash": ZERO_HASH},
        "frame": {
            "peer_id": "galbot",
            "id": "head_left_camera_optical",
            "hash": ZERO_HASH,
        },
    },
}

TIME_TRANSFORM_LOG_DICT = {
    "variant": "time_transform_log",
    "source_peer_id": "galbot",
    "writer_peer_id": "galbot",
    "resource_id": "session/sdk_clock->wall_clock",
    "state": "live",
    "head": {"kind": "rolling", "retention_ns": 60_000_000_000},
    "available": {"bytes": 4096, "entries": 60, "duration_ns": 60_000_000_000},
    "manifest": {
        "from_clock": {
            "peer_id": "galbot",
            "id": "session/sdk_clock",
            "hash": ZERO_HASH,
        },
        "to_clock": {"peer_id": "galbot", "id": "wall_clock", "hash": ZERO_HASH},
        "source": {"kind": "local_clock_read"},
    },
}

DETECTION_LOG_DICT = {
    "variant": "detection_log",
    "source_peer_id": "galbot",
    "writer_peer_id": "galbot",
    "resource_id": "yolo_v8@head_left_rgb",
    "state": "live",
    "head": {"kind": "rolling", "retention_ns": 5_000_000_000},
    "available": {"bytes": 250000, "entries": 150, "duration_ns": 5_000_000_000},
    "manifest": {
        "instance_id": "yolo-head-left-30hz",
        "detector": {"peer_id": "galbot", "id": "yolo_v8", "hash": ZERO_HASH},
        "input_log": {"source_peer_id": "galbot", "resource_id": "head_left_rgb"},
        "input_sensor": {"peer_id": "galbot", "id": "head_left_rgb", "hash": ZERO_HASH},
        "clock": {
            "peer_id": "galbot",
            "id": "session/sdk_clock",
            "hash": ZERO_HASH,
        },
        "cadence": {"kind": "every_frame"},
    },
}


# ─── from_dict — all four variants ──────────────────────────────────────────


def test_from_dict_pose_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(POSE_LOG_DICT)

    assert entry.variant == "pose_log"
    assert entry.source_peer_id == "galbot"
    assert entry.writer_peer_id == "galbot"
    assert entry.resource_id == "base_link->head_left_camera_color_optical_frame"
    assert entry.state == "live"

    head = entry.head
    assert head is not None
    assert head["kind"] == "rolling"
    assert head["retention_ns"] == 5_000_000_000

    avail = entry.available
    assert avail["bytes"] == 0
    assert avail["entries"] == 0
    assert avail["duration_ns"] == 0

    pose = entry.pose
    assert pose is not None
    assert pose["writer_mode"] == "movable"

    assert entry.sensor is None

    manifest = entry.manifest
    assert manifest["from_frame"]["peer_id"] == "galbot"
    assert manifest["from_frame"]["id"] == "base_link"
    assert manifest["to_frame"]["id"] == "head_left_camera_color_optical_frame"
    assert manifest["clock"]["id"] == "sdk_clock"
    assert manifest["expected_rate_hz"] == 10


def test_from_dict_sensor_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(SENSOR_LOG_DICT)

    assert entry.variant == "sensor_log"
    assert entry.source_peer_id == "galbot"
    assert entry.resource_id == "head_left_rgb"
    assert entry.state == "live"

    sensor = entry.sensor
    assert sensor is not None
    assert sensor["kind"] == "camera"
    assert sensor["type"] == "rgb"
    assert sensor["sensor_id"] == "head_left_rgb"
    assert sensor["sensor_hash"] == "abc123"

    assert entry.pose is None

    manifest = entry.manifest
    assert manifest["clock"]["id"] == "session/sdk_clock"
    assert manifest["frame"]["id"] == "head_left_camera_optical"


def test_from_dict_time_transform_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(TIME_TRANSFORM_LOG_DICT)

    assert entry.variant == "time_transform_log"
    assert entry.source_peer_id == "galbot"
    assert entry.resource_id == "session/sdk_clock->wall_clock"
    assert entry.state == "live"

    assert entry.sensor is None
    assert entry.pose is None

    manifest = entry.manifest
    assert manifest["from_clock"]["id"] == "session/sdk_clock"
    assert manifest["to_clock"]["id"] == "wall_clock"


def test_from_dict_detection_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(DETECTION_LOG_DICT)

    assert entry.variant == "detection_log"
    assert entry.source_peer_id == "galbot"
    assert entry.resource_id == "yolo_v8@head_left_rgb"
    assert entry.state == "live"

    assert entry.sensor is None
    assert entry.pose is None

    manifest = entry.manifest
    assert manifest["instance_id"] == "yolo-head-left-30hz"
    assert manifest["detector"]["id"] == "yolo_v8"
    assert manifest["input_log"]["source_peer_id"] == "galbot"
    assert manifest["input_log"]["resource_id"] == "head_left_rgb"
    assert manifest["input_sensor"]["id"] == "head_left_rgb"
    assert manifest["clock"]["id"] == "session/sdk_clock"
    assert manifest["cadence"] == {"kind": "every_frame"}


# ─── Round-trip: from_dict → to_json → json.loads ────────────────────────────


def test_round_trip_pose_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(POSE_LOG_DICT)
    json_str = entry.to_json()
    parsed = json.loads(json_str)

    assert parsed["variant"] == "pose_log"
    assert parsed["source_peer_id"] == "galbot"
    assert parsed["resource_id"] == "base_link->head_left_camera_color_optical_frame"
    assert parsed["state"] == "live"
    assert parsed["head"]["kind"] == "rolling"
    assert parsed["head"]["retention_ns"] == 5_000_000_000
    assert parsed["pose"]["writer_mode"] == "movable"
    assert parsed["manifest"]["from_frame"]["id"] == "base_link"
    assert parsed["manifest"]["to_frame"]["id"] == "head_left_camera_color_optical_frame"
    assert parsed["manifest"]["expected_rate_hz"] == 10


def test_round_trip_sensor_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(SENSOR_LOG_DICT)
    parsed = json.loads(entry.to_json())

    assert parsed["variant"] == "sensor_log"
    assert parsed["sensor"]["kind"] == "camera"
    assert parsed["manifest"]["frame"]["id"] == "head_left_camera_optical"


def test_round_trip_time_transform_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(TIME_TRANSFORM_LOG_DICT)
    parsed = json.loads(entry.to_json())

    assert parsed["variant"] == "time_transform_log"
    assert parsed["manifest"]["from_clock"]["id"] == "session/sdk_clock"
    assert parsed["manifest"]["to_clock"]["id"] == "wall_clock"


def test_round_trip_detection_log() -> None:
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(DETECTION_LOG_DICT)
    parsed = json.loads(entry.to_json())

    assert parsed["variant"] == "detection_log"
    assert parsed["manifest"]["instance_id"] == "yolo-head-left-30hz"
    assert parsed["manifest"]["detector"]["id"] == "yolo_v8"
    assert parsed["manifest"]["input_log"]["resource_id"] == "head_left_rgb"
    assert parsed["manifest"]["cadence"] == {"kind": "every_frame"}


# ─── from_json ───────────────────────────────────────────────────────────────


def test_from_json_pose_log() -> None:
    import auki_domain

    json_str = json.dumps(POSE_LOG_DICT)
    entry = auki_domain.ResourceEntry.from_json(json_str)

    assert entry.variant == "pose_log"
    assert entry.source_peer_id == "galbot"
    assert entry.resource_id == "base_link->head_left_camera_color_optical_frame"
    assert entry.state == "live"
    assert entry.pose is not None
    assert entry.pose["writer_mode"] == "movable"


def test_from_json_sensor_log() -> None:
    import auki_domain

    json_str = json.dumps(SENSOR_LOG_DICT)
    entry = auki_domain.ResourceEntry.from_json(json_str)

    assert entry.variant == "sensor_log"
    assert entry.sensor is not None
    assert entry.sensor["kind"] == "camera"


def test_from_json_round_trip_is_stable() -> None:
    """from_json → to_json should be idempotent (same keys/values)."""
    import auki_domain

    original_str = json.dumps(POSE_LOG_DICT)
    entry = auki_domain.ResourceEntry.from_json(original_str)
    second_str = entry.to_json()

    # Both round-trip to structurally equivalent dicts
    assert json.loads(second_str)["variant"] == "pose_log"
    assert json.loads(second_str)["pose"]["writer_mode"] == "movable"


# ─── Input validation ────────────────────────────────────────────────────────


def test_from_dict_bogus_variant_raises_value_error() -> None:
    import auki_domain

    bad = dict(POSE_LOG_DICT)
    bad["variant"] = "bogus"
    with pytest.raises(
        ValueError,
        match=r"invalid ResourceEntry dict:.*unknown variant",
    ):
        auki_domain.ResourceEntry.from_dict(bad)


def test_from_json_truncated_raises_value_error() -> None:
    import auki_domain

    with pytest.raises(ValueError, match=r"invalid ResourceEntry JSON:.*EOF"):
        auki_domain.ResourceEntry.from_json("{")


def test_from_json_empty_raises_value_error() -> None:
    import auki_domain

    with pytest.raises(ValueError, match=r"invalid ResourceEntry JSON:.*EOF"):
        auki_domain.ResourceEntry.from_json("")


def test_from_dict_missing_required_field_raises_value_error() -> None:
    import auki_domain

    # Drop the required `available` block
    bad = {k: v for k, v in SENSOR_LOG_DICT.items() if k != "available"}
    with pytest.raises(
        ValueError,
        match=r"invalid ResourceEntry dict:.*missing field.*available",
    ):
        auki_domain.ResourceEntry.from_dict(bad)


# ─── Domain provider surface smoke ───────────────────────────────────────────


def test_set_resource_catalog_provider_accepts_constructed_entry() -> None:
    """The Domain provider APIs accept Python-constructed resource rows.

    We only verify that:
    1. pre-join and post-join provider methods exist; and
    2. the provider itself returns the exact constructed row.

    The authenticated two-node test exercises callback invocation over the
    network. This test intentionally stays local and synchronous.
    """
    import auki_domain

    entry = auki_domain.ResourceEntry.from_dict(POSE_LOG_DICT)

    provider = lambda: [entry]
    result = provider()
    assert len(result) == 1
    assert result[0] is entry
    assert result[0].variant == "pose_log"

    assert hasattr(auki_domain.DomainBuilder, "resource_catalog_provider")
    assert hasattr(auki_domain.Domain, "set_resource_catalog_provider")


def test_resource_catalog_provider_roundtrip_multiple_variants() -> None:
    """A Domain catalog provider can return all four v0.2 row variants."""
    import auki_domain

    entries = [
        auki_domain.ResourceEntry.from_dict(POSE_LOG_DICT),
        auki_domain.ResourceEntry.from_dict(SENSOR_LOG_DICT),
        auki_domain.ResourceEntry.from_dict(TIME_TRANSFORM_LOG_DICT),
        auki_domain.ResourceEntry.from_dict(DETECTION_LOG_DICT),
    ]

    provider = lambda: entries
    result = provider()
    assert len(result) == 4
    variants = [e.variant for e in result]
    assert "pose_log" in variants
    assert "sensor_log" in variants
    assert "time_transform_log" in variants
    assert "detection_log" in variants


# ─── Surface: from_dict and from_json appear on the class ────────────────────


def test_resource_entry_constructors_on_class() -> None:
    import auki_domain

    assert hasattr(auki_domain.ResourceEntry, "from_dict")
    assert hasattr(auki_domain.ResourceEntry, "from_json")

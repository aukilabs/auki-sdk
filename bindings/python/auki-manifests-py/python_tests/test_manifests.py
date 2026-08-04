"""Python-side tests for ``auki_manifests``.

Run via::

    cd bindings/python/auki-manifests-py
    python -m venv .venv && source .venv/bin/activate
    pip install maturin pytest
    maturin develop --release
    pytest python_tests/

Tests reflect the #216 restructure:
- All builders take ``source_peer_id`` and ``writer_peer_id`` as the first
  two keyword arguments.
- Cross-registry references use nested ``RegistryRef`` dicts
  ``{"peer_id": ..., "id": ..., "hash": ...}`` rather than flat
  ``*_id`` + ``*_hash`` pairs.
- ``DetectionLogManifest.input_log`` uses ``LogRef``
  ``{"source_peer_id": ..., "resource_id": ...}``.
"""

from __future__ import annotations

import json
import pathlib

import pytest

import auki_manifests


# ─── Fixtures ────────────────────────────────────────────────────────────────

SOURCE_PEER_ID = "galbot"
WRITER_PEER_ID = "galbot"

SENSOR_REF = {
    "peer_id": "galbot",
    "id": "K1-AABBCCDDEEFF/head_left_cam",
    "hash": "e8cb3879fcfa7f716047aa0892b0c0c0",
}
CLOCK_REF = {
    "peer_id": "galbot",
    "id": "K1-AABBCCDDEEFF/utc",
    "hash": "89f84f4c2e09bef81d385b2af1d17e6c",
}
FRAME_REF = {
    "peer_id": "galbot",
    "id": "K1-AABBCCDDEEFF/head_left_cam_optical",
    "hash": "fd0dc3789e898b71b5e16ee122a81a44",
}


# ─── Sensor Log ──────────────────────────────────────────────────────────────


def test_build_sensor_log_manifest_contains_all_required_fields():
    m = auki_manifests.build_sensor_log_manifest(
        source_peer_id=SOURCE_PEER_ID,
        writer_peer_id=WRITER_PEER_ID,
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        sensor=SENSOR_REF,
        clock=CLOCK_REF,
        segment_duration_ns=1_000_000_000,
        retention_ns=30_000_000_000,
        frame=FRAME_REF,
    )
    assert m["source_peer_id"] == SOURCE_PEER_ID
    assert m["writer_peer_id"] == WRITER_PEER_ID
    assert m["app_id"] == "boosterapp"
    assert m["session_id"] == "550e8400-e29b-41d4-a716-446655440000"
    # Cross-refs are now nested objects
    assert m["sensor"]["peer_id"] == "galbot"
    assert m["sensor"]["id"] == "K1-AABBCCDDEEFF/head_left_cam"
    assert m["sensor"]["hash"] == "e8cb3879fcfa7f716047aa0892b0c0c0"
    assert m["clock"]["peer_id"] == "galbot"
    assert m["clock"]["id"] == "K1-AABBCCDDEEFF/utc"
    assert m["frame"]["peer_id"] == "galbot"
    assert m["frame"]["id"] == "K1-AABBCCDDEEFF/head_left_cam_optical"
    assert m["segment_duration_ns"] == 1_000_000_000
    assert m["retention_ns"] == 30_000_000_000


def test_build_sensor_log_manifest_frame_is_optional():
    m = auki_manifests.build_sensor_log_manifest(
        source_peer_id=SOURCE_PEER_ID,
        writer_peer_id=WRITER_PEER_ID,
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        sensor=SENSOR_REF,
        clock=CLOCK_REF,
        segment_duration_ns=1_000_000_000,
        retention_ns=30_000_000_000,
    )
    assert "frame" not in m


def test_build_sensor_log_manifest_source_writer_can_differ():
    """Park materializing Galbot's log: source stays galbot, writer changes."""
    m = auki_manifests.build_sensor_log_manifest(
        source_peer_id="galbot",
        writer_peer_id="park",
        app_id="park-vis",
        session_id="01HV-park-session",
        sensor=SENSOR_REF,
        clock=CLOCK_REF,
        segment_duration_ns=10_000_000_000,
        retention_ns=300_000_000_000,
    )
    assert m["source_peer_id"] == "galbot"
    assert m["writer_peer_id"] == "park"
    assert m["app_id"] == "park-vis"


# ─── Pose Log ────────────────────────────────────────────────────────────────


def test_build_pose_log_manifest_with_ros2_tf_source():
    from_frame = {
        "peer_id": "galbot",
        "id": "K1-AABBCCDDEEFF/base_link",
        "hash": "fd0dc3789e898b71b5e16ee122a81a44",
    }
    to_frame = {
        "peer_id": "galbot",
        "id": "K1-AABBCCDDEEFF/head_left_cam_optical",
        "hash": "11223344556677889900aabbccddeeff",
    }
    m = auki_manifests.build_pose_log_manifest(
        source_peer_id=SOURCE_PEER_ID,
        writer_peer_id=WRITER_PEER_ID,
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        from_frame=from_frame,
        to_frame=to_frame,
        clock=CLOCK_REF,
        source={
            "kind": "ros2_tf",
            "publishers": ["amcl", "robot_state_publisher", "tf_broadcaster"],
        },
        writer_mode="movable",
        expected_rate_hz=100,
        segment_duration_ns=1_000_000_000,
        retention_ns=30_000_000_000,
    )
    assert m["source_peer_id"] == SOURCE_PEER_ID
    assert m["writer_peer_id"] == WRITER_PEER_ID
    assert m["from_frame"]["peer_id"] == "galbot"
    assert m["from_frame"]["id"] == "K1-AABBCCDDEEFF/base_link"
    assert m["to_frame"]["peer_id"] == "galbot"
    assert m["to_frame"]["id"] == "K1-AABBCCDDEEFF/head_left_cam_optical"
    assert m["clock"]["peer_id"] == "galbot"
    assert m["source"]["kind"] == "ros2_tf"
    assert m["source"]["publishers"] == [
        "amcl",
        "robot_state_publisher",
        "tf_broadcaster",
    ]
    assert m["writer_mode"] == "movable"
    assert m["expected_rate_hz"] == 100


def test_build_pose_log_manifest_rejects_unknown_writer_mode():
    with pytest.raises(ValueError, match="writer_mode"):
        auki_manifests.build_pose_log_manifest(
            source_peer_id="x",
            writer_peer_id="x",
            app_id="x",
            session_id="y",
            from_frame={"peer_id": "p", "id": "a", "hash": "b"},
            to_frame={"peer_id": "p", "id": "c", "hash": "d"},
            clock={"peer_id": "p", "id": "e", "hash": "f"},
            source={"kind": "ros2_tf", "publishers": []},
            writer_mode="nonsense",
            expected_rate_hz=0,
            segment_duration_ns=1,
            retention_ns=0,
        )


# ─── TimeTransform Log ────────────────────────────────────────────────────────


def test_build_time_transform_log_manifest_with_local_clock_read_source():
    from_clock = {
        "peer_id": "galbot",
        "id": "K1-AABBCCDDEEFF/monotonic",
        "hash": "deadbeefcafefeed",
    }
    to_clock = {
        "peer_id": "galbot",
        "id": "K1-AABBCCDDEEFF/utc",
        "hash": "1234567890abcdef",
    }
    m = auki_manifests.build_time_transform_log_manifest(
        source_peer_id=SOURCE_PEER_ID,
        writer_peer_id=WRITER_PEER_ID,
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        from_clock=from_clock,
        to_clock=to_clock,
        source={"kind": "local_clock_read"},
        segment_duration_ns=1_000_000_000,
        retention_ns=60_000_000_000,
    )
    assert m["source_peer_id"] == SOURCE_PEER_ID
    assert m["writer_peer_id"] == WRITER_PEER_ID
    assert m["from_clock"]["peer_id"] == "galbot"
    assert m["from_clock"]["id"] == "K1-AABBCCDDEEFF/monotonic"
    assert m["to_clock"]["peer_id"] == "galbot"
    assert m["to_clock"]["id"] == "K1-AABBCCDDEEFF/utc"
    assert m["source"]["kind"] == "local_clock_read"
    assert m["segment_duration_ns"] == 1_000_000_000


# ─── Detection Log ────────────────────────────────────────────────────────────


def test_build_detection_log_manifest_contains_all_required_fields():
    detector = {
        "peer_id": "galbot",
        "id": "aukilabs/qr/v1",
        "hash": "abc123def4567890abc123def4567890",
    }
    input_log = {"source_peer_id": "galbot", "resource_id": "rec-456"}
    input_sensor = {
        "peer_id": "galbot",
        "id": "K1-AABBCCDDEEFF/head_left_cam",
        "hash": "e8cb3879fcfa7f716047aa0892b0c0c0",
    }
    m = auki_manifests.build_detection_log_manifest(
        source_peer_id=SOURCE_PEER_ID,
        writer_peer_id=WRITER_PEER_ID,
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        instance_id="qr-head-left-1hz",
        detector=detector,
        input_log=input_log,
        input_sensor=input_sensor,
        clock=CLOCK_REF,
        cadence={"kind": "periodic", "period_ns": 1_000_000_000},
        segment_duration_ns=1_000_000_000,
        retention_ns=30_000_000_000,
    )
    assert m["source_peer_id"] == SOURCE_PEER_ID
    assert m["writer_peer_id"] == WRITER_PEER_ID
    assert m["app_id"] == "boosterapp"
    assert m["instance_id"] == "qr-head-left-1hz"
    assert m["cadence"] == {"kind": "periodic", "period_ns": 1_000_000_000}
    assert m["detector"]["peer_id"] == "galbot"
    assert m["detector"]["id"] == "aukilabs/qr/v1"
    assert m["detector"]["hash"] == "abc123def4567890abc123def4567890"
    assert m["input_log"]["source_peer_id"] == "galbot"
    assert m["input_log"]["resource_id"] == "rec-456"
    assert m["input_sensor"]["peer_id"] == "galbot"
    assert m["input_sensor"]["id"] == "K1-AABBCCDDEEFF/head_left_cam"
    assert m["input_sensor"]["hash"] == "e8cb3879fcfa7f716047aa0892b0c0c0"
    assert m["clock"]["id"] == "K1-AABBCCDDEEFF/utc"
    assert m["segment_duration_ns"] == 1_000_000_000
    assert m["retention_ns"] == 30_000_000_000
    assert "intent" not in m  # match-the-existing-builders for v1


# ─── Cross-language parity: Python construction == locked Rust fixture bytes ──
#
# Each test constructs the same value as the on-disk locked fixture via the
# Python API, then compares the structure parsed from the produced dict against
# the fixture.  Since auki-manifests-py returns a Python dict (not raw bytes),
# we compare the key set and key values after a round-trip through json.dumps.
#
# The "identical canonical bytes" invariant is enforced on the Rust side by the
# auki-manifests locked_json test harness; here we confirm the Python API
# produces all the same fields and values.
#
# Fixtures live at:
#   crates/auki-manifests/tests/locked/<name>.json

_FIXTURES_ROOT = (
    pathlib.Path(__file__).parent.parent.parent.parent.parent
    / "crates"
    / "auki-manifests"
    / "tests"
    / "locked"
)


def _load_fixture(name: str) -> dict:
    content = (_FIXTURES_ROOT / name).read_text(encoding="utf-8").rstrip()
    return json.loads(content)


def _assert_dicts_match(produced: dict, expected: dict, *, name: str) -> None:
    """Recursively assert that produced matches expected (structural parity)."""
    produced_json = json.dumps(produced, sort_keys=True)
    expected_json = json.dumps(expected, sort_keys=True)
    assert produced_json == expected_json, (
        f"Parity mismatch for {name}:\n"
        f"  Expected: {expected_json}\n"
        f"   Produced: {produced_json}"
    )


def test_parity_sensor_log_origin() -> None:
    """Python build_sensor_log_manifest matches sensor_log_origin.json."""
    fixture = _load_fixture("sensor_log_origin.json")
    m = auki_manifests.build_sensor_log_manifest(
        source_peer_id=fixture["source_peer_id"],
        writer_peer_id=fixture["writer_peer_id"],
        app_id=fixture["app_id"],
        session_id=fixture["session_id"],
        sensor=fixture["sensor"],
        clock=fixture["clock"],
        segment_duration_ns=fixture["segment_duration_ns"],
        retention_ns=fixture["retention_ns"],
        frame=fixture.get("frame"),
    )
    _assert_dicts_match(m, fixture, name="sensor_log_origin.json")


def test_parity_sensor_log_materialized() -> None:
    """Python build_sensor_log_manifest matches sensor_log_materialized.json."""
    fixture = _load_fixture("sensor_log_materialized.json")
    m = auki_manifests.build_sensor_log_manifest(
        source_peer_id=fixture["source_peer_id"],
        writer_peer_id=fixture["writer_peer_id"],
        app_id=fixture["app_id"],
        session_id=fixture["session_id"],
        sensor=fixture["sensor"],
        clock=fixture["clock"],
        segment_duration_ns=fixture["segment_duration_ns"],
        retention_ns=fixture["retention_ns"],
        frame=fixture.get("frame"),
    )
    _assert_dicts_match(m, fixture, name="sensor_log_materialized.json")


def test_parity_pose_log_movable() -> None:
    """Python build_pose_log_manifest matches pose_log_movable.json."""
    fixture = _load_fixture("pose_log_movable.json")
    m = auki_manifests.build_pose_log_manifest(
        source_peer_id=fixture["source_peer_id"],
        writer_peer_id=fixture["writer_peer_id"],
        app_id=fixture["app_id"],
        session_id=fixture["session_id"],
        from_frame=fixture["from_frame"],
        to_frame=fixture["to_frame"],
        clock=fixture["clock"],
        source=fixture["source"],
        writer_mode=fixture["writer_mode"],
        expected_rate_hz=fixture["expected_rate_hz"],
        segment_duration_ns=fixture["segment_duration_ns"],
        retention_ns=fixture["retention_ns"],
    )
    _assert_dicts_match(m, fixture, name="pose_log_movable.json")


def test_parity_time_transform_log() -> None:
    """Python build_time_transform_log_manifest matches time_transform_log.json."""
    fixture = _load_fixture("time_transform_log.json")
    m = auki_manifests.build_time_transform_log_manifest(
        source_peer_id=fixture["source_peer_id"],
        writer_peer_id=fixture["writer_peer_id"],
        app_id=fixture["app_id"],
        session_id=fixture["session_id"],
        from_clock=fixture["from_clock"],
        to_clock=fixture["to_clock"],
        source=fixture["source"],
        segment_duration_ns=fixture["segment_duration_ns"],
        retention_ns=fixture["retention_ns"],
    )
    _assert_dicts_match(m, fixture, name="time_transform_log.json")


def test_parity_detection_log() -> None:
    """Python build_detection_log_manifest matches detection_log.json."""
    fixture = _load_fixture("detection_log.json")
    m = auki_manifests.build_detection_log_manifest(
        source_peer_id=fixture["source_peer_id"],
        writer_peer_id=fixture["writer_peer_id"],
        app_id=fixture["app_id"],
        session_id=fixture["session_id"],
        instance_id=fixture["instance_id"],
        detector=fixture["detector"],
        input_log=fixture["input_log"],
        input_sensor=fixture["input_sensor"],
        clock=fixture["clock"],
        cadence=fixture["cadence"],
        segment_duration_ns=fixture["segment_duration_ns"],
        retention_ns=fixture["retention_ns"],
    )
    _assert_dicts_match(m, fixture, name="detection_log.json")

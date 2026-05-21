"""Python-side tests for ``auki_manifests``.

Run via::

    cd bindings/python/auki-manifests-py
    python -m venv .venv && source .venv/bin/activate
    pip install maturin pytest
    maturin develop --release
    pytest python_tests/

Tests pin the manifest dict shapes per builder. Cross-language byte
equality with Rust-built manifests is preserved by JCS canonicalization
at write time (the wrappers go through `serde_json::Value` and the
Rust crate's existing JCS path).
"""

from __future__ import annotations

import auki_manifests


# ─── Detection Log (the ESL detector's case) ─────────────────────────────────


def test_build_detection_log_manifest_contains_all_required_fields():
    m = auki_manifests.build_detection_log_manifest(
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        detector_id="aukilabs/qr/v1",
        detector_hash="abc123def4567890abc123def4567890",
        input_log_id="rec-456",
        input_sensor_id="K1-AABBCCDDEEFF/head_left_cam",
        input_sensor_hash="e8cb3879fcfa7f716047aa0892b0c0c0",
        clock_id="K1-AABBCCDDEEFF/utc",
        clock_hash="89f84f4c2e09bef81d385b2af1d17e6c",
        segment_duration_ns=1_000_000_000,
        retention_ns=30_000_000_000,
    )
    assert m["app_id"] == "boosterapp"
    assert m["detector_id"] == "aukilabs/qr/v1"
    assert m["detector_hash"] == "abc123def4567890abc123def4567890"
    assert m["input_log_id"] == "rec-456"
    assert m["input_sensor_id"] == "K1-AABBCCDDEEFF/head_left_cam"
    assert m["input_sensor_hash"] == "e8cb3879fcfa7f716047aa0892b0c0c0"
    assert m["clock_id"] == "K1-AABBCCDDEEFF/utc"
    assert m["clock_hash"] == "89f84f4c2e09bef81d385b2af1d17e6c"
    assert m["segment_duration_ns"] == 1_000_000_000
    assert m["retention_ns"] == 30_000_000_000
    assert "intent" not in m  # match-the-existing-builders for v1


# ─── Sensor Log family ───────────────────────────────────────────────────────


def test_build_sensor_log_manifest_round_trip():
    m = auki_manifests.build_sensor_log_manifest(
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        sensor_id="K1-AABBCCDDEEFF/head_left_cam",
        sensor_hash="e8cb3879fcfa7f716047aa0892b0c0c0",
        clock_id="K1-AABBCCDDEEFF/utc",
        clock_hash="89f84f4c2e09bef81d385b2af1d17e6c",
        segment_duration_ns=1_000_000_000,
        retention_ns=30_000_000_000,
        frame_id="K1-AABBCCDDEEFF/head_left_cam_optical",
        frame_hash="fd0dc3789e898b71b5e16ee122a81a44",
    )
    assert m["sensor_id"] == "K1-AABBCCDDEEFF/head_left_cam"
    assert m["clock_id"] == "K1-AABBCCDDEEFF/utc"
    assert m["frame_id"] == "K1-AABBCCDDEEFF/head_left_cam_optical"
    assert m["frame_hash"] == "fd0dc3789e898b71b5e16ee122a81a44"


# ─── Pose Log + PoseSource (dict seam) ───────────────────────────────────────


def test_build_pose_log_manifest_with_ros2_tf_source():
    m = auki_manifests.build_pose_log_manifest(
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        from_frame_id="K1-AABBCCDDEEFF/base_link",
        from_frame_hash="fd0dc3789e898b71b5e16ee122a81a44",
        to_frame_id="K1-AABBCCDDEEFF/head_left_cam_optical",
        to_frame_hash="11223344556677889900aabbccddeeff",
        clock_id="K1-AABBCCDDEEFF/utc",
        clock_hash="89f84f4c2e09bef81d385b2af1d17e6c",
        source={
            "kind": "ros2_tf",
            "publishers": ["amcl", "robot_state_publisher", "tf_broadcaster"],
        },
        writer_mode="movable",
        expected_rate_hz=100,
        segment_duration_ns=1_000_000_000,
        retention_ns=30_000_000_000,
    )
    assert m["source"]["kind"] == "ros2_tf"
    assert m["source"]["publishers"] == [
        "amcl",
        "robot_state_publisher",
        "tf_broadcaster",
    ]
    assert m["writer_mode"] == "movable"
    assert m["expected_rate_hz"] == 100


def test_build_pose_log_manifest_rejects_unknown_writer_mode():
    import pytest

    with pytest.raises(ValueError, match="writer_mode"):
        auki_manifests.build_pose_log_manifest(
            app_id="x", session_id="y",
            from_frame_id="a", from_frame_hash="b",
            to_frame_id="c", to_frame_hash="d",
            clock_id="e", clock_hash="f",
            source={"kind": "ros2_tf", "publishers": []},
            writer_mode="nonsense",
            expected_rate_hz=0,
            segment_duration_ns=1, retention_ns=0,
        )


# ─── TimeTransform Log + TimeTransformSource (dict seam) ─────────────────────


def test_build_time_transform_log_manifest_with_local_clock_read_source():
    m = auki_manifests.build_time_transform_log_manifest(
        app_id="boosterapp",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        from_clock_id="K1-AABBCCDDEEFF/monotonic",
        from_clock_hash="deadbeefcafefeed",
        to_clock_id="K1-AABBCCDDEEFF/utc",
        to_clock_hash="1234567890abcdef",
        source={"kind": "local_clock_read"},
        segment_duration_ns=1_000_000_000,
        retention_ns=60_000_000_000,
    )
    assert m["source"]["kind"] == "local_clock_read"
    assert m["from_clock_id"] == "K1-AABBCCDDEEFF/monotonic"

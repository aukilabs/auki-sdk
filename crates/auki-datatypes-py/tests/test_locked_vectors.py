"""Cross-language byte-equality tests for the betterproto-generated dataclasses.

Each test pins a Rust fixture from `auki-datatypes/src/lib.rs` and asserts that
the betterproto encoder produces byte-identical bytes. If a regen breaks wire
compat with the Rust prost encoder, these tests trip immediately.

The locked vectors are copied verbatim from `auki-datatypes/src/lib.rs`'s
`*_serializes_to_locked_wire_bytes` tests. **Keep them in sync.** When the
Rust crate adds or changes a locked vector, mirror it here.

Run via::

    pip install -e .[test]
    pytest tests/
"""

from __future__ import annotations

import auki_datatypes as adt


# ─── Step 1 — auki.camera ────────────────────────────────────────────────────


def test_pinhole_camera_log_entry_locked_wire_bytes():
    # auki-datatypes/src/lib.rs::pinhole_camera_log_entry_serializes_to_locked_wire_bytes
    entry = adt.camera.PinholeCameraLogEntry(
        dynamic_intrinsics=adt.camera.DynamicIntrinsics(
            fx=1234.5,
            fy=1234.5,
            cx=272.0,
            cy=244.0,
            distortion_coefficients=[0.1, -0.2, 0.001, 0.002, 0.0],
        ),
        frame=bytes([0x00, 0x01, 0x02, 0x03]),
    )
    expected = (
        "0a4e0900000000004a93401100000000004a93401900000000000071402100000000008"
        "06e402a289a9999999999b93f9a9999999999c9bffca9f1d24d62503ffca9f1d24d62603"
        "f0000000000000000120400010203"
    )
    assert bytes(entry).hex() == expected


# ─── Step 3 — auki.point_cloud ───────────────────────────────────────────────


def test_point_cloud_log_entry_locked_wire_bytes():
    # auki-datatypes/src/lib.rs::point_cloud_log_entry_serializes_to_locked_wire_bytes
    entry = adt.point_cloud.PointCloudLogEntry(data=bytes(range(24)))
    expected = "0a18000102030405060708090a0b0c0d0e0f1011121314151617"
    assert bytes(entry).hex() == expected


# ─── Step 4 — auki.audio ─────────────────────────────────────────────────────


def test_audio_log_entry_locked_wire_bytes():
    # auki-datatypes/src/lib.rs::audio_log_entry_serializes_to_locked_wire_bytes
    # 16 bytes of stereo `pcm_s16le` — i.wrapping_mul(17) for i in 0..16
    data = bytes((i * 17) & 0xFF for i in range(16))
    entry = adt.audio.AudioLogEntry(data=data)
    expected = "0a1000112233445566778899aabbccddeeff"
    assert bytes(entry).hex() == expected


# ─── Step 5 — auki.pose ──────────────────────────────────────────────────────


def test_spatial_transform_locked_wire_bytes():
    # auki-datatypes/src/lib.rs::spatial_transform_serializes_to_locked_wire_bytes
    st = adt.pose.SpatialTransform(
        translation=adt.pose.Vec3(x=1.0, y=2.0, z=3.0),
        orientation=adt.pose.Quat(x=0.0, y=0.0, z=0.0, w=1.0),
    )
    expected = (
        "0a1b09000000000000f03f110000000000000040190000000000000840"
        "120921000000000000f03f"
    )
    assert bytes(st).hex() == expected


# ─── Step 6 — auki.time_transform ────────────────────────────────────────────


def test_time_transform_entry_locked_wire_bytes():
    # auki-datatypes/src/lib.rs::time_transform_entry_serializes_to_locked_wire_bytes
    entry = adt.time_transform.TimeTransformEntry(
        offset_ns=1_000_000,
        uncertainty_ns=250,
    )
    expected = "08c0843d10fa01"
    assert bytes(entry).hex() == expected


# ─── Step 8 — auki.detection ─────────────────────────────────────────────────


def test_detection_log_entry_locked_wire_bytes():
    # auki-datatypes/src/lib.rs::detection_log_entry_serializes_to_locked_wire_bytes
    entry = adt.detection.DetectionLogEntry(data=bytes(range(12)))
    expected = "0a0c000102030405060708090a0b"
    assert bytes(entry).hex() == expected


# ─── PR #77 — auki.joint_encoders ────────────────────────────────────────────


def test_joint_encoders_log_entry_locked_wire_bytes():
    # auki-datatypes/src/lib.rs::joint_encoders_log_entry_serializes_to_locked_wire_bytes
    # 6-DOF fixture: [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]
    entry = adt.joint_encoders.JointEncodersLogEntry(angles_rad=[0.0, 1.0, 2.0, 3.0, 4.0, 5.0])
    expected = "0a18000000000000803f0000004000004040000080400000a040"
    assert bytes(entry).hex() == expected


# ─── Round-trip parity ───────────────────────────────────────────────────────


def test_detection_log_entry_round_trips():
    e = adt.detection.DetectionLogEntry(data=b"hello")
    e2 = adt.detection.DetectionLogEntry().parse(bytes(e))
    assert e2.data == b"hello"


def test_spatial_transform_round_trips():
    st = adt.pose.SpatialTransform(
        translation=adt.pose.Vec3(x=10.0, y=-20.0, z=0.5),
        orientation=adt.pose.Quat(x=0.0, y=0.0, z=0.7071, w=0.7071),
    )
    st2 = adt.pose.SpatialTransform().parse(bytes(st))
    assert st2.translation.x == 10.0
    assert st2.translation.y == -20.0
    assert st2.orientation.w == 0.7071


def test_stream_manifest_round_trips():
    manifest = adt.stream.StreamManifest(
        sensor_id="robot/head_cam",
        sensor_hash="sensor-hash",
        clock_id="robot/clock",
        clock_hash="clock-hash",
        frame_id="robot/head_cam/frame",
        frame_hash="frame-hash",
    )
    parsed = adt.stream.StreamManifest().parse(bytes(manifest))
    assert parsed == manifest


def test_module_re_exports_all_packages():
    # Smoke test — every proto package is re-exported from the top
    # level so consumers can `from auki_datatypes import detection`
    # rather than `from auki_datatypes.auki.detection`.
    for name in [
        "audio",
        "camera",
        "detection",
        "joint_encoders",
        "joint_encoders_stream",
        "point_cloud",
        "point_cloud_stream",
        "pose",
        "stream",
        "time_transform",
    ]:
        assert hasattr(adt, name), f"missing top-level submodule: {name}"

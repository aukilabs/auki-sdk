"""Python-side tests for ``auki_layout``.

Run via::

    cd bindings/python/auki-layout-py
    python -m venv .venv && source .venv/bin/activate
    pip install maturin pytest
    maturin develop --release
    pytest python_tests/

The wrappers are pure-function — Python sees `str` returns. Tests pin
the same path-substitution rules the Rust crate's tests pin.
"""

from __future__ import annotations

import auki_layout


APP = "/home/booster/auki/boosterapp"


def test_module_exposes_path_helpers():
    for name in [
        "registries_root",
        "sensor_entry_path",
        "clock_entry_path",
        "frame_entry_path",
        "session_root",
        "timetransform_log_path",
        "sensorlog_path",
        "poselog_path",
        "detection_log_path",
        "id_to_segment",
    ]:
        assert hasattr(auki_layout, name), f"missing {name}"


def test_registries_root():
    assert auki_layout.registries_root(APP) == f"{APP}/registries"


def test_sensor_entry_path_substitutes_slashes():
    assert auki_layout.sensor_entry_path(
        APP, "K1-AABBCCDDEEFF/head_left_cam", "deadbeef"
    ) == f"{APP}/registries/sensors/K1-AABBCCDDEEFF__head_left_cam/deadbeef.json"


def test_session_root():
    assert auki_layout.session_root(APP, "abc-123") == f"{APP}/abc-123"


def test_timetransform_log_path_uses_double_underscore():
    session = auki_layout.session_root(APP, "abc-123")
    assert (
        auki_layout.timetransform_log_path(session, "K1-AABB/utc", "K1-AABB/monotonic")
        == f"{APP}/abc-123/timetransform_logs/K1-AABB__utc__K1-AABB__monotonic"
    )


def test_sensorlog_path_passes_through_log_id():
    session = auki_layout.session_root(APP, "abc-123")
    assert (
        auki_layout.sensorlog_path(session, "rec-456")
        == f"{APP}/abc-123/sensorlogs/rec-456"
    )


def test_poselog_path():
    session = auki_layout.session_root(APP, "abc-123")
    assert (
        auki_layout.poselog_path(session, "K1-AABB/base_link", "K1-AABB/cam_optical")
        == f"{APP}/abc-123/poselogs/K1-AABB__base_link__K1-AABB__cam_optical"
    )


def test_detection_log_path_keys_on_detector_id_and_input_log_id():
    session = auki_layout.session_root(APP, "abc-123")
    assert (
        auki_layout.detection_log_path(session, "aukilabs/qr/v1", "rec-456")
        == f"{APP}/abc-123/detection_logs/aukilabs__qr__v1__rec-456"
    )


def test_id_to_segment_is_idempotent_for_ids_without_slashes():
    assert auki_layout.id_to_segment("plain") == "plain"
    assert auki_layout.id_to_segment("foo/bar/baz") == "foo__bar__baz"

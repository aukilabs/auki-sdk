"""Python-side tests for ``auki_logs``.

Run via::

    cd crates/auki-logs-py
    python -m venv .venv && source .venv/bin/activate
    pip install maturin pytest
    maturin develop --release
    pytest python_tests/

Tests are split into two tiers:

1. **Surface tests** — module shape, type construction, getters, error
   mapping. Fast.
2. **Round-trip tests** — open + append + read + tail. End-to-end.
"""

from __future__ import annotations

import threading
import time
from pathlib import Path

import pytest

import auki_logs


def manifest(segment_ns: int = 1_000_000_000, retention_ns: int = 60_000_000_000) -> dict:
    return {
        "segment_duration_ns": segment_ns,
        "retention_ns": retention_ns,
        "kind": "test",
    }


# ─── Surface tests ───────────────────────────────────────────────────────────


def test_module_exports_expected_classes():
    assert hasattr(auki_logs, "Log")
    assert hasattr(auki_logs, "LogReader")
    assert hasattr(auki_logs, "TailIter")
    assert hasattr(auki_logs, "Entry")


def test_open_creates_layout_and_writes_manifest(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.close()
    assert (tmp_path / "log_manifest.json").exists()
    assert (tmp_path / "segments").is_dir()


def test_manifest_returns_dict(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    m = log.manifest()
    assert isinstance(m, dict)
    assert m["segment_duration_ns"] == 1_000_000_000
    assert m["retention_ns"] == 60_000_000_000
    assert m["kind"] == "test"
    log.close()


def test_open_rejects_invalid_manifest(tmp_path: Path):
    # `segment_duration_ns` must be > 0; the validator surfaces a
    # ValueError mapped from `auki_logs::Error::Manifest`.
    with pytest.raises(ValueError, match="manifest"):
        auki_logs.Log.open(str(tmp_path), {"segment_duration_ns": 0, "retention_ns": 0})


def test_append_after_close_raises(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.close()
    with pytest.raises(RuntimeError, match="closed"):
        log.append(100, b"hi")


# ─── Round-trip tests ────────────────────────────────────────────────────────


def test_round_trip_two_entries(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.append(100, b"hello")
    log.append(200, b"world")
    log.close()

    reader = auki_logs.Log.read(str(tmp_path))
    entries = reader.entries()
    assert len(entries) == 2
    assert entries[0].timestamp_ns == 100
    assert entries[0].payload == b"hello"
    assert entries[1].timestamp_ns == 200
    assert entries[1].payload == b"world"


def test_empty_payload_round_trip(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.append(100, b"")
    log.close()

    reader = auki_logs.Log.read(str(tmp_path))
    entries = reader.entries()
    assert len(entries) == 1
    assert entries[0].payload == b""


def test_segment_starts_lists_segments(tmp_path: Path):
    seg = 1_000_000_000  # 1s
    log = auki_logs.Log.open(str(tmp_path), manifest(segment_ns=seg, retention_ns=60 * seg))
    log.append(100, b"in seg 0")
    log.append(seg + 100, b"in seg 1")  # rolls over
    log.close()

    reader = auki_logs.Log.read(str(tmp_path))
    starts = reader.segment_starts()
    assert starts == [0, seg]


def test_context_manager_closes_log(tmp_path: Path):
    with auki_logs.Log.open(str(tmp_path), manifest()) as log:
        log.append(100, b"inside")
    # After __exit__, the log is closed.
    with pytest.raises(RuntimeError, match="closed"):
        log.append(200, b"after")


def test_set_retention_persists(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.set_retention(120_000_000_000)  # 120s
    assert log.manifest()["retention_ns"] == 120_000_000_000
    log.close()
    # Re-open: persisted value drives behavior.
    reader = auki_logs.Log.read(str(tmp_path))
    assert reader.manifest()["retention_ns"] == 120_000_000_000


# ─── Tail tests ──────────────────────────────────────────────────────────────


def test_tail_starts_at_eof_skipping_existing_entries(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.append(100, b"existing")
    log.flush()

    tail = auki_logs.Log.tail(str(tmp_path))
    assert tail.try_next() is None  # no entries past current EOF

    log.append(200, b"new")
    log.flush()

    entry = tail.try_next()
    assert entry is not None
    assert entry.timestamp_ns == 200
    assert entry.payload == b"new"
    log.close()


def test_tail_iterator_protocol_blocks_until_entry(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.flush()

    tail = auki_logs.Log.tail(str(tmp_path))

    # Spawn a writer thread that appends after a short delay.
    def writer():
        time.sleep(0.05)
        log.append(100, b"e1")
        log.flush()

    t = threading.Thread(target=writer)
    t.start()

    # Blocking next() should yield the entry once the writer flushes.
    entry = next(iter(tail))
    t.join()
    assert entry.timestamp_ns == 100
    assert entry.payload == b"e1"
    log.close()


def test_tail_with_poll_interval_does_not_raise(tmp_path: Path):
    auki_logs.Log.open(str(tmp_path), manifest()).close()
    tail = auki_logs.Log.tail(str(tmp_path)).with_poll_interval(50)
    assert tail.try_next() is None


def test_entry_repr_is_informative(tmp_path: Path):
    log = auki_logs.Log.open(str(tmp_path), manifest())
    log.append(100, b"payload")
    log.close()
    reader = auki_logs.Log.read(str(tmp_path))
    entries = reader.entries()
    r = repr(entries[0])
    assert "100" in r
    assert "7 bytes" in r  # len(b"payload") == 7


# ─── ESL-style end-to-end ────────────────────────────────────────────────────


def test_detector_pattern_smoke(tmp_path: Path):
    """The minimum surface a phase-2 detector loop exercises:

    1. Open input + output logs.
    2. Tail the input log.
    3. For each input entry, run a fake detector and append to output.
    4. Confirm both round-trip cleanly.

    No prost — the test passes opaque bytes through. A real detector
    would deserialize input bytes via ``betterproto`` (the
    ``auki-datatypes`` Python codegen, not yet built) and serialize
    output bytes the same way.
    """
    input_dir = tmp_path / "input"
    output_dir = tmp_path / "output"

    input_log = auki_logs.Log.open(str(input_dir), manifest())
    output_log = auki_logs.Log.open(str(output_dir), manifest())

    # Feed the input log + detector-loop simultaneously so the tail picks
    # up entries as they arrive.
    def fake_camera():
        for i in range(3):
            time.sleep(0.02)
            input_log.append(100 * (i + 1), f"frame-{i}".encode())
            input_log.flush()

    cam_thread = threading.Thread(target=fake_camera)
    cam_thread.start()

    tail = auki_logs.Log.tail(str(input_dir)).with_poll_interval(5)
    seen = []
    for _ in range(3):
        entry = next(iter(tail))
        seen.append((entry.timestamp_ns, entry.payload))
        # "Detect" is just append-the-input-prefix-plus-a-tag.
        detection = b"detected:" + entry.payload
        output_log.append(entry.timestamp_ns, detection)

    cam_thread.join()
    input_log.close()
    output_log.close()

    # Read the output log back; confirm three "detected:" entries.
    reader = auki_logs.Log.read(str(output_dir))
    entries = reader.entries()
    assert len(entries) == 3
    for i, e in enumerate(entries):
        assert e.timestamp_ns == 100 * (i + 1)
        assert e.payload == b"detected:" + f"frame-{i}".encode()

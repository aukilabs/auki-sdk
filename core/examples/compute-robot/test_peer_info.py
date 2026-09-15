"""Authorization tests for the opt-in Info demonstration; no network."""
import asyncio
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import pytest
from test_example import Data, task, RUN, DOMAIN, load


def test_info_only_discloses_run_marker_to_expected_peer():
    path = Path(__file__).with_name("peer_info.py")
    assert path.exists(), "Info demonstration is missing"
    spec = importlib.util.spec_from_file_location("peer_info", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    info = mod.info_provider("robot-peer", RUN, "compute-peer")
    assert info({"peer_id": "another-peer"}) is None
    assert info({"peer_id": "compute-peer"})["name"] == RUN
    assert info({"peer_id": "compute-peer"})["peer_id"] == "robot-peer"


def test_compute_rejects_unconfigured_peer_before_data_access():
    data = Data()
    item = task(data, meta={"run_id": RUN, "input_id": "input", "remote_peer_id": "unapproved", "remote_route": "route"})
    with pytest.raises(ValueError):
        asyncio.run(load("compute").handle(item, DOMAIN, RUN))
    assert data.reads == []

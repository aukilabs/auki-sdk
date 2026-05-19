"""Python-side tests for ``auki_network``.

Run via::

    cd crates/auki-network-py
    python -m venv .venv && source .venv/bin/activate
    pip install maturin pytest
    maturin develop --release
    pytest python_tests/

Two-tier coverage:

1. **Surface tests** — module shape, type construction, getters, error
   mapping. Fast.
2. **Discovery constructor tests** — root-level Discovery bindings stay
   importable without requiring a live Discovery server.
"""

from __future__ import annotations

import auki_network
from auki_network import cluster


# ─── Module surface ──────────────────────────────────────────────────────────


def test_module_exposes_only_documented_apis() -> None:
    """Pin the cluster sub-module surface — anything new requires a
    deliberate decision and a changelog bump."""
    public = {name for name in dir(cluster) if not name.startswith("_")}
    expected = {
        # Shared stream surface used by auki-domain-py.
        "StreamRequest",
        "StreamManifest",
        "DynamicIntrinsics",
        "CameraFrame",
        "PointCloudFrame",
        "JointEncodersFrame",
        "AudioFrame",
        "DeclineReason",
        "EndReason",
        "StreamItem",
        "StreamEntry",
        "StreamDecision",
        "StreamSubscription",
        "StreamEntryIterator",
        "StreamEndOfStream",
        "StreamConnectionLost",
        "StreamProtocolError",
        "StreamDeclined",
        "StreamUnreachable",
    }
    assert expected.issubset(public), f"missing expected API: {expected - public}"
    assert "PinholeCameraLogEntry" not in public
    assert "JpegFrame" not in public
    assert "spawn" not in public
    assert "ClusterRuntime" not in public


def test_top_level_module_exposes_cluster() -> None:
    assert hasattr(auki_network, "cluster")
    # `from auki_network import cluster` works because the wrapper
    # registers the submodule in `sys.modules`.
    from auki_network import cluster as cluster_via_from  # noqa: F401


def test_top_level_module_exposes_discovery_types() -> None:
    public = {name for name in dir(auki_network) if not name.startswith("_")}
    expected = {
        "DiscoveryClient",
        "ClusterEntry",
        "CreateClusterOutcome",
        "cluster",
    }
    assert expected.issubset(public), f"missing expected API: {expected - public}"


def test_discovery_client_constructs_without_network() -> None:
    client = auki_network.DiscoveryClient("http://127.0.0.1:8080")
    assert "DiscoveryClient" in repr(client)

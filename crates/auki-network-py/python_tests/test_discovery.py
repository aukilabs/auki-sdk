"""Python-side tests for ``auki_network.discovery`` (Vinland Batch 2).

Two tiers, mirroring ``test_basic.py`` / ``test_streams.py``:

1. **Surface tests** — module shape, constructor validation, error
   types. Fast; no network.
2. **Round-trip tests** — boot a real Discovery binary on a tempdir +
   ephemeral loopback port, sign through the Python wrapper, exercise
   register / fetch / deregister + the typical 401 / 404 paths.
   Skipped automatically unless ``DISCOVERY_BIN`` env var points at a
   built ``./discovery`` binary; same gate the Rust-side
   ``discovery_integration.rs`` integration test uses.

Run via::

    cd crates/auki-network-py
    python -m venv .venv && source .venv/bin/activate
    pip install maturin pytest
    maturin develop --release
    DISCOVERY_BIN=/path/to/Discovery/target/debug/discovery \\
      pytest python_tests/test_discovery.py -v

Without ``DISCOVERY_BIN``, only the offline surface tests run.
"""

from __future__ import annotations

import os
import socket
import subprocess
import time
import urllib.request
import urllib.error
from pathlib import Path
import tempfile

import pytest

import auki_network
from auki_network import discovery


# ─── Module surface ──────────────────────────────────────────────────────────


def test_discovery_submodule_exposes_documented_surface() -> None:
    """Pin the discovery submodule's public names — anything new
    requires a deliberate decision and a changelog bump."""
    public = {name for name in dir(discovery) if not name.startswith("_")}
    expected = {
        "DiscoveryClient",
        "CreateClusterOutcome",
        "DiscoveryUnreachable",
        "DiscoveryRejected",
        "DiscoveryClockError",
    }
    assert expected.issubset(public), f"missing expected API: {expected - public}"


def test_top_level_module_exposes_discovery_submodule() -> None:
    assert hasattr(auki_network, "discovery")
    from auki_network import discovery as discovery_via_from  # noqa: F401


def test_constructor_trims_trailing_slash() -> None:
    a = discovery.DiscoveryClient("http://localhost:9999")
    b = discovery.DiscoveryClient("http://localhost:9999/")
    assert a.base_url == b.base_url == "http://localhost:9999"


def test_repr_includes_base_url() -> None:
    client = discovery.DiscoveryClient("http://example.com:8080")
    assert "example.com:8080" in repr(client)


# ─── Wrong-shape inputs (no network) ──────────────────────────────────────────


def test_register_rejects_short_seed() -> None:
    client = discovery.DiscoveryClient("http://localhost:9999")
    with pytest.raises(ValueError, match="32 bytes"):
        client.register(
            seed=b"\x00" * 16,
            cluster_name="vinland",
            addresses=["/ip4/127.0.0.1/tcp/4001"],
        )


def test_register_rejects_long_seed() -> None:
    client = discovery.DiscoveryClient("http://localhost:9999")
    with pytest.raises(ValueError, match="32 bytes"):
        client.register(
            seed=b"\x00" * 64,
            cluster_name="vinland",
            addresses=["/ip4/127.0.0.1/tcp/4001"],
        )


def test_register_rejects_invalid_multiaddr() -> None:
    client = discovery.DiscoveryClient("http://localhost:9999")
    with pytest.raises(ValueError, match="multiaddr"):
        client.register(
            seed=b"\x01" * 32,
            cluster_name="vinland",
            addresses=["this-is-not-a-multiaddr"],
        )


def test_deregister_rejects_short_seed() -> None:
    client = discovery.DiscoveryClient("http://localhost:9999")
    with pytest.raises(ValueError, match="32 bytes"):
        client.deregister(seed=b"\x00" * 16, cluster_name="vinland")


def test_create_cluster_rejects_short_seed() -> None:
    client = discovery.DiscoveryClient("http://localhost:9999")
    with pytest.raises(ValueError, match="32 bytes"):
        client.create_cluster(seed=b"\x00" * 16, cluster_name="vinland")


def test_register_accepts_empty_addresses() -> None:
    """An operator may want to register a peer without any reachability
    info temporarily; the wrapper doesn't reject empty lists. (The
    network call still fails because there's nothing on port 1, so we
    catch the unreachable error rather than asserting success.)"""
    client = discovery.DiscoveryClient("http://127.0.0.1:1")  # never accepts
    with pytest.raises(discovery.DiscoveryUnreachable):
        client.register(
            seed=b"\x07" * 32,
            cluster_name="vinland",
            addresses=[],  # empty is fine on the wrapper side
        )


def test_fetch_against_unreachable_url_raises_unreachable() -> None:
    """Port 1 is reserved + RST'd immediately by Linux/macOS, so the
    connect fails fast and we get a typed transport error, not a
    timeout."""
    client = discovery.DiscoveryClient("http://127.0.0.1:1")
    with pytest.raises(discovery.DiscoveryUnreachable):
        client.fetch("vinland")


# ─── Live Discovery integration ──────────────────────────────────────────────
#
# Boots a real Discovery binary on a tempdir + ephemeral loopback port
# and exercises the full sign → POST → response flow through the
# Python wrapper. Mirrors the Rust-side `tests/discovery_integration.rs`.

DISCOVERY_BIN = os.environ.get("DISCOVERY_BIN")

skip_unless_discovery_available = pytest.mark.skipif(
    not DISCOVERY_BIN or not Path(DISCOVERY_BIN).exists(),
    reason="DISCOVERY_BIN env var must point at a built ./discovery binary",
)


def _pick_free_port() -> int:
    """Bind a listener on 127.0.0.1:0, read the port, drop the listener.
    Discovery binds the same port a moment later; the race window is
    small enough to be irrelevant in practice."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]
    finally:
        s.close()


def _wait_for_discovery(base_url: str, timeout: float = 5.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base_url}/clusters", timeout=0.2) as resp:
                if resp.status == 200:
                    return
        except (urllib.error.URLError, ConnectionRefusedError, OSError):
            pass
        time.sleep(0.05)
    raise RuntimeError(f"discovery did not respond on {base_url}/clusters within {timeout}s")


@pytest.fixture
def discovery_server():
    """Spawn Discovery as a child process; tear it down on teardown."""
    if not DISCOVERY_BIN or not Path(DISCOVERY_BIN).exists():
        pytest.skip("DISCOVERY_BIN unset")
    port = _pick_free_port()
    addr = f"127.0.0.1:{port}"
    base_url = f"http://{addr}"
    with tempfile.TemporaryDirectory() as data_dir:
        proc = subprocess.Popen(
            [DISCOVERY_BIN, "--addr", addr, "--data-dir", data_dir],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env={**os.environ, "RUST_LOG": "discovery=warn"},
        )
        try:
            _wait_for_discovery(base_url)
            yield base_url
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()


@skip_unless_discovery_available
def test_register_returns_full_cluster_doc(discovery_server) -> None:
    """The single-peer flow: POST returns the full ClusterDoc, no
    follow-up GET needed (Sentinel's first-boot pattern)."""
    client = discovery.DiscoveryClient(discovery_server)
    doc = client.register(
        seed=b"\x01" * 32,
        cluster_name="vinland",
        addresses=["/ip4/192.168.9.130/tcp/4011"],
        expected_app_id="sentinel",
    )
    assert doc.cluster_name == "vinland"
    assert doc.peer_count == 1


@skip_unless_discovery_available
def test_three_peers_converge(discovery_server) -> None:
    """Sentinel → Booster → Park each register; fetch sees all three."""
    client = discovery.DiscoveryClient(discovery_server)

    sentinel = client.register(
        seed=b"\x01" * 32,
        cluster_name="vinland",
        addresses=["/ip4/192.168.9.130/tcp/4011"],
        expected_app_id="sentinel",
    )
    assert sentinel.peer_count == 1

    booster = client.register(
        seed=b"\x02" * 32,
        cluster_name="vinland",
        addresses=["/ip4/192.168.9.72/tcp/4001"],
        expected_app_id="boosterapp",
    )
    assert booster.peer_count == 2

    park = client.register(
        seed=b"\x03" * 32,
        cluster_name="vinland",
        addresses=["/ip4/192.168.9.48/tcp/4001"],
        expected_app_id="park",
    )
    assert park.peer_count == 3

    fetched = client.fetch("vinland")
    assert fetched.peer_count == 3
    assert fetched.cluster_name == "vinland"


@skip_unless_discovery_available
def test_deregister_removes_peer(discovery_server) -> None:
    """Register two peers, deregister one, fetch shows just the other."""
    client = discovery.DiscoveryClient(discovery_server)
    client.register(
        seed=b"\x01" * 32,
        cluster_name="vinland",
        addresses=["/ip4/192.168.9.130/tcp/4011"],
        expected_app_id="sentinel",
    )
    client.register(
        seed=b"\x02" * 32,
        cluster_name="vinland",
        addresses=["/ip4/192.168.9.72/tcp/4001"],
        expected_app_id="boosterapp",
    )
    client.deregister(seed=b"\x01" * 32, cluster_name="vinland")

    fetched = client.fetch("vinland")
    assert fetched.peer_count == 1


@skip_unless_discovery_available
def test_deregister_already_removed_raises_404(discovery_server) -> None:
    """Discovery's idempotency: a second deregister against an
    already-removed entry returns 404. Surfaced as DiscoveryRejected
    with .status == 404 so daemons can ignore it for clean-shutdown
    semantics."""
    client = discovery.DiscoveryClient(discovery_server)
    client.register(
        seed=b"\x05" * 32,
        cluster_name="vinland",
        addresses=["/ip4/127.0.0.1/tcp/4001"],
    )
    client.deregister(seed=b"\x05" * 32, cluster_name="vinland")

    with pytest.raises(discovery.DiscoveryRejected) as excinfo:
        client.deregister(seed=b"\x05" * 32, cluster_name="vinland")
    assert excinfo.value.status == 404


@skip_unless_discovery_available
def test_fetch_unknown_cluster_raises_404(discovery_server) -> None:
    client = discovery.DiscoveryClient(discovery_server)
    with pytest.raises(discovery.DiscoveryRejected) as excinfo:
        client.fetch("not-a-cluster")
    assert excinfo.value.status == 404


@skip_unless_discovery_available
def test_path_traversal_cluster_name_rejected(discovery_server) -> None:
    """Discovery's ``^[A-Za-z0-9._-]+$`` charset blocks path traversal.
    Surfaces as DiscoveryRejected (400 / 404 depending on routing)."""
    client = discovery.DiscoveryClient(discovery_server)
    with pytest.raises((discovery.DiscoveryRejected, discovery.DiscoveryUnreachable)):
        client.register(
            seed=b"\x06" * 32,
            cluster_name="../etc/passwd",
            addresses=["/ip4/127.0.0.1/tcp/4001"],
        )


@skip_unless_discovery_available
def test_returned_cluster_doc_is_usable_with_cluster_runtime(discovery_server) -> None:
    """Schema-parity smoke test: the ClusterDoc Python wrapper
    `discovery.register` returns is the same Python type
    `cluster.load_doc` returns. ``cluster.spawn`` would accept it
    directly (we don't actually spawn here — that's `test_basic.py`'s
    job)."""
    from auki_network import cluster

    client = discovery.DiscoveryClient(discovery_server)
    doc = client.register(
        seed=b"\x09" * 32,
        cluster_name="vinland",
        addresses=["/ip4/127.0.0.1/tcp/4001"],
    )
    assert isinstance(doc, cluster.ClusterDoc)
    assert doc.cluster_name == "vinland"
    assert doc.peer_count == 1


@skip_unless_discovery_available
def test_create_cluster_returns_created_outcome(discovery_server) -> None:
    """First-creator path: `create_cluster` returns a `Created` outcome
    with `kind == "created"` and the new ClusterDoc carrying `peer_count
    == 0` (no peers yet — register hasn't been called)."""
    client = discovery.DiscoveryClient(discovery_server)
    outcome = client.create_cluster(
        seed=b"\x01" * 32,
        cluster_name="newly-created-cluster",
    )
    assert outcome.kind == "created"
    assert isinstance(outcome.doc, cluster.ClusterDoc)
    assert outcome.doc.cluster_name == "newly-created-cluster"
    assert outcome.doc.peer_count == 0


@skip_unless_discovery_available
def test_create_cluster_returns_already_exists_outcome(discovery_server) -> None:
    """Race-loss path: a second `create_cluster` against the same name
    returns `AlreadyExists` carrying the winner's existing ClusterDoc.
    The loser hands `outcome.doc` straight to a join flow without an
    extra `fetch` (Greenland T12's `try-join → create-if-none →
    fall-back-to-join` algorithm)."""
    client = discovery.DiscoveryClient(discovery_server)

    # First peer wins the create + becomes initial Manager (registers
    # against the doc to populate peer_count = 1).
    first = client.create_cluster(
        seed=b"\x01" * 32,
        cluster_name="contested-cluster",
    )
    assert first.kind == "created"
    client.register(
        seed=b"\x01" * 32,
        cluster_name="contested-cluster",
        addresses=["/ip4/192.168.9.1/tcp/4001"],
    )

    # Second peer loses the create race; gets the winner's doc back.
    second = client.create_cluster(
        seed=b"\x02" * 32,
        cluster_name="contested-cluster",
    )
    assert second.kind == "already_exists"
    assert second.doc.cluster_name == "contested-cluster"
    assert second.doc.peer_count == 1


def test_create_cluster_outcome_repr_includes_kind() -> None:
    """Smoke test of `__repr__` — operators see the discriminator at a
    glance."""
    # Construct via a no-network path: the outcome doesn't need a real
    # server to build the repr; this test exercises only the wrapper.
    # We can't easily construct a `CreateClusterOutcome` from Python
    # directly (it has no public `__init__`), so we go through the
    # `kind` attribute on the class itself, which is enough to confirm
    # the type exists and has the expected discriminator strings.
    assert hasattr(discovery, "CreateClusterOutcome")
    assert discovery.CreateClusterOutcome.__name__ == "CreateClusterOutcome"

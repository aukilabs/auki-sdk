"""Smoke tests for the `auki_domain` Python module.

Run after building the wheel:

    maturin develop -m crates/auki-domain-py/Cargo.toml
    pytest crates/auki-domain-py/python_tests/

These tests don't need a live Discovery — they exercise the module
surface, the input-validation paths, and the exception classes.
End-to-end `init_domain` tests against a real Discovery would belong
in a separate live-gated test file once Discovery has a Python test
harness consumers can drive (mirroring `auki-network-py`'s
`python_tests/test_discovery.py`).
"""

from __future__ import annotations

import pytest


def test_module_imports() -> None:
    """The `auki_domain` module imports and exposes the documented
    public surface."""
    import auki_domain

    assert hasattr(auki_domain, "init_domain")
    assert hasattr(auki_domain, "DomainHandle")
    assert hasattr(auki_domain, "DomainAlreadyExists")
    assert hasattr(auki_domain, "DiscoveryUnreachable")
    assert hasattr(auki_domain, "DiscoveryRejected")
    assert hasattr(auki_domain, "DiscoveryClockError")
    assert hasattr(auki_domain, "RuntimeSpawnError")


def test_init_domain_rejects_wrong_seed_length() -> None:
    """`init_domain` validates seed lengths synchronously, before
    any tokio runtime / Discovery / swarm work."""
    import auki_domain

    short = b"\x00" * 16
    valid = b"\x00" * 32
    with pytest.raises(ValueError, match="wallet_seed must be exactly 32 bytes"):
        auki_domain.init_domain(
            wallet_seed=short,
            peer_seed=valid,
            discovery_url="http://127.0.0.1:8080",
            domain_name="Vinland",
            addresses=["/ip4/127.0.0.1/tcp/4001"],
            participant_provider=lambda: None,
        )
    with pytest.raises(ValueError, match="peer_seed must be exactly 32 bytes"):
        auki_domain.init_domain(
            wallet_seed=valid,
            peer_seed=short,
            discovery_url="http://127.0.0.1:8080",
            domain_name="Vinland",
            addresses=["/ip4/127.0.0.1/tcp/4001"],
            participant_provider=lambda: None,
        )


def test_init_domain_rejects_empty_domain_name() -> None:
    """Empty `domain_name` rejects synchronously — would surface as a
    confusing 4xx from Discovery otherwise."""
    import auki_domain

    valid = b"\x00" * 32
    with pytest.raises(ValueError, match="domain_name must not be empty"):
        auki_domain.init_domain(
            wallet_seed=valid,
            peer_seed=valid,
            discovery_url="http://127.0.0.1:8080",
            domain_name="",
            addresses=["/ip4/127.0.0.1/tcp/4001"],
            participant_provider=lambda: None,
        )


def test_init_domain_rejects_empty_addresses() -> None:
    """At least one dialable multiaddr is required — Discovery has
    to publish *something* peers can reach us at."""
    import auki_domain

    valid = b"\x00" * 32
    with pytest.raises(ValueError, match="addresses must contain at least one"):
        auki_domain.init_domain(
            wallet_seed=valid,
            peer_seed=valid,
            discovery_url="http://127.0.0.1:8080",
            domain_name="Vinland",
            addresses=[],
            participant_provider=lambda: None,
        )


def test_init_domain_rejects_unparseable_multiaddr() -> None:
    """Multiaddr parse errors surface with the offending string
    quoted so operators can find their typo."""
    import auki_domain

    valid = b"\x00" * 32
    with pytest.raises(ValueError, match="invalid address"):
        auki_domain.init_domain(
            wallet_seed=valid,
            peer_seed=valid,
            discovery_url="http://127.0.0.1:8080",
            domain_name="Vinland",
            addresses=["this-is-not-a-multiaddr"],
            participant_provider=lambda: None,
        )


def test_exception_classes_have_expected_bases() -> None:
    """Operator-friendly exception hierarchy: `DiscoveryUnreachable`
    extends `ConnectionError` (catchable as a transport failure);
    the others extend `RuntimeError`."""
    import auki_domain

    assert issubclass(auki_domain.DiscoveryUnreachable, ConnectionError)
    assert issubclass(auki_domain.DiscoveryRejected, RuntimeError)
    assert issubclass(auki_domain.DiscoveryClockError, RuntimeError)
    assert issubclass(auki_domain.DomainAlreadyExists, RuntimeError)
    assert issubclass(auki_domain.RuntimeSpawnError, RuntimeError)

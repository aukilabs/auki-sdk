# Sprint — auki-identity-py

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now (initial release — landed)

The crate ships the three documented APIs and nothing else:

- `load_or_mint_seed(path) -> bytes` — wraps `auki_identity::load_or_mint_seed`, with `OSError` / `ValueError` mapping per the upstream `SeedError` enum.
- `Wallet.from_seed(seed)` + `Wallet.derive_child(label)` + `Wallet.peer_id()` — wraps `auki_identity::Wallet` and the canonical libp2p PeerId encoding from `auki_network::PeerIdentity::from_seed`.
- `app_instance.derive() -> str` — wraps `auki_network::app_instance::derive`, with `RuntimeError` (variant name in message) / `OSError` mapping per the upstream `DeriveError` enum.

Built via `maturin` (PEP 517 backend declared in `pyproject.toml`). PyO3 0.22 with the `Bound<...>` API; `abi3-py38` so one wheel works on Python 3.8+. `crate-type = ["cdylib", "rlib"]` so `cargo test` can drive the bindings via `Python::with_gil` while `maturin develop` produces a Python-importable extension.

Tests: 5 Rust-side smoke tests in `lib.rs`; 13 Python-side end-to-end tests in `python_tests/test_basic.py`, including a Unix-only `0o600` mode check, a container-tolerant `app_instance.derive` test, and the cross-language locked vector for `Wallet.from_seed(b'\x03' * 32).derive_child("peer/v1").peer_id()`.

## Next

In priority order:

1. **End-to-end-test the pip-from-Git install** before recommending it in the README. The `pip install git+...#subdirectory=` flow is documented but unverified. Until validated, the supported install path is `git clone && maturin develop`.

2. **Boosterapp sidecar integration.** Already shipped — Boosterapp's `/api/info` v0.0.11 sidecar consumes `Wallet.from_seed` + `derive_child("peer/v1")` + `peer_id()` + `app_instance.derive` end-to-end on the K1. Locked vector validated.

## Smaller follow-ups

- **PyPI publication.** Defer until Boosterapp wants it. When we ship, use `maturin-action` in CI; abi3 keeps the wheel matrix small (one wheel per OS/arch, not per Python minor).
- **Type stubs (`auki_identity.pyi`).** Improves IDE autocomplete in downstream Python. Worth doing before any third-party consumer picks this up.
- **`__version__` attribute.** Standard Python convention; trivial to add.

## Open items

See [`../parking_lot.md`](../parking_lot.md). Forward-looking trade-offs only:

- PyPI distribution policy.
- Python version floor (currently 3.8 via abi3).

The cross-language locked-vector literal is filled in (was the original "next" item; resolved 2026-05-04). Async / Swarm bindings are not deferred to a future `auki-py` — they shipped as the sibling [`auki-network-py`](../../auki-network-py) crate per the per-component naming decision.

## Out of scope by design

- Async / Tokio / libp2p Swarm. Lives in the sibling [`auki-network-py`](../../auki-network-py) crate (per-component naming).
- `Wallet.sign` / `verify` / creation certs. Out of scope for the identity-py surface; the broader `auki-network-py` consumes signing internally for Vinland's Discovery `register` flow but doesn't re-expose the primitives — consumers that need raw signing wait for an `auki-jcs-py` or similar.
- WASM. Native Python extension only.

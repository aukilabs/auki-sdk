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

1. **Fill in the locked `peer_id` literal.** `python_tests/test_basic.py`'s `LOCKED_PEER_ID_FROM_SEED_03` is currently `None` — the test asserts shape only. Run `maturin develop && python -c "import auki_identity; print(auki_identity.Wallet.from_seed(b'\\x03'*32).derive_child('peer/v1').peer_id())"`, paste the result as a literal, push a follow-up. This makes the cross-language byte-for-byte assertion strict. Tracked in [`parking_lot.md`](../parking_lot.md).

2. **End-to-end-test the pip-from-Git install** before recommending it in the README. The `pip install git+...#subdirectory=` flow is documented but unverified. Until validated, the supported install path is `git clone && maturin develop`.

3. **Boosterapp sidecar integration.** First downstream consumer. Once Boosterapp's `/api/info` v0.0.11 implementation is in flight, validate the `peer_id` and `app_instance` values populated through this crate match what the Rust side produces for the same hardware + same seed file.

## Smaller follow-ups

- **PyPI publication.** Defer until Boosterapp wants it. When we ship, use `maturin-action` in CI; abi3 keeps the wheel matrix small (one wheel per OS/arch, not per Python minor).
- **Type stubs (`auki_identity.pyi`).** Improves IDE autocomplete in downstream Python. Worth doing before any third-party consumer picks this up.
- **`__version__` attribute.** Standard Python convention; trivial to add.

## Open items

See [`../parking_lot.md`](../parking_lot.md). Three items, all forward-looking:

- PyPI distribution policy.
- Locked-vector regeneration (the immediate next step above).
- Async / Swarm bindings (out of scope here; will be in the full `auki-py` crate).
- Python version floor (currently 3.8 via abi3).

## Out of scope by design

- Async / Tokio / libp2p Swarm. Lands in the full `auki-py` crate later.
- `Wallet.sign` / `verify` / creation certs. Same reasoning — full `auki-py` track.
- WASM. Native Python extension only.

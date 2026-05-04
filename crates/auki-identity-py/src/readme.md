# `auki-identity-py/src/`

PyO3 bindings for the three identity primitives Boosterapp's Python sidecar needs to implement `/api/info` v0.0.11. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`lib.rs`](lib.rs) — the entire binding. PyO3 0.22 `Bound<...>` API, `#[pymodule]` macro generates the C entry point Python imports.

## Public Python surface

```python
auki_identity.load_or_mint_seed(path: str) -> bytes        # 32 bytes
auki_identity.Wallet.from_seed(seed: bytes) -> Wallet
Wallet.derive_child(label: str) -> Wallet
Wallet.peer_id() -> str                                    # "12D3KooW…"
auki_identity.app_instance.derive() -> str                 # "aabbccddeeff"
```

That's it. No `sign`, no `verify`, no creation certs, no Swarm. Those land in the full `auki-py` crate later.

## How the wrappers map to the Rust crates

| Python | Rust |
|---|---|
| `load_or_mint_seed(path)` | `auki_identity::load_or_mint_seed(&Path)` |
| `Wallet.from_seed(seed)` | `auki_identity::Wallet::from_seed(&[u8; 32])` |
| `Wallet.derive_child(label)` | `auki_identity::Wallet::derive_child(&str)` |
| `Wallet.peer_id()` | `auki_network::PeerIdentity::from_seed(&wallet.seed()).peer_id().to_string()` |
| `app_instance.derive()` | `auki_network::app_instance::derive()` (requires `app_instance` feature) |

`Wallet.peer_id()` does **not** implicitly `derive_child("peer/v1")` — the caller does. This matches the upstream Rust contract: `PeerIdentity::from_wallet(&w)` is sugar for `from_seed(w.derive_child("peer/v1").seed())`, and the Python equivalent is `w.derive_child("peer/v1").peer_id()`. Documented on the method docstring.

## Error mapping

| Rust | Python |
|---|---|
| `auki_identity::SeedError::Io(_)` | `OSError` |
| `auki_identity::SeedError::InvalidLength(n)` | `ValueError` (message includes `n`) |
| `auki_network::app_instance::DeriveError::NoNetworkInterfaces` | `RuntimeError` (variant name in message) |
| `auki_network::app_instance::DeriveError::NoSuitableMac` | `RuntimeError` (variant name in message) |
| `auki_network::app_instance::DeriveError::Io(_)` | `OSError` |

The variant name is included in the `RuntimeError` message so callers can branch on container vs. laptop-with-only-private-Wi-Fi without re-coding the recipe.

## Build modes

The crate is dual-mode by design.

- **Python extension build** (`maturin develop` / `maturin build`) — `crate-type = ["cdylib"]` with PyO3's `extension-module` feature on. The host Python interpreter resolves runtime symbols at import time; the extension itself does not link Python. Maturin enables the feature via `[tool.maturin].features = ["pyo3/extension-module"]` in [`pyproject.toml`](../pyproject.toml).
- **Rust-side test build** (`cargo test`) — `crate-type` includes `rlib`; default Cargo features are *empty* so `extension-module` is off, and the dev-dep on `pyo3` with `auto-initialize` links a real Python runtime. This lets the smoke tests in [`lib.rs`](lib.rs)'s `#[cfg(test)] mod tests` use `Python::with_gil` without spinning up a host process.

The two feature modes are mutually exclusive — `extension-module` skips linking Python; `auto-initialize` requires it linked. The default-empty + maturin-enables-it pattern keeps both code paths working without manual feature flags. Standard PyO3 setup; see the [PyO3 user guide](https://pyo3.rs/v0.22.0/).

## Tests (5 Rust-side smoke + 13 Python-side)

### Rust-side (`lib.rs`)

| Test | Asserts |
|---|---|
| `module_builds_and_exposes_three_apis` | `auki_identity` PyModule builds; `load_or_mint_seed`, `Wallet`, `app_instance.derive` are all callable from Python |
| `wallet_from_seed_then_peer_id_is_deterministic` | Same seed → same `derive_child("peer/v1").peer_id()`; canonical `12D3KooW` prefix |
| `wallet_from_seed_rejects_wrong_length` | Non-32-byte seed → `ValueError` |
| `load_or_mint_seed_round_trip_via_pyo3_layer` | The `#[pyfunction]` entry point round-trips bytes correctly through tempdir |
| `locked_peer_id_vector` | Shape pin for `Wallet.from_seed(&[3u8; 32]).derive_child("peer/v1").peer_id()` — the cross-language locked vector |

### Python-side (`python_tests/test_basic.py`)

| Test | Asserts |
|---|---|
| `test_load_or_mint_seed_mints_when_missing` | First call mints, persists, returns the same 32 bytes that hit disk |
| `test_load_or_mint_seed_idempotent` | Second call returns identical bytes |
| `test_load_or_mint_seed_creates_parent_directories` | Multi-level missing parents are created |
| `test_load_or_mint_seed_rejects_wrong_length_with_value_error` | Wrong-length pre-existing file → `ValueError` |
| `test_load_or_mint_seed_accepts_existing_32_byte_file` | Pre-existing 32-byte file is read verbatim |
| `test_load_or_mint_seed_sets_0600_mode_on_unix` | Unix-only; persisted file has owner-r/w-only mode |
| `test_wallet_from_seed_then_derive_child_then_peer_id_round_trip` | The full `seed → wallet → derive_child("peer/v1") → peer_id` flow works; canonical `12D3KooW` prefix |
| `test_wallet_peer_id_is_deterministic` | Same seed → same `peer_id` |
| `test_wallet_peer_id_differs_across_seeds` | Different seeds → different `peer_id`s |
| `test_wallet_derive_child_differs_across_labels` | `peer/v1` and `app/boosterapp` produce different children |
| `test_wallet_from_seed_rejects_wrong_length` | Non-32-byte seed → `ValueError` |
| `test_app_instance_derive_returns_12_lowercase_hex_or_runtime_error` | Returns 12 lowercase hex chars on hardware, or accepts `RuntimeError` for container / private-Wi-Fi-only environments |
| `test_locked_peer_id_vector` | Shape pin + optional exact match against the cross-language locked literal |
| `test_module_exposes_only_documented_apis` | Pin the public surface — anything new requires a deliberate decision |

## Dependencies

- `auki-identity` (path) — wallet primitive + `load_or_mint_seed`. Renamed to `auki-identity-rs` in `Cargo.toml` via `package =` to avoid colliding with our own lib name `auki_identity` (which is also the Python module name).
- `auki-network` (path, `app_instance` feature) — `PeerIdentity::from_seed` for the libp2p PeerId encoding, plus `app_instance::derive`.
- `pyo3` 0.22 with `abi3-py38` — stable-ABI bindings, one wheel works on Python 3.8+.

Build-time:

- `maturin` 1.5+ — invoked through `pyproject.toml`'s `build-system`.

## Consumers

- **Boosterapp Python sidecar** — populates `/api/info`'s `peer_id` and `app_instance` fields, persists the seed at `~/.auki/boosterapp/identity.seed`. First and primary consumer.
- *(planned)* The full `auki-py` crate will subsume this one's surface and add Swarm + async on top. `auki-identity-py` continues as the lightweight identity-only package for sidecars that don't need the network layer.

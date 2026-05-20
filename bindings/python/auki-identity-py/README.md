# auki-identity-py

PyO3 bindings for a tiny slice of the Auki SDK — exactly three primitives:

1. `load_or_mint_seed(path)` — persist a 32-byte ed25519 seed across daemon restarts.
2. `Wallet.from_seed(seed).derive_child(label).peer_id()` — the canonical libp2p PeerId for a wallet, multibase-base58 (`12D3KooW…`).
3. `app_instance.derive()` — first non-loopback IEEE-administered MAC, 12 lowercase hex chars (`aabbccddeeff`).

## Why this exists

**The identity-only slice of the Python SDK.** Pure synchronous primitives — no GIL/Tokio dance, no async, no Swarm — for daemons that only need wallet, peer-id derivation, and `app_instance`. Boosterapp's Python sidecar uses it for the `/api/info` shape. The async / libp2p / streaming half lives in the sibling [`auki-network-py`](../auki-network-py) crate (per-component naming).

## Install

The crate is built as a native Python extension via [maturin](https://www.maturin.rs/). Install one of two ways:

**Editable / development build** — clone the SDK and `maturin develop` against the in-tree crate:

```bash
git clone https://github.com/aukilabs/auki-sdk.git
cd auki-sdk/bindings/python/auki-identity-py
python -m venv .venv && source .venv/bin/activate
pip install maturin
maturin develop --release
python -c "import auki_identity; print(auki_identity.app_instance.derive())"
```

**Pip-from-Git (subdirectory)** — once the SDK is tagged with this crate included, you can install directly:

```bash
pip install "git+https://github.com/aukilabs/auki-sdk.git@<tag>#subdirectory=bindings/python/auki-identity-py"
```

Pip's PEP 517 build will invoke maturin via the `pyproject.toml`'s `build-system`. A Rust toolchain (`rustup`) and a working C compiler are required at install time. We have not yet validated the subdirectory install end-to-end — until we do, the `maturin develop` flow above is the supported path.

## Usage

```python
import auki_identity

# 1. Persistent peer-key seed (32 bytes, atomic write, 0o600 on Unix).
seed = auki_identity.load_or_mint_seed("/var/lib/boosterapp/identity.seed")
assert len(seed) == 32

# 2. Wallet → derive_child → libp2p PeerId.
w = auki_identity.Wallet.from_seed(seed)
peer = w.derive_child("peer/v1")          # canonical "peer/v1" label
peer_id = peer.peer_id()                   # "12D3KooW…"
peer_seed = peer.seed()                    # 32-byte derived seed; useful when constructing lower-level peer identity

# 3. Per-machine identifier for /api/info.app_instance.
try:
    instance = auki_identity.app_instance.derive()  # "aabbccddeeff"
except RuntimeError as e:
    # Containers and laptops with only Private Wi-Fi enabled may have
    # no IEEE-administered MAC visible. Boosterapp's sidecar should
    # fall back to a wallet-derived stable id in that case.
    instance = "fallback"
```

The recipes match `auki-identity` and `auki-network` byte-for-byte — a Python sidecar and a Rust daemon sharing the same seed file produce the same `peer_id` and `app_instance`.

## What this is *not*

- **Not async.** No Tokio, no `asyncio` integration. Synchronous calls only.
- **Not the libp2p Swarm.** No dialing, no listening, no protocols — those live in [`auki-network-py`](../auki-network-py).
- **Not signing or verification.** No `Wallet.sign`, no `verify`, no creation certs. The broader [`auki-network-py`](../auki-network-py) consumes signing internally for the Vinland Discovery flow but doesn't re-expose the primitives.
- **Not a key store.** `load_or_mint_seed` is the entire defence on disk: file mode `0o600` on Unix, raw bytes. Stronger threat models wrap their own keystore around this primitive.
- **Not WASM.** Native Python extension; the WASM-friendly subset of `auki-identity` is not exposed here.

## Errors

| Function | Raises | When |
|---|---|---|
| `load_or_mint_seed(path)` | `OSError` | Filesystem error reading, writing, or creating directories. |
| `load_or_mint_seed(path)` | `ValueError` | `path` exists but is not exactly 32 bytes long. |
| `Wallet.from_seed(seed)` | `ValueError` | `seed` is not exactly 32 bytes. |
| `app_instance.derive()` | `RuntimeError` | `NoNetworkInterfaces` / `NoSuitableMac` — no enumerable interfaces, or every interface is loopback / locally-administered. Common in containers. |
| `app_instance.derive()` | `OSError` | Underlying `getifaddrs` / `GetAdaptersAddresses` syscall failed. |

## Tests

Two layers; both must pass before tagging.

```bash
# Rust-side smoke (links a real Python interpreter via pyo3's
# `auto-initialize` dev-feature; doesn't need maturin):
cargo test -p auki-identity-py

# Python-side end-to-end:
maturin develop
pytest python_tests/
```

The Python suite includes a **locked cross-language `peer_id` vector** — `Wallet.from_seed(b'\x03' * 32).derive_child("peer/v1").peer_id()` — that the parallel Rust agent's `auki-network` test asserts the same string for. If both pass, the bindings agree byte-for-byte with the Rust crate.

## Versioning

Pre-1.0 alongside the rest of the SDK. The Python surface is intentionally small and stable; consumers that need the libp2p / async surface import [`auki-network-py`](../auki-network-py) alongside this crate (per-component naming).

## Implementation status

See [`src/readme.md`](src/readme.md) for the per-file map and current status.

# Parking lot — auki-identity-py

Open questions specific to the Python bindings. Cross-cutting questions about the underlying primitives belong in [`auki-identity/parking_lot.md`](../auki-identity/parking_lot.md) or [`auki-network/parking_lot.md`](../auki-network/parking_lot.md).

---

## PyPI distribution

Today this crate is installed from Git only — `maturin develop` against the in-tree checkout for development, or `pip install git+https://github.com/aukilabs/auki-sdk.git@<tag>#subdirectory=crates/auki-identity-py` for downstream consumers (Boosterapp's sidecar). Both paths require a Rust toolchain at install time.

When we tag a release that includes this crate, do we want to publish prebuilt wheels to PyPI as well? Pros: zero-Rust install for downstream Python apps; faster CI. Cons: another release surface to maintain; need to set up a CI matrix building wheels for Linux x86_64 / aarch64 / macOS x86_64 / arm64 / Windows; need to coordinate version numbers between PyPI and the Git tag.

My instinct: defer until Boosterapp actually wants it. The Git-install path works for now. When we publish, use `maturin-action` in GitHub Actions; abi3 means one wheel per OS/arch (not per Python minor), which keeps the matrix small.

---

## Locked cross-language `peer_id` vector regeneration

The Python test [`test_locked_peer_id_vector`](python_tests/test_basic.py) has a placeholder `LOCKED_PEER_ID_FROM_SEED_03 = None` — it currently asserts shape only, not the exact base58 string. The plan is to compute the value once with `maturin develop && python -c "..."`, paste it as a literal, and let the test then assert exact match. The parallel agent landing the locked-vectors PR in `auki-network` will do the same on the Rust side; both should agree byte-for-byte.

Outstanding: someone with a Rust toolchain runs the snippet and lands a follow-up filling in `LOCKED_PEER_ID_FROM_SEED_03`. Parked because the agent that authored this crate could not execute Rust in its sandbox; flagged in the PR description.

---

## Async / Swarm bindings — **RESOLVED** (per-component, coexist)

The async / libp2p / streaming Python surface shipped as the sibling [`auki-network-py`](../auki-network-py) crate (per-component naming decision, 2026-05-05). The two crates coexist: `auki-identity-py` stays the lightweight identity-only package for sidecars that only need the data primitives; `auki-network-py` is for daemons that need the network layer. A consumer that wants both imports both. The original parking-lot question — "does `auki-identity-py` get folded into a future `auki-py`?" — was rejected in favour of per-component naming.

---

## Python version floor

We target `abi3-py38` — Python 3.8+. CPython 3.8 is in security-only support and will reach end-of-life in October 2024. Bumping to `abi3-py39` would let us drop a few legacy code paths and gain typing improvements; bumping to `abi3-py310` aligns with what most of our internal tooling already runs.

Defer until we have a concrete reason to drop 3.8 — likely when one of the underlying Rust deps drops something we depend on, or Boosterapp's sidecar moves to a newer interpreter.

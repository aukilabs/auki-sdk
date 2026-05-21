# Python Bindings

Python-facing SDK packages live here. The structure preserves the existing per-component package names: PyO3 wrappers sit beside the pure Python `auki-datatypes-py` package, but Rust SDK implementation crates remain in [`../../crates`](../../crates).

| Package | What it does |
|---|---|
| [`auki-datatypes-py`](auki-datatypes-py) | Pure Python betterproto bindings for `auki-datatypes` protobuf schemas. |
| [`auki-uniffi-test`](auki-uniffi-test) | UniFFI-generated Python package smoke path with bundled native libraries. |
| [`auki-identity-py`](auki-identity-py) | Wallet primitives and per-machine identity helpers. |
| [`auki-layout-py`](auki-layout-py) | Python wrappers for SDK-canonical path helpers. |
| [`auki-logs-py`](auki-logs-py) | PyO3 wrapper for the segmented log framing primitive. |
| [`auki-manifests-py`](auki-manifests-py) | Python wrappers for SDK-canonical log manifest builders. |
| [`auki-network-py`](auki-network-py) | Discovery value types plus shared stream payload and decision classes. |
| [`auki-domain-py`](auki-domain-py) | Python daemon facade for `ClusterManager`. |
| [`auki-registry-py`](auki-registry-py) | Python constructors and IO helpers for Sensor / Clock / Frame registries. |
| [`auki-session-py`](auki-session-py) | Scaffolded transport-neutral session lifecycle surface. |

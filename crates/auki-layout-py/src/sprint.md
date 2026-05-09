# Sprint — auki-layout-py

Current work and next steps. Spec: [outer `README.md`](../README.md).

## Now

Crate landed 2026-05-09 alongside [`auki-manifests-py`](../../auki-manifests-py) — pure-function wrappers around every path helper in [`auki-layout`](../../auki-layout). Closes the path-construction half of the [`detectors`](https://github.com/aukilabs/detectors) phase-2 Python ergonomics gap.

## Next

1. **Type stubs (`auki_layout.pyi`)** — track [`auki-network-py`](../../auki-network-py)'s parallel discussion. Surface is small, so low priority.
2. **PyPI distribution policy** — same question as `auki-identity-py` / `auki-network-py` / `auki-logs-py`. Defer until a non-source-build consumer needs the wheel.

## Out-of-band

- The path conventions themselves (substitution rules, directory names) live in [`auki-layout`](../../auki-layout). Changes there propagate here automatically — these wrappers don't introduce a second layer of conventions to reason about.

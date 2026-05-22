# auki-session-py

PyO3 bindings for `auki-session` — a transport-neutral, in-process Python surface for opening sessions, registering sensors and clocks, and writing / listing sensor and pose logs.

The aspirational shape is the **source-of-truth API** for SDK control-plane operations: both the [HTTP Control API](../../../docs/control-api.md) (frozen at SDK release v0.0.23) and the forthcoming libp2p control protocols (`/auki/control/info/0.0.1`, `/auki/control/sensor_logs/0.0.1`, …) are thin wrappers over this surface.

**Status:** WIP (scaffolding only) — the Python module is empty. First implementation gates on the `payload` encoding decision, tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5).

## Public surface

None yet — the `auki_session` Python module currently exports nothing.

## Depends on

- `pyo3` only. The Rust `auki-session` crate it will eventually wrap has not been written yet.

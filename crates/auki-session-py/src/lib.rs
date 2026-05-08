//! PyO3 bindings for `auki-session` — transport-neutral in-process
//! Python surface for opening sessions, registering sensors and
//! clocks, writing/listing sensor and pose logs.
//!
//! This is the **source-of-truth API** for SDK control-plane
//! operations. Both the [HTTP Control API](../../../docs/control-api.md)
//! (frozen at SDK release v0.0.23) and the forthcoming libp2p
//! control protocols (`/auki/control/info/0.0.1`,
//! `/auki/control/sensor_logs/0.0.1`, …) are thin wrappers over this
//! surface — every consumer-facing operation maps to a method here.
//!
//! **Status:** scaffolding only. The Python module is empty; first
//! implementation gates on the `payload` encoding decision in
//! [`parking_lot.md`](../parking_lot.md). See [`README.md`](../README.md)
//! for the aspirational surface (six design decisions resolved
//! 2026-05-07; two open questions remaining).

use pyo3::prelude::*;

#[pymodule]
fn auki_session(_py: Python<'_>, _m: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}

//! Single source of truth for the Auki SDK's shared cross-language data
//! types — the typed payload shapes that flow through logs and streams.
//!
//! The `.proto` files live in [`proto/`](../proto/); `build.rs` invokes
//! `prost-build` to generate Rust code into `OUT_DIR`, included here
//! one module per `.proto` package.
//!
//! Crate name names the **responsibility** (canonical shared data
//! types), not the implementation (protobuf via prost). Encoding could
//! change someday; the responsibility doesn't.
//!
//! See the [outer `README.md`](../README.md) for the spec, the
//! [`parking_lot.md`](../parking_lot.md) for open questions, and
//! [`src/readme.md`](readme.md) for current implementation status.
//!
//! **Status:** scaffolding only. The placeholder package below exists
//! to validate the `prost-build` pipeline end-to-end. Real packages
//! (`auki.camera`, `auki.pose`, `auki.audio`, etc.) land in subsequent
//! PRs — see [`src/sprint.md`](sprint.md).

#![allow(missing_docs, clippy::derive_partial_eq_without_eq)]

/// Placeholder package — pipeline-check only. Removed once the first
/// real schema lands.
pub mod placeholder {
    include!(concat!(env!("OUT_DIR"), "/auki.placeholder.rs"));
}

#[cfg(test)]
mod tests {
    use super::placeholder::PipelineCheck;
    use prost::Message;

    /// Smoke test that `prost-build` actually ran, the generated code
    /// compiled, and the encode/decode round-trip works. When the
    /// placeholder gets removed, this test goes with it — the real
    /// schemas have their own locked conformance vectors.
    #[test]
    fn placeholder_pipeline_check_round_trips() {
        let msg = PipelineCheck::default();
        let bytes = msg.encode_to_vec();
        let decoded = PipelineCheck::decode(&*bytes).expect("decode");
        assert_eq!(msg, decoded);
    }
}

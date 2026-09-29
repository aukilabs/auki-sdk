//! Components use existing registry clocks. All Component timestamp fields use
//! nanoseconds relative to the referenced clock's epoch, regardless of its native unit.
//! The host must retain/serve the referenced entry and convert native ticks to ns.
use crate::ClockReference;
pub use auki_registry::{ClockBody, ClockMeta, ClockRegistryEntry, Scope};

/// Check identity syntax; resolving the entry and verifying its hash is the host's responsibility.
pub fn validate_clock(clock: &ClockReference) -> Result<(), String> {
    if clock.peer_id.trim().is_empty()
        || (clock.hash.len() != 32
            || !clock
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    {
        return Err("clock requires a peer, registry id, and definition hash".into());
    }
    auki_registry::validate_registry_id(&clock.id).map_err(|e| e.to_string())
}

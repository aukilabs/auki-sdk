//! Mechanical runtime facade for authenticated Auki peers.
//!
//! The public facade is intentionally introduced in later commits. This first
//! slice owns the strict private DMS relay-booking HTTP boundary.

// This boundary is staged before the coordinator that consumes it.
#[allow(dead_code)]
mod relay;

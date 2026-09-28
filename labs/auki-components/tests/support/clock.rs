// Deterministic clock definition for isolated fixtures, never a production default.
pub fn fixture_clock(id: &str) -> auki_components::ClockReference {
    use auki_components::clock::{ClockBody, ClockMeta, ClockRegistryEntry, Scope};
    let entry = ClockRegistryEntry {
        peer_id: "fixture-clock-owner".into(),
        session_id: "fixture-boot-1".into(),
        clock_id: id.into(),
        body: ClockBody::MonotonicClock(ClockMeta {
            unit: "nanoseconds".into(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        }),
    };
    auki_components::ClockReference {
        peer_id: entry.peer_id.clone(),
        id: entry.clock_id.clone(),
        hash: entry.hash(),
    }
}

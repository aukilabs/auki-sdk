use auki_time::{
    ClockSyncObservation, ClockSyncState, DomainClockDescriptor, NtpExchange, SessionClock,
    compute_ntp_sample, estimate_domain_clock,
};

#[test]
fn rust_root_api_remains_source_compatible() {
    let clock = SessionClock::new("12D3KooWPeerExample", "session-123", "monotonic");
    assert_eq!(
        clock.clock_id(),
        "12D3KooWPeerExample/session-123/monotonic"
    );
    assert!(!clock.clock_hash().is_empty());

    let sample = compute_ntp_sample(NtpExchange {
        local_send_ns: 1_000,
        remote_receive_ns: 1_001_050,
        remote_send_ns: 1_001_080,
        local_receive_ns: 1_130,
    })
    .unwrap();
    assert_eq!(sample.offset_ns, 1_000_000);
    assert_eq!(sample.uncertainty_ns, 100);

    let mut sync = ClockSyncState::default();
    let estimate = sync
        .observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-b/session-7/monotonic",
            "hash-b",
            sample,
        ))
        .unwrap();

    let domain = estimate_domain_clock(
        estimate,
        DomainClockDescriptor::new(
            "cluster-a",
            "cluster-a/domain-clock",
            "domain-hash",
            "12D3PeerB",
            "peer-b/session-7/monotonic",
            "hash-b",
            250,
        ),
    )
    .unwrap();

    assert_eq!(domain.total_offset_ns, 1_000_250);
    assert_eq!(domain.time_transform().convert_ns(1_130), Some(1_001_380));
}

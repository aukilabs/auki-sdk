use auki_components::*;
use std::sync::Arc;
#[test]
fn brackets_lease_adjacent_samples_and_respect_eviction() {
    let b = Buffer::new("history", 2).unwrap();
    assert!(b.bracket_time_ns(1).unwrap().before.is_none());
    for (seq, t) in [(0, 10), (1, 20)] {
        b.append_shared(Arc::new(Envelope::new(seq, t, t))).unwrap();
    }
    let exact = b.bracket_time_ns(10).unwrap();
    assert!(Arc::ptr_eq(
        exact.before.as_ref().unwrap(),
        exact.after.as_ref().unwrap()
    ));
    let between = b.bracket_time_ns(15).unwrap();
    assert_eq!(between.before.unwrap().sequence, 0);
    assert_eq!(between.after.unwrap().sequence, 1);
    assert!(b.bracket_time_ns(9).unwrap().before.is_none());
    assert!(b.bracket_time_ns(21).unwrap().after.is_none());
    b.append_shared(Arc::new(Envelope::new(2, 30, 30))).unwrap();
    assert!(b.bracket_time_ns(15).unwrap().before.is_none());
    assert_eq!(exact.before.unwrap().timestamp_ns, 10); // lease survives eviction
}
#[test]
fn ambiguous_timestamp_policies_cannot_be_bracketed() {
    for policy in [
        SourceTimestampPolicy::NonDecreasing,
        SourceTimestampPolicy::Unordered,
    ] {
        let b = Buffer::<u64>::with_limits_and_time_policy(
            "history",
            BufferLimits::entries(2),
            BufferTimePolicy {
                source_timestamps: policy,
                duration_basis: DurationTimeBasis::ArrivalTime,
            },
            |_| 8,
        )
        .unwrap();
        assert!(matches!(
            b.bracket_time_ns(1),
            Err(BufferError::AmbiguousTimeOrdering)
        ));
    }
}

//! Target-neutral decisions shared by the native and browser peer runtimes.
//!
//! Executors, transports, and cleanup actors deliberately remain
//! target-specific. This module only owns decisions which must not drift
//! between those implementations.

use std::time::Duration;

use auki_relay_booking::{RelayBookingMode, RelayBookingSnapshot, RelayBookingState};
use chrono::{DateTime, Utc};

use crate::{AukiRelayConfig, AukiRelayMode};

pub(crate) const RELAY_AUTHORITY_SAFETY_MARGIN: Duration = Duration::from_secs(20);
pub(crate) const RELAY_STARTUP_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RejectedAuthorityRevision {
    AlreadyReplaced,
    Current,
    Stale,
}

pub(crate) fn rejected_authority_revision(
    current: u64,
    rejected: u64,
) -> RejectedAuthorityRevision {
    if current > rejected {
        RejectedAuthorityRevision::AlreadyReplaced
    } else if current == rejected {
        RejectedAuthorityRevision::Current
    } else {
        RejectedAuthorityRevision::Stale
    }
}

pub(crate) fn next_authority_revision(current: Option<u64>) -> Option<u64> {
    match current {
        Some(current) => current.checked_add(1),
        None => Some(1),
    }
}

pub(crate) fn booking_mode(policy: AukiRelayConfig) -> RelayBookingMode {
    match policy.mode {
        AukiRelayMode::Public => RelayBookingMode::Public,
        AukiRelayMode::Dedicated => RelayBookingMode::Dedicated,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveBookingValidation {
    Match,
    PolicyMismatch,
    Ended,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RelayBookingExpectation {
    pub(crate) mode: RelayBookingMode,
    pub(crate) requested_duration_seconds: u64,
    pub(crate) relay_count: u8,
}

impl From<AukiRelayConfig> for RelayBookingExpectation {
    fn from(policy: AukiRelayConfig) -> Self {
        Self {
            mode: booking_mode(policy),
            requested_duration_seconds: policy.requested_duration.as_secs(),
            relay_count: policy.relay_count,
        }
    }
}

pub(crate) fn validate_active_booking(
    snapshot: &RelayBookingSnapshot,
    expected: RelayBookingExpectation,
) -> ActiveBookingValidation {
    if snapshot.mode != expected.mode
        || snapshot.requested_duration_seconds != expected.requested_duration_seconds
        || snapshot.relay_count != expected.relay_count
    {
        ActiveBookingValidation::PolicyMismatch
    } else if snapshot.state != RelayBookingState::Active {
        ActiveBookingValidation::Ended
    } else {
        ActiveBookingValidation::Match
    }
}

pub(crate) fn relay_authorized_until(
    requested_until: DateTime<Utc>,
    authority_expires_at: DateTime<Utc>,
    provider_lease_expires_at: DateTime<Utc>,
) -> DateTime<Utc> {
    requested_until
        .min(authority_expires_at)
        .min(provider_lease_expires_at)
        - chrono::Duration::from_std(RELAY_AUTHORITY_SAFETY_MARGIN)
            .expect("the fixed relay safety margin fits chrono")
}

pub(crate) fn cap_relay_renewal_delay(
    remaining: Duration,
    preferred: Duration,
    http_timeout: Duration,
) -> Duration {
    preferred.min(
        remaining
            .saturating_sub(RELAY_AUTHORITY_SAFETY_MARGIN)
            .saturating_sub(http_timeout),
    )
}

pub(crate) fn cap_relay_status_poll_delay(
    desired: Duration,
    usable_until: DateTime<Utc>,
    now: DateTime<Utc>,
    http_timeout: Duration,
) -> Duration {
    let deadline_delay = usable_until
        .signed_duration_since(now)
        .to_std()
        .unwrap_or_default()
        .saturating_sub(http_timeout);
    desired.min(deadline_delay).max(Duration::from_millis(1))
}

#[cfg(test)]
mod tests {
    use auki_relay_booking::{RelayBookingSnapshot, RelaySlotState};
    use uuid::Uuid;

    use super::*;

    fn snapshot(policy: AukiRelayConfig) -> RelayBookingSnapshot {
        let now = Utc::now();
        RelayBookingSnapshot {
            booking_id: Uuid::new_v4(),
            mode: booking_mode(policy),
            state: RelayBookingState::Active,
            relay_count: policy.relay_count,
            requested_duration_seconds: policy.requested_duration.as_secs(),
            requested_until: now + chrono::Duration::hours(1),
            authority_expires_at: now + chrono::Duration::minutes(5),
            assigned_count: 0,
            provider_ready_count: 0,
            unfilled_count: policy.relay_count,
            created_at: now,
            ended_at: None,
            slots: (0..policy.relay_count)
                .map(|index| auki_relay_booking::RelaySlotSnapshot {
                    slot_id: Uuid::new_v4(),
                    slot_index: index,
                    state: RelaySlotState::Queued,
                    assignment_id: None,
                    reservation_epoch: None,
                    provider_peer_id: None,
                    provider_base_addresses: None,
                    limits: None,
                    provider_lease_expires_at: None,
                    recovery_expires_at: None,
                })
                .collect(),
        }
    }

    #[test]
    fn rejected_authority_revision_has_one_shared_fence() {
        assert_eq!(
            rejected_authority_revision(3, 2),
            RejectedAuthorityRevision::AlreadyReplaced
        );
        assert_eq!(
            rejected_authority_revision(3, 3),
            RejectedAuthorityRevision::Current
        );
        assert_eq!(
            rejected_authority_revision(3, 4),
            RejectedAuthorityRevision::Stale
        );
        assert_eq!(next_authority_revision(None), Some(1));
        assert_eq!(next_authority_revision(Some(3)), Some(4));
        assert_eq!(next_authority_revision(Some(u64::MAX)), None);
    }

    #[test]
    fn booking_validation_distinguishes_policy_drift_from_ended_authority() {
        let policy = AukiRelayConfig::default();
        let mut current = snapshot(policy);
        assert_eq!(
            validate_active_booking(&current, policy.into()),
            ActiveBookingValidation::Match
        );

        current.requested_duration_seconds += 1;
        assert_eq!(
            validate_active_booking(&current, policy.into()),
            ActiveBookingValidation::PolicyMismatch
        );
        current.requested_duration_seconds = policy.requested_duration.as_secs();
        current.state = RelayBookingState::Canceled;
        assert_eq!(
            validate_active_booking(&current, policy.into()),
            ActiveBookingValidation::Ended
        );
    }

    #[test]
    fn relay_deadline_and_renewal_cap_share_one_safety_margin() {
        let now = Utc::now();
        let authorized_until = relay_authorized_until(
            now + chrono::Duration::minutes(10),
            now + chrono::Duration::minutes(8),
            now + chrono::Duration::minutes(6),
        );
        assert_eq!(
            authorized_until,
            now + chrono::Duration::minutes(6)
                - chrono::Duration::from_std(RELAY_AUTHORITY_SAFETY_MARGIN).unwrap()
        );

        assert_eq!(
            cap_relay_renewal_delay(
                Duration::from_secs(40),
                Duration::from_secs(30),
                Duration::from_secs(10),
            ),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn relay_status_poll_respects_desired_cadence_and_live_deadline() {
        let now = Utc::now();
        assert_eq!(
            cap_relay_status_poll_delay(
                Duration::from_secs(30),
                now + chrono::Duration::seconds(60),
                now,
                Duration::from_secs(10),
            ),
            Duration::from_secs(30)
        );
        assert_eq!(
            cap_relay_status_poll_delay(
                Duration::from_secs(30),
                now + chrono::Duration::seconds(25),
                now,
                Duration::from_secs(10),
            ),
            Duration::from_secs(15)
        );
        assert_eq!(
            cap_relay_status_poll_delay(
                Duration::from_secs(30),
                now + chrono::Duration::seconds(5),
                now,
                Duration::from_secs(10),
            ),
            Duration::from_millis(1)
        );
    }
}

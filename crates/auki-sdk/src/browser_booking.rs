use std::{str::FromStr, time::Duration};

use auki_p2p::{
    ExpectedRelayLimits, PeerId, RelayBaseTransport, RelayProvider, RelayReservationError,
};
use auki_relay_booking::{RELAY_HTTP_REQUEST_TIMEOUT, RelayBookingSnapshot, RelaySlotState};
use uuid::Uuid;

use crate::{
    AukiRelayConfig,
    runtime_policy::{
        ActiveBookingValidation, RELAY_AUTHORITY_SAFETY_MARGIN, cap_relay_renewal_delay,
        relay_authorized_until, validate_active_booking,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RelayFence {
    pub(crate) slot_id: Uuid,
    pub(crate) assignment_id: Uuid,
    pub(crate) reservation_epoch: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadyRelay {
    pub(crate) booking_id: Uuid,
    pub(crate) requested_until: chrono::DateTime<chrono::Utc>,
    pub(crate) authority_expires_at: chrono::DateTime<chrono::Utc>,
    pub(crate) provider_lease_expires_at: chrono::DateTime<chrono::Utc>,
    pub(crate) fence: RelayFence,
    pub(crate) provider: RelayProvider,
}

pub(crate) fn ready_relay(
    snapshot: &RelayBookingSnapshot,
    policy: AukiRelayConfig,
) -> Result<Option<ReadyRelay>, ReadyRelayError> {
    validate_policy(snapshot, policy)?;
    let slot = snapshot
        .slots
        .first()
        .ok_or(ReadyRelayError::Invalid("booking omitted its relay slot"))?;
    if slot.state != RelaySlotState::Ready {
        return Ok(None);
    }

    let assignment_id = slot.assignment_id.ok_or(ReadyRelayError::Invalid(
        "ready relay omitted its assignment ID",
    ))?;
    let reservation_epoch = slot.reservation_epoch.ok_or(ReadyRelayError::Invalid(
        "ready relay omitted its reservation epoch",
    ))?;
    let provider_lease_expires_at =
        slot.provider_lease_expires_at
            .ok_or(ReadyRelayError::Invalid(
                "ready relay omitted its provider lease deadline",
            ))?;
    let relay_peer_id = PeerId::from_str(slot.provider_peer_id.as_deref().ok_or(
        ReadyRelayError::Invalid("ready relay omitted its provider Peer ID"),
    )?)
    .map_err(|_| ReadyRelayError::Invalid("ready relay has an invalid provider Peer ID"))?;
    let bases = slot
        .provider_base_addresses
        .as_ref()
        .ok_or(ReadyRelayError::Invalid(
            "ready relay omitted its provider bases",
        ))?;
    let limits = slot.limits.ok_or(ReadyRelayError::Invalid(
        "ready relay omitted its finite limits",
    ))?;
    let expected_limits = ExpectedRelayLimits::new(
        Duration::from_secs(u64::from(limits.duration_seconds)),
        limits.data_bytes_per_direction,
    )?;
    let provider = RelayProvider::new_for_transport(
        relay_peer_id,
        bases,
        RelayBaseTransport::Wss,
        expected_limits,
    )?;

    Ok(Some(ReadyRelay {
        booking_id: snapshot.booking_id,
        requested_until: snapshot.requested_until,
        authority_expires_at: snapshot.authority_expires_at,
        provider_lease_expires_at,
        fence: RelayFence {
            slot_id: slot.slot_id,
            assignment_id,
            reservation_epoch,
        },
        provider,
    }))
}

pub(crate) fn matches_ready_relay(
    pinned: &ReadyRelay,
    snapshot: &RelayBookingSnapshot,
    policy: AukiRelayConfig,
) -> Result<bool, ReadyRelayError> {
    Ok(ready_relay(snapshot, policy)?.is_some_and(|current| {
        current.booking_id == pinned.booking_id
            && current.fence == pinned.fence
            && current.provider == pinned.provider
    }))
}

pub(crate) fn relay_usable_until(ready: &ReadyRelay) -> chrono::DateTime<chrono::Utc> {
    relay_authorized_until(
        ready.requested_until,
        ready.authority_expires_at,
        ready.provider_lease_expires_at,
    )
}

pub(crate) fn relay_renewal_start_deadline(ready: &ReadyRelay) -> chrono::DateTime<chrono::Utc> {
    ready.authority_expires_at
        - chrono::Duration::from_std(RELAY_AUTHORITY_SAFETY_MARGIN + RELAY_HTTP_REQUEST_TIMEOUT)
            .expect("the fixed browser relay renewal margin fits chrono")
}

pub(crate) fn booking_renewal_delay_at(
    ready: &ReadyRelay,
    now: chrono::DateTime<chrono::Utc>,
) -> Duration {
    let remaining = ready
        .authority_expires_at
        .signed_duration_since(now)
        .to_std()
        .unwrap_or_default();
    let preferred = remaining / 4;
    cap_relay_renewal_delay(remaining, preferred, RELAY_HTTP_REQUEST_TIMEOUT)
}

pub(crate) fn pull_booking_renewal_forward(
    scheduled: chrono::DateTime<chrono::Utc>,
    ready: &ReadyRelay,
    now: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    let candidate = now
        + chrono::Duration::from_std(booking_renewal_delay_at(ready, now))
            .expect("the bounded booking renewal delay fits chrono");
    scheduled.min(candidate)
}

fn validate_policy(
    snapshot: &RelayBookingSnapshot,
    policy: AukiRelayConfig,
) -> Result<(), ReadyRelayError> {
    match validate_active_booking(snapshot, policy.into()) {
        ActiveBookingValidation::Match => Ok(()),
        ActiveBookingValidation::PolicyMismatch | ActiveBookingValidation::Ended => Err(
            ReadyRelayError::Invalid("active relay booking does not match the browser policy"),
        ),
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadyRelayError {
    #[error("invalid DMS relay snapshot: {0}")]
    Invalid(&'static str),
    #[error("invalid relay provider: {0}")]
    Provider(#[from] RelayReservationError),
}

#[cfg(test)]
mod tests {
    use auki_p2p::Identity;
    use auki_relay_booking::{RelayBookingState, RelayLimits, RelaySlotSnapshot};
    use chrono::Duration as ChronoDuration;

    use super::*;
    use crate::runtime_policy::booking_mode;

    fn policy() -> AukiRelayConfig {
        AukiRelayConfig::default()
    }

    fn snapshot(slot_state: RelaySlotState) -> RelayBookingSnapshot {
        let now = chrono::Utc::now();
        let relay = Identity::generate().peer_id();
        let assigned = slot_state == RelaySlotState::Ready;
        RelayBookingSnapshot {
            booking_id: Uuid::new_v4(),
            mode: booking_mode(policy()),
            state: RelayBookingState::Active,
            relay_count: policy().relay_count,
            requested_duration_seconds: policy().requested_duration.as_secs(),
            requested_until: now + ChronoDuration::hours(24),
            authority_expires_at: now + ChronoDuration::minutes(5),
            assigned_count: u8::from(assigned),
            provider_ready_count: u8::from(assigned),
            unfilled_count: u8::from(!assigned),
            created_at: now,
            ended_at: None,
            slots: vec![RelaySlotSnapshot {
                slot_id: Uuid::new_v4(),
                slot_index: 0,
                state: slot_state,
                assignment_id: assigned.then(Uuid::new_v4),
                reservation_epoch: assigned.then(Uuid::new_v4),
                provider_peer_id: assigned.then(|| relay.to_string()),
                provider_base_addresses: assigned.then(|| {
                    vec![
                        format!("/dns4/relay.example.com/tcp/443/p2p/{relay}"),
                        format!("/dns4/relay.example.com/tcp/4443/wss/p2p/{relay}"),
                    ]
                }),
                limits: assigned.then_some(RelayLimits {
                    duration_seconds: 900,
                    data_bytes_per_direction: 10_737_418_240,
                }),
                provider_lease_expires_at: assigned.then_some(now + ChronoDuration::minutes(3)),
                recovery_expires_at: None,
            }],
        }
    }

    #[test]
    fn selects_one_ready_wss_provider_with_native_counterpart() {
        let snapshot = snapshot(RelaySlotState::Ready);
        let ready = ready_relay(&snapshot, policy()).unwrap().unwrap();
        let target = Identity::generate().peer_id();

        assert_eq!(ready.booking_id, snapshot.booking_id);
        assert_eq!(
            ready.provider_lease_expires_at,
            snapshot.slots[0].provider_lease_expires_at.unwrap()
        );
        assert!(
            ready
                .provider
                .circuit_route_for_transport(RelayBaseTransport::Wss, target)
                .unwrap()
                .to_string()
                .contains("/wss/")
        );
        assert!(
            ready
                .provider
                .circuit_route_for_transport(RelayBaseTransport::Tcp, target)
                .is_ok()
        );
    }

    #[test]
    fn requires_wss_but_keeps_tcp_optional() {
        let mut wss_only = snapshot(RelaySlotState::Ready);
        wss_only.slots[0]
            .provider_base_addresses
            .as_mut()
            .unwrap()
            .remove(0);
        let ready = ready_relay(&wss_only, policy()).unwrap().unwrap();
        assert!(
            ready
                .provider
                .base_for_transport(RelayBaseTransport::Wss)
                .is_some()
        );
        assert!(
            ready
                .provider
                .base_for_transport(RelayBaseTransport::Tcp)
                .is_none()
        );

        let mut tcp_only = snapshot(RelaySlotState::Ready);
        tcp_only.slots[0]
            .provider_base_addresses
            .as_mut()
            .unwrap()
            .remove(1);
        assert!(ready_relay(&tcp_only, policy()).is_err());
    }

    #[test]
    fn every_non_ready_slot_stays_unpublishable() {
        for state in [
            RelaySlotState::Queued,
            RelaySlotState::Starting,
            RelaySlotState::Recovering,
            RelaySlotState::Reassigning,
            RelaySlotState::Ended,
        ] {
            assert!(ready_relay(&snapshot(state), policy()).unwrap().is_none());
        }
    }

    #[test]
    fn booking_fence_or_provider_replacement_breaks_the_pin() {
        let original = snapshot(RelaySlotState::Ready);
        let pinned = ready_relay(&original, policy()).unwrap().unwrap();
        let mut replacements = Vec::new();

        let mut booking = original.clone();
        booking.booking_id = Uuid::new_v4();
        replacements.push(booking);

        let mut fence = original.clone();
        fence.slots[0].reservation_epoch = Some(Uuid::new_v4());
        replacements.push(fence);

        let mut limits = original.clone();
        limits.slots[0].limits.as_mut().unwrap().duration_seconds += 1;
        replacements.push(limits);

        let mut provider = original.clone();
        let relay = Identity::generate().peer_id();
        provider.slots[0].provider_peer_id = Some(relay.to_string());
        provider.slots[0].provider_base_addresses = Some(vec![format!(
            "/dns4/other-relay.example.com/tcp/4443/wss/p2p/{relay}"
        )]);
        replacements.push(provider);

        for replacement in replacements {
            assert!(!matches_ready_relay(&pinned, &replacement, policy()).unwrap());
        }

        let mut refreshed_deadlines = original;
        refreshed_deadlines.authority_expires_at += ChronoDuration::minutes(1);
        refreshed_deadlines.slots[0].provider_lease_expires_at = refreshed_deadlines.slots[0]
            .provider_lease_expires_at
            .map(|deadline| deadline + ChronoDuration::minutes(1));
        assert!(matches_ready_relay(&pinned, &refreshed_deadlines, policy()).unwrap());
    }

    #[test]
    fn renewal_delay_uses_quarter_life_without_crossing_the_safety_margin() {
        let now = chrono::Utc::now();
        let mut ready = ready_relay(&snapshot(RelaySlotState::Ready), policy())
            .unwrap()
            .unwrap();

        ready.authority_expires_at = now + ChronoDuration::seconds(100);
        assert_eq!(
            booking_renewal_delay_at(&ready, now),
            Duration::from_secs(25)
        );

        ready.authority_expires_at = now + ChronoDuration::seconds(35);
        assert_eq!(
            booking_renewal_delay_at(&ready, now),
            Duration::from_secs(5)
        );

        ready.authority_expires_at = now + ChronoDuration::seconds(24);
        assert_eq!(booking_renewal_delay_at(&ready, now), Duration::ZERO);

        ready.authority_expires_at = now - ChronoDuration::seconds(1);
        assert_eq!(booking_renewal_delay_at(&ready, now), Duration::ZERO);
    }

    #[test]
    fn usable_deadline_uses_the_earliest_authority_with_safety_margin() {
        let now = chrono::Utc::now();
        let mut ready = ready_relay(&snapshot(RelaySlotState::Ready), policy())
            .unwrap()
            .unwrap();
        ready.requested_until = now + ChronoDuration::seconds(300);
        ready.authority_expires_at = now + ChronoDuration::seconds(200);
        ready.provider_lease_expires_at = now + ChronoDuration::seconds(100);

        assert_eq!(
            relay_usable_until(&ready),
            now + ChronoDuration::seconds(80)
        );
        assert_eq!(
            relay_renewal_start_deadline(&ready),
            now + ChronoDuration::seconds(170)
        );
    }

    #[test]
    fn an_earlier_snapshot_deadline_pulls_renewal_forward_only() {
        let now = chrono::Utc::now();
        let scheduled = now + ChronoDuration::seconds(60);
        let mut ready = ready_relay(&snapshot(RelaySlotState::Ready), policy())
            .unwrap()
            .unwrap();

        ready.authority_expires_at = now + ChronoDuration::seconds(35);
        assert_eq!(
            pull_booking_renewal_forward(scheduled, &ready, now),
            now + ChronoDuration::seconds(5)
        );

        ready.authority_expires_at = now + ChronoDuration::seconds(300);
        assert_eq!(
            pull_booking_renewal_forward(scheduled, &ready, now),
            scheduled
        );
    }
}

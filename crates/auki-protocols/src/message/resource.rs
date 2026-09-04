//! Canonical identity of one receiver-owned typed-message channel.

#![forbid(unsafe_code)]

use auki_registry::RegistryRef;
use libp2p_identity::PeerId;

/// Atomic identity advertised by the resource catalog and opened by Message v1.
///
/// Message transport has no storage path or persistence configuration:
/// registration binds only this identity and clock to a bounded live receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageChannelResource {
    /// Peer that owns and receives the channel.
    pub owner_peer_id: PeerId,
    /// Resource identifier scoped to `owner_peer_id`.
    pub resource_id: String,
    /// Clock declaration defining the meaning of message timestamps.
    pub clock: RegistryRef,
}

impl MessageChannelResource {
    /// Validate the channel identity and clock reference.
    pub fn validate(&self) -> Result<(), MessageChannelResourceError> {
        if self.resource_id.is_empty() {
            return Err(MessageChannelResourceError::EmptyResourceId);
        }
        if self.clock.peer_id.is_empty() || self.clock.id.is_empty() || self.clock.hash.is_empty() {
            return Err(MessageChannelResourceError::EmptyClockReference);
        }
        self.clock
            .peer_id
            .parse::<PeerId>()
            .map_err(|_| MessageChannelResourceError::InvalidClockPeerId)?;
        Ok(())
    }
}

/// Validation failure for one [`MessageChannelResource`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MessageChannelResourceError {
    /// The receiver-scoped resource identifier is empty.
    #[error("message channel resource_id is empty")]
    EmptyResourceId,
    /// At least one clock reference field is empty.
    #[error("message channel clock RegistryRef contains an empty field")]
    EmptyClockReference,
    /// The clock owner is not a valid libp2p peer identity.
    #[error("message channel clock peer_id is not a valid PeerId")]
    InvalidClockPeerId,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }

    fn resource() -> MessageChannelResource {
        let owner_peer_id = peer();
        MessageChannelResource {
            owner_peer_id,
            resource_id: "events".into(),
            clock: RegistryRef {
                peer_id: owner_peer_id.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        }
    }

    #[test]
    fn validates_only_complete_channel_identity_and_clock() {
        assert_eq!(resource().validate(), Ok(()));

        let mut invalid = resource();
        invalid.resource_id.clear();
        assert_eq!(
            invalid.validate(),
            Err(MessageChannelResourceError::EmptyResourceId)
        );

        let mut invalid = resource();
        invalid.clock.hash.clear();
        assert_eq!(
            invalid.validate(),
            Err(MessageChannelResourceError::EmptyClockReference)
        );

        let mut invalid = resource();
        invalid.clock.peer_id = "not-a-peer".into();
        assert_eq!(
            invalid.validate(),
            Err(MessageChannelResourceError::InvalidClockPeerId)
        );
    }
}

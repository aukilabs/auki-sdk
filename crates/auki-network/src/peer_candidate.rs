//! Experimental peer-candidate helpers for local decentralized discovery.
//!
//! This module intentionally models a *candidate* pipeline, not authority.
//! A stored candidate can become an [`AllowedPeer`] dial input for the
//! transport runtime, but it does not prove membership, trust, served-domain
//! state, offer authority, payload correctness, or local-policy acceptance.
//! Callers must still perform post-connection identity/domain/offer/policy
//! validation before treating the peer as usable.

use crate::network_runtime::AllowedPeer;
use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};
use std::{collections::HashMap, time::Duration};

/// Experimental source label for a non-authoritative peer candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeerCandidateSource {
    /// Locally configured by an operator or local application policy.
    Configured,
    /// Learned from an already-connected peer's advertisement.
    ConnectedPeerAdvertisement,
}

impl PeerCandidateSource {
    /// Stable source marker used in logs/status output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::ConnectedPeerAdvertisement => "connected_peer_advertisement",
        }
    }
}

/// Experimental Auki peer candidate v1.
///
/// Domain and capability hints remain informational. They are kept on the
/// candidate only so validators can reject obviously non-authoritative attempts
/// to smuggle acceptance through discovery metadata. Converting a candidate to
/// [`AllowedPeer`] is transport eligibility for dialing only; it is not an
/// authority, membership, or application-use promotion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AukiPeerCandidateV1 {
    pub source: PeerCandidateSource,
    pub peer_id: PeerId,
    pub addrs: Vec<Multiaddr>,
    pub observed_at_ms: u64,
    pub expires_at_ms: u64,
    pub domain_hints: Vec<String>,
    pub capability_hints: Vec<String>,
}

impl AukiPeerCandidateV1 {
    /// Construct a candidate with a relative TTL.
    pub fn with_ttl(
        source: PeerCandidateSource,
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
        observed_at_ms: u64,
        ttl: Duration,
    ) -> Self {
        let ttl_ms = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX);
        Self {
            source,
            peer_id,
            addrs,
            observed_at_ms,
            expires_at_ms: observed_at_ms.saturating_add(ttl_ms),
            domain_hints: Vec::new(),
            capability_hints: Vec::new(),
        }
    }

    /// Return the transport dial input represented by this candidate.
    ///
    /// This is only for connection establishment. Higher layers must not treat
    /// the resulting [`AllowedPeer`] as membership, trust, domain authority,
    /// offer authority, or local-policy acceptance.
    pub fn to_allowed_peer(&self) -> AllowedPeer {
        AllowedPeer {
            peer_id: self.peer_id,
            multiaddrs: self.addrs.clone(),
        }
    }
}

/// Rejection reasons with stable marker strings for QA and logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerCandidateRejectReason {
    UnsupportedSource { source: String },
    EmptyAddrs,
    Expired { now_ms: u64, expires_at_ms: u64 },
    AddrPeerIdMismatch { expected: PeerId, actual: PeerId },
    MissingTerminalPeerId { addr: Multiaddr },
    NonAuthoritativeDomainHints,
    NonAuthoritativeCapabilityHints,
}

impl PeerCandidateRejectReason {
    pub fn marker(&self) -> &'static str {
        match self {
            Self::UnsupportedSource { .. } => "unsupported_source",
            Self::EmptyAddrs => "empty_addrs",
            Self::Expired { .. } => "expired",
            Self::AddrPeerIdMismatch { .. } => "peer_id_mismatch",
            Self::MissingTerminalPeerId { .. } => "missing_terminal_peer_id",
            Self::NonAuthoritativeDomainHints => "non_authoritative_domain_hints",
            Self::NonAuthoritativeCapabilityHints => "non_authoritative_capability_hints",
        }
    }
}

/// Parse a wire/source string into an experimental candidate source.
pub fn parse_peer_candidate_source(
    source: &str,
) -> Result<PeerCandidateSource, PeerCandidateRejectReason> {
    match source {
        "configured" => Ok(PeerCandidateSource::Configured),
        "connected_peer_advertisement" => Ok(PeerCandidateSource::ConnectedPeerAdvertisement),
        other => Err(PeerCandidateRejectReason::UnsupportedSource {
            source: other.to_string(),
        }),
    }
}

/// Validate one candidate as a non-authoritative dial candidate.
pub fn validate_peer_candidate(
    candidate: &AukiPeerCandidateV1,
    now_ms: u64,
) -> Result<(), PeerCandidateRejectReason> {
    if candidate.expires_at_ms <= now_ms {
        return Err(PeerCandidateRejectReason::Expired {
            now_ms,
            expires_at_ms: candidate.expires_at_ms,
        });
    }
    if candidate.addrs.is_empty() {
        return Err(PeerCandidateRejectReason::EmptyAddrs);
    }
    if !candidate.domain_hints.is_empty() {
        return Err(PeerCandidateRejectReason::NonAuthoritativeDomainHints);
    }
    if !candidate.capability_hints.is_empty() {
        return Err(PeerCandidateRejectReason::NonAuthoritativeCapabilityHints);
    }

    for addr in &candidate.addrs {
        match terminal_peer_id(addr) {
            Some(actual) if actual == candidate.peer_id => {}
            Some(actual) => {
                return Err(PeerCandidateRejectReason::AddrPeerIdMismatch {
                    expected: candidate.peer_id,
                    actual,
                });
            }
            None => {
                return Err(PeerCandidateRejectReason::MissingTerminalPeerId {
                    addr: addr.clone(),
                });
            }
        }
    }

    Ok(())
}

/// Return the peer id only when the multiaddr ends in `/p2p/<peer-id>`.
pub fn terminal_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    match addr.iter().last() {
        Some(Protocol::P2p(peer_id)) => Some(peer_id),
        _ => None,
    }
}

/// Small experimental cache that stores validated candidates by peer id.
#[derive(Debug, Default)]
pub struct PeerCandidateCache {
    candidates: HashMap<PeerId, AukiPeerCandidateV1>,
}

impl PeerCandidateCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate and store a candidate. A later candidate for the same peer id
    /// replaces the previous one only after passing validation.
    pub fn store(
        &mut self,
        candidate: AukiPeerCandidateV1,
        now_ms: u64,
    ) -> Result<(), PeerCandidateRejectReason> {
        validate_peer_candidate(&candidate, now_ms)?;
        self.candidates.insert(candidate.peer_id, candidate);
        Ok(())
    }

    pub fn get(&self, peer_id: &PeerId) -> Option<&AukiPeerCandidateV1> {
        self.candidates.get(peer_id)
    }

    pub fn contains_peer(&self, peer_id: &PeerId) -> bool {
        self.candidates.contains_key(peer_id)
    }

    /// Produce all non-expired candidates as transport dial entries.
    ///
    /// The returned entries are suitable for asking [`NetworkRuntime`](crate::NetworkRuntime)
    /// to establish a connection. They are not proof that the peer is a member,
    /// trusted, or usable by an application.
    pub fn eligible_allowed_peers(&self, now_ms: u64) -> Vec<AllowedPeer> {
        self.candidates
            .values()
            .filter(|candidate| validate_peer_candidate(candidate, now_ms).is_ok())
            .map(AukiPeerCandidateV1::to_allowed_peer)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerIdentity;

    fn identity(seed: u8) -> PeerIdentity {
        PeerIdentity::from_seed(&[seed; 32])
    }

    fn addr_for(peer_id: PeerId, port: u16) -> Multiaddr {
        format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer_id}")
            .parse()
            .unwrap()
    }

    #[test]
    fn peer_candidate_stores_connected_peer_advertisement_as_allowed_peer() {
        let peer_id = identity(42).peer_id();
        let candidate = AukiPeerCandidateV1::with_ttl(
            PeerCandidateSource::ConnectedPeerAdvertisement,
            peer_id,
            vec![addr_for(peer_id, 10142)],
            1_000,
            Duration::from_secs(30),
        );
        let mut cache = PeerCandidateCache::new();

        cache.store(candidate, 1_001).unwrap();

        let eligible = cache.eligible_allowed_peers(1_002);
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].peer_id, peer_id);
        assert_eq!(eligible[0].multiaddrs, vec![addr_for(peer_id, 10142)]);
    }

    #[test]
    fn peer_candidate_rejects_terminal_peer_id_mismatch() {
        let expected = identity(7).peer_id();
        let actual = identity(8).peer_id();
        let candidate = AukiPeerCandidateV1::with_ttl(
            PeerCandidateSource::ConnectedPeerAdvertisement,
            expected,
            vec![addr_for(actual, 10008)],
            1_000,
            Duration::from_secs(30),
        );

        let err = validate_peer_candidate(&candidate, 1_001).unwrap_err();
        assert!(matches!(
            err,
            PeerCandidateRejectReason::AddrPeerIdMismatch { .. }
        ));
        assert_eq!(err.marker(), "peer_id_mismatch");
    }

    #[test]
    fn peer_candidate_rejects_non_terminal_peer_id() {
        let peer_id = identity(11).peer_id();
        let addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/10011/p2p/{peer_id}/p2p-circuit")
            .parse()
            .unwrap();
        let candidate = AukiPeerCandidateV1::with_ttl(
            PeerCandidateSource::ConnectedPeerAdvertisement,
            peer_id,
            vec![addr],
            1_000,
            Duration::from_secs(30),
        );

        let err = validate_peer_candidate(&candidate, 1_001).unwrap_err();
        assert!(matches!(
            err,
            PeerCandidateRejectReason::MissingTerminalPeerId { .. }
        ));
        assert_eq!(err.marker(), "missing_terminal_peer_id");
    }

    #[test]
    fn peer_candidate_rejects_unsupported_source_string() {
        let err = parse_peer_candidate_source("dht").unwrap_err();
        assert_eq!(
            err,
            PeerCandidateRejectReason::UnsupportedSource {
                source: "dht".into()
            }
        );
        assert_eq!(err.marker(), "unsupported_source");
    }

    #[test]
    fn peer_candidate_rejects_expired_candidate() {
        let peer_id = identity(9).peer_id();
        let candidate = AukiPeerCandidateV1::with_ttl(
            PeerCandidateSource::ConnectedPeerAdvertisement,
            peer_id,
            vec![addr_for(peer_id, 10009)],
            1_000,
            Duration::from_millis(10),
        );

        let err = validate_peer_candidate(&candidate, 1_010).unwrap_err();
        assert!(matches!(err, PeerCandidateRejectReason::Expired { .. }));
        assert_eq!(err.marker(), "expired");
    }

    #[test]
    fn peer_candidate_rejects_domain_hints_as_non_authoritative() {
        let peer_id = identity(10).peer_id();
        let mut candidate = AukiPeerCandidateV1::with_ttl(
            PeerCandidateSource::ConnectedPeerAdvertisement,
            peer_id,
            vec![addr_for(peer_id, 10010)],
            1_000,
            Duration::from_secs(30),
        );
        candidate.domain_hints.push("domain:example".into());

        let err = validate_peer_candidate(&candidate, 1_001).unwrap_err();
        assert_eq!(err, PeerCandidateRejectReason::NonAuthoritativeDomainHints);
        assert_eq!(err.marker(), "non_authoritative_domain_hints");
    }
}

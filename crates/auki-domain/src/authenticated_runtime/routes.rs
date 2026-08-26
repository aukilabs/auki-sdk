use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

use auki_p2p::{Multiaddr, PeerId, canonicalize_circuit_route, validate_direct_route};
use tokio_util::sync::CancellationToken;

/// Maximum number of peers that may have configured routes in one Domain.
pub(crate) const MAX_DOMAIN_ROUTE_PEERS: usize = 1_024;
/// Maximum number of canonical candidates configured for one expected peer.
pub(crate) const MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER: usize = 16;
/// Maximum raw candidates one replacement may inspect before canonicalization.
///
/// Canonical duplicates do not consume configured slots, but the input itself
/// must still be resource-bounded so a hostile or accidental iterator cannot
/// make route validation unbounded.
pub(crate) const MAX_DOMAIN_ROUTE_INPUT_CANDIDATES_PER_PEER: usize = 4_096;
/// Maximum number of canonical candidates configured across one Domain.
pub(crate) const MAX_DOMAIN_ROUTE_CANDIDATES: usize = 4_096;
/// Maximum binary multiaddr size accepted from the host.
pub(crate) const MAX_DOMAIN_ROUTE_ENCODED_BYTES: usize = 1_024;

/// One expected peer and its canonical, deterministically ordered candidates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerRoutes {
    /// Peer that every candidate is required to reach.
    pub(crate) expected_peer: PeerId,
    /// Direct candidates first, followed by circuit candidates; each group is
    /// ordered by its canonical string form.
    pub(crate) candidates: Vec<Multiaddr>,
}

/// Immutable point-in-time view of one Domain's route catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DomainRouteSnapshot {
    /// Monotonic catalog revision. It advances only when catalog state changes.
    pub(crate) revision: u64,
    /// Peer route sets in stable Peer-ID order.
    pub(crate) peers: Vec<PeerRoutes>,
    /// Sum of all canonical candidates in `peers`.
    pub(crate) total_candidates: usize,
}

/// A rejected Domain route operation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DomainRoutesError {
    /// The owning Domain has begun or completed shutdown.
    #[error("the Domain route catalog is stopped")]
    Stopped,
    /// The binary multiaddr exceeds the fixed pre-validation bound.
    #[error(
        "route candidate for {expected_peer} is {encoded_bytes} encoded bytes; maximum is {maximum}"
    )]
    EncodedRouteTooLong {
        /// Expected destination peer.
        expected_peer: PeerId,
        /// Candidate's binary multiaddr length.
        encoded_bytes: usize,
        /// Configured hard maximum.
        maximum: usize,
    },
    /// The candidate is neither a valid direct route nor a complete CRv2 route.
    #[error(
        "route candidate for {expected_peer} is invalid (direct: {direct_error}; circuit: {circuit_error})"
    )]
    InvalidCandidate {
        /// Expected destination peer.
        expected_peer: PeerId,
        /// Direct-route validation diagnostic.
        direct_error: Box<str>,
        /// Circuit-route validation diagnostic.
        circuit_error: Box<str>,
    },
    /// One peer's deduplicated candidate set exceeds its bound.
    #[error(
        "peer {expected_peer} has {candidate_count} canonical route candidates; maximum is {maximum}"
    )]
    PeerCandidateLimitExceeded {
        /// Expected destination peer.
        expected_peer: PeerId,
        /// Deduplicated candidate count.
        candidate_count: usize,
        /// Configured hard maximum.
        maximum: usize,
    },
    /// One replacement supplied too many raw candidates to inspect safely.
    #[error(
        "peer {expected_peer} supplied more than {maximum} route candidates before canonicalization"
    )]
    InputCandidateLimitExceeded {
        /// Expected destination peer.
        expected_peer: PeerId,
        /// Fixed raw-input maximum.
        maximum: usize,
    },
    /// A new peer would exceed the Domain-wide peer bound.
    #[error("Domain route catalog would contain {peer_count} peers; maximum is {maximum}")]
    PeerLimitExceeded {
        /// Resulting peer count.
        peer_count: usize,
        /// Configured hard maximum.
        maximum: usize,
    },
    /// A replacement would exceed the Domain-wide candidate bound.
    #[error(
        "Domain route catalog would contain {candidate_count} route candidates; maximum is {maximum}"
    )]
    CandidateLimitExceeded {
        /// Resulting candidate count.
        candidate_count: usize,
        /// Configured hard maximum.
        maximum: usize,
    },
    /// The catalog can no longer advance its monotonic revision.
    #[error("Domain route catalog revision is exhausted")]
    RevisionExhausted,
}

/// Result returned by private Domain route-catalog operations.
pub(crate) type DomainRoutesResult<T> = Result<T, DomainRoutesError>;

/// Cloneable, lifecycle-fenced catalog of explicit untrusted route hints.
///
/// Routes never confer authority. Consumers must still use the expected peer
/// and the owning Domain's authenticated protocol requirements when dialing.
#[derive(Clone)]
pub(crate) struct DomainRoutes {
    inner: Arc<DomainRoutesInner>,
}

struct DomainRoutesInner {
    lifecycle: CancellationToken,
    state: Mutex<RouteState>,
}

#[derive(Default)]
struct RouteState {
    revision: u64,
    peers: BTreeMap<PeerId, Vec<Multiaddr>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateKind {
    Direct,
    Circuit,
}

struct CanonicalCandidate {
    kind: CandidateKind,
    route: Multiaddr,
}

impl DomainRoutes {
    /// Create an empty catalog fenced by the owning Domain lifecycle.
    pub(crate) fn new(lifecycle: CancellationToken) -> Self {
        Self {
            inner: Arc::new(DomainRoutesInner {
                lifecycle,
                state: Mutex::new(RouteState::default()),
            }),
        }
    }

    /// Atomically replace every candidate for `expected_peer`.
    ///
    /// An empty replacement removes the peer. Invalid or over-limit input
    /// leaves the prior state and revision unchanged.
    pub(crate) fn replace(
        &self,
        expected_peer: PeerId,
        candidates: impl IntoIterator<Item = Multiaddr>,
    ) -> DomainRoutesResult<DomainRouteSnapshot> {
        self.ensure_running()?;
        let candidates = canonicalize_candidates(expected_peer, candidates)?;
        if candidates.len() > MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER {
            return Err(DomainRoutesError::PeerCandidateLimitExceeded {
                expected_peer,
                candidate_count: candidates.len(),
                maximum: MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER,
            });
        }

        let mut state = self.lock_state();
        self.ensure_running()?;

        let prior = state.peers.get(&expected_peer);
        if prior.is_some_and(|prior| prior == &candidates)
            || (prior.is_none() && candidates.is_empty())
        {
            return Ok(snapshot_of(&state));
        }

        let replacing_existing = prior.is_some();
        let peer_count =
            state.peers.len() + usize::from(!replacing_existing && !candidates.is_empty());
        if peer_count > MAX_DOMAIN_ROUTE_PEERS {
            return Err(DomainRoutesError::PeerLimitExceeded {
                peer_count,
                maximum: MAX_DOMAIN_ROUTE_PEERS,
            });
        }

        let prior_count = prior.map_or(0, Vec::len);
        let candidate_count = total_candidates(&state)
            .saturating_sub(prior_count)
            .saturating_add(candidates.len());
        if candidate_count > MAX_DOMAIN_ROUTE_CANDIDATES {
            return Err(DomainRoutesError::CandidateLimitExceeded {
                candidate_count,
                maximum: MAX_DOMAIN_ROUTE_CANDIDATES,
            });
        }

        state.revision = state
            .revision
            .checked_add(1)
            .ok_or(DomainRoutesError::RevisionExhausted)?;
        if candidates.is_empty() {
            state.peers.remove(&expected_peer);
        } else {
            state.peers.insert(expected_peer, candidates);
        }
        Ok(snapshot_of(&state))
    }

    /// Atomically remove every candidate for `expected_peer`.
    pub(crate) fn remove(&self, expected_peer: PeerId) -> DomainRoutesResult<DomainRouteSnapshot> {
        self.replace(expected_peer, [])
    }

    /// Return an immutable snapshot of the current catalog.
    pub(crate) fn snapshot(&self) -> DomainRoutesResult<DomainRouteSnapshot> {
        self.ensure_running()?;
        let state = self.lock_state();
        self.ensure_running()?;
        Ok(snapshot_of(&state))
    }

    /// Return the canonical candidates currently configured for one peer.
    pub(crate) fn candidates(&self, expected_peer: PeerId) -> DomainRoutesResult<Vec<Multiaddr>> {
        self.ensure_running()?;
        let state = self.lock_state();
        self.ensure_running()?;
        Ok(state.peers.get(&expected_peer).cloned().unwrap_or_default())
    }

    /// Validate and canonicalize one exact route supplied to a Domain protocol.
    pub(super) fn canonicalize_candidate(
        expected_peer: PeerId,
        candidate: Multiaddr,
    ) -> DomainRoutesResult<Multiaddr> {
        canonicalize_classified_candidate(expected_peer, candidate).map(|candidate| candidate.route)
    }

    fn ensure_running(&self) -> DomainRoutesResult<()> {
        if self.inner.lifecycle.is_cancelled() {
            Err(DomainRoutesError::Stopped)
        } else {
            Ok(())
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, RouteState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(super) fn canonicalize_candidates(
    expected_peer: PeerId,
    candidates: impl IntoIterator<Item = Multiaddr>,
) -> DomainRoutesResult<Vec<Multiaddr>> {
    let mut canonical = Vec::<CanonicalCandidate>::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        if index >= MAX_DOMAIN_ROUTE_INPUT_CANDIDATES_PER_PEER {
            return Err(DomainRoutesError::InputCandidateLimitExceeded {
                expected_peer,
                maximum: MAX_DOMAIN_ROUTE_INPUT_CANDIDATES_PER_PEER,
            });
        }
        let candidate = canonicalize_classified_candidate(expected_peer, candidate)?;
        if canonical
            .iter()
            .any(|existing| existing.route == candidate.route)
        {
            continue;
        }
        canonical.push(candidate);
        if canonical.len() > MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER {
            return Err(DomainRoutesError::PeerCandidateLimitExceeded {
                expected_peer,
                candidate_count: canonical.len(),
                maximum: MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER,
            });
        }
    }
    let mut candidates = canonical;
    candidates.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.route.to_string().cmp(&right.route.to_string()))
    });
    candidates.dedup_by(|left, right| left.route == right.route);
    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.route)
        .collect())
}

fn canonicalize_classified_candidate(
    expected_peer: PeerId,
    candidate: Multiaddr,
) -> DomainRoutesResult<CanonicalCandidate> {
    let encoded_bytes = candidate.len();
    if encoded_bytes > MAX_DOMAIN_ROUTE_ENCODED_BYTES {
        return Err(DomainRoutesError::EncodedRouteTooLong {
            expected_peer,
            encoded_bytes,
            maximum: MAX_DOMAIN_ROUTE_ENCODED_BYTES,
        });
    }

    match validate_direct_route(&candidate, expected_peer) {
        Ok(route) => Ok(CanonicalCandidate {
            kind: CandidateKind::Direct,
            route,
        }),
        Err(direct_error) => match canonicalize_circuit_route(&candidate, expected_peer) {
            Ok(circuit) => Ok(CanonicalCandidate {
                kind: CandidateKind::Circuit,
                route: circuit.route,
            }),
            Err(circuit_error) => Err(DomainRoutesError::InvalidCandidate {
                expected_peer,
                direct_error: direct_error.to_string().into_boxed_str(),
                circuit_error: circuit_error.to_string().into_boxed_str(),
            }),
        },
    }
}

fn snapshot_of(state: &RouteState) -> DomainRouteSnapshot {
    let mut peers = state
        .peers
        .iter()
        .map(|(expected_peer, candidates)| PeerRoutes {
            expected_peer: *expected_peer,
            candidates: candidates.clone(),
        })
        .collect::<Vec<_>>();
    peers.sort_unstable_by_key(|peer| peer.expected_peer.to_string());
    DomainRouteSnapshot {
        revision: state.revision,
        total_candidates: peers.iter().map(|peer| peer.candidates.len()).sum(),
        peers,
    }
}

fn total_candidates(state: &RouteState) -> usize {
    state.peers.values().map(Vec::len).sum()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use auki_p2p::Protocol;

    use super::*;

    fn peer(seed: u64) -> PeerId {
        let mut encoded = [0_u8; 34];
        encoded[1] = 32;
        encoded[2..10].copy_from_slice(&seed.to_be_bytes());
        PeerId::from_bytes(&encoded).expect("test Peer ID multihash must parse")
    }

    fn addr(value: impl AsRef<str>) -> Multiaddr {
        Multiaddr::from_str(value.as_ref()).expect("test multiaddr must parse")
    }

    fn direct_candidates(count: usize) -> Vec<Multiaddr> {
        (0..count)
            .map(|index| addr(format!("/ip4/127.0.0.1/tcp/{}", index + 1)))
            .collect()
    }

    fn circuit_route(target: PeerId, relay: PeerId) -> Multiaddr {
        addr(format!(
            "/dns4/relay.example.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}"
        ))
    }

    #[test]
    fn canonical_routes_are_deduplicated_direct_first_and_stably_ordered() {
        let expected_peer = peer(1);
        let relay = peer(2);
        let lifecycle = CancellationToken::new();
        let routes = DomainRoutes::new(lifecycle);
        let direct_a = addr("/dns4/z.example/tcp/4001");
        let direct_a_with_peer = addr(format!("{direct_a}/p2p/{expected_peer}"));
        let direct_b = addr("/ip4/127.0.0.1/tcp/4000");
        let circuit = circuit_route(expected_peer, relay);

        let snapshot = routes
            .replace(
                expected_peer,
                [
                    circuit.clone(),
                    direct_b.clone(),
                    direct_a_with_peer,
                    direct_a.clone(),
                ],
            )
            .unwrap();

        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.total_candidates, 3);
        assert_eq!(
            snapshot.peers,
            vec![PeerRoutes {
                expected_peer,
                candidates: vec![direct_a.clone(), direct_b.clone(), circuit.clone()],
            }]
        );
        assert_eq!(
            routes.candidates(expected_peer).unwrap(),
            vec![direct_a, direct_b, circuit]
        );

        let unchanged = routes
            .replace(expected_peer, routes.candidates(expected_peer).unwrap())
            .unwrap();
        assert_eq!(unchanged.revision, 1);
    }

    #[test]
    fn exact_direct_and_complete_circuit_grammar_is_enforced() {
        let expected_peer = peer(10);
        let other_peer = peer(11);
        let relay = peer(12);
        let direct = addr(format!("/dns6/node.example/tcp/1234/p2p/{expected_peer}"));
        assert_eq!(
            DomainRoutes::canonicalize_candidate(expected_peer, direct).unwrap(),
            addr("/dns6/node.example/tcp/1234")
        );

        let circuit = circuit_route(expected_peer, relay);
        assert_eq!(
            DomainRoutes::canonicalize_candidate(expected_peer, circuit.clone()).unwrap(),
            circuit
        );

        let invalid = [
            addr("/ip4/127.0.0.1/udp/4001"),
            addr("/ip4/127.0.0.1/tcp/0"),
            addr(format!("/ip4/127.0.0.1/tcp/4001/p2p/{other_peer}")),
            addr(format!(
                "/dns4/relay.example.com/tcp/443/p2p/{relay}/p2p-circuit"
            )),
            circuit_route(other_peer, relay),
        ];
        for candidate in invalid {
            assert!(matches!(
                DomainRoutes::canonicalize_candidate(expected_peer, candidate),
                Err(DomainRoutesError::InvalidCandidate { .. })
            ));
        }
    }

    #[test]
    fn oversized_binary_multiaddr_is_rejected_before_route_validation() {
        let expected_peer = peer(20);
        let oversized = addr(format!(
            "/dns4/{}/tcp/443",
            "a".repeat(MAX_DOMAIN_ROUTE_ENCODED_BYTES + 1)
        ));
        assert!(oversized.len() > MAX_DOMAIN_ROUTE_ENCODED_BYTES);

        assert!(matches!(
            DomainRoutes::canonicalize_candidate(expected_peer, oversized),
            Err(DomainRoutesError::EncodedRouteTooLong {
                encoded_bytes,
                maximum: MAX_DOMAIN_ROUTE_ENCODED_BYTES,
                ..
            }) if encoded_bytes > MAX_DOMAIN_ROUTE_ENCODED_BYTES
        ));
    }

    #[test]
    fn per_peer_bound_is_deduplicated_and_failure_is_atomic() {
        let expected_peer = peer(30);
        let routes = DomainRoutes::new(CancellationToken::new());
        let original = addr("/ip4/127.0.0.1/tcp/4001");
        routes.replace(expected_peer, [original.clone()]).unwrap();
        let before = routes.snapshot().unwrap();

        let duplicate_input = vec![original.clone(); MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER + 1];
        let deduplicated = routes.replace(expected_peer, duplicate_input).unwrap();
        assert_eq!(deduplicated, before);

        let too_many = direct_candidates(MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER + 1);
        assert!(matches!(
            routes.replace(expected_peer, too_many),
            Err(DomainRoutesError::PeerCandidateLimitExceeded {
                candidate_count,
                maximum: MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER,
                ..
            }) if candidate_count == MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER + 1
        ));
        assert_eq!(routes.snapshot().unwrap(), before);

        let invalid = addr("/ip4/127.0.0.1/udp/4001");
        assert!(matches!(
            routes.replace(expected_peer, [invalid]),
            Err(DomainRoutesError::InvalidCandidate { .. })
        ));
        assert_eq!(routes.snapshot().unwrap(), before);

        assert!(matches!(
            routes.replace(
                expected_peer,
                std::iter::repeat_n(original, MAX_DOMAIN_ROUTE_INPUT_CANDIDATES_PER_PEER + 1,),
            ),
            Err(DomainRoutesError::InputCandidateLimitExceeded {
                maximum: MAX_DOMAIN_ROUTE_INPUT_CANDIDATES_PER_PEER,
                ..
            })
        ));
        assert_eq!(routes.snapshot().unwrap(), before);
    }

    #[test]
    fn domain_peer_bound_rejects_only_the_new_peer_atomically() {
        let routes = DomainRoutes::new(CancellationToken::new());
        let candidate = addr("/ip4/127.0.0.1/tcp/4001");
        {
            let mut state = routes.lock_state();
            state.peers.extend(
                (0..MAX_DOMAIN_ROUTE_PEERS)
                    .map(|index| (peer(1_000 + index as u64), vec![candidate.clone()])),
            );
            state.revision = MAX_DOMAIN_ROUTE_PEERS as u64;
        }
        let before = routes.snapshot().unwrap();
        assert_eq!(before.peers.len(), MAX_DOMAIN_ROUTE_PEERS);

        assert!(matches!(
            routes.replace(peer(99_999), [candidate]),
            Err(DomainRoutesError::PeerLimitExceeded {
                peer_count,
                maximum: MAX_DOMAIN_ROUTE_PEERS,
            }) if peer_count == MAX_DOMAIN_ROUTE_PEERS + 1
        ));
        assert_eq!(routes.snapshot().unwrap(), before);
    }

    #[test]
    fn domain_candidate_bound_and_replacement_accounting_are_atomic() {
        let routes = DomainRoutes::new(CancellationToken::new());
        let candidates = direct_candidates(MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER);
        let full_peers = MAX_DOMAIN_ROUTE_CANDIDATES / MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER;
        {
            let mut state = routes.lock_state();
            state.peers.extend(
                (0..full_peers).map(|index| (peer(10_000 + index as u64), candidates.clone())),
            );
            state.revision = full_peers as u64;
        }
        let before = routes.snapshot().unwrap();
        assert_eq!(before.total_candidates, MAX_DOMAIN_ROUTE_CANDIDATES);

        assert!(matches!(
            routes.replace(peer(200_000), [addr("/ip4/127.0.0.1/tcp/5000")]),
            Err(DomainRoutesError::CandidateLimitExceeded {
                candidate_count,
                maximum: MAX_DOMAIN_ROUTE_CANDIDATES,
            }) if candidate_count == MAX_DOMAIN_ROUTE_CANDIDATES + 1
        ));
        assert_eq!(routes.snapshot().unwrap(), before);

        let replaced_peer = peer(10_000);
        let shrunk = routes
            .replace(replaced_peer, [addr("/ip4/127.0.0.1/tcp/6000")])
            .unwrap();
        assert_eq!(
            shrunk.total_candidates,
            MAX_DOMAIN_ROUTE_CANDIDATES - MAX_DOMAIN_ROUTE_CANDIDATES_PER_PEER + 1
        );
    }

    #[test]
    fn remove_is_idempotent_and_empty_replace_does_not_retain_a_peer() {
        let expected_peer = peer(40);
        let routes = DomainRoutes::new(CancellationToken::new());

        assert_eq!(routes.remove(expected_peer).unwrap().revision, 0);
        assert_eq!(routes.replace(expected_peer, []).unwrap().peers, vec![]);
        assert_eq!(
            routes
                .replace(expected_peer, [addr("/ip6/::1/tcp/4001")])
                .unwrap()
                .revision,
            1
        );
        let removed = routes.remove(expected_peer).unwrap();
        assert_eq!(removed.revision, 2);
        assert!(removed.peers.is_empty());
        assert_eq!(routes.remove(expected_peer).unwrap().revision, 2);
    }

    #[test]
    fn cancellation_fences_all_handles_without_mutating_state() {
        let expected_peer = peer(50);
        let lifecycle = CancellationToken::new();
        let routes = DomainRoutes::new(lifecycle.clone());
        let clone = routes.clone();
        routes
            .replace(expected_peer, [addr("/ip4/127.0.0.1/tcp/4001")])
            .unwrap();
        let before = routes.snapshot().unwrap();

        lifecycle.cancel();
        assert!(matches!(clone.snapshot(), Err(DomainRoutesError::Stopped)));
        assert!(matches!(
            clone.candidates(expected_peer),
            Err(DomainRoutesError::Stopped)
        ));
        assert!(matches!(
            clone.replace(expected_peer, [addr("/ip4/127.0.0.1/tcp/5000")]),
            Err(DomainRoutesError::Stopped)
        ));
        assert!(matches!(
            clone.remove(expected_peer),
            Err(DomainRoutesError::Stopped)
        ));

        let state = routes.lock_state();
        assert_eq!(snapshot_of(&state), before);
    }

    #[test]
    fn revision_exhaustion_leaves_existing_routes_unchanged() {
        let expected_peer = peer(60);
        let routes = DomainRoutes::new(CancellationToken::new());
        routes
            .replace(expected_peer, [addr("/ip4/127.0.0.1/tcp/4001")])
            .unwrap();
        {
            let mut state = routes.lock_state();
            state.revision = u64::MAX;
        }
        let before = routes.snapshot().unwrap();

        assert!(matches!(
            routes.replace(expected_peer, [addr("/ip4/127.0.0.1/tcp/5000")]),
            Err(DomainRoutesError::RevisionExhausted)
        ));
        assert_eq!(routes.snapshot().unwrap(), before);
    }

    #[test]
    fn direct_classification_precedes_circuit_routes() {
        let expected_peer = peer(70);
        let relay = peer(71);
        let routes = DomainRoutes::new(CancellationToken::new());
        let snapshot = routes
            .replace(
                expected_peer,
                [
                    circuit_route(expected_peer, relay),
                    addr("/ip4/127.0.0.1/tcp/4001"),
                ],
            )
            .unwrap();

        assert!(
            !snapshot.peers[0].candidates[0]
                .iter()
                .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
        );
        assert!(
            snapshot.peers[0].candidates[1]
                .iter()
                .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
        );
    }
}

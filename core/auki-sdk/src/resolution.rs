//! Exact identity lookup and authenticated connection of a selected result.

use std::{future::Future, time::Duration};

use auki_p2p::{CandidateRouteKind, canonicalize_candidate_route};
use chrono::Utc;
use futures::{FutureExt, pin_mut};
use futures_timer::Delay;
use uuid::Uuid;

use crate::{
    AukiDiscoveryCandidate, AukiPeerProtocols, AukiProtocolError, AukiProtocolRouteAttempt,
    AuthenticatedPeer, AuthenticatedRouteStream, PeerId,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const REJECTED_STREAM_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// An exact transport or machine identity, scoped to the owning peer's Domain.
///
/// A robot or compute node can have several simultaneously advertised Peer IDs.
/// This identity constrains discovery and the signed claims of the connection;
/// it does not grant application, Domain data, or DMS task permissions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AukiPeerIdentity {
    /// One libp2p transport identity.
    Peer(PeerId),
    /// One DDS robot subject, with signed peer type `robot`.
    Robot(Uuid),
    /// One DDS compute-node subject, with signed peer type `compute`.
    Compute(Uuid),
}

impl AukiPeerIdentity {
    pub(crate) fn matches_candidate(&self, candidate: &AukiDiscoveryCandidate) -> bool {
        match self {
            Self::Peer(id) => candidate.peer_id() == *id,
            Self::Robot(id) => {
                candidate.peer_type() == Some("robot")
                    && candidate.subject_id() == Some(id.to_string().as_str())
            }
            Self::Compute(id) => {
                candidate.peer_type() == Some("compute")
                    && candidate.subject_id() == Some(id.to_string().as_str())
            }
        }
    }

    fn matches_authenticated(&self, peer: &AuthenticatedPeer) -> bool {
        match self {
            Self::Peer(id) => peer.peer_id == *id,
            Self::Robot(id) => {
                peer.peer_type.as_deref() == Some("robot") && peer.subject == id.to_string()
            }
            Self::Compute(id) => {
                peer.peer_type.as_deref() == Some("compute") && peer.subject == id.to_string()
            }
        }
    }
}

/// One untrusted lookup result retaining the identity, Domain and protocol requested.
///
/// Select a result explicitly when a machine has several peers, then call
/// [`AukiPeerProtocols::open_resolved`]. The SDK never chooses an arbitrary peer
/// or treats this advertisement as authenticated identity evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AukiResolvedPeer {
    pub(crate) identity: AukiPeerIdentity,
    pub(crate) domain_id: Uuid,
    pub(crate) protocol_id: String,
    pub(crate) candidate: AukiDiscoveryCandidate,
}

impl AukiResolvedPeer {
    /// Exact identity originally requested, independent of advertisement claims.
    pub fn identity(&self) -> AukiPeerIdentity {
        self.identity
    }

    /// Domain in which this result was discovered.
    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    /// Exact mounted protocol requested during lookup.
    pub fn protocol_id(&self) -> &str {
        &self.protocol_id
    }

    /// Expiring, untrusted route and protocol hints for this one Peer ID.
    pub fn candidate(&self) -> &AukiDiscoveryCandidate {
        &self.candidate
    }
}

/// Failure to connect a selected exact-identity discovery result.
#[derive(Debug, thiserror::Error)]
pub enum AukiPeerConnectError {
    /// The result belongs to a different Domain from the opening peer.
    #[error("resolved peer belongs to a different Domain")]
    WrongDomain,
    /// The advertisement expired; perform a fresh lookup before opening.
    #[error("resolved peer advertisement has expired")]
    Expired,
    /// No advertised route is supported by this platform.
    #[error("resolved peer has no route supported by this platform")]
    NoSupportedRoutes,
    /// The complete route-opening operation exceeded its fixed deadline.
    #[error("resolved peer connection exceeded its 30-second deadline")]
    TimedOut,
    /// The existing authenticated P2P runtime rejected the operation.
    #[error(transparent)]
    Protocol(#[from] AukiProtocolError),
    /// All supported routes failed without exposing an application stream.
    #[error("all supported routes to the resolved peer failed")]
    AllRoutesFailed {
        /// Bounded route diagnostics in attempt order.
        attempts: Vec<AukiProtocolRouteAttempt>,
    },
    /// Signed remote claims do not match the selected Peer ID or requested machine.
    #[error(
        "authenticated peer does not match the requested identity (cleanup failed: {cleanup_failed})"
    )]
    IdentityMismatch {
        /// Explicit close failed or exceeded its deadline; Drop still releases ownership.
        cleanup_failed: bool,
    },
}

impl AukiPeerProtocols {
    /// Open a selected lookup result and verify its signed identity before returning I/O.
    ///
    /// Tries each supported advertised route at most once, in its canonical order,
    /// within a 30-second total deadline. Native uses direct TCP and TCP relays;
    /// browsers use WSS relays. A machine-identity mismatch fails immediately and
    /// closes the stream before the caller can send application bytes. The usual
    /// signature, issuer, audience, expiry, Peer ID, Domain, and scope checks still
    /// apply. Dropping the future cancels opening; peer shutdown fences it too.
    pub async fn open_resolved(
        &self,
        resolved: &AukiResolvedPeer,
    ) -> Result<AuthenticatedRouteStream, AukiPeerConnectError> {
        if self.domain_id() != resolved.domain_id {
            return Err(AukiPeerConnectError::WrongDomain);
        }
        before_deadline(self.open_resolved_routes(resolved), CONNECT_TIMEOUT)
            .await
            .ok_or(AukiPeerConnectError::TimedOut)?
    }

    async fn open_resolved_routes(
        &self,
        resolved: &AukiResolvedPeer,
    ) -> Result<AuthenticatedRouteStream, AukiPeerConnectError> {
        let candidate = resolved.candidate();
        let mut attempts = Vec::new();
        if candidate.expires_at() <= Utc::now() {
            return Err(AukiPeerConnectError::Expired);
        }
        for route in candidate.routes() {
            let canonical = canonicalize_candidate_route(route, candidate.peer_id())
                .expect("discovery has already validated each candidate route");
            if !supported_route(canonical.kind()) {
                continue;
            }
            if candidate.expires_at() <= Utc::now() {
                return Err(AukiPeerConnectError::Expired);
            }
            match self
                .open_exact(candidate.peer_id(), route.clone(), resolved.protocol_id())
                .await
            {
                Ok(stream) => {
                    if stream.remote_peer().peer_id == candidate.peer_id()
                        && resolved
                            .identity
                            .matches_authenticated(stream.remote_peer())
                    {
                        return Ok(stream);
                    }
                    let cleanup_failed = !matches!(
                        before_deadline(stream.close(), REJECTED_STREAM_CLOSE_TIMEOUT).await,
                        Some(Ok(()))
                    );
                    return Err(AukiPeerConnectError::IdentityMismatch { cleanup_failed });
                }
                Err(AukiProtocolError::P2p(error)) => {
                    let unsupported_protocol = matches!(
                        &error,
                        auki_p2p::Error::TargetedStream(
                            auki_p2p::TargetedStreamError::UnsupportedProtocol { .. }
                        )
                    );
                    attempts.push(AukiProtocolRouteAttempt {
                        route: route.clone(),
                        error: error.to_string().chars().take(1_024).collect(),
                        unsupported_protocol,
                    });
                }
                Err(error) => return Err(error.into()),
            }
        }
        if attempts.is_empty() {
            Err(AukiPeerConnectError::NoSupportedRoutes)
        } else {
            Err(AukiPeerConnectError::AllRoutesFailed { attempts })
        }
    }
}

fn supported_route(kind: CandidateRouteKind) -> bool {
    #[cfg(target_arch = "wasm32")]
    return kind == CandidateRouteKind::RelayWss;
    #[cfg(not(target_arch = "wasm32"))]
    return matches!(
        kind,
        CandidateRouteKind::DirectTcp | CandidateRouteKind::RelayTcp
    );
}

pub(crate) async fn before_deadline<T>(
    operation: impl Future<Output = T>,
    duration: Duration,
) -> Option<T> {
    let operation = operation.fuse();
    let deadline = Delay::new(duration).fuse();
    pin_mut!(operation, deadline);
    futures::select_biased! {
        result = operation => Some(result),
        () = deadline => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    fn identity_matching_requires_exact_signed_subject_and_type() {
        let machine = Uuid::new_v4();
        let mut remote = AuthenticatedPeer {
            peer_id: crate::Identity::generate().peer_id(),
            subject: machine.to_string(),
            peer_type: Some("robot".into()),
            domain_ids: vec![Uuid::new_v4()],
            scopes: vec!["p2p".into()],
            application: None,
            verified_until: Utc::now() + chrono::Duration::minutes(1),
        };
        assert!(AukiPeerIdentity::Robot(machine).matches_authenticated(&remote));
        assert!(!AukiPeerIdentity::Compute(machine).matches_authenticated(&remote));
        assert!(!AukiPeerIdentity::Robot(Uuid::new_v4()).matches_authenticated(&remote));
        assert!(AukiPeerIdentity::Peer(remote.peer_id).matches_authenticated(&remote));
        assert!(
            !AukiPeerIdentity::Peer(crate::Identity::generate().peer_id())
                .matches_authenticated(&remote)
        );
        remote.peer_type = Some("compute".into());
        assert!(AukiPeerIdentity::Compute(machine).matches_authenticated(&remote));
        remote.peer_type = None;
        assert!(!AukiPeerIdentity::Compute(machine).matches_authenticated(&remote));
        remote.peer_type = Some("user".into());
        assert!(!AukiPeerIdentity::Robot(machine).matches_authenticated(&remote));
        remote.peer_type = Some("robot".into());
        remote.subject = format!(" {machine}");
        assert!(!AukiPeerIdentity::Robot(machine).matches_authenticated(&remote));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    fn route_selection_uses_the_platform_transport() {
        let peer = crate::Identity::generate().peer_id();
        let relay = crate::Identity::generate().peer_id();
        for (address, browser_supported) in [
            ("/ip4/127.0.0.1/tcp/4001".to_owned(), false),
            (
                format!("/dns4/relay.example.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{peer}"),
                false,
            ),
            (
                format!("/dns4/relay.example.com/tcp/443/wss/p2p/{relay}/p2p-circuit/p2p/{peer}"),
                true,
            ),
        ] {
            let route = canonicalize_candidate_route(&address.parse().unwrap(), peer).unwrap();
            assert_eq!(
                supported_route(route.kind()),
                if cfg!(target_arch = "wasm32") {
                    browser_supported
                } else {
                    !browser_supported
                }
            );
        }
    }
}

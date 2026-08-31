use auki_p2p::{
    ApplicationProtocol, ApplicationProtocolSpec, AuthenticatedApplicationStream, Multiaddr, PeerId,
};

/// One exact application protocol and its inbound resource bounds.
#[derive(Clone, Debug)]
pub struct AukiProtocolSpec {
    pub(crate) protocol_id: String,
    pub(crate) inner: ApplicationProtocolSpec,
}

impl AukiProtocolSpec {
    /// Validate one explicitly versioned protocol identifier and its handler bounds.
    pub fn new(
        protocol_id: impl Into<String>,
        max_concurrency: usize,
        max_frame_bytes: u32,
    ) -> Result<Self, AukiProtocolError> {
        let protocol_id = protocol_id.into();
        let protocol =
            ApplicationProtocol::new(protocol_id.clone()).map_err(AukiProtocolError::P2p)?;
        let inner = ApplicationProtocolSpec::new(protocol, max_concurrency, max_frame_bytes)
            .map_err(AukiProtocolError::P2p)?;
        Ok(Self { protocol_id, inner })
    }

    /// Exact libp2p application protocol identifier.
    pub fn protocol_id(&self) -> &str {
        &self.protocol_id
    }

    /// Maximum number of concurrently handled inbound streams.
    pub fn max_concurrency(&self) -> usize {
        self.inner.max_concurrency()
    }

    /// Maximum frame size the mounted codec is required to enforce.
    pub fn max_frame_bytes(&self) -> u32 {
        self.inner.max_frame_bytes()
    }
}

/// Mutually authenticated inbound application stream with its declared frame bound.
pub type AukiProtocolStream = AuthenticatedApplicationStream;

/// One failed route attempt made while opening an authenticated protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AukiProtocolRouteAttempt {
    /// Canonical route that was attempted.
    pub route: Multiaddr,
    /// Bounded transport diagnostic.
    pub error: String,
    /// Whether the peer was reached and rejected only this protocol identifier.
    pub unsupported_protocol: bool,
}

/// Failure from the SDK-owned authenticated application-protocol surface.
#[derive(Debug, thiserror::Error)]
pub enum AukiProtocolError {
    /// The owning peer has begun or completed shutdown.
    #[error("the Auki peer protocol surface is stopped")]
    Stopped,
    /// The same exact protocol identifier is already mounted.
    #[error("authenticated protocol {0} is already registered")]
    DuplicateProtocol(String),
    /// The monotonic registration generation cannot advance further.
    #[error("protocol registration generation is exhausted")]
    GenerationExhausted,
    /// No route is configured for the expected remote peer.
    #[error("no route is configured for expected peer {0}")]
    NoRoutes(PeerId),
    /// Every configured route failed.
    #[error(
        "all routes to expected peer {peer_id} failed for authenticated protocol {protocol_id}"
    )]
    AllRoutesFailed {
        /// Expected mutually authenticated remote peer.
        peer_id: Box<PeerId>,
        /// Exact requested protocol identifier.
        protocol_id: String,
        /// Stable route attempt order.
        attempts: Vec<AukiProtocolRouteAttempt>,
    },
    /// One exact route hint is invalid for the selected transport.
    #[error("route for expected peer {peer_id} is invalid: {reason}")]
    InvalidRoute {
        /// Expected mutually authenticated remote peer.
        peer_id: PeerId,
        /// Validation diagnostic.
        reason: String,
    },
    /// The underlying authenticated P2P runtime rejected the operation.
    #[error("authenticated protocol operation failed: {0}")]
    P2p(#[source] auki_p2p::Error),
    /// Ordered handler cleanup exceeded its fixed deadline.
    #[error("authenticated protocol cleanup timed out")]
    CleanupTimeout,
}

impl AukiProtocolError {
    /// Whether every route reached the peer and rejected only this protocol ID.
    pub fn all_routes_unsupported_protocol(&self) -> bool {
        matches!(
            self,
            Self::AllRoutesFailed { attempts, .. }
                if !attempts.is_empty()
                    && attempts.iter().all(|attempt| attempt.unsupported_protocol)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_uses_the_shared_p2p_bounds() {
        assert!(AukiProtocolSpec::new("/example/echo/1.0.0", 0, 1).is_err());
        assert!(AukiProtocolSpec::new("/example/echo/1.0.0", 1, 0).is_err());
        let spec = AukiProtocolSpec::new("/example/echo/1.0.0", 4, 4096).unwrap();
        assert_eq!(spec.protocol_id(), "/example/echo/1.0.0");
        assert_eq!(spec.max_concurrency(), 4);
        assert_eq!(spec.max_frame_bytes(), 4096);
    }

    #[test]
    fn unsupported_fallback_requires_every_attempt() {
        let peer_id = auki_p2p::Identity::generate().peer_id();
        let attempt = AukiProtocolRouteAttempt {
            route: "/ip4/127.0.0.1/tcp/4001".parse().unwrap(),
            error: "unsupported".into(),
            unsupported_protocol: true,
        };
        let error = AukiProtocolError::AllRoutesFailed {
            peer_id: Box::new(peer_id),
            protocol_id: "/example/echo/1.0.0".into(),
            attempts: vec![attempt],
        };
        assert!(error.all_routes_unsupported_protocol());
    }
}

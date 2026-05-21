//! Browser-compatible Domain session primitives.
//!
//! This module is deliberately small for the first browser slice: it
//! establishes that browser Domain sessions belong to `auki-domain`,
//! while transport-backed create/join/stream behavior continues to live
//! behind shared SDK networking as it becomes wasm-capable.

use auki_network::PeerIdentity;
use serde::Serialize;

/// Browser-compatible Domain session owned by the shared Domain crate.
#[derive(Clone)]
pub struct BrowserDomainSession {
    identity: PeerIdentity,
}

impl BrowserDomainSession {
    /// Build a browser Domain session from an SDK peer identity.
    pub fn new(identity: PeerIdentity) -> Self {
        Self { identity }
    }

    /// Canonical libp2p PeerId for this browser Domain peer.
    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }

    /// Clone the SDK peer identity for browser transport helpers that
    /// need to build a libp2p swarm outside this crate.
    pub fn identity(&self) -> PeerIdentity {
        self.identity.clone()
    }

    /// Explicit result for transport-backed operations that are not
    /// connected to a browser-capable network runtime yet.
    pub fn transport_unavailable(&self) -> BrowserDomainResult {
        BrowserDomainResult::fail(
            "transport_unavailable",
            "Browser SDK transport is not implemented yet.",
        )
    }

    /// Successful void result for local state-only operations.
    pub fn ok(&self) -> BrowserDomainResult {
        BrowserDomainResult::ok()
    }
}

impl BrowserDomainResult {
    /// Successful void browser Domain result.
    pub fn ok() -> Self {
        Self {
            ok: true,
            value: Some(()),
            error: None,
        }
    }

    /// Failed browser Domain result with a stable SDK/browser error code.
    pub fn fail(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(BrowserDomainError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// UI-facing result shape shared with the TypeScript browser facade.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserDomainResult {
    /// Whether the operation succeeded.
    pub ok: bool,
    /// Present for successful void operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<()>,
    /// Present for failed operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BrowserDomainError>,
}

/// UI-facing browser Domain error.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserDomainError {
    /// Stable machine-readable error code.
    pub code: &'static str,
    /// Human-readable error message.
    pub message: String,
}

//! Credential-to-authority preparation for an Auki P2P peer.
//!
//! This crate authenticates a User or trusted native App through the Auki API,
//! asks DDS which Domains that principal may enter, proves ownership of one
//! libp2p identity, and returns a validated [`PreparedPeer`].
//!
//! The high-level `auki_sdk::AukiPeer::start` operation consumes the identity
//! and prepared authority, then owns credential renewal, relay booking,
//! authenticated transport, protocols, fencing, and shutdown. This crate alone
//! does not discover peers, resolve or publish routes, contact DMS, book a
//! relay, or spawn background tasks.
//!
//! ```no_run
//! use auki_auth::{AuthClient, AuthEnvironment, Credentials, DomainSelection};
//! use auki_p2p::Identity;
//!
//! # async fn prepare() -> Result<(), Box<dyn std::error::Error>> {
//! let identity = Identity::load_or_create("./state/auki-peer.identity")?;
//! let session = AuthClient::new(AuthEnvironment::dev())?
//!     .authenticate(Credentials::user_password(
//!         "developer@example.com",
//!         "password-from-a-secret-store",
//!     ))
//!     .await?;
//! let selected = session
//!     .accessible_domains()
//!     .await?
//!     .into_iter()
//!     .next()
//!     .ok_or("no accessible Domain")?;
//! let prepared = session
//!     .authorize_peer(
//!         DomainSelection::new(selected.domain.id),
//!         &identity.proof(),
//!     )
//!     .await?;
//!
//! // Pass `identity` and `prepared` to `auki_sdk::AukiPeer::start`.
//! # let _ = (identity, prepared);
//! # Ok(())
//! # }
//! ```
//!
//! User authentication and authority preparation also compile for Wasm. The
//! generic Web facade wraps them in `AukiUserSession`, creates a fresh in-memory
//! identity for each `0.1` peer start, and requires one confirmed WSS relay.
//! App credentials remain native-only because their secret must never ship to a
//! browser. Peer discovery and automatic route publication are separate work
//! on every platform.

mod client;
mod error;
mod secret;
mod types;
mod wire;

pub use client::{AuthClient, AuthEnvironment, AuthLimits, AuthSession};
pub use error::{Error, Result};
pub use secret::SecretString;
#[cfg(not(target_arch = "wasm32"))]
pub use types::AppCredentials;
pub use types::{
    AuthorityRenewal, AuthorityRenewalProvider, Credentials, DomainChoice, DomainDescriptor,
    DomainSelection, PeerAuthorityProvider, PreparedPeer, PrincipalKind, RenewedAuthority,
    UserPassword,
};

#[cfg(test)]
mod tests;

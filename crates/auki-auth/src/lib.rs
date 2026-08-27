//! Native credential-to-authority preparation for an Auki P2P peer.
//!
//! This crate authenticates users or trusted applications through the Auki
//! API, asks DDS which Domains that principal may enter, proves ownership of a
//! stable libp2p identity, and returns a validated [`PreparedPeer`]. It does not
//! join a Domain, discover routes, book relays, or run background tasks.
//!
//! ```no_run
//! use auki_auth::{AuthClient, AuthEnvironment, Credentials, DomainSelection};
//! use auki_domain::{Domain, DomainConfig};
//! use auki_p2p::Identity;
//! use auki_session::Peer;
//!
//! # async fn experiment() -> Result<(), Box<dyn std::error::Error>> {
//! let client = AuthClient::new(AuthEnvironment::dev())?;
//! let session = client
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
//! // Reuse this file on every launch. Losing it changes the Peer ID.
//! let identity = Identity::load_or_create("./state/auki-peer.identity")?;
//! let prepared = session
//!     .authorize_peer(
//!         DomainSelection::new(selected.domain.id),
//!         &identity.proof(),
//!     )
//!     .await?;
//!
//! // Starting a Domain is an explicit, mechanical composition step. Route
//! // discovery remains a separate concern.
//! let peer = Peer::new(prepared.peer_id.to_string(), "robot-experiment");
//! let sdk_session = peer.start_session()?;
//! let domain = Domain::builder(
//!     &peer,
//!     &sdk_session,
//!     DomainConfig::new(prepared.domain.id, identity),
//! )
//! .authority(
//!     prepared.verification_keys.clone(),
//!     prepared.initial_credential.clone(),
//! )
//! .join()
//! .await?;
//!
//! // Renewal is deliberately host-driven. Install keys before the credential
//! // on the existing Domain; overlap keeps the old credential usable if the
//! // ordered update is interrupted. No reconnect is needed.
//! let renewed = prepared.renewal.renew().await?;
//! let authority = domain.authority();
//! authority
//!     .install_verification_keys(renewed.verification_keys)
//!     .await?;
//! authority.install_credential(renewed.credential).await?;
//! domain.leave().await?;
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
mod secret;
mod types;
mod wire;

pub use client::{AuthClient, AuthEnvironment, AuthLimits, AuthSession};
pub use error::{Error, Result};
pub use secret::SecretString;
pub use types::{
    AppCredentials, AuthorityRenewal, AuthorityRenewalProvider, Credentials, DomainChoice,
    DomainDescriptor, DomainSelection, PeerAuthorityProvider, PreparedPeer, PrincipalKind,
    RenewedAuthority, UserPassword,
};

#[cfg(test)]
mod tests;

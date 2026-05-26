//! Authority-chain validation for v1 peer relationships.

use super::{
    domain::{
        DelegationScope, DomainDeclaration, DomainDelegation, DomainError,
        VerifiedDomainDeclaration, VerifiedDomainDelegation, decode_domain_id,
    },
    error,
    identity::{PeerBinding, PeerBindingError, VerifiedPeerBinding},
};
use auki_identity::PublicKey as WalletPublicKey;
use libp2p_identity::PeerId;
use serde_json::Value;
use std::{fmt, str::FromStr};

const FIELD_DOMAIN_ID: &str = "domain_id";
const FIELD_DOMAIN_DECLARATION: &str = "domain_declaration";
const FIELD_DELEGATION: &str = "delegation";
const FIELD_METADATA: &str = "metadata";

/// Local peer authorization decision for the authority-chain path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAuthorization {
    /// Accept this peer after peer-binding verification.
    Authorized,
    /// Reject this peer after peer-binding verification.
    Rejected,
}

/// Baseline peer authorization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAuthorizationMode {
    /// Accept any peer with a valid peer binding.
    All,
    /// Accept only configured peer ids or wallet public keys.
    WhitelistedOnly,
    /// Defer allow or deny to application policy.
    AppPolicy,
}

impl PeerAuthorizationMode {
    /// Return the RFC string value for this authorization mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::WhitelistedOnly => "whitelisted-only",
            Self::AppPolicy => "app-policy",
        }
    }
}

impl Default for PeerAuthorizationMode {
    fn default() -> Self {
        Self::All
    }
}

impl fmt::Display for PeerAuthorizationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PeerAuthorizationMode {
    type Err = PeerAuthorizationModeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "all" => Ok(Self::All),
            "whitelisted-only" => Ok(Self::WhitelistedOnly),
            "app-policy" => Ok(Self::AppPolicy),
            _ => Err(PeerAuthorizationModeError::UnsupportedMode {
                actual: value.to_owned(),
            }),
        }
    }
}

/// Errors produced while parsing peer authorization modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAuthorizationModeError {
    /// Authorization mode string was unsupported.
    UnsupportedMode {
        /// Actual mode string.
        actual: String,
    },
}

impl fmt::Display for PeerAuthorizationModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMode { actual } => {
                write!(f, "unsupported peer authorization mode {actual}")
            }
        }
    }
}

impl std::error::Error for PeerAuthorizationModeError {}

/// Local policy used to evaluate a verified peer binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAuthorizationPolicy<'a> {
    /// Accept any peer with a valid peer binding.
    All,
    /// Accept only peers whose peer id or wallet public key appears in a whitelist.
    WhitelistedOnly {
        /// Configured libp2p peer ids.
        peer_ids: &'a [PeerId],
        /// Configured wallet public keys.
        wallet_public_keys: &'a [WalletPublicKey],
    },
    /// Application code supplies the already evaluated allow or deny decision.
    AppPolicy {
        /// Application policy decision.
        decision: PeerAuthorization,
    },
}

impl<'a> PeerAuthorizationPolicy<'a> {
    /// Create an `all` authorization policy.
    pub fn all() -> Self {
        Self::All
    }

    /// Create a `whitelisted-only` authorization policy.
    pub fn whitelisted_only(
        peer_ids: &'a [PeerId],
        wallet_public_keys: &'a [WalletPublicKey],
    ) -> Self {
        Self::WhitelistedOnly {
            peer_ids,
            wallet_public_keys,
        }
    }

    /// Create an `app-policy` authorization policy from an application decision.
    pub fn app_policy(decision: PeerAuthorization) -> Self {
        Self::AppPolicy { decision }
    }

    /// Return the baseline mode represented by this policy.
    pub fn mode(&self) -> PeerAuthorizationMode {
        match self {
            Self::All => PeerAuthorizationMode::All,
            Self::WhitelistedOnly { .. } => PeerAuthorizationMode::WhitelistedOnly,
            Self::AppPolicy { .. } => PeerAuthorizationMode::AppPolicy,
        }
    }

    /// Evaluate this policy against a verified peer binding.
    pub fn evaluate(&self, peer: &VerifiedPeerBinding) -> PeerAuthorization {
        match self {
            Self::All => PeerAuthorization::Authorized,
            Self::WhitelistedOnly {
                peer_ids,
                wallet_public_keys,
            } => {
                let peer_allowed = peer_ids.iter().any(|peer_id| peer_id == &peer.peer_id);
                let wallet_allowed = wallet_public_keys
                    .iter()
                    .any(|wallet_public_key| wallet_public_key == &peer.wallet_public_key);
                if peer_allowed || wallet_allowed {
                    PeerAuthorization::Authorized
                } else {
                    PeerAuthorization::Rejected
                }
            }
            Self::AppPolicy { decision } => *decision,
        }
    }
}

impl Default for PeerAuthorizationPolicy<'_> {
    fn default() -> Self {
        Self::All
    }
}

/// A declared-domain object from a v1 peer handshake.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredDomain {
    /// Domain id claimed by the declared-domain wrapper.
    pub domain_id: String,
    /// Domain declaration object for `domain_id`.
    pub domain_declaration: DomainDeclaration,
    /// Optional delegation authorizing the remote peer.
    pub delegation: Option<DomainDelegation>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
}

/// Authority accepted for one served domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedServedDomain {
    /// Accepted domain id.
    pub domain_id: String,
    /// Whether authority came directly from owner wallet or delegation.
    pub authority: ServedDomainAuthority,
    /// Verified domain declaration.
    pub domain_declaration: VerifiedDomainDeclaration,
    /// Verified delegation when delegated authority was required.
    pub delegation: Option<VerifiedDomainDelegation>,
}

/// How a served domain was authorized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServedDomainAuthority {
    /// Remote peer wallet is the domain owner wallet.
    DirectOwner,
    /// Remote peer is authorized by a domain-owner delegation.
    Delegated,
}

/// Domain-level authority-chain rejection diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedDeclaredDomain {
    /// Domain id from the declared-domain wrapper, if available.
    pub domain_id: Option<String>,
    /// Stable RFC failure code.
    pub failure_code: &'static str,
    /// Structured rejection reason.
    pub reason: DomainRejectionReason,
}

/// Structured reason for a domain-level rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainRejectionReason {
    /// Declared-domain wrapper was malformed.
    InvalidDeclaredDomain(String),
    /// Domain declaration was malformed or invalid.
    InvalidDeclaration(DomainError),
    /// Declared-domain wrapper id did not match the verified declaration id.
    DomainIdMismatch {
        /// Domain id from declared-domain wrapper.
        declared: String,
        /// Verified domain declaration id.
        declaration: String,
    },
    /// Remote peer is not the owner wallet and no delegation was present.
    MissingDelegation,
    /// Delegation was malformed, mismatched, or otherwise invalid.
    InvalidDelegation(DomainError),
    /// Delegation has expired.
    ExpiredDelegation(DomainError),
}

/// Result of validating a remote peer authority chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityChain {
    /// Verified remote peer binding.
    pub peer: VerifiedPeerBinding,
    /// Domains the peer may serve.
    pub accepted_served_domains: Vec<AcceptedServedDomain>,
    /// Declared domains rejected independently.
    pub rejected_declared_domains: Vec<RejectedDeclaredDomain>,
}

/// Input for validating one remote peer authority chain.
pub struct AuthorityChainInput<'a> {
    /// Remote peer binding object from the handshake.
    pub peer_binding: Option<&'a PeerBinding>,
    /// Transport-authenticated libp2p peer id for the remote connection.
    pub authenticated_peer_id: &'a PeerId,
    /// Local peer authorization result.
    pub peer_authorization: PeerAuthorization,
    /// Declared domains from the remote handshake.
    pub declared_domains: &'a [DeclaredDomain],
    /// Receiver's current UTC time as RFC3339 `Z` string.
    pub now: &'a str,
}

/// Input for validating one remote peer authority chain with a peer authorization policy.
pub struct AuthorityChainPolicyInput<'a> {
    /// Remote peer binding object from the handshake.
    pub peer_binding: Option<&'a PeerBinding>,
    /// Transport-authenticated libp2p peer id for the remote connection.
    pub authenticated_peer_id: &'a PeerId,
    /// Local peer authorization policy.
    pub peer_authorization_policy: PeerAuthorizationPolicy<'a>,
    /// Declared domains from the remote handshake.
    pub declared_domains: &'a [DeclaredDomain],
    /// Receiver's current UTC time as RFC3339 `Z` string.
    pub now: &'a str,
}

/// Fatal peer-level authority-chain errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityChainError {
    /// Peer binding was absent from the handshake.
    MissingPeerBinding,
    /// Peer binding failed RFC-0005 verification.
    InvalidPeerBinding(PeerBindingError),
    /// Local peer authorization rejected the verified peer.
    PeerRejected,
}

impl AuthorityChainError {
    /// Stable RFC failure code for this fatal error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::MissingPeerBinding => error::IDENTITY_MISSING_PEER_BINDING,
            Self::InvalidPeerBinding(PeerBindingError::InvalidSignature) => {
                error::IDENTITY_INVALID_SIGNATURE
            }
            Self::InvalidPeerBinding(PeerBindingError::PeerIdMismatch { .. }) => {
                error::IDENTITY_PEER_ID_MISMATCH
            }
            Self::InvalidPeerBinding(_) => error::IDENTITY_INVALID_PEER_BINDING,
            Self::PeerRejected => error::AUTHORIZATION_PEER_REJECTED,
        }
    }
}

impl fmt::Display for AuthorityChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPeerBinding => write!(f, "peer binding is missing"),
            Self::InvalidPeerBinding(error) => write!(f, "invalid peer binding: {error}"),
            Self::PeerRejected => write!(f, "peer authorization rejected remote peer"),
        }
    }
}

impl std::error::Error for AuthorityChainError {}

impl DeclaredDomain {
    /// Construct a declared domain from already parsed authority objects.
    pub fn new(
        domain_id: impl Into<String>,
        domain_declaration: DomainDeclaration,
        delegation: Option<DomainDelegation>,
    ) -> Self {
        Self {
            domain_id: domain_id.into(),
            domain_declaration,
            delegation,
            metadata: None,
        }
    }

    /// Parse a declared-domain JSON object.
    pub fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "declared domain is not a json object".to_owned())?;

        let domain_id = object
            .get(FIELD_DOMAIN_ID)
            .ok_or_else(|| "declared domain missing domain_id".to_owned())?
            .as_str()
            .ok_or_else(|| "declared domain domain_id is not a string".to_owned())?
            .to_owned();
        decode_domain_id(&domain_id).map_err(|error| error.to_string())?;

        let domain_declaration = object
            .get(FIELD_DOMAIN_DECLARATION)
            .ok_or_else(|| "declared domain missing domain_declaration".to_owned())
            .and_then(|value| {
                DomainDeclaration::from_value(value.clone()).map_err(|error| error.to_string())
            })?;

        let delegation = object
            .get(FIELD_DELEGATION)
            .map(|value| {
                DomainDelegation::from_value(value.clone()).map_err(|error| error.to_string())
            })
            .transpose()?;

        let metadata = object
            .get(FIELD_METADATA)
            .map(|value| {
                if value.is_object() {
                    Ok(value.clone())
                } else {
                    Err("declared domain metadata is not an object".to_owned())
                }
            })
            .transpose()?;

        Ok(Self {
            domain_id,
            domain_declaration,
            delegation,
            metadata,
        })
    }
}

/// Validate one remote peer authority chain.
pub fn validate_authority_chain(
    input: AuthorityChainInput<'_>,
) -> Result<AuthorityChain, AuthorityChainError> {
    let peer = verify_remote_peer(input.peer_binding, input.authenticated_peer_id)?;

    validate_verified_authority_chain(
        peer,
        input.peer_authorization,
        input.declared_domains,
        input.now,
    )
}

/// Validate one remote peer authority chain and evaluate peer authorization from policy.
pub fn validate_authority_chain_with_authorization_policy(
    input: AuthorityChainPolicyInput<'_>,
) -> Result<AuthorityChain, AuthorityChainError> {
    let peer = verify_remote_peer(input.peer_binding, input.authenticated_peer_id)?;
    let peer_authorization = input.peer_authorization_policy.evaluate(&peer);

    validate_verified_authority_chain(peer, peer_authorization, input.declared_domains, input.now)
}

fn verify_remote_peer(
    peer_binding: Option<&PeerBinding>,
    authenticated_peer_id: &PeerId,
) -> Result<VerifiedPeerBinding, AuthorityChainError> {
    let peer_binding = peer_binding.ok_or(AuthorityChainError::MissingPeerBinding)?;
    peer_binding
        .verify_for_peer_id(authenticated_peer_id)
        .map_err(AuthorityChainError::InvalidPeerBinding)
}

fn validate_verified_authority_chain(
    peer: VerifiedPeerBinding,
    peer_authorization: PeerAuthorization,
    declared_domains: &[DeclaredDomain],
    now: &str,
) -> Result<AuthorityChain, AuthorityChainError> {
    if peer_authorization == PeerAuthorization::Rejected {
        return Err(AuthorityChainError::PeerRejected);
    }

    let mut accepted_served_domains = Vec::new();
    let mut rejected_declared_domains = Vec::new();

    for declared_domain in declared_domains {
        match validate_declared_domain(declared_domain, &peer, now) {
            Ok(accepted) => accepted_served_domains.push(accepted),
            Err(rejected) => rejected_declared_domains.push(rejected),
        }
    }

    Ok(AuthorityChain {
        peer,
        accepted_served_domains,
        rejected_declared_domains,
    })
}

fn validate_declared_domain(
    declared_domain: &DeclaredDomain,
    peer: &VerifiedPeerBinding,
    now: &str,
) -> Result<AcceptedServedDomain, RejectedDeclaredDomain> {
    if let Err(error) = decode_domain_id(&declared_domain.domain_id) {
        return Err(reject_domain(
            Some(declared_domain.domain_id.clone()),
            error::DOMAIN_INVALID_DECLARATION,
            DomainRejectionReason::InvalidDeclaration(error),
        ));
    }

    let domain_declaration = declared_domain
        .domain_declaration
        .verify()
        .map_err(|error| {
            let failure_code = declaration_failure_code(&error);
            reject_domain(
                Some(declared_domain.domain_id.clone()),
                failure_code,
                DomainRejectionReason::InvalidDeclaration(error),
            )
        })?;

    if declared_domain.domain_id != domain_declaration.domain_id {
        return Err(reject_domain(
            Some(declared_domain.domain_id.clone()),
            error::DOMAIN_ID_MISMATCH,
            DomainRejectionReason::DomainIdMismatch {
                declared: declared_domain.domain_id.clone(),
                declaration: domain_declaration.domain_id.clone(),
            },
        ));
    }

    if peer.wallet_public_key == domain_declaration.domain_owner_public_key {
        return Ok(AcceptedServedDomain {
            domain_id: domain_declaration.domain_id.clone(),
            authority: ServedDomainAuthority::DirectOwner,
            domain_declaration,
            delegation: None,
        });
    }

    let Some(delegation) = &declared_domain.delegation else {
        return Err(reject_domain(
            Some(domain_declaration.domain_id.clone()),
            error::DOMAIN_MISSING_DELEGATION,
            DomainRejectionReason::MissingDelegation,
        ));
    };

    let verified_delegation = delegation
        .verify_for_authority(
            &domain_declaration.domain_id,
            &domain_declaration.domain_owner_public_key,
            &peer.wallet_public_key,
            &peer.peer_id,
            DelegationScope::Serve,
            now,
        )
        .map_err(|error| {
            let failure_code = delegation_failure_code(&error);
            let reason = if failure_code == error::DOMAIN_EXPIRED_DELEGATION {
                DomainRejectionReason::ExpiredDelegation(error)
            } else {
                DomainRejectionReason::InvalidDelegation(error)
            };
            reject_domain(
                Some(domain_declaration.domain_id.clone()),
                failure_code,
                reason,
            )
        })?;

    Ok(AcceptedServedDomain {
        domain_id: domain_declaration.domain_id.clone(),
        authority: ServedDomainAuthority::Delegated,
        domain_declaration,
        delegation: Some(verified_delegation),
    })
}

fn declaration_failure_code(error: &DomainError) -> &'static str {
    match error {
        DomainError::DomainIdMismatch { .. } => error::DOMAIN_ID_MISMATCH,
        _ => error::DOMAIN_INVALID_DECLARATION,
    }
}

fn delegation_failure_code(error: &DomainError) -> &'static str {
    match error {
        DomainError::DelegationExpired { .. } => error::DOMAIN_EXPIRED_DELEGATION,
        _ => error::DOMAIN_INVALID_DELEGATION,
    }
}

fn reject_domain(
    domain_id: Option<String>,
    failure_code: &'static str,
    reason: DomainRejectionReason,
) -> RejectedDeclaredDomain {
    RejectedDeclaredDomain {
        domain_id,
        failure_code,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::{
        base64url,
        domain::{DOMAIN_NONCE_LEN, derive_domain_id},
    };
    use auki_identity::Wallet;
    use serde_json::json;
    use std::str::FromStr;

    const PEER_ID: &str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
    const OTHER_PEER_ID: &str = "12D3KooWFU1bqozGMWdqN2Ckh2YHNbr9n5Lypw6uJrNkbm2ptVbF";
    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";
    const VALID_FROM: &str = "2026-05-26T11:00:00Z";
    const EXPIRES_AT: &str = "2026-05-26T13:00:00Z";
    const NOW: &str = "2026-05-26T12:00:00Z";
    const NONCE: [u8; DOMAIN_NONCE_LEN] = [7u8; DOMAIN_NONCE_LEN];

    fn owner_wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed")
    }

    fn delegate_wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![4u8; 32]).expect("32-byte seed")
    }

    fn peer_id() -> PeerId {
        PeerId::from_str(PEER_ID).expect("valid peer id")
    }

    fn other_peer_id() -> PeerId {
        PeerId::from_str(OTHER_PEER_ID).expect("valid peer id")
    }

    fn owner_domain_declaration() -> DomainDeclaration {
        DomainDeclaration::create(&owner_wallet(), &NONCE, Some("warehouse")).unwrap()
    }

    fn owner_domain_id() -> String {
        derive_domain_id(&owner_wallet().public_key(), &NONCE)
    }

    fn peer_binding(wallet: &Wallet, peer_id: &PeerId) -> PeerBinding {
        PeerBinding::create(wallet, peer_id, ISSUED_AT, None).unwrap()
    }

    fn delegated_domain() -> DeclaredDomain {
        let domain_id = owner_domain_id();
        let delegation = DomainDelegation::create(
            &owner_wallet(),
            &domain_id,
            &delegate_wallet().public_key(),
            &peer_id(),
            &[DelegationScope::Serve],
            VALID_FROM,
            EXPIRES_AT,
            None,
        )
        .unwrap();

        DeclaredDomain::new(domain_id, owner_domain_declaration(), Some(delegation))
    }

    #[test]
    fn parses_peer_authorization_modes() {
        assert_eq!(
            "all".parse::<PeerAuthorizationMode>().unwrap(),
            PeerAuthorizationMode::All
        );
        assert_eq!(
            "whitelisted-only".parse::<PeerAuthorizationMode>().unwrap(),
            PeerAuthorizationMode::WhitelistedOnly
        );
        assert_eq!(
            "app-policy".parse::<PeerAuthorizationMode>().unwrap(),
            PeerAuthorizationMode::AppPolicy
        );
        assert_eq!(PeerAuthorizationMode::default(), PeerAuthorizationMode::All);
        assert_eq!(PeerAuthorizationMode::AppPolicy.as_str(), "app-policy");
        assert_eq!(
            "open".parse::<PeerAuthorizationMode>(),
            Err(PeerAuthorizationModeError::UnsupportedMode {
                actual: "open".to_owned(),
            })
        );
    }

    #[test]
    fn peer_authorization_policy_evaluates_all_whitelist_and_app_policy() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let verified = binding.verify_for_peer_id(&peer_id()).unwrap();
        let allowed_peer_ids = vec![peer_id()];
        let denied_peer_ids = vec![other_peer_id()];
        let allowed_wallets = vec![delegate_wallet().public_key()];
        let denied_wallets = vec![owner_wallet().public_key()];

        assert_eq!(
            PeerAuthorizationPolicy::all().evaluate(&verified),
            PeerAuthorization::Authorized
        );
        assert_eq!(
            PeerAuthorizationPolicy::whitelisted_only(&allowed_peer_ids, &[]).evaluate(&verified),
            PeerAuthorization::Authorized
        );
        assert_eq!(
            PeerAuthorizationPolicy::whitelisted_only(&[], &allowed_wallets).evaluate(&verified),
            PeerAuthorization::Authorized
        );
        assert_eq!(
            PeerAuthorizationPolicy::whitelisted_only(&denied_peer_ids, &denied_wallets)
                .evaluate(&verified),
            PeerAuthorization::Rejected
        );
        assert_eq!(
            PeerAuthorizationPolicy::app_policy(PeerAuthorization::Rejected).evaluate(&verified),
            PeerAuthorization::Rejected
        );
        assert_eq!(
            PeerAuthorizationPolicy::whitelisted_only(&allowed_peer_ids, &[]).mode(),
            PeerAuthorizationMode::WhitelistedOnly
        );
    }

    #[test]
    fn validates_direct_owner_served_domain_without_delegation() {
        let binding = peer_binding(&owner_wallet(), &peer_id());
        let declared = DeclaredDomain::new(owner_domain_id(), owner_domain_declaration(), None);

        let result = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap();

        assert_eq!(result.accepted_served_domains.len(), 1);
        assert_eq!(result.rejected_declared_domains, vec![]);
        assert_eq!(
            result.accepted_served_domains[0].authority,
            ServedDomainAuthority::DirectOwner
        );
        assert!(result.accepted_served_domains[0].delegation.is_none());
    }

    #[test]
    fn validates_delegated_served_domain() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let declared = delegated_domain();

        let result = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap();

        assert_eq!(result.accepted_served_domains.len(), 1);
        assert_eq!(result.rejected_declared_domains, vec![]);
        assert_eq!(
            result.accepted_served_domains[0].authority,
            ServedDomainAuthority::Delegated
        );
        assert!(result.accepted_served_domains[0].delegation.is_some());
    }

    #[test]
    fn rejects_missing_peer_binding_before_domains() {
        let declared = delegated_domain();

        let error = validate_authority_chain(AuthorityChainInput {
            peer_binding: None,
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap_err();

        assert_eq!(error, AuthorityChainError::MissingPeerBinding);
        assert_eq!(error.failure_code(), error::IDENTITY_MISSING_PEER_BINDING);
    }

    #[test]
    fn rejects_peer_authorization_before_domains() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let declared = delegated_domain();

        let error = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Rejected,
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap_err();

        assert_eq!(error, AuthorityChainError::PeerRejected);
        assert_eq!(error.failure_code(), error::AUTHORIZATION_PEER_REJECTED);
    }

    #[test]
    fn validates_authority_chain_with_whitelisted_peer_authorization_policy() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let declared = delegated_domain();
        let allowed_peer_ids = vec![peer_id()];

        let result =
            validate_authority_chain_with_authorization_policy(AuthorityChainPolicyInput {
                peer_binding: Some(&binding),
                authenticated_peer_id: &peer_id(),
                peer_authorization_policy: PeerAuthorizationPolicy::whitelisted_only(
                    &allowed_peer_ids,
                    &[],
                ),
                declared_domains: &[declared],
                now: NOW,
            })
            .unwrap();

        assert_eq!(result.accepted_served_domains.len(), 1);
        assert_eq!(result.rejected_declared_domains, vec![]);
    }

    #[test]
    fn rejects_authority_chain_with_empty_whitelist_before_domains() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let declared = delegated_domain();

        let error = validate_authority_chain_with_authorization_policy(AuthorityChainPolicyInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization_policy: PeerAuthorizationPolicy::whitelisted_only(&[], &[]),
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap_err();

        assert_eq!(error, AuthorityChainError::PeerRejected);
        assert_eq!(error.failure_code(), error::AUTHORIZATION_PEER_REJECTED);
    }

    #[test]
    fn rejects_peer_id_mismatch_as_peer_level_failure() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let declared = delegated_domain();

        let error = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &other_peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap_err();

        assert_eq!(error.failure_code(), error::IDENTITY_PEER_ID_MISMATCH);
    }

    #[test]
    fn rejects_declared_domain_id_mismatch() {
        let binding = peer_binding(&owner_wallet(), &peer_id());
        let declared = DeclaredDomain::new(
            base64url::encode(&[0u8; 32]),
            owner_domain_declaration(),
            None,
        );

        let result = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap();

        assert_eq!(result.accepted_served_domains, vec![]);
        assert_eq!(result.rejected_declared_domains.len(), 1);
        assert_eq!(
            result.rejected_declared_domains[0].failure_code,
            error::DOMAIN_ID_MISMATCH
        );
    }

    #[test]
    fn rejects_missing_delegation_when_peer_is_not_owner() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let declared = DeclaredDomain::new(owner_domain_id(), owner_domain_declaration(), None);

        let result = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[declared],
            now: NOW,
        })
        .unwrap();

        assert_eq!(result.accepted_served_domains, vec![]);
        assert_eq!(result.rejected_declared_domains.len(), 1);
        assert_eq!(
            result.rejected_declared_domains[0].failure_code,
            error::DOMAIN_MISSING_DELEGATION
        );
        assert_eq!(
            result.rejected_declared_domains[0].reason,
            DomainRejectionReason::MissingDelegation
        );
    }

    #[test]
    fn rejects_expired_delegation_with_expiry_failure_code() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let declared = delegated_domain();

        let result = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[declared],
            now: EXPIRES_AT,
        })
        .unwrap();

        assert_eq!(result.accepted_served_domains, vec![]);
        assert_eq!(result.rejected_declared_domains.len(), 1);
        assert_eq!(
            result.rejected_declared_domains[0].failure_code,
            error::DOMAIN_EXPIRED_DELEGATION
        );
        assert!(matches!(
            result.rejected_declared_domains[0].reason,
            DomainRejectionReason::ExpiredDelegation(_)
        ));
    }

    #[test]
    fn keeps_domain_diagnostics_independent() {
        let binding = peer_binding(&delegate_wallet(), &peer_id());
        let accepted = delegated_domain();
        let rejected = DeclaredDomain::new(owner_domain_id(), owner_domain_declaration(), None);

        let result = validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&binding),
            authenticated_peer_id: &peer_id(),
            peer_authorization: PeerAuthorization::Authorized,
            declared_domains: &[accepted, rejected],
            now: NOW,
        })
        .unwrap();

        assert_eq!(result.accepted_served_domains.len(), 1);
        assert_eq!(result.rejected_declared_domains.len(), 1);
        assert_eq!(
            result.rejected_declared_domains[0].failure_code,
            error::DOMAIN_MISSING_DELEGATION
        );
    }

    #[test]
    fn parses_declared_domain_value() {
        let declaration = owner_domain_declaration();
        let delegation = delegated_domain().delegation.unwrap();
        let value = json!({
            "domain_id": owner_domain_id(),
            "domain_declaration": declaration.into_value(),
            "delegation": delegation.into_value(),
            "metadata": {"note": "local"}
        });

        let parsed = DeclaredDomain::from_value(value).unwrap();

        assert_eq!(parsed.domain_id, owner_domain_id());
        assert!(parsed.delegation.is_some());
        assert_eq!(parsed.metadata, Some(json!({"note": "local"})));
    }
}

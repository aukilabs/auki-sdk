//! Runtime configuration and policy types for `auki-p2p`.

use auki_identity::PublicKey as WalletPublicKey;
use auki_protocol::v1::{
    authority::{PeerAuthorization, PeerAuthorizationMode, PeerAuthorizationPolicy},
    identity::PeerBindingFreshnessPolicy,
};
use libp2p_identity::PeerId;
use multiaddr::{Multiaddr, Protocol};
use serde_json::Value;
use std::{fmt, net::Ipv6Addr};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

/// Top-level runtime configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct AukiP2pConfig {
    /// Runtime size/count limits.
    pub limits: RuntimeLimits,
    /// Peer-binding freshness enforcement.
    pub peer_binding_freshness: PeerBindingFreshnessConfig,
    /// Peer admission mode.
    pub peer_admission: PeerAdmissionConfig,
    /// Local domain access policy hook.
    pub domain_access_policy: DomainAccessPolicy,
    /// Local offer policy hook.
    pub offer_policy: OfferPolicy,
    /// Dial policy for configured and future Discovery addresses.
    pub dial_policy: DialPolicy,
    /// Status privacy and redaction behavior.
    pub status_privacy: StatusPrivacyConfig,
    /// Authority deadline behavior.
    pub authority_deadline: AuthorityDeadlineConfig,
    /// Explicitly configured peers.
    pub configured_peers: Vec<ConfiguredPeer>,
}

/// Runtime size/count limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    /// Lifecycle handshake frame body limit.
    pub handshake_frame_body_bytes: u64,
    /// Offer-catalog response frame body limit.
    pub catalog_response_frame_body_bytes: u64,
    /// Get response frame body limit.
    pub get_response_frame_body_bytes: u64,
    /// Subscribe data-message frame body limit.
    pub subscribe_message_frame_body_bytes: u64,
    /// Maximum declared domains per handshake.
    pub declared_domains_per_handshake: usize,
    /// Maximum authorization-material entries per handshake.
    pub authorization_material_entries: usize,
    /// Maximum offers per catalog response.
    pub offers_per_catalog: usize,
    /// Maximum registry refs per offer.
    pub registry_refs_per_offer: usize,
    /// Maximum inline canonical registry JSON bytes per ref.
    pub inline_registry_json_bytes: u64,
    /// Maximum serialized metadata bytes for one object.
    pub metadata_bytes: u64,
    /// Maximum active subscriptions per peer.
    pub active_subscriptions_per_peer: usize,
    /// Maximum active libp2p connections per peer id.
    pub active_connections_per_peer_id: usize,
    /// Retained status failure records.
    pub retained_status_failures: usize,
    /// Retained completed path history entries.
    pub completed_path_history: usize,
    /// Repeated failures retained per minute before aggregation/rate limiting.
    pub repeated_failure_rate_limit_per_minute: usize,
}

/// Peer-binding freshness configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerBindingFreshnessConfig {
    /// Whether freshness is enforced.
    pub enforce: bool,
    /// Maximum accepted binding age in milliseconds.
    pub max_age_ms: u64,
    /// Maximum accepted future skew in milliseconds.
    pub future_tolerance_ms: u64,
}

/// Peer admission configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAdmissionConfig {
    /// Accept any peer with a valid peer binding.
    All,
    /// Accept only configured peer ids or wallet public keys.
    WhitelistedOnly {
        /// Allowed libp2p peer ids.
        peer_ids: Vec<PeerId>,
        /// Allowed wallet public keys.
        wallet_public_keys: Vec<WalletPublicKey>,
    },
    /// Defer the allow/deny decision to application policy.
    AppPolicy,
}

/// Local domain access policy hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainAccessPolicy {
    /// Allow otherwise-authorized domains.
    AllowAll,
    /// Application code must decide per domain.
    AppPolicy,
}

/// Local offer policy hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferPolicy {
    /// Allow otherwise-usable offers.
    AllowAll,
    /// Application code must decide per offer.
    AppPolicy,
}

/// Dial policy for manually configured and future Discovery addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialPolicy {
    /// Allow loopback addresses.
    pub allow_loopback: bool,
    /// Allow unspecified addresses.
    pub allow_unspecified: bool,
    /// Allow link-local addresses.
    pub allow_link_local: bool,
    /// Allow private/local IPv4 or IPv6 ranges.
    pub allow_private_ip: bool,
    /// Allow relay-mediated paths.
    pub allow_relay: bool,
    /// Allow DNS-based addresses.
    pub allow_dns: bool,
}

/// Status redaction configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusPrivacyConfig {
    /// Redact addresses.
    pub redact_addresses: bool,
    /// Redact labels.
    pub redact_labels: bool,
    /// Redact metadata.
    pub redact_metadata: bool,
    /// Redact diagnostics.
    pub redact_diagnostics: bool,
    /// Redact authorization-material hints.
    pub redact_authorization_material: bool,
}

/// Authority deadline configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityDeadlineConfig {
    /// Include peer-binding freshness in authority deadlines.
    pub include_peer_binding_freshness: bool,
    /// Include delegation expiry in authority deadlines.
    pub include_delegation_expiry: bool,
    /// Include authorization-material expiry in authority deadlines.
    pub include_authorization_material_expiry: bool,
    /// Include offer expiry when enforced.
    pub include_offer_expiry: bool,
    /// Optional local session lifetime in milliseconds.
    pub local_session_lifetime_ms: Option<u64>,
}

/// Explicitly configured peer.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfiguredPeer {
    /// Expected libp2p peer id.
    pub peer_id: PeerId,
    /// Expected wallet public key.
    pub wallet_public_key: Option<WalletPublicKey>,
    /// Dial addresses for this peer.
    pub dial_addresses: Vec<Multiaddr>,
    /// Expected advertised addresses, when configured separately.
    pub advertised_addresses: Vec<Multiaddr>,
    /// Non-authoritative local metadata.
    pub metadata: Option<Value>,
}

/// Configuration errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A limit was zero.
    ZeroLimit {
        /// Limit name.
        field: &'static str,
    },
    /// App-policy conversion was requested without an app decision.
    MissingAppPolicyDecision,
}

/// Dial-policy rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialPolicyError {
    /// Rejection reason.
    pub reason: DialPolicyRejection,
    /// Address that was rejected.
    pub address: Multiaddr,
}

/// Dial-policy rejection reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialPolicyRejection {
    /// Loopback address was not allowed.
    Loopback,
    /// Unspecified address was not allowed.
    Unspecified,
    /// Link-local address was not allowed.
    LinkLocal,
    /// Private/local address was not allowed.
    PrivateIp,
    /// IPv4 broadcast address was not allowed.
    Broadcast,
    /// Relay path was not allowed.
    Relay,
    /// DNS address was not allowed.
    Dns,
}

impl Default for AukiP2pConfig {
    fn default() -> Self {
        Self::development()
    }
}

impl AukiP2pConfig {
    /// Development-friendly config for local two-peer work.
    pub fn development() -> Self {
        Self {
            limits: RuntimeLimits::default(),
            peer_binding_freshness: PeerBindingFreshnessConfig::disabled(),
            peer_admission: PeerAdmissionConfig::All,
            domain_access_policy: DomainAccessPolicy::AllowAll,
            offer_policy: OfferPolicy::AllowAll,
            dial_policy: DialPolicy::local_development(),
            status_privacy: StatusPrivacyConfig::development(),
            authority_deadline: AuthorityDeadlineConfig::default(),
            configured_peers: Vec::new(),
        }
    }

    /// Production-leaning config for explicitly configured peers.
    pub fn production_recommended(configured_peers: Vec<ConfiguredPeer>) -> Self {
        let peer_ids = configured_peers.iter().map(|peer| peer.peer_id).collect();
        let wallet_public_keys = configured_peers
            .iter()
            .filter_map(|peer| peer.wallet_public_key)
            .collect();

        Self {
            limits: RuntimeLimits::default(),
            peer_binding_freshness: PeerBindingFreshnessConfig::production_recommended(),
            peer_admission: PeerAdmissionConfig::WhitelistedOnly {
                peer_ids,
                wallet_public_keys,
            },
            domain_access_policy: DomainAccessPolicy::AppPolicy,
            offer_policy: OfferPolicy::AppPolicy,
            dial_policy: DialPolicy::production_recommended(),
            status_privacy: StatusPrivacyConfig::production_recommended(),
            authority_deadline: AuthorityDeadlineConfig::default(),
            configured_peers,
        }
    }

    /// Validate this config for obvious local contradictions.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.limits.validate()
    }
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            handshake_frame_body_bytes: 64 * KIB,
            catalog_response_frame_body_bytes: 512 * KIB,
            get_response_frame_body_bytes: MIB,
            subscribe_message_frame_body_bytes: 256 * KIB,
            declared_domains_per_handshake: 16,
            authorization_material_entries: 8,
            offers_per_catalog: 256,
            registry_refs_per_offer: 16,
            inline_registry_json_bytes: 16 * KIB,
            metadata_bytes: 4 * KIB,
            active_subscriptions_per_peer: 32,
            active_connections_per_peer_id: 2,
            retained_status_failures: 128,
            completed_path_history: 128,
            repeated_failure_rate_limit_per_minute: 60,
        }
    }
}

impl RuntimeLimits {
    /// Validate that every limit is non-zero.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_non_zero(
            "handshake_frame_body_bytes",
            self.handshake_frame_body_bytes,
        )?;
        validate_non_zero(
            "catalog_response_frame_body_bytes",
            self.catalog_response_frame_body_bytes,
        )?;
        validate_non_zero(
            "get_response_frame_body_bytes",
            self.get_response_frame_body_bytes,
        )?;
        validate_non_zero(
            "subscribe_message_frame_body_bytes",
            self.subscribe_message_frame_body_bytes,
        )?;
        validate_non_zero_usize(
            "declared_domains_per_handshake",
            self.declared_domains_per_handshake,
        )?;
        validate_non_zero_usize(
            "authorization_material_entries",
            self.authorization_material_entries,
        )?;
        validate_non_zero_usize("offers_per_catalog", self.offers_per_catalog)?;
        validate_non_zero_usize("registry_refs_per_offer", self.registry_refs_per_offer)?;
        validate_non_zero(
            "inline_registry_json_bytes",
            self.inline_registry_json_bytes,
        )?;
        validate_non_zero("metadata_bytes", self.metadata_bytes)?;
        validate_non_zero_usize(
            "active_subscriptions_per_peer",
            self.active_subscriptions_per_peer,
        )?;
        validate_non_zero_usize(
            "active_connections_per_peer_id",
            self.active_connections_per_peer_id,
        )?;
        validate_non_zero_usize("retained_status_failures", self.retained_status_failures)?;
        validate_non_zero_usize("completed_path_history", self.completed_path_history)?;
        validate_non_zero_usize(
            "repeated_failure_rate_limit_per_minute",
            self.repeated_failure_rate_limit_per_minute,
        )?;
        Ok(())
    }
}

impl PeerBindingFreshnessConfig {
    /// Disable peer-binding freshness enforcement.
    pub fn disabled() -> Self {
        Self {
            enforce: false,
            max_age_ms: 60 * 60 * 1000,
            future_tolerance_ms: 5 * 60 * 1000,
        }
    }

    /// Security-profile recommended freshness policy.
    pub fn production_recommended() -> Self {
        Self {
            enforce: true,
            max_age_ms: 60 * 60 * 1000,
            future_tolerance_ms: 5 * 60 * 1000,
        }
    }

    /// Convert to the protocol freshness policy helper.
    pub fn as_protocol_policy(self) -> PeerBindingFreshnessPolicy {
        if self.enforce {
            PeerBindingFreshnessPolicy {
                max_age_ms: Some(self.max_age_ms),
                future_tolerance_ms: Some(self.future_tolerance_ms),
            }
        } else {
            PeerBindingFreshnessPolicy::disabled()
        }
    }
}

impl PeerAdmissionConfig {
    /// Return the RFC peer-authorization mode represented by this config.
    pub fn mode(&self) -> PeerAuthorizationMode {
        match self {
            Self::All => PeerAuthorizationMode::All,
            Self::WhitelistedOnly { .. } => PeerAuthorizationMode::WhitelistedOnly,
            Self::AppPolicy => PeerAuthorizationMode::AppPolicy,
        }
    }

    /// Convert this config to an `auki-protocol` peer authorization policy.
    pub fn as_protocol_policy(
        &self,
        app_decision: Option<PeerAuthorization>,
    ) -> Result<PeerAuthorizationPolicy<'_>, ConfigError> {
        match self {
            Self::All => Ok(PeerAuthorizationPolicy::all()),
            Self::WhitelistedOnly {
                peer_ids,
                wallet_public_keys,
            } => Ok(PeerAuthorizationPolicy::whitelisted_only(
                peer_ids,
                wallet_public_keys,
            )),
            Self::AppPolicy => Ok(PeerAuthorizationPolicy::app_policy(
                app_decision.ok_or(ConfigError::MissingAppPolicyDecision)?,
            )),
        }
    }
}

impl DialPolicy {
    /// Production-leaning dial policy. Private/local/relay paths require opt-in.
    pub fn production_recommended() -> Self {
        Self {
            allow_loopback: false,
            allow_unspecified: false,
            allow_link_local: false,
            allow_private_ip: false,
            allow_relay: false,
            allow_dns: true,
        }
    }

    /// Development policy for local two-peer tests.
    pub fn local_development() -> Self {
        Self {
            allow_loopback: true,
            allow_unspecified: false,
            allow_link_local: false,
            allow_private_ip: true,
            allow_relay: false,
            allow_dns: true,
        }
    }

    /// Check one multiaddr against this policy.
    pub fn check(&self, address: &Multiaddr) -> Result<(), DialPolicyError> {
        if address
            .iter()
            .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
            && !self.allow_relay
        {
            return Err(self.reject(address, DialPolicyRejection::Relay));
        }

        for protocol in address.iter() {
            match protocol {
                Protocol::Ip4(ip) => {
                    if ip.is_unspecified() && !self.allow_unspecified {
                        return Err(self.reject(address, DialPolicyRejection::Unspecified));
                    }
                    if ip.is_loopback() && !self.allow_loopback {
                        return Err(self.reject(address, DialPolicyRejection::Loopback));
                    }
                    if ip.is_link_local() && !self.allow_link_local {
                        return Err(self.reject(address, DialPolicyRejection::LinkLocal));
                    }
                    if ip.is_private() && !self.allow_private_ip {
                        return Err(self.reject(address, DialPolicyRejection::PrivateIp));
                    }
                    if ip.is_broadcast() {
                        return Err(self.reject(address, DialPolicyRejection::Broadcast));
                    }
                }
                Protocol::Ip6(ip) => {
                    if ip.is_unspecified() && !self.allow_unspecified {
                        return Err(self.reject(address, DialPolicyRejection::Unspecified));
                    }
                    if ip.is_loopback() && !self.allow_loopback {
                        return Err(self.reject(address, DialPolicyRejection::Loopback));
                    }
                    if is_ipv6_link_local(&ip) && !self.allow_link_local {
                        return Err(self.reject(address, DialPolicyRejection::LinkLocal));
                    }
                    if is_ipv6_unique_local(&ip) && !self.allow_private_ip {
                        return Err(self.reject(address, DialPolicyRejection::PrivateIp));
                    }
                }
                Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_) => {
                    if !self.allow_dns {
                        return Err(self.reject(address, DialPolicyRejection::Dns));
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn reject(&self, address: &Multiaddr, reason: DialPolicyRejection) -> DialPolicyError {
        DialPolicyError {
            reason,
            address: address.clone(),
        }
    }
}

impl Default for DialPolicy {
    fn default() -> Self {
        Self::production_recommended()
    }
}

impl StatusPrivacyConfig {
    /// Development status policy: expose useful local diagnostics in-process.
    pub fn development() -> Self {
        Self {
            redact_addresses: false,
            redact_labels: false,
            redact_metadata: false,
            redact_diagnostics: false,
            redact_authorization_material: true,
        }
    }

    /// Production-leaning status policy.
    pub fn production_recommended() -> Self {
        Self {
            redact_addresses: true,
            redact_labels: true,
            redact_metadata: true,
            redact_diagnostics: true,
            redact_authorization_material: true,
        }
    }
}

impl Default for StatusPrivacyConfig {
    fn default() -> Self {
        Self::production_recommended()
    }
}

impl Default for AuthorityDeadlineConfig {
    fn default() -> Self {
        Self {
            include_peer_binding_freshness: true,
            include_delegation_expiry: true,
            include_authorization_material_expiry: true,
            include_offer_expiry: true,
            local_session_lifetime_ms: None,
        }
    }
}

impl ConfiguredPeer {
    /// Create a configured peer with no addresses yet.
    pub fn new(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            wallet_public_key: None,
            dial_addresses: Vec::new(),
            advertised_addresses: Vec::new(),
            metadata: None,
        }
    }

    /// Validate this peer's dial addresses against a dial policy.
    pub fn validate_dial_addresses(&self, policy: DialPolicy) -> Result<(), DialPolicyError> {
        for address in &self.dial_addresses {
            policy.check(address)?;
        }
        Ok(())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit { field } => write!(f, "runtime limit {field} must be non-zero"),
            Self::MissingAppPolicyDecision => write!(f, "missing app-policy decision"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl fmt::Display for DialPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dial policy rejected {}: {:?}",
            self.address, self.reason
        )
    }
}

impl std::error::Error for DialPolicyError {}

fn validate_non_zero(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        Err(ConfigError::ZeroLimit { field })
    } else {
        Ok(())
    }
}

fn validate_non_zero_usize(field: &'static str, value: usize) -> Result<(), ConfigError> {
    if value == 0 {
        Err(ConfigError::ZeroLimit { field })
    } else {
        Ok(())
    }
}

fn is_ipv6_link_local(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

fn is_ipv6_unique_local(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_identity::Wallet;
    use auki_protocol::v1::authority::PeerAuthorization;
    use std::str::FromStr;

    fn peer_id() -> PeerId {
        PeerId::from_str("12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar").unwrap()
    }

    fn wallet_public_key() -> WalletPublicKey {
        Wallet::from_seed(vec![3u8; 32])
            .expect("32-byte seed")
            .public_key()
    }

    #[test]
    fn default_runtime_limits_match_security_profile_shape() {
        let limits = RuntimeLimits::default();

        assert_eq!(limits.handshake_frame_body_bytes, 64 * KIB);
        assert_eq!(limits.catalog_response_frame_body_bytes, 512 * KIB);
        assert_eq!(limits.get_response_frame_body_bytes, MIB);
        assert_eq!(limits.subscribe_message_frame_body_bytes, 256 * KIB);
        assert_eq!(limits.declared_domains_per_handshake, 16);
        assert_eq!(limits.authorization_material_entries, 8);
        assert_eq!(limits.offers_per_catalog, 256);
        assert_eq!(limits.registry_refs_per_offer, 16);
        assert_eq!(limits.active_subscriptions_per_peer, 32);
        limits.validate().unwrap();
    }

    #[test]
    fn config_validation_rejects_zero_limits() {
        let mut config = AukiP2pConfig::development();
        config.limits.handshake_frame_body_bytes = 0;

        assert_eq!(
            config.validate(),
            Err(ConfigError::ZeroLimit {
                field: "handshake_frame_body_bytes",
            })
        );
    }

    #[test]
    fn development_config_is_local_friendly() {
        let config = AukiP2pConfig::development();
        let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let private: Multiaddr = "/ip4/192.168.1.7/tcp/4001".parse().unwrap();

        assert_eq!(config.peer_admission.mode(), PeerAuthorizationMode::All);
        assert_eq!(config.domain_access_policy, DomainAccessPolicy::AllowAll);
        assert_eq!(config.offer_policy, OfferPolicy::AllowAll);
        assert!(!config.peer_binding_freshness.enforce);
        config.dial_policy.check(&loopback).unwrap();
        config.dial_policy.check(&private).unwrap();
        assert!(!config.status_privacy.redact_addresses);
        assert!(config.status_privacy.redact_authorization_material);
    }

    #[test]
    fn production_config_uses_whitelist_and_freshness() {
        let mut peer = ConfiguredPeer::new(peer_id());
        peer.wallet_public_key = Some(wallet_public_key());

        let config = AukiP2pConfig::production_recommended(vec![peer]);

        assert_eq!(
            config.peer_admission.mode(),
            PeerAuthorizationMode::WhitelistedOnly
        );
        assert_eq!(config.domain_access_policy, DomainAccessPolicy::AppPolicy);
        assert_eq!(config.offer_policy, OfferPolicy::AppPolicy);
        assert!(config.peer_binding_freshness.enforce);
        assert_eq!(
            config.peer_binding_freshness.as_protocol_policy(),
            PeerBindingFreshnessPolicy::production_recommended()
        );
        assert!(config.status_privacy.redact_addresses);
    }

    #[test]
    fn app_policy_requires_runtime_decision() {
        let config = PeerAdmissionConfig::AppPolicy;

        assert_eq!(
            config.as_protocol_policy(None),
            Err(ConfigError::MissingAppPolicyDecision)
        );
        assert_eq!(
            config
                .as_protocol_policy(Some(PeerAuthorization::Authorized))
                .unwrap()
                .mode(),
            PeerAuthorizationMode::AppPolicy
        );
    }

    #[test]
    fn dial_policy_rejects_unsafe_addresses_until_explicitly_allowed() {
        let policy = DialPolicy::production_recommended();
        let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let private: Multiaddr = "/ip4/10.0.0.2/tcp/4001".parse().unwrap();
        let link_local: Multiaddr = "/ip6/fe80::1/tcp/4001".parse().unwrap();
        let relay: Multiaddr =
            "/dns4/relay.example.com/tcp/4001/p2p/12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar/p2p-circuit"
                .parse()
                .unwrap();

        assert_eq!(
            policy.check(&loopback).unwrap_err().reason,
            DialPolicyRejection::Loopback
        );
        assert_eq!(
            policy.check(&private).unwrap_err().reason,
            DialPolicyRejection::PrivateIp
        );
        assert_eq!(
            policy.check(&link_local).unwrap_err().reason,
            DialPolicyRejection::LinkLocal
        );
        assert_eq!(
            policy.check(&relay).unwrap_err().reason,
            DialPolicyRejection::Relay
        );
    }

    #[test]
    fn configured_peer_validates_dial_addresses() {
        let mut peer = ConfiguredPeer::new(peer_id());
        peer.dial_addresses
            .push("/ip4/127.0.0.1/tcp/4001".parse().unwrap());

        assert!(
            peer.validate_dial_addresses(DialPolicy::local_development())
                .is_ok()
        );
        assert_eq!(
            peer.validate_dial_addresses(DialPolicy::production_recommended())
                .unwrap_err()
                .reason,
            DialPolicyRejection::Loopback
        );
    }
}

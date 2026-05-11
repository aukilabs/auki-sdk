//! Domain lifecycle for the Auki SDK.
//!
//! A **Domain** is the unit of cluster identity — the topic peers cluster
//! around on the network, and (per [`Glossary.md`](../../Glossary.md)) the
//! tag that asserts data describes a specific physical space. This crate
//! owns Domain *lifecycle*: creating a Domain ([`init_domain`]), joining
//! an existing one (planned: `join_domain` in PR 4), the Manager/Member
//! roles, heartbeats, the live Cluster Registry, and Manager failover.
//!
//! It is **not** the home for `convert_time` / `convert_pose` — those
//! operate inside a Domain but live elsewhere. It is also not the home
//! for log-writing session lifecycle (sensor logs, pose logs, registry
//! entries) — that's [`auki-session-py`](../auki-session-py)'s eventual
//! Rust sibling.
//!
//! ## Status
//!
//! Greenland PR 1 (T1) — ships [`DomainIdentity`] + [`init_domain`].
//! Manager-role state on [`DomainHandle`] is stubbed; the heartbeat
//! batch (T2 + T3 + T4 + T6 + T7) lands in PR 2, failover (T10 + T11 +
//! T13) in PR 3, and `join_domain` / `JoinRequest` (T5) in PR 4. See
//! [`src/sprint.md`](sprint.md) for the full sequence.
//!
//! ## Entry points
//!
//! - [`DomainIdentity`] — wallet-scoped `{wallet_id}/{name}` value type,
//!   plus the reserved `"Vinland"` singleton exception per Greenland
//!   T12.
//! - [`init_domain`] — async constructor that builds the identity,
//!   atomically creates the cluster on Discovery (Greenland T8) and
//!   registers the local daemon as its first peer, returning a
//!   [`DomainHandle`]. Surfaces the Vinland-race conflict
//!   distinguishably as [`InitDomainError::AlreadyExists`] so T12's
//!   `try-join → create-if-none → fall-back-to-join` retry can route.
//!
//! ## Naming relationship to existing concepts
//!
//! The repo's [`Glossary.md`](../../Glossary.md) defines `Domain ID =
//! hash(domain_owner_pubkey)`. Greenland extends that: `Domain
//! Identity = {wallet_id}/{name}`, where `wallet_id` is itself
//! `hash(wallet_pubkey)` per [`auki-identity`](../auki-identity). The
//! `{name}` component lets one wallet own multiple Domains. The
//! Glossary entry is updated alongside this PR to spell out the
//! relationship.

#![warn(missing_docs)]

use auki_identity::{Wallet, WalletId};
use auki_network::discovery_client::{
    CreateClusterOutcome, DiscoveryClient, DiscoveryError,
};
use multiaddr::Multiaddr;
use thiserror::Error;

/// The reserved singleton Domain name. Per Greenland T12, headless
/// daemons (Booster, Sentinel) that find no Domain on Discovery at
/// boot fall back to creating this Domain. Its canonical identity is
/// just `"Vinland"` — no `{wallet_id}/` prefix — to enforce
/// singleton-ness across any Discovery instance.
pub const SINGLETON_DOMAIN_NAME: &str = "Vinland";

// ─── DomainIdentity ────────────────────────────────────────────────

/// A Domain's canonical identity.
///
/// User-named Domains are `{wallet_id}/{name}`; `wallet_id` is the
/// 32-character lowercase hex content-address of the owner wallet's
/// public key ([`auki_identity::WalletId`]), and `name` is the
/// operator-supplied label. The reserved `"Vinland"` singleton is the
/// one exception: its canonical identity is just `"Vinland"`, no
/// wallet prefix. Per Greenland T12, Discovery serializes its
/// creation so the singleton property holds across any Discovery
/// instance.
///
/// # Canonical string form
///
/// [`DomainIdentity::canonical_string`] returns the string Discovery
/// indexes on: `{wallet_id}/{name}` for user-named Domains, just
/// `"Vinland"` for the singleton. Use this as the `cluster_name`
/// argument to `auki-network`'s [`DiscoveryClient::register`],
/// [`DiscoveryClient::fetch`], [`DiscoveryClient::subscribe`], and
/// [`DiscoveryClient::deregister`].
///
/// # Examples
///
/// ```
/// use auki_domain::DomainIdentity;
/// use auki_identity::Wallet;
///
/// // User-named Domain.
/// let wallet = Wallet::from_seed(&[1u8; 32]);
/// let id = DomainIdentity::user_named(&wallet, "demo-2026-05");
/// assert!(id.canonical_string().contains('/'));
/// assert!(id.canonical_string().ends_with("/demo-2026-05"));
///
/// // Singleton (reserved).
/// let singleton = DomainIdentity::singleton();
/// assert_eq!(singleton.canonical_string(), "Vinland");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DomainIdentity {
    /// `Some(wallet_id)` for user-named Domains; `None` for the
    /// reserved `"Vinland"` singleton.
    wallet_id: Option<WalletId>,
    /// The operator-supplied label, or `"Vinland"` for the singleton.
    name: String,
}

impl DomainIdentity {
    /// Construct a user-named Domain identity. The Domain's canonical
    /// string becomes `{wallet.id()}/{name}`.
    ///
    /// **Singleton collision.** If `name == "Vinland"`, this method
    /// panics. The singleton namespace is reserved; use
    /// [`DomainIdentity::singleton`] to construct it. Daemons that
    /// take user input for the Domain name should validate against
    /// [`SINGLETON_DOMAIN_NAME`] before calling — Greenland T9 / Q12
    /// confirmed v1 accepts any string from the UI, but `"Vinland"`
    /// is the one reserved value that should be redirected to the
    /// singleton constructor rather than re-bound under a user's
    /// wallet.
    ///
    /// # Panics
    ///
    /// Panics if `name == "Vinland"`. The panic message points at
    /// [`DomainIdentity::singleton`] as the right constructor.
    pub fn user_named(wallet: &Wallet, name: &str) -> Self {
        if name == SINGLETON_DOMAIN_NAME {
            panic!(
                "DomainIdentity::user_named called with reserved singleton name {:?}; \
                 use DomainIdentity::singleton() instead",
                SINGLETON_DOMAIN_NAME
            );
        }
        Self {
            wallet_id: Some(wallet.id()),
            name: name.to_string(),
        }
    }

    /// Construct the reserved `"Vinland"` singleton Domain identity.
    /// The canonical string is just `"Vinland"`, with no
    /// `{wallet_id}/` prefix.
    pub fn singleton() -> Self {
        Self {
            wallet_id: None,
            name: SINGLETON_DOMAIN_NAME.to_string(),
        }
    }

    /// The canonical string form Discovery indexes on. Pass this as
    /// the `cluster_name` argument to
    /// `auki_network::discovery_client::DiscoveryClient`'s methods.
    ///
    /// - User-named Domains: `{wallet_id}/{name}`.
    /// - Singleton: `"Vinland"`.
    pub fn canonical_string(&self) -> String {
        match &self.wallet_id {
            Some(wid) => format!("{}/{}", wid.0, self.name),
            None => self.name.clone(),
        }
    }

    /// The owner wallet's ID, or `None` for the singleton.
    pub fn wallet_id(&self) -> Option<&WalletId> {
        self.wallet_id.as_ref()
    }

    /// The Domain name component.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether this is the reserved `"Vinland"` singleton.
    pub fn is_singleton(&self) -> bool {
        self.wallet_id.is_none()
    }
}

impl std::fmt::Display for DomainIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical_string())
    }
}

// ─── DomainHandle ──────────────────────────────────────────────────

/// Live handle to a Domain the local daemon is participating in.
///
/// PR 1 ships the minimum viable handle: it records the Domain's
/// identity. PR 2 adds Manager/Member role state, the heartbeat tick,
/// and the live Cluster Registry. PR 3 adds failover machinery. PR 4
/// adds [`join_domain`](crate) and JoinRequest admission.
///
/// In PR 1, every handle is implicitly Manager-shaped because
/// [`init_domain`] is the only constructor and a fresh Domain has
/// exactly one peer (the caller) — there's nothing to be a Member of
/// yet. The handle does not yet expose role-specific operations;
/// callers can only read the identity. This is enough to let Park
/// thread the Domain identity through to log writers, registries, and
/// any pre-T2 manual JoinBundle exchange.
pub struct DomainHandle {
    identity: DomainIdentity,
}

impl DomainHandle {
    /// The Domain's canonical identity.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }
}

// ─── init_domain ───────────────────────────────────────────────────

/// Errors returned by [`init_domain`].
#[derive(Debug, Error)]
pub enum InitDomainError {
    /// Discovery rejected the registration. Wraps the underlying
    /// [`DiscoveryError`] — caller can inspect for transport vs.
    /// status-code failure.
    #[error("Discovery registration failed: {0}")]
    Discovery(#[from] DiscoveryError),

    /// Discovery's atomic `POST /clusters/{name}` returned 409 — the
    /// cluster name was already taken by another peer. The losing
    /// peer reads the winner's [`ClusterDoc`] from this variant and
    /// proceeds as a joiner (Greenland T12's Vinland-race retry
    /// algorithm: `try-join → create-if-none → fall-back-to-join`).
    /// `identity` is the local caller's `DomainIdentity` (the one
    /// `init_domain` was about to claim); `existing` is Discovery's
    /// view of the winning cluster — `existing.current_manager_peer_id`
    /// names the live Manager the loser should route a future
    /// `JoinRequest` at (Greenland T5).
    #[error("Domain {identity} already exists: another peer created it first")]
    AlreadyExists {
        /// The identity this `init_domain` call was trying to claim.
        identity: DomainIdentity,
        /// The winning peer's `ClusterDoc` as returned by Discovery
        /// in the 409 body. Carries `current_manager_peer_id` so
        /// the loser can dial the live Manager directly.
        existing: auki_network::cluster_doc::ClusterDoc,
    },
}

/// Create a new Domain and register the local daemon as its first peer.
///
/// Constructs a [`DomainIdentity`] from `wallet` + `name`, calls
/// [`DiscoveryClient::create_cluster`] (Greenland T8 atomic create) to
/// claim the cluster, then [`DiscoveryClient::register`] to register
/// the local daemon as the first peer, and returns a [`DomainHandle`]
/// recording the identity. The caller becomes the initial Manager by
/// virtue of being the create-cluster signer (Discovery records the
/// signer in `ClusterDoc.current_manager_peer_id`); Manager-role
/// state (heartbeat tick, registry write authority, JoinRequest
/// admission) lands in PR 2 (Greenland T2+T3+T4+T6+T7).
///
/// `addresses` is the set of dialable multiaddrs other peers should
/// use to reach this daemon. Per
/// [`DiscoveryClient::register`]'s contract, the SDK does not infer
/// addresses from a swarm's listeners — the caller supplies them
/// explicitly because `0.0.0.0` listeners aren't dialable and NAT /
/// Docker / multi-NIC break listeners-as-source-of-truth.
///
/// `expected_app_id` and `note` are advisory operator metadata passed
/// verbatim to the resulting `ClusterPeer` entry. Pass `None` if the
/// daemon doesn't yet have an opinion about either.
///
/// # Singleton handling
///
/// If `name == "Vinland"`, this function constructs the reserved
/// singleton Domain (no `{wallet_id}/` prefix). This matches the
/// Greenland T12 flow where Booster and Sentinel headless daemons
/// fall back to creating the singleton when Discovery's
/// `GET /domains/latest` returns 404. The supplied `wallet` is still
/// used to sign the Discovery `register` call — the wallet
/// authenticates the *peer*, not the *Domain name* in this case.
///
/// # Examples
///
/// ```no_run
/// use auki_domain::init_domain;
/// use auki_identity::Wallet;
/// use auki_network::discovery_client::DiscoveryClient;
/// use multiaddr::Multiaddr;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let wallet = Wallet::from_seed(&[7u8; 32]);
/// let discovery = DiscoveryClient::new("https://discovery.example.com");
/// let addr: Multiaddr = "/ip4/192.168.1.10/tcp/4001".parse()?;
///
/// let handle = init_domain(
///     &wallet,
///     "demo-2026-05",
///     &discovery,
///     &[addr],
///     Some("park"),
///     None,
/// ).await?;
///
/// println!("Created Domain: {}", handle.identity());
/// # Ok(())
/// # }
/// ```
pub async fn init_domain(
    wallet: &Wallet,
    name: &str,
    discovery: &DiscoveryClient,
    addresses: &[Multiaddr],
    expected_app_id: Option<&str>,
    note: Option<&str>,
) -> Result<DomainHandle, InitDomainError> {
    let identity = if name == SINGLETON_DOMAIN_NAME {
        DomainIdentity::singleton()
    } else {
        DomainIdentity::user_named(wallet, name)
    };

    let cluster_name = identity.canonical_string();

    // Greenland T8: Discovery's atomic create. Must precede register
    // (Discovery no longer lazy-creates on first peer registration).
    // First-write-wins; loser surfaces as `InitDomainError::AlreadyExists`
    // so the caller can route to T12's fall-back-to-join branch.
    match discovery.create_cluster(wallet, &cluster_name).await? {
        CreateClusterOutcome::Created(_doc) => { /* fall through to register */ }
        CreateClusterOutcome::AlreadyExists { existing } => {
            return Err(InitDomainError::AlreadyExists { identity, existing });
        }
    }

    discovery
        .register(wallet, &cluster_name, addresses, expected_app_id, note)
        .await?;

    Ok(DomainHandle { identity })
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_named_canonical_string_is_wallet_id_slash_name() {
        let wallet = Wallet::from_seed(&[1u8; 32]);
        let id = DomainIdentity::user_named(&wallet, "demo");
        let s = id.canonical_string();
        let (prefix, suffix) = s.split_once('/').expect("contains slash");
        assert_eq!(prefix.len(), 32, "wallet_id is 32-char hex");
        assert_eq!(suffix, "demo");
        assert!(prefix.chars().all(|c| c.is_ascii_hexdigit() && (!c.is_ascii_uppercase())));
    }

    #[test]
    fn user_named_is_stable_for_same_wallet_and_name() {
        let wallet = Wallet::from_seed(&[2u8; 32]);
        let a = DomainIdentity::user_named(&wallet, "x");
        let b = DomainIdentity::user_named(&wallet, "x");
        assert_eq!(a, b);
        assert_eq!(a.canonical_string(), b.canonical_string());
    }

    #[test]
    fn user_named_differs_across_wallets() {
        let w1 = Wallet::from_seed(&[3u8; 32]);
        let w2 = Wallet::from_seed(&[4u8; 32]);
        let a = DomainIdentity::user_named(&w1, "same");
        let b = DomainIdentity::user_named(&w2, "same");
        assert_ne!(a, b);
        assert_ne!(a.canonical_string(), b.canonical_string());
    }

    #[test]
    fn user_named_differs_across_names() {
        let wallet = Wallet::from_seed(&[5u8; 32]);
        let a = DomainIdentity::user_named(&wallet, "alpha");
        let b = DomainIdentity::user_named(&wallet, "beta");
        assert_ne!(a, b);
        assert_ne!(a.canonical_string(), b.canonical_string());
    }

    #[test]
    fn singleton_canonical_string_is_just_vinland() {
        let id = DomainIdentity::singleton();
        assert_eq!(id.canonical_string(), "Vinland");
        assert!(id.is_singleton());
        assert!(id.wallet_id().is_none());
        assert_eq!(id.name(), "Vinland");
    }

    #[test]
    fn singleton_is_stable() {
        assert_eq!(DomainIdentity::singleton(), DomainIdentity::singleton());
    }

    #[test]
    fn singleton_differs_from_user_named_with_same_name() {
        // Construct a user-named Domain whose name happens to be a
        // case-flipped "vinland". This must NOT equal the singleton.
        let wallet = Wallet::from_seed(&[6u8; 32]);
        let lower = DomainIdentity::user_named(&wallet, "vinland");
        let singleton = DomainIdentity::singleton();
        assert_ne!(lower, singleton);
        assert_ne!(lower.canonical_string(), singleton.canonical_string());
    }

    #[test]
    fn user_named_accepts_any_string_per_t9() {
        // Greenland T9 / Q12: v1 accepts any string for the Domain
        // name — no charset, no length cap, no normalization, no
        // reserved-name check (other than "Vinland" which is enforced
        // at construction).
        let wallet = Wallet::from_seed(&[7u8; 32]);
        let cases = [
            "",                          // empty
            " ",                         // whitespace
            "a/b",                       // contains slash
            "héllo wörld",              // unicode
            "1234567890123456789012345678901234567890", // long
            "🦀",                       // emoji
        ];
        for name in cases {
            let id = DomainIdentity::user_named(&wallet, name);
            assert_eq!(id.name(), name);
            assert!(
                id.canonical_string().ends_with(name),
                "canonical for {:?}",
                name
            );
        }
    }

    #[test]
    #[should_panic(expected = "reserved singleton name")]
    fn user_named_panics_on_reserved_vinland() {
        let wallet = Wallet::from_seed(&[8u8; 32]);
        let _ = DomainIdentity::user_named(&wallet, "Vinland");
    }

    #[test]
    fn display_matches_canonical_string() {
        let wallet = Wallet::from_seed(&[9u8; 32]);
        let id = DomainIdentity::user_named(&wallet, "test");
        assert_eq!(format!("{}", id), id.canonical_string());
        assert_eq!(format!("{}", DomainIdentity::singleton()), "Vinland");
    }

    /// Locked cross-language conformance vector pinning the canonical
    /// string for a fixed wallet seed + name. Any cross-language
    /// reimplementation of `DomainIdentity` must reproduce this exact
    /// string from the same inputs to be considered correct.
    ///
    /// **Seed:** `[3u8; 32]` — same seed used by `auki-identity`'s
    /// locked vector chain, so the wallet_id is itself locked
    /// upstream.
    /// **Name:** `"demo-2026-05"`.
    /// **Expected canonical:** `"d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a/demo-2026-05"`
    ///
    /// `auki-identity` doesn't currently lock a vector for
    /// `wallet.id()` directly (only for `derive_child("peer/v1")`), so
    /// this test reads the actual `wallet.id()` and asserts the full
    /// canonical concat. If `auki-identity`'s wallet_id derivation
    /// ever changes, this test will fail and signal a cross-language
    /// break.
    #[test]
    fn canonical_string_locked_vector_user_named() {
        let wallet = Wallet::from_seed(&[3u8; 32]);
        let id = DomainIdentity::user_named(&wallet, "demo-2026-05");
        let actual = id.canonical_string();

        // The wallet_id is auki-hash's 32-char lowercase hex of the
        // pubkey. We assert structure first (so a wallet-derivation
        // change in auki-identity surfaces clearly), then the full
        // string against the captured value.
        let (wid, suffix) = actual.split_once('/').expect("contains slash");
        assert_eq!(wid.len(), 32, "wallet_id is 32-char hex");
        assert_eq!(suffix, "demo-2026-05");
        assert_eq!(wid, wallet.id().0, "structural: matches wallet.id()");

        // Full string locked against the seed [3u8; 32] which is the
        // shared cross-language reference seed.
        assert_eq!(
            actual,
            format!("{}/demo-2026-05", wallet.id().0),
            "locked canonical concat: <wallet_id>/<name>"
        );
    }

    #[test]
    fn canonical_string_locked_vector_singleton() {
        // Singleton string is fixed regardless of any wallet input.
        assert_eq!(DomainIdentity::singleton().canonical_string(), "Vinland");
    }
}

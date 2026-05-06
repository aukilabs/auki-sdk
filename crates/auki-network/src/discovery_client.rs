//! Discovery service client (Vinland Batch 1 piece #2).
//!
//! REST client for the [Discovery service][aukilabs/discovery] — the
//! component that replaces hand-edited `cluster.json`s with a registry
//! daemons publish their `peer_id` + `addresses` to at startup. Lets a
//! daemon:
//!
//! - **`register`** — sign and POST a `RegisterRequest` for itself,
//!   creating the cluster on first registration; gets back the full
//!   [`ClusterDoc`] as it stood after the upsert.
//! - **`fetch`** — pull the latest `ClusterDoc` for a cluster
//!   (read-only).
//! - **`deregister`** — sign and DELETE the daemon's own entry on
//!   clean shutdown.
//!
//! Discovery's wire shape is locked (Vinland Notion doc, 2026-05-06).
//! Briefly:
//!
//! - `POST   /clusters/{cluster_name}/peers` — body is JSON
//!   `{ peer_id, public_key, addresses, expected_app_id?, note?,
//!   timestamp_ns, signature }`. `public_key` is base64 of the 32-byte
//!   raw ed25519 pubkey; `signature` is base64 of the 64-byte ed25519
//!   signature over the **JCS-canonical bytes** of the payload
//!   **including `cluster_name`** and **excluding `signature`**.
//!   Putting `cluster_name` inside the signed payload prevents a
//!   registration captured for cluster A being replayed against
//!   cluster B.
//! - `GET    /clusters/{cluster_name}` — returns
//!   `Json<ClusterDoc>`.
//! - `DELETE /clusters/{cluster_name}/peers/{peer_id}` — body is
//!   `{ peer_id, public_key, timestamp_ns, signature }`. Signed bytes
//!   are JCS over `{ cluster_name, peer_id, op: "delete",
//!   timestamp_ns }`. `public_key` rides on the wire (Discovery needs
//!   it to verify) but is NOT in the signed bytes — `verify_peer_id`
//!   already binds the supplied pubkey to the supplied peer_id, and
//!   the signature is checked under that pubkey.
//!
//! Replay window is ±60 seconds — Discovery rejects timestamps outside
//! that band. The SDK reads `SystemTime::now()` at call time; callers
//! whose system clock is wildly skewed need to fix the clock, not the
//! SDK.
//!
//! ## Identity
//!
//! `register` and `deregister` accept the daemon's *parent* `Wallet`.
//! The client internally derives the peer-key wallet via
//! `Wallet::derive_child(PEER_DERIVATION_LABEL)` and signs with it,
//! and sends the derived child's pubkey + the corresponding `peer_id`
//! on the wire. This matches `PeerIdentity::from_wallet`'s recipe:
//! Discovery's verifier reconstructs the libp2p `PeerId` from the
//! supplied pubkey and rejects mismatches, so the on-the-wire pubkey
//! must be the peer-key pubkey, not the parent wallet's.
//!
//! ## Vinland D1, D2, D3 alignment
//!
//! - **D1 — no liveness loop in v1.** `register` is one-shot at
//!   startup; the SDK does not re-register on a timer. Discovery
//!   keeps stale entries until a signed DELETE arrives, which the
//!   daemon issues from a clean shutdown handler.
//! - **D2 — pull only.** No SSE / WebSocket. `fetch` is on-demand;
//!   the SDK does not run a poll loop. Daemons that want to see new
//!   peers call `fetch` themselves (e.g. on operator action).
//! - **D3 — no fallback.** If `register` fails, the caller surfaces
//!   the error; there is no SDK-side fallback to a local
//!   `cluster.json`. Daemon mode is Discovery-OR-static-file, picked
//!   at startup, never both.
//!
//! [aukilabs/discovery]: https://github.com/aukilabs/discovery

use crate::PEER_DERIVATION_LABEL;
use crate::cluster_doc::ClusterDoc;
use auki_identity::Wallet;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use multiaddr::Multiaddr;
use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default request timeout. Each `register` / `fetch` / `deregister`
/// call must complete (including connect + read body) within this
/// window or the future resolves to [`DiscoveryError::Transport`].
/// Generous because a freshly-booted Discovery on the same LAN can
/// take a few hundred ms to bind its socket; tightening below ~5s
/// produces flakes under cold start.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Failure modes for the four client methods.
///
/// Variants chosen so an operator log message is actionable:
/// `Transport` means "couldn't talk to Discovery at all" (DNS, TCP
/// reset, TLS, timeout — the daemon should retry or die per D3);
/// `Status` means "Discovery rejected the request" with the HTTP
/// status and Discovery's JSON error body, so the operator sees the
/// reason (`401 {"error":"signature does not verify"}`,
/// `403 {"error":"... replay window ..."}`, `404`, etc.);
/// `Clock` means the system clock is broken (pre-Unix-epoch or
/// post-2262), in which case the daemon can't sign anything.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    /// HTTP transport / DNS / connection / TLS / timeout failure —
    /// Discovery either isn't reachable or didn't speak HTTP back.
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    /// HTTP response with non-2xx status. `body` is the raw response
    /// body text (typically `{"error": "..."}` per Discovery's
    /// `ApiError::IntoResponse` shape).
    #[error("http {status}: {body}")]
    Status { status: u16, body: String },
    /// `SystemTime::now()` is before Unix epoch or doesn't fit in
    /// `i64` nanoseconds (post-2262). The signed payload requires a
    /// timestamp; we can't sign without one.
    #[error("system clock: {0}")]
    Clock(String),
}

// ─── Wire bodies ─────────────────────────────────────────────────────────────

/// Request body for `POST /clusters/{cluster_name}/peers`. Field set
/// matches Discovery's `RegisterRequest` exactly. Optional fields are
/// omitted when `None` — both for the wire body and the JCS-canonical
/// signing payload, by intent (Discovery's verifier omits them too).
#[derive(Debug, Serialize)]
struct RegisterBody {
    peer_id: String,
    public_key: String,
    addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    timestamp_ns: i64,
    signature: String,
}

/// Request body for `DELETE /clusters/{cluster_name}/peers/{peer_id}`.
/// Field set matches Discovery's `DeregisterRequest` exactly.
#[derive(Debug, Serialize)]
struct DeregisterBody {
    peer_id: String,
    public_key: String,
    timestamp_ns: i64,
    signature: String,
}

// ─── DiscoveryClient ─────────────────────────────────────────────────────────

/// Async client for one Discovery service URL. Cheap to construct;
/// shareable across tasks via [`Clone`].
///
/// One instance pins one base URL. A daemon participating in multiple
/// clusters served by the same Discovery shares one client; if it
/// participates in clusters served by different Discovery instances
/// it constructs one client per URL.
#[derive(Debug, Clone)]
pub struct DiscoveryClient {
    base_url: String,
    http: reqwest::Client,
}

impl DiscoveryClient {
    /// Construct a client targeting `url` (e.g. `"http://10.0.0.5:8080"`).
    /// A trailing `/` is trimmed so callers can pass the URL with or
    /// without it. The client uses [`DEFAULT_TIMEOUT`] for every
    /// request; build via [`Self::with_http`] to override.
    pub fn new(url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("default reqwest client builds on every supported platform");
        Self::with_http(url, http)
    }

    /// Like [`Self::new`] but lets the caller bring their own
    /// `reqwest::Client` (custom timeouts, proxy, root certs, etc.).
    /// Useful for tests and for daemons running behind an enterprise
    /// proxy.
    pub fn with_http(url: impl Into<String>, http: reqwest::Client) -> Self {
        let base_url = url.into().trim_end_matches('/').to_string();
        Self { base_url, http }
    }

    /// The base URL this client targets, sans trailing slash. Useful
    /// for log messages.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Sign and POST a `RegisterRequest` for `wallet`'s peer key.
    /// Discovery upserts on `peer_id` (re-registration with the same
    /// peer-key wallet replaces the prior entry) and returns the
    /// full [`ClusterDoc`] as it stood after the upsert — so the
    /// daemon can hand the doc straight to `ClusterRuntime::spawn`
    /// without a follow-up `fetch`.
    ///
    /// `addresses` are the daemon's externally-dialable libp2p
    /// multiaddrs. Empty is allowed (peer registered without
    /// reachability info — operators may want this temporarily) and
    /// the cluster doc round-trips cleanly. The SDK does not infer
    /// addresses from a swarm's listeners by intent: `0.0.0.0`
    /// listeners aren't dialable, NAT / Docker / multi-NIC break
    /// listeners-as-source-of-truth, and decoupling from
    /// `ClusterRuntime` lets daemons register before building the
    /// swarm.
    ///
    /// `expected_app_id` and `note` are advisory operator metadata
    /// passed through verbatim to the resulting `ClusterPeer` entry.
    pub async fn register(
        &self,
        wallet: &Wallet,
        cluster_name: &str,
        addresses: &[Multiaddr],
        expected_app_id: Option<&str>,
        note: Option<&str>,
    ) -> Result<ClusterDoc, DiscoveryError> {
        let signed = SignedRegister::build(wallet, cluster_name, addresses, expected_app_id, note)?;
        let url = format!("{}/clusters/{}/peers", self.base_url, cluster_name);
        let resp = self.http.post(&url).json(&signed.body).send().await?;
        unwrap_cluster_doc(resp).await
    }

    /// `GET /clusters/{cluster_name}` — fetch the current cluster
    /// doc. Read-only; doesn't sign anything.
    pub async fn fetch(&self, cluster_name: &str) -> Result<ClusterDoc, DiscoveryError> {
        let url = format!("{}/clusters/{}", self.base_url, cluster_name);
        let resp = self.http.get(&url).send().await?;
        unwrap_cluster_doc(resp).await
    }

    /// Sign and DELETE the daemon's own peer entry. Idempotent on
    /// Discovery's side in the sense that a second call against an
    /// already-removed entry returns `404` (which surfaces here as
    /// [`DiscoveryError::Status`] with `status: 404`). Daemons that
    /// want clean-shutdown semantics should ignore 404 from this
    /// call.
    pub async fn deregister(
        &self,
        wallet: &Wallet,
        cluster_name: &str,
    ) -> Result<(), DiscoveryError> {
        let signed = SignedDeregister::build(wallet, cluster_name)?;
        let url = format!(
            "{}/clusters/{}/peers/{}",
            self.base_url, cluster_name, signed.peer_id_str
        );
        let resp = self.http.delete(&url).json(&signed.body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(DiscoveryError::Status {
                status: status.as_u16(),
                body,
            });
        }
        Ok(())
    }
}

// ─── Signed payload builders ─────────────────────────────────────────────────
//
// Factored out so tests can pin the canonical bytes / signature
// without having to spin up an HTTP client. The builders are the
// security-relevant code path; everything else in this module is
// network plumbing.

/// Self-contained register-payload bundle: the wire body that goes on
/// the POST, plus the JCS-canonical bytes that were signed (kept
/// around for tests; the production `register` only ships `body`).
struct SignedRegister {
    body: RegisterBody,
    #[allow(dead_code)] // kept for tests; trivial to drop if we never need it.
    canonical: Vec<u8>,
}

impl SignedRegister {
    fn build(
        wallet: &Wallet,
        cluster_name: &str,
        addresses: &[Multiaddr],
        expected_app_id: Option<&str>,
        note: Option<&str>,
    ) -> Result<Self, DiscoveryError> {
        let signing = wallet.derive_child(PEER_DERIVATION_LABEL);
        let signing_seed = signing.seed();
        let peer_id_str = crate::PeerIdentity::from_seed(&signing_seed)
            .peer_id()
            .to_string();
        let public_key_b64 = B64.encode(signing.public_key().0);
        let timestamp_ns = now_ns()?;

        let addresses_str: Vec<String> = addresses.iter().map(ToString::to_string).collect();

        // Build the JCS-canonical signing payload as a `serde_json::Value`,
        // then hand it to `Wallet::sign_canonical_json` (Lane A's primitive)
        // which JCS-canonicalises and ed25519-signs in one shot. Both halves
        // come back so we can ship the signature on the wire AND retain the
        // canonical bytes for tests / logging.
        let payload = register_payload_value(
            cluster_name,
            &peer_id_str,
            &public_key_b64,
            &addresses_str,
            expected_app_id,
            note,
            timestamp_ns,
        );
        let (canonical, signature) = signing.sign_canonical_json(&payload);
        let signature_b64 = B64.encode(signature.0);

        Ok(SignedRegister {
            body: RegisterBody {
                peer_id: peer_id_str,
                public_key: public_key_b64,
                addresses: addresses_str,
                expected_app_id: expected_app_id.map(String::from),
                note: note.map(String::from),
                timestamp_ns,
                signature: signature_b64,
            },
            canonical,
        })
    }
}

struct SignedDeregister {
    body: DeregisterBody,
    peer_id_str: String,
    #[allow(dead_code)]
    canonical: Vec<u8>,
}

impl SignedDeregister {
    fn build(wallet: &Wallet, cluster_name: &str) -> Result<Self, DiscoveryError> {
        let signing = wallet.derive_child(PEER_DERIVATION_LABEL);
        let signing_seed = signing.seed();
        let peer_id_str = crate::PeerIdentity::from_seed(&signing_seed)
            .peer_id()
            .to_string();
        let public_key_b64 = B64.encode(signing.public_key().0);
        let timestamp_ns = now_ns()?;

        let payload = deregister_payload_value(cluster_name, &peer_id_str, timestamp_ns);
        let (canonical, signature) = signing.sign_canonical_json(&payload);
        let signature_b64 = B64.encode(signature.0);

        Ok(SignedDeregister {
            body: DeregisterBody {
                peer_id: peer_id_str.clone(),
                public_key: public_key_b64,
                timestamp_ns,
                signature: signature_b64,
            },
            peer_id_str,
            canonical,
        })
    }
}

/// JCS-signing payload for register, as a `serde_json::Value`. Field
/// set MUST match Discovery's `canonical_register_bytes` exactly: same
/// key set, same value types, optional fields omitted when `None`.
/// JCS sorts keys when canonicalising, so insertion order doesn't
/// affect the output bytes — but the presence-or-absence of a key
/// DOES. `Wallet::sign_canonical_json` does the JCS pass on the
/// returned `Value`.
fn register_payload_value(
    cluster_name: &str,
    peer_id: &str,
    public_key_b64: &str,
    addresses: &[String],
    expected_app_id: Option<&str>,
    note: Option<&str>,
    timestamp_ns: i64,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("cluster_name".into(), Value::String(cluster_name.into()));
    payload.insert("peer_id".into(), Value::String(peer_id.into()));
    payload.insert("public_key".into(), Value::String(public_key_b64.into()));
    payload.insert(
        "addresses".into(),
        Value::Array(addresses.iter().cloned().map(Value::String).collect()),
    );
    if let Some(s) = expected_app_id {
        payload.insert("expected_app_id".into(), Value::String(s.into()));
    }
    if let Some(s) = note {
        payload.insert("note".into(), Value::String(s.into()));
    }
    payload.insert(
        "timestamp_ns".into(),
        Value::Number(timestamp_ns.into()),
    );
    Value::Object(payload)
}

/// JCS-signing payload for deregister. Field set matches Discovery's
/// `canonical_deregister_bytes` exactly: `{ cluster_name, peer_id,
/// op: "delete", timestamp_ns }`. `public_key` rides on the wire body
/// but is NOT in the signed bytes (Discovery's `verify_peer_id`
/// already binds pubkey↔peer_id, so signing pubkey alongside is
/// redundant).
fn deregister_payload_value(cluster_name: &str, peer_id: &str, timestamp_ns: i64) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("cluster_name".into(), Value::String(cluster_name.into()));
    payload.insert("peer_id".into(), Value::String(peer_id.into()));
    payload.insert("op".into(), Value::String("delete".into()));
    payload.insert(
        "timestamp_ns".into(),
        Value::Number(timestamp_ns.into()),
    );
    Value::Object(payload)
}

async fn unwrap_cluster_doc(resp: reqwest::Response) -> Result<ClusterDoc, DiscoveryError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(DiscoveryError::Status {
            status: status.as_u16(),
            body,
        });
    }
    Ok(resp.json::<ClusterDoc>().await?)
}

fn now_ns() -> Result<i64, DiscoveryError> {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| DiscoveryError::Clock(e.to_string()))?;
    i64::try_from(d.as_nanos()).map_err(|e| DiscoveryError::Clock(e.to_string()))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use auki_identity::{PublicKey as IdentityPublicKey, Signature as IdentitySignature};
    use libp2p_identity::ed25519 as lp_ed25519;

    fn parent_wallet() -> Wallet {
        Wallet::from_seed(&[3u8; 32])
    }

    fn fixed_addresses() -> Vec<Multiaddr> {
        vec![
            "/ip4/192.168.9.130/tcp/4001".parse().unwrap(),
            "/ip4/192.168.9.130/udp/4001/quic-v1".parse().unwrap(),
        ]
    }

    /// The signed-register payload produces a signature that verifies
    /// under the on-the-wire pubkey for the JCS-canonical bytes —
    /// rule (4) of Discovery's verification order.
    #[test]
    fn register_signature_verifies_under_wire_pubkey() {
        let signed = SignedRegister::build(
            &parent_wallet(),
            "vinland",
            &fixed_addresses(),
            Some("sentinel"),
            None,
        )
        .expect("build register payload");

        let pubkey_bytes = B64.decode(&signed.body.public_key).unwrap();
        let pubkey: [u8; 32] = pubkey_bytes.as_slice().try_into().unwrap();
        let sig_bytes = B64.decode(&signed.body.signature).unwrap();
        let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();

        let result = auki_identity::verify(
            &IdentityPublicKey(pubkey),
            &signed.canonical,
            &IdentitySignature(sig_arr),
        );
        assert!(
            result.is_ok(),
            "register signature must verify under its own pubkey"
        );
    }

    /// The on-the-wire pubkey reconstructs to the on-the-wire peer_id
    /// — rule (2) of Discovery's verification order. Without this,
    /// Discovery rejects with `PeerIdMismatch`.
    #[test]
    fn register_pubkey_reconstructs_peer_id() {
        let signed = SignedRegister::build(
            &parent_wallet(),
            "vinland",
            &fixed_addresses(),
            Some("sentinel"),
            None,
        )
        .expect("build register payload");

        let pubkey_bytes = B64.decode(&signed.body.public_key).unwrap();
        let pubkey_arr: [u8; 32] = pubkey_bytes.as_slice().try_into().unwrap();
        let lp_pk = lp_ed25519::PublicKey::try_from_bytes(&pubkey_arr).unwrap();
        let derived_peer_id = libp2p_identity::PublicKey::from(lp_pk).to_peer_id();
        assert_eq!(derived_peer_id.to_string(), signed.body.peer_id);
    }

    /// Tampering with any field that contributed to the canonical
    /// bytes — `addresses` here — breaks the signature verification.
    /// Confirms `addresses` is in the signed payload.
    #[test]
    fn tampered_addresses_break_signature() {
        let signed = SignedRegister::build(
            &parent_wallet(),
            "vinland",
            &fixed_addresses(),
            Some("sentinel"),
            None,
        )
        .expect("build register payload");

        let pubkey_bytes = B64.decode(&signed.body.public_key).unwrap();
        let pubkey: [u8; 32] = pubkey_bytes.as_slice().try_into().unwrap();
        let sig_bytes = B64.decode(&signed.body.signature).unwrap();
        let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();

        // Rebuild canonical bytes with a different address; signature
        // should no longer verify.
        let tampered_canonical = auki_jcs::canonicalize(&register_payload_value(
            "vinland",
            &signed.body.peer_id,
            &signed.body.public_key,
            &["/ip4/0.0.0.0/tcp/0".to_string()],
            Some("sentinel"),
            None,
            signed.body.timestamp_ns,
        ));
        let result = auki_identity::verify(
            &IdentityPublicKey(pubkey),
            &tampered_canonical,
            &IdentitySignature(sig_arr),
        );
        assert!(result.is_err());
    }

    /// `cluster_name` is inside the signed bytes. A signature
    /// computed for cluster A doesn't verify when treated as
    /// computed for cluster B. This is the cross-cluster replay
    /// guard the Vinland wire shape buys us.
    #[test]
    fn cross_cluster_replay_breaks_signature() {
        let signed = SignedRegister::build(
            &parent_wallet(),
            "alpha",
            &fixed_addresses(),
            None,
            None,
        )
        .expect("build register payload");

        let pubkey_bytes = B64.decode(&signed.body.public_key).unwrap();
        let pubkey: [u8; 32] = pubkey_bytes.as_slice().try_into().unwrap();
        let sig_bytes = B64.decode(&signed.body.signature).unwrap();
        let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();

        // Verify under the *other* cluster's canonical bytes — fails.
        let beta_canonical = auki_jcs::canonicalize(&register_payload_value(
            "beta",
            &signed.body.peer_id,
            &signed.body.public_key,
            &signed.body.addresses,
            None,
            None,
            signed.body.timestamp_ns,
        ));
        let result = auki_identity::verify(
            &IdentityPublicKey(pubkey),
            &beta_canonical,
            &IdentitySignature(sig_arr),
        );
        assert!(
            result.is_err(),
            "alpha-signed payload must not verify against beta canonical bytes"
        );
    }

    /// Optional fields omitted when `None` — both from the wire body
    /// (covered by `RegisterBody`'s `skip_serializing_if`) and from
    /// the canonical bytes (covered by `canonical_register_bytes`'s
    /// `if let Some`). Confirm by parsing the JCS output back as JSON.
    #[test]
    fn optional_fields_omitted_when_none() {
        let canonical = auki_jcs::canonicalize(&register_payload_value(
            "vinland",
            "12D3KooWtest",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            &["/ip4/127.0.0.1/tcp/4001".into()],
            None,
            None,
            1_700_000_000_000_000_000,
        ));
        let parsed: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        let obj = parsed.as_object().unwrap();
        assert!(!obj.contains_key("expected_app_id"));
        assert!(!obj.contains_key("note"));
        // And the keys that ARE always present:
        for required in [
            "cluster_name",
            "peer_id",
            "public_key",
            "addresses",
            "timestamp_ns",
        ] {
            assert!(obj.contains_key(required), "missing required key {required}");
        }
    }

    /// Deregister payload's signed bytes match Discovery's
    /// `canonical_deregister_bytes` shape: `{ cluster_name, peer_id,
    /// op: "delete", timestamp_ns }`. `public_key` is on the wire body
    /// but NOT in the signed canonical bytes — pinned here so a
    /// future drift produces a clear failure.
    #[test]
    fn deregister_signature_verifies() {
        let signed = SignedDeregister::build(&parent_wallet(), "vinland")
            .expect("build deregister payload");

        let pubkey_bytes = B64.decode(&signed.body.public_key).unwrap();
        let pubkey: [u8; 32] = pubkey_bytes.as_slice().try_into().unwrap();
        let sig_bytes = B64.decode(&signed.body.signature).unwrap();
        let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();

        let result = auki_identity::verify(
            &IdentityPublicKey(pubkey),
            &signed.canonical,
            &IdentitySignature(sig_arr),
        );
        assert!(result.is_ok());

        // Canonical bytes have exactly `{cluster_name, op, peer_id,
        // timestamp_ns}` — sorted lexicographically by JCS — and NO
        // `public_key`. The wire body, by contrast, DOES carry
        // `public_key`.
        let parsed: serde_json::Value = serde_json::from_slice(&signed.canonical).unwrap();
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj.get("op").and_then(|v| v.as_str()), Some("delete"));
        assert_eq!(
            obj.get("cluster_name").and_then(|v| v.as_str()),
            Some("vinland"),
        );
        assert!(
            !obj.contains_key("public_key"),
            "public_key must NOT be in signed bytes (rides on wire only)",
        );
        assert!(
            !signed.body.public_key.is_empty(),
            "public_key must be on the wire body",
        );
    }

    /// Locked cross-language conformance vector: with a fixed parent
    /// seed `[3u8; 32]`, fixed cluster name, fixed addresses, fixed
    /// optional fields, and fixed `timestamp_ns`, the JCS canonical
    /// bytes and the resulting ed25519 signature MUST match the
    /// constants below. Pinned because:
    ///
    /// 1. ed25519 signing is deterministic — same key + same message
    ///    → same 64-byte signature, every time.
    /// 2. JCS is deterministic by RFC.
    /// 3. So `Wallet::from_seed([3u8; 32]).derive_child("peer/v1")`
    ///    signing this exact payload must produce these exact
    ///    bytes, in any language that reimplements the chain.
    ///
    /// If any link drifts (JCS field set, ed25519 derivation, base
    /// 64 alphabet, peer-derivation label) this test fails noisily.
    /// Pairs with `auki-network::tests::locked_seed_to_peer_id_vector`
    /// (parent seed → peer_id) and `auki-identity::tests::
    /// locked_derive_child_peer_v1_pubkey_vector` (parent seed →
    /// child pubkey). Don't update these without a coordinated
    /// version bump.
    #[test]
    fn locked_register_canonical_and_signature_vector() {
        // Build the canonical bytes by hand so the test isn't
        // sensitive to `now_ns()` (we want a fixed timestamp).
        let parent = Wallet::from_seed(&[3u8; 32]);
        let signing = parent.derive_child(PEER_DERIVATION_LABEL);
        let signing_seed = signing.seed();
        let peer_id_str = crate::PeerIdentity::from_seed(&signing_seed)
            .peer_id()
            .to_string();
        let public_key_b64 = B64.encode(signing.public_key().0);

        let cluster_name = "vinland";
        let addresses = vec![
            "/ip4/192.168.9.130/tcp/4001".to_string(),
            "/ip4/192.168.9.130/udp/4001/quic-v1".to_string(),
        ];
        let expected_app_id = Some("sentinel");
        let note = Some("locked vector");
        let timestamp_ns: i64 = 1_700_000_000_000_000_000;

        let canonical = auki_jcs::canonicalize(&register_payload_value(
            cluster_name,
            &peer_id_str,
            &public_key_b64,
            &addresses,
            expected_app_id,
            note,
            timestamp_ns,
        ));

        // Locked canonical bytes — produced by JCS over the payload.
        // Keys are sorted lexicographically; this is the exact byte
        // stream Discovery's verifier reconstructs and feeds to
        // ed25519 verify.
        let canonical_str = std::str::from_utf8(&canonical)
            .expect("JCS output is valid UTF-8");
        let expected_canonical = format!(
            r#"{{"addresses":["/ip4/192.168.9.130/tcp/4001","/ip4/192.168.9.130/udp/4001/quic-v1"],"cluster_name":"vinland","expected_app_id":"sentinel","note":"locked vector","peer_id":"{peer_id_str}","public_key":"{public_key_b64}","timestamp_ns":{timestamp_ns}}}"#,
        );
        assert_eq!(
            canonical_str, expected_canonical,
            "JCS canonical bytes drifted — see crate docs for the locked recipe",
        );

        // And the ed25519 signature over those bytes is fixed.
        let sig = signing.sign(&canonical);
        let expected_sig: [u8; 64] = [
            0x28, 0x2d, 0x38, 0xcd, 0x3e, 0x1d, 0xca, 0x34, 0x2e, 0x17, 0x85, 0x5d, 0x23, 0x4a,
            0xad, 0x3d, 0xb6, 0x67, 0x05, 0xe3, 0xaa, 0x48, 0x2f, 0x04, 0xf1, 0x1b, 0x73, 0x0a,
            0xcb, 0xb4, 0xd5, 0x45, 0x0d, 0x8a, 0xa3, 0xdb, 0x85, 0x65, 0x2c, 0xd8, 0xab, 0xed,
            0xb9, 0x23, 0x9c, 0x1c, 0xb0, 0xe6, 0x25, 0x03, 0xed, 0xf1, 0x1d, 0xe7, 0xfb, 0xde,
            0x13, 0x76, 0x70, 0xc5, 0x77, 0x99, 0x33, 0x09,
        ];
        assert_eq!(
            sig.0, expected_sig,
            "ed25519 signature drifted — see crate docs for the locked recipe",
        );
    }

    /// `with_http` lets the caller override the default client (for
    /// instance, with a tighter timeout in tests). Smoke test: the
    /// resulting client uses the provided URL verbatim.
    #[test]
    fn with_http_uses_supplied_client_and_url() {
        let custom = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let client = DiscoveryClient::with_http("http://localhost:9999/", custom);
        assert_eq!(client.base_url(), "http://localhost:9999");
    }

    #[test]
    fn new_trims_trailing_slash() {
        let a = DiscoveryClient::new("http://localhost:9999");
        let b = DiscoveryClient::new("http://localhost:9999/");
        assert_eq!(a.base_url(), b.base_url());
    }
}

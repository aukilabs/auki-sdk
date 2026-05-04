//! `cluster.json` — the discovery doc.
//!
//! A static, hand-edited (or operator-generated) directory of pinned
//! peers for a cluster: the peer-ids and dialable multiaddrs of every
//! daemon the integrator wants this node to know about. Loaded at
//! daemon startup so a fresh process knows who its neighbours are
//! without a separate Discovery Service.
//!
//! ## What this is *not*
//!
//! - **Not a bootstrap address list.** `cluster.json` is the directory,
//!   not a hint set. Every entry has a known `peer_id`; libp2p Noise
//!   rejects mismatches at connection time, which is what gives identity
//!   continuity across daemon restarts.
//! - **Not authoritative for `app_id`.** The optional
//!   [`ClusterPeer::expected_app_id`] is advisory only — the
//!   authoritative `app_id` rides over the wire (per the daemon's
//!   `/api/info` response). The doc value is for fail-fast operator
//!   logging on mismatch.
//! - **Not signed.** The ansuz milestone treats `cluster.json` as a
//!   plain config file; cryptographic attestation of the cluster
//!   membership list is a future concern.
//!
//! ## Path layout
//!
//! Default: `<app_root>/registries/cluster_registries/cluster.json` —
//! sibling to `registries/sensors/`, `registries/clocks/`, and
//! `registries/frames/`. Unlike those, `cluster_registries/` is **flat**
//! (one `cluster.json`), not hash-keyed; ansuz doesn't lift the cluster
//! doc into a Cluster Registry primitive.
//!
//! Resolution order (highest priority first):
//!
//! 1. CLI override (`--cluster-doc <path>`, wired by integrator)
//! 2. `AUKI_CLUSTER_DOC` environment variable
//! 3. `<app_root>/registries/cluster_registries/cluster.json`
//!
//! [`resolve_path`] applies that precedence; integrators just hand it
//! their `app_root` and the optional CLI flag.
//!
//! ## Example
//!
//! ```json
//! {
//!   "version": 1,
//!   "cluster_name": "demo-2026-05",
//!   "peers": [
//!     {
//!       "peer_id": "12D3KooWGRUacXgYqsMd9V9zUYHEqtbwWSPN5x9eaA1k4VFZ7yK7",
//!       "addresses": [
//!         "/ip4/192.168.1.10/tcp/4001",
//!         "/ip4/192.168.1.10/udp/4001/quic-v1"
//!       ],
//!       "expected_app_id": "boosterapp",
//!       "note": "robot 1 — K1 NUC"
//!     }
//!   ]
//! }
//! ```
//!
//! ```no_run
//! # use std::path::Path;
//! # use auki_network::cluster_doc;
//! let path = cluster_doc::resolve_path(Path::new("/var/lib/boosterapp"), None);
//! let doc = cluster_doc::load(&path).expect("load cluster.json");
//! for peer in &doc.peers {
//!     println!("{} → {} addr(s)", peer.peer_id, peer.addresses.len());
//! }
//! ```

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// The schema version this loader speaks. `cluster.json` files with any
/// other value are rejected with [`LoadError::UnsupportedVersion`].
///
/// Bumped only on an incompatible shape change; additive fields use
/// `#[serde(default)]` and don't bump the version.
pub const SUPPORTED_VERSION: u32 = 1;

/// Environment variable consulted by [`resolve_path`]. Set this in a
/// daemon's launch environment to pin the discovery doc independent of
/// the app-root layout.
pub const ENV_OVERRIDE: &str = "AUKI_CLUSTER_DOC";

/// Sub-path under `<app_root>` where the doc lives by default. Sibling
/// of `registries/sensors/`, `registries/clocks/`, etc.
pub const DEFAULT_RELATIVE_PATH: &str = "registries/cluster_registries/cluster.json";

// ─── ClusterDoc ──────────────────────────────────────────────────────────────

/// The whole `cluster.json` document.
///
/// `peers` is an ordered list — duplicates and ordering are
/// operator-controlled; the loader does not deduplicate or sort. A peer
/// with zero `addresses` is permitted and round-trips cleanly; consumers
/// that need a dialable peer should filter such entries out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterDoc {
    /// Schema version. Must equal [`SUPPORTED_VERSION`]; checked
    /// post-deserialization in [`load`].
    pub version: u32,
    /// Human-readable cluster identifier. Surfaced in operator logs;
    /// no semantic role beyond labelling.
    pub cluster_name: String,
    /// Ordered list of pinned peers.
    pub peers: Vec<ClusterPeer>,
}

/// One pinned peer entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterPeer {
    /// Required. libp2p Noise rejects connection-time mismatches; this
    /// is what gives identity continuity across daemon restarts. A
    /// Boosterapp that reboots is recognizable as the same Boosterapp
    /// because the same wallet seed produces the same `peer_id`.
    pub peer_id: PeerId,
    /// Dialable multiaddrs for this peer. Direct (`/ip4/.../tcp/...`)
    /// or circuit-relay-mediated (`/p2p/<relay>/p2p-circuit/p2p/<peer>`)
    /// are both accepted by the loader; the swarm picks among them at
    /// dial time. Empty list is allowed (operator might temporarily
    /// remove all addresses while keeping the peer pinned).
    #[serde(with = "multiaddr_vec_serde")]
    pub addresses: Vec<Multiaddr>,
    /// Optional advisory `app_id` (e.g. `"boosterapp"`, `"sentinel"`).
    /// **Not authoritative** — the wire-borne `app_id` (from the
    /// daemon's `/api/info`) wins. Used for fail-fast operator logging
    /// when the doc and the daemon disagree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_app_id: Option<String>,
    /// Optional human-readable note. The loader preserves it but
    /// nothing in the SDK reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `multiaddr` 0.18 dropped its serde feature; we serialize each
/// `Multiaddr` as its canonical text form (`/ip4/.../tcp/...`) and parse
/// back via `FromStr`. Mirrors the adapter used by `ReachabilityRecord`.
mod multiaddr_vec_serde {
    use multiaddr::Multiaddr;
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(addrs: &Vec<Multiaddr>, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(addrs.len()))?;
        for a in addrs {
            seq.serialize_element(&a.to_string())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Multiaddr>, D::Error> {
        let strs: Vec<String> = Vec::deserialize(d)?;
        strs.into_iter()
            .map(|s| {
                Multiaddr::from_str(&s).map_err(|e| {
                    // Prefix with "multiaddr:" so the loader's
                    // classify_parse_error can lift this into
                    // LoadError::InvalidMultiaddr regardless of the
                    // upstream error's Display text.
                    serde::de::Error::custom(format!("multiaddr: parse {s:?}: {e}"))
                })
            })
            .collect()
    }
}

// ─── LoadError ───────────────────────────────────────────────────────────────

/// Errors from [`load`].
///
/// Variants match the failure modes a daemon needs to report distinctly
/// in operator logs: file not present, JSON malformed, schema mismatch,
/// or one of the strongly-typed fields (peer-id, multiaddr) failing to
/// parse. The `String` payloads on the parse-error variants carry the
/// offending text so the operator can fix the doc without a debugger.
#[derive(Debug)]
pub enum LoadError {
    /// Filesystem error opening or reading the file.
    Io(std::io::Error),
    /// JSON syntax error or shape mismatch (missing required field,
    /// wrong type). Includes the underlying `serde_json` diagnostic.
    Parse(serde_json::Error),
    /// The doc parsed but its `version` is not [`SUPPORTED_VERSION`].
    /// Carries the actual version so the operator can correlate.
    UnsupportedVersion(u32),
    /// A `peer_id` string failed to parse as a libp2p `PeerId`.
    /// Carries the offending string.
    InvalidPeerId(String),
    /// A multiaddr string failed to parse. Carries the offending string.
    InvalidMultiaddr(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "cluster.json: i/o: {e}"),
            LoadError::Parse(e) => write!(f, "cluster.json: parse: {e}"),
            LoadError::UnsupportedVersion(v) => write!(
                f,
                "cluster.json: unsupported version {v} (this loader speaks {SUPPORTED_VERSION})"
            ),
            LoadError::InvalidPeerId(s) => {
                write!(f, "cluster.json: invalid peer_id {s:?}")
            }
            LoadError::InvalidMultiaddr(s) => {
                write!(f, "cluster.json: invalid multiaddr {s:?}")
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io(e) => Some(e),
            LoadError::Parse(e) => Some(e),
            LoadError::UnsupportedVersion(_)
            | LoadError::InvalidPeerId(_)
            | LoadError::InvalidMultiaddr(_) => None,
        }
    }
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

// ─── load + path helpers ─────────────────────────────────────────────────────

/// Read and parse a `cluster.json` from `path`.
///
/// Two-phase parse: peek at the `version` field first so a v2+ doc with
/// a different shape still reports a clean
/// [`LoadError::UnsupportedVersion`] instead of an opaque
/// [`LoadError::Parse`]. Then full-deserialize the typed structure.
///
/// On `serde_json` errors during the typed phase we attempt to map the
/// underlying cause to a more specific variant — `peer_id` and multiaddr
/// fields fail through `serde::de::Error::custom` strings, which we
/// recognize and lift into [`LoadError::InvalidPeerId`] /
/// [`LoadError::InvalidMultiaddr`]. Any other parse failure stays as
/// [`LoadError::Parse`].
pub fn load(path: &Path) -> Result<ClusterDoc, LoadError> {
    let bytes = fs::read(path)?;

    // Phase 1: extract just the `version` field. A future v2 doc that
    // restructures the rest of the file should still report cleanly.
    #[derive(Deserialize)]
    struct VersionPeek {
        version: u32,
    }
    let peek: VersionPeek = serde_json::from_slice(&bytes).map_err(LoadError::Parse)?;
    if peek.version != SUPPORTED_VERSION {
        return Err(LoadError::UnsupportedVersion(peek.version));
    }

    // Phase 2: typed deserialize.
    let doc: ClusterDoc = serde_json::from_slice(&bytes).map_err(classify_parse_error)?;
    Ok(doc)
}

/// Default on-disk location: `<app_root>/registries/cluster_registries/cluster.json`.
/// Sibling to the hash-keyed registries; flat (single file) by design.
pub fn default_path(app_root: &Path) -> PathBuf {
    app_root.join(DEFAULT_RELATIVE_PATH)
}

/// Resolve the cluster-doc path with the standard precedence:
/// `cli_override > $AUKI_CLUSTER_DOC > <app_root>/<DEFAULT_RELATIVE_PATH>`.
///
/// Empty / unset env var falls through to the default, so a daemon's
/// launch script can leave `AUKI_CLUSTER_DOC` unset without forcing the
/// caller to special-case the absence.
pub fn resolve_path(app_root: &Path, cli_override: Option<&Path>) -> PathBuf {
    if let Some(p) = cli_override {
        return p.to_path_buf();
    }
    if let Some(p) = env::var_os(ENV_OVERRIDE) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    default_path(app_root)
}

/// Detect serde_json errors that originated from our `peer_id` /
/// multiaddr `serde::de::Error::custom` calls and lift them into the
/// stronger [`LoadError`] variants.
///
/// We classify by inspecting the error message for two signals: known
/// substrings emitted by the underlying parsers (multiaddr's `Display`,
/// libp2p PeerId's `Display`) and known substrings emitted by
/// `serde::de::Error::custom` from this module's own adapter. If
/// neither matches we keep the generic [`LoadError::Parse`] — which
/// is still correct, just less actionable.
///
/// The classifier is best-effort by design: if a future libp2p or
/// multiaddr release rephrases its error text, the worst that happens
/// is a borderline message degrades from `InvalidPeerId` to `Parse`.
/// Tests cover the current (multiaddr 0.18, libp2p-identity 0.2) text.
fn classify_parse_error(e: serde_json::Error) -> LoadError {
    let msg = e.to_string();
    let lower = msg.to_ascii_lowercase();
    // multiaddr 0.18's Error::Display strings include the word
    // "protocol" (e.g. "unknown protocol string: ...") or
    // "multiaddr"; the parser also fails with "invalid digit"-style
    // messages on shorter typos. Rather than chasing every variant we
    // anchor on three strong signals.
    let is_multiaddr = lower.contains("multiaddr")
        || lower.contains("unknown protocol")
        || lower.contains("invalid protocol");
    // libp2p-identity's PeerId Display says "decoding multihash" /
    // "base58" / "PeerId"; we additionally guard with the field-name
    // path serde_json puts in some errors.
    let is_peer_id = lower.contains("peer_id")
        || lower.contains("peer id")
        || lower.contains("multihash")
        || lower.contains("base58");

    if is_multiaddr {
        LoadError::InvalidMultiaddr(extract_quoted(&msg).unwrap_or(msg))
    } else if is_peer_id {
        LoadError::InvalidPeerId(extract_quoted(&msg).unwrap_or(msg))
    } else {
        LoadError::Parse(e)
    }
}

/// Pull the first double-quoted substring out of a serde error message
/// (the offending value), so the error variant carries something more
/// useful than the entire serde diagnostic. Returns `None` if there's
/// no quoted token.
fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let rest = &s[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A valid two-peer doc as both struct and canonical JSON. Used in
    /// round-trip and resolve tests. Peer-ids are real (derived from
    /// known seeds) so the parser actually validates them.
    fn sample_doc() -> ClusterDoc {
        // Use deterministic peer-ids derived via PeerIdentity so the
        // test doesn't bake in an opaque base58 string.
        let p1 = crate::PeerIdentity::from_seed(&[1u8; 32]).peer_id();
        let p2 = crate::PeerIdentity::from_seed(&[2u8; 32]).peer_id();
        ClusterDoc {
            version: 1,
            cluster_name: "demo-2026-05".to_string(),
            peers: vec![
                ClusterPeer {
                    peer_id: p1,
                    addresses: vec![
                        "/ip4/192.168.1.10/tcp/4001".parse().unwrap(),
                        "/ip4/192.168.1.10/udp/4001/quic-v1".parse().unwrap(),
                    ],
                    expected_app_id: Some("boosterapp".to_string()),
                    note: Some("robot 1 — K1 NUC".to_string()),
                },
                ClusterPeer {
                    peer_id: p2,
                    addresses: vec!["/ip4/10.0.0.5/tcp/4001".parse().unwrap()],
                    expected_app_id: Some("sentinel".to_string()),
                    note: None,
                },
            ],
        }
    }

    fn write_temp_file(contents: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("temp file");
        f.write_all(contents).expect("write");
        f.flush().expect("flush");
        f
    }

    #[test]
    fn round_trips_through_serde() {
        let original = sample_doc();
        let json = serde_json::to_string_pretty(&original).expect("serialize");
        let f = write_temp_file(json.as_bytes());
        let loaded = load(f.path()).expect("load");
        assert_eq!(loaded, original);
    }

    #[test]
    fn loads_canonical_example_from_spec() {
        // Mirrors the example in the crate README / spec doc verbatim
        // (modulo the peer-id, which has to be a real one).
        let p1 = crate::PeerIdentity::from_seed(&[1u8; 32]).peer_id();
        let json = format!(
            r#"{{
              "version": 1,
              "cluster_name": "demo-2026-05",
              "peers": [
                {{
                  "peer_id": "{}",
                  "addresses": [
                    "/ip4/192.168.1.10/tcp/4001",
                    "/ip4/192.168.1.10/udp/4001/quic-v1"
                  ],
                  "expected_app_id": "boosterapp",
                  "note": "robot 1 — K1 NUC"
                }}
              ]
            }}"#,
            p1
        );
        let f = write_temp_file(json.as_bytes());
        let doc = load(f.path()).expect("load");
        assert_eq!(doc.version, 1);
        assert_eq!(doc.cluster_name, "demo-2026-05");
        assert_eq!(doc.peers.len(), 1);
        assert_eq!(doc.peers[0].peer_id, p1);
        assert_eq!(doc.peers[0].addresses.len(), 2);
        assert_eq!(doc.peers[0].expected_app_id.as_deref(), Some("boosterapp"));
        assert_eq!(doc.peers[0].note.as_deref(), Some("robot 1 — K1 NUC"));
    }

    #[test]
    fn missing_optional_fields_default_to_none() {
        let p1 = crate::PeerIdentity::from_seed(&[3u8; 32]).peer_id();
        let json = format!(
            r#"{{
              "version": 1,
              "cluster_name": "minimal",
              "peers": [
                {{ "peer_id": "{p1}", "addresses": [] }}
              ]
            }}"#
        );
        let f = write_temp_file(json.as_bytes());
        let doc = load(f.path()).expect("load");
        assert_eq!(doc.peers[0].expected_app_id, None);
        assert_eq!(doc.peers[0].note, None);
        // Empty addresses is allowed — operator may have temporarily
        // pulled a peer's reachability while keeping it in the directory.
        assert!(doc.peers[0].addresses.is_empty());
    }

    #[test]
    fn io_error_for_missing_file() {
        let path = std::path::PathBuf::from("/definitely/does/not/exist/cluster.json");
        match load(&path) {
            Err(LoadError::Io(_)) => {}
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    #[test]
    fn parse_error_for_invalid_json() {
        let f = write_temp_file(b"this is not json {");
        match load(f.path()) {
            Err(LoadError::Parse(_)) => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_version_rejected() {
        let p1 = crate::PeerIdentity::from_seed(&[4u8; 32]).peer_id();
        let json = format!(
            r#"{{
              "version": 99,
              "cluster_name": "future",
              "peers": [
                {{ "peer_id": "{p1}", "addresses": [] }}
              ]
            }}"#
        );
        let f = write_temp_file(json.as_bytes());
        match load(f.path()) {
            Err(LoadError::UnsupportedVersion(99)) => {}
            other => panic!("expected UnsupportedVersion(99), got {other:?}"),
        }
    }

    #[test]
    fn version_one_accepted() {
        let p1 = crate::PeerIdentity::from_seed(&[5u8; 32]).peer_id();
        let json = format!(
            r#"{{
              "version": 1,
              "cluster_name": "v1-ok",
              "peers": [{{ "peer_id": "{p1}", "addresses": [] }}]
            }}"#
        );
        let f = write_temp_file(json.as_bytes());
        let doc = load(f.path()).expect("v1 should load");
        assert_eq!(doc.version, 1);
    }

    #[test]
    fn invalid_peer_id_rejected() {
        let json = r#"{
          "version": 1,
          "cluster_name": "bad-peer",
          "peers": [{ "peer_id": "not-a-real-peer-id", "addresses": [] }]
        }"#;
        let f = write_temp_file(json.as_bytes());
        match load(f.path()) {
            Err(LoadError::InvalidPeerId(_)) => {}
            other => panic!("expected InvalidPeerId, got {other:?}"),
        }
    }

    #[test]
    fn invalid_multiaddr_rejected() {
        let p1 = crate::PeerIdentity::from_seed(&[6u8; 32]).peer_id();
        let json = format!(
            r#"{{
              "version": 1,
              "cluster_name": "bad-multiaddr",
              "peers": [{{
                "peer_id": "{p1}",
                "addresses": ["this-is-not-a-multiaddr"]
              }}]
            }}"#
        );
        let f = write_temp_file(json.as_bytes());
        match load(f.path()) {
            Err(LoadError::InvalidMultiaddr(_)) => {}
            other => panic!("expected InvalidMultiaddr, got {other:?}"),
        }
    }

    #[test]
    fn default_path_is_under_registries_cluster_registries() {
        let app_root = std::path::Path::new("/var/lib/boosterapp");
        let path = default_path(app_root);
        assert_eq!(
            path,
            std::path::PathBuf::from(
                "/var/lib/boosterapp/registries/cluster_registries/cluster.json"
            )
        );
    }

    /// Process-wide lock for env-mutating tests. Cargo runs tests in a
    /// single process by default — touching a global env var without
    /// serialization races on read/write between threads. The mutex
    /// makes the four `resolve_path_*` env tests sequential among
    /// themselves while leaving the rest of the suite parallel.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn resolve_path_falls_back_to_default() {
        let _g = env_lock();
        // SAFETY: env::remove_var is unsafe per Rust 1.74+; we hold
        // the env_lock so no other env-mutating test races with us.
        unsafe { env::remove_var(ENV_OVERRIDE) };
        let app_root = std::path::Path::new("/srv/auki");
        let resolved = resolve_path(app_root, None);
        assert_eq!(resolved, default_path(app_root));
    }

    #[test]
    fn resolve_path_honours_cli_override() {
        let _g = env_lock();
        unsafe { env::remove_var(ENV_OVERRIDE) };
        let app_root = std::path::Path::new("/srv/auki");
        let cli = std::path::PathBuf::from("/etc/auki/cluster.json");
        let resolved = resolve_path(app_root, Some(&cli));
        assert_eq!(resolved, cli);
    }

    #[test]
    fn resolve_path_honours_env_override() {
        let _g = env_lock();
        let app_root = std::path::Path::new("/srv/auki");
        let env_path = "/opt/auki/cluster-from-env.json";
        // SAFETY: test-scoped env mutation under env_lock.
        unsafe { env::set_var(ENV_OVERRIDE, env_path) };
        let resolved = resolve_path(app_root, None);
        assert_eq!(resolved, std::path::PathBuf::from(env_path));
        unsafe { env::remove_var(ENV_OVERRIDE) };
    }

    #[test]
    fn resolve_path_cli_beats_env() {
        let _g = env_lock();
        let app_root = std::path::Path::new("/srv/auki");
        let env_path = "/opt/auki/cluster-from-env.json";
        let cli = std::path::PathBuf::from("/etc/auki/cluster-from-cli.json");
        // SAFETY: test-scoped env mutation under env_lock.
        unsafe { env::set_var(ENV_OVERRIDE, env_path) };
        let resolved = resolve_path(app_root, Some(&cli));
        assert_eq!(resolved, cli);
        unsafe { env::remove_var(ENV_OVERRIDE) };
    }

    #[test]
    fn resolve_path_treats_empty_env_as_unset() {
        // An exported-but-empty env var is a common operator mistake
        // (`AUKI_CLUSTER_DOC=` in a launch script). We treat it as if
        // the var weren't set at all rather than trying to load `""`.
        let _g = env_lock();
        unsafe { env::set_var(ENV_OVERRIDE, "") };
        let app_root = std::path::Path::new("/srv/auki");
        let resolved = resolve_path(app_root, None);
        assert_eq!(resolved, default_path(app_root));
        unsafe { env::remove_var(ENV_OVERRIDE) };
    }

    #[test]
    fn pretty_serialized_form_is_stable_under_round_trip() {
        // Belt-and-braces over `round_trips_through_serde`: confirms the
        // optional fields are skipped when None and the doc shape stays
        // legible to a human inspecting the file with $EDITOR.
        let p1 = crate::PeerIdentity::from_seed(&[7u8; 32]).peer_id();
        let doc = ClusterDoc {
            version: 1,
            cluster_name: "stability".to_string(),
            peers: vec![ClusterPeer {
                peer_id: p1,
                addresses: vec!["/ip4/127.0.0.1/tcp/4001".parse().unwrap()],
                expected_app_id: None,
                note: None,
            }],
        };
        let json = serde_json::to_string_pretty(&doc).unwrap();
        // None-valued optional fields are skipped on serialize.
        assert!(!json.contains("expected_app_id"));
        assert!(!json.contains("note"));
        // And the doc still round-trips clean.
        let f = write_temp_file(json.as_bytes());
        let back = load(f.path()).expect("load");
        assert_eq!(doc, back);
    }
}

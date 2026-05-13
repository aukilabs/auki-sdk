//! Python bindings for `auki-domain`.
//!
//! Exposes `ClusterMembership` and `ClusterMember` so Python daemons
//! can construct, read, mutate, and JSON-round-trip the authoritative
//! cluster-membership document the Manager owns.
//!
//! Surface (under the `auki_domain` Python module):
//!
//! - `ClusterMember(peer_id, multiaddrs, join_ts_ns, successor_token=None)`
//!   — value-type pyclass mirroring the Rust struct of the same name.
//! - `ClusterMembership(cluster_name)` + `.peers`, `.admit(member)`,
//!   `.filename`, `.to_json()`, `ClusterMembership.from_json(s)` —
//!   matches the Rust API one-to-one.
//!
//! Strings on the boundary: `peer_id` is the canonical libp2p
//! peer-id string; `multiaddrs` are the canonical text form
//! (`/ip4/.../tcp/...`). The wrapper parses these at construction
//! and re-stringifies them on read so Python code never sees a raw
//! byte buffer.

use auki_domain_rs::{ClusterMember, ClusterMembership};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use std::str::FromStr;

// ─── ClusterMember pyclass ─────────────────────────────────────────

/// One peer in a `ClusterMembership` document.
#[pyclass(name = "ClusterMember")]
#[derive(Clone)]
pub struct PyClusterMember {
    inner: ClusterMember,
}

#[pymethods]
impl PyClusterMember {
    /// Construct a new member.
    ///
    /// - `peer_id`: canonical libp2p peer-id string (e.g. `"12D3KooW…"`).
    /// - `multiaddrs`: list of canonical multiaddr strings.
    /// - `join_ts_ns`: unix nanoseconds at which the Manager admitted
    ///   this peer.
    /// - `successor_token`: optional opaque bytes; `None` for v1 demo
    ///   peers (the v1 Discovery contract skips signature verification).
    #[new]
    #[pyo3(signature = (peer_id, multiaddrs, join_ts_ns, successor_token = None))]
    fn new(
        peer_id: &str,
        multiaddrs: Vec<String>,
        join_ts_ns: i64,
        successor_token: Option<Vec<u8>>,
    ) -> PyResult<Self> {
        let peer_id_parsed = PeerId::from_str(peer_id)
            .map_err(|e| PyValueError::new_err(format!("invalid peer_id {peer_id:?}: {e}")))?;
        let multiaddrs = multiaddrs
            .into_iter()
            .map(|s| {
                Multiaddr::from_str(&s)
                    .map_err(|e| PyValueError::new_err(format!("invalid multiaddr {s:?}: {e}")))
            })
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self {
            inner: ClusterMember {
                peer_id: peer_id_parsed,
                multiaddrs,
                join_ts_ns,
                successor_token,
            },
        })
    }

    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id.to_string()
    }

    #[getter]
    fn multiaddrs(&self) -> Vec<String> {
        self.inner.multiaddrs.iter().map(|m| m.to_string()).collect()
    }

    #[getter]
    fn join_ts_ns(&self) -> i64 {
        self.inner.join_ts_ns
    }

    #[getter]
    fn successor_token(&self) -> Option<Vec<u8>> {
        self.inner.successor_token.clone()
    }

    fn __repr__(&self) -> String {
        let token = match &self.inner.successor_token {
            Some(b) => format!("<{} bytes>", b.len()),
            None => "None".to_string(),
        };
        format!(
            "ClusterMember(peer_id={:?}, multiaddrs={:?}, join_ts_ns={}, successor_token={})",
            self.inner.peer_id.to_string(),
            self.multiaddrs(),
            self.inner.join_ts_ns,
            token,
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── ClusterMembership pyclass ─────────────────────────────────────

/// The cluster's authoritative membership document.
///
/// Held by the current Manager in RAM. The filename convention is
/// `<cluster_name>.json` — see [`filename`].
#[pyclass(name = "ClusterMembership")]
pub struct PyClusterMembership {
    inner: ClusterMembership,
}

#[pymethods]
impl PyClusterMembership {
    /// Construct an empty membership document for `cluster_name`.
    #[new]
    fn new(cluster_name: String) -> Self {
        Self {
            inner: ClusterMembership::new(cluster_name),
        }
    }

    /// Parse a JSON string into a `ClusterMembership`. Inverse of
    /// `to_json`.
    #[staticmethod]
    fn from_json(s: &str) -> PyResult<Self> {
        let inner: ClusterMembership = serde_json::from_str(s)
            .map_err(|e| PyValueError::new_err(format!("invalid ClusterMembership JSON: {e}")))?;
        Ok(Self { inner })
    }

    #[getter]
    fn cluster_name(&self) -> String {
        self.inner.cluster_name.clone()
    }

    #[getter]
    fn peers(&self) -> Vec<PyClusterMember> {
        self.inner
            .peers
            .iter()
            .map(|m| PyClusterMember { inner: m.clone() })
            .collect()
    }

    /// The per-cluster filename: `<cluster_name>.json`.
    #[getter]
    fn filename(&self) -> String {
        self.inner.filename()
    }

    /// Append a member. Returns the index of the new entry.
    fn admit(&mut self, member: &PyClusterMember) -> usize {
        self.inner.admit(member.inner.clone())
    }

    /// Serialize to JSON. Inverse of `from_json`.
    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner)
            .map_err(|e| PyTypeError::new_err(format!("serializing ClusterMembership: {e}")))
    }

    fn __repr__(&self) -> String {
        format!(
            "ClusterMembership(cluster_name={:?}, peers={})",
            self.inner.cluster_name,
            self.inner.peers.len(),
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── Module entry point ────────────────────────────────────────────

/// `auki_domain` Python module.
#[pymodule]
fn auki_domain(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyClusterMember>()?;
    m.add_class::<PyClusterMembership>()?;
    Ok(())
}

//! Python wrapper for `auki_network::discovery_client` (Vinland Batch 2).
//!
//! Sync-shaped surface per [Pattern A](https://www.notion.so/3585c8e9659280699681caec256e0616)
//! — the SDK owns the asyncio loop on a daemon Python thread; sidecar
//! consumers stay sync-shaped. Each Python method internally
//! `block_on`s the async Rust method on the wrapper's process-wide
//! tokio runtime ([`crate::cluster_tokio_runtime`]); callers do not
//! need a runtime of their own.
//!
//! ## Surface
//!
//! ```python
//! from auki_network.discovery import (
//!     DiscoveryClient,
//!     DiscoveryUnreachable, DiscoveryRejected, DiscoveryClockError,
//! )
//!
//! client = DiscoveryClient("http://10.0.0.5:8080")
//!
//! # register: takes the PARENT wallet seed (32 bytes). The client
//! # internally derives the peer-key wallet via derive_child("peer/v1")
//! # and signs with that, putting the child's pubkey + corresponding
//! # peer_id on the wire.
//! doc = client.register(
//!     seed=wallet_seed,
//!     cluster_name="vinland",
//!     addresses=["/ip4/192.168.9.130/tcp/4001"],
//!     expected_app_id="sentinel",  # optional
//!     note=None,                    # optional
//! )
//!
//! doc = client.fetch("vinland")
//!
//! client.deregister(seed=wallet_seed, cluster_name="vinland")
//! ```
//!
//! ## `seed` is the PARENT wallet seed
//!
//! Note: `discovery.DiscoveryClient.register` takes the **parent**
//! wallet seed; `cluster.spawn(seed=...)` takes the **peer** seed
//! (already derived via `wallet.derive_child("peer/v1")`). The two are
//! different by convention:
//!
//! | call | seed argument | derivation |
//! | --- | --- | --- |
//! | `cluster.spawn(seed=...)` | peer seed | caller derives via `wallet.derive_child("peer/v1").seed()` |
//! | `discovery.DiscoveryClient.register(seed=...)` | parent seed | wrapper derives internally |
//!
//! The asymmetry mirrors the underlying Rust APIs:
//! `auki_network::cluster_runtime::ClusterRuntime::spawn` constructs
//! the swarm via `PeerIdentity::from_seed(seed)` (direct ed25519, no
//! derivation), while `auki_network::discovery_client::register`
//! accepts a `&Wallet` and does the `derive_child(PEER_DERIVATION_LABEL)`
//! itself. Each is the more natural shape on its own side of the FFI.
//! Sidecar boilerplate:
//!
//! ```python
//! wallet_seed = auki_identity.load_or_mint_seed(seed_path)  # 32 bytes
//! wallet     = auki_identity.Wallet.from_seed(wallet_seed)
//! peer       = wallet.derive_child("peer/v1")
//!
//! # Discovery: parent seed.
//! discovery_client.register(seed=wallet_seed, cluster_name="vinland", ...)
//!
//! # Cluster: peer seed.
//! cluster.spawn(seed=peer.seed(), doc=..., participant_provider=...)
//! ```
//!
//! ## Errors
//!
//! Three typed Python exceptions cover the failure modes:
//!
//! - **`DiscoveryUnreachable`** — transport / DNS / connect / TLS /
//!   timeout. Discovery isn't reachable; the daemon should retry or
//!   surface as a startup failure (per Vinland D3, the SDK does not
//!   fall back to a local `cluster.json`).
//! - **`DiscoveryRejected`** — HTTP non-2xx response. Carries
//!   `.status: int` and `.body: str` attributes so the operator log
//!   shows Discovery's reason verbatim (`{"error": "signature does
//!   not verify"}` etc.).
//! - **`DiscoveryClockError`** — `SystemTime::now()` failed (pre-1970
//!   system clock or post-2262). Rare; means the host clock is broken.

use crate::ClusterDoc;
use crate::cluster_tokio_runtime;
use auki_network_rs::discovery_client::{
    CreateClusterOutcome as RustCreateClusterOutcome, DiscoveryClient as RustDiscoveryClient,
    DiscoveryError as RustDiscoveryError,
};
use multiaddr::Multiaddr;
use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};
use std::str::FromStr;

// ─── Exceptions ──────────────────────────────────────────────────────────────────────
//
// One Python exception per Rust `DiscoveryError` variant. Modeled via
// `create_exception!` so consumers catch by name. The `Rejected` variant
// carries `.status` and `.body` attributes for operator log clarity.

create_exception!(
    auki_network,
    DiscoveryUnreachable,
    pyo3::exceptions::PyException,
    "Discovery isn't reachable: transport / DNS / connect / TLS / \
     timeout. The daemon should retry or surface as a startup failure \
     (Vinland D3 — the SDK does not fall back to a local cluster.json)."
);

create_exception!(
    auki_network,
    DiscoveryRejected,
    pyo3::exceptions::PyException,
    "Discovery responded with a non-2xx status. Carries `.status: int` \
     and `.body: str` attributes; e.g. status=401 body='{\"error\":\
     \"signature does not verify\"}' or status=403 body='{\"error\":\
     \"... replay window ...\"}'."
);

create_exception!(
    auki_network,
    DiscoveryClockError,
    pyo3::exceptions::PyException,
    "System clock failed (pre-1970 or post-2262). Rare; the host clock \
     is broken — the SDK can't sign without a valid timestamp."
);

fn map_discovery_error(err: RustDiscoveryError) -> PyErr {
    match err {
        RustDiscoveryError::Transport(e) => DiscoveryUnreachable::new_err(format!("{e}")),
        RustDiscoveryError::Status { status, body } => {
            // Build the exception with `.status` and `.body` attached.
            // PyO3's `create_exception!` only gives us the bare class;
            // we instantiate via `new_err` and set the attributes
            // afterwards in the Rust→Python conversion seam.
            Python::with_gil(|py| {
                let err =
                    DiscoveryRejected::new_err((format!("HTTP {status}: {body}"),));
                if let Ok(instance) = err.value_bound(py).downcast::<pyo3::types::PyAny>() {
                    let _ = instance.setattr("status", status);
                    let _ = instance.setattr("body", body);
                }
                err
            })
        }
        RustDiscoveryError::Clock(msg) => DiscoveryClockError::new_err(msg),
    }
}

// ─── DiscoveryClient ───────────────────────────────────────────────────────────────────
//
// Sync-shaped wrapper over Rust's async `DiscoveryClient`. Each method
// blocks the calling Python thread until the round trip completes;
// internally `block_on`s on the shared `cluster_tokio_runtime()` so the
// caller doesn't need a runtime of their own.

/// REST client for an Auki Discovery service.
///
/// Cheap to construct; shareable across Python threads (the underlying
/// `reqwest::Client` pools connections internally). One instance pins
/// one base URL.
#[pyclass(name = "DiscoveryClient")]
pub struct DiscoveryClient {
    inner: RustDiscoveryClient,
}

#[pymethods]
impl DiscoveryClient {
    /// Construct a client targeting `url` (e.g.
    /// `"http://10.0.0.5:8080"`). A trailing `/` is trimmed.
    #[new]
    #[pyo3(text_signature = "(url, /)")]
    fn new(url: &str) -> Self {
        Self {
            inner: RustDiscoveryClient::new(url),
        }
    }

    /// Base URL this client targets, sans trailing slash. Useful for
    /// log messages.
    #[getter]
    fn base_url(&self) -> &str {
        self.inner.base_url()
    }

    /// Sign and POST a registration for the wallet whose seed bytes are
    /// `seed` (32 bytes — the **parent** wallet seed; the wrapper
    /// derives the peer-key wallet via `derive_child("peer/v1")`
    /// internally and signs with that). Discovery upserts on `peer_id`
    /// and returns the full `ClusterDoc` as it stood after the upsert.
    ///
    /// Sync-blocking: this thread is parked until Discovery responds
    /// or the request times out.
    ///
    /// `addresses` is a list of multiaddr strings (e.g.
    /// `"/ip4/192.168.1.10/tcp/4001"`). Empty list is allowed.
    ///
    /// `expected_app_id` and `note` are advisory operator metadata
    /// passed through verbatim.
    ///
    /// Raises:
    /// - `ValueError` if `seed` isn't exactly 32 bytes or any string
    ///   in `addresses` doesn't parse as a multiaddr.
    /// - `DiscoveryUnreachable` if Discovery isn't reachable at all.
    /// - `DiscoveryRejected` (with `.status` + `.body`) if Discovery
    ///   returned non-2xx (typically 401 signature mismatch, 403 replay
    ///   window, 400 cluster_name charset, 404 unknown cluster).
    /// - `DiscoveryClockError` if `SystemTime::now()` is broken.
    #[pyo3(
        text_signature = "($self, seed, cluster_name, addresses, *, expected_app_id=None, note=None)",
        signature = (
            seed,
            cluster_name,
            addresses,
            *,
            expected_app_id = None,
            note = None,
        ),
    )]
    fn register(
        &self,
        py: Python<'_>,
        seed: &Bound<'_, PyBytes>,
        cluster_name: &str,
        addresses: Vec<String>,
        expected_app_id: Option<String>,
        note: Option<String>,
    ) -> PyResult<ClusterDoc> {
        let wallet = wallet_from_seed_bytes(seed)?;
        let addrs = parse_multiaddrs(&addresses)?;

        let inner = self.inner.clone();
        let cluster_name = cluster_name.to_string();
        let expected_app_id_owned = expected_app_id.clone();
        let note_owned = note.clone();
        let result = py.allow_threads(|| {
            let rt = cluster_tokio_runtime();
            rt.block_on(async {
                inner
                    .register(
                        &wallet,
                        &cluster_name,
                        &addrs,
                        expected_app_id_owned.as_deref(),
                        note_owned.as_deref(),
                    )
                    .await
            })
        });

        match result {
            Ok(doc) => Ok(ClusterDoc { inner: doc }),
            Err(e) => Err(map_discovery_error(e)),
        }
    }

    /// `POST /clusters/{cluster_name}` — atomically create a cluster.
    /// `seed` is the **parent** wallet seed; the wrapper signs with the
    /// caller's wallet so Discovery can identify and record the
    /// initial Manager (Greenland T1 / T8 — singleton-cluster bootstrap
    /// path on the way to wallet-scoped Domains).
    ///
    /// Returns a [`CreateClusterOutcome`] tagged with `kind` ==
    /// `"created"` (HTTP 201; signing peer became initial Manager) or
    /// `"already_exists"` (HTTP 409; another peer beat this call to
    /// creation). On `already_exists`, the returned outcome carries
    /// the winner's [`ClusterDoc`] parsed from Discovery's
    /// `{ error: "already_exists", existing: ClusterDoc }` body — the
    /// loser hands `outcome.doc` straight to a join flow without an
    /// extra `fetch`. Daemons implementing the Greenland T12
    /// `try-join → create-if-none → fall-back-to-join` algorithm
    /// branch on `kind` after `create_cluster` returns.
    ///
    /// Sync-blocking. Raises the same exceptions as `register` for
    /// non-409 transport / parse / status failures.
    #[pyo3(text_signature = "($self, seed, cluster_name)")]
    fn create_cluster(
        &self,
        py: Python<'_>,
        seed: &Bound<'_, PyBytes>,
        cluster_name: &str,
    ) -> PyResult<CreateClusterOutcome> {
        let wallet = wallet_from_seed_bytes(seed)?;
        let inner = self.inner.clone();
        let cluster_name = cluster_name.to_string();
        let result = py.allow_threads(|| {
            let rt = cluster_tokio_runtime();
            rt.block_on(async move { inner.create_cluster(&wallet, &cluster_name).await })
        });
        match result {
            Ok(RustCreateClusterOutcome::Created(doc)) => Ok(CreateClusterOutcome {
                kind: "created",
                doc: ClusterDoc { inner: doc },
            }),
            Ok(RustCreateClusterOutcome::AlreadyExists { existing }) => Ok(CreateClusterOutcome {
                kind: "already_exists",
                doc: ClusterDoc { inner: existing },
            }),
            Err(e) => Err(map_discovery_error(e)),
        }
    }

    /// `GET /clusters/{cluster_name}` — fetch the current cluster doc.
    /// Read-only; doesn't sign anything.
    ///
    /// Sync-blocking. Raises the same exceptions as `register`.
    #[pyo3(text_signature = "($self, cluster_name, /)")]
    fn fetch(&self, py: Python<'_>, cluster_name: &str) -> PyResult<ClusterDoc> {
        let inner = self.inner.clone();
        let cluster_name = cluster_name.to_string();
        let result = py.allow_threads(|| {
            let rt = cluster_tokio_runtime();
            rt.block_on(async move { inner.fetch(&cluster_name).await })
        });
        match result {
            Ok(doc) => Ok(ClusterDoc { inner: doc }),
            Err(e) => Err(map_discovery_error(e)),
        }
    }

    /// Sign and DELETE the daemon's own peer entry. `seed` is the
    /// **parent** wallet seed; the wrapper derives the peer-key wallet
    /// internally and signs with that.
    ///
    /// Discovery treats a second deregister against an already-removed
    /// entry as `404`, which surfaces here as `DiscoveryRejected` with
    /// `.status == 404`. Daemons that want clean-shutdown idempotency
    /// catch `DiscoveryRejected` and ignore status 404.
    ///
    /// Sync-blocking. Raises the same exceptions as `register`.
    #[pyo3(text_signature = "($self, seed, cluster_name)")]
    fn deregister(
        &self,
        py: Python<'_>,
        seed: &Bound<'_, PyBytes>,
        cluster_name: &str,
    ) -> PyResult<()> {
        let wallet = wallet_from_seed_bytes(seed)?;
        let inner = self.inner.clone();
        let cluster_name = cluster_name.to_string();
        let result = py.allow_threads(|| {
            let rt = cluster_tokio_runtime();
            rt.block_on(async move { inner.deregister(&wallet, &cluster_name).await })
        });
        match result {
            Ok(()) => Ok(()),
            Err(e) => Err(map_discovery_error(e)),
        }
    }

    fn __repr__(&self) -> String {
        format!("DiscoveryClient(base_url={:?})", self.inner.base_url())
    }
}

// ─── CreateClusterOutcome ────────────────────────────────────────────────────────────────

/// Outcome of [`DiscoveryClient.create_cluster`]. Mirrors the Rust enum
/// `auki_network::discovery_client::CreateClusterOutcome`.
///
/// `kind` is the discriminator string — `"created"` (HTTP 201; this
/// peer became initial Manager) or `"already_exists"` (HTTP 409;
/// another peer beat this call to creation). `doc` carries the
/// resulting [`ClusterDoc`] in either case — the winner's state in
/// the `already_exists` case, parsed from Discovery's
/// `{ error: "already_exists", existing: ClusterDoc }` body so the
/// loser can hand it straight to a join flow without an extra
/// `fetch`.
///
/// Greenland T12's `try-join → create-if-none → fall-back-to-join`
/// algorithm branches on `kind` after `create_cluster` returns:
/// `"created"` → register against `doc` as the initial Manager;
/// `"already_exists"` → register against `doc` as a joiner.
#[pyclass(frozen)]
#[derive(Clone, Debug)]
pub struct CreateClusterOutcome {
    /// `"created"` (201) or `"already_exists"` (409).
    #[pyo3(get)]
    pub kind: &'static str,
    /// Newly-created cluster doc on `created`, or the winning peer's
    /// existing cluster doc on `already_exists`. Either way, callable
    /// callers can register against this doc.
    #[pyo3(get)]
    pub doc: ClusterDoc,
}

#[pymethods]
impl CreateClusterOutcome {
    fn __repr__(&self) -> String {
        format!(
            "CreateClusterOutcome(kind={:?}, doc=<{} peers>)",
            self.kind,
            self.doc.peer_count(),
        )
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────────────────

fn wallet_from_seed_bytes(seed: &Bound<'_, PyBytes>) -> PyResult<auki_identity::Wallet> {
    let bytes = seed.as_bytes();
    if bytes.len() != 32 {
        return Err(PyValueError::new_err(format!(
            "seed must be exactly 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    Ok(auki_identity::Wallet::from_seed(&arr))
}

fn parse_multiaddrs(addrs: &[String]) -> PyResult<Vec<Multiaddr>> {
    addrs
        .iter()
        .map(|s| {
            Multiaddr::from_str(s)
                .map_err(|e| PyValueError::new_err(format!("invalid multiaddr {s:?}: {e}")))
        })
        .collect()
}

// ─── Module registration ───────────────────────────────────────────────────────────────

pub(crate) fn register_module(py: Python<'_>, discovery: &Bound<'_, PyModule>) -> PyResult<()> {
    discovery.add_class::<DiscoveryClient>()?;
    discovery.add_class::<CreateClusterOutcome>()?;
    discovery.add(
        "DiscoveryUnreachable",
        py.get_type_bound::<DiscoveryUnreachable>(),
    )?;
    discovery.add(
        "DiscoveryRejected",
        py.get_type_bound::<DiscoveryRejected>(),
    )?;
    discovery.add(
        "DiscoveryClockError",
        py.get_type_bound::<DiscoveryClockError>(),
    )?;
    Ok(())
}

// ─── Tests ─────────────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_client_constructs_and_trims_url() {
        Python::with_gil(|_py| {
            let a = DiscoveryClient::new("http://localhost:9999");
            let b = DiscoveryClient::new("http://localhost:9999/");
            assert_eq!(a.base_url(), "http://localhost:9999");
            assert_eq!(a.base_url(), b.base_url());
        });
    }

    #[test]
    fn register_rejects_wrong_seed_length() {
        Python::with_gil(|py| {
            let client = DiscoveryClient::new("http://localhost:9999");
            let bad_seed = PyBytes::new_bound(py, &[0u8; 16]);
            let err = client
                .register(
                    py,
                    &bad_seed,
                    "vinland",
                    vec!["/ip4/127.0.0.1/tcp/4001".into()],
                    None,
                    None,
                )
                .expect_err("16-byte seed must be rejected");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(
                err.to_string().contains("32 bytes"),
                "error must name the 32-byte requirement: {err}",
            );
        });
    }

    #[test]
    fn register_rejects_invalid_multiaddr() {
        Python::with_gil(|py| {
            let client = DiscoveryClient::new("http://localhost:9999");
            let seed = PyBytes::new_bound(py, &[1u8; 32]);
            let err = client
                .register(
                    py,
                    &seed,
                    "vinland",
                    vec!["this-is-not-a-multiaddr".into()],
                    None,
                    None,
                )
                .expect_err("invalid multiaddr must be rejected");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(err.to_string().contains("invalid multiaddr"));
        });
    }

    #[test]
    fn deregister_rejects_wrong_seed_length() {
        Python::with_gil(|py| {
            let client = DiscoveryClient::new("http://localhost:9999");
            let bad_seed = PyBytes::new_bound(py, &[0u8; 64]);
            let err = client
                .deregister(py, &bad_seed, "vinland")
                .expect_err("64-byte seed must be rejected");
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }

    #[test]
    fn fetch_against_unreachable_url_raises_unreachable() {
        // Port 1 is reserved + RST'd immediately by Linux/macOS, so
        // the connect fails fast and we get a transport error.
        Python::with_gil(|py| {
            let client = DiscoveryClient::new("http://127.0.0.1:1");
            let err = client
                .fetch(py, "vinland")
                .expect_err("unreachable URL must error");
            assert!(
                err.is_instance_of::<DiscoveryUnreachable>(py),
                "expected DiscoveryUnreachable, got: {err}",
            );
        });
    }

    #[test]
    fn module_registration_exposes_the_documented_surface() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "discovery").unwrap();
            register_module(py, &module).unwrap();
            assert!(module.getattr("DiscoveryClient").is_ok());
            assert!(module.getattr("DiscoveryUnreachable").is_ok());
            assert!(module.getattr("DiscoveryRejected").is_ok());
            assert!(module.getattr("DiscoveryClockError").is_ok());
        });
    }
}

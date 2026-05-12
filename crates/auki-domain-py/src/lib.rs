//! PyO3 bindings for [`auki_domain`].
//!
//! Exposes [`init_domain`] — the post-v0.0.33 entry point Python daemons
//! use to construct a [`ClusterRuntime`] through Discovery. The
//! `auki_network.cluster.spawn` Python function was removed in
//! [auki-sdk v0.0.33] (cluster-trust-boundary PR B); this crate is the
//! replacement.
//!
//! ## Surface
//!
//! - [`init_domain`] — synchronous-blocking Python function. Builds the
//!   libp2p swarm internally, drives `auki_domain::init_domain` on a
//!   process-wide tokio runtime, returns a [`DomainHandle`].
//! - [`DomainHandle`] — opaque Python handle with `.identity` (the
//!   canonical Domain string), `.peers()` (snapshot of connected peers),
//!   and `.shutdown()` (consumes the runtime).
//! - Typed Python exceptions for the four [`InitDomainError`] paths.
//!
//! ## What's NOT in this PR
//!
//! - **`stream_provider` Python callable.** `init_domain` accepts no
//!   `stream_provider` kwarg yet — the wrapper passes
//!   `auki_network::stream_runtime::decline_all_streams()` so producer-
//!   side stream support is degraded vs. the pre-v0.0.33
//!   `auki_network.cluster.spawn` surface. Wiring a Python callable
//!   through to a Rust `StreamProvider` requires reusing
//!   `auki-network-py`'s `build_stream_provider` (currently
//!   `pub(crate)`) or copying ~500 lines of `PyStreamDecision` /
//!   `PyAcceptInfo` / `PyDeclineReason` plumbing. Filed in
//!   [`parking_lot.md`](../parking_lot.md) as the immediate follow-up.
//!   BoosterApp + Sentinel daemons can `init_domain` and run peer-list
//!   logic today, but can't accept inbound stream subscriptions until
//!   the follow-up lands.
//!
//! - **`runtime.open_*_stream()` consumer-side methods.** Park (the
//!   consumer) is Rust-side and uses `auki-network`'s Rust API
//!   directly. No Python daemon consumes streams today, so the consumer
//!   surface deferred. Same follow-up.
//!
//! - **`runtime.update_cluster_doc(new_doc)` for SSE-driven membership
//!   refresh.** `init_domain` only performs the initial create-and-
//!   register; the daemon is expected to subscribe to Discovery's SSE
//!   stream and feed fresh `ClusterDoc`s in so the libp2p allow-list
//!   stays in sync. The `ClusterDoc` Python pyclass would need to be
//!   reachable from this crate; filed as a follow-up. In the meantime,
//!   the local allow-list reflects the cluster's membership at
//!   `init_domain`-time; peers that join after won't be dialable
//!   until the daemon restarts.
//!
//! [auki-sdk v0.0.33]: https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.33

use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use auki_domain_rs::{init_domain as rust_init_domain, DomainHandle as RustDomainHandle, InitDomainError};
use auki_identity::Wallet;
use auki_network::cluster_runtime::{
    ClusterRuntime as RustClusterRuntime, ParticipantInfoProvider,
    PeerSnapshot as RustPeerSnapshot,
};
use auki_network::discovery_client::{DiscoveryClient, DiscoveryError};
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{build_swarm, SwarmConfig};
use auki_network::{ParticipantInfo as RustParticipantInfo, PeerIdentity};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use pyo3::create_exception;
use pyo3::exceptions::{PyConnectionError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use tokio::runtime::Runtime;

// ─── Typed Python exceptions ─────────────────────────────────────────

create_exception!(
    auki_domain,
    DiscoveryUnreachable,
    PyConnectionError,
    "Discovery's host couldn't be reached. Carries the original message; \
     transport-level failure (DNS, refused connect, TLS handshake)."
);

create_exception!(
    auki_domain,
    DiscoveryRejected,
    PyRuntimeError,
    "Discovery responded with a non-2xx status (and the response wasn't \
     the typed-409 path that becomes `DomainAlreadyExists`). Carries \
     `.status: int` and `.body: str` so daemons can log specifics."
);

create_exception!(
    auki_domain,
    DiscoveryClockError,
    PyRuntimeError,
    "Discovery's clock and ours disagree by more than its allowed skew. \
     Restart the local NTP loop and retry."
);

create_exception!(
    auki_domain,
    DomainAlreadyExists,
    PyRuntimeError,
    "Discovery's atomic `POST /clusters/{name}` returned 409 — another \
     peer beat this call to creation. Greenland T12 callers branch on \
     this exception to fall back to a join flow. Carries \
     `.identity: str` (the canonical string this `init_domain` was \
     trying to claim) and `.cluster_name: str` (the same value, named \
     for symmetry with Discovery's wire field). The winner's full \
     `ClusterDoc` would surface as a third attribute once `ClusterDoc` \
     is reachable from this crate — filed as a follow-up; today, \
     daemons that hit this exception can re-issue `init_domain` with \
     the same name and the SDK's second call will go through the \
     non-create register path."
);

create_exception!(
    auki_domain,
    RuntimeSpawnError,
    PyRuntimeError,
    "`ClusterRuntime::from_swarm` failed AFTER Discovery had already \
     accepted the create + register. The cluster exists and the peer is \
     registered; the local runtime didn't construct. Daemons that hit \
     this may want to deregister before retrying so Discovery doesn't \
     carry a phantom peer entry."
);

fn map_init_domain_error(err: InitDomainError) -> PyErr {
    match err {
        InitDomainError::Discovery(d) => map_discovery_error(d),
        InitDomainError::AlreadyExists { identity, existing: _ } => {
            // `existing` is `auki_network::cluster_doc::ClusterDoc`. Surfacing
            // it as a Python `ClusterDoc` requires either depending on
            // `auki-network-py`'s pyclass (lib-name collision) or duplicating
            // the wire-shape pyclass here. Both are out-of-scope for the
            // initial PR; we surface `identity` + `cluster_name` strings so
            // the Greenland T12 fall-back-to-join retry can reissue
            // `init_domain` with the same name (the second call will hit
            // Discovery's existing-cluster path through `register`, not
            // `create_cluster`'s 409 path). Filed as a follow-up.
            let canonical = identity.canonical_string();
            Python::with_gil(|py| {
                let exc = DomainAlreadyExists::new_err((format!(
                    "Domain {canonical} already exists; another peer created it first"
                ),));
                if let Ok(instance) = exc.value_bound(py).downcast::<pyo3::PyAny>() {
                    let _ = instance.setattr("identity", &canonical);
                    let _ = instance.setattr("cluster_name", &canonical);
                }
                exc
            })
        }
        InitDomainError::RuntimeSpawn(s) => {
            RuntimeSpawnError::new_err(format!("ClusterRuntime construction failed: {s}"))
        }
    }
}

fn map_discovery_error(err: DiscoveryError) -> PyErr {
    match err {
        DiscoveryError::Transport(t) => {
            DiscoveryUnreachable::new_err(format!("transport error: {t}"))
        }
        DiscoveryError::Status { status, body } => {
            Python::with_gil(|py| {
                let exc = DiscoveryRejected::new_err((format!("HTTP {status}: {body}"),));
                if let Ok(instance) = exc.value_bound(py).downcast::<pyo3::PyAny>() {
                    let _ = instance.setattr("status", status);
                    let _ = instance.setattr("body", body);
                }
                exc
            })
        }
        DiscoveryError::Clock(detail) => DiscoveryClockError::new_err(detail),
    }
}

// ─── DomainHandle ────────────────────────────────────────────────────

/// Live handle to a Domain the local daemon is participating in.
///
/// Returned by [`init_domain`]. Wraps the underlying
/// [`auki_domain::DomainHandle`] (which owns the `ClusterRuntime`).
/// The handle's lifecycle is:
///
/// - Construct via `init_domain(...)`.
/// - Inspect connected peers via `peers()`.
/// - Tear down via `shutdown()` (consumes the runtime; subsequent
///   `peers()` calls raise `RuntimeError`).
///
/// `identity` is a `str` getter (no method-call cost). `peers()` and
/// `shutdown()` are sync-blocking; `peers()` is lock-light (brief
/// mutex hold around a snapshot copy).
#[pyclass(name = "DomainHandle")]
pub struct DomainHandle {
    identity: String,
    /// `Mutex<Option<...>>` so `shutdown()` can `take()` the inner
    /// runtime (whose Rust `shutdown` consumes `self`) while still
    /// holding the wrapper as `&self`. After `take()`, subsequent
    /// `peers()` / `shutdown()` calls find `None` and raise.
    inner: Mutex<Option<RustClusterRuntime>>,
}

#[pymethods]
impl DomainHandle {
    /// The Domain's canonical identity string — `{wallet_id}/{name}`
    /// for user-named Domains, just `"Vinland"` for the reserved
    /// singleton. Same value Discovery indexes on and that
    /// `runtime.peers()` peer entries belong to.
    #[getter]
    fn identity(&self) -> &str {
        &self.identity
    }

    /// Snapshot of currently-connected peers. Lock-light — copies
    /// entries out from under a brief mutex hold. Safe to call from
    /// any Python thread.
    ///
    /// Returns a list of dicts, each carrying:
    /// - `peer_id: str` — canonical libp2p form (`"12D3KooW..."`).
    /// - `info: dict` — the peer's `ParticipantInfo` (`app`, `name`,
    ///   `session_id`, `session_clock_id`, `session_clock_hash`,
    ///   `session_now_ns`, `cluster_joined_at_ns`, `peer_id`,
    ///   `app_instance`).
    /// - `first_seen_ns: int` — local monotonic timestamp at which
    ///   this peer's `ParticipantInfo` first arrived.
    ///
    /// Same shape as `auki_network.cluster.ClusterRuntime.peers()`'s
    /// dict-rendering so daemons can swap construction paths without
    /// touching the consumer code. Raises `RuntimeError` if the
    /// runtime has been shut down.
    fn peers(&self, py: Python<'_>) -> PyResult<Vec<Py<PyDict>>> {
        let inner = self.inner.lock().expect("DomainHandle mutex poisoned");
        let rt = inner.as_ref().ok_or_else(shutdown_error)?;
        let snapshots = rt.peers();
        let mut out = Vec::with_capacity(snapshots.len());
        for snap in snapshots {
            out.push(peer_snapshot_to_dict(py, &snap)?);
        }
        Ok(out)
    }

    /// Signal the driver task to shut down and abort it. Idempotent
    /// in the sense that a second call raises rather than silently
    /// no-ops — use-after-shutdown is almost always a bug, and a
    /// noisy raise is the right signal.
    ///
    /// Raises `RuntimeError` if already shut down.
    fn shutdown(&self) -> PyResult<()> {
        let mut inner = self.inner.lock().expect("DomainHandle mutex poisoned");
        let rt = inner.take().ok_or_else(shutdown_error)?;
        rt.shutdown();
        Ok(())
    }

    fn __repr__(&self) -> String {
        let inner = self.inner.lock().expect("DomainHandle mutex poisoned");
        match inner.as_ref() {
            Some(rt) => format!(
                "DomainHandle(identity={:?}, connected_peers={})",
                self.identity,
                rt.peers().len()
            ),
            None => format!("DomainHandle(identity={:?}, shut_down=True)", self.identity),
        }
    }
}

fn shutdown_error() -> PyErr {
    PyRuntimeError::new_err("DomainHandle has been shut down")
}

// `Debug` so tests can `expect_err` on a `PyResult<DomainHandle>` (the
// success type needs `Debug`). The Mutex<Option<...>> inner is opaque;
// we just print the identity + a shutdown-state hint.
impl std::fmt::Debug for DomainHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().expect("DomainHandle mutex poisoned");
        f.debug_struct("DomainHandle")
            .field("identity", &self.identity)
            .field("shut_down", &inner.is_none())
            .finish()
    }
}

fn peer_snapshot_to_dict(py: Python<'_>, snap: &RustPeerSnapshot) -> PyResult<Py<PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("peer_id", snap.peer_id.to_string())?;
    d.set_item("first_seen_ns", snap.first_seen_ns)?;
    let info = PyDict::new_bound(py);
    info.set_item("app", &snap.info.app)?;
    info.set_item("name", &snap.info.name)?;
    info.set_item("session_id", &snap.info.session_id)?;
    info.set_item("session_clock_id", &snap.info.session_clock_id)?;
    info.set_item("session_clock_hash", &snap.info.session_clock_hash)?;
    info.set_item("session_now_ns", snap.info.session_now_ns)?;
    info.set_item("cluster_joined_at_ns", snap.info.cluster_joined_at_ns)?;
    info.set_item("peer_id", snap.info.peer_id.to_string())?;
    info.set_item("app_instance", &snap.info.app_instance)?;
    d.set_item("info", info)?;
    Ok(d.unbind())
}

// ─── init_domain ─────────────────────────────────────────────────────

/// Process-wide tokio runtime owned in `OnceLock<Runtime>`. The Rust
/// `init_domain` is async; the wrapper `block_on`s here so the Python
/// caller stays sync-shaped. The runtime persists for the process's
/// lifetime — same model as `auki-network-py`'s `cluster_tokio_runtime`
/// (separate instance because the two crates can't share without a
/// shared bridge crate; the cost is one extra thread pool, which is
/// fine).
fn domain_tokio_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        // Best-effort tracing init so warnings from the participant-
        // provider closure plumbing reach stderr. `try_init` is
        // idempotent: a host process that already installed a
        // subscriber wins.
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .try_init();
        Runtime::new().expect("failed to build tokio runtime for auki_domain")
    })
}

/// Construct a `ParticipantInfoProvider` from a Python callable that
/// returns a Python object whose attributes are duck-typed to a
/// `ParticipantInfo`. Duck typing (rather than typed `extract::<
/// PyParticipantInfo>()`) avoids depending on `auki-network-py`'s
/// `ParticipantInfo` pyclass — which would be a lib-name collision
/// because both crates' `[lib] name` resolve to library targets in
/// the same Cargo build graph. The trade-off: a daemon that returns
/// the wrong shape gets a logged warning and the runtime sees `None`
/// (drops the reply channel), instead of a typed-extract failure.
///
/// Reads attributes: `app`, `name`, `session_id`, `session_clock_id`,
/// `session_clock_hash`, `session_now_ns`, `cluster_joined_at_ns`,
/// `peer_id`, `app_instance`. `cluster_joined_at_ns` may be `None`;
/// every other field is required.
///
/// `peer_id` is read as a string and parsed as a libp2p PeerId; parse
/// failure logs and returns `None`.
///
/// This matches the existing `auki_network.cluster.ParticipantInfo`
/// pyclass field layout — daemons that constructed an
/// `auki_network.cluster.ParticipantInfo(...)` in their pre-v0.0.33
/// `participant_provider` closure can return that same object here
/// without changes.
fn build_participant_provider(callable: Py<PyAny>) -> ParticipantInfoProvider {
    Arc::new(move || -> Option<RustParticipantInfo> {
        Python::with_gil(|py| {
            let result = match callable.call0(py) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "participant_provider raised; dropping reply");
                    return None;
                }
            };
            if result.is_none(py) {
                return None;
            }
            let bound = result.bind(py);
            // Duck-typed read of every field. Any access failure logs
            // + drops; daemons see a missed reply rather than a
            // crash.
            macro_rules! read_str {
                ($attr:literal) => {
                    match bound.getattr($attr).and_then(|a| a.extract::<String>()) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(
                                attr = $attr,
                                error = %e,
                                "participant_provider returned object missing/wrong-typed field; dropping reply"
                            );
                            return None;
                        }
                    }
                };
            }
            let app = read_str!("app");
            let name = read_str!("name");
            let session_id = read_str!("session_id");
            let session_clock_id = read_str!("session_clock_id");
            let session_clock_hash = read_str!("session_clock_hash");
            let session_now_ns = match bound.getattr("session_now_ns").and_then(|a| a.extract::<u64>()) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(error = %e, "participant_provider session_now_ns wrong-typed; dropping reply");
                    return None;
                }
            };
            let cluster_joined_at_ns = match bound.getattr("cluster_joined_at_ns").and_then(|a| a.extract::<Option<u64>>()) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(error = %e, "participant_provider cluster_joined_at_ns wrong-typed; dropping reply");
                    return None;
                }
            };
            let peer_id_str = read_str!("peer_id");
            let app_instance = read_str!("app_instance");
            let peer_id = match PeerId::from_str(&peer_id_str) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(peer_id = %peer_id_str, error = %e, "participant_provider peer_id unparseable; dropping reply");
                    return None;
                }
            };
            Some(RustParticipantInfo {
                app,
                name,
                session_id,
                session_clock_id,
                session_clock_hash,
                session_now_ns,
                cluster_joined_at_ns,
                peer_id,
                app_instance,
            })
        })
    })
}

/// Create a new Domain (or fall back to joining on a 409) and return
/// a [`DomainHandle`] that owns the live cluster runtime.
///
/// This is the post-v0.0.33 entry point for Python daemons —
/// `auki_network.cluster.spawn` was removed in the cluster-trust-
/// boundary PR B; this function is its replacement. Discovery is
/// mandatory; there is no static-`cluster.json` fallback and no
/// Discovery-less dev mode.
///
/// # Arguments
///
/// - `wallet_seed` (`bytes`, exactly 32 bytes): the **parent** wallet
///   seed. Drives the Domain's `wallet_id` for user-named Domains and
///   signs the create/register payloads at Discovery.
/// - `peer_seed` (`bytes`, exactly 32 bytes): the **peer** seed —
///   `Wallet::from_seed(wallet_seed).derive_child("peer/v1").seed()`
///   in the daemon's existing identity pipeline. Drives the libp2p
///   swarm's `PeerId`.
/// - `discovery_url` (`str`): URL of the Discovery service the daemon
///   talks to. No env-var fallback at this layer — the daemon's CLI
///   resolves precedence and passes the final string here.
/// - `domain_name` (`str`): the Domain to claim. Pass `"Vinland"` for
///   the reserved Greenland T12 singleton (no `{wallet_id}/` prefix);
///   any other string is wrapped as `{wallet_id}/{name}`.
/// - `addresses` (`list[str]`): the multiaddrs to advertise to
///   Discovery as this peer's dialable addresses (e.g.
///   `["/ip4/192.168.9.72/tcp/4001"]`). Required; the SDK does not
///   infer them from the swarm's bound listeners (`0.0.0.0` isn't
///   dialable; NAT / Docker break listeners-as-source-of-truth).
/// - `participant_provider` (`callable`): zero-arg callable invoked
///   on every inbound `/auki/cluster/0.0.1` request. Returns a Python
///   object whose attributes match the `ParticipantInfo` shape (see
///   [`build_participant_provider`]) or `None` to drop the reply.
/// - `listen_addresses` (`list[str] | None`): swarm listen multiaddrs.
///   Defaults to `["/ip4/0.0.0.0/tcp/0", "/ip4/0.0.0.0/udp/0/quic-v1"]`
///   (OS-chosen ports, dual-stack TCP + QUIC). Pass specific ports to
///   pin them so a daemon restart keeps the same bound multiaddrs.
/// - `agent_version` (`str | None`): swarm `IdentifyInfo.agent_version`.
///   Defaults to `auki-domain-py/{version}`.
/// - `expected_app_id` (`str | None`): operator metadata passed to
///   Discovery's `register` and recorded on the `ClusterPeer` entry.
///   Optional.
/// - `note` (`str | None`): operator-friendly free-form metadata
///   passed to Discovery's `register`. Optional.
///
/// # Returns
///
/// A [`DomainHandle`] owning the live cluster runtime. `handle.identity`
/// is the canonical Domain string; `handle.peers()` + `handle.shutdown()`
/// drive the runtime.
///
/// # Exceptions
///
/// - `ValueError` — seed length wrong, multiaddr unparseable, or
///   `domain_name` empty.
/// - `auki_domain.DomainAlreadyExists` — Discovery returned 409 on the
///   atomic `POST /clusters/{name}`. Carries `.identity` and
///   `.cluster_name` strings.
/// - `auki_domain.DiscoveryUnreachable` — transport failure (DNS,
///   connect-refused, TLS).
/// - `auki_domain.DiscoveryRejected` — non-2xx HTTP from Discovery
///   that isn't the typed-409 path. Carries `.status: int` and
///   `.body: str`.
/// - `auki_domain.DiscoveryClockError` — clock-skew rejection.
/// - `auki_domain.RuntimeSpawnError` — Discovery accepted but local
///   runtime construction failed afterwards.
#[pyfunction]
#[pyo3(
    name = "init_domain",
    text_signature = "(wallet_seed, peer_seed, discovery_url, domain_name, addresses, participant_provider, *, listen_addresses=None, agent_version=None, expected_app_id=None, note=None)",
    signature = (
        wallet_seed,
        peer_seed,
        discovery_url,
        domain_name,
        addresses,
        participant_provider,
        *,
        listen_addresses = None,
        agent_version = None,
        expected_app_id = None,
        note = None,
    ),
)]
#[allow(clippy::too_many_arguments)]
fn init_domain_py(
    py: Python<'_>,
    wallet_seed: &Bound<'_, PyBytes>,
    peer_seed: &Bound<'_, PyBytes>,
    discovery_url: &str,
    domain_name: &str,
    addresses: Vec<String>,
    participant_provider: Py<PyAny>,
    listen_addresses: Option<Vec<String>>,
    agent_version: Option<String>,
    expected_app_id: Option<String>,
    note: Option<String>,
) -> PyResult<DomainHandle> {
    // 1. Validate seed lengths up-front. `Wallet::from_seed` /
    //    `PeerIdentity::from_seed` both accept `[u8; 32]`; an out-of-
    //    range slice would panic on `copy_from_slice`.
    let wallet_seed_bytes = seed_array(wallet_seed, "wallet_seed")?;
    let peer_seed_bytes = seed_array(peer_seed, "peer_seed")?;

    // 2. Reject obvious garbage early.
    if domain_name.is_empty() {
        return Err(PyValueError::new_err("domain_name must not be empty"));
    }
    if addresses.is_empty() {
        return Err(PyValueError::new_err(
            "addresses must contain at least one multiaddr — Discovery \
             needs a dialable address to advertise",
        ));
    }

    // 3. Parse multiaddrs. Two failure surfaces (`addresses` for the
    //    advertised set; `listen_addresses` for the bind set) — both
    //    surface as `ValueError` with the offending string in the
    //    message.
    let advertised: Vec<Multiaddr> = addresses
        .iter()
        .map(|s| {
            s.parse::<Multiaddr>().map_err(|e| {
                PyValueError::new_err(format!("invalid address {s:?}: {e}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let listen: Vec<Multiaddr> = match listen_addresses {
        None => vec![
            "/ip4/0.0.0.0/tcp/0".parse().expect("hardcoded valid"),
            "/ip4/0.0.0.0/udp/0/quic-v1"
                .parse()
                .expect("hardcoded valid"),
        ],
        Some(list) => list
            .iter()
            .map(|s| {
                s.parse::<Multiaddr>().map_err(|e| {
                    PyValueError::new_err(format!("invalid listen multiaddr {s:?}: {e}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };

    // 4. Build the wallet and the swarm. Wallet drives the Domain
    //    identity + Discovery signing; PeerIdentity drives the
    //    swarm's PeerId.
    let wallet = Wallet::from_seed(&wallet_seed_bytes);
    let peer_identity = PeerIdentity::from_seed(&peer_seed_bytes);
    let agent_version = agent_version
        .unwrap_or_else(|| format!("auki-domain-py/{}", env!("CARGO_PKG_VERSION")));
    let swarm_config = SwarmConfig {
        listen_addresses: listen,
        agent_version,
        // Relay-server is `aukilabs/relay`'s job; wrapper consumers
        // never want it.
        enable_relay_server: false,
    };
    let swarm = build_swarm(&peer_identity, swarm_config).map_err(|e| {
        PyRuntimeError::new_err(format!("swarm build failed: {e}"))
    })?;

    // 5. Build the participant_provider closure (duck-typed Python
    //    callable; see `build_participant_provider`). `stream_provider`
    //    intentionally not wired in this PR — see crate-level docs.
    let provider = build_participant_provider(participant_provider);
    let stream_provider = decline_all_streams();

    // 6. Build the Discovery client. Sync constructor — no I/O until
    //    `init_domain` calls into it. `DiscoveryClient::new` is
    //    infallible at construction (URL parsing happens at request
    //    time); a malformed URL surfaces as a transport error on the
    //    first call.
    let discovery = DiscoveryClient::new(discovery_url);

    // 7. Drive the async `init_domain` on the process-wide tokio
    //    runtime, releasing the GIL so the Python participant_provider
    //    callback can reacquire it when invoked from a runtime worker
    //    thread. `block_on` returns when init_domain resolves.
    let wallet_for_call = wallet;
    let result: Result<RustDomainHandle, InitDomainError> = py.allow_threads(|| {
        let rt = domain_tokio_runtime();
        let _guard = rt.enter();
        rt.block_on(async {
            rust_init_domain(
                &wallet_for_call,
                domain_name,
                &discovery,
                swarm,
                &advertised,
                expected_app_id.as_deref(),
                note.as_deref(),
                provider,
                stream_provider,
            )
            .await
        })
    });

    let handle = result.map_err(map_init_domain_error)?;
    let identity = handle.identity.canonical_string();
    Ok(DomainHandle {
        identity,
        inner: Mutex::new(Some(handle.runtime)),
    })
}

fn seed_array(seed: &Bound<'_, PyBytes>, name: &str) -> PyResult<[u8; 32]> {
    let bytes = seed.as_bytes();
    if bytes.len() != 32 {
        return Err(PyValueError::new_err(format!(
            "{name} must be exactly 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    Ok(arr)
}

// ─── Module entry point ──────────────────────────────────────────────

/// Populate the `auki_domain` module. Exposed as a free function so
/// tests can drive it directly; the `#[pymodule]` entry point below
/// is a thin wrapper.
fn populate_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();

    m.add_class::<DomainHandle>()?;
    m.add_function(wrap_pyfunction!(init_domain_py, m)?)?;

    m.add(
        "DomainAlreadyExists",
        py.get_type_bound::<DomainAlreadyExists>(),
    )?;
    m.add(
        "DiscoveryUnreachable",
        py.get_type_bound::<DiscoveryUnreachable>(),
    )?;
    m.add(
        "DiscoveryRejected",
        py.get_type_bound::<DiscoveryRejected>(),
    )?;
    m.add(
        "DiscoveryClockError",
        py.get_type_bound::<DiscoveryClockError>(),
    )?;
    m.add(
        "RuntimeSpawnError",
        py.get_type_bound::<RuntimeSpawnError>(),
    )?;
    Ok(())
}

/// `auki_domain` module. The `#[pymodule]` macro generates the
/// `PyInit_auki_domain` C entry point Python imports.
#[pymodule]
fn auki_domain(m: &Bound<'_, PyModule>) -> PyResult<()> {
    populate_module(m)
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper for tests that need a populated module.
    fn populated_module(py: Python<'_>) -> Bound<'_, PyModule> {
        let m = PyModule::new_bound(py, "auki_domain").expect("module");
        populate_module(&m).expect("populate");
        m
    }

    #[test]
    fn module_exposes_init_domain_function_and_exception_classes() {
        Python::with_gil(|py| {
            let m = populated_module(py);
            assert!(m.getattr("init_domain").is_ok(), "init_domain function");
            assert!(m.getattr("DomainHandle").is_ok(), "DomainHandle class");
            for exc in &[
                "DomainAlreadyExists",
                "DiscoveryUnreachable",
                "DiscoveryRejected",
                "DiscoveryClockError",
                "RuntimeSpawnError",
            ] {
                assert!(
                    m.getattr(*exc).is_ok(),
                    "expected exception class {exc} on the module"
                );
            }
        });
    }

    #[test]
    fn seed_array_rejects_wrong_length() {
        Python::with_gil(|py| {
            let too_short = PyBytes::new_bound(py, &[0u8; 16]);
            let err =
                seed_array(&too_short, "wallet_seed").expect_err("16 bytes must reject");
            let s = err.to_string();
            assert!(s.contains("wallet_seed must be exactly 32 bytes"), "{s}");
            assert!(s.contains("got 16"), "{s}");

            let too_long = PyBytes::new_bound(py, &[0u8; 33]);
            let err = seed_array(&too_long, "peer_seed").expect_err("33 bytes must reject");
            let s = err.to_string();
            assert!(s.contains("peer_seed must be exactly 32 bytes"), "{s}");
            assert!(s.contains("got 33"), "{s}");
        });
    }

    #[test]
    fn seed_array_accepts_32_bytes() {
        Python::with_gil(|py| {
            let exact = PyBytes::new_bound(py, &[7u8; 32]);
            let arr = seed_array(&exact, "wallet_seed").expect("32-byte seed accepts");
            assert_eq!(arr, [7u8; 32]);
        });
    }

    /// `init_domain` rejects an empty `domain_name` synchronously
    /// (before any Discovery call). The check lives before tokio
    /// block_on so we can exercise it without a live Discovery.
    #[test]
    fn init_domain_rejects_empty_domain_name() {
        Python::with_gil(|py| {
            let wallet = PyBytes::new_bound(py, &[1u8; 32]);
            let peer = PyBytes::new_bound(py, &[2u8; 32]);
            let provider = py.None();
            let err = init_domain_py(
                py,
                &wallet,
                &peer,
                "http://127.0.0.1:8080",
                "",
                vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
                provider,
                None,
                None,
                None,
                None,
            )
            .expect_err("empty domain_name must reject");
            assert!(err.to_string().contains("domain_name must not be empty"));
        });
    }

    /// `init_domain` rejects an empty `addresses` list synchronously
    /// (Discovery needs at least one dialable multiaddr to advertise).
    #[test]
    fn init_domain_rejects_empty_addresses() {
        Python::with_gil(|py| {
            let wallet = PyBytes::new_bound(py, &[1u8; 32]);
            let peer = PyBytes::new_bound(py, &[2u8; 32]);
            let provider = py.None();
            let err = init_domain_py(
                py,
                &wallet,
                &peer,
                "http://127.0.0.1:8080",
                "Vinland",
                vec![],
                provider,
                None,
                None,
                None,
                None,
            )
            .expect_err("empty addresses must reject");
            assert!(err.to_string().contains("addresses must contain at least one"));
        });
    }

    /// Seed-length validation catches a wallet_seed before any
    /// further work runs.
    #[test]
    fn init_domain_rejects_short_wallet_seed() {
        Python::with_gil(|py| {
            let short = PyBytes::new_bound(py, &[1u8; 16]);
            let peer = PyBytes::new_bound(py, &[2u8; 32]);
            let provider = py.None();
            let err = init_domain_py(
                py,
                &short,
                &peer,
                "http://127.0.0.1:8080",
                "Vinland",
                vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
                provider,
                None,
                None,
                None,
                None,
            )
            .expect_err("16-byte wallet_seed must reject");
            assert!(err.to_string().contains("wallet_seed must be exactly 32 bytes"));
        });
    }

    #[test]
    fn init_domain_rejects_short_peer_seed() {
        Python::with_gil(|py| {
            let wallet = PyBytes::new_bound(py, &[1u8; 32]);
            let short = PyBytes::new_bound(py, &[2u8; 16]);
            let provider = py.None();
            let err = init_domain_py(
                py,
                &wallet,
                &short,
                "http://127.0.0.1:8080",
                "Vinland",
                vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
                provider,
                None,
                None,
                None,
                None,
            )
            .expect_err("16-byte peer_seed must reject");
            assert!(err.to_string().contains("peer_seed must be exactly 32 bytes"));
        });
    }

    /// Bad multiaddr is caught at parse, before any network call.
    #[test]
    fn init_domain_rejects_unparseable_address() {
        Python::with_gil(|py| {
            let wallet = PyBytes::new_bound(py, &[1u8; 32]);
            let peer = PyBytes::new_bound(py, &[2u8; 32]);
            let provider = py.None();
            let err = init_domain_py(
                py,
                &wallet,
                &peer,
                "http://127.0.0.1:8080",
                "Vinland",
                vec!["not-a-multiaddr".to_string()],
                provider,
                None,
                None,
                None,
                None,
            )
            .expect_err("garbage multiaddr must reject");
            assert!(err.to_string().contains("invalid address"));
        });
    }

    /// `build_participant_provider` duck-types attributes off an
    /// arbitrary Python object. We don't need a real
    /// `auki_network.cluster.ParticipantInfo` pyclass; any object
    /// with the right attributes (the daemon's existing ParticipantInfo
    /// has `#[getter]`s, so it works through this path) is accepted.
    /// An object missing an attribute returns `None` (drops the reply).
    #[test]
    fn participant_provider_drops_when_attribute_missing() {
        Python::with_gil(|py| {
            // Empty class — no attributes. The provider should log and
            // return None.
            let module = PyModule::from_code_bound(
                py,
                "class Bad:\n    pass\n",
                "test_bad.py",
                "test_bad",
            )
            .expect("module");
            let cls = module.getattr("Bad").unwrap();
            let bad_instance = cls.call0().unwrap().unbind();
            let callable = PyModule::from_code_bound(
                py,
                "def make_provider(obj):\n    return lambda: obj\n",
                "factory.py",
                "factory",
            )
            .expect("factory module")
            .getattr("make_provider")
            .unwrap()
            .call1((bad_instance,))
            .unwrap()
            .unbind();

            let provider = build_participant_provider(callable);
            assert!(provider().is_none(), "missing attribute -> drop reply");
        });
    }

    /// A correctly-shaped duck-typed object round-trips into a
    /// `RustParticipantInfo`. The shape matches the
    /// `auki_network.cluster.ParticipantInfo` pyclass field layout so
    /// existing daemons return that pyclass instance through this seam
    /// without code changes.
    #[test]
    fn participant_provider_extracts_well_shaped_object() {
        Python::with_gil(|py| {
            let module = PyModule::from_code_bound(
                py,
                r#"
class Info:
    def __init__(self, peer_id):
        self.app = "boosterapp"
        self.name = "k1-walker"
        self.session_id = "test-session"
        self.session_clock_id = "K1/clock"
        self.session_clock_hash = "deadbeef"
        self.session_now_ns = 1234
        self.cluster_joined_at_ns = None
        self.peer_id = peer_id
        self.app_instance = "aabbccddeeff"

def make():
    pid = "12D3KooWJrqdtuEqJYK5SswzL7XSCwMmpYWLcoKkWnWtPyJpcLDz"
    info = Info(pid)
    return lambda: info
"#,
                "info.py",
                "info",
            )
            .expect("module");
            let callable = module.getattr("make").unwrap().call0().unwrap().unbind();
            let provider = build_participant_provider(callable);
            let info = provider().expect("well-shaped object extracts");
            assert_eq!(info.app, "boosterapp");
            assert_eq!(info.name, "k1-walker");
            assert_eq!(info.session_id, "test-session");
            assert_eq!(info.session_clock_id, "K1/clock");
            assert_eq!(info.session_clock_hash, "deadbeef");
            assert_eq!(info.session_now_ns, 1234);
            assert!(info.cluster_joined_at_ns.is_none());
            assert_eq!(info.app_instance, "aabbccddeeff");
        });
    }

    /// Provider returning `None` is the documented "drop this reply"
    /// signal — covered for clarity.
    #[test]
    fn participant_provider_passes_through_none_return() {
        Python::with_gil(|py| {
            let module = PyModule::from_code_bound(
                py,
                "def make():\n    return lambda: None\n",
                "none.py",
                "none",
            )
            .expect("module");
            let callable = module.getattr("make").unwrap().call0().unwrap().unbind();
            let provider = build_participant_provider(callable);
            assert!(provider().is_none());
        });
    }
}

//! PyO3 bindings for `auki-network`'s cluster layer.
//!
//! Lets a Python process participate in an ansuz cluster as a libp2p
//! peer:
//!
//! - [`cluster.load_doc`](load_doc) — read and parse a `cluster.json`,
//!   return an opaque [`ClusterDoc`] handle.
//! - [`cluster.ParticipantInfo`](ParticipantInfo) — typed wire-shape
//!   class the consumer constructs and returns from the
//!   `participant_provider` callable.
//! - [`cluster.PeerSnapshot`](PeerSnapshot) — typed read-only view of
//!   one connected peer; produced by `runtime.peers()`.
//! - `cluster.spawn(seed, doc, participant_provider, **kwargs)` — boot
//!   a process-wide tokio runtime (lazily, in a `std::thread`) and
//!   drive a [`auki_network::cluster_runtime::ClusterRuntime`] against
//!   `doc`. Returns an opaque [`ClusterRuntime`] handle.
//! - `runtime.peers()` / `runtime.shutdown()` — see [`ClusterRuntime`].
//!
//! The `participant_provider` callable runs **on the cluster runtime's
//! single tokio worker** (one task polls the swarm and invokes the
//! provider). Keep it cheap: build the [`ParticipantInfo`] from cached
//! state, no I/O, no contended locks. Sustained GIL contention on this
//! callable measurably impacts the cluster's responsiveness.
//!
//! See [`crates/auki-network-py/README.md`](../README.md) for the
//! Python-side surface and install instructions.

// Path 1 (locked) for grimsby's `stream_provider` Python binding —
// bridges Python `async def` generators onto Rust `futures::Stream`.
// Public surface stays inside this crate; consumers call through
// `cluster.spawn(..., stream_provider=...)` and `runtime.open_stream`.
mod stream_bridge;
mod stream_types;

// Vinland Batch 2 — the `auki_network.discovery` Python submodule
// wrapping `auki_network::discovery_client::DiscoveryClient`.
// Sync-shaped per Pattern A; each method `block_on`s on the shared
// `cluster_tokio_runtime()`.
mod discovery;

use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};

// `auki_network_rs` is the upstream Rust crate, renamed via `package =`
// in Cargo.toml so it doesn't collide with this crate's own lib name
// (`auki_network` — also the Python module name).
use auki_network_rs::{
    ParticipantInfo as RustParticipantInfo,
    cluster_doc::{self, ClusterDoc as RustClusterDoc, LoadError},
    cluster_name::{self as rust_cluster_name, ClusterNameError},
    cluster_runtime::{
        ClusterRuntime as RustClusterRuntime, ParticipantInfoProvider, SpawnError,
    },
    stream_protocol::StreamRequest as RustStreamRequest,
    stream_runtime::decline_all_streams,
    swarm::SwarmConfig,
};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::runtime::Runtime;

use crate::stream_types::{
    PyStreamSubscription, build_stream_provider, open_stream_error_to_pyerr,
};

// ─── ParticipantInfo ─────────────────────────────────────────────────────────

/// Identity card a participant exchanges over `/auki/cluster/1.0.0` and
/// serves on `GET /api/info`. One schema, two transports — see
/// `auki_network::participant::ParticipantInfo` for the field
/// semantics.
///
/// Construct via the `#[new]`-backed constructor; the consumer's
/// `participant_provider` callable returns one of these on each
/// inbound cluster request, so the runtime can reply with fresh
/// `session_now_ns` per call rather than stale at spawn time.
#[pyclass(name = "ParticipantInfo")]
#[derive(Clone, Debug)]
pub struct ParticipantInfo {
    inner: RustParticipantInfo,
}

#[pymethods]
impl ParticipantInfo {
    /// Construct a `ParticipantInfo`. Every field is required;
    /// `cluster_joined_at_ns` may be `None` (the participant hasn't
    /// connected to anyone yet).
    ///
    /// Raises `ValueError` if `peer_id` does not parse as a libp2p
    /// PeerId (canonical form: `"12D3KooW..."`).
    #[new]
    #[pyo3(signature = (
        *,
        app,
        name,
        session_id,
        session_clock_id,
        session_clock_hash,
        session_now_ns,
        cluster_joined_at_ns,
        peer_id,
        app_instance,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        app: String,
        name: String,
        session_id: String,
        session_clock_id: String,
        session_clock_hash: String,
        session_now_ns: u64,
        cluster_joined_at_ns: Option<u64>,
        peer_id: &str,
        app_instance: String,
    ) -> PyResult<Self> {
        let peer_id = PeerId::from_str(peer_id)
            .map_err(|e| PyValueError::new_err(format!("invalid peer_id {peer_id:?}: {e}")))?;
        Ok(Self {
            inner: RustParticipantInfo {
                app,
                name,
                session_id,
                session_clock_id,
                session_clock_hash,
                session_now_ns,
                cluster_joined_at_ns,
                peer_id,
                app_instance,
            },
        })
    }

    #[getter]
    fn app(&self) -> &str {
        &self.inner.app
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    #[getter]
    fn session_id(&self) -> &str {
        &self.inner.session_id
    }

    #[getter]
    fn session_clock_id(&self) -> &str {
        &self.inner.session_clock_id
    }

    #[getter]
    fn session_clock_hash(&self) -> &str {
        &self.inner.session_clock_hash
    }

    #[getter]
    fn session_now_ns(&self) -> u64 {
        self.inner.session_now_ns
    }

    #[getter]
    fn cluster_joined_at_ns(&self) -> Option<u64> {
        self.inner.cluster_joined_at_ns
    }

    /// Canonical libp2p PeerId string (`"12D3KooW..."`).
    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id.to_string()
    }

    #[getter]
    fn app_instance(&self) -> &str {
        &self.inner.app_instance
    }

    fn __repr__(&self) -> String {
        format!(
            "ParticipantInfo(app={:?}, name={:?}, session_id={:?}, session_now_ns={}, peer_id={:?}, app_instance={:?})",
            self.inner.app,
            self.inner.name,
            self.inner.session_id,
            self.inner.session_now_ns,
            self.inner.peer_id.to_string(),
            self.inner.app_instance,
        )
    }

    /// Field-wise equality on every field of the wire shape. Note that
    /// `session_now_ns` is by design refreshed on each provider call,
    /// so two `ParticipantInfo` objects taken from the same peer at
    /// different moments will compare unequal — this is correct for
    /// "are these the exact same payload?" but `__hash__` is left
    /// undefined to avoid the implication that instances are stable
    /// dict keys.
    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl ParticipantInfo {
    /// Internal: clone out the Rust `ParticipantInfo` for handing to
    /// the auki-network layer. Used by the `cluster.spawn` provider
    /// plumbing once it lands (Phase 2 of the wrapper rollout).
    #[allow(dead_code)]
    pub(crate) fn to_rust(&self) -> RustParticipantInfo {
        self.inner.clone()
    }

    #[allow(dead_code)]
    pub(crate) fn from_rust(inner: RustParticipantInfo) -> Self {
        Self { inner }
    }
}

// ─── PeerSnapshot ────────────────────────────────────────────────────────────

/// Read-only view of one connected peer, returned by `runtime.peers()`.
///
/// The runtime owns the live state internally; instances of this class
/// are copies taken at snapshot time. Constructed only by the runtime
/// (no Python `__new__`) — Python code reads, doesn't build.
#[pyclass(name = "PeerSnapshot")]
#[derive(Clone, Debug)]
pub struct PeerSnapshot {
    peer_id: String,
    info: ParticipantInfo,
    first_seen_ns: u64,
}

#[pymethods]
impl PeerSnapshot {
    /// Canonical libp2p PeerId string of this peer.
    #[getter]
    fn peer_id(&self) -> &str {
        &self.peer_id
    }

    /// Most recent `ParticipantInfo` received from this peer. Refreshed
    /// on every response.
    #[getter]
    fn info(&self) -> ParticipantInfo {
        self.info.clone()
    }

    /// Peer's `session_now_ns` value at the moment of the **first**
    /// response received from this peer's current session. Sticky
    /// across reconnects within the same peer-session; reset if the
    /// peer's `session_id` changes (peer restarted with a fresh
    /// session).
    #[getter]
    fn first_seen_ns(&self) -> u64 {
        self.first_seen_ns
    }

    fn __repr__(&self) -> String {
        format!(
            "PeerSnapshot(peer_id={:?}, app={:?}, name={:?}, first_seen_ns={})",
            self.peer_id, self.info.inner.app, self.info.inner.name, self.first_seen_ns,
        )
    }
}

impl PeerSnapshot {
    /// Internal: build a `PeerSnapshot` from the auki-network type.
    /// Used by `runtime.peers()` once the cluster.spawn plumbing lands.
    #[allow(dead_code)]
    pub(crate) fn from_rust(snapshot: auki_network_rs::cluster_runtime::PeerSnapshot) -> Self {
        Self {
            peer_id: snapshot.peer_id.to_string(),
            info: ParticipantInfo::from_rust(snapshot.info),
            first_seen_ns: snapshot.first_seen_ns,
        }
    }
}

// ─── ClusterDoc + load_doc ───────────────────────────────────────────────────

/// Opaque handle around a parsed `cluster.json`. Construct via
/// `cluster.load_doc(path)`; consume by passing to `cluster.spawn`.
///
/// Python sees this as an opaque token — no field access. The Rust
/// side validates everything at parse time (peer-id strings, multiaddr
/// strings, schema version), so a `ClusterDoc` instance is by
/// construction valid.
#[pyclass(name = "ClusterDoc")]
#[derive(Clone, Debug)]
pub struct ClusterDoc {
    pub(crate) inner: RustClusterDoc,
}

#[pymethods]
impl ClusterDoc {
    /// Number of peers pinned in the doc. Useful for sanity-checking a
    /// hand-edited file from Python without exposing the full peer
    /// list.
    #[getter]
    fn peer_count(&self) -> usize {
        self.inner.peers.len()
    }

    /// Cluster name from the doc's `cluster_name` field. Operator
    /// label only — no semantic role.
    #[getter]
    fn cluster_name(&self) -> &str {
        &self.inner.cluster_name
    }

    fn __repr__(&self) -> String {
        format!(
            "ClusterDoc(cluster_name={:?}, peer_count={})",
            self.inner.cluster_name,
            self.inner.peers.len(),
        )
    }
}

/// Read and parse a `cluster.json` file from `path`. Returns a
/// `ClusterDoc` handle that can be passed to `cluster.spawn`.
///
/// Raises:
/// - `OSError` on filesystem errors (file not found, permission denied,
///   etc.).
/// - `ValueError` on any structural problem with the file's contents:
///   JSON syntax error, missing required field, unsupported schema
///   version, invalid `peer_id` string, or invalid multiaddr string.
///   The error message names the offending value where applicable so
///   the operator can fix the doc without a debugger.
#[pyfunction]
#[pyo3(name = "load_doc", text_signature = "(path, /)")]
fn load_doc(path: PathBuf) -> PyResult<ClusterDoc> {
    let inner = cluster_doc::load(&path).map_err(map_load_error)?;
    Ok(ClusterDoc { inner })
}

/// Resolve `cluster_name` from a CLI flag (passed in as `flag`) or the
/// `AUKI_CLUSTER_NAME` env var, in that order. Strict semantics per
/// sawslin Decision #1: if neither is set, raises `ValueError`.
///
/// Pass `flag=None` (the default) when the CLI parser saw no
/// `--cluster-name`. Empty-string env values are treated as unset.
///
/// Daemons with a back-compat reason to default to a specific cluster
/// (notably boosterapp's "vinland" carve-out) implement their own
/// resolver instead of calling this — strict is the default for
/// everything else.
#[pyfunction]
#[pyo3(name = "resolve_cluster_name", signature = (flag = None))]
fn resolve_cluster_name(flag: Option<&str>) -> PyResult<String> {
    rust_cluster_name::resolve(flag).map_err(|e| match e {
        ClusterNameError::Unset => PyValueError::new_err(e.to_string()),
    })
}

fn map_load_error(e: LoadError) -> PyErr {
    match e {
        LoadError::Io(io_err) => PyOSError::new_err(format!("cluster.json i/o: {io_err}")),
        LoadError::Parse(parse_err) => {
            PyValueError::new_err(format!("cluster.json parse: {parse_err}"))
        }
        LoadError::UnsupportedVersion(v) => PyValueError::new_err(format!(
            "cluster.json: unsupported version {v} (this loader speaks {})",
            cluster_doc::SUPPORTED_VERSION,
        )),
        LoadError::InvalidPeerId(s) => {
            PyValueError::new_err(format!("cluster.json: invalid peer_id {s:?}"))
        }
        LoadError::InvalidMultiaddr(s) => {
            PyValueError::new_err(format!("cluster.json: invalid multiaddr {s:?}"))
        }
    }
}

// ─── Tokio runtime singleton ─────────────────────────────────────────────────

/// Process-wide tokio multi-thread runtime. Created lazily on the first
/// `cluster.spawn`, lives for the rest of the process. Multi-thread is
/// the simplest path that makes `tokio::runtime::Handle::try_current()`
/// succeed inside `ClusterRuntime::spawn`; a single `auki-network-py`
/// process typically holds at most one or two `ClusterRuntime`s, so the
/// runtime is heavily under-utilized — that's fine.
///
/// `pub(crate)` so [`crate::stream_types`] (the source-Stream Drop guard
/// and the consumer-side blocking iterator) can spawn / `block_on`
/// against the same runtime instead of building a sibling.
pub(crate) fn cluster_tokio_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        // Best-effort tracing init so `tracing::warn!` from the
        // participant-provider plumbing reaches stderr (and via systemd
        // → journald). `try_init` is idempotent: a host process that
        // already installed a subscriber wins. Default filter is `warn`
        // so a healthy cluster runs quiet; raise via `RUST_LOG=info` /
        // `RUST_LOG=auki_network=debug` when debugging.
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .try_init();
        Runtime::new().expect("create tokio runtime")
    })
}

// ─── ClusterRuntime ──────────────────────────────────────────────────────────

/// Opaque handle to a running cluster runtime — drives a libp2p swarm
/// against a [`ClusterDoc`], auto-dialing peers, exchanging
/// [`ParticipantInfo`] over `/auki/cluster/1.0.0`, and maintaining a
/// live peer state map.
///
/// Construct via [`spawn`]; consume via [`shutdown`][ClusterRuntime::shutdown].
/// After `shutdown()`, all methods raise `RuntimeError`.
#[pyclass(name = "ClusterRuntime")]
pub struct ClusterRuntime {
    /// `Mutex<Option<...>>` so `shutdown()` can `take()` the inner
    /// runtime (whose Rust `shutdown` consumes `self`) while still
    /// holding the wrapper as `&self` — Python instances live on the
    /// heap and don't have an "owned" form. After `take()`, subsequent
    /// `peers()` / `shutdown()` calls find `None` and raise.
    inner: Mutex<Option<RustClusterRuntime>>,
}

#[pymethods]
impl ClusterRuntime {
    /// Snapshot of currently-connected peers. Lock-light — copies entries
    /// out from under a brief mutex hold. Safe to call from any Python
    /// thread, including the HTTP request handler thread (no async, no
    /// tokio runtime context required on the caller's side).
    ///
    /// Raises `RuntimeError` if the runtime has been shut down.
    fn peers(&self) -> PyResult<Vec<PeerSnapshot>> {
        let inner = self.inner.lock().expect("ClusterRuntime mutex poisoned");
        let rt = inner.as_ref().ok_or_else(shutdown_error)?;
        Ok(rt.peers().into_iter().map(PeerSnapshot::from_rust).collect())
    }

    /// Open an outbound `/auki/stream/1.0.0` subscription on `peer_id`
    /// for the named sensor with `T = JpegFrame` (grimsby deliverable
    /// #4). The producer must accept with [`PyStreamDecision.accept(...)`];
    /// for `T = PointCloudFrame` (Dagaz Batch 2) call
    /// [`open_pointcloud_stream`][ClusterRuntime::open_pointcloud_stream]
    /// instead.
    ///
    /// **Synchronous-blocking**: returns once the producer has Accepted
    /// or Declined the request, or the SDK's 30s timeout fires. The
    /// caller's thread is blocked while the wrapper's tokio runtime
    /// drives the libp2p substream open + wire-handshake. Pattern A
    /// (per the BoosterApp Claude flag): asyncio plumbing stays
    /// internal to the SDK; the caller stays sync-shaped.
    ///
    /// Returns a [`StreamSubscription`]; iterate frames via
    /// `subscription.frames()` (also sync-blocking — see
    /// [`StreamSubscription`] for end-of-stream signalling). Each frame's
    /// `payload` is a [`JpegFrame`].
    ///
    /// Raises:
    /// - `ValueError` if `peer_id` does not parse as a libp2p PeerId.
    /// - `auki_network.cluster.StreamDeclined(reason)` — producer accepted
    ///   the substream open but declined the request.
    ///   `args[0]` is a `DeclineReason`.
    /// - `auki_network.cluster.StreamUnreachable(detail)` — libp2p
    ///   couldn't open the substream (peer not reachable, peer doesn't
    ///   speak `/auki/stream/1.0.0`, or the open timed out).
    /// - `auki_network.cluster.StreamProtocolError(detail)` — peer
    ///   wrote malformed bytes during the request/reply exchange.
    /// - `RuntimeError` if the runtime has been shut down.
    #[pyo3(text_signature = "($self, peer_id, sensor_id)")]
    fn open_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: String,
    ) -> PyResult<PyStreamSubscription> {
        let peer = PeerId::from_str(peer_id).map_err(|e| {
            PyValueError::new_err(format!("invalid peer_id {peer_id:?}: {e}"))
        })?;

        // Hold a clone of the inner runtime across the await — the
        // mutex guard cannot cross `.await` (not Send for Mutex), and
        // the underlying RustClusterRuntime is Send + Sync, so a quick
        // copy-out works. The runtime is dropped via `shutdown()`
        // which `take()`s the inner; we only need it alive long enough
        // to drive the open_stream call.
        //
        // We can't `Clone` RustClusterRuntime — it isn't Clone — so
        // call `open_stream` on a borrow that lives for the duration of
        // the block_on call. That means holding the guard the whole
        // time. Easiest: do the work in a closure that borrows the
        // guard, releasing it after block_on completes.
        let request = RustStreamRequest { sensor_id };
        let result = py.allow_threads(|| {
            let inner = self.inner.lock().expect("ClusterRuntime mutex poisoned");
            let rt = inner.as_ref().ok_or(())?;
            let tokio_rt = cluster_tokio_runtime();
            Ok(tokio_rt.block_on(async {
                rt.open_stream::<auki_network_rs::stream_protocol::JpegFrame>(peer, request)
                    .await
            }))
        });

        let rust_sub = match result {
            Err(()) => return Err(shutdown_error()),
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(open_stream_error_to_pyerr(py, e)),
        };

        Ok(PyStreamSubscription::from_rust_jpeg(rust_sub))
    }

    /// Open an outbound `/auki/stream/1.0.0` subscription on `peer_id`
    /// for the named sensor with `T = PointCloudFrame` (Dagaz Batch 2).
    /// The producer must accept with
    /// [`PyStreamDecision.accept_pointcloud(...)`].
    ///
    /// Same blocking shape as [`open_stream`][ClusterRuntime::open_stream]:
    /// returns once the producer has Accepted, Declined, or the 30s
    /// timeout fires. Returns a [`StreamSubscription`] whose frames
    /// carry [`PointCloudFrame`] payloads — raw CDR-encoded
    /// `PointCloud2` ROS message bytes that the consumer parses on its
    /// side. Same exception surface as `open_stream`.
    #[pyo3(text_signature = "($self, peer_id, sensor_id)")]
    fn open_pointcloud_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: String,
    ) -> PyResult<PyStreamSubscription> {
        let peer = PeerId::from_str(peer_id).map_err(|e| {
            PyValueError::new_err(format!("invalid peer_id {peer_id:?}: {e}"))
        })?;

        let request = RustStreamRequest { sensor_id };
        let result = py.allow_threads(|| {
            let inner = self.inner.lock().expect("ClusterRuntime mutex poisoned");
            let rt = inner.as_ref().ok_or(())?;
            let tokio_rt = cluster_tokio_runtime();
            Ok(tokio_rt.block_on(async {
                rt.open_stream::<auki_network_rs::stream_protocol::PointCloudFrame>(
                    peer, request,
                )
                .await
            }))
        });

        let rust_sub = match result {
            Err(()) => return Err(shutdown_error()),
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(open_stream_error_to_pyerr(py, e)),
        };

        Ok(PyStreamSubscription::from_rust_pointcloud(rust_sub))
    }

    /// Signal the driver task to shut down and abort it. Idempotent in
    /// the sense that a second call raises rather than silently no-ops
    /// — use-after-shutdown is almost always a bug, and a noisy raise
    /// is the right signal.
    ///
    /// Raises `RuntimeError` if already shut down.
    fn shutdown(&self) -> PyResult<()> {
        let mut inner = self.inner.lock().expect("ClusterRuntime mutex poisoned");
        let rt = inner.take().ok_or_else(shutdown_error)?;
        rt.shutdown();
        Ok(())
    }

    fn __repr__(&self) -> String {
        let inner = self.inner.lock().expect("ClusterRuntime mutex poisoned");
        match inner.as_ref() {
            Some(rt) => format!("ClusterRuntime(connected_peers={})", rt.peers().len()),
            None => "ClusterRuntime(shut_down=True)".to_string(),
        }
    }
}

fn shutdown_error() -> PyErr {
    PyRuntimeError::new_err("ClusterRuntime has been shut down")
}

// Manual `Debug` so test helpers like `Result::expect_err` can format the
// runtime; the underlying `auki_network::cluster_runtime::ClusterRuntime`
// doesn't derive `Debug`. Don't reach into the inner runtime — just
// report whether it's still alive.
impl std::fmt::Debug for ClusterRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().expect("ClusterRuntime mutex poisoned");
        f.debug_struct("ClusterRuntime")
            .field("state", &if inner.is_some() { "running" } else { "shut_down" })
            .finish()
    }
}

// ─── cluster.spawn ───────────────────────────────────────────────────────────

/// Boot a `ClusterRuntime` against `doc`. Lazily creates a process-wide
/// tokio runtime on first call; subsequent calls share it.
///
/// **`seed` must be the *peer* seed**, not the wallet seed. The runtime
/// constructs the swarm's keypair via
/// `PeerIdentity::from_seed(seed)` — i.e. direct ed25519 from the 32
/// bytes — *not* via `from_wallet`. For consumers that root identity in
/// a wallet (e.g. BoosterApp's sidecar), the right invocation is:
///
/// ```python
/// wallet = auki_identity.Wallet.from_seed(load_or_mint_seed(...))
/// peer = wallet.derive_child("peer/v1")
/// runtime = auki_network.cluster.spawn(seed=peer.seed(), doc=doc, ...)
/// # runtime's PeerId == peer.peer_id(), by construction.
/// ```
///
/// Passing the wallet seed directly will produce a swarm whose PeerId
/// is `from_seed(wallet_seed)` rather than the wallet-derived peer
/// identity — the two differ, and operators putting the
/// wallet-derived peer_id in `cluster.json` will get Noise mismatch
/// rejections at connection time.
///
/// `participant_provider` is a Python callable invoked **per inbound
/// `/auki/cluster/1.0.0` request** by the cluster runtime's worker
/// task. The wrapper acquires the GIL, calls it, and:
///
/// - returns the `ParticipantInfo` to the runtime if the callable
///   returned one,
/// - returns `None` to the runtime if the callable returned Python's
///   `None`, raised an exception (caught + logged via
///   `tracing::warn!`), or returned anything other than a
///   `ParticipantInfo` (also logged).
///
/// On `None`, the runtime drops the inbound request's reply channel —
/// the requester sees a timeout, the runtime stays alive, future
/// requests still get answered.
///
/// **Provider performance contract:** the callable runs on the
/// runtime's only worker. Brief GIL contention is fine; sustained
/// contention (I/O, contended locks beyond a brief copy) measurably
/// impacts cluster responsiveness. Build the `ParticipantInfo` from
/// cached state.
///
/// Kwargs (all optional, sensible defaults):
/// - `stream_provider: Callable[[StreamRequest], StreamDecision] | None` —
///   inbound `/auki/stream/1.0.0` request handler (grimsby deliverable
///   #4). Synchronous; on `Accept`, the source must be a Python async
///   iterator yielding `ProducerFrame` values. Default `None` —
///   consumer-only mode: every inbound request gets
///   `DeclineReason.sensor_not_found`. The same shape Park uses on the
///   Rust side (`decline_all_streams`).
/// - `listen_addresses: list[str] | None` — multiaddr strings the
///   swarm will listen on. Default: TCP+QUIC on `0.0.0.0`, OS-chosen
///   ports (`/ip4/0.0.0.0/tcp/0`, `/ip4/0.0.0.0/udp/0/quic-v1`). An
///   empty list builds a dial-only swarm.
/// - `agent_version: str | None` — reported in libp2p `identify`
///   responses. Default: `auki-network-py/<crate-version>`.
/// - `enable_mdns: bool` — `_p2p._udp.local.` LAN discovery. Default
///   `True` (matches `SwarmConfig::default`).
///
/// Raises:
/// - `ValueError` if `seed` is not exactly 32 bytes, or if any string
///   in `listen_addresses` does not parse as a multiaddr.
/// - `RuntimeError` if the underlying swarm build fails (transport
///   stack assembly, `listen_on` rejection, etc.).
#[pyfunction]
#[pyo3(
    name = "spawn",
    text_signature = "(seed, doc, participant_provider, *, stream_provider=None, listen_addresses=None, agent_version=None, enable_mdns=True)",
    signature = (
        seed,
        doc,
        participant_provider,
        *,
        stream_provider = None,
        listen_addresses = None,
        agent_version = None,
        enable_mdns = true,
    ),
)]
#[allow(clippy::too_many_arguments)]
fn spawn(
    py: Python<'_>,
    seed: &Bound<'_, PyBytes>,
    doc: &ClusterDoc,
    participant_provider: PyObject,
    stream_provider: Option<PyObject>,
    listen_addresses: Option<Vec<String>>,
    agent_version: Option<String>,
    enable_mdns: bool,
) -> PyResult<ClusterRuntime> {
    // 1. Validate seed length — wrapper-side, before calling into Rust.
    //    The runtime accepts `[u8; 32]`, so an out-of-range slice would
    //    panic on `copy_from_slice`. Catching here gives a clean
    //    `ValueError` instead of a Rust panic across the FFI seam.
    let seed_bytes: &[u8] = seed.as_bytes();
    if seed_bytes.len() != 32 {
        return Err(PyValueError::new_err(format!(
            "seed must be exactly 32 bytes, got {}",
            seed_bytes.len()
        )));
    }
    let mut seed_arr = [0u8; 32];
    seed_arr.copy_from_slice(seed_bytes);

    // 2. Build listen addresses — defaults to OS-chosen TCP + QUIC.
    let listen = match listen_addresses {
        None => vec![
            "/ip4/0.0.0.0/tcp/0"
                .parse::<Multiaddr>()
                .expect("hardcoded multiaddr is valid"),
            "/ip4/0.0.0.0/udp/0/quic-v1"
                .parse::<Multiaddr>()
                .expect("hardcoded multiaddr is valid"),
        ],
        Some(addrs) => addrs
            .into_iter()
            .map(|s| {
                s.parse::<Multiaddr>().map_err(|e| {
                    PyValueError::new_err(format!("invalid multiaddr {s:?}: {e}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };

    let agent_version = agent_version
        .unwrap_or_else(|| format!("auki-network-py/{}", env!("CARGO_PKG_VERSION")));

    let swarm_config = SwarmConfig {
        listen_addresses: listen,
        agent_version,
        enable_mdns,
        // Relay-server is the dedicated `aukilabs/relay` app's job;
        // wrapper consumers (Boosterapp, Sentinel) never want it.
        // Hardcoded off; if a future consumer needs it we'll add a
        // kwarg.
        enable_relay_server: false,
    };

    // 3. Build the `ParticipantInfoProvider` — a closure that calls
    //    the Python callable through the GIL on each invocation. The
    //    runtime calls this once per inbound cluster request.
    let provider_obj: Py<PyAny> = participant_provider;
    let provider: ParticipantInfoProvider = Arc::new(move || -> Option<RustParticipantInfo> {
        Python::with_gil(|py| {
            // Call. A Python exception here is caught and logged; the
            // runtime sees `None` and drops the reply channel.
            let result = match provider_obj.call0(py) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "participant_provider raised; dropping reply");
                    return None;
                }
            };
            // Python `None` → drop the reply (transient unavailability;
            // session clock not bound yet, etc.).
            if result.is_none(py) {
                return None;
            }
            // Try to extract a ParticipantInfo. Anything else is a bug
            // in the consumer — log and drop.
            match result.bind(py).extract::<ParticipantInfo>() {
                Ok(info) => Some(info.to_rust()),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "participant_provider returned non-ParticipantInfo; dropping reply",
                    );
                    None
                }
            }
        })
    });

    // 4. Build the `StreamProvider<JpegFrame>`. If the caller supplied
    //    `stream_provider=...`, wrap it as a Python-driven provider
    //    (the source-Stream is a Python async iterator drained on the
    //    wrapper's asyncio loop). Otherwise default to
    //    `decline_all_streams()` — the consumer-only shape Park's Rust
    //    side took in [park#20].
    let stream_prov = match stream_provider {
        Some(callable) => build_stream_provider(callable),
        None => decline_all_streams(),
    };

    // 5. Enter the tokio runtime context so `ClusterRuntime::spawn`'s
    //    internal `Handle::try_current()` succeeds. The driver task is
    //    spawned on a tokio worker thread, so the guard can drop
    //    immediately after spawn returns — the task continues running
    //    on its own.
    //
    //    `py.allow_threads` releases the GIL during the spawn so the
    //    runtime task can acquire it later for the provider callback
    //    without deadlocking.
    let rt = cluster_tokio_runtime();
    let cluster = py.allow_threads(|| {
        let _guard = rt.enter();
        RustClusterRuntime::spawn(
            seed_arr,
            doc.inner.clone(),
            swarm_config,
            provider,
            stream_prov,
        )
    });

    let cluster = cluster.map_err(map_spawn_error)?;
    Ok(ClusterRuntime {
        inner: Mutex::new(Some(cluster)),
    })
}

fn map_spawn_error(e: SpawnError) -> PyErr {
    match e {
        SpawnError::BuildSwarm(b) => PyRuntimeError::new_err(format!("swarm build failed: {b}")),
        SpawnError::NoTokioRuntime => PyRuntimeError::new_err(
            "no tokio runtime — internal: the wrapper should always enter the runtime context before calling spawn",
        ),
    }
}

// ─── Module entry point ──────────────────────────────────────────────────────

/// Populate the module — exposed as a free function so tests can drive
/// it directly. The `#[pymodule]` entry point below is a thin wrapper.
fn populate_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();

    // Submodule: auki_network.cluster
    let cluster = PyModule::new_bound(py, "cluster")?;
    cluster.add_class::<ParticipantInfo>()?;
    cluster.add_class::<PeerSnapshot>()?;
    cluster.add_class::<ClusterDoc>()?;
    cluster.add_class::<ClusterRuntime>()?;
    cluster.add_function(wrap_pyfunction!(load_doc, &cluster)?)?;
    cluster.add_function(wrap_pyfunction!(resolve_cluster_name, &cluster)?)?;
    cluster.add_function(wrap_pyfunction!(spawn, &cluster)?)?;

    // Grimsby Stream<T> surface (deliverable #4).
    stream_types::register(py, &cluster)?;

    // Register the submodule in `sys.modules` so
    // `from auki_network import cluster` works the same as
    // `import auki_network.cluster`. Without this, only attribute
    // access through the parent module finds it.
    py.import_bound("sys")?
        .getattr("modules")?
        .set_item("auki_network.cluster", &cluster)?;
    m.add_submodule(&cluster)?;

    // Submodule: auki_network.discovery (Vinland Batch 2).
    let discovery_mod = PyModule::new_bound(py, "discovery")?;
    discovery::register_module(py, &discovery_mod)?;
    py.import_bound("sys")?
        .getattr("modules")?
        .set_item("auki_network.discovery", &discovery_mod)?;
    m.add_submodule(&discovery_mod)?;

    Ok(())
}

/// `auki_network` module. The `#[pymodule]` macro generates the
/// `PyInit_auki_network` C entry point Python imports.
#[pymodule]
fn auki_network(m: &Bound<'_, PyModule>) -> PyResult<()> {
    populate_module(m)
}

// ─── Rust-side smoke tests ────────────────────────────────────────────────────
//
// `cargo test` builds with the default features off (no
// `extension-module`) and the `auto-initialize` dev-dep enabled — that
// combination links a real Python interpreter into the test binary so
// `Python::with_gil` works without a host process.

#[cfg(test)]
mod tests {
    use super::*;

    /// Compute a real PeerId from a fixed ed25519 seed, mirroring the
    /// recipe used by `auki_network::participant`'s test fixture. Done
    /// on the fly so we don't bake a literal that could drift if the
    /// libp2p protobuf-multihash-base58 encoding changes — if the
    /// string is wrong, the test does the wrong thing silently.
    fn fixture_peer_id() -> String {
        use libp2p_identity::{Keypair, ed25519};
        let mut seed = [7u8; 32];
        let secret =
            ed25519::SecretKey::try_from_bytes(&mut seed).expect("32 bytes is a valid secret");
        let kp = Keypair::from(ed25519::Keypair::from(secret));
        kp.public().to_peer_id().to_string()
    }

    fn make_participant_info() -> ParticipantInfo {
        let peer_id = fixture_peer_id();
        ParticipantInfo::new(
            "boosterapp".into(),
            "k1-walker".into(),
            "11111111-2222-4333-8444-555555555555".into(),
            "K1-AABBCCDDEEFF/session-monotonic".into(),
            "abc123".into(),
            12_345_678_900,
            Some(1_745_000_000),
            &peer_id,
            "aabbccddeeff".into(),
        )
        .expect("construct fixture ParticipantInfo")
    }

    #[test]
    fn module_exposes_cluster_submodule_with_documented_surface() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_network").unwrap();
            populate_module(&module).unwrap();

            let cluster = module.getattr("cluster").unwrap();
            // Ansuz surface (v0.0.14).
            assert!(cluster.getattr("ParticipantInfo").is_ok());
            assert!(cluster.getattr("PeerSnapshot").is_ok());
            assert!(cluster.getattr("ClusterDoc").is_ok());
            assert!(cluster.getattr("ClusterRuntime").is_ok());
            assert!(cluster.getattr("load_doc").is_ok());
            assert!(cluster.getattr("resolve_cluster_name").is_ok());
            assert!(cluster.getattr("spawn").is_ok());
            // Grimsby Stream<T> surface (deliverable #4 / v0.0.17) +
            // Dagaz Batch 2 PointCloudFrame (v0.0.21).
            assert!(cluster.getattr("StreamRequest").is_ok());
            assert!(cluster.getattr("AcceptInfo").is_ok());
            assert!(cluster.getattr("JpegFrame").is_ok());
            assert!(cluster.getattr("PointCloudFrame").is_ok());
            assert!(cluster.getattr("DeclineReason").is_ok());
            assert!(cluster.getattr("EndReason").is_ok());
            assert!(cluster.getattr("ProducerFrame").is_ok());
            assert!(cluster.getattr("ConsumerFrame").is_ok());
            assert!(cluster.getattr("StreamDecision").is_ok());
            assert!(cluster.getattr("StreamSubscription").is_ok());
            assert!(cluster.getattr("FrameIterator").is_ok());
            assert!(cluster.getattr("StreamEndOfStream").is_ok());
            assert!(cluster.getattr("StreamConnectionLost").is_ok());
            assert!(cluster.getattr("StreamProtocolError").is_ok());
            assert!(cluster.getattr("StreamDeclined").is_ok());
            assert!(cluster.getattr("StreamUnreachable").is_ok());
        });
    }

    /// Pin the `auki_network.discovery` submodule surface (Vinland
    /// Batch 2 / v0.0.19). Anything new requires a deliberate decision
    /// and a changelog bump.
    #[test]
    fn module_exposes_discovery_submodule_with_documented_surface() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_network").unwrap();
            populate_module(&module).unwrap();

            let discovery = module.getattr("discovery").unwrap();
            assert!(discovery.getattr("DiscoveryClient").is_ok());
            assert!(discovery.getattr("DiscoveryUnreachable").is_ok());
            assert!(discovery.getattr("DiscoveryRejected").is_ok());
            assert!(discovery.getattr("DiscoveryClockError").is_ok());

            // `from auki_network import discovery` works because the
            // wrapper registered the submodule in `sys.modules`.
            let sys = py.import_bound("sys").unwrap();
            let modules = sys.getattr("modules").unwrap();
            assert!(modules.contains("auki_network.discovery").unwrap());
        });
    }

    #[test]
    fn participant_info_round_trips_through_constructor_and_getters() {
        Python::with_gil(|_py| {
            let p = make_participant_info();
            assert_eq!(p.app(), "boosterapp");
            assert_eq!(p.name(), "k1-walker");
            assert_eq!(p.session_now_ns(), 12_345_678_900);
            assert_eq!(p.cluster_joined_at_ns(), Some(1_745_000_000));
            assert_eq!(p.peer_id(), fixture_peer_id());
            assert_eq!(p.app_instance(), "aabbccddeeff");
        });
    }

    #[test]
    fn participant_info_rejects_invalid_peer_id() {
        Python::with_gil(|_py| {
            let result = ParticipantInfo::new(
                "boosterapp".into(),
                "k1-walker".into(),
                "session".into(),
                "clock".into(),
                "hash".into(),
                0,
                None,
                "not-a-peer-id",
                "aabbccddeeff".into(),
            );
            assert!(result.is_err(), "invalid peer_id must fail to parse");
        });
    }

    #[test]
    fn participant_info_eq_compares_all_fields() {
        Python::with_gil(|_py| {
            let a = make_participant_info();
            let b = make_participant_info();
            assert!(a.__eq__(&b));

            // Mutate one field via re-construction; equality breaks.
            let peer_id = fixture_peer_id();
            let c = ParticipantInfo::new(
                "sentinel".into(), // different app
                "k1-walker".into(),
                "11111111-2222-4333-8444-555555555555".into(),
                "K1-AABBCCDDEEFF/session-monotonic".into(),
                "abc123".into(),
                12_345_678_900,
                Some(1_745_000_000),
                &peer_id,
                "aabbccddeeff".into(),
            )
            .unwrap();
            assert!(!a.__eq__(&c));
        });
    }

    #[test]
    fn load_doc_round_trips_a_minimal_cluster_json() {
        Python::with_gil(|_py| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("cluster.json");
            // Minimal valid cluster.json — single peer, no addresses.
            // Covers the happy path; address-having peers are exercised
            // by `auki-network`'s own cluster_doc tests, which we don't
            // re-run here.
            let peer_id = fixture_peer_id();
            std::fs::write(
                &path,
                format!(
                    r#"{{
                        "version": 1,
                        "cluster_name": "test",
                        "peers": [{{
                            "peer_id": "{peer_id}",
                            "addresses": []
                        }}]
                    }}"#,
                ),
            )
            .unwrap();

            let doc = load_doc(path).expect("load_doc happy path");
            assert_eq!(doc.cluster_name(), "test");
            assert_eq!(doc.peer_count(), 1);
        });
    }

    #[test]
    fn load_doc_rejects_missing_file_with_oserror() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("does-not-exist.json");
            let err = load_doc(path).expect_err("missing file must error");
            // Verify the error type maps to OSError (PyOSError).
            assert!(err.is_instance_of::<PyOSError>(py));
        });
    }

    #[test]
    fn load_doc_rejects_unsupported_version_with_value_error() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("cluster.json");
            std::fs::write(
                &path,
                r#"{"version": 99, "cluster_name": "x", "peers": []}"#,
            )
            .unwrap();
            let err = load_doc(path).expect_err("unsupported version must error");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(
                err.to_string().contains("unsupported version 99"),
                "error message should name the bad version: {err}",
            );
        });
    }

    // ─── resolve_cluster_name ───────────────────────────────────────────

    /// Tests touching `AUKI_CLUSTER_NAME` must hold this lock — Cargo
    /// runs tests in parallel, and `set_var` / `remove_var` aren't
    /// thread-safe with concurrent reads.
    fn cluster_name_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save_and_clear_cluster_name_env() -> Option<String> {
        let prev = std::env::var(rust_cluster_name::ENV_VAR).ok();
        // SAFETY: serialized via `cluster_name_env_lock`.
        unsafe { std::env::remove_var(rust_cluster_name::ENV_VAR) };
        prev
    }

    fn restore_cluster_name_env(prev: Option<String>) {
        // SAFETY: serialized via `cluster_name_env_lock`.
        unsafe {
            match prev {
                Some(v) => std::env::set_var(rust_cluster_name::ENV_VAR, v),
                None => std::env::remove_var(rust_cluster_name::ENV_VAR),
            }
        }
    }

    #[test]
    fn resolve_cluster_name_returns_flag_when_set() {
        let _g = cluster_name_env_lock();
        let prev = save_and_clear_cluster_name_env();
        let result = resolve_cluster_name(Some("sawslin"));
        restore_cluster_name_env(prev);
        assert_eq!(result.unwrap(), "sawslin");
    }

    #[test]
    fn resolve_cluster_name_falls_back_to_env() {
        let _g = cluster_name_env_lock();
        let prev = save_and_clear_cluster_name_env();
        // SAFETY: lock held.
        unsafe { std::env::set_var(rust_cluster_name::ENV_VAR, "from-env") };
        let result = resolve_cluster_name(None);
        restore_cluster_name_env(prev);
        assert_eq!(result.unwrap(), "from-env");
    }

    #[test]
    fn resolve_cluster_name_raises_value_error_when_unset() {
        Python::with_gil(|py| {
            let _g = cluster_name_env_lock();
            let prev = save_and_clear_cluster_name_env();
            let result = resolve_cluster_name(None);
            restore_cluster_name_env(prev);
            let err = result.expect_err("missing cluster_name must raise");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(err.to_string().contains("cluster_name unset"));
        });
    }

    #[test]
    fn load_doc_rejects_invalid_peer_id_with_value_error() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("cluster.json");
            std::fs::write(
                &path,
                r#"{"version": 1, "cluster_name": "x", "peers": [{"peer_id": "not-a-peer-id", "addresses": []}]}"#,
            )
            .unwrap();
            let err = load_doc(path).expect_err("invalid peer_id must error");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(
                err.to_string().contains("invalid peer_id"),
                "error message should flag invalid peer_id: {err}",
            );
        });
    }

    /// One-shot helper to compute the peer_ids the Python E2E test bakes
    /// as constants. Print-only — `cargo test print_python_e2e_peer_ids
    /// -- --nocapture` regenerates the strings if a seed needs to change.
    /// Test always passes (it's an output emitter, not an assertion).
    #[test]
    fn print_python_e2e_peer_ids() {
        use libp2p_identity::{Keypair, ed25519};
        for byte in [16u8, 17u8] {
            let mut seed = [byte; 32];
            let secret = ed25519::SecretKey::try_from_bytes(&mut seed).unwrap();
            let kp = Keypair::from(ed25519::Keypair::from(secret));
            eprintln!(
                "Python E2E fixture: seed [0x{byte:02x}; 32] => {}",
                kp.public().to_peer_id()
            );
        }
    }

    // ─── spawn / ClusterRuntime tests ────────────────────────────────────────

    /// Build a minimal valid ClusterDoc with no peers — the alone-cluster
    /// shape. Used for spawn tests that exercise lifecycle without
    /// actually dialing anyone.
    fn empty_cluster_doc() -> ClusterDoc {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cluster.json");
        std::fs::write(
            &path,
            r#"{"version": 1, "cluster_name": "alone", "peers": []}"#,
        )
        .unwrap();
        load_doc(path).unwrap()
    }

    #[test]
    fn spawn_rejects_wrong_seed_length_with_value_error() {
        Python::with_gil(|py| {
            let doc = empty_cluster_doc();
            let bad_seed = PyBytes::new_bound(py, &[0u8; 16]);
            let provider = py.None();
            let err = spawn(py, &bad_seed, &doc, provider, None, None, None, true)
                .expect_err("16-byte seed must be rejected");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(
                err.to_string().contains("32 bytes"),
                "error message should name 32-byte requirement: {err}",
            );
        });
    }

    #[test]
    fn spawn_rejects_invalid_multiaddr_with_value_error() {
        Python::with_gil(|py| {
            let doc = empty_cluster_doc();
            let seed = PyBytes::new_bound(py, &[0u8; 32]);
            let provider = py.None();
            let err = spawn(
                py,
                &seed,
                &doc,
                provider,
                None,
                Some(vec!["not a multiaddr".to_string()]),
                None,
                true,
            )
            .expect_err("invalid multiaddr must be rejected");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(
                err.to_string().contains("invalid multiaddr"),
                "error message should flag invalid multiaddr: {err}",
            );
        });
    }

    #[test]
    fn spawn_then_peers_then_shutdown_round_trip() {
        // Real-runtime exercise. Spawns a swarm, listens on a loopback
        // port, no peers in the doc so no dialing. Verifies:
        //   - spawn returns a usable ClusterRuntime
        //   - peers() returns [] when doc is empty
        //   - shutdown() succeeds
        //   - shutdown() a second time raises RuntimeError
        //   - peers() after shutdown raises RuntimeError
        Python::with_gil(|py| {
            let doc = empty_cluster_doc();
            let seed = PyBytes::new_bound(py, &[42u8; 32]);
            let provider = py.None(); // never invoked (no peers)
            let listen = Some(vec!["/ip4/127.0.0.1/tcp/0".to_string()]);

            let runtime = spawn(py, &seed, &doc, provider, None, listen, None, false)
                .expect("spawn happy path");

            // Empty cluster — no peers visible.
            let peers = runtime.peers().expect("peers() before shutdown");
            assert_eq!(peers.len(), 0);

            // First shutdown clean.
            runtime.shutdown().expect("first shutdown");

            // Second shutdown raises (use-after-shutdown is a bug).
            let err = runtime
                .shutdown()
                .expect_err("second shutdown must raise");
            assert!(err.is_instance_of::<PyRuntimeError>(py));
            assert!(err.to_string().contains("shut down"));

            // peers() after shutdown raises.
            let err = runtime
                .peers()
                .expect_err("peers() after shutdown must raise");
            assert!(err.is_instance_of::<PyRuntimeError>(py));
        });
    }
}

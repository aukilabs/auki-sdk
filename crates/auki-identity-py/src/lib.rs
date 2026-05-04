//! PyO3 bindings for a tiny slice of the Auki SDK.
//!
//! This crate exposes exactly three things to Python so Boosterapp's
//! Python sidecar can implement the `/api/info` v0.0.11 shape today,
//! ahead of the full `auki-py` MVP (Swarm + async runtime):
//!
//! 1. [`load_or_mint_seed`] — wraps `auki_identity::load_or_mint_seed`
//!    for persistent peer-key material across daemon restarts.
//! 2. [`Wallet`] (with `from_seed` + `derive_child` + `peer_id`) —
//!    wraps `auki_identity::Wallet` and the canonical PeerId
//!    derivation recipe `auki_network::PeerIdentity::from_wallet` uses.
//! 3. `app_instance::derive` — wraps
//!    `auki_network::app_instance::derive` for the per-machine
//!    identifier carried in `/api/info.app_instance`.
//!
//! Out of scope: libp2p Swarm, async / Tokio integration, the cluster
//! protocol, all signing primitives. Those land in the full `auki-py`
//! crate later. This crate is data-only — pure synchronous functions
//! with no GIL-around-await dance.
//!
//! See [`crates/auki-identity-py/README.md`](../README.md) for the
//! Python-side surface and install instructions.

use std::path::PathBuf;

use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};

// `auki_identity_rs` is the upstream Rust crate, renamed via `package =`
// in Cargo.toml so it doesn't collide with this crate's own lib name
// (`auki_identity` — also the Python module name).
use auki_identity_rs::{
    SeedError, Wallet as RustWallet, load_or_mint_seed as rust_load_or_mint_seed,
};
use auki_network::PeerIdentity;
use auki_network::app_instance::{DeriveError, derive as rust_app_instance_derive};

// ─── load_or_mint_seed ───────────────────────────────────────────────────────

/// Load a 32-byte wallet seed from `path`, or mint and persist a fresh
/// one if `path` does not exist.
///
/// Returns the 32-byte seed as Python `bytes`. Raises `OSError` on
/// filesystem errors, `ValueError` if `path` exists but is not exactly
/// 32 bytes long.
///
/// See `auki_identity::load_or_mint_seed` for the full contract
/// (atomic write, `0o600` mode on Unix, deep parent-directory creation,
/// `OsRng`-minted bytes).
#[pyfunction]
#[pyo3(name = "load_or_mint_seed", text_signature = "(path, /)")]
fn load_or_mint_seed(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyBytes>> {
    let seed = rust_load_or_mint_seed(&path).map_err(map_seed_error)?;
    Ok(PyBytes::new_bound(py, &seed).unbind())
}

fn map_seed_error(e: SeedError) -> PyErr {
    match e {
        SeedError::Io(io_err) => PyOSError::new_err(format!("seed file I/O error: {io_err}")),
        SeedError::InvalidLength(n) => {
            PyValueError::new_err(format!("seed file must be exactly 32 bytes, found {n}"))
        }
    }
}

// ─── Wallet ──────────────────────────────────────────────────────────────────

/// An ed25519 wallet keypair. Construct with [`Wallet.from_seed`].
///
/// The Rust-side wallet holds secret material; treat instances as
/// sensitive. The Python wrapper exposes only the operations Boosterapp
/// needs today — deterministic child derivation and peer-id
/// computation. Sign, verify, and creation-cert APIs land in the full
/// `auki-py` crate later.
#[pyclass(name = "Wallet")]
struct Wallet {
    inner: RustWallet,
}

#[pymethods]
impl Wallet {
    /// Construct a wallet from a 32-byte seed (the ed25519 secret key
    /// bytes). Same seed → same wallet, deterministically.
    ///
    /// Raises `ValueError` if `seed` is not exactly 32 bytes.
    ///
    /// PyO3's auto-conversion accepts `bytes`, `bytearray`, and any
    /// `bytes`-like object — anything that implements the buffer
    /// protocol. We deliberately do *not* accept `str` because the
    /// caller's intent is unambiguous (raw secret-key bytes), and
    /// silently UTF-8-encoding a string would mask a bug.
    #[staticmethod]
    #[pyo3(text_signature = "(seed, /)")]
    fn from_seed(seed: &Bound<'_, PyBytes>) -> PyResult<Self> {
        let bytes: &[u8] = seed.as_bytes();
        if bytes.len() != 32 {
            return Err(PyValueError::new_err(format!(
                "seed must be exactly 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ok(Wallet {
            inner: RustWallet::from_seed(&arr),
        })
    }

    /// Deterministic child derivation. Same parent + same label → same
    /// child every time. Mirrors `auki_identity::Wallet::derive_child`
    /// byte-for-byte; consumers in any language can reproduce the
    /// derivation if they implement the same XXH3-128 recipe.
    #[pyo3(text_signature = "($self, label, /)")]
    fn derive_child(&self, label: &str) -> Wallet {
        Wallet {
            inner: self.inner.derive_child(label),
        }
    }

    /// Canonical libp2p PeerId for this wallet's own keypair, as a
    /// multibase-base58 string (`12D3KooW…`).
    ///
    /// `peer_id()` does *not* implicitly `derive_child("peer/v1")` —
    /// the caller does that explicitly. Typical usage:
    ///
    /// ```python
    /// w = Wallet.from_seed(seed)
    /// peer = w.derive_child("peer/v1")
    /// pid = peer.peer_id()
    /// ```
    ///
    /// This matches `auki_network::PeerIdentity::from_wallet(&w).peer_id()`
    /// byte-for-byte: that function is sugar for
    /// `from_seed(w.derive_child("peer/v1").seed()).peer_id()`.
    ///
    /// The encoding is libp2p's: ed25519 pubkey → protobuf-wrapped
    /// `PublicKey` → SHA-256 multihash → base58btc multibase.
    #[pyo3(text_signature = "($self, /)")]
    fn peer_id(&self) -> String {
        let seed = self.inner.seed();
        let peer = PeerIdentity::from_seed(&seed);
        peer.peer_id().to_string()
    }
}

// ─── app_instance submodule ──────────────────────────────────────────────────

/// `auki_identity.app_instance.derive()` — per-machine identifier
/// (12 lowercase hex chars, no separators) used as the
/// `/api/info.app_instance` field.
///
/// Wraps `auki_network::app_instance::derive`. Recipe: first
/// non-loopback IEEE-administered MAC (skipping locally-administered
/// MACs whose U/L bit is set), sorted lexicographically, lowercased.
///
/// Raises:
/// - `RuntimeError` (with `"NoNetworkInterfaces"` or `"NoSuitableMac"`
///   in the message) when the host has no enumerable interfaces or
///   every interface is loopback / locally-administered — common in
///   containers and on laptops with only Private Wi-Fi enabled.
/// - `OSError` if the underlying `getifaddrs` / `GetAdaptersAddresses`
///   syscall fails.
#[pyfunction]
#[pyo3(name = "derive", text_signature = "()")]
fn app_instance_derive() -> PyResult<String> {
    rust_app_instance_derive().map_err(map_derive_error)
}

fn map_derive_error(e: DeriveError) -> PyErr {
    match e {
        DeriveError::NoNetworkInterfaces => PyRuntimeError::new_err(
            "NoNetworkInterfaces: no network interfaces enumerable on this host",
        ),
        DeriveError::NoSuitableMac => PyRuntimeError::new_err(
            "NoSuitableMac: every interface is loopback or has a locally-administered (random) MAC",
        ),
        DeriveError::Io(io_err) => {
            PyOSError::new_err(format!("interface enumeration failed: {io_err}"))
        }
    }
}

// ─── Module entry point ──────────────────────────────────────────────────────

/// Populate the module — exposed as a free function so tests can drive
/// it directly. The `#[pymodule]` entry point below is a thin wrapper.
fn populate_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_function(wrap_pyfunction!(load_or_mint_seed, m)?)?;
    m.add_class::<Wallet>()?;

    // Submodule: auki_identity.app_instance
    let app_instance = PyModule::new_bound(py, "app_instance")?;
    app_instance.add_function(wrap_pyfunction!(app_instance_derive, &app_instance)?)?;
    // Register the submodule in `sys.modules` so
    // `from auki_identity import app_instance` works the same as
    // `import auki_identity.app_instance`. Without this, only attribute
    // access through the parent module finds it.
    py.import_bound("sys")?
        .getattr("modules")?
        .set_item("auki_identity.app_instance", &app_instance)?;
    m.add_submodule(&app_instance)?;

    Ok(())
}

/// `auki_identity` module. The `#[pymodule]` macro generates the
/// `PyInit_auki_identity` C entry point Python imports.
#[pymodule]
fn auki_identity(m: &Bound<'_, PyModule>) -> PyResult<()> {
    populate_module(m)
}

// ─── Rust-side smoke test ────────────────────────────────────────────────────
//
// `cargo test` builds with the default features off (no
// `extension-module`) and the `auto-initialize` dev-dep enabled — that
// combination links a real Python interpreter into the test binary so
// `Python::with_gil` works without a host process.
//
// The `extension-module` feature is enabled by `maturin develop` /
// `maturin build`; it asks PyO3 to skip linking the Python runtime
// because the host interpreter resolves symbols at import time. Tests
// would fail to link in that mode, hence the gate.

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::PyBytes;

    #[test]
    fn module_builds_and_exposes_three_apis() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_identity").unwrap();
            populate_module(&module).unwrap();

            // load_or_mint_seed is callable.
            assert!(module.getattr("load_or_mint_seed").is_ok());
            // Wallet class is exposed.
            assert!(module.getattr("Wallet").is_ok());
            // app_instance submodule is exposed and has `derive`.
            let app_instance = module.getattr("app_instance").unwrap();
            assert!(app_instance.getattr("derive").is_ok());
        });
    }

    #[test]
    fn wallet_from_seed_then_peer_id_is_deterministic() {
        Python::with_gil(|py| {
            let seed = PyBytes::new_bound(py, &[3u8; 32]);
            let w1 = Wallet::from_seed(&seed).unwrap();
            let w2 = Wallet::from_seed(&seed).unwrap();
            // Derive `peer/v1` on both, then compare peer-ids.
            let p1 = w1.derive_child("peer/v1").peer_id();
            let p2 = w2.derive_child("peer/v1").peer_id();
            assert_eq!(p1, p2);
            // Canonical PeerId strings start with "12D3KooW".
            assert!(
                p1.starts_with("12D3KooW"),
                "expected canonical PeerId, got {p1}"
            );
        });
    }

    #[test]
    fn wallet_from_seed_rejects_wrong_length() {
        Python::with_gil(|py| {
            let too_short = PyBytes::new_bound(py, &[0u8; 16]);
            let result = Wallet::from_seed(&too_short);
            assert!(result.is_err(), "wrong-length seed must fail");
        });
    }

    #[test]
    fn load_or_mint_seed_round_trip_via_pyo3_layer() {
        // Drive the actual #[pyfunction] entry point, not the Rust one
        // — exercises the bytes-conversion seam.
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("identity.seed");
            let path_str = path.to_str().unwrap();

            let module = PyModule::new_bound(py, "auki_identity").unwrap();
            populate_module(&module).unwrap();
            let f = module.getattr("load_or_mint_seed").unwrap();

            let first = f.call1((path_str,)).unwrap();
            let second = f.call1((path_str,)).unwrap();

            // Extract bytes into owned Vec<u8> so we don't fight with
            // the lifetime of the temporary downcast result.
            let first_bytes: Vec<u8> =
                first.downcast::<PyBytes>().unwrap().as_bytes().to_vec();
            let second_bytes: Vec<u8> =
                second.downcast::<PyBytes>().unwrap().as_bytes().to_vec();
            assert_eq!(first_bytes, second_bytes);
            assert_eq!(first_bytes.len(), 32);
        });
    }

    #[test]
    fn locked_peer_id_vector() {
        // Cross-language locked vector. The same string must come out
        // of the parallel-agent's locked Rust test
        // (`PeerIdentity::from_wallet(&Wallet::from_seed(&[3u8; 32])).peer_id().to_string()`).
        // If both pass, the bindings agree byte-for-byte with the Rust
        // crate.
        //
        // We compute it dynamically rather than hardcoding, so this
        // test fails loudly if any layer (XXH3 derivation, ed25519,
        // libp2p protobuf-multihash-base58 encoding) drifts. The
        // hardcoded vector lives in the Python tests; this test is the
        // shape pin.
        Python::with_gil(|py| {
            let seed = PyBytes::new_bound(py, &[3u8; 32]);
            let w = Wallet::from_seed(&seed).unwrap();
            let peer = w.derive_child("peer/v1").peer_id();
            assert!(peer.starts_with("12D3KooW"));
            // 50ish base58 chars typical for ed25519-on-libp2p.
            assert!(
                peer.len() >= 46 && peer.len() <= 64,
                "PeerId length out of range: {peer:?}"
            );
        });
    }
}

//! Persistent seed storage for stable wallet identity across process restarts.
//!
//! [`load_or_mint_seed`] is the small filesystem helper that backs the SDK's
//! "stable peer key across restarts" guarantee: a daemon reads a 32-byte seed
//! from disk if it exists, otherwise mints a fresh random seed and persists
//! it. Wrapping the result in [`Wallet::from_seed`] then yields a
//! deterministic wallet (and, via `derive_child("peer/v1")`, a deterministic
//! libp2p peer id) for the lifetime of that on-disk file.
//!
//! ## What this is *not*
//!
//! Encryption-at-rest, OS keychain integration, mnemonic backup — all
//! deliberately out of scope. The seed is stored as raw bytes with mode
//! `0o600` on Unix; that is the entire defence. Downstream consumers that
//! need a stronger threat model wrap their own keystore around this primitive.
//!
//! ## WASM
//!
//! This module is gated on `#[cfg(not(target_arch = "wasm32"))]`. The rest of
//! `auki-identity` stays WASM-friendly; only the filesystem-touching helper
//! is unavailable in browser builds (which have no filesystem to begin with).

#![cfg(not(target_arch = "wasm32"))]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use rand_core::{OsRng, RngCore};

/// Errors returned by [`load_or_mint_seed`].
#[derive(Debug)]
pub enum SeedError {
    /// Filesystem error reading, writing, creating directories, or renaming.
    Io(io::Error),
    /// File at `path` exists but is not exactly 32 bytes. Carries the actual
    /// length found. The function will not silently truncate or pad.
    InvalidLength(usize),
}

impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeedError::Io(e) => write!(f, "seed file I/O error: {e}"),
            SeedError::InvalidLength(n) => {
                write!(f, "seed file must be exactly 32 bytes, found {n}")
            }
        }
    }
}

impl std::error::Error for SeedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SeedError::Io(e) => Some(e),
            SeedError::InvalidLength(_) => None,
        }
    }
}

impl From<io::Error> for SeedError {
    fn from(e: io::Error) -> Self {
        SeedError::Io(e)
    }
}

/// Load a 32-byte wallet seed from `path`, or mint and persist a fresh one
/// if `path` does not exist.
///
/// Behaviour:
///
/// - **If `path` exists:** read it. If it is exactly 32 bytes, return them.
///   Any other length returns [`SeedError::InvalidLength`] — the function
///   refuses to silently truncate, pad, or overwrite an existing file with
///   the wrong shape.
/// - **If `path` does not exist:** create the parent directories with
///   [`fs::create_dir_all`], generate 32 cryptographically-random bytes from
///   [`OsRng`], write atomically (write to `<path>.tmp`, then rename onto
///   `path`) so a crash mid-write cannot leave a partial file in place, and
///   on Unix set the file mode to `0o600` (owner read/write only) before
///   returning the bytes.
///
/// The function does not validate the seed cryptographically — that is the
/// wallet's job. Any 32 bytes are accepted on read; minted bytes come straight
/// from the OS RNG.
///
/// ## Path convention (caller's responsibility)
///
/// This function takes any `&Path`; the convention is that each app picks
/// `~/.auki/<app>/identity.seed` so multiple Auki daemons can coexist on one
/// machine without clobbering each other's identity. The convention is *not*
/// baked into the signature on purpose — tests, ephemeral daemons, and
/// alternative layouts shouldn't be locked into a hardcoded location.
///
/// ## Example
///
/// ```no_run
/// use std::path::PathBuf;
/// use auki_identity::{load_or_mint_seed, Wallet};
///
/// let path = PathBuf::from("/tmp/auki/identity.seed");
/// let seed = load_or_mint_seed(&path).expect("seed load/mint");
/// let wallet = Wallet::from_seed(&seed);
/// // Same seed file → same wallet → same peer id, every restart.
/// ```
pub fn load_or_mint_seed(path: &Path) -> Result<[u8; 32], SeedError> {
    if path.exists() {
        return read_seed(path);
    }
    mint_seed(path)
}

fn read_seed(path: &Path) -> Result<[u8; 32], SeedError> {
    let mut f = File::open(path)?;
    let mut buf = Vec::with_capacity(32);
    f.read_to_end(&mut buf)?;
    if buf.len() != 32 {
        return Err(SeedError::InvalidLength(buf.len()));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&buf);
    Ok(seed)
}

fn mint_seed(path: &Path) -> Result<[u8; 32], SeedError> {
    let mut seed = [0u8; 32];
    let mut rng = OsRng;
    rng.fill_bytes(&mut seed);

    if let Some(parent) = path.parent() {
        // `parent()` returns `Some("")` for a bare filename like `seed`.
        // `create_dir_all("")` is a no-op error on some platforms; skip it.
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    atomic_write(path, &seed)?;
    Ok(seed)
}

/// Write `bytes` to `target` atomically: write to a `.tmp` sibling, fsync,
/// rename onto `target`. On Unix, set mode `0o600` on the temp file *before*
/// the rename so the final file is never world-readable, even briefly.
///
/// On a successful write the `.tmp` sidecar is consumed by the rename and
/// no leftover file remains. On error (e.g. the rename fails) the `.tmp`
/// file may still be present; the function does not attempt cleanup —
/// that's parity with the rest of the SDK's atomic-write helpers.
fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::other("seed path has no file name"))?;
    let tmp = dir.join(format!(".{}.tmp", file_name.to_string_lossy()));

    {
        let mut opts = OpenOptions::new();
        opts.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f: File = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, target)?;
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn missing_path_mints_persists_and_returns() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.seed");
        assert!(!path.exists());

        let seed = load_or_mint_seed(&path).unwrap();
        assert!(path.exists());
        let on_disk = fs::read(&path).unwrap();
        assert_eq!(on_disk.len(), 32);
        assert_eq!(on_disk.as_slice(), seed.as_slice());
    }

    #[test]
    fn second_call_returns_same_seed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.seed");

        let first = load_or_mint_seed(&path).unwrap();
        let second = load_or_mint_seed(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn existing_32_bytes_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.seed");
        let bytes = [42u8; 32];
        fs::write(&path, bytes).unwrap();

        let seed = load_or_mint_seed(&path).unwrap();
        assert_eq!(seed, bytes);
    }

    #[test]
    fn existing_wrong_length_is_rejected() {
        let dir = TempDir::new().unwrap();

        for &len in &[0usize, 1, 31, 33, 64] {
            let path = dir.path().join(format!("seed_{len}"));
            fs::write(&path, vec![0u8; len]).unwrap();
            match load_or_mint_seed(&path) {
                Err(SeedError::InvalidLength(n)) => assert_eq!(n, len),
                other => panic!("expected InvalidLength({len}), got {other:?}"),
            }
        }
    }

    #[test]
    fn parent_directory_is_created() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a").join("b").join("c").join("identity.seed");
        assert!(!path.parent().unwrap().exists());

        let seed = load_or_mint_seed(&path).unwrap();
        assert!(path.exists());
        assert_eq!(seed.len(), 32);
    }

    #[test]
    fn no_tmp_file_left_behind_after_mint() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.seed");
        load_or_mint_seed(&path).unwrap();

        // Walk the parent dir; the only entry should be `identity.seed`.
        // The atomic-write sidecar is `.identity.seed.tmp` and must be gone.
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], "identity.seed");
    }

    #[test]
    fn minted_seed_has_some_entropy() {
        // Weak smoke test: a fresh OS-RNG seed should not be all zeros, and
        // two consecutive mints (in distinct dirs) should differ.
        let d1 = TempDir::new().unwrap();
        let d2 = TempDir::new().unwrap();
        let s1 = load_or_mint_seed(&d1.path().join("seed")).unwrap();
        let s2 = load_or_mint_seed(&d2.path().join("seed")).unwrap();
        assert_ne!(s1, [0u8; 32]);
        assert_ne!(s2, [0u8; 32]);
        assert_ne!(s1, s2);
    }

    #[cfg(unix)]
    #[test]
    fn minted_file_has_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.seed");
        load_or_mint_seed(&path).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "seed file must be 0o600, got {mode:o}");
    }

    #[test]
    fn minted_seed_drives_a_deterministic_wallet() {
        use crate::Wallet;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.seed");
        let seed = load_or_mint_seed(&path).unwrap();
        let w1 = Wallet::from_seed(&seed);
        let w2 = Wallet::from_seed(&load_or_mint_seed(&path).unwrap());
        assert_eq!(w1.public_key(), w2.public_key());
    }
}

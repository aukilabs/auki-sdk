//! `cluster_name` resolution from CLI flag or `AUKI_CLUSTER_NAME` env var.
//!
//! The Auki SDK already accepts `cluster_name` as a parameter everywhere it
//! matters (see [`crate::cluster_doc::ClusterDoc::cluster_name`] and the
//! `cluster_name` argument on every [`crate::discovery_client`] method).
//! What's missing is a shared way for daemons (boosterapp, sentinel, park)
//! to read the value out of `--cluster-name <name>` or `AUKI_CLUSTER_NAME`
//! without each reimplementing the same precedence + fail-fast.
//!
//! This module supplies that.
//!
//! ## Strict semantics ([sawslin Decision #1](https://www.notion.so/3585c8e9659280dd9093c703d88e1530))
//!
//! - **Flag wins over env.** Operators run a daemon with `--cluster-name foo`
//!   and that's the cluster, regardless of what's in the environment.
//! - **No default.** Both flag and env unset → [`ClusterNameError::Unset`].
//!   The intent: a daemon never silently joins the wrong cluster because
//!   somebody forgot to wire `--cluster-name` at deploy time.
//! - **Env value of empty string is treated as unset.** Avoids the trap of
//!   `AUKI_CLUSTER_NAME=` exported into a shell from a stale `.env`.
//!
//! ## Boosterapp's back-compat carve-out
//!
//! Sawslin Decision #1 has a follow-up: **boosterapp** uses
//! `flag > env > default "vinland"` rather than strict fail-fast, because a
//! K1 deployment without a re-flashed systemd unit needs to keep joining
//! the historical vinland cluster. That carve-out lives in boosterapp; this
//! helper stays strict. Other daemons (park, sentinel, future) follow strict.

/// Reasons [`resolve`] can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterNameError {
    /// Neither the CLI flag nor the `AUKI_CLUSTER_NAME` env var was set
    /// (or the env var was the empty string). The daemon must specify a
    /// cluster — there is no default.
    Unset,
}

impl std::fmt::Display for ClusterNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClusterNameError::Unset => write!(
                f,
                "cluster_name unset: pass --cluster-name <name> or set \
                 AUKI_CLUSTER_NAME (no default)"
            ),
        }
    }
}

impl std::error::Error for ClusterNameError {}

/// Name of the env var consulted as fallback when the CLI flag is absent.
pub const ENV_VAR: &str = "AUKI_CLUSTER_NAME";

/// Resolve `cluster_name` from a CLI flag (passed in as `flag`) or the
/// `AUKI_CLUSTER_NAME` env var, in that order. See module docs for
/// semantics.
///
/// Pass `flag = None` when the CLI parser saw no `--cluster-name`. Empty
/// string env values are treated as unset.
pub fn resolve(flag: Option<&str>) -> Result<String, ClusterNameError> {
    if let Some(name) = flag {
        return Ok(name.to_string());
    }
    match std::env::var(ENV_VAR) {
        Ok(s) if !s.is_empty() => Ok(s),
        _ => Err(ClusterNameError::Unset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sentinel value used when the env var is not the unit under test —
    /// keeps tests independent of any actual `AUKI_CLUSTER_NAME` set in
    /// the developer's shell.
    fn with_env<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let prev = std::env::var(ENV_VAR).ok();
        // SAFETY: setenv/unsetenv on POSIX is not thread-safe with
        // concurrent reads. Cargo's default test harness runs tests in
        // parallel; tests touching this env var must serialize via the
        // module's `_serial` mutex (below). The body executes between
        // set and restore.
        unsafe {
            match value {
                Some(v) => std::env::set_var(ENV_VAR, v),
                None => std::env::remove_var(ENV_VAR),
            }
        }
        let out = body();
        unsafe {
            match prev {
                Some(p) => std::env::set_var(ENV_VAR, p),
                None => std::env::remove_var(ENV_VAR),
            }
        }
        out
    }

    /// Tests that read or write `AUKI_CLUSTER_NAME` must hold this mutex.
    /// Cargo runs tests in parallel by default; without serialization, two
    /// env-var-touching tests interleave and assertions race against each
    /// other's setvar/unsetvar.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn flag_wins_when_set() {
        let _g = env_lock();
        with_env(Some("from-env"), || {
            assert_eq!(resolve(Some("from-flag")).unwrap(), "from-flag");
        });
    }

    #[test]
    fn falls_back_to_env_when_flag_absent() {
        let _g = env_lock();
        with_env(Some("from-env"), || {
            assert_eq!(resolve(None).unwrap(), "from-env");
        });
    }

    #[test]
    fn errors_when_both_absent() {
        let _g = env_lock();
        with_env(None, || {
            assert_eq!(resolve(None), Err(ClusterNameError::Unset));
        });
    }

    #[test]
    fn empty_env_string_is_treated_as_unset() {
        let _g = env_lock();
        with_env(Some(""), || {
            assert_eq!(resolve(None), Err(ClusterNameError::Unset));
        });
    }

    #[test]
    fn empty_flag_string_is_passed_through() {
        // `Some("")` is a deliberate explicit choice from the CLI parser.
        // We don't second-guess it — the empty string would fail in
        // `ClusterDoc` validation if it ever made it that far. Test pins
        // the contract so a future change is deliberate.
        let _g = env_lock();
        with_env(None, || {
            assert_eq!(resolve(Some("")).unwrap(), "");
        });
    }
}

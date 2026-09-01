/// Terminal failure category retained by a peer lifecycle result.
///
/// Detailed errors are returned from startup or shutdown. Status intentionally
/// remains small, copyable, and safe to retain in watch channels and logs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AukiPeerFailure {
    /// The authenticated P2P transport failed.
    Transport,
    /// Signed authority could not be renewed or installed.
    Authority,
    /// Relay booking or reservation reconciliation failed.
    Relay,
    /// The facade lifecycle monitor stopped unexpectedly.
    Supervisor,
    /// Ordered resource cleanup failed or timed out.
    Cleanup,
}

/// Stable result from waiting for an Auki peer runtime to stop.
///
/// Platform-specific transport diagnostics remain in startup, shutdown, and
/// logs. Lifecycle observers receive only this small cross-target result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AukiPeerExit {
    /// The peer reached its requested terminal state.
    Stopped,
    /// An unexpected runtime component failed.
    Failed(AukiPeerFailure),
}

/// Local lifecycle and readiness snapshot for one facade-owned peer.
///
/// Startup is an atomic readiness gate: the first observable status is the
/// state in which [`crate::AukiPeer::start`] returns. Startup failures are
/// returned directly rather than exposed as an intermediate status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(not(target_arch = "wasm32"))]
pub enum AukiPeerStatus {
    /// Authority and transport are ready and required reachability is confirmed.
    Ready,
    /// The transport is alive but no current signed authority is usable.
    AuthorityUnavailable,
    /// Relay-backed reachability is required but currently unavailable.
    RelayUnavailable,
    /// A terminal runtime component failed.
    Failed(AukiPeerFailure),
    /// Ordered shutdown has begun and new work should not be accepted.
    Stopping,
    /// Every facade-owned runtime capability has stopped.
    Stopped,
}

#[cfg(not(target_arch = "wasm32"))]
impl AukiPeerStatus {
    /// Whether the runtime can currently accept application work.
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Whether no later ready transition is possible.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Failed(_) | Self::Stopping | Self::Stopped)
    }

    /// Whether this is a stable result from waiting for the peer to stop.
    ///
    /// `Stopping` is terminal for readiness but is not final: ordered cleanup
    /// may still finish as either `Stopped` or `Failed(Cleanup)`.
    pub const fn is_final(self) -> bool {
        matches!(self, Self::Failed(_) | Self::Stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn status_helpers_distinguish_readiness_and_terminal_states() {
        assert!(AukiPeerStatus::Ready.is_ready());
        assert!(!AukiPeerStatus::RelayUnavailable.is_ready());
        assert!(AukiPeerStatus::Failed(AukiPeerFailure::Relay).is_terminal());
        assert!(AukiPeerStatus::Stopped.is_terminal());
        assert!(AukiPeerStatus::Stopping.is_terminal());
        assert!(AukiPeerStatus::Failed(AukiPeerFailure::Relay).is_final());
        assert!(AukiPeerStatus::Stopped.is_final());
        assert!(!AukiPeerStatus::Stopping.is_final());
    }

    #[test]
    fn exit_result_is_target_neutral() {
        assert_eq!(AukiPeerExit::Stopped, AukiPeerExit::Stopped);
        assert_eq!(
            AukiPeerExit::Failed(AukiPeerFailure::Transport),
            AukiPeerExit::Failed(AukiPeerFailure::Transport)
        );
    }
}

//! Manager-side join authorization hook.
//!
//! Apps (e.g. DPM) install an [`AdmitGate`] so inbound `/auki/join/0.0.1`
//! requests are checked before membership mutation. Async so future
//! Zitadel / HTTP token verification can await without changing the
//! trait shape.

use libp2p_identity::PeerId;
use std::future::Future;
use std::pin::Pin;

/// Manager-side checker for join `authorization` (HTTP Authorization
/// header analogue on [`auki_network::join_protocol::JoinRequest`]).
pub trait AdmitGate: Send + Sync {
    /// Return `Ok(())` to admit, or `Err(reason)` to reject with that
    /// reason string in `JoinResponse::Reject`.
    fn check(
        &self,
        peer: PeerId,
        authorization: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;
}

/// Joiner + Manager auth knobs threaded through bootstrap / create / join.
#[derive(Clone, Default)]
pub struct JoinAuthConfig {
    /// Manager-side gate. `None` → open admit (legacy Hagall behavior).
    pub admit_gate: Option<std::sync::Arc<dyn AdmitGate>>,
    /// Value placed in outbound `JoinRequest.authorization` when joining
    /// (full header, e.g. `Bearer <token>`). Empty when unused.
    pub join_authorization: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct DelayOkGate;

    impl AdmitGate for DelayOkGate {
        fn check(
            &self,
            _peer: PeerId,
            authorization: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
            let auth = authorization.to_string();
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                if auth == "Bearer ok" {
                    Ok(())
                } else {
                    Err("unauthorized".into())
                }
            })
        }
    }

    #[tokio::test]
    async fn async_admit_gate_awaits_and_checks_authorization() {
        let gate: Arc<dyn AdmitGate> = Arc::new(DelayOkGate);
        let peer = PeerId::random();
        assert!(gate.check(peer, "Bearer ok").await.is_ok());
        assert_eq!(
            gate.check(peer, "Bearer nope").await.unwrap_err(),
            "unauthorized"
        );
    }
}

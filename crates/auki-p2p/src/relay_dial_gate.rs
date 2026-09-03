use async_channel::{Receiver, Sender};
use std::sync::Arc;

/// Match libp2p Relay v2's per-connection circuit-handshake capacity.
///
/// The upstream relay client drops requests submitted beyond this boundary
/// instead of applying backpressure. Keep excess Auki dials outside libp2p
/// until a previous circuit handshake has completed.
pub(crate) const MAX_CONCURRENT_RELAY_CIRCUIT_DIALS: usize = 10;

#[derive(Clone)]
pub(crate) struct RelayCircuitDialGate {
    inner: Arc<RelayCircuitDialGateInner>,
}

struct RelayCircuitDialGateInner {
    available: Receiver<()>,
    returned: Sender<()>,
}

impl RelayCircuitDialGate {
    pub(crate) fn new() -> Self {
        let (returned, available) = async_channel::bounded(MAX_CONCURRENT_RELAY_CIRCUIT_DIALS);
        for _ in 0..MAX_CONCURRENT_RELAY_CIRCUIT_DIALS {
            returned
                .try_send(())
                .expect("new relay circuit dial gate has room for every permit");
        }
        Self {
            inner: Arc::new(RelayCircuitDialGateInner {
                available,
                returned,
            }),
        }
    }

    pub(crate) async fn acquire(&self) -> RelayCircuitDialPermit {
        self.inner
            .available
            .recv()
            .await
            .expect("relay circuit dial gate retains its sender");
        RelayCircuitDialPermit {
            returned: self.inner.returned.clone(),
        }
    }
}

pub(crate) struct RelayCircuitDialPermit {
    returned: Sender<()>,
}

impl Drop for RelayCircuitDialPermit {
    fn drop(&mut self) {
        let returned = self.returned.try_send(());
        debug_assert!(
            returned.is_ok(),
            "relay circuit dial permit returned more than once"
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use futures::{pin_mut, FutureExt};

    use super::*;

    #[tokio::test]
    async fn waits_until_an_inflight_circuit_handshake_finishes() {
        let gate = RelayCircuitDialGate::new();
        let cloned_gate = gate.clone();
        let mut permits = Vec::new();
        for _ in 0..MAX_CONCURRENT_RELAY_CIRCUIT_DIALS {
            permits.push(gate.acquire().await);
        }

        let waiting = cloned_gate.acquire();
        pin_mut!(waiting);
        assert!(waiting.as_mut().now_or_never().is_none());

        permits.pop();
        waiting.await;
    }
}
